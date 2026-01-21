# Project Design: Web-to-Markdown Harvester

## 1. Executive Summary
The **Web-to-Markdown Harvester** is a native Windows desktop application built in Rust. Its primary purpose is to generate high-quality text datasets for Large Language Models (LLMs) by scraping web pages, stripping non-essential content (ads, navigation, images), and converting the core information into Markdown format.

The application emphasizes performance (parallel downloads), robustness (handling errors gracefully), and user control (real-time monitoring and intervention).

## 2. User Workflow
1.  **Input:** The user pastes a list of HTTP/HTTPS URLs into the application.
2.  **Processing:** The user initiates the process. The application downloads pages in parallel.
3.  **Monitoring:** A real-time dashboard shows the status of each URL (Downloading, Processing, Done, Failed), along with the estimated LLM token count per document and a running total.
4.  **Intervention:**
    *   The user can add more URLs to the queue while processing is active.
    *   The user can click "Stop/Finish" to halt pending downloads and finalize the session.
5.  **Output:** The application saves:
    *   Individual `.md` files for each successfully processed URL.
    *   A single concatenated text file containing all documents, formatted for immediate pasting into an LLM prompt.

## 3. The UI Framework: Understanding `CommanDuctUI`

*Note to Developers: This project utilizes `CommanDuctUI`, a specific library designed for this architecture. It is **not** a standard immediate-mode GUI (like egui) or a web-view wrapper (like Tauri).*

**Core Philosophy: Command-Event Separation**
`CommanDuctUI` enforces a strict separation between the Application Logic (the "Brain") and the Native Windows UI (the "View"). They do not share memory directly. They communicate exclusively via message passing.

### The Cycle
1.  **The Brain (AppLogic):** Maintains the state (list of URLs, progress percentages, token counts).
2.  **The Output (Commands):** When the state changes, the Brain sends a **Command** to the UI.
    *   *Example:* `PlatformCommand::AddRow { id: 5, columns: ["http://google.com", "Pending"] }`
    *   *Example:* `PlatformCommand::UpdateLabel { id: TOTAL_TOKENS, text: "5043" }`
3.  **The Input (Events):** When the user interacts with the Window, the UI sends an **Event** to the Brain.
    *   *Example:* `AppEvent::ButtonClicked { id: BTN_ADD_URL }`
    *   *Example:* `AppEvent::WindowClosed`

### Key constraints
*   **Logical IDs:** You never touch a Windows Handle (`HWND`). You assign integer IDs (e.g., `const BTN_START: i32 = 100;`) to controls, and the library manages the mapping.
*   **Thread Safety:** The UI runs on the main OS thread. The `AppLogic` receives events on that thread. Heavy lifting (scraping) **must** occur on background threads to prevent freezing the UI.

## 4. Architecture Overview

### A. The Presentation Layer (Main Thread)
*   **Component:** `MyAppLogic` (Implementation of `PlatformEventHandler`).
*   **Responsibility:**
    *   Receives `AppEvent`s from the UI.
    *   Manages the `SessionState` (list of active downloads).
    *   Spawns the backend engine.
    *   Polls the backend for progress updates via a channel.
    *   Dispatches `PlatformCommand`s to update the progress grid and status bars.

### B. The Harvester Engine (Background Thread / Async Runtime)
*   **Component:** `HarvesterEngine`.
*   **Tech Stack:** `tokio` (Async Runtime), `reqwest` (HTTP Client).
*   **Responsibility:**
    *   Accepts URLs via an input channel (MPSC).
    *   Manages a pool of concurrent download tasks.
    *   Performs HTML processing and Token counting (CPU-bound tasks).
    *   Sends status updates (`DownloadStarted`, `ConversionFinished`, `Error`) back to the UI thread via an output channel.

## 5. Core Logic Implementation

### 5.1 The Processing Pipeline (Per URL)
Every URL goes through these distinct stages:

1.  **Fetch:**
    *   Use `reqwest` to perform a GET request.
    *   *Constraint:* Ignore Domain Rate Limiting for MVP.
    *   *Constraint:* If the status code is not 200, or headers indicate non-text content (e.g., PDF/Image), mark as **Failed**.
2.  **Sanitize (Readability):**
    *   Use a library port of Mozilla's **Readability** (e.g., `readability` crate).
    *   This parses the DOM and strips navbars, footers, and scripts, extracting only the `<article>` content.
3.  **Convert (HTML -> Markdown):**
    *   Convert the sanitized HTML into Markdown.
    *   Library candidate: `html2md` or custom logic to handle headers and lists cleanly.
4.  **Metadata Injection:**
    *   Extract Title, Author, and Original URL.
    *   Prepend this data as **YAML Frontmatter** to the Markdown string.
5.  **Analysis:**
    *   Calculate LLM Token count using `tiktoken-rs` (using `cl100k_base` encoding).
6.  **Persistence:**
    *   Generate a "Simplified Filename" (See 5.2).
    *   Write content to disk.

### 5.2 Filename Sanitization Strategy
We cannot trust web titles to be valid filenames.
*   **Logic:**
    1.  Take the Page Title.
    2.  Replace invalid characters (`/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`) with hyphens or underscores.
    3.  Truncate to a reasonable length (e.g., 50 chars) to avoid filesystem limits.
    4.  Append `.md`.
    5.  *Conflict Resolution:* If `file.md` exists, auto-rename to `file_1.md`.

## 6. Data Structure Definitions (Draft)

```rust
// Message sent from UI to Backend
enum JobRequest {
    AddUrl(String),
    StopEngine,
}

// Message sent from Backend to UI
enum JobUpdate {
    Started { id: usize, url: String },
    Progress { id: usize, state: String }, // e.g., "Downloading", "Converting"
    Finished { id: usize, token_count: usize, filename: String },
    Failed { id: usize, error_msg: String },
}

// Struct for the UI to track rows
struct DownloadItem {
    id: usize,
    url: String,
    status: ProcessingStatus,
    token_count: Option<usize>,
}
```

## 7. Roadmap & Phases

### Phase 1: MVP (The Core)
*   **UI:** Simple URL input box, "Add" button, and a Grid/Table view for status.
*   **Engine:** `reqwest` + `readability` implementation.
*   **Features:**
    *   Parallel downloads.
    *   Real-time token counting.
    *   Sanitized filename generation.
    *   Single-file concatenation export.
    *   YAML Frontmatter metadata.
    *   Visual error marking for failed downloads (403, 404).

### Phase 2: Enhanced User Experience
*   **Manual Link Vetting:** After a page is downloaded, parse all `<a href>` tags within the content. Allow the user to right-click a row and "View Links", then select references to add to the queue. (Strictly manual selection, no auto-recursion).
*   **Resume Capability:** Save the queue state to disk on exit so the user doesn't lose progress if the app closes.

### Phase 3: Advanced Access (Post-MVP)
*   **Cookie Import:** Add a UI dialog to paste a cookie string (e.g., `Netscape` format or raw header) to bypass paywalls.
*   **PDF Support:** Integrate `pdf-extract` to handle URLs ending in `.pdf`.
*   **Stealth Mode:** Replace `reqwest` with `reqwest-impersonate` to handle sites that block standard generic HTTP clients (Cloudflare protection).

## 8. Testing Strategy

1.  **Isolation:** Do *not* run unit tests against live websites (Google, Wikipedia) as they change or rate-limit.
2.  **Mocking:** Use **`wiremock`**. Spin up a local HTTP server within the test suite that serves pre-baked HTML files.
3.  **Pipeline Verification:**
    *   Create a folder of sample HTML files (messy code, ads, popups).
    *   Run the `Readability -> Markdown` conversion.
    *   Assert that the output Markdown matches a "Golden Master" reference file.
4.  **Sanitization Tests:** Fuzz the filename sanitizer with strings containing OS-illegal characters and Emoji to ensure no crashes or invalid paths.
