use std::collections::HashMap;

use crate::{briefing::LoadedArticle, JobId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualDecision {
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    job_key_by_id: HashMap<JobId, ArticleFilterKey>,
    loaded_by_url: HashMap<String, LoadedArticle>,
}

impl Default for PreTriageSession {
    fn default() -> Self {
        Self {
            lifecycle: PreTriageLifecycle::Idle,
            entries: Vec::new(),
            job_key_by_id: HashMap::new(),
            loaded_by_url: HashMap::new(),
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

    pub fn is_reviewing(&self) -> bool {
        matches!(self.phase(), PreTriagePhase::Reviewing)
    }

    pub fn is_interactive(&self) -> bool {
        matches!(self.lifecycle, PreTriageLifecycle::Loaded)
    }

    pub fn has_unresolved_review(&self) -> bool {
        self.entries.iter().any(|entry| {
            matches!(entry.auto_verdict, AutoVerdict::Review) && entry.manual_decision.is_none()
        })
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
            job_key_by_id: HashMap::new(),
            loaded_by_url: articles
                .into_iter()
                .map(|article| (article.url.clone(), article))
                .collect(),
        };
        session.refresh_loaded_lifecycle("no articles passed pre-triage filters");
        session
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
        self.refresh_loaded_lifecycle("no included articles after manual decisions");
        Ok(())
    }

    pub fn clear_manual_decisions(&mut self) {
        for entry in &mut self.entries {
            entry.manual_decision = None;
        }
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
            if let Some(entry) = self.entries.iter().find(|entry| entry.key.url == *job_url) {
                self.job_key_by_id.insert(*job_id, entry.key.clone());
            }
        }
    }

    pub fn key_for_job(&self, job_id: JobId) -> Option<ArticleFilterKey> {
        self.job_key_by_id.get(&job_id).cloned()
    }

    pub fn entry_for_url(&self, url: &str) -> Option<&ArticleFilterEntry> {
        self.entries.iter().find(|entry| entry.key.url == url)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Construct a session that is directly in `ReadyToTriage` phase with no entries.
    ///
    /// This state is unreachable via normal transitions (the phase derivation logic
    /// transitions to `Failed` when the resolved included URL set is empty). This
    /// constructor exists solely to exercise the defensive guard in `working_corpus`
    /// that prevents mis-classification if state is ever constructed directly in tests.
    #[cfg(test)]
    pub fn ready_to_triage_empty_for_test() -> Self {
        Self {
            lifecycle: PreTriageLifecycle::Loaded,
            entries: Vec::new(),
            job_key_by_id: HashMap::new(),
            loaded_by_url: HashMap::new(),
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
        self.entries
            .iter()
            .filter_map(|entry| match resolved_decision(entry) {
                ManualDecision::Include => Some(entry.key.url.clone()),
                ManualDecision::Exclude => None,
            })
            .collect()
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
