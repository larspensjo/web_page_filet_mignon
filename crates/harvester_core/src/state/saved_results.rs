//! Reducer-owned projection of paid results. No file reads or view-time cache walks.
use super::AppState;
use crate::{
    ArticleSummaryResult, ArticleTriageResult, SignalCandidateCacheKey, SummaryCacheKey,
    TriageCacheKey,
};
use harvester_engine::{llm::dto::SignalCandidateResult, llm::prompt::PromptId, WindowArticle};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedArticleResults {
    pub article: WindowArticle,
    pub in_window: bool,
    pub actionable: bool,
    pub triage: Option<(TriageCacheKey, ArticleTriageResult)>,
    pub summary: Option<ArticleSummaryResult>,
    pub newest_summary: Option<SummaryCacheKey>,
    pub signal: Option<SignalCandidateResult>,
    pub signal_key: Option<SignalCandidateCacheKey>,
}

impl AppState {
    #[cfg(test)]
    pub(crate) fn prepare_saved_session_fixture(&mut self) {
        for job in self.jobs.values_mut() {
            if job.fetched_utc.is_none() && job.outcome == Some(crate::JobResultKind::Success) {
                job.fetched_utc = Some(
                    chrono::DateTime::parse_from_rfc3339("2026-09-28T12:00:00Z")
                        .expect("fixture date")
                        .with_timezone(&chrono::Utc),
                );
            }
        }
        for url in self
            .signal_candidate()
            .iter_completed()
            .map(|(url, _)| url.to_owned())
            .collect::<Vec<_>>()
        {
            if self.triage().article_content_hash(&url).is_some()
                || self.briefing().articles().iter().any(|a| a.url == url)
            {
                continue;
            }
            let key = harvester_engine::archive_url_key(&url);
            self.saved_articles
                .entry(key)
                .or_insert_with(|| WindowArticle {
                    url: url.to_string(),
                    content_hash: format!("fixture-hash-{url}"),
                    source_title: None,
                    fetched_utc: None,
                });
        }
    }

    #[cfg(test)]
    pub(crate) fn discard_fixture_pending_results(&mut self) {
        self.pending_results.clear();
    }
    pub(crate) fn saved_results_entries(&self) -> impl Iterator<Item = &SavedArticleResults> {
        self.saved_results.values()
    }

    pub(crate) fn saved_results_for_url(&self, url: &str) -> Option<&SavedArticleResults> {
        self.saved_results.get(url).or_else(|| {
            self.saved_results
                .get(&harvester_engine::archive_url_key(url))
        })
    }

    pub(crate) fn current_summary_for_url(&self, url: &str) -> Option<&ArticleSummaryResult> {
        self.saved_results_for_url(url)?.summary.as_ref()
    }

    pub(crate) fn newest_summary_for_url(&self, url: &str) -> Option<&ArticleSummaryResult> {
        let key = self.saved_results_for_url(url)?.newest_summary.as_ref()?;
        self.summary_cache().lookup(key).map(|entry| &entry.result)
    }

    pub(crate) fn saved_articles_loaded(&mut self, articles: Vec<WindowArticle>) {
        self.saved_articles_ready = true;
        self.saved_articles.clear();
        for article in articles {
            self.saved_articles
                .entry(harvester_engine::archive_url_key(&article.url))
                .or_insert(article);
        }
        self.rebuild_saved_results();
    }

