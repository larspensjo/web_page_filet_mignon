use std::collections::{HashMap, HashSet};

use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::PromptVersion;

use crate::cache_utils::hex_digest;

const ACTIVE_PROMPT_ID: &str = "ArticleSignalCandidate";
pub const DEFAULT_SELECTION_THRESHOLD: u8 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalCandidateState {
    Pending,
    Scoring { request_id: u64 },
    Deferred,
    Completed { result: SignalCandidateResult },
    Failed { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalCandidateDialogDefault {
    OnAllSettled,
    OffPartial,
    OffDisabled,
    OffEmpty,
}

/// Manual exclusion key. Versioned so a stale exclusion never silently drops a
/// future unrelated cluster that reused the same slug.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OverrideKey {
    pub signal_key: String,
    pub prompt_id: String,
    pub prompt_version: PromptVersion,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SignalCandidateSession {
    states: HashMap<String, SignalCandidateState>,
    pending_request_ids: HashMap<String, u64>,
    pending_urls_by_request: HashMap<u64, String>,
    enqueued: u32,
    completed: u32,
    failed: u32,
    excluded: HashSet<OverrideKey>,
}

impl SignalCandidateSession {
    pub fn enqueue(&mut self, url: String) -> bool {
        if self.states.contains_key(&url) {
            return false;
        }
        self.states.insert(url, SignalCandidateState::Pending);
        self.enqueued += 1;
        true
    }

    pub fn mark_scoring(&mut self, url: &str, request_id: u64) {
        if let Some(slot) = self.states.get_mut(url) {
            *slot = SignalCandidateState::Scoring { request_id };
            self.pending_request_ids.insert(url.to_string(), request_id);
            self.pending_urls_by_request
                .insert(request_id, url.to_string());
        }
    }

    pub fn complete(&mut self, url: &str, result: SignalCandidateResult) {
        if let Some(slot) = self.states.get_mut(url) {
            if matches!(
                slot,
                SignalCandidateState::Completed { .. } | SignalCandidateState::Failed { .. }
            ) {
                return;
            }
            *slot = SignalCandidateState::Completed { result };
            self.completed += 1;
            if let Some(request_id) = self.pending_request_ids.remove(url) {
                self.pending_urls_by_request.remove(&request_id);
            }
        }
    }

    pub fn fail(&mut self, url: &str, reason: impl Into<String>) {
        if let Some(slot) = self.states.get_mut(url) {
            if matches!(
                slot,
                SignalCandidateState::Completed { .. } | SignalCandidateState::Failed { .. }
            ) {
                return;
            }
            *slot = SignalCandidateState::Failed {
                reason: reason.into(),
            };
            self.failed += 1;
            if let Some(request_id) = self.pending_request_ids.remove(url) {
                self.pending_urls_by_request.remove(&request_id);
            }
        }
    }

    pub fn defer(&mut self, url: &str) {
        if let Some(slot) = self.states.get_mut(url) {
            *slot = SignalCandidateState::Deferred;
            if let Some(request_id) = self.pending_request_ids.remove(url) {
                self.pending_urls_by_request.remove(&request_id);
            }
        }
    }

    pub fn rearm_deferred(&mut self) {
        self.states
            .retain(|_, state| !matches!(state, SignalCandidateState::Deferred));
    }

    pub fn deferred_urls(&self) -> Vec<String> {
        self.states
            .iter()
            .filter(|(_, state)| matches!(state, SignalCandidateState::Deferred))
            .map(|(url, _)| url.clone())
            .collect()
    }

    pub fn state_for(&self, url: &str) -> Option<&SignalCandidateState> {
        self.states.get(url)
    }

    pub fn request_id_for(&self, url: &str) -> Option<u64> {
        self.pending_request_ids.get(url).copied()
    }

    pub fn url_for_request(&self, request_id: u64) -> Option<&str> {
        self.pending_urls_by_request
            .get(&request_id)
            .map(String::as_str)
    }

    pub fn iter_completed(&self) -> impl Iterator<Item = (&str, &SignalCandidateResult)> {
        self.states.iter().filter_map(|(url, state)| match state {
            SignalCandidateState::Completed { result } => Some((url.as_str(), result)),
            _ => None,
        })
    }

    pub fn iter_states(&self) -> impl Iterator<Item = (&str, &SignalCandidateState)> {
        self.states.iter().map(|(url, state)| (url.as_str(), state))
    }

    pub fn enqueued_count(&self) -> u32 {
        self.enqueued
    }

    pub fn completed_count(&self) -> u32 {
        self.completed
    }

    pub fn failed_count(&self) -> u32 {
        self.failed
    }

    pub fn in_flight_count(&self) -> u32 {
        self.states
            .values()
            .filter(|state| {
                matches!(
                    state,
                    SignalCandidateState::Pending | SignalCandidateState::Scoring { .. }
                )
            })
            .count() as u32
    }

    pub fn excluded(&self) -> &HashSet<OverrideKey> {
        &self.excluded
    }

    pub fn set_excluded(&mut self, set: HashSet<OverrideKey>) {
        self.excluded = set;
    }

    pub fn add_exclusion(&mut self, key: OverrideKey) {
        self.excluded.insert(key);
    }

    pub fn remove_exclusion(&mut self, key: &OverrideKey) {
        self.excluded.remove(key);
    }

    pub fn override_fingerprint(&self) -> String {
        use sha2::Digest;

        let mut entries: Vec<&OverrideKey> = self.excluded.iter().collect();
        entries.sort_by(|a, b| {
            a.signal_key
                .cmp(&b.signal_key)
                .then(a.prompt_id.cmp(&b.prompt_id))
                .then(a.prompt_version.cmp(&b.prompt_version))
        });

        let mut h = sha2::Sha256::new();
        for key in entries {
            h.update(key.signal_key.as_bytes());
            h.update(b"|");
            h.update(key.prompt_id.as_bytes());
            h.update(b"|");
            h.update(key.prompt_version.to_be_bytes());
            h.update(b";");
        }
        hex_digest(h.finalize())
    }
}

#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub url: String,
    pub result: SignalCandidateResult,
}

#[derive(Debug, Clone)]
pub struct SelectionPolicy {
    pub threshold: u8,
    pub active_prompt_version: PromptVersion,
    pub excluded: HashSet<OverrideKey>,
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_SELECTION_THRESHOLD,
            active_prompt_version: 1,
            excluded: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateArchiveSelection {
    pub selected_urls: Vec<String>,
    pub threshold: u8,
    pub override_fingerprint: String,
    pub cache_fingerprint: String,
    pub token_estimates: crate::ArchiveTokenEstimates,
    pub scoring_in_progress: bool,
}

impl SignalCandidateArchiveSelection {
    pub fn new(
        selected_urls: Vec<String>,
        threshold: u8,
        override_fingerprint: String,
        cache_fingerprint: String,
        token_estimates: crate::ArchiveTokenEstimates,
        scoring_in_progress: bool,
    ) -> Self {
        Self {
            selected_urls,
            threshold,
            override_fingerprint,
            cache_fingerprint,
            token_estimates,
            scoring_in_progress,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SignalCandidateSelection {
    pub selected_urls: Vec<String>,
    cluster_sizes: HashMap<String, usize>,
    selected_signal_key_for: HashMap<String, String>,
}

impl SignalCandidateSelection {
    pub fn compute(input: &[ScoredCandidate], policy: SelectionPolicy) -> Self {
        let mut clusters: HashMap<String, Vec<&ScoredCandidate>> = HashMap::new();
        for candidate in input
            .iter()
            .filter(|c| c.result.signal_score >= policy.threshold)
        {
            let cluster_key = canonical_signal_key(&candidate.result.signal_key);
            clusters.entry(cluster_key).or_default().push(candidate);
        }
        let excluded_cluster_keys: HashSet<String> = policy
            .excluded
            .iter()
            .filter(|override_key| {
                override_key.prompt_id == ACTIVE_PROMPT_ID
                    && override_key.prompt_version == policy.active_prompt_version
            })
            .map(|override_key| canonical_signal_key(&override_key.signal_key))
            .collect();

        let mut reps: Vec<&ScoredCandidate> = Vec::with_capacity(clusters.len());
        let mut cluster_sizes: HashMap<String, usize> = HashMap::with_capacity(clusters.len());
        for (key, members) in clusters {
            cluster_sizes.insert(key, members.len());
            let rep = members
                .into_iter()
                .min_by(|a, b| {
                    a.result
                        .source_tier
                        .cmp(&b.result.source_tier)
                        .then(b.result.signal_score.cmp(&a.result.signal_score))
                        .then(a.url.cmp(&b.url))
                })
                .expect("at least one member per cluster");
            reps.push(rep);
        }

        reps.retain(|candidate| {
            !excluded_cluster_keys.contains(&canonical_signal_key(&candidate.result.signal_key))
        });

        reps.sort_by(|a, b| {
            b.result
                .signal_score
                .cmp(&a.result.signal_score)
                .then(a.result.source_tier.cmp(&b.result.source_tier))
                .then(a.url.cmp(&b.url))
        });

        let mut selected_signal_key_for = HashMap::with_capacity(reps.len());
        let selected_urls = reps
            .iter()
            .map(|candidate| {
                selected_signal_key_for.insert(
                    candidate.url.clone(),
                    canonical_signal_key(&candidate.result.signal_key),
                );
                candidate.url.clone()
            })
            .collect();

        Self {
            selected_urls,
            cluster_sizes,
            selected_signal_key_for,
        }
    }

    pub fn cluster_size_for(&self, url: &str) -> usize {
        self.selected_signal_key_for
            .get(url)
            .and_then(|key| self.cluster_sizes.get(key).copied())
            .unwrap_or(0)
    }

    pub fn cluster_size_for_signal_key(&self, signal_key: &str) -> usize {
        self.cluster_sizes
            .get(&canonical_signal_key(signal_key))
            .copied()
            .unwrap_or(0)
    }

    pub fn signal_key_for(&self, url: &str) -> Option<&str> {
        self.selected_signal_key_for.get(url).map(String::as_str)
    }
}

pub fn canonical_signal_key(signal_key: &str) -> String {
    let tokens: Vec<&str> = signal_key
        .split('-')
        .filter(|token| !token.is_empty())
        .collect();

    if let Some(actor) = primary_actor_token(&tokens) {
        if is_model_access_restriction_event(&tokens) {
            return format!("{actor}-frontier-model-access-restriction");
        }
    }

    signal_key.to_string()
}

pub fn is_signal_key_excluded(
    excluded: &HashSet<OverrideKey>,
    signal_key: &str,
    active_prompt_version: PromptVersion,
) -> bool {
    let cluster_key = canonical_signal_key(signal_key);
    excluded.iter().any(|override_key| {
        override_key.prompt_id == ACTIVE_PROMPT_ID
            && override_key.prompt_version == active_prompt_version
            && canonical_signal_key(&override_key.signal_key) == cluster_key
    })
}

fn primary_actor_token<'a>(tokens: &'a [&str]) -> Option<&'a str> {
    tokens
        .iter()
        .copied()
        .find(|token| !is_actor_noise_token(token))
}

fn is_actor_noise_token(token: &str) -> bool {
    matches!(
        token,
        "after"
            | "all"
            | "bar"
            | "barred"
            | "bars"
            | "block"
            | "blocked"
            | "blocks"
            | "commerce"
            | "department"
            | "directive"
            | "foreign"
            | "foreigners"
            | "government"
            | "national"
            | "nationals"
            | "order"
            | "ordered"
            | "orders"
            | "s"
            | "security"
            | "trump"
            | "u"
            | "us"
            | "users"
    )
}

fn is_model_access_restriction_event(tokens: &[&str]) -> bool {
    let has_model_scope = tokens.iter().any(|token| {
        matches!(
            *token,
            "claude" | "fable" | "frontier" | "model" | "models" | "mythos" | "opus"
        )
    });
    let has_access_restriction = tokens.iter().any(|token| {
        matches!(
            *token,
            "access"
                | "bar"
                | "barred"
                | "bars"
                | "block"
                | "blocked"
                | "blocks"
                | "disable"
                | "disabled"
                | "disables"
                | "restrict"
                | "restricted"
                | "restriction"
                | "shutdown"
                | "suspend"
                | "suspended"
                | "suspends"
                | "suspension"
        )
    });
    let has_strong_access_action = tokens.iter().any(|token| {
        matches!(
            *token,
            "bar"
                | "barred"
                | "bars"
                | "block"
                | "blocked"
                | "blocks"
                | "disable"
                | "disabled"
                | "disables"
                | "restrict"
                | "restricted"
                | "restriction"
                | "shutdown"
                | "suspend"
                | "suspended"
                | "suspends"
                | "suspension"
        )
    });
    let has_policy_scope = tokens.iter().any(|token| {
        matches!(
            *token,
            "control"
                | "controls"
                | "directive"
                | "export"
                | "foreign"
                | "foreigners"
                | "government"
                | "license"
                | "licensing"
                | "national"
                | "nationals"
                | "order"
                | "ordered"
                | "orders"
                | "regulation"
                | "regulatory"
                | "security"
                | "users"
        )
    });
    let has_export_controls = tokens.iter().any(|token| matches!(*token, "export"))
        && tokens
            .iter()
            .any(|token| matches!(*token, "control" | "controls"));

    has_model_scope
        && (has_policy_scope || has_strong_access_action)
        && (has_access_restriction || has_export_controls)
}

/// Why `archive_final_selection` chose the list it did.
///
/// Mirrors the settled outcomes of [`compute_dialog_default`]:
/// `OnAllSettled` -> `SignalFiltered`, `OffEmpty` -> `FullCorpusNoCandidates`,
/// `OffDisabled` -> `FullCorpusSignalUnavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveSelectionSource {
    /// The settled signal-candidate selection narrowed the base corpus.
    SignalFiltered,
    /// Scoring produced results but none met the threshold/exclusions; the full base corpus is used.
    FullCorpusNoCandidates,
    /// No candidates were scored at all; the full base corpus is used.
    FullCorpusSignalUnavailable,
}

/// The exact ordered URL list the Archive would export right now, plus the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveFinalSelection {
    pub ordered_urls: Vec<String>,
    pub source: ArchiveSelectionSource,
}

pub fn compute_dialog_default(
    settled: u32,
    in_progress: u32,
    failed: u32,
    selection_size: usize,
) -> SignalCandidateDialogDefault {
    if settled == 0 && failed == 0 {
        return SignalCandidateDialogDefault::OffDisabled;
    }
    if in_progress > 0 {
        return SignalCandidateDialogDefault::OffPartial;
    }
    if selection_size == 0 {
        return SignalCandidateDialogDefault::OffEmpty;
    }
    SignalCandidateDialogDefault::OnAllSettled
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::dto::{Confidence, SourceTier};

    fn sample_result(score: u8, key: &str, tier: SourceTier) -> SignalCandidateResult {
        SignalCandidateResult {
            signal_score: score,
            signal_key: key.into(),
            themes: vec!["t".into()],
            draft_gist: "x".repeat(120),
            source_tier: tier,
            confidence: Confidence::High,
            reasoning: "r".into(),
            input_tokens: 100,
            output_tokens: 10,
        }
    }

    fn cand(url: &str, score: u8, key: &str, tier: SourceTier) -> ScoredCandidate {
        ScoredCandidate {
            url: url.into(),
            result: sample_result(score, key, tier),
        }
    }

    fn policy(threshold: u8, excluded: HashSet<OverrideKey>) -> SelectionPolicy {
        SelectionPolicy {
            threshold,
            active_prompt_version: 1,
            excluded,
        }
    }

    #[test]
    fn pending_then_scoring_then_completed_transitions() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("https://a/1".into());
        assert!(matches!(
            s.state_for("https://a/1"),
            Some(SignalCandidateState::Pending)
        ));

        s.mark_scoring("https://a/1", 42);
        assert!(matches!(
            s.state_for("https://a/1"),
            Some(SignalCandidateState::Scoring { request_id: 42 })
        ));

        s.complete("https://a/1", sample_result(80, "k-one", SourceTier::Tier1));
        assert!(matches!(
            s.state_for("https://a/1"),
            Some(SignalCandidateState::Completed { .. })
        ));
        assert_eq!(s.completed_count(), 1);
    }

    #[test]
    fn failure_increments_failed_counter() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("u".into());
        s.mark_scoring("u", 1);
        s.fail("u", "validation: bad");
        assert_eq!(s.failed_count(), 1);
        assert!(matches!(
            s.state_for("u"),
            Some(SignalCandidateState::Failed { .. })
        ));
    }

    #[test]
    fn duplicate_terminal_transitions_do_not_double_count() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("u".into());
        s.mark_scoring("u", 1);
        s.complete("u", sample_result(80, "k-one", SourceTier::Tier1));
        s.complete("u", sample_result(70, "k-two", SourceTier::Tier2));
        s.fail("u", "late failure");

        assert_eq!(s.enqueued_count(), 1);
        assert_eq!(s.completed_count(), 1);
        assert_eq!(s.failed_count(), 0);
        assert_eq!(s.in_flight_count(), 0);
    }

    #[test]
    fn fail_then_late_complete_does_not_double_count() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("u".into());
        s.mark_scoring("u", 1);
        s.fail("u", "validation: bad");
        s.complete("u", sample_result(80, "k-one", SourceTier::Tier1));

        assert_eq!(s.enqueued_count(), 1);
        assert_eq!(s.completed_count(), 0);
        assert_eq!(s.failed_count(), 1);
        assert_eq!(s.in_flight_count(), 0);
    }

    #[test]
    fn duplicate_enqueue_is_idempotent() {
        let mut s = SignalCandidateSession::default();
        s.enqueue("u".into());
        s.enqueue("u".into());
        assert_eq!(s.enqueued_count(), 1);
    }

    #[test]
    fn threshold_filters_low_scores() {
        let input = vec![
            cand("a", 80, "k1", SourceTier::Tier1),
            cand("b", 40, "k2", SourceTier::Tier1),
        ];
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(sel.selected_urls, vec!["a"]);
    }

    #[test]
    fn dedup_by_signal_key_keeps_best_tier_then_score() {
        let input = vec![
            cand("a", 80, "same-key", SourceTier::Tier2),
            cand("b", 70, "same-key", SourceTier::Tier1),
            cand("c", 90, "same-key", SourceTier::Tier3),
        ];
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(
            sel.selected_urls,
            vec!["b"],
            "Tier1 representative wins over Tier2/Tier3"
        );
    }

    #[test]
    fn dedup_by_canonical_model_access_restriction_key() {
        let input = vec![
            cand(
                "a",
                92,
                "anthropic-model-access-suspension-foreign-national-order",
                SourceTier::Tier2,
            ),
            cand(
                "b",
                86,
                "anthropic-disables-fable-mythos-export-controls",
                SourceTier::Tier2,
            ),
            cand(
                "c",
                84,
                "anthropic-disable-advanced-models-foreign-access",
                SourceTier::Tier1,
            ),
            cand(
                "d",
                73,
                "anthropic-blocks-claude-fable-mythos-access",
                SourceTier::Tier3,
            ),
        ];

        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));

        assert_eq!(sel.selected_urls, vec!["c"]);
        assert_eq!(
            sel.cluster_size_for_signal_key("anthropic-export-controls-mythos-fable"),
            4
        );
    }

    #[test]
    fn canonical_model_access_restriction_exclusion_removes_cluster() {
        let input = vec![
            cand(
                "a",
                92,
                "anthropic-model-access-suspension-foreign-national-order",
                SourceTier::Tier2,
            ),
            cand(
                "b",
                86,
                "anthropic-disables-fable-mythos-export-controls",
                SourceTier::Tier2,
            ),
        ];
        let mut excluded = HashSet::new();
        excluded.insert(OverrideKey {
            signal_key: "anthropic-export-controls-mythos-fable".into(),
            prompt_id: "ArticleSignalCandidate".into(),
            prompt_version: 1,
        });

        let sel = SignalCandidateSelection::compute(&input, policy(60, excluded));

        assert!(sel.selected_urls.is_empty());
    }

    #[test]
    fn model_release_key_does_not_cluster_with_access_restriction() {
        let input = vec![
            cand(
                "a",
                86,
                "anthropic-disables-fable-mythos-export-controls",
                SourceTier::Tier2,
            ),
            cand(
                "b",
                85,
                "anthropic-claude-fable-5-launch-pricing",
                SourceTier::Tier2,
            ),
        ];

        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));

        assert_eq!(sel.selected_urls.len(), 2);
    }

    #[test]
    fn dedup_tie_breaks_within_same_tier_by_score_then_url() {
        let input = vec![
            cand("z", 80, "same-key", SourceTier::Tier1),
            cand("a", 80, "same-key", SourceTier::Tier1),
            cand("m", 70, "same-key", SourceTier::Tier1),
        ];
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(sel.selected_urls, vec!["a"]);
    }

    #[test]
    fn manual_exclusion_removes_cluster() {
        let input = vec![
            cand("a", 90, "drop-this-cluster", SourceTier::Tier1),
            cand("b", 80, "keep-this-cluster", SourceTier::Tier2),
        ];
        let mut excluded = HashSet::new();
        excluded.insert(OverrideKey {
            signal_key: "drop-this-cluster".into(),
            prompt_id: "ArticleSignalCandidate".into(),
            prompt_version: 1,
        });
        let sel = SignalCandidateSelection::compute(&input, policy(60, excluded));
        assert_eq!(sel.selected_urls, vec!["b"]);
    }

    #[test]
    fn stale_manual_exclusion_version_does_not_remove_current_cluster() {
        let input = vec![cand("a", 90, "same-cluster", SourceTier::Tier1)];
        let mut excluded = HashSet::new();
        excluded.insert(OverrideKey {
            signal_key: "same-cluster".into(),
            prompt_id: "ArticleSignalCandidate".into(),
            prompt_version: 1,
        });
        let sel = SignalCandidateSelection::compute(
            &input,
            SelectionPolicy {
                threshold: 60,
                active_prompt_version: 2,
                excluded,
            },
        );
        assert_eq!(sel.selected_urls, vec!["a"]);
    }

    #[test]
    fn final_sort_is_score_desc_tier_asc_url() {
        let input = vec![
            cand("z", 80, "k1", SourceTier::Tier2),
            cand("a", 80, "k2", SourceTier::Tier1),
            cand("m", 90, "k3", SourceTier::Tier3),
        ];
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(sel.selected_urls, vec!["m", "a", "z"]);
    }

    #[test]
    fn cluster_counts_reported_for_dupes_column() {
        let input = vec![
            cand("a", 90, "shared", SourceTier::Tier1),
            cand("b", 80, "shared", SourceTier::Tier2),
            cand("c", 70, "shared", SourceTier::Tier3),
            cand("d", 60, "solo", SourceTier::Tier1),
        ];
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(sel.cluster_size_for("a"), 3);
        assert_eq!(sel.cluster_size_for("d"), 1);
    }

    #[test]
    fn dialog_default_zero_settled_zero_failed_is_off_disabled() {
        assert_eq!(
            compute_dialog_default(0, 0, 0, 0),
            SignalCandidateDialogDefault::OffDisabled
        );
    }

    #[test]
    fn dialog_default_scoring_in_progress_is_off_partial() {
        assert_eq!(
            compute_dialog_default(2, 1, 0, 2),
            SignalCandidateDialogDefault::OffPartial
        );
    }

    #[test]
    fn dialog_default_settled_but_empty_selection_is_off_empty() {
        assert_eq!(
            compute_dialog_default(5, 0, 0, 0),
            SignalCandidateDialogDefault::OffEmpty
        );
    }

    #[test]
    fn dialog_default_all_settled_with_selection_is_on_all_settled() {
        assert_eq!(
            compute_dialog_default(5, 0, 0, 3),
            SignalCandidateDialogDefault::OnAllSettled
        );
    }

    #[test]
    fn selection_keeps_all_clusters_without_cap() {
        // 30 distinct above-threshold clusters — more than the old cap of 25.
        let input: Vec<ScoredCandidate> = (0..30)
            .map(|i| {
                cand(
                    &format!("https://example.com/{i}"),
                    90,
                    &format!("key-{i}"),
                    SourceTier::Tier1,
                )
            })
            .collect();
        let sel = SignalCandidateSelection::compute(&input, policy(60, Default::default()));
        assert_eq!(
            sel.selected_urls.len(),
            30,
            "no cap: every distinct above-threshold cluster is selected"
        );
    }
}
