# OpenAI Client Extraction - Design & Implementation Plan

## Overview

Refactor the reusable OpenAI API client code out of `harvester_engine` into a standalone Rust library crate. Start with a local workspace crate so Harvester can keep building while the boundary is proven, then split that crate into its own public GitHub repository with independent version history and releases.

The target outcome is:

- Other projects do not depend on `web_page_filet_mignon`.
- The OpenAI client has its own public API, tests, documentation, versioning, and repository.
- Harvester keeps its domain-specific prompt orchestration, validation, replay, quota, and UI state flow.
- The migration happens incrementally, with each phase buildable and testable.

## Current Code Findings

The reusable OpenAI/client code is concentrated in `harvester_engine::llm`:

- `crates/harvester_engine/src/llm/providers/openai.rs`
  - `OpenAiProvider`
  - request serialization
  - response parsing
  - model listing
  - HTTP status/error mapping
- `crates/harvester_engine/src/llm/types.rs`
  - `ModelId`
  - `ProviderKind`
  - `ChatMessage`
  - `ChatRole`
  - `ResponseFormat`
  - `FinishReason`
  - `TokenUsage`
  - `LlmRequest`
  - `LlmResponse`
  - `LlmError`
- `crates/harvester_engine/src/llm/provider.rs`
  - `LlmProvider`
- `crates/harvester_engine/src/llm/pricing.rs`
  - `ModelPricing`
  - `PricingRegistry`
- `crates/harvester_engine/src/llm/mock_provider.rs`
  - reusable test infrastructure; it depends only on generic LLM/provider types and can move behind a `test-support` feature.

The following should remain in Harvester at first:

- `crates/harvester_engine/src/llm/handle.rs`
  - worker thread
  - prompt rendering
  - retry orchestration
  - quota checks
  - replay persistence
  - response validation
  - Harvester metadata
- `crates/harvester_engine/src/llm/prompt*.rs`
- `crates/harvester_engine/src/llm/prompts/`
- `crates/harvester_engine/src/llm/dto.rs`
- `crates/harvester_engine/src/llm/validation.rs`
- `crates/harvester_engine/src/llm/replay.rs`
- `crates/harvester_engine/src/llm/run_metadata.rs`

## Boundary Principle

The new crate should be generic OpenAI client infrastructure. It should not know about:

- Harvester
- article triage
- article summaries
- briefings
- prompt template registries
- replay cache files
- UI state
- batch jobs
- MCP article-corpus behavior
- `engine_logging`

Harvester should continue to own:

- where API keys come from
- what models are used for triage, summary, and briefing
- how prompts are rendered
- how output JSON is validated
- how usage and cost metadata flows through state
- how failures are shown to users

## Pre-Phase 1 Decisions

Resolve these before moving code. They affect public API shape and dependency surface, so deferring them until after publication would create avoidable breaking changes.

### Crate Scope And Public Type Names

Choose one coherent API direction before Phase 1:

1. **OpenAI-focused provider crate**
   - Keep the repository/crate naming from this plan: `rs-openai-provider-kit` / `openai_provider_kit`.
   - Public API should avoid unused provider variants such as `Anthropic` and `Google`.
   - Decide whether public names stay generic (`LlmRequest`, `LlmProvider`) or become OpenAI-specific before `v0.1.0`.
   - If Harvester still needs provider-agnostic concepts, keep those wrappers in `harvester_engine::llm`.

2. **Provider-agnostic LLM crate**
   - Rename before extraction, for example to `rs-llm-provider-kit` / `llm_provider_kit`.
   - Keep `ProviderKind::{OpenAi, Anthropic, Google}` and generic `Llm*` names.
   - Add only the OpenAI provider implementation in the first release.

Recommended current choice: use **OpenAI-focused provider crate** and keep only the OpenAI provider in the public crate. Harvester can retain its broader provider abstractions if needed.

The examples below preserve the current `Llm*` names to show the lowest-risk mechanical extraction path. If the OpenAI-focused option is selected, adjust those snippets before implementation so the new crate does not publish undecided provider-agnostic names or unused provider variants. Temporary compatibility aliases are acceptable while `publish = false`; they should not leak into the public `v0.1.0` API by accident.

### Dependency And TLS Surface

Decide the public dependency surface before `v0.1.0`:

