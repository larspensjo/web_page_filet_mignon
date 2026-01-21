## Implementation plan

Scope and architecture assumptions follow the Unidirectional Data Flow + effects model and crate-boundary discipline described in the project docs.  

### Core constraints to enforce from step 1

* **UDF loop:** `PlatformEvent -> Msg -> update(state,msg) -> effects -> render(state)-> PlatformCommands`. `update()` is **pure**. 
* **Encapsulation:** no “getters” that expose internal struct state broadly; expose **capabilities** (methods) and **small immutable snapshots** when needed (e.g., `JobRowView`). Prefer `pub(crate)` over `pub`. Use module facades to keep internals private. 
* **Determinism for logic/tests:** deterministic IDs, ordering (`BTreeMap`/sorted vectors), stable file naming, stable export format.  
* **Finishing intake policy (locked 2026-01-21):** `SessionState::Finishing` keeps the intake closed; drop/ignore paste or start-style messages while draining. Auto-resume from `Finishing`/`Finished` is deferred and must be feature-flagged if added later.

### source code organization
```text
repo-root/
  Cargo.toml                  # workspace
  Cargo.lock
  README.md
  LICENSE*
  rust-toolchain.toml         # optional: pin toolchain
  .gitmodules                 # if using CommanDuctUI as submodule
  .gitignore

  src/
    CommanDuctUI/             # git submodule (kept untouched)

  crates/
    harvester_app/            # binary: Win32 UI + event loop + effect runner wiring
      Cargo.toml
      src/
        main.rs
        ui/
          mod.rs
          layout.rs
          constants.rs
        platform/
          mod.rs              # CommanDuctUI adapter: AppEvent -> Msg, Commands out
        effects/
          mod.rs              # async runner: Effect -> engine calls -> Msg back

    harvester_core/           # pure: state machine + view model + formatting helpers
      Cargo.toml
      src/
        lib.rs                # public facade only
        state.rs              # private fields; constructors & capability methods
        msg.rs
        effect.rs
        update.rs             # update(state,msg) -> (state,effects)
        view_model.rs         # thin snapshots for rendering
      tests/                  # optional: integration tests of reducer invariants

    harvester_engine/         # IO pipeline: fetch/extract/convert/tokenize/write
      Cargo.toml
      src/
        lib.rs                # public facade: EngineHandle + events
        engine.rs             # internal task orchestration (private)
        fetch/
          mod.rs
        extract/
          mod.rs
        convert/
          mod.rs
        tokenize/
          mod.rs
        persist/
          mod.rs
        types.rs              # Stage, FailureKind, engine events
      tests/
        fixtures/             # HTML samples, expected MD snapshots
        wiremock/             # mock server based tests

  docs/
    ProjectConcept.md
    Architecture.md           # UDF loop, crate boundaries, encapsulation rules
    Testing.md                # golden tests, wiremock, determinism rules

  tools/
    justfile                  # or scripts/ for build/test/lint helpers
    scripts/

  .github/
    workflows/
      ci.yml
```


---

## Phase 0 — Repo + scaffolding (buildable after each step)

### Step 0.1 — Workspace skeleton + CI-like local commands

**Deliverable**

* Cargo workspace with:

  * `harvester_app` (binary): CommanDuctUI integration + UDF store + effect runner hookup.
  * `harvester_core` (lib): pure domain types + `update()` + `render_model()` + formatting helpers.
  * `harvester_engine` (lib): fetch/process pipeline + concurrency + cancellation.
* Root `justfile` or `scripts/` with:

  * `cargo build`, `cargo test`, `cargo fmt`, `cargo clippy` (optional for later hardening).

**Tests**

* `harvester_core` has a trivial unit test (sanity compile + `update()` no-op).

**Encapsulation notes**

* `harvester_core::lib.rs` is the public facade; internal modules private (`mod update; mod state; mod msg;`).

---

## Phase 1 — UDF “store” MVP (UI shows state, no networking yet)

### Step 1.1 — Define minimal domain model in `harvester_core`

**Deliverable**

