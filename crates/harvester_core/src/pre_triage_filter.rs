use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{briefing::LoadedArticle, JobId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterReason {
    BlockedHost,
    VerySmallContent,
    PaywallShellTitle,
    SmallMediumContent,
    BoilerplateDensity,
    HighLinkDensity,
    StubPhraseLowContent,
}

impl FilterReason {
    fn sort_key(self) -> u8 {
        match self {
            Self::BlockedHost => 0,
            Self::VerySmallContent => 1,
            Self::PaywallShellTitle => 2,
            Self::SmallMediumContent => 3,
            Self::BoilerplateDensity => 4,
            Self::HighLinkDensity => 5,
            Self::StubPhraseLowContent => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoVerdict {
    HardExclude,
    Review,
    Include,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManualDecision {
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArticleFilterKey {
    pub url: String,
    pub content_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleFilterEntry {
    pub key: ArticleFilterKey,
    pub source_title: Option<String>,
    pub auto_verdict: AutoVerdict,
    pub reasons: Vec<FilterReason>,
    pub manual_decision: Option<ManualDecision>,
}

impl ArticleFilterEntry {
    pub(crate) fn is_excluded(&self) -> bool {
        matches!(resolved_decision(self), ManualDecision::Exclude)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreTriagePhase {
    Idle,
    LoadingArticles,
    Reviewing,
    ReadyToTriage,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreTriageLifecycle {
    Idle,
    LoadingArticles,
    Loaded,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreTriagePolicy {
    blocked_hosts: Vec<String>,
    paywall_shell_titles: Vec<String>,
    boilerplate_phrases: Vec<String>,
    stub_phrases: Vec<String>,
    hard_exclude_word_count: usize,
    hard_exclude_char_count: usize,
    review_word_count_min: usize,
    review_word_count_max_exclusive: usize,
    review_boilerplate_hits: usize,
    review_link_density_threshold: f64,
}

impl Default for PreTriagePolicy {
    fn default() -> Self {
        Self {
            blocked_hosts: vec!["youtube.com".to_string(), "youtu.be".to_string()],
            paywall_shell_titles: vec![
                "subscribe to read".to_string(),
                "sign in to continue".to_string(),
            ],
            boilerplate_phrases: vec![
                "continue reading".to_string(),
                "enable cookies".to_string(),
                "subscribe".to_string(),
                "sign in".to_string(),
                "advertisement".to_string(),
            ],
            stub_phrases: vec![
                "watch now".to_string(),
                "continue reading".to_string(),
                "enable cookies".to_string(),
            ],
            hard_exclude_word_count: 60,
            hard_exclude_char_count: 500,
            review_word_count_min: 60,
            review_word_count_max_exclusive: 180,
            review_boilerplate_hits: 3,
            review_link_density_threshold: 0.25,
        }
    }
}

impl PreTriagePolicy {
    pub fn evaluate(&self, article: &LoadedArticle) -> (AutoVerdict, Vec<FilterReason>) {
        let normalized_title = normalize_text(article.source_title.as_deref().unwrap_or_default());
        let normalized_body = normalize_text(&article.prepared_text);
        let host = host_from_url(&article.url);
        let word_count = word_count(&normalized_body);
        let char_count = normalized_body.chars().count();
        let boilerplate_hits = phrase_hits(&normalized_body, &self.boilerplate_phrases);
        let link_density = markdown_link_density(&article.prepared_text);
        let low_content = word_count < self.review_word_count_max_exclusive;
        let stub_hits = phrase_hits(&normalized_body, &self.stub_phrases);

        let mut reasons = Vec::new();

        if host
            .as_deref()
            .is_some_and(|h| host_is_blocked(h, &self.blocked_hosts))
        {
            reasons.push(FilterReason::BlockedHost);
        }
        if word_count < self.hard_exclude_word_count || char_count < self.hard_exclude_char_count {
            reasons.push(FilterReason::VerySmallContent);
        }
        if self.paywall_shell_titles.contains(&normalized_title) {
            reasons.push(FilterReason::PaywallShellTitle);
        }

        let hard_exclude = reasons.iter().any(|r| {
            matches!(
                r,
                FilterReason::BlockedHost
                    | FilterReason::VerySmallContent
                    | FilterReason::PaywallShellTitle
            )
        });
        if hard_exclude {
            reasons.sort_unstable_by_key(|reason| reason.sort_key());
            reasons.dedup();
            return (AutoVerdict::HardExclude, reasons);
        }

        if (self.review_word_count_min..self.review_word_count_max_exclusive).contains(&word_count)
        {
            reasons.push(FilterReason::SmallMediumContent);
        }
        if boilerplate_hits >= self.review_boilerplate_hits {
            reasons.push(FilterReason::BoilerplateDensity);
        }
        if link_density > self.review_link_density_threshold {
            reasons.push(FilterReason::HighLinkDensity);
        }
        if low_content && stub_hits > 0 {
            reasons.push(FilterReason::StubPhraseLowContent);
        }

        reasons.sort_unstable_by_key(|reason| reason.sort_key());
        reasons.dedup();
        if reasons.is_empty() {
            (AutoVerdict::Include, reasons)
        } else {
            (AutoVerdict::Review, reasons)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreTriageSession {
    lifecycle: PreTriageLifecycle,
    entries: Vec<ArticleFilterEntry>,
    first_entry_by_url: HashMap<String, usize>,
    unresolved_review_count: usize,
    job_key_by_id: HashMap<JobId, ArticleFilterKey>,
    loaded_by_url: HashMap<String, LoadedArticle>,
    preparation_budgets: HashMap<String, usize>,
}

impl Default for PreTriageSession {
    fn default() -> Self {
        Self {
            lifecycle: PreTriageLifecycle::Idle,
            entries: Vec::new(),
            first_entry_by_url: HashMap::new(),
            unresolved_review_count: 0,
            job_key_by_id: HashMap::new(),
            loaded_by_url: HashMap::new(),
            preparation_budgets: HashMap::new(),
        }
    }
}

impl PreTriageSession {
    pub fn new_loading() -> Self {
        Self {
            lifecycle: PreTriageLifecycle::LoadingArticles,
            ..Self::default()
        }
    }

    pub fn phase(&self) -> PreTriagePhase {
        match &self.lifecycle {
            PreTriageLifecycle::Idle => PreTriagePhase::Idle,
            PreTriageLifecycle::LoadingArticles => PreTriagePhase::LoadingArticles,
            PreTriageLifecycle::Loaded => {
                if self.has_unresolved_review() {
                    PreTriagePhase::Reviewing
                } else {
                    PreTriagePhase::ReadyToTriage
                }
            }
            PreTriageLifecycle::Failed { reason } => PreTriagePhase::Failed {
                reason: reason.clone(),
            },
        }
    }

    pub fn entries(&self) -> &[ArticleFilterEntry] {
        &self.entries
    }

    pub(crate) fn window_articles(
        &self,
    ) -> impl Iterator<Item = (&ArticleFilterEntry, &LoadedArticle)> + '_ {
        self.entries.iter().filter_map(|entry| {
            self.loaded_by_url
                .get(&entry.key.url)
                .map(|article| (entry, article))
        })
    }

    pub fn is_reviewing(&self) -> bool {
        matches!(self.phase(), PreTriagePhase::Reviewing)
    }

    pub fn is_interactive(&self) -> bool {
        matches!(self.lifecycle, PreTriageLifecycle::Loaded)
    }

    pub fn has_unresolved_review(&self) -> bool {
        self.unresolved_review_count > 0
    }

    pub fn load_articles(articles: Vec<LoadedArticle>, policy: &PreTriagePolicy) -> Self {
        let entries: Vec<ArticleFilterEntry> = articles
            .iter()
            .map(|article| {
                let key = ArticleFilterKey {
                    content_hash: stable_hash_u64(&article.content_hash),
                    url: article.url.clone(),
                };
                let (auto_verdict, reasons) = policy.evaluate(article);
                ArticleFilterEntry {
                    key,
                    source_title: article.source_title.clone(),
                    auto_verdict,
                    reasons,
                    manual_decision: None,
                }
            })
            .collect();
        let mut session = Self {
            lifecycle: PreTriageLifecycle::Idle,
            entries,
            first_entry_by_url: HashMap::new(),
            unresolved_review_count: 0,
            job_key_by_id: HashMap::new(),
            preparation_budgets: HashMap::new(),
            loaded_by_url: articles
                .into_iter()
                .map(|article| (article.url.clone(), article))
                .collect(),
        };
        session.rebuild_entry_index();
        session.refresh_unresolved_review_count();
        session.refresh_loaded_lifecycle("no articles passed pre-triage filters");
        session
    }

    pub fn held_articles(&self) -> Vec<harvester_engine::HeldArticle> {
        self.entries
            .iter()
            .filter_map(|e| {
                let a = self.loaded_by_url.get(&e.key.url)?;
                Some(harvester_engine::HeldArticle {
                    url: a.url.clone(),
                    content_hash: a.content_hash.clone(),
                    preparation_budget: *self.preparation_budgets.get(&a.url)?,
                })
            })
            .collect()
    }

    pub(crate) fn preparation_budget(&self, url: &str) -> Option<usize> {
        self.preparation_budgets.get(url).copied()
    }

    /// Retain preparation and verdicts for subsequent deltas without remaining actionable.
    pub(crate) fn finish_handoff(&mut self) {
        self.lifecycle = PreTriageLifecycle::Idle;
    }

    pub fn merge_delta(
        &mut self,
        delta: harvester_engine::TriageArticleDelta,
        policy: &PreTriagePolicy,
    ) {
        let mut prepared: HashMap<_, _> = delta
            .articles
            .into_iter()
            .map(|a| ((a.url.clone(), a.content_hash.clone()), a))
            .collect();
        let mut previous: HashMap<_, _> = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|entry| (entry.key.url.clone(), entry))
            .collect();
        let mut loaded = std::mem::take(&mut self.loaded_by_url);
        let mut budgets = std::mem::take(&mut self.preparation_budgets);
        let mut seen_urls = std::collections::HashSet::new();
        for member in delta.members {
            // The loader selects one filename-first member per URL. Keep the
            // reducer stable if an external delta nevertheless contains duplicates.
            if !seen_urls.insert(member.url.clone()) {
                continue;
            }
            let old = loaded
                .remove(&member.url)
                .filter(|a| a.content_hash == member.content_hash);
            let same_identity = old.is_some();
            let previous_budget = budgets.remove(&member.url);
            let mut article = match prepared
                .remove(&(member.url.clone(), member.content_hash.clone()))
                .or_else(|| old.filter(|_| previous_budget == Some(delta.preparation_budget)))
            {
                Some(a) => a,
                None => continue,
            };
            if article.prepared_text.len() > delta.preparation_budget {
                continue;
            }
            article.source_title = member.source_title;
            article.fetched_utc = member.fetched_utc;
            // A changed preparation budget does not change identity or its verdict.
            let entry = if same_identity {
                previous.remove(&article.url)
            } else {
                None
            };
            let mut entry = entry.unwrap_or_else(|| {
                let (auto_verdict, reasons) = policy.evaluate(&article);
                ArticleFilterEntry {
                    key: ArticleFilterKey {
                        url: article.url.clone(),
                        content_hash: stable_hash_u64(&article.content_hash),
                    },
                    source_title: article.source_title.clone(),
                    auto_verdict,
                    reasons,
                    manual_decision: None,
                }
            });
            entry.source_title = article.source_title.clone();
            self.preparation_budgets
                .insert(article.url.clone(), delta.preparation_budget);
            self.loaded_by_url.insert(article.url.clone(), article);
            self.entries.push(entry);
        }
        self.job_key_by_id.clear();
        self.rebuild_entry_index();
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
    }

    pub fn set_manual_decision(
        &mut self,
        key: &ArticleFilterKey,
        decision: ManualDecision,
    ) -> Result<(), &'static str> {
        if !self.is_interactive() {
            return Err("manual decisions are only allowed while reviewing");
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| &entry.key == key) else {
            return Err("filter key not found");
        };
        entry.manual_decision = Some(decision);
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no included articles after manual decisions");
        Ok(())
    }

    pub fn clear_manual_decisions(&mut self) {
        for entry in &mut self.entries {
            entry.manual_decision = None;
        }
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
    }

    pub fn apply_manual_overrides(
        &mut self,
        overrides: &HashMap<ArticleFilterKey, ManualDecision>,
    ) {
        if overrides.is_empty() {
            self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
            return;
        }
        for entry in &mut self.entries {
            entry.manual_decision = overrides.get(&entry.key).copied();
        }
        self.refresh_unresolved_review_count();
        self.refresh_loaded_lifecycle("no articles passed pre-triage filters");
    }

    pub fn manual_overrides(&self) -> HashMap<ArticleFilterKey, ManualDecision> {
        self.entries
            .iter()
            .filter_map(|entry| {
                entry
                    .manual_decision
                    .map(|decision| (entry.key.clone(), decision))
            })
            .collect()
    }

    /// Returns the committed included URL set.
    ///
    /// This is intentionally available only in `ReadyToTriage`: during `Reviewing`, unresolved
    /// review rows are still tentative, so callers that need provisional include/exclude state
    /// should use `tentative_included_urls()` or `resolved_included_articles()` instead.
    pub fn resolved_included_urls(&self) -> Vec<String> {
        if !matches!(self.phase(), PreTriagePhase::ReadyToTriage) {
            return Vec::new();
        }
        self.resolved_included_urls_internal()
    }

    /// Returns the URLs that would be included based on current auto-verdicts and manual
    /// decisions, without requiring the session to be in `ReadyToTriage` phase.
    ///
    /// Used during the `Reviewing` phase to show a provisional corpus before all review
    /// items are settled. Manual decisions override auto-verdicts; `HardExclude` auto-verdicts
    /// without a manual override are excluded; everything else is tentatively included.
    pub(crate) fn tentative_included_urls(&self) -> Vec<String> {
        self.resolved_included_urls_internal()
    }

    /// Borrow the provisional include set without allocating one URL per article.
    /// This follows the same decision rules as `tentative_included_urls` and preserves
    /// duplicate entries if a loaded delta contains the same URL more than once.
    pub(crate) fn tentative_included_url_refs(&self) -> impl Iterator<Item = &str> + '_ {
        self.entries.iter().filter_map(|entry| {
            (resolved_decision(entry) == ManualDecision::Include).then_some(entry.key.url.as_str())
        })
    }

    #[cfg(test)]
    pub(crate) fn tentative_included_url_count(&self) -> usize {
        self.tentative_included_url_refs().count()
    }

    pub(crate) fn has_resolved_included_article(&self) -> bool {
        self.tentative_included_url_refs()
            .any(|url| self.loaded_by_url.contains_key(url))
    }

    /// Returns the currently included articles for interactive workflows.
    ///
    /// Unlike `resolved_included_urls()`, this includes tentative decisions while `Reviewing`.
    /// The triage handoff uses this so the user can run triage from an in-progress review.
    pub fn resolved_included_articles(&self) -> Vec<LoadedArticle> {
        self.resolved_included_urls_internal()
            .into_iter()
            .filter_map(|url| self.loaded_by_url.get(&url).cloned())
            .collect()
    }

    pub fn corpus_fingerprint(&self) -> u64 {
        stable_hash_u64(&self.resolved_included_urls_internal().join("\n"))
    }

    pub fn bind_job_ids(&mut self, job_url_pairs: &[(JobId, String)]) {
        self.job_key_by_id.clear();
        for (job_id, job_url) in job_url_pairs {
            if let Some(entry) = self.entry_for_url(job_url) {
                self.job_key_by_id.insert(*job_id, entry.key.clone());
            }
        }
    }

    pub fn key_for_job(&self, job_id: JobId) -> Option<ArticleFilterKey> {
        self.job_key_by_id.get(&job_id).cloned()
    }

    pub fn entry_for_url(&self, url: &str) -> Option<&ArticleFilterEntry> {
        self.first_entry_by_url
            .get(url)
            .map(|&index| &self.entries[index])
    }

    pub fn article_content_hash(&self, url: &str) -> Option<&str> {
        self.loaded_by_url
            .get(url)
            .map(|article| article.content_hash.as_str())
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Construct a session whose internal lifecycle is `Loaded` with no entries.
    ///
    /// This state is unreachable via normal transitions: normal lifecycle refreshes transition to
    /// `Failed` when the resolved included URL set is empty. This constructor exists solely to
    /// exercise defensive guards if state is ever constructed directly in tests.
    #[cfg(test)]
    pub fn loaded_empty_for_test() -> Self {
        Self {
            lifecycle: PreTriageLifecycle::Loaded,
            entries: Vec::new(),
            first_entry_by_url: HashMap::new(),
            unresolved_review_count: 0,
            job_key_by_id: HashMap::new(),
            loaded_by_url: HashMap::new(),
            preparation_budgets: HashMap::new(),
        }
    }

    fn refresh_loaded_lifecycle(&mut self, empty_included_reason: &'static str) {
        if self.entries.is_empty() {
            self.lifecycle = PreTriageLifecycle::Failed {
                reason: "no articles available".to_string(),
            };
            return;
        }
        let included = self.resolved_included_urls_internal();
        if included.is_empty() {
            self.lifecycle = PreTriageLifecycle::Failed {
                reason: empty_included_reason.to_string(),
            };
            return;
        }
        self.lifecycle = PreTriageLifecycle::Loaded;
    }

    fn resolved_included_urls_internal(&self) -> Vec<String> {
        self.tentative_included_url_refs()
            .map(str::to_owned)
            .collect()
    }

    fn rebuild_entry_index(&mut self) {
        self.first_entry_by_url.clear();
        for (index, entry) in self.entries.iter().enumerate() {
            self.first_entry_by_url
                .entry(entry.key.url.clone())
                .or_insert(index);
        }
    }

    fn refresh_unresolved_review_count(&mut self) {
        self.unresolved_review_count = self
            .entries
            .iter()
            .filter(|entry| {
                matches!(entry.auto_verdict, AutoVerdict::Review) && entry.manual_decision.is_none()
            })
            .count();
    }
}

fn resolved_decision(entry: &ArticleFilterEntry) -> ManualDecision {
    if let Some(decision) = entry.manual_decision {
        return decision;
    }
    match entry.auto_verdict {
        AutoVerdict::HardExclude => ManualDecision::Exclude,
        AutoVerdict::Review | AutoVerdict::Include => ManualDecision::Include,
    }
}

fn host_from_url(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
}

fn host_is_blocked(host: &str, blocked_hosts: &[String]) -> bool {
    blocked_hosts
        .iter()
        .any(|blocked| host == blocked || host.ends_with(&format!(".{blocked}")))
}

fn normalize_text(raw: &str) -> String {
    raw.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn phrase_hits(text: &str, phrases: &[String]) -> usize {
    phrases
        .iter()
        .filter(|phrase| text.contains(phrase.as_str()))
        .count()
}

fn markdown_link_density(markdown: &str) -> f64 {
    let word_count = markdown.split_whitespace().count();
    if word_count == 0 {
        return 0.0;
    }
    let links = markdown.matches("](").count();
    links as f64 / word_count as f64
}

pub(crate) fn stable_hash_u64(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(url: &str, content_hash: &str, text: &str) -> LoadedArticle {
        LoadedArticle {
            url: url.to_string(),
            source_title: None,
            prepared_text: text.to_string(),
            content_hash: content_hash.to_string(),
            fetched_utc: None,
        }
    }

    fn assert_borrowed_includes_match_materialized_values(session: &PreTriageSession) {
        let materialized = session.tentative_included_urls();
        let borrowed = session.tentative_included_url_refs().collect::<Vec<_>>();
        assert_eq!(
            borrowed,
            materialized.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(session.tentative_included_url_count(), materialized.len());
        assert_eq!(
            session.has_resolved_included_article(),
            !session.resolved_included_articles().is_empty()
        );
        let unresolved = session
            .entries
            .iter()
            .filter(|entry| {
                matches!(entry.auto_verdict, AutoVerdict::Review) && entry.manual_decision.is_none()
            })
            .count();
        assert_eq!(session.unresolved_review_count, unresolved);
        for entry in &session.entries {
            assert_eq!(
                session.entry_for_url(&entry.key.url),
                session
                    .entries
                    .iter()
                    .find(|candidate| candidate.key.url == entry.key.url)
            );
        }
    }

    #[test]
    fn borrowed_included_article_queries_follow_verdict_and_delta_changes() {
        let url_a = "https://pre-triage-query.example/a";
        let url_b = "https://pre-triage-query.example/b";
        let rich_text = std::iter::repeat_n("contentword", 220)
            .collect::<Vec<_>>()
            .join(" ");
        let mut session = PreTriageSession::load_articles(
            vec![
                article(url_a, "hash-a", &rich_text),
                article(url_b, "hash-b-old", "too short"),
            ],
            &PreTriagePolicy::default(),
        );
        assert_borrowed_includes_match_materialized_values(&session);
        assert_eq!(session.tentative_included_url_count(), 1);

        let a_key = session
            .entries()
            .iter()
            .find(|entry| entry.key.url == url_a)
            .expect("article A entry")
            .key
            .clone();
        session
            .set_manual_decision(&a_key, ManualDecision::Exclude)
            .expect("loaded decisions can change");
        assert_borrowed_includes_match_materialized_values(&session);
        assert!(!session.has_resolved_included_article());

        session.merge_delta(
            harvester_engine::TriageArticleDelta::full_window(
                vec![
                    article(url_a, "hash-a", &rich_text),
                    article(url_b, "hash-b-new", &rich_text),
                ],
                100_000,
            ),
            &PreTriagePolicy::default(),
        );
        assert_borrowed_includes_match_materialized_values(&session);
        assert_eq!(session.tentative_included_urls(), vec![url_b.to_string()]);
        assert!(session.has_resolved_included_article());

        session.merge_delta(
            harvester_engine::TriageArticleDelta::full_window(
                vec![article(url_b, "hash-b-replaced", &rich_text)],
                100_000,
            ),
            &PreTriagePolicy::default(),
        );
        assert_borrowed_includes_match_materialized_values(&session);
        assert_eq!(session.entry_for_url(url_a), None);
    }
}