    pub(crate) fn rebuild_saved_results(&mut self) {
        self.saved_scope_clock = self.last_observed_utc();
        self.saved_results_global_revision = self.unfinished_revisions().1;
        let mut articles: BTreeMap<_, Cow<'_, WindowArticle>> = self
            .saved_articles
            .iter()
            .map(|(key, article)| (key.clone(), Cow::Borrowed(article)))
            .collect();
        for article in self
            .pre_triage()
            .window_articles()
            .map(|(_, article)| article)
        {
            articles.insert(
                harvester_engine::archive_url_key(&article.url),
                Cow::Owned(WindowArticle {
                    url: article.url.clone(),
                    content_hash: article.content_hash.clone(),
                    source_title: article.source_title.clone(),
                    fetched_utc: article.fetched_utc.clone(),
                }),
            );
        }
        for article in self.briefing().articles() {
            articles
                .entry(harvester_engine::archive_url_key(&article.url))
                .or_insert_with(|| {
                    Cow::Owned(WindowArticle {
                        url: article.url.clone(),
                        content_hash: article.content_hash.clone(),
                        source_title: article.source_title.clone(),
                        fetched_utc: article.fetched_utc.clone(),
                    })
                });
        }
        for article in self.triage().articles() {
            articles
                .entry(harvester_engine::archive_url_key(&article.url))
                .or_insert_with(|| {
                    Cow::Owned(WindowArticle {
                        url: article.url.clone(),
                        content_hash: article.content_hash.clone(),
                        source_title: article.source_title.clone(),
                        fetched_utc: article.fetched_utc.clone(),
                    })
                });
        }
        let jobs: HashMap<_, _> = self
            .jobs
            .values()
            .map(|job| (job.archive_url_key().into_owned(), job))
            .collect();
        let mut results = BTreeMap::new();
        let summary_context_hash = crate::context_hash(self.context_for(PromptId::ArticleSummary));
        let signal_context_hash =
            crate::context_hash(self.context_for(PromptId::ArticleSignalCandidate));
        for (url_key, article) in articles {
            let fetched = article
                .fetched_utc
                .as_deref()
                .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                .map(|date| date.with_timezone(&chrono::Utc));
            // Window membership follows the corpus scan/export frontmatter rule.
            // Job timestamps govern only the time-scoped job lists.
            let in_window = match self.briefing_since_utc() {
                Some(since) => fetched.is_none_or(|date| date >= since),
                None => true,
            };
            let job = jobs.get(&url_key).copied();
            let fetched = job.map(|job| job.fetched_utc).unwrap_or(fetched);
            let recent = fetched
                .zip(self.last_observed_utc())
                .is_some_and(|(date, now)| date >= now - chrono::Duration::hours(24));
            if !in_window && !recent {
                continue;
            }
            let actionable = self.pre_triage().is_resolved_included(&article.url);
            let entry = self.resolve_saved_article(
                article.into_owned(),
                in_window,
                actionable,
                &summary_context_hash,
                &signal_context_hash,
            );
            results.insert(url_key, entry);
        }
        self.saved_results = results;
        self.saved_urls_by_hash.clear();
        self.saved_urls_by_signal_key.clear();
        for (url, entry) in &self.saved_results {
            self.saved_urls_by_hash
                .entry(entry.article.content_hash.clone())
                .or_default()
                .push(url.clone());
            if let Some(key) = &entry.signal_key {
                self.saved_urls_by_signal_key
                    .entry(key.clone())
                    .or_default()
                    .push(url.clone());
            }
        }
    }

    pub(crate) fn rebuild_saved_results_if_changed(&mut self) {
        if self.saved_results_global_revision != self.unfinished_revisions().1 {
            self.rebuild_saved_results();
        }
    }

    fn resolve_saved_article(
        &self,
        article: WindowArticle,
        in_window: bool,
        actionable: bool,
        summary_context_hash: &str,
        signal_context_hash: &str,
    ) -> SavedArticleResults {
        let triage = self
            .current_triage_cache_key(&article.content_hash)
            .and_then(|key| self.triage_cache().lookup(&key))
            .map(|(key, result)| (key.clone(), result.clone()));
        let summary_key = self
            .current_summary_cache_key_with_context_hash(
                &article.content_hash,
                summary_context_hash,
            )
            .ok();
        let summary = summary_key
            .as_ref()
            .and_then(|key| self.try_reuse_summary(key))
            .cloned();
        let signal_key = triage.as_ref().zip(summary_key.as_ref().zip(summary.as_ref())).and_then(|((_, triage), (key, summary))| {
            if triage.priority < crate::update::signal_candidate::PRIORITY_CUTOFF_INCLUSIVE { return None; }
            crate::update::signal_candidate::input_key_for_current_result_fields_with_context_hash(self,
                crate::update::signal_candidate::SignalArticleFields { url: &article.url, source_title: article.source_title.as_deref(), fetched_utc: article.fetched_utc.as_deref() }, triage, key, summary, signal_context_hash)
        });
        let signal = signal_key
            .as_ref()
            .and_then(|key| self.try_reuse_signal_candidate(key));
        let newest_summary = self
            .saved_newest_summaries
            .get(&article.content_hash)
            .cloned();
        SavedArticleResults {
            article,
            in_window,
            actionable,
            triage,
            summary,
            newest_summary,
            signal,
            signal_key,
        }
    }

