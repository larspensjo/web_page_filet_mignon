use std::collections::HashMap;

use harvester_engine::llm::dto::SignalCandidateResult;
use harvester_engine::llm::prompt::{PromptId, PromptVersion};
use serde::Serialize;

use crate::cache_utils::hex_digest;
use crate::summary_cache::context_hash;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignalCandidateCacheKey {
    pub signal_input_hash: String,
    /// Carried for the persisted cache-key shape; always `ArticleSignalCandidate`.
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    pub model_id: String,
    pub context_hash: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SignalCandidateCacheKeyError {
    #[error("signal_input_hash must be non-empty")]
    EmptyInputHash,
    #[error("model_id must be non-empty")]
    EmptyModelId,
    #[error("prompt_version missing")]
    MissingPromptVersion,
}

/// Components fed into the canonical input hash. Order is significant: serde
/// preserves struct field order, so this hash is reproducible across processes.
#[derive(Debug, Clone, Serialize)]
pub struct SignalCandidateInputBundle<'a> {
    pub url: &'a str,
    pub outlet: &'a str,
    pub title: &'a str,
    pub published_at: &'a str,
    pub triage_priority: u8,
    pub triage_tags_sorted: Vec<&'a str>,
    pub summary: &'a str,
    pub key_points: &'a [String],
    pub upstream_summary_cache_digest: String,
}

impl SignalCandidateInputBundle<'_> {
    pub fn hash(&self) -> String {
        let json = serde_json::to_string(self).expect("serializable");
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, json.as_bytes());
        hex_digest(sha2::Digest::finalize(hasher))
    }
}

impl SignalCandidateCacheKey {
    pub fn try_new(
        input_bundle: &SignalCandidateInputBundle<'_>,
        prompt_version: Option<PromptVersion>,
        model_id: Option<&str>,
        context: &[(String, String)],
    ) -> Result<Self, SignalCandidateCacheKeyError> {
        let model_id = model_id
            .filter(|s| !s.is_empty())
            .ok_or(SignalCandidateCacheKeyError::EmptyModelId)?
            .to_string();
        let prompt_version =
            prompt_version.ok_or(SignalCandidateCacheKeyError::MissingPromptVersion)?;
        let signal_input_hash = input_bundle.hash();
        if signal_input_hash.is_empty() {
            return Err(SignalCandidateCacheKeyError::EmptyInputHash);
        }
        Ok(Self {
            signal_input_hash,
            prompt_id: PromptId::ArticleSignalCandidate,
            prompt_version,
            model_id,
            context_hash: context_hash(context),
        })
    }

    pub fn digest(&self) -> String {
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        sha2::Digest::update(&mut hasher, self.signal_input_hash.as_bytes());
        sha2::Digest::update(&mut hasher, self.prompt_id.to_string().as_bytes());
        sha2::Digest::update(&mut hasher, self.prompt_version.to_be_bytes());
        sha2::Digest::update(&mut hasher, self.model_id.as_bytes());
        sha2::Digest::update(&mut hasher, self.context_hash.as_bytes());
        hex_digest(sha2::Digest::finalize(hasher))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalCandidateCacheEntry {
    pub result: SignalCandidateResult,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Default)]
pub struct SignalCandidateCache {
    pub entries: HashMap<SignalCandidateCacheKey, SignalCandidateCacheEntry>,
}

impl SignalCandidateCache {
    pub fn get(&self, key: &SignalCandidateCacheKey) -> Option<&SignalCandidateCacheEntry> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: SignalCandidateCacheKey, entry: SignalCandidateCacheEntry) {
        self.entries.insert(key, entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::llm::dto::{Confidence, SourceTier};

    fn sample_bundle<'a>(
        url: &'a str,
        summary: &'a str,
        upstream: &'a str,
    ) -> SignalCandidateInputBundle<'a> {
        SignalCandidateInputBundle {
            url,
            outlet: "example.com",
            title: "Title",
            published_at: "2026-05-25",
            triage_priority: 3,
            triage_tags_sorted: vec!["ai", "chips"],
            summary,
            key_points: &[],
            upstream_summary_cache_digest: upstream.to_string(),
        }
    }

    #[test]
    fn key_changes_when_summary_changes() {
        let a = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "summary-A", "upstream-1"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        let b = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "summary-B", "upstream-1"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        assert_ne!(a, b, "different summary text -> different key");
    }

    #[test]
    fn key_changes_when_upstream_summary_cache_changes() {
        let a = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "same-summary", "upstream-1"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        let b = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "same-summary", "upstream-2"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        assert_ne!(
            a, b,
            "upstream summary cache digest is part of the input hash"
        );
    }

    #[test]
    fn key_changes_when_prompt_version_or_model_changes() {
        let base = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        let v2 = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(2),
            Some("m"),
            &[],
        )
        .unwrap();
        let m2 = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("other"),
            &[],
        )
        .unwrap();
        assert_ne!(base, v2);
        assert_ne!(base, m2);
    }

    #[test]
    fn key_changes_when_context_hash_changes() {
        let a_ctx: &[(String, String)] = &[("k".into(), "v1".into())];
        let b_ctx: &[(String, String)] = &[("k".into(), "v2".into())];
        let a = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            a_ctx,
        )
        .unwrap();
        let b = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            b_ctx,
        )
        .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn cache_round_trip() {
        let key = SignalCandidateCacheKey::try_new(
            &sample_bundle("u", "s", "up"),
            Some(1),
            Some("m"),
            &[],
        )
        .unwrap();
        let mut cache = SignalCandidateCache::default();
        let entry = SignalCandidateCacheEntry {
            result: SignalCandidateResult {
                signal_score: 90,
                signal_key: "nvda-q4-earnings".into(),
                themes: vec!["inference-scarcity".into()],
                draft_gist: "x".repeat(120),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".into(),
                input_tokens: 100,
                output_tokens: 10,
            },
            created_at_utc: "2026-05-25T00:00:00Z".into(),
        };
        cache.insert(key.clone(), entry.clone());
        assert_eq!(cache.get(&key), Some(&entry));
    }
}