- Use `reqwest` with `default-features = false`.
- Prefer `rustls` as the default TLS feature unless there is a platform-specific reason not to.
- Do not carry Harvester's full `reqwest` feature set automatically; `stream`, `gzip`, `deflate`, and `brotli` should be included only if the extracted code actually needs them.
- Treat `with_client(reqwest::Client)` as public SemVer surface. Either remove it before publication or gate it behind a clearly documented feature such as `reqwest-passthrough`.
- Keep live OpenAI tests ignored or behind an explicit environment flag.

## Phase 1 - Add A Local Standalone-Ready Crate

### Goal

Create a new local workspace crate that can later become the external repository without exposing Harvester-specific concepts.

### Name Choices

Use distinct names for GitHub and Rust packaging:

```text
GitHub repository: rs-openai-provider-kit
Crate name:        openai_provider_kit
Workspace folder:  crates/openai_provider_kit
Rust module:       openai_provider_kit
```

Rationale:

- `rs-openai-provider-kit` makes the GitHub repository clearly Rust-specific and lowers collision risk in GitHub's global namespace.
- `openai_provider_kit` keeps the crate, folder, and module names idiomatic for Rust.
- The phrase "provider kit" matches the existing `LlmProvider` boundary and leaves room for request building, model listing, response parsing, test helpers, and execution utilities.
- Avoid `openai_wire` because it can imply OpenAI wire-format compatibility rather than a client/provider library.
- Avoid carrying the `rs_` prefix into the crate name; `rs_openai_provider_kit` is noisier and less idiomatic.

### Files To Add

```text
crates/openai_provider_kit/
  Cargo.toml
  src/
    lib.rs
    types.rs
    provider.rs
    openai.rs
```

Move `mock_provider.rs` in the first increment if `test-support` is included:

```text
crates/openai_provider_kit/
  src/
    test_support.rs
  tests/
    openai.rs
```

Keep `pricing.rs` in Harvester for the first increment unless a compile dependency forces it to move.

### Initial `Cargo.toml` Shape

Start with an explicit dependency surface:

```toml
[package]
name = "openai_provider_kit"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
rust-version.workspace = true
publish = false

[features]
default = ["rustls"]
rustls = ["reqwest/rustls"]
native-tls = ["reqwest/native-tls"]
test-support = ["dep:tokio", "tokio/sync"]
reqwest-passthrough = []

[dependencies]
async-trait = "0.1"
reqwest = { version = "0.13.1", default-features = false, features = [] }
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio = { version = "1", default-features = false, optional = true }

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
wiremock = "0.6"
```

Adjust the `reqwest` feature names if the exact crate version uses different feature identifiers.

### Initial `lib.rs` Shape

Keep the entry point thin:

```rust
mod openai;
mod provider;
mod types;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use openai::OpenAiProvider;
pub use provider::LlmProvider;
pub use types::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse,
    ModelId, ProviderKind, ResponseFormat, TokenUsage,
};
```

### Workspace Changes

Add the crate to root `Cargo.toml`:

```toml
[workspace]
members = [
    "crates/openai_provider_kit",
    ...
]
```

Add a dependency from `harvester_engine`:

```toml
[dependencies]
openai_provider_kit = { path = "../openai_provider_kit" }
```

### Initial Code To Move

Move these first:

- `types.rs`
- `provider.rs`
- `providers/openai.rs`, renamed to `openai.rs`
- `mock_provider.rs`, renamed to `test_support.rs` and gated behind `test-support` if it is part of the first public API

Move later only if still useful:

- `pricing.rs`

### Public API Shape

The new crate should expose a small public surface:

```rust
pub use openai::OpenAiProvider;
pub use provider::LlmProvider;
pub use types::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmRequest, LlmResponse,
    ModelId, ProviderKind, ResponseFormat, TokenUsage,
};
```

Keep `OpenAiProvider::new(api_key)` as the normal construction path.

Keep `OpenAiProvider::from_env()` as a convenience, but do not make environment loading central to the design.

Keep `with_base_url()` because it supports tests, proxies, and local mock servers.

Do not expose `with_client(reqwest::Client)` by default. Prefer `with_base_url()` plus test servers for tests. If client injection is needed, gate `with_client` behind `reqwest-passthrough` and document the SemVer implication.

Demote implementation helpers such as request-body building, response parsing, status mapping, and reqwest error mapping from `pub` to `pub(crate)` unless they are intentionally part of the public API. Move protocol tests into the new crate so they can test those helpers locally without forcing them to remain public.

### Compatibility Re-Exports

