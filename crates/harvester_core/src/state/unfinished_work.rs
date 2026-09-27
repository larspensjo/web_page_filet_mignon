use crate::briefing::{ArticleSummaryState, TriageSelectionPolicy};
use crate::pre_triage_filter::ArticleFilterEntry;
use crate::summary_cache::SummaryCacheKey;
use crate::triage::{ArticleTriageState, TriageArticle};
use crate::{AppState, LoadedArticle};
use harvester_engine::llm::prompt::PromptId;
use std::collections::HashMap;

type ArticleIdentity<'a> = (&'a str, &'a str);
type SummarySessionEntry<'a> = (usize, &'a ArticleSummaryState, Option<&'a SummaryCacheKey>);

struct ContextHashes<'a> {
    summary: &'a str,
    scoring: &'a str,
}

pub const DEFAULT_REPROCESS_NOTICE_ARTICLE_THRESHOLD: usize = 150;
pub const DEFAULT_REPROCESS_NOTICE_QUOTA_PERCENT: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnfinishedWorkClass {
    NotEligible,
    InProgress,
    NeedsTriage,
    NeedsSummary,
    NeedsScoring,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnfinishedStageVerdict {
    Unknown,
    NotEligible,
    InProgress,
    NeedsWork,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnfinishedStageVerdicts {
    pub triage: UnfinishedStageVerdict,
    pub summary: UnfinishedStageVerdict,
    pub scoring: UnfinishedStageVerdict,
}

impl UnfinishedStageVerdicts {
    fn class(self) -> UnfinishedWorkClass {
        use UnfinishedStageVerdict as V;
        match (self.triage, self.summary, self.scoring) {
            (V::NotEligible, _, _) | (_, V::NotEligible, _) | (_, _, V::NotEligible) => {
                UnfinishedWorkClass::NotEligible
            }
            (V::InProgress, _, _) | (_, V::InProgress, _) | (_, _, V::InProgress) => {
                UnfinishedWorkClass::InProgress
            }
            (V::NeedsWork, _, _) => UnfinishedWorkClass::NeedsTriage,
            (_, V::NeedsWork, _) => UnfinishedWorkClass::NeedsSummary,
            (_, _, V::NeedsWork) => UnfinishedWorkClass::NeedsScoring,
            _ => UnfinishedWorkClass::Complete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UnfinishedWork {
    #[default]
    Unknown,
    Known(UnfinishedWorkSummary),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnfinishedWorkSummary {
    pub not_eligible: usize,
    pub in_progress: usize,
    pub needs_triage: usize,
    pub needs_summary: usize,
    pub needs_scoring: usize,
    pub complete: usize,
    pub articles_with_work: usize,
    pub estimated_calls: u64,
}

impl UnfinishedWorkSummary {
    pub fn window_articles(&self) -> usize {
        self.not_eligible
            + self.in_progress
            + self.needs_triage
            + self.needs_summary
            + self.needs_scoring
            + self.complete
    }

    fn adjust(&mut self, class: UnfinishedWorkClass, add: bool) {
        let (count, calls, needs_work) = match class {
            UnfinishedWorkClass::NotEligible => (&mut self.not_eligible, 0, false),
            UnfinishedWorkClass::InProgress => (&mut self.in_progress, 0, false),
            UnfinishedWorkClass::NeedsTriage => (&mut self.needs_triage, 3, true),
            UnfinishedWorkClass::NeedsSummary => (&mut self.needs_summary, 2, true),
            UnfinishedWorkClass::NeedsScoring => (&mut self.needs_scoring, 1, true),
            UnfinishedWorkClass::Complete => (&mut self.complete, 0, false),
        };
        if add {
            *count += 1;
            if needs_work {
                self.articles_with_work += 1;
            }
            self.estimated_calls += calls;
        } else {
            *count -= 1;
            if needs_work {
                self.articles_with_work -= 1;
            }
            self.estimated_calls -= calls;
        }
    }
}

/// Evaluate the large-reprocess notice using strict greater-than thresholds.
/// An absent call quota means the quota-based condition does not apply.
pub fn evaluate_reprocess_notice(
    previously_in_window_articles_needing_work: usize,
    estimated_calls: u64,
    remaining_session_calls: Option<u64>,
) -> bool {
    if previously_in_window_articles_needing_work > DEFAULT_REPROCESS_NOTICE_ARTICLE_THRESHOLD {
        return true;
    }
    let Some(remaining_session_calls) = remaining_session_calls else {
        return false;
    };
    u128::from(estimated_calls) * 100
        > u128::from(remaining_session_calls) * u128::from(DEFAULT_REPROCESS_NOTICE_QUOTA_PERCENT)
}

impl AppState {
    pub(crate) fn reprocess_counts(
        &mut self,
        previous: &std::collections::HashSet<(String, String)>,
    ) -> (usize, u64) {
        self.recompute_unfinished_work();
        previous
            .iter()
            .filter_map(|identity| self.unfinished_classes.get(identity))
            .fold((0, 0), |(articles, calls), class| {
                let estimate = match class {
                    UnfinishedWorkClass::NeedsTriage => 3,
                    UnfinishedWorkClass::NeedsSummary => 2,
                    UnfinishedWorkClass::NeedsScoring => 1,
                    _ => 0,
                };
                (articles + usize::from(estimate > 0), calls + estimate)
            })
    }
    pub(crate) fn note_unfinished_inputs_changed(&mut self) {
        self.unfinished_inputs_revision = self.unfinished_inputs_revision.wrapping_add(1);
    }

    pub(crate) fn note_unfinished_global_inputs_changed(&mut self) {
        self.note_unfinished_inputs_changed();
        self.unfinished_global_revision = self.unfinished_global_revision.wrapping_add(1);
    }

    pub(crate) fn unfinished_revisions(&self) -> (u64, u64) {
        (
            self.unfinished_inputs_revision,
            self.unfinished_global_revision,
        )
    }

    pub(crate) fn note_unfinished_evictions(&mut self, hashes: Vec<String>) {
        self.unfinished_evicted_content_hashes.extend(hashes);
    }

    /// The last reducer-computed completeness summary. Views only read this value.
    pub fn unfinished_work(&self) -> &UnfinishedWork {
        &self.unfinished_work
    }

    pub(crate) fn recompute_unfinished_work(&mut self) {
        let (work, classes) = self.classify_unfinished_work();
        self.unfinished_work = work;
        self.unfinished_classes = classes;
        self.unfinished_evicted_content_hashes.clear();
    }

    pub(crate) fn refresh_unfinished_evictions(&mut self) {
        let hashes = std::mem::take(&mut self.unfinished_evicted_content_hashes);
        for hash in hashes {
            let identities = self
                .pre_triage
                .window_articles()
                .filter(|(_, article)| article.content_hash == hash)
                .map(|(_, article)| (article.url.clone(), article.content_hash.clone()))
                .collect::<Vec<_>>();
            for (url, content_hash) in identities {
                self.refresh_unfinished_identity(&url, &content_hash);
            }
        }
    }

    fn classify_unfinished_work(
        &self,
    ) -> (
        UnfinishedWork,
        HashMap<(String, String), UnfinishedWorkClass>,
    ) {
        if !self.completeness_metadata_ready() {
            return (UnfinishedWork::Unknown, HashMap::new());
        }

        let triage_states: HashMap<ArticleIdentity<'_>, &TriageArticle> = self
            .triage
            .articles()
            .iter()
            .map(|article| {
                (
                    (article.url.as_str(), article.content_hash.as_str()),
                    article,
                )
            })
            .collect();
        let summary_states: HashMap<ArticleIdentity<'_>, SummarySessionEntry<'_>> = self
            .briefing
            .articles()
            .iter()
            .enumerate()
            .map(|(index, article)| {
                (
                    (article.url.as_str(), article.content_hash.as_str()),
                    (
                        index,
                        &article.summary_state,
                        article.cache_key_snapshot.as_ref(),
                    ),
                )
            })
            .collect();

        let summary_policy = self.briefing_triage_policy();
        let summary_context_hash = crate::context_hash(self.context_for(PromptId::ArticleSummary));
        let scoring_context_hash =
            crate::context_hash(self.context_for(PromptId::ArticleSignalCandidate));
        let mut summary = UnfinishedWorkSummary::default();
        let mut classes = HashMap::new();
        for (entry, article) in self.pre_triage.window_articles() {
            let Some(verdicts) = self.classify_window_article(
                entry,
                article,
                &triage_states,
                &summary_states,
                summary_policy,
                ContextHashes {
                    summary: &summary_context_hash,
                    scoring: &scoring_context_hash,
                },
            ) else {
                return (UnfinishedWork::Unknown, HashMap::new());
            };
            let class = verdicts.class();
            classes.insert((article.url.clone(), article.content_hash.clone()), class);
            summary.adjust(class, true);
        }
        (UnfinishedWork::Known(summary), classes)
    }

    pub(crate) fn refresh_unfinished_identity(&mut self, url: &str, content_hash: &str) {
        if !matches!(self.unfinished_work, UnfinishedWork::Known(_)) {
            self.recompute_unfinished_work();
            return;
        }
        if !self
            .pre_triage
            .window_articles()
            .any(|(_, article)| article.url == url && article.content_hash == content_hash)
        {
            return;
        }
        let Some(verdicts) = self.unfinished_stage_verdicts(url, content_hash) else {
            self.recompute_unfinished_work();
            return;
        };
        let class = verdicts.class();
        let identity = (url.to_string(), content_hash.to_string());
        let old = self.unfinished_classes.insert(identity, class);
        if let UnfinishedWork::Known(summary) = &mut self.unfinished_work {
            if let Some(old) = old {
                summary.adjust(old, false);
            }
            summary.adjust(class, true);
        }
    }

    /// Classify each stage for a window identity under the current prompt keys.
    /// Downstream stages are unknown until their required upstream result exists.
    pub fn unfinished_stage_verdicts(
        &self,
        url: &str,
        content_hash: &str,
    ) -> Option<UnfinishedStageVerdicts> {
        use UnfinishedStageVerdict as V;
        let (entry, article) = self
            .pre_triage
            .window_articles()
            .find(|(_, article)| article.url == url && article.content_hash == content_hash)?;
        if !self.completeness_metadata_ready() {
            return Some(UnfinishedStageVerdicts {
                triage: V::Unknown,
                summary: V::Unknown,
                scoring: V::Unknown,
            });
        }
        let triage_states = self
            .triage
            .articles()
            .iter()
            .filter(|candidate| candidate.url == url && candidate.content_hash == content_hash)
            .map(|candidate| {
                (
                    (candidate.url.as_str(), candidate.content_hash.as_str()),
                    candidate,
                )
            })
            .collect();
        let summary_states = self
            .briefing
            .articles()
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.url == url && candidate.content_hash == content_hash)
            .map(|(index, candidate)| {
                (
                    (candidate.url.as_str(), candidate.content_hash.as_str()),
                    (
                        index,
                        &candidate.summary_state,
                        candidate.cache_key_snapshot.as_ref(),
                    ),
                )
            })
            .collect();
        let summary_context_hash = crate::context_hash(self.context_for(PromptId::ArticleSummary));
        let scoring_context_hash =
            crate::context_hash(self.context_for(PromptId::ArticleSignalCandidate));
        self.classify_window_article(
            entry,
            article,
            &triage_states,
            &summary_states,
            self.briefing_triage_policy(),
            ContextHashes {
                summary: &summary_context_hash,
                scoring: &scoring_context_hash,
            },
        )
    }

    fn completeness_metadata_ready(&self) -> bool {
        if !self.prompt_contexts_ready
            || self.prompt_contexts_load_failed
            || !self.triage_metadata_ready()
            || self.triage_cache_metadata().is_none()
            || (self.is_briefing_metadata_ready() && self.summary_cache_metadata().is_none())
        {
            return false;
        }

        [
            PromptId::ArticleTriage,
            PromptId::ArticleSummary,
            PromptId::ArticleSignalCandidate,
        ]
        .into_iter()
        .all(|prompt_id| {
            self.active_version_for(prompt_id).is_some()
                && self
                    .effective_model_for(prompt_id)
                    .is_some_and(|model| !model.trim().is_empty())
        })
    }

    fn classify_window_article(
        &self,
        entry: &ArticleFilterEntry,
        article: &LoadedArticle,
        triage_states: &HashMap<ArticleIdentity<'_>, &TriageArticle>,
        summary_states: &HashMap<ArticleIdentity<'_>, SummarySessionEntry<'_>>,
        summary_policy: TriageSelectionPolicy,
        context_hashes: ContextHashes<'_>,
    ) -> Option<UnfinishedStageVerdicts> {
        use UnfinishedStageVerdict as V;
        if entry.is_excluded() {
            return Some(UnfinishedStageVerdicts {
                triage: V::NotEligible,
                summary: V::NotEligible,
                scoring: V::NotEligible,
            });
        }

        let triage_key = self.current_triage_cache_key(&article.content_hash)?;
        let triage_result = self
            .triage_cache
            .lookup(&triage_key)
            .map(|(_, result)| result);
        let Some(triage_result) = triage_result else {
            let current_attempt = triage_states
                .get(&(article.url.as_str(), article.content_hash.as_str()))
                .is_some_and(|candidate| match &candidate.triage_state {
                    ArticleTriageState::Pending => true,
                    ArticleTriageState::InProgress { .. } | ArticleTriageState::Deferred => {
                        candidate.cache_key_snapshot.as_ref() == Some(&triage_key)
                    }
                    _ => false,
                });
            return Some(UnfinishedStageVerdicts {
                triage: if current_attempt {
                    V::InProgress
                } else {
                    V::NeedsWork
                },
                summary: V::Unknown,
                scoring: V::Unknown,
            });
        };

        if triage_result.priority <= summary_policy.cutoff_exclusive {
            return Some(UnfinishedStageVerdicts {
                triage: V::Complete,
                summary: V::NotEligible,
                scoring: V::NotEligible,
            });
        }

        let summary_key = self
            .current_summary_cache_key_with_context_hash(
                &article.content_hash,
                context_hashes.summary,
            )
            .ok()?;
        let summary_result = self.try_reuse_summary(&summary_key);
        let Some(summary_result) = summary_result else {
            let current_attempt = summary_states
                .get(&(article.url.as_str(), article.content_hash.as_str()))
                .is_some_and(|(_, candidate_state, cache_key)| match candidate_state {
                    ArticleSummaryState::Pending => true,
                    ArticleSummaryState::InProgress { .. } | ArticleSummaryState::Deferred => {
                        *cache_key == Some(&summary_key)
                    }
                    _ => false,
                });
            return Some(UnfinishedStageVerdicts {
                triage: V::Complete,
                summary: if current_attempt {
                    V::InProgress
                } else {
                    V::NeedsWork
                },
                scoring: V::Unknown,
            });
        };

        if triage_result.priority < crate::update::signal_candidate::PRIORITY_CUTOFF_INCLUSIVE {
            // The current summary and scoring cutoffs coincide; this retains the
            // independent scoring verdict if those policies later diverge.
            return Some(UnfinishedStageVerdicts {
                triage: V::Complete,
                summary: V::Complete,
                scoring: V::NotEligible,
            });
        }

        let scoring_key =
            crate::update::signal_candidate::input_key_for_current_result_fields_with_context_hash(
                self,
                crate::update::signal_candidate::SignalArticleFields {
                    url: &article.url,
                    source_title: article.source_title.as_deref(),
                    fetched_utc: article.fetched_utc.as_deref(),
                },
                triage_result,
                &summary_key,
                summary_result,
                context_hashes.scoring,
            )?;
        if self.try_reuse_signal_candidate(&scoring_key).is_some() {
            return Some(UnfinishedStageVerdicts {
                triage: V::Complete,
                summary: V::Complete,
                scoring: V::Complete,
            });
        }

        let scoring_digest = scoring_key.digest();
        let current_attempt =
            self.signal_candidate
                .state_for(&article.url)
                .is_some_and(|candidate_state| {
                    matches!(
                        candidate_state,
                        crate::signal_candidate::SignalCandidateState::Pending
                    ) || (self.signal_candidate.input_digest_for(&article.url)
                        == Some(scoring_digest.as_str())
                        && matches!(
                            candidate_state,
                            crate::signal_candidate::SignalCandidateState::Scoring { .. }
                                | crate::signal_candidate::SignalCandidateState::Deferred
                        ))
                });
        Some(UnfinishedStageVerdicts {
            triage: V::Complete,
            summary: V::Complete,
            scoring: if current_attempt {
                V::InProgress
            } else {
                V::NeedsWork
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reprocess_notice_uses_strict_article_and_quota_thresholds() {
        assert!(!evaluate_reprocess_notice(150, 0, Some(1_000)));
        assert!(evaluate_reprocess_notice(151, 0, Some(1_000)));
        assert!(!evaluate_reprocess_notice(0, 50, Some(100)));
        assert!(evaluate_reprocess_notice(0, 51, Some(100)));
        assert!(!evaluate_reprocess_notice(0, u64::MAX, None));
    }
}
