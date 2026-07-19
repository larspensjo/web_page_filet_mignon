# Changelog

All notable changes to `openai_provider_kit` will be documented in this file.

The format follows Keep a Changelog-style sections, and this crate uses semantic versioning once it is published outside the Harvester workspace.

## 0.3.0 - 2026-07-19

### Added

- Generic OpenAI Batch API transport, JSONL codecs, and public Chat Completions body codec.

### Changed

- Batch listing requests use the maximum page size so callers can reconcile all pages with the `after` cursor.

## [0.1.0] - Unreleased

### Added

- Initial local crate extracted from Harvester.
- OpenAI Chat Completions provider with request serialization and response parsing.
- Public request, response, model, token usage, provider trait, and error types.
- Model listing with chat-model filtering.
- Optional `test-support` mock providers for downstream tests.
- Optional `reqwest-passthrough` feature for explicit Reqwest client injection.
- README, license, example, and future GitHub Actions CI workflow for repository split preparation.

## 0.2.0 - 2026-07-15

### Changed
- HTTP 429 responses whose body carries `error.code`/`error.type` of
  `insufficient_quota` now map to `LlmError::QuotaExhausted` instead of
  `LlmError::RateLimited`, so callers can distinguish an exhausted credit
  balance from transient rate limiting. Other 429s are unchanged.