* Types (keep them small and explicit):

  * `AppState { session: SessionState, jobs: BTreeMap<JobId, JobState>, metrics: MetricsState, ui: UiState, dirty: bool }`
  * `enum SessionState { Idle, Running, Finishing, Finished }`
  * `enum Msg { UrlsPasted(String), StartClicked, StopFinishClicked, Tick, ... }`
  * `enum Effect { /* empty for now */ }`
* `fn update(state: AppState, msg: Msg) -> (AppState, Vec<Effect>)` (pure).

**Tests**

* Unit tests for:

  * paste parsing (splits lines, trims, ignores empty),
  * state transitions: Idle->Running on Start, Running->Finishing on Stop/Finish.

**Encapsulation**

* `AppState` fields private; provide only intentful methods like `state.apply_job_added(...)` if needed internally.
* Expose `AppState::view()` returning a **thin snapshot** (`AppViewModel`) for rendering only.

### Step 1.2 — Minimal CommanDuctUI window + event mapping

**Deliverable**

* `harvester_app` creates window with:

  * multiline input,
  * Start button,
  * Stop/Finish button,
  * status area (log panel or TreeView queue as recommended for MVP). 
* Map platform events to `Msg` and feed store.

**Tests**

* Platform-free tests only (still in `harvester_core`).

### Step 1.3 — Render throttling (Tick-driven) to avoid UI flooding

**Deliverable**

* Add a timer that emits `Msg::Tick` every ~50–100ms.
* Render only if `state.dirty` (or generation counter) changed; clear dirty after render. 

**Tests**

* Unit test: messages that do not change view keep dirty false.

---

## Phase 2 — Engine pipeline skeleton (still MVP, but “real work” begins)

### Step 2.1 — Job model + deterministic IDs + stable ordering

**Deliverable**

* Add:

  * `type JobId = u64` (monotonic counter in state; deterministic).
  * `JobState { url: UrlString, stage: Stage, outcome: Option<JobOutcome>, tokens: Option<u32> }`
  * `enum Stage { Queued, Downloading, Sanitizing, Converting, Tokenizing, Writing, Done }` 
* `Msg` adds engine-originated events (but not produced yet):

  * `Msg::JobProgress { job_id, stage, tokens: Option<u32>, bytes: Option<u64> }`
  * `Msg::JobDone { job_id, result: JobResultKind }`

**Tests**

* `update()` correctly applies progress and completion messages.
* Ordering test: `jobs` iterate deterministically (`BTreeMap`).

**Encapsulation**

* No direct mutation of `jobs` from outside `update()`. UI never touches jobs except via view snapshot.

### Step 2.2 — Effect system + effect runner (no-op engine)

**Deliverable**

* `Effect` now includes:

  * `Effect::EnqueueUrl { job_id, url }`
  * `Effect::StartSession`
  * `Effect::StopFinish { policy: StopPolicy }`
* `harvester_app` adds an effect runner that executes effects asynchronously and sends follow-up `Msg` back to the store (channel). 
* Engine is still stubbed: on `EnqueueUrl`, simulate progress and completion with timers (no HTTP).

**Tests**

* `harvester_core`: verify that `StartClicked` produces `StartSession` and `EnqueueUrl` effects.
* `harvester_app`: optional smoke test behind a feature flag (not required yet).

---

## Phase 3 — Real fetching + processing + persistence (MVP feature-complete)

### Step 3.1 — Implement fetch stage with bounds

**Deliverable (`harvester_engine`)**

* `Fetcher` trait + default `ReqwestFetcher`.
* Enforce:

  * connect/read timeout,
  * redirect limit,
  * max response size,
  * content-type filtering (fail fast with `UnsupportedContentType`). 
* Emit `JobProgress` messages.
* Track original URL, final URL, and redirect count in job metadata.

**Tests**

* Use `wiremock` to simulate:

  * 200 HTML,
  * 404,
  * slow response (timeout),
  * too-large response.
* No live-site tests. 

**Encapsulation**

