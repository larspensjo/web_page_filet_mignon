use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use harvester_core::{SignalCandidateCache, SignalCandidateCacheEntry, SignalCandidateCacheKey};
use harvester_engine::llm::dto::{Confidence, SignalCandidateResult, SourceTier};
use harvester_engine::llm::prompt::PromptId;
use harvester_engine::{ensure_output_dir, AtomicFileWriter};
use serde::{Deserialize, Serialize};

const CURRENT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedKey {
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
struct PersistedEntry {
    result: PersistedResult,
    created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedFile {
    #[serde(default = "default_version")]
    version: u32,
    entries: Vec<(PersistedKey, PersistedEntry)>,
}

fn default_version() -> u32 {
    CURRENT_FORMAT_VERSION
}

pub fn save(path: &Path, cache: &SignalCandidateCache) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_output_dir(parent).map_err(io::Error::other)?;
    }
    let persisted = to_persisted(cache);
    let ron_text = ron::ser::to_string_pretty(&persisted, ron::ser::PrettyConfig::default())
        .map_err(|err| io::Error::other(err.to_string()))?;
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".signal_candidate_cache.ron");
    AtomicFileWriter::new(PathBuf::from(parent_dir))
        .write(filename, &ron_text)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(())
}

pub fn load(path: &Path) -> io::Result<SignalCandidateCache> {
    if !path.exists() {
        return Ok(SignalCandidateCache::default());
    }
    let text = std::fs::read_to_string(path)?;
    let persisted: PersistedFile = ron::from_str(&text)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    if persisted.version != CURRENT_FORMAT_VERSION {
        engine_logging::engine_warn!(
            "[signal-cache] discarding unknown cache version {} at {:?}",
            persisted.version,
            path
        );
        return Ok(SignalCandidateCache::default());
    }
    from_persisted(persisted).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn to_persisted(cache: &SignalCandidateCache) -> PersistedFile {
    PersistedFile {
        version: CURRENT_FORMAT_VERSION,
        entries: cache
            .entries
            .iter()
            .map(|(key, entry)| {
                (
                    PersistedKey {
                        signal_input_hash: key.signal_input_hash.clone(),
                        prompt_id: key.prompt_id.to_string(),
                        prompt_version: key.prompt_version,
                        model_id: key.model_id.clone(),
                        context_hash: key.context_hash.clone(),
                    },
                    PersistedEntry {
                        result: PersistedResult {
                            signal_score: entry.result.signal_score,
                            signal_key: entry.result.signal_key.clone(),
                            themes: entry.result.themes.clone(),
                            draft_gist: entry.result.draft_gist.clone(),
                            source_tier: source_tier_str(entry.result.source_tier).to_string(),
                            confidence: confidence_str(entry.result.confidence).to_string(),
                            reasoning: entry.result.reasoning.clone(),
                            input_tokens: entry.result.input_tokens,
                            output_tokens: entry.result.output_tokens,
                        },
                        created_at_utc: entry.created_at_utc.clone(),
                    },
                )
            })
            .collect(),
    }
}

fn from_persisted(persisted: PersistedFile) -> Result<SignalCandidateCache, String> {
    let mut entries = HashMap::with_capacity(persisted.entries.len());
    for (persisted_key, persisted_entry) in persisted.entries {
        let prompt_id = match persisted_key.prompt_id.as_str() {
            "ArticleSignalCandidate" => PromptId::ArticleSignalCandidate,
            other => return Err(format!("unknown prompt_id in signal cache: {other}")),
        };
        let result = SignalCandidateResult {
            signal_score: persisted_entry.result.signal_score,
            signal_key: persisted_entry.result.signal_key,
            themes: persisted_entry.result.themes,
            draft_gist: persisted_entry.result.draft_gist,
            source_tier: source_tier_from_str(&persisted_entry.result.source_tier)?,
            confidence: confidence_from_str(&persisted_entry.result.confidence)?,
            reasoning: persisted_entry.result.reasoning,
            input_tokens: persisted_entry.result.input_tokens,
            output_tokens: persisted_entry.result.output_tokens,
        };
        entries.insert(
            SignalCandidateCacheKey {
                signal_input_hash: persisted_key.signal_input_hash,
                prompt_id,
                prompt_version: persisted_key.prompt_version,
                model_id: persisted_key.model_id,
                context_hash: persisted_key.context_hash,
            },
            SignalCandidateCacheEntry {
                result,
                created_at_utc: persisted_entry.created_at_utc,
            },
        );
    }
    Ok(SignalCandidateCache { entries })
}

fn source_tier_str(tier: SourceTier) -> &'static str {
    match tier {
        SourceTier::Tier1 => "Tier1",
        SourceTier::Tier2 => "Tier2",
        SourceTier::Tier3 => "Tier3",
    }
}

fn source_tier_from_str(value: &str) -> Result<SourceTier, String> {
    match value {
        "Tier1" => Ok(SourceTier::Tier1),
        "Tier2" => Ok(SourceTier::Tier2),
        "Tier3" => Ok(SourceTier::Tier3),
        other => Err(format!("unknown source_tier: {other}")),
    }
}

fn confidence_str(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "High",
        Confidence::Medium => "Medium",
        Confidence::Low => "Low",
    }
}

fn confidence_from_str(value: &str) -> Result<Confidence, String> {
    match value {
        "High" => Ok(Confidence::High),
        "Medium" => Ok(Confidence::Medium),
        "Low" => Ok(Confidence::Low),
        other => Err(format!("unknown confidence: {other}")),
    }
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