    pub(crate) fn refresh_saved_hash(&mut self, hash: &str) {
        let summary_context_hash = crate::context_hash(self.context_for(PromptId::ArticleSummary));
        let signal_context_hash =
            crate::context_hash(self.context_for(PromptId::ArticleSignalCandidate));
        let urls = self
            .saved_urls_by_hash
            .get(hash)
            .cloned()
            .unwrap_or_default();
        for url in urls {
            let previous = self.saved_results.remove(&url).expect("indexed article");
            if let Some(key) = &previous.signal_key {
                if let Some(urls) = self.saved_urls_by_signal_key.get_mut(key) {
                    urls.retain(|candidate| candidate != &url);
                }
            }
            let entry = self.resolve_saved_article(
                previous.article,
                previous.in_window,
                previous.actionable,
                &summary_context_hash,
                &signal_context_hash,
            );
            if let Some(key) = &entry.signal_key {
                self.saved_urls_by_signal_key
                    .entry(key.clone())
                    .or_default()
                    .push(url.clone());
            }
            self.saved_results.insert(url, entry);
        }
    }

    pub(crate) fn refresh_saved_signal(&mut self, key: &SignalCandidateCacheKey) {
        let urls = self
            .saved_urls_by_signal_key
            .get(key)
            .cloned()
            .unwrap_or_default();
        for url in urls {
            let signal = self.try_reuse_signal_candidate(key);
            if let Some(entry) = self.saved_results.get_mut(&url) {
                entry.signal = signal;
            }
        }
    }

    pub(crate) fn rebuild_saved_newest_summaries(&mut self) {
        let mut newest: HashMap<String, SummaryCacheKey> = HashMap::new();
        for (key, entry) in self.summary_cache().iter() {
            if key.prompt_id != PromptId::ArticleSummary {
                continue;
            }
            if newest.get(&key.content_hash).is_none_or(|previous| {
                entry.created_at_utc
                    >= self
                        .summary_cache()
                        .lookup(previous)
                        .expect("saved key")
                        .created_at_utc
            }) {
                newest.insert(key.content_hash.clone(), key.clone());
            }
        }
        self.saved_newest_summaries = newest;
    }

    pub(crate) fn refresh_saved_newest_summary(
        &mut self,
        key: &SummaryCacheKey,
        previous_created_at: Option<&str>,
    ) {
        let candidate = self.summary_cache().lookup(key).expect("saved key");
        // Equal timestamps use the last winner in store iteration order. Growth
        // can reorder tied buckets; older writes/replacements must also resolve
        // those ties. A strictly newer write is the unique winner without a walk.
        let newest = if key.prompt_id == PromptId::ArticleSummary
            && previous_created_at
                .is_none_or(|previous| candidate.created_at_utc.as_str() > previous)
        {
            Some(key.clone())
        } else {
            self.summary_cache()
                .iter()
                .filter(|(candidate, _)| {
                    candidate.content_hash == key.content_hash
                        && candidate.prompt_id == PromptId::ArticleSummary
                })
                .max_by(|(_, a), (_, b)| a.created_at_utc.cmp(&b.created_at_utc))
                .map(|(key, _)| key.clone())
        };
        if let Some(newest) = newest {
            self.saved_newest_summaries
                .insert(key.content_hash.clone(), newest);
        }
        self.refresh_saved_hash(&key.content_hash);
    }
}