* Engine exposes only `EngineHandle::enqueue(job_id, url)` and emits typed events; internal tasks/channels are private.

### Step 3.2 — Readability extract + HTML→MD conversion behind traits

**Deliverable**

* `Extractor` trait and `Converter` trait; default implementations via selected crates. 
* Pipeline stages:

  1. fetch HTML bytes
  2. decode (charset strategy: Content-Type → BOM → meta charset → chardetng fallback; normalize to UTF-8)
  3. extract main content
  4. convert to Markdown

**Tests**

* Golden tests (`insta`) for:

  * markdown output from fixed HTML fixtures,
  * “stable formatting” snapshot. 
  * Encoding corpus tests: UTF-8 (with/without BOM), ISO-8859-1, Shift-JIS, conflicting header/meta.

### Step 3.3 — Frontmatter + token counting + deterministic filename

**Deliverable**

* Build final per-URL Markdown:

  * frontmatter (url/title/timestamp/encoding/token_count),
  * body markdown.
* Deterministic filename: `{sanitized_title}--{short_hash(url)}.md`. 
* Token counter behind `TokenCounter` trait.

**Tests**

* Property tests for filename sanitizer (Windows-safe, no reserved names, stable).
* Snapshot test includes frontmatter fields (timestamp can be injected/fixed in tests).

### Step 3.4 — Atomic writes + output folder validation

**Deliverable**

* Validate/create output folder at session start.
* Write via temp file then rename (atomic where possible). 
* On write completion: `Msg::JobDone { result }`.

**Tests**

* Use `tempfile` dir; assert:

  * partial writes don’t exist after failure,
  * final file exists and matches snapshot.

### Step 3.5 — Stop/Finish semantics + cancellation

**Deliverable**

* Implement explicit policy (recommended MVP default):
  * stop accepting new URLs immediately,
  * jobs in `Stage::Queued` cancelled immediately,
  * jobs in `Stage::Downloading` or later complete current stage, then check cancellation before next stage.
* Use `CancellationToken` (or equivalent) checked between stages.
* Define watchdog timeouts: Extract 30s, Convert 15s, Tokenize 10s. Exceeding → `FailureKind::ProcessingTimeout`.

**Tests**

* `wiremock` delayed endpoints:

  * enqueue multiple,
  * stop/finish,
  * assert queued are cancelled, in-flight completes (or cancelled per policy).
  * Manifest includes failure summary grouped by `FailureKind`.

### Step 3.6 — Session finalization export (“LLM paste”)

**Deliverable**

* Only generate concatenated export when session reaches `Finished`. 
* Deterministic delimiter format:

  * `===== DOC START =====` / `===== DOC END =====`
  * include url/title/tokens/fetched_utc header. 
* Optional manifest file with counts and totals.

**Tests**

* Parse the concatenated export and assert delimiter counts match successful jobs.
* Snapshot manifest.

---

## Phase 4 — Hardening pass (still incremental and testable)

### Step 4.1 — UI polish without breaking encapsulation

**Deliverable**

* TreeView items show `stage/status/tokens`.
* Add “Open output folder”.
* Add error details per job (typed `FailureKind`). 

**Tests**

* `render_model()` test: given state, produces expected UI rows (no Win32).

### Step 4.2 — Robustness checklist items

**Deliverable**

* Record original vs final URL and optional redirect chain.
* CPU-stage watchdog for extract/convert/tokenize. 
* Optional retry (1x) for transient network failures.

**Tests**

* Wiremock redirect chain + 5xx retry scenarios.

---

## Future ideas (after MVP)

* Resume capability (persist queue + outcomes + manifest and reload). 
* Preview pane for generated Markdown per selected job.
* Export chunking by token budget.
* PDF ingestion (extract text → markdown).
* Optional per-host concurrency cap (off by default, since URLs are manually curated). 
* Heuristic quality filters (minimum content length, boilerplate detection).
* Cookie import / authenticated sessions (where legal/appropriate).
* “Extraction A/B harness”: run multiple `Extractor`/`Converter` implementations on the same fixture corpus and compare diffs via `insta`.
