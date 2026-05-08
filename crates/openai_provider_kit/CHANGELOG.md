# Changelog

All notable changes to `openai_provider_kit` will be documented in this file.

The format follows Keep a Changelog-style sections, and this crate uses semantic versioning once it is published outside the Harvester workspace.

## [0.1.0] - Unreleased

### Added

- Initial local crate extracted from Harvester.
- OpenAI Chat Completions provider with request serialization and response parsing.
- Public request, response, model, token usage, provider trait, and error types.
- Model listing with chat-model filtering.
- Optional `test-support` mock providers for downstream tests.
- Optional `reqwest-passthrough` feature for explicit Reqwest client injection.
- README, license, example, and future GitHub Actions CI workflow for repository split preparation.
