use harvester_core::{SignalCandidateCache, SignalCandidateCacheEntry, SignalCandidateCacheKey};
#[cfg(test)]
use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
#[cfg(test)]
use harvester_engine::llm::prompt::PromptId;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedKey {
    signal_input_hash: String,
    prompt_id: String,
    prompt_version: u32,
    model_id: String,
    context_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedResult {
    signal_score: u8,
    signal_key: String,
    themes: Vec<String>,
    draft_gist: String,
    source_tier: String,
    confidence: String,
    reasoning: String,
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedEntry {
    result: PersistedResult,
    created_at_utc: String,
}

pub(crate) fn legacy(
    path: &Path,
) -> io::Result<Vec<(SignalCandidateCacheKey, SignalCandidateCacheEntry)>> {
    crate::result_store::read_ron::<
        SignalCandidateCacheKey,
        SignalCandidateCacheEntry,
        PersistedKey,
        PersistedEntry,
    >(path)
}

pub fn load(path: &Path) -> io::Result<SignalCandidateCache> {
    let records = crate::result_store::load::<
        SignalCandidateCacheKey,
        SignalCandidateCacheEntry,
        PersistedKey,
        PersistedEntry,
    >(path, legacy)?;
    let mut cache = SignalCandidateCache::default();
    for (key, entry) in records {
        cache.insert(key, entry);
    }
    Ok(cache)
}

/// Append the supplied entries. Production completions use the runner's ordered sink.
#[cfg(test)]
pub(crate) fn save(path: &Path, cache: &SignalCandidateCache) -> io::Result<()> {
    let records: Vec<_> = cache.entries.iter().collect();
    crate::result_store::AppendFile::open::<
        SignalCandidateCacheKey,
        SignalCandidateCacheEntry,
        PersistedKey,
        PersistedEntry,
    >(path, legacy)?
    .append::<_, _, PersistedKey, PersistedEntry>(&records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::SignalCandidateInputBundle;
    use tempfile::TempDir;

    #[test]
    fn cache_round_trip_preserves_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".signal_candidate_cache.ron");

        let mut cache = SignalCandidateCache::default();
        let key = SignalCandidateCacheKey {
            signal_input_hash: "abc".into(),
            prompt_id: PromptId::ArticleSignalCandidate,
            prompt_version: 1,
            model_id: "gpt-x".into(),
            context_hash: "ctx".into(),
        };
        let entry = SignalCandidateCacheEntry {
            result: SignalCandidateResult {
                signal_score: 80,
                signal_key: "test-event-key".into(),
                themes: vec!["t".into()],
                draft_gist: "x".repeat(60),
                source_tier: SourceTier::Tier1,
                confidence: Confidence::High,
                reasoning: "r".into(),
                input_tokens: 100,
                output_tokens: 10,
            },
            created_at_utc: "2026-05-25T00:00:00Z".into(),
        };
        cache.insert(key.clone(), entry.clone());

        save(&path, &cache).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.get(&key), Some(&entry));
    }

    #[test]
    fn cache_load_returns_default_when_file_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.ron");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn cache_round_trip_preserves_try_new_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".signal_candidate_cache.ron");
        let key_points = vec!["Point A".to_string()];
        let bundle = SignalCandidateInputBundle {
            url: "https://example.com/a",
            outlet: "example.com",
            title: "Example title",
            published_at: "2026-05-25",
            triage_priority: 3,
            triage_tags_sorted: vec!["ai", "chips"],
            summary: "Example summary",
            key_points: &key_points,
            upstream_summary_cache_digest: "summary-digest".into(),
        };
        let key = SignalCandidateCacheKey::try_new(
            &bundle,
            Some(1),
            Some("gpt-signal"),
            &[("context".into(), "value".into())],
        )
        .unwrap();
        let entry = SignalCandidateCacheEntry {
            result: SignalCandidateResult {
                signal_score: 82,
                signal_key: "example-signal-key".into(),
                themes: vec!["AI Infrastructure".into()],
                draft_gist:
                    "Example company disclosed a concrete AI infrastructure deployment today."
                        .into(),
                source_tier: SourceTier::Tier2,
                confidence: Confidence::Medium,
                reasoning: "Concrete event from a reputable outlet.".into(),
                input_tokens: 100,
                output_tokens: 10,
            },
            created_at_utc: "2026-05-25T00:00:00Z".into(),
        };
        let mut cache = SignalCandidateCache::default();
        cache.insert(key.clone(), entry.clone());

        save(&path, &cache).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.get(&key), Some(&entry));
    }
}
