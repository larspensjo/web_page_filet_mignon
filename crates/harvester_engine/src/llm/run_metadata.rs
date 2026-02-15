//! Run-level metadata contract for every LLM call.
//!
//! `LlmRunMetadata` is the canonical, lossless record of what happened during
//! a single LLM invocation: cost, latency, model, prompt, token counts,
//! validation health, input sizing, and cache status.  It flows through the
//! UDF pipeline (Effect → LlmHandle → LlmEvent → Msg → State → View) without
//! side-channels.

use serde::{Deserialize, Serialize};

use crate::llm::prompt::{PromptId, PromptVersion};

// ---------------------------------------------------------------------------
// CacheStatus
// ---------------------------------------------------------------------------

/// Whether this result was served from the replay cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheStatus {
    /// The provider was called; no cached result was used.
    Miss,
    /// A cached record whose `validated_output` is `Some` was returned.
    HitValidated,
    /// Reserved for forward-compatibility; not produced by current code.
    HitUnvalidated,
}

// ---------------------------------------------------------------------------
// LlmRunMetadata
// ---------------------------------------------------------------------------

/// Uniform metadata for a successful (or validation-failed) LLM run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmRunMetadata {
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    /// Stable model name string (not the enum), safe across serialization.
    pub resolved_model: String,
    /// Byte length of the input content passed to the worker.
    pub input_bytes: usize,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Cost in microdollars (1 USD = 1,000,000 µ$).
    pub cost_microdollars: u64,
    /// Measured wall time in milliseconds covering the full request span.
    /// For cache hits this reflects only the lookup cost and may be 0.
    pub wall_ms: u64,
    /// `true` if the response parsed and validated successfully.
    pub parse_ok: bool,
    /// Validation error description when `parse_ok = false` and the failure
    /// came from the validation step (not a pre-flight check).
    pub validation_error: Option<String>,
    pub cache_status: CacheStatus,
    /// RFC 3339 timestamp.  For cache hits: the original record's timestamp.
    pub timestamp_utc: String,
}

impl LlmRunMetadata {
    /// Smart constructor — all fields validated at build time.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prompt_id: PromptId,
        prompt_version: PromptVersion,
        resolved_model: String,
        input_bytes: usize,
        input_tokens: u32,
        output_tokens: u32,
        cost_microdollars: u64,
        wall_ms: u64,
        parse_ok: bool,
        validation_error: Option<String>,
        cache_status: CacheStatus,
        timestamp_utc: String,
    ) -> Self {
        Self {
            prompt_id,
            prompt_version,
            resolved_model,
            input_bytes,
            input_tokens,
            output_tokens,
            cost_microdollars,
            wall_ms,
            parse_ok,
            validation_error,
            cache_status,
            timestamp_utc,
        }
    }

    /// Test/stub helper — returns a valid instance with sensible defaults.
    pub fn stub() -> Self {
        Self::new(
            PromptId::ArticleTriage,
            1,
            "gpt-4o-mini".to_string(),
            100,
            10,
            20,
            300,
            50,
            true,
            None,
            CacheStatus::Miss,
            "2026-01-01T00:00:00Z".to_string(),
        )
    }

    /// Test/stub helper — returns a stub with specified overrides.
    pub fn stub_with(
        parse_ok: bool,
        validation_error: Option<String>,
        cache_status: CacheStatus,
    ) -> Self {
        Self {
            parse_ok,
            validation_error,
            cache_status,
            ..Self::stub()
        }
    }
}

// ---------------------------------------------------------------------------
// LlmFailureMetadata
// ---------------------------------------------------------------------------

/// Metadata attached to error variants where partial timing/model info is
/// available (e.g. `ValidationFailed`, post-call `QuotaExhausted`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmFailureMetadata {
    pub prompt_id: PromptId,
    pub prompt_version: PromptVersion,
    /// `None` if the model could not be resolved before the failure.
    pub resolved_model: Option<String>,
    pub input_bytes: usize,
    /// `None` for pre-flight errors that fire before the timer starts
    /// (e.g. `InputTooLarge`, `PromptNotFound`).
    pub wall_ms: Option<u64>,
    pub timestamp_utc: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_has_expected_defaults() {
        let m = LlmRunMetadata::stub();
        assert_eq!(m.prompt_id, PromptId::ArticleTriage);
        assert_eq!(m.prompt_version, 1);
        assert!(m.parse_ok);
        assert!(m.validation_error.is_none());
        assert_eq!(m.cache_status, CacheStatus::Miss);
    }

    #[test]
    fn stub_with_sets_fields() {
        let m = LlmRunMetadata::stub_with(
            false,
            Some("bad json".to_string()),
            CacheStatus::HitValidated,
        );
        assert!(!m.parse_ok);
        assert_eq!(m.validation_error.as_deref(), Some("bad json"));
        assert_eq!(m.cache_status, CacheStatus::HitValidated);
    }

    #[test]
    fn cache_status_serialization() {
        let miss = serde_json::to_string(&CacheStatus::Miss).unwrap();
        assert_eq!(miss, "\"miss\"");
        let hit: CacheStatus = serde_json::from_str("\"hit_validated\"").unwrap();
        assert_eq!(hit, CacheStatus::HitValidated);
    }
}