To avoid a large cross-crate import churn in the first phase, keep compatibility re-exports from `harvester_engine::llm`:

```rust
pub use openai_provider_kit::{
    ChatMessage, ChatRole, FinishReason, LlmError, LlmProvider, LlmRequest,
    LlmResponse, ModelId, OpenAiProvider, ProviderKind, ResponseFormat, TokenUsage,
};
```

This lets existing Harvester code continue to compile while the new crate becomes the source of truth.

Compatibility re-exports only cover external call sites. In the same change that moves files, update internal `harvester_engine` imports that currently point at moved modules:

- `crate::llm::types::*`
- `crate::llm::provider::*`
- `crate::llm::providers::*`
- `super::types::*`
- `super::provider::*`

Known affected files include:

- `crates/harvester_engine/src/llm/handle.rs`
- `crates/harvester_engine/src/llm/replay.rs`
- `crates/harvester_engine/src/llm/pricing.rs`
- `crates/harvester_engine/src/llm/mock_provider.rs` if it is not moved
- `crates/harvester_engine/src/llm/mod.rs`

### Tests

Move the relevant tests from:

```text
crates/harvester_engine/tests/llm_openai.rs
```

into:

```text
crates/openai_provider_kit/tests/openai.rs
```

The moved tests should cover:

- request body serialization
- `max_tokens` vs `max_completion_tokens`
- response parsing
- content part arrays
- refusal handling
- finish reason mapping
- cached token parsing
- HTTP status mapping
- timeout/network error mapping
- model listing and filtering

Do not duplicate these protocol-level tests in Harvester. Harvester should keep its orchestration-level tests around `LlmHandle`, prompts, validation, replay, quota, and UI-facing effects.

### Validation

Run after Phase 1:

```powershell
cargo build
cargo test -p openai_provider_kit
cargo test -p harvester_engine
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Phase 1 contract: avoid broad call-site churn outside `harvester_engine` by relying on compatibility re-exports. Add a short entry to `docs/EngineeringDiary.md` if the extraction lands.

## Phase 2 - Reduce Harvester Coupling

### Goal

Make the new crate the clear owner of generic OpenAI/client types while Harvester keeps only application orchestration.

### Steps

1. Update direct users in `harvester_app`, `harvester_batch`, `harvester_mcp`, `harvester_core`, and `harvester_io` only when doing so is low-risk.
2. Keep Harvester-facing re-exports until downstream crates are updated.
3. Keep `LlmHandle` in Harvester because it depends on Harvester prompts, validation, replay, quota, logging, and metadata.
4. Keep pricing in Harvester unless a public cost-estimation API is intentionally added to the provider kit.
5. Add a short `docs/EngineeringDiary.md` entry for notable boundary decisions or reusable migration lessons.

### Pricing Decision

Do not move pricing in the first increment unless necessary.

Options:

- Keep `TokenUsage` in `openai_provider_kit` and keep `PricingRegistry` in Harvester.
- Move `PricingRegistry` later if the public crate should help consumers estimate cost.

Recommended first choice: keep pricing in Harvester until the external API is stable.

After `TokenUsage` moves, Harvester's `PricingRegistry` should import `openai_provider_kit::TokenUsage`.

### Model Constants Decision

Keep Harvester workflow defaults in Harvester:

- `DEFAULT_TRIAGE_MODEL`
- `DEFAULT_SUMMARY_MODEL`
- `DEFAULT_BRIEFING_MODEL`

Generic OpenAI model constants may move only if they are part of the public crate's intended API.

Recommended first release: ship the provider kit without model constants. Keep all current `OPENAI_MODEL_*` constants in Harvester until the public crate has a deliberate model-catalog policy.

## Phase 3 - Prepare The Crate For Public Use

### Goal

Make the local crate ready to stand on its own before splitting it into a repository.

### Files To Add

```text
crates/openai_provider_kit/
  README.md
  CHANGELOG.md
  LICENSE
  examples/
    simple_chat.rs
```

### Cargo Metadata

Add package metadata suitable for eventual publication:

```toml
[package]
name = "openai_provider_kit"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
rust-version.workspace = true
description = "Small Rust client abstraction for OpenAI chat-style API calls"
repository = "https://github.com/<owner>/rs-openai-provider-kit"
readme = "README.md"
keywords = ["openai", "llm", "api", "chat"]
categories = ["api-bindings", "asynchronous"]
```

### README Content

Include:

- installation via Git dependency during early development
- simple completion example
- model listing example
- error handling example
- explicit note that API keys are supplied by the consuming app
- supported Rust version
- license

### CI Design For The Future Repo

Use a minimal GitHub Actions workflow:

```powershell
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Do not require a live OpenAI API key for normal CI.

