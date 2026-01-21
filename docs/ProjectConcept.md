# Project Design: Web-to-Markdown Harvester (Updated)

## 1. Executive Summary
The **Web-to-Markdown Harvester** is a native Windows desktop application that turns a list of HTTP/HTTPS URLs into a clean, LLM-friendly Markdown dataset. It prioritizes:

- **Robustness:** predictable behavior under failures, cancellations, and partial runs.
- **Quality:** good extraction/sanitization with regression protection (golden tests).
- **Performance:** parallel fetching with backpressure and correct handling of CPU-bound work.
- **User control:** live monitoring, safe “Stop/Finish”, and repeatable outputs.

This project uses **CommanDuctUI** (command/event separation). The UI is driven by app state; the app never manipulates UI handles directly.

---

## 2. User Workflow
1. **Input**
   - Paste one URL per line.
   - Optionally add more URLs while a session is running.

2. **Start**
   - User starts processing.
   - The application downloads pages in parallel (with a configurable concurrency limit).

3. **Monitor**
   - A live dashboard shows per-URL stage + status.
   - Token count per document and running totals are updated as documents complete.

4. **Intervention**
   - User can add URLs during a running session.
   - User can click **Stop/Finish** to stop accepting new work and finalize exports.

5. **Output**
   - Individual `.md` files for each successfully processed URL.
   - A concatenated “LLM paste” file containing all processed documents in a deterministic format.
   - (Optional) A run manifest summarizing outcomes and totals.

---

## 3. The UI Framework: `CommanDuctUI` Considerations

### 3.1 Core Philosophy: Command–Event Separation
- **Brain (AppLogic):** owns state, decides what should appear.
- **UI (Platform):** renders and emits events.
- Communication is **message passing only**.

### 3.2 Practical UI Constraint (Important)
The MVP concept mentions a **Grid/Table** with commands like `AddRow` / row updates. If a true grid control is not available in CommanDuctUI, treat this as an early blocker and choose one of:

- **TreeView queue (recommended for MVP):**
  - Each URL is an item.
  - Item text includes stage + status + token count.
  - Optional grouping by domain or by status.
- **Status log panel (lowest effort):**
  - Append-only multi-line text showing per-URL progress.
  - Less interactive, but fast to implement.
- **Extend CommanDuctUI with a table/list view control:**
  - Best UX, but more engineering and test surface area.

---

## 4. Architecture Overview

### 4.1 Separation of Concerns
**Presentation Layer (Main Thread)**
- Component: `MyAppLogic` implements `PlatformEventHandler`.
- Responsibilities:
  - Maintain view-model state (queue, counts, current session state).
  - Emit UI commands (create controls, update text, reflect status changes).
  - Handle input events (Add URL, Start, Stop/Finish, Open output folder, etc.).

**Engine Layer (Background Workers)**
- Responsibilities:
  - Accept URLs via a **bounded** channel (backpressure).
  - Run a concurrency-limited pipeline (download + process + persist).
  - Send typed progress events back to the UI thread.

### 4.2 Make “Session” a First-Class State Machine
Define explicit session states and transitions:

- `Idle`
- `Running`
- `Finishing` (stop accepting new URLs; drain or cancel pending work per policy)
- `Finished`
- (Optional) `Cancelled` (if you want a hard cancel distinct from finishing)

**Stop/Finish semantics (must be precise):**
- “Stop accepting new URLs” should be immediate.
- Decide policy for queued + in-flight work:
  - **Policy A (safe default):** allow in-flight jobs to complete; cancel queued jobs.
  - **Policy B (hard stop):** cancel everything ASAP.
- Exports should run only after the engine reaches a stable end state.

### 4.3 Backpressure, Cancellation, and UI Update Coalescing
- Use bounded channels to avoid unbounded memory growth on huge paste operations.
- Implement cancellation (e.g., `CancellationToken`) and check it between pipeline stages.
- Coalesce UI updates:
  - Batch progress events and render deltas on a short timer tick (e.g., 50–100ms).
  - Avoid flooding the UI command queue for high URL counts.

### 4.4 CPU-bound Work in an Async Architecture
Readability parsing, HTML→Markdown conversion, and token counting are CPU heavy. If you use Tokio:

- Run network fetch as async.
- Run CPU-heavy stages in `spawn_blocking` (or a dedicated CPU thread pool).
- Put explicit limits on the CPU pool if needed to avoid contention.

---

## 5. Core Logic Implementation

### 5.1 Typed Progress Model (Recommended)
Avoid “stringly typed” progress. Prefer:

- `enum Stage { Queued, Downloading, Sanitizing, Converting, Tokenizing, Writing, Done }`
- `enum FailureKind { HttpStatus(u16), Timeout, TooLarge, UnsupportedContentType, ParseError, IoError, Cancelled, Other }`

Progress event example:
```rust
struct JobProgress {
    job_id: u64,
    url: String,
    stage: Stage,
    // Optional metrics:
    bytes_downloaded: Option<u64>,
    tokens: Option<u32>,
    error: Option<FailureKind>,
}
```

### 5.2 Per-URL Processing Pipeline
Each URL goes through these stages:

1. **Fetch**
   - `reqwest` GET
   - Enforce:
     - timeouts
     - redirect limit
     - max response size
     - content-type filtering (text/html vs unsupported)

2. **Sanitize (Readability)**
   - Parse DOM and extract the main article content.
   - Remove scripts/nav/boilerplate as much as possible.

3. **Convert (HTML → Markdown)**
   - Use a known converter (`html2md`) or controlled custom conversion.
   - Aim for stable output formatting to support golden tests.

4. **Metadata Injection**
   - Title, author (if found), canonical/final URL, fetch timestamp.
   - YAML frontmatter at the top of the document.

5. **Analysis**
   - Token count using `tiktoken-rs` (record the encoding used, e.g., `cl100k_base`).

6. **Persistence**
   - Stable deterministic filename.
   - Atomic writes where practical (write temp → rename).

7. **Export Integration**
   - Append to session “LLM paste” export using a deterministic delimiter format (see 5.4).

### 5.3 Filename Strategy (Deterministic and Collision-Proof)
Instead of relying only on title sanitization + `_1` suffix, prefer deterministic uniqueness:

- Default: `"{sanitized_title}--{short_hash(url)}.md"`
- If title is missing: `"document--{short_hash(url)}.md"`

This avoids collisions between pages that share titles and makes reruns stable.

### 5.4 Concatenated “LLM Paste” Export Format (Specify Early)
Define an explicit format so it is reliable and easy to split:

- Hard delimiter that will not appear naturally:
  - `===== DOC START =====`
  - `===== DOC END =====`
- Minimal metadata header per doc:
  - URL (final/canonical)
  - title
  - token_count
  - timestamp
- Optional: auto-split into multiple export files by token budget (useful for prompt limits).

Example:
```text
===== DOC START =====
url: https://example.com/a
title: Example A
tokens: 1234
fetched_utc: 2026-01-21T10:11:12Z
----- MARKDOWN -----
...markdown...
===== DOC END =====
```

---

## 6. Robustness Checklist (High ROI)
- HTTP:
  - connect/read timeouts
  - redirect limits
  - decompression
  - charset handling
  - response size cap
  - explicit unsupported content-type classification
- Retry policy:
  - retry transient failures (timeouts, 5xx) once (or configurable)
  - do not retry permanent failures (most 4xx) unless user manually retries
- Dedupe:
  - normalize URLs (strip fragments, canonicalize) and avoid accidental duplicates (or mark duplicates explicitly)
- Persistence:
  - atomic write pattern to prevent corrupted partial files
  - ensure output directory created and validated early
- Telemetry to user:
  - per-job failure kind
  - summary counts: succeeded/failed/cancelled

---

## 7. Roadmap & Phases (Revised)

### Phase 1: MVP (Core Value + Regression Protection)
- **UI**
  - URL input
  - Start
  - Stop/Finish with defined semantics
  - Queue display using TreeView or status log (choose approach explicitly)
  - Total token count + progress summary
- **Engine**
  - `reqwest` fetch + readability + HTML→MD + frontmatter + token count
  - bounded queue + concurrency limit
  - deterministic file naming with URL hash
  - deterministic concatenated export format with delimiters
- **Testing (must ship with MVP)**
  - wiremock-based pipeline tests
  - golden master tests for Markdown conversion

### Phase 2: Enhanced UX + Reliability
- Manual link vetting (strict manual selection; no auto-recursion)
- Resume capability (persist queue + partial results + manifest)
- Preview pane:
  - select an item and view its generated Markdown
- Export chunking by token budget

### Phase 3: Advanced Access / Formats
- Cookie import (for authenticated content)
- PDF support (extract text and convert to Markdown with metadata)
- “Stealth mode” / impersonation client (where justified and legal)
- Quality controls:
  - drop boilerplate-heavy pages
  - minimum content length
  - heuristic removal of “related links” sections

---

## 8. Testing Strategy (Expanded)

1. **Isolation**
   - Do not test against live sites.

2. **Mocking**
   - Use `wiremock` to serve stable fixtures.

3. **Golden Tests (Per Stage)**
   - Have fixtures and goldens for:
     - readability output (sanitized HTML)
     - markdown output
     - final frontmatter-injected output

4. **Concurrency/Cancellation Tests**
   - Serve delayed endpoints and assert Stop/Finish semantics:
     - queued items become cancelled (or remain queued) per policy
     - in-flight items complete or cancel per policy

5. **Property / Fuzz Tests**
   - Filename sanitizer:
     - never creates invalid Windows filenames
     - handles emoji and illegal characters
   - Concatenated export:
     - contains correct document delimiter counts
     - can be parsed deterministically

6. **UI Logic Tests (Platform-Free)**
   - Given events + state, assert emitted commands and state transitions.

---

## 9. Early Decisions / Potential Blockers (Call Out Explicitly)
To avoid rework, decide early:

1. **Queue visualization control** (TreeView vs log vs new grid control).
2. **Stop/Finish policy** (in-flight complete vs hard cancel).
3. **Deterministic export format** (delimiter + metadata + chunking).
4. **Concurrency model** (async fetch + blocking CPU stages with limits).
5. **Regression corpus** for extraction quality (fixtures + golden outputs).

