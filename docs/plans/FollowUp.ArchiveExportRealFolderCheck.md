# Follow-up: confirm the archive export fix on the real output folder

Status: waiting for Lars's next Archive press after he restarts the desktop app on the new
build. The fix itself is commit 6aa6e2b, "Follow-up (no plan): archive export reads only what it
cannot rule out, with a visible notice".

## Why

On 2026-11-26 at 05:44 an export spent 93 s in `phase=read-articles`, reading all 17,981 root
articles at about 5 ms each to keep 662 window files. Fixture benchmarks cannot reproduce
that per-file cost, because fresh temp files read at about 0.11 ms. So the speed-up is proven
only by the first real export after the fix.

## What to read

Search `engine.log` in the repository root for the newest `request_id` of the session:

1. `[archive-dialog] click received request_id=N gate=allowed`
2. `[archive-dialog] dialog data ready request_id=N ...`
3. `[archive-dialog] shown request_id=N ...`
4. `[archive-export] files=<total> ...` and the `phase=read-articles scanned=...` lines
5. `[archive-export] hints=<n> hints_wait_ms=<ms> files_read=<n> files_skipped=<n> request_id=N`
6. `[archive-export] phase=select-documents ... elapsed_ms=<ms>`
7. `[archive-dialog] completion delivered request_id=N elapsed_ms=<ms>`

## Expected figures

- Lines 1 to 3 have the same timestamp to the second, so opening the dialog is not the delay.
  If there is a gap of several seconds, the open path is slow. Report it and do not guess.
- `hints` is close to the total number of root files, about 18,000.
- `files_read` is about 700: the window since the last checkpoint, plus new, changed or
  undated files and non-articles.
- `files_skipped` is about `files - files_read`, about 17,300.
- `hints_wait_ms` is near 0. A large value means the press happened during the startup scan,
  and the export waited for it. That wait is correct but slow, so report it.
- Each skipped file still costs one metadata call, so `phase=select-documents elapsed_ms` and
  `completion delivered elapsed_ms` should be a few seconds instead of about 93,000 ms.
- Lars sees "Exporting archive...", then "Reading articles N of M", then
  "archive.md written: N articles".

## If it does not match

- `hints=0` means the scan index was empty, for example when the press came before the first
  triage load. Then the export did a full read and the time is unchanged. That is correct but
  not the fix. Check whether `LoadArticlesForTriage` ran first.
- A `files_read` far above 700 with a normal `hints` count means fingerprints did not match.
  Something rewrote the files or their modification times. Investigate before changing the
  skip rule.
- Never relax the per-file proof for speed. See docs/DecisionLog.md, 2026-11-26.
