The supported desktop system has one Tauri host; the retired native desktop host
is no longer part of the build or runtime.

Structured threat model for the batch host and the Tauri desktop host, covering:

1. **Assets**: downloaded content, output files, persisted state, API keys stored in the Windows Credential Manager entries `Harvester/OpenAI` and `Harvester/BraveSearch`, user's system
2. **Trust boundaries**:
   - User input (semi-trusted)
   - Downloaded web content (untrusted)
   - Persisted state (untrusted for side effects — may be hand-edited or corrupted)
   - LLM API responses (untrusted, Phase 1+)
   - Stored API keys (confidential). The fixed launcher flag `--use-stored-keys` is Harvester's boundary for requesting the entries. Windows controls access to the entries by account.
3. **Threat categories** with mitigations:
   - SSRF → URL policy module (Part 4)
   - Content injection (frontmatter) → sanitization (Part 2)
   - Path traversal → output directory confinement (Part 1)
   - Denial-of-wallet (LLM cost) → quotas (Part 6, expanded in Phase 1)
   - Prompt injection via article content → nonce-delimited rendering, DTO validation, replay auditing (Phase 1+)
   - Data exfiltration via prompt injection (LLM output leak) → strict validation, result gating, replay records for forensic review
   - Resource exhaustion → per-run download and LLM quotas, per-URL limits
   - Secret inheritance into agent-controlled processes. Keys from Credential Manager are no longer inherited through environment variables. Direct reads by code running as Lars are an accepted residual risk. Harvester-only keys with provider spending limits reduce the impact of exposure.
   - Agent-modified launcher, helper, or application code → accepted residual risk: an agent can change source that the user subsequently runs with real keys. The user accepts this because code changes are reviewable and reviewed before they are run, whereas environment inheritance is invisible and automatic.
   - Persistent parent key variables. Harvester ignores `OPENAI_API_KEY` and `BRAVE_SEARCH_API_KEY` regardless of their scope. The launcher warns once for each one it finds and suggests removing it if no other application needs it. Existing values can still be inherited by child processes from the caller.
   - Unattended credential access. Any process running as Lars, including an LLM coding agent, can read these entries directly without a human gate. The only remaining guard against an agent reading them is the `Agents.md` rule that agents never obtain API keys.
   - Long-lived key-bearing research service → the corpus MCP server is gone, so no long-lived key-bearing process answers agent questions.
   - Desktop IPC → the shipped Tauri page is an untrusted boundary. It may send only `UiIntent`, decoded fail-closed; it cannot name reducer result messages or arbitrary URLs. Snapshot bodies are served only from the last derived `BodyTable`.
   - Desktop assets → the desktop window never starts a dev server or loads remote content. `harvester://` resolves only confined files beneath the built bundle and carries the bridge CSP.
4. **System invariants**:
   - Untrusted content is never interpolated into structured formats without sanitization
   - Persisted data is untrusted input for side effects
   - All model calls are synchronous; no Batch API submission or collection path remains.
   - LLM outputs are advisory and tainted until validated. Replay records are write-only forensics: the synchronous model path writes them for review, and no provider lookup reads them to satisfy requests.
   - Harvester reads API keys from Credential Manager only under the launcher flag. Agents use the key-free paths for builds, tests and diagnostics.
   - Key values stay out of source, files, environments, logs, Debug output and user-facing messages. OpenAI and Brave remove key echoes from provider error details before those details reach logs or the UI.
   - Side effects require passing through `EffectRunner` policy checks; runtime
     persistence is a reducer-emitted effect whose snapshot is captured during
     update, never by a host reading state out of band
   - All resource consumption is bounded
   - Probe mode is a scoped diagnostic exemption: it constructs no `AppState`, `RuntimePaths`, effects, or secrets, and only adds its acknowledgement/report IPC commands in that mode. It also ships the permanently embedded `probe-*` capability window-label glob in `crates/harvester_ui/capabilities/default.json`; this does not widen page authority because `core:default` grants no window creation, closing, or destruction, the only labels created in code are production `main` and `probe-<slug>` inside `run_probe`, and embedded capabilities cannot be conditional on `--probe-ipc`.
5. **Lessons learned** (from review):
   - Duplicate IO paths create policy drift; centralize enforcement
   - Generic failure collapsing removes traceability
   - Byte slicing of user/content strings is brittle; use char-boundary-safe helpers

Extracted-link storage derives filenames solely from the SHA-256 of the canonical
archive URL key. URL text and retired downloaded paths never enter filesystem paths.
The store directory must canonicalize to the output root's own `.article_links`
child; redirected directories and link-file symlinks are refused. Atomic link
temporaries stay inside the validated store directory. Malformed files or individual
records are logged and skipped. Link selection requests I/O through a reducer effect;
only a successful reply for the still-selected matching successful job fills its links.
Empty replies cannot clear resident links. Opening a link
continues to resolve the core-owned index, never a page-supplied URL. Runtime-state
migration never replaces the original before a flushed, length-and-SHA-256-verified
backup exists, and never overwrites an existing backup.