Live tests should remain ignored or behind an explicit environment flag.

Before tagging `v0.1.0`, finalize:

- OpenAI-only vs. provider-agnostic scope.
- Public `Llm*` vs. `OpenAi*` naming.
- Whether `ProviderKind` exists in the public API and, if it does, which variants it exposes.
- Whether `from_env()` remains in the default feature set or moves behind a convenience feature.
- Whether `with_client()` exists publicly and under which feature.

## Phase 4 - Split Into A Public GitHub Repository

### Goal

Give the client crate independent version history and make it accessible to other users.

### Recommended Split Method

Use `git subtree split` after the local crate has a clean boundary:

```powershell
git subtree split --prefix=crates/openai_provider_kit -b openai-provider-kit-split
```

Create the new public GitHub repository, then push:

```powershell
git remote add openai-provider-kit https://github.com/<owner>/rs-openai-provider-kit.git
git push openai-provider-kit openai-provider-kit-split:main
```

This creates an independent repository without carrying the rest of `web_page_filet_mignon`.

History note: `git subtree split` preserves history for files under `crates/openai_provider_kit`, but it generally does not include the earlier history of the files at their old `crates/harvester_engine/src/llm/...` paths. If preserving pre-move file history in the new repository matters, use a path-rewriting tool such as `git filter-repo` instead of relying on `git subtree split`. Otherwise document that the public repository history starts at the extraction commit.

### Initial Release

Create a `v0.1.0` tag after:

- README is accurate
- examples compile
- CI passes
- license is present
- public API is intentionally small

## Phase 5 - Consume The External Crate From Harvester

### Goal

Remove Harvester's local ownership of the OpenAI client crate and consume it as an ordinary dependency.

### Git Dependency First

Replace the local path dependency:

```toml
openai_provider_kit = { path = "../openai_provider_kit" }
```

with a pinned Git dependency:

```toml
openai_provider_kit = { git = "https://github.com/<owner>/rs-openai-provider-kit", tag = "v0.1.0" }
```

### crates.io Later

If the library should be easy for the broader Rust community to use, publish it to crates.io and switch Harvester to:

```toml
openai_provider_kit = "0.1"
```

### Remove Local Crate

After Harvester builds against the external dependency:

1. Remove `crates/openai_provider_kit`.
2. Remove it from root workspace members.
3. Remove compatibility re-exports from `harvester_engine::llm` in a follow-up commit once all Harvester call sites import the external crate directly.
4. Keep only Harvester-specific wrappers in `harvester_engine::llm`.
5. Add a short `docs/EngineeringDiary.md` entry for the external cutover.

### Validation

Run:

```powershell
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt
```

## Phase 6 - Public API Hardening

### Goal

Make the public crate easier to maintain for other users.

### Candidate Improvements

- Add builder-style request construction if the current `LlmRequest` API feels too narrow.
- Add optional feature flags:
  - `rustls`
  - `native-tls`
  - `test-support`
  - `reqwest-passthrough`
- Decide whether the crate should support only Chat Completions or add a Responses API model.
- Add rustdoc examples for the most common functions.
- Add a changelog discipline before any breaking change.

## Open Decisions

- Final GitHub repository owner.
- Final confirmation that `rs-openai-provider-kit` is available on GitHub before creation.
- Final confirmation that `openai_provider_kit` is available on crates.io before publication.
- Whether pricing belongs in the public crate.
- Whether the first public version should keep Chat Completions semantics or introduce Responses API abstractions.

## Recommended First Increment

Start with the smallest useful extraction:

1. Add `crates/openai_provider_kit`.
2. Move generic request/response/error types.
3. Move the provider trait.
4. Move `OpenAiProvider`.
5. Move reusable mock providers behind a `test-support` feature if that feature is included in the first increment.
6. Move OpenAI provider tests.
7. Update internal `harvester_engine` imports that reference moved modules.
8. Re-export the moved types from `harvester_engine::llm`.
9. Keep Harvester prompts, validation, replay, metadata, quota, pricing, and orchestration in Harvester.

This creates a real, buildable local library without forcing the final public API too early.
