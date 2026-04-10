# Visual Spec UI Polish Implementation Plan

> **For agentic workers:** If your environment provides them, the `superpowers:subagent-driven-development` or `superpowers:executing-plans` skills are recommended for task-by-task execution with review checkpoints. These skills are optional — the plan is self-contained and can be executed with any agent or by hand. Steps use checkbox (`- [ ]`) syntax for tracking progress.

**Goal:** Bring the Harvester UI closer to `docs/visual_design/VisualDesignSpec.md` by fixing priority-badge color mapping, warming cool neutrals, rendering triage tags as pill-style chips, and (optionally) polishing the reading pane, row metadata, selection emphasis, and secondary tab strip.

**Architecture:** This is a pure presentation-layer change. All edits live under `crates/harvester_app/src/platform/ui/` (style/render/markdown-to-RTF) and `crates/harvester_core/src/preview.rs` (markdown formatter). No reducer logic, no new effects, no new state shape. The unidirectional data flow stays intact: reducers emit the same view model; only how the view is styled and serialized changes.

**Tech Stack:** Rust, Cargo workspace. Existing `pulldown_cmark` → RTF converter in [markdown_to_rtf.rs](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs). Styles defined as `PlatformCommand::DefineStyle` commands in [layout.rs](../../crates/harvester_app/src/platform/ui/layout.rs). Unit tests in [render_tests.rs](../../crates/harvester_app/src/platform/ui/render_tests.rs) and inline `#[cfg(test)]` modules in `preview.rs` / `markdown_to_rtf.rs`.

## Background — what's wrong and why

Reviewing a screenshot of the running app against the spec surfaced four priority issues and four polish issues.

### The four priority issues

1. **Priority badges collapse into two buckets.** The triage prompt at [triage.rs:23](../../crates/harvester_engine/src/llm/prompts/triage.rs#L23) asks the model for a score from **1 (lowest) to 5 (highest)**. But the render mapping at [render.rs:1833-1837](../../crates/harvester_app/src/platform/ui/render.rs#L1833-L1837) is:

   ```rust
   0..=3 => StyleId::BadgePriorityLow,
   4     => StyleId::BadgePriorityMedium,
   5     => StyleId::BadgePriorityHigh,
   _     => StyleId::BadgePriorityCritical,
   ```

   So P1, P2, P3 all render as the same gray "Low" pill, P4 as purple "Medium," and P5 as amber "High." `Critical` (red) is unreachable on the real scale. The spec (lines 66, 189 of VisualDesignSpec.md) explicitly calls out priority color as a scan accelerator — the current collapse defeats that.

2. **`BadgePriorityLow` is a cool blue-gray.** [layout.rs:1306-1316](../../crates/harvester_app/src/platform/ui/layout.rs#L1306-L1316) sets its background to `#565C66`, which reads as blue-gray. The spec bans cool blue-grays (VisualDesignSpec.md line 275); all neutrals must carry warm yellow-brown undertones.

3. **Tags are a comma-joined string, not pills.** [preview.rs:60](../../crates/harvester_core/src/preview.rs#L60) writes `Tags: {}` with `result.tags.join(", ")`. The spec (line 191) says "Prefer badges or pills over dense bracketed inline metadata." The reading pane is a rich edit control fed RTF by [markdown_to_rtf.rs](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs), and the converter currently treats inline code spans as plain text ([markdown_to_rtf.rs:53](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs#L53)). Adding character-level background shading to inline code runs gives us a "chip" look inside the existing rich edit, without introducing new platform controls.

4. **The preview tag line uses plain text even when the RTF engine could style it.** Once (3) adds shaded inline code support, the triage formatter needs to wrap each tag in backticks so the RTF converter renders them as chips.

### The four polish issues (optional, after the priority fixes)

5. **Reading pane has a visible outer border**, fighting the spec's "use tonal contrast, avoid heavy box outlines" guidance (VisualDesignSpec.md lines 127-131).
6. **Row metadata duplicates the domain.** Row line 1 is a URL starting with `www.foo.com/...`; row line 2 is `www.foo.com · N tags`. The domain appears twice.
7. **Selection emphasis is subtle.** Only a faint background shift plus an accent-colored left edge. Spec line 178 asks for "strong selected-state styling using Accent Primary."
8. **Right-pane secondary tab strip** has the same visual weight as the top-level tabs, creating two competing tab systems.

---

# Part A — Recommended changes (do these first)

## Task 1: Remap `triage_priority_style` to span the 1-5 scale

**Files:**
- Modify: [crates/harvester_app/src/platform/ui/render.rs:1826-1838](../../crates/harvester_app/src/platform/ui/render.rs#L1826-L1838)
- Modify: [crates/harvester_app/src/platform/ui/render_tests.rs:251-269](../../crates/harvester_app/src/platform/ui/render_tests.rs#L251-L269)

**Rationale:** The model produces scores 1..=5. With four available badge styles (Low / Medium / High / Critical), the best mapping gives the rare top-of-scale items the most attention: P1→Low, P2→Low, P3→Medium, P4→High, P5→Critical. P1 and P2 both sit at "low urgency" — collapsing them is fine, collapsing P1..P3 is not because P3 is the statistical middle.

- [ ] **Step 1.1: Update the existing priority badge test to pin the new contract**

Open [render_tests.rs](../../crates/harvester_app/src/platform/ui/render_tests.rs). Find the test `triage_results_items_show_priority_and_category_badges` near line 251. Replace the single-priority assertion with parameterized coverage of the full 1..=5 scale. The replacement test body:

```rust
#[test]
fn triage_results_priority_badge_maps_full_scale() {
    // The triage prompt returns priority 1 (lowest) to 5 (highest/most urgent).
    // See crates/harvester_engine/src/llm/prompts/triage.rs.
    let cases: &[(u8, StyleId)] = &[
        (1, StyleId::BadgePriorityLow),
        (2, StyleId::BadgePriorityLow),
        (3, StyleId::BadgePriorityMedium),
        (4, StyleId::BadgePriorityHigh),
        (5, StyleId::BadgePriorityCritical),
    ];

    for (priority, expected_style) in cases {
        let mut job = make_job(1, "https://example.com", Stage::Done, None, None, None);
        job.summary_title = Some("Example headline".to_string());
        job.triage_annotation = Some(harvester_core::TriageAnnotationView {
            priority: *priority,
            category: "business".to_string(),
            tags: vec!["tag-a".to_string()],
        });

        let item = build_list_box_item(LeftTab::TriageResults, &job);

        assert_eq!(item.badges.len(), 2, "priority {priority}");
        assert_eq!(item.badges[0].text, format!("P{priority}"));
        assert_eq!(
            item.badges[0].style, *expected_style,
            "priority {priority} should map to {:?}",
            expected_style
        );
        assert_eq!(item.badges[1].text, "Business");
        assert_eq!(item.badges[1].style, StyleId::BadgeCategory);
    }
}
```

Remove the old `triage_results_items_show_priority_and_category_badges` test — the new one supersedes it.

- [ ] **Step 1.2: Run the test to confirm it fails**

Run: `cargo test -p harvester_app --lib triage_results_priority_badge_maps_full_scale`
Expected: FAIL — priority 1 is expected to map to `BadgePriorityLow` (matches) but priority 3 currently maps to `BadgePriorityLow` while the test expects `BadgePriorityMedium`; priority 5 currently maps to `BadgePriorityHigh` while the test expects `BadgePriorityCritical`.

- [ ] **Step 1.3: Implement the new mapping**

Edit [render.rs:1826-1838](../../crates/harvester_app/src/platform/ui/render.rs#L1826-L1838). Replace the body of `triage_priority_style` with:

```rust
fn triage_priority_style(job: &JobRowView) -> StyleId {
    // Triage prompt (crates/harvester_engine/src/llm/prompts/triage.rs) asks the
    // model for priority 1 (lowest) through 5 (highest/most urgent). We have four
    // badge styles, so P1 and P2 share the muted "Low" pill and P3..P5 each get
    // their own color to accelerate scan speed on the high-urgency tail.
    match job
        .triage_annotation
        .as_ref()
        .map(|triage| triage.priority)
        .unwrap_or_default()
    {
        0..=2 => StyleId::BadgePriorityLow,
        3 => StyleId::BadgePriorityMedium,
        4 => StyleId::BadgePriorityHigh,
        _ => StyleId::BadgePriorityCritical,
    }
}
```

Note: priority 0 is impossible per the prompt contract but we keep it in the `Low` bucket for safety (the default for a missing annotation is `u8::default() == 0`).

- [ ] **Step 1.4: Re-run the test to confirm it passes**

Run: `cargo test -p harvester_app --lib triage_results_priority_badge_maps_full_scale`
Expected: PASS.

- [ ] **Step 1.5: Run the full crate test suite to catch incidental breakage**

Run: `cargo test -p harvester_app`
Expected: all green. If another test pinned an old priority→style expectation, update it to match the new mapping.

- [ ] **Step 1.6: Run clippy and fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: no warnings, no diff after fmt.

- [ ] **Step 1.7: Commit**

```bash
git add crates/harvester_app/src/platform/ui/render.rs \
        crates/harvester_app/src/platform/ui/render_tests.rs
git commit -m "Remap priority badge styles to span the 1-5 triage scale"
```

---

## Task 2: Warm the `BadgePriorityLow` background

**Files:**
- Modify: [crates/harvester_app/src/platform/ui/layout.rs:1306-1316](../../crates/harvester_app/src/platform/ui/layout.rs#L1306-L1316)

**Rationale:** `#565C66` is a cool blue-gray. The spec's warm-neutral palette uses `#2a2a28`, `#30302e`, `#3d3d3a` for subdued surfaces. Picking `#3d3d3a` (the spec's "Surface Overlay" token) as the pill background lets the Low badge sit in the same neutral family as dropdowns and tooltips, removing the alien cool tone without making the pill louder. The foreground swaps from the current cool near-white `#F0F3F5` to the spec's warm Text Secondary `#B0AEA5`, which also reduces the contrast and visually demotes low-priority items.

- [ ] **Step 2.1: Edit the `BadgePriorityLow` color literal**

In [layout.rs](../../crates/harvester_app/src/platform/ui/layout.rs) find the block that begins at line 1305 with `StyleId::BadgePriorityLow,`. Replace the whole tuple with:

```rust
        (
            StyleId::BadgePriorityLow,
            Color {
                r: 0x3D,
                g: 0x3D,
                b: 0x3A,
            },
            Color {
                r: 0xB0,
                g: 0xAE,
                b: 0xA5,
            },
        ),
```

- [ ] **Step 2.2: Build and run the crate tests**

Run: `cargo build -p harvester_app && cargo test -p harvester_app`
Expected: all green. (No test asserts the exact RGB values, so this is a pure visual change.)

- [ ] **Step 2.3: Run clippy and fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 2.4: Commit**

```bash
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "Warm BadgePriorityLow to match the spec's neutral palette"
```

---

## Task 3: Shade inline code runs in the RTF converter (enables tag pills)

**Files:**
- Modify: [crates/harvester_app/src/platform/ui/markdown_to_rtf.rs](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs) (color table, `Event::Code` handling, add two small helpers)

**Rationale:** The reading pane is a rich edit control whose content is RTF produced by `convert_markdown_to_rtf`. Today, `Event::Text` and `Event::Code` are handled identically (line 53), so inline backtick spans render as plain text. RTF supports character-level background shading via `\chshdng10000\chcbpatN` (pattern percentage + character background pattern color, indexed into the color table). By adding a fifth color to the `colortbl` — a chip background matching the spec's "Surface Overlay" `#3d3d3a` — and wrapping inline code runs with that shading, backtick spans render as small chips. This gives us tag pills in Task 4 without introducing new platform controls or touching the layout tree.

This task is test-driven: the existing tests in `markdown_to_rtf.rs` pin exact output for several cases and give us a template.

- [ ] **Step 3.1: Read the existing tests to understand the test style**

Run: open [markdown_to_rtf.rs:243-393](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs#L243-L393) and skim the `#[cfg(test)]` module. You'll see tests that call `convert_markdown_to_rtf(...)` and assert substrings appear (or don't) in the output.

- [ ] **Step 3.2: Add a failing test for inline code shading**

Append this test inside the existing `#[cfg(test)] mod tests { ... }` block at the end of `markdown_to_rtf.rs`:

```rust
#[test]
fn inline_code_gets_character_background_shading() {
    let rtf = convert_markdown_to_rtf("a `chip` b");
    // The new chip background color must be declared in the color table.
    // `\red61\green61\blue58;` corresponds to #3D3D3A (spec's Surface Overlay).
    assert!(
        rtf.contains("\\red61\\green61\\blue58;"),
        "color table should include the chip background color; got: {rtf}"
    );
    // The inline code run must be wrapped in character shading control words.
    // We expect \chshdng10000\chcbpat<N> to enable, and \chshdng0\chcbpat0 to reset.
    assert!(
        rtf.contains("\\chshdng10000"),
        "inline code should enable character shading; got: {rtf}"
    );
    assert!(
        rtf.contains("\\chshdng0\\chcbpat0"),
        "inline code should reset character shading after the run; got: {rtf}"
    );
    // The chip text itself must still appear verbatim.
    assert!(rtf.contains("chip"));
}
```

- [ ] **Step 3.3: Run the test to confirm it fails**

Run: `cargo test -p harvester_app --lib inline_code_gets_character_background_shading`
Expected: FAIL — the assertions about `\red61\green61\blue58;`, `\chshdng10000`, and the reset sequence don't match the current output.

- [ ] **Step 3.4: Add the chip background color to the color table**

Edit [markdown_to_rtf.rs:15-30](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs#L15-L30). Add a new constant under the existing color constants:

```rust
const COLOR_BODY_TEXT_RTF: &str = "\\red176\\green174\\blue165;";
const COLOR_HEADING_TEXT_RTF: &str = "\\red250\\green249\\blue245;";
const COLOR_BACKGROUND_RTF: &str = "\\red48\\green48\\blue46;";
const COLOR_LINK_RTF: &str = "\\red217\\green119\\blue87;";
const COLOR_CHIP_BG_RTF: &str = "\\red61\\green61\\blue58;}";
```

Note two things:
1. `COLOR_LINK_RTF` previously ended in `}` to close the color table. That trailing `}` moves to the new last entry `COLOR_CHIP_BG_RTF`. Remove the `}` from `COLOR_LINK_RTF` in the same edit.
2. The chip background is index 5 in the color table (1=body, 2=heading, 3=background, 4=link, 5=chip — the table uses 1-based indexing per RTF spec, and we push them in that order).

Also update the `convert_markdown_to_rtf` preamble that pushes the color table so the new color appears after `COLOR_LINK_RTF`. Find this block near [markdown_to_rtf.rs:26-30](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs#L26-L30):

```rust
    rtf.push_str("{\\colortbl;");
    rtf.push_str(COLOR_BODY_TEXT_RTF);
    rtf.push_str(COLOR_HEADING_TEXT_RTF);
    rtf.push_str(COLOR_BACKGROUND_RTF);
    rtf.push_str(COLOR_LINK_RTF);
```

Replace with:

```rust
    rtf.push_str("{\\colortbl;");
    rtf.push_str(COLOR_BODY_TEXT_RTF);
    rtf.push_str(COLOR_HEADING_TEXT_RTF);
    rtf.push_str(COLOR_BACKGROUND_RTF);
    rtf.push_str(COLOR_LINK_RTF);
    rtf.push_str(COLOR_CHIP_BG_RTF);
```

- [ ] **Step 3.5: Wrap `Event::Code` with character shading**

Edit [markdown_to_rtf.rs:53](../../crates/harvester_app/src/platform/ui/markdown_to_rtf.rs#L53). The current line collapses `Text` and `Code` into one arm:

```rust
            Event::Text(text) | Event::Code(text) => escape_rtf_text(&mut rtf, text.as_ref()),
```

Split them:

```rust
            Event::Text(text) => escape_rtf_text(&mut rtf, text.as_ref()),
            Event::Code(text) => {
                // Render inline code as a shaded chip (tag pill style).
                // \chshdng10000 = full character shading pattern; \chcbpat5 = use
                // chip background color (index 5 in the color table).
                rtf.push_str("\\chshdng10000\\chcbpat5 ");
                escape_rtf_text(&mut rtf, text.as_ref());
                rtf.push_str("\\chshdng0\\chcbpat0 ");
            }
```

- [ ] **Step 3.6: Re-run the new test to confirm it passes**

Run: `cargo test -p harvester_app --lib inline_code_gets_character_background_shading`
Expected: PASS.

- [ ] **Step 3.7: Run the whole `markdown_to_rtf` test module to catch regressions**

Run: `cargo test -p harvester_app --lib markdown_to_rtf::tests`
Expected: all green. The older tests all use `Event::Text` content, so they shouldn't be affected, but one of them (`"```code```"` near line 373) exercises the fenced code path — make sure it still passes. If a fenced-code test fails because it was formerly hitting `Event::Code`, update the assertion to include the shading control words.

- [ ] **Step 3.8: Run the full crate test suite**

Run: `cargo test -p harvester_app`
Expected: all green.

- [ ] **Step 3.9: Run clippy and fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 3.10: Commit**

```bash
git add crates/harvester_app/src/platform/ui/markdown_to_rtf.rs
git commit -m "Render inline code as shaded chips in preview RTF"
```

---

## Task 4: Emit each triage tag as a sanitized, backticked chip

**Files:**
- Modify: [crates/harvester_core/src/preview.rs:59-62](../../crates/harvester_core/src/preview.rs#L59-L62) (formatter)
- Modify: [crates/harvester_core/src/preview.rs:178-195](../../crates/harvester_core/src/preview.rs#L178-L195) (`triage_formatter_produces_stable_markdown` test)
- Add: a new private helper `sanitize_tag_for_chip` + regression tests inside the `#[cfg(test)]` module at the bottom of `preview.rs`
- Check: [crates/harvester_core/src/state/tests.rs:1273](../../crates/harvester_core/src/state/tests.rs#L1273) (may need an assertion update)

**Rationale:** After Task 3, the RTF converter renders `` `foo` `` as a chip. Emitting each tag wrapped in backticks instead of joined by commas makes the preview tag line render as a row of small chips, matching VisualDesignSpec.md line 191 ("Prefer badges or pills").

**Sanitization (from code review):** Triage tags come from LLM output. Current validation enforces string type, max length, and max item count, but does not reject or normalize markdown-significant characters. A tag containing a backtick `` ` `` would close the inline code span early (producing plain text + a stray backtick), and a tag containing `\n` or `\r` would break the whole paragraph. We must sanitize tag text before wrapping it in backticks. The sanitizer lives in `preview.rs` (not in the tag validator) because the escaping concern is specific to the markdown→RTF chip transport, not to tag storage.

Sanitization rules:
- Replace any backtick `` ` `` with U+2032 PRIME `′` (a visually similar character that does not close markdown code spans). This preserves intent when the model produced an ASCII apostrophe-like glyph.
- Replace any CR/LF with a single space (chips cannot span lines).
- Trim surrounding whitespace. Drop the tag entirely if it becomes empty after trimming.

- [ ] **Step 4.1: Add failing tests for the formatter (happy path + adversarial inputs)**

Open [preview.rs:178-195](../../crates/harvester_core/src/preview.rs#L178-L195). Replace the assertion at line 191:

```rust
        assert!(formatted.contains("Tags: vulnerability, zero-day"));
```

with:

```rust
        // Each tag is wrapped in a backtick span so the RTF preview renders
        // them as shaded chips (see markdown_to_rtf::convert_markdown_to_rtf).
        assert!(formatted.contains("Tags: `vulnerability` `zero-day`"));
```

Then append these three new regression tests inside the same `#[cfg(test)] mod tests { ... }` block:

```rust
#[test]
fn triage_formatter_sanitizes_backticks_in_tag_text() {
    // A tag containing a literal backtick must not be allowed to close the
    // inline code span. Verify the output contains no adjacent backticks
    // (which would indicate a broken span) and that the sanitized form is
    // present.
    let result = ArticleTriageResult {
        category: "Security".to_string(),
        priority: 3,
        tags: vec!["back`tick".to_string(), "clean".to_string()],
        rationale: "test".to_string(),
        input_tokens: 0,
        output_tokens: 0,
    };
    let formatted = format_triage_for_preview(Some("t"), &result);
    // No "``" — that would mean a code span got closed and reopened.
    assert!(
        !formatted.contains("``"),
        "formatter must not emit adjacent backticks; got: {formatted}"
    );
    // Sanitized form: backtick replaced with U+2032 PRIME.
    assert!(formatted.contains("`back\u{2032}tick`"));
    assert!(formatted.contains("`clean`"));
}

#[test]
fn triage_formatter_sanitizes_newlines_in_tag_text() {
    let result = ArticleTriageResult {
        category: "Security".to_string(),
        priority: 3,
        tags: vec!["line1\nline2".to_string(), "tab\rreturn".to_string()],
        rationale: "test".to_string(),
        input_tokens: 0,
        output_tokens: 0,
    };
    let formatted = format_triage_for_preview(Some("t"), &result);
    // The Tags: line stays on a single line; no stray CR/LF leaks into a chip.
    let tags_line = formatted
        .lines()
        .find(|line| line.starts_with("Tags:"))
        .expect("Tags: line present");
    assert!(!tags_line.contains('\r'));
    assert!(tags_line.contains("`line1 line2`"));
    assert!(tags_line.contains("`tab return`"));
}

#[test]
fn triage_formatter_drops_empty_tags_after_sanitization() {
    let result = ArticleTriageResult {
        category: "Security".to_string(),
        priority: 3,
        tags: vec!["  ".to_string(), "keep".to_string(), "\n\n".to_string()],
        rationale: "test".to_string(),
        input_tokens: 0,
        output_tokens: 0,
    };
    let formatted = format_triage_for_preview(Some("t"), &result);
    let tags_line = formatted
        .lines()
        .find(|line| line.starts_with("Tags:"))
        .expect("Tags: line present");
    // Only the non-empty sanitized tag survives.
    assert_eq!(tags_line.trim(), "Tags: `keep`");
}
```

- [ ] **Step 4.2: Run the tests to confirm they fail**

Run: `cargo test -p harvester_core --lib preview::tests`
Expected: FAIL — the current output still uses the comma-joined form and no sanitizer exists.

- [ ] **Step 4.3: Add the `sanitize_tag_for_chip` helper**

Inside `preview.rs`, add a private helper next to `title_case_label` (near the bottom of the file, still inside the `pub mod` / file scope but outside `#[cfg(test)]`):

```rust
/// Prepare an LLM-produced tag string for rendering as a markdown inline
/// code chip. Strips backticks (which would close the span) and CR/LF
/// (which would break the paragraph), and trims surrounding whitespace.
/// Returns `None` if nothing remains.
fn sanitize_tag_for_chip(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|ch| match ch {
            '`' => '\u{2032}', // U+2032 PRIME — visually similar, inert in markdown
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
```

- [ ] **Step 4.4: Update the formatter to use sanitized chips**

Edit [preview.rs:59-62](../../crates/harvester_core/src/preview.rs#L59-L62). Replace:

```rust
    if !result.tags.is_empty() {
        let _ = writeln!(out, "Tags: {}", result.tags.join(", "));
        out.push('\n');
    }
```

with:

```rust
    let chips: Vec<String> = result
        .tags
        .iter()
        .filter_map(|tag| sanitize_tag_for_chip(tag))
        .map(|tag| format!("`{tag}`"))
        .collect();
    if !chips.is_empty() {
        // Each chip is an inline code span so the RTF renderer shows them
        // as shaded chips in the preview pane. See sanitize_tag_for_chip
        // for why tag text must be normalized before wrapping in backticks.
        let _ = writeln!(out, "Tags: {}", chips.join(" "));
        out.push('\n');
    }
```

Note: the emptiness check moved from the raw `result.tags` to the post-sanitize `chips` so that a tag list of only whitespace correctly skips the Tags line.

- [ ] **Step 4.5: Re-run the formatter tests**

Run: `cargo test -p harvester_core --lib preview::tests`
Expected: PASS — the baseline test, all three adversarial regression tests, and any existing preview tests all green.

- [ ] **Step 4.6: Check for indirect assertions and update them**

Run: `cargo test -p harvester_core`
Expected: all green. If [state/tests.rs:1273](../../crates/harvester_core/src/state/tests.rs#L1273) (or any other test) has an assertion like `assert!(content.contains("Tags: vulnerability, zero-day"))`, update it to match the new format: `` `vulnerability` `zero-day` ``.

Run the whole workspace tests:

Run: `cargo test`
Expected: all green.

- [ ] **Step 4.7: Run clippy and fmt**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`
Expected: clean.

- [ ] **Step 4.8: Commit**

```bash
git add crates/harvester_core/src/preview.rs \
        crates/harvester_core/src/state/tests.rs
git commit -m "Render triage tags as sanitized chip-style inline code spans"
```

- [ ] **Step 4.9: Manual smoke test (visual verification)**

Run: `cargo run -p harvester_app`

In the running app, select any already-triaged article in the Triage Results tab and look at the right pane's Triage tab viewer. Expected:
- The priority-category line still reads e.g. `Business · Priority P4`.
- Below it, the `Tags:` line shows each tag with a visible shaded background behind it, instead of a flat comma-separated string.
- The chip background should be a warm dark gray, not blue.

- [ ] **Step 4.10: Update the engineering diary**

Append a new entry to [docs/EngineeringDiary.md](../EngineeringDiary.md). Keep it short, per Agents.md guidance. Example body:

```markdown
## 2026-04-10 — Visual spec polish (priority badges + tag chips)

Closed three VisualDesignSpec gaps in the reading and list panes:
- `triage_priority_style` now spans the 1-5 triage scale (render.rs).
- `BadgePriorityLow` warmed to the spec's Surface Overlay neutral (layout.rs).
- Triage preview tags render as shaded chips via new inline-code RTF shading
  (markdown_to_rtf.rs + preview.rs).
See docs/visual_design/Plan.visual-spec-ui-polish.md for the full rationale.
```

- [ ] **Step 4.11: Commit the diary entry**

```bash
git add docs/EngineeringDiary.md
git commit -m "Diary: visual spec polish (priority badges + tag chips)"
```

---

### Part A — done. Verify before moving on.

- [ ] Run `cargo test` — all green.
- [ ] Run `cargo clippy --all-targets -- -D warnings && cargo fmt` — clean.
- [ ] Run the app and visually confirm priority badges now have three distinct colors (for priorities 3, 4, 5) and tags in the Triage preview render as shaded chips.

Part B below is independent — each optional task is self-contained. You can stop after Part A if the result already meets the bar.

---

# Part B — Optional polish (do later if Part A doesn't feel sufficient)

Each task here is independent and can be implemented in any order or skipped entirely. Unlike Part A, several require exploration to pin down exact line numbers, so each task starts with an "investigate" step.

## Task 5: Soften the reading pane's outer border

**Files:**
- Investigate, then modify: [crates/harvester_app/src/platform/ui/layout.rs](../../crates/harvester_app/src/platform/ui/layout.rs)

**Rationale:** The reading pane currently has a visible 1px border around the whole RichEdit container, making it read as "boxed." Spec lines 127-131 say to prefer tonal contrast over heavy box outlines. The pane already sits on `Surface Raised` (a lighter tonal step) so the border is redundant in most places.

- [ ] **Step 5.1: Find the reading pane container style**

Run:
```
Grep pattern="VIEWER_PREVIEW|reading_pane|preview_container" path=crates/harvester_app/src/platform/ui/layout.rs
```

Look for a `DefineStyle` block that sets a `border` field on the preview viewer or its container. Note the line numbers.

- [ ] **Step 5.2: Decide: drop the border entirely or subdue it**

If the container's background already uses `Surface Raised` (`#30302e`-ish) and adjacent surfaces use `Surface` (`#1e1e1c`-ish), the tonal step alone should be enough — drop the border entirely. If the adjacent surfaces are the same tone, keep a 1px border but change its color to `Border Subtle` (`#2a2a28`) so it becomes a hint rather than a frame.

- [ ] **Step 5.3: Apply the change**

Edit the `DefineStyle` block you found. Either set the border width to 0, or change the border color from `Border Default` (`#30302e`) to `Border Subtle` (`#2a2a28`). Show the exact literal change in the commit message.

- [ ] **Step 5.4: Build, test, visually verify**

Run: `cargo build -p harvester_app && cargo run -p harvester_app`

Expected: reading pane no longer reads as a "box." The list pane / reading pane separation still reads clearly due to tonal contrast. If it doesn't, revert.

- [ ] **Step 5.5: clippy, fmt, commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo fmt`

```bash
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "Subdue reading pane outer border in favor of tonal contrast"
```

---

## Task 6: Dedup row metadata (drop the redundant domain)

**Files:**
- Modify: [crates/harvester_app/src/platform/ui/render.rs](../../crates/harvester_app/src/platform/ui/render.rs) (the function that builds row metadata — near `job_source_label` around line 1849)
- Modify: [crates/harvester_app/src/platform/ui/render_tests.rs](../../crates/harvester_app/src/platform/ui/render_tests.rs) (tests at lines 226, 248, 268 that pin `"example.com · …"` metadata strings)

**Rationale:** In the triage list, row line 1 is a URL like `www.foo.com/investing/.../amazon-...`. Row line 2 is `www.foo.com · 9 tags`. The domain appears twice. Dropping it from the metadata line reclaims visual space and reduces duplication. Where tags or timestamps would otherwise make the metadata line empty (e.g. Jobs tab), keep the domain.

- [ ] **Step 6.1: Locate the metadata builders**

Run:
```
Grep pattern="metadata.*=|fn.*metadata|source_label" path=crates/harvester_app/src/platform/ui/render.rs
```

Identify the function(s) that build `ListBoxItemView::metadata` for each `LeftTab` variant. There may be one per tab.

- [ ] **Step 6.2: Update the failing test assertions first**

After Task 1 has run, the test that previously asserted `"example.com · 1 tag"` at render_tests.rs:268 no longer exists (it was replaced by the parameterized `triage_results_priority_badge_maps_full_scale`, which does not assert metadata). The tests that still pin metadata strings are:

- `triage_review_items_show_indirect_badge_and_disabled_state` — asserts `item.metadata == "Security · example.com"` (originally render_tests.rs:248).
- The Jobs-tab test asserting `"example.com · 100 · 2.0 KB"` (originally render_tests.rs:226).

Decide which tabs should drop the domain and which shouldn't. A reasonable split:

- **Jobs tab:** keep — domain is primary context here, metadata stays `"example.com · 100 · 2.0 KB"`.
- **Triage Review:** drop — category badge already orients the reader, metadata becomes `"Security"`.
- **Triage Results:** add a metadata assertion to the Task 1 parameterized test expecting `"1 tag"` (the post-fix expectation) so we have regression coverage for this tab too.

Concretely, after Task 1 ran, the `triage_results_priority_badge_maps_full_scale` test body contains no `assert_eq!(item.metadata, ...)` line. Add one inside the loop:

```rust
        // Metadata should NOT repeat the domain — the URL in the title row
        // already shows it. Only the tag count remains.
        assert_eq!(item.metadata, "1 tag");
```

And change the existing `triage_review_items_show_indirect_badge_and_disabled_state` assertion from:

```rust
    assert_eq!(item.metadata, "Security · example.com");
```

to:

```rust
    assert_eq!(item.metadata, "Security");
```

- [ ] **Step 6.3: Run tests to confirm they fail**

Note: `render_tests.rs` is included via `#[path = "render_tests.rs"] mod tests;` in [render.rs:2008-2009](../../crates/harvester_app/src/platform/ui/render.rs#L2008-L2009), so the compiled module is named `tests`, not `render_tests`. Filter on a concrete test name instead:

Run: `cargo test -p harvester_app --lib triage_review_items_show_indirect_badge_and_disabled_state triage_results_priority_badge_maps_full_scale`
(Or simply `cargo test -p harvester_app --lib` to run the whole lib test set.)

Expected: FAIL on the two updated assertions.

- [ ] **Step 6.4: Update the metadata builders**

In the functions you found in Step 6.1, strip the domain segment from the Triage Review and Triage Results paths. The exact code depends on the current builder shape; show the before/after in the commit.

- [ ] **Step 6.5: Re-run tests**

Run: `cargo test -p harvester_app`
Expected: all green.

- [ ] **Step 6.6: Manual smoke test**

Run the app, confirm the Triage Results rows now show only `N tags` (or similar) on the metadata line, with the domain still visible in the title row via the URL itself.

- [ ] **Step 6.7: clippy, fmt, commit**

```bash
cargo clippy --all-targets -- -D warnings && cargo fmt
git add crates/harvester_app/src/platform/ui/render.rs \
        crates/harvester_app/src/platform/ui/render_tests.rs
git commit -m "Drop redundant domain from triage row metadata"
```

---

## Task 7: Stronger selected-row background tint

**Files:**
- Investigate, then modify: [crates/harvester_app/src/platform/ui/layout.rs](../../crates/harvester_app/src/platform/ui/layout.rs) (`ListBoxSelectionAccent` and any related list-box selection styles)

**Rationale:** Spec line 178 calls for "strong selected-state styling using Accent Primary." Currently the selected row is mostly signalled by the left-edge accent bar plus a very faint background shift. An accent-wash background (roughly 8% of Accent Primary `#c96442` over `Surface` `#1e1e1c`) makes the selection pop without competing with the active item's content.

- [ ] **Step 7.1: Find the selection style block**

Run:
```
Grep pattern="ListBoxSelection|selection_background|SelectionAccent" path=crates/harvester_app/src/platform/ui/layout.rs
```

Note the line numbers of the `DefineStyle` for the selected row background (not just the accent bar).

- [ ] **Step 7.2: Pick a new background color**

8% of `#c96442` over `#1e1e1c` (eyeballing as `rgb(0.92*0x1e + 0.08*0xc9, 0.92*0x1e + 0.08*0x64, 0.92*0x1e + 0.08*0x42)`) ≈ `#2c2321`. Use that as the new selection background.

- [ ] **Step 7.3: Apply the change**

Edit the relevant `DefineStyle` block to use `#2c2321` (`r: 0x2c, g: 0x23, b: 0x21`) for the selected row background. Leave the accent bar color unchanged.

- [ ] **Step 7.4: Build, run, visually verify**

Run: `cargo run -p harvester_app`
Expected: the selected row is visibly tinted warm orange/brown compared to unselected rows, the text stays readable, the accent edge still anchors the left side.

- [ ] **Step 7.5: clippy, fmt, commit**

```bash
cargo clippy --all-targets -- -D warnings && cargo fmt
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "Strengthen selected-row tint with a low-opacity accent wash"
```

---

## Task 8: Demote the right-pane secondary tab strip (within CommanDuctUI API limits)

**Files:**
- Investigate, then modify: [crates/harvester_app/src/platform/ui/layout.rs](../../crates/harvester_app/src/platform/ui/layout.rs) (right-pane tab bar style)

**Scope constraint (from code review):** The CommanDuctUI tab bar handler at [src/CommanDuctUI/src/controls/tab_bar_handler.rs:475-487](../../src/CommanDuctUI/src/controls/tab_bar_handler.rs#L475-L487) unconditionally paints a 3px accent line under the active tab, and `SetTabBarStyle` at [src/CommanDuctUI/src/types.rs:836-843](../../src/CommanDuctUI/src/types.rs#L836-L843) only exposes `background_color`, `text_color`, `accent_color`, and `font`. Removing or suppressing the accent line would require a generic CommanDuctUI API + handler change — per Agents.md, that also requires a version bump, CHANGELOG update, and dark-theme preservation.

This task deliberately **stays within the existing API**. We keep the accent underline, and demote the secondary tab strip using only the knobs that are already available:
1. Smaller font (via `font: Some(FontDescription { size: ... })` on the `SetTabBarStyle` style for the secondary strip).
2. Muted accent color (use Text Tertiary / Border Default as the underline color instead of Accent Primary, so the underline still exists but reads as a hint rather than a second primary accent).
3. Muted text color overall (Text Tertiary for the inactive tabs, Text Secondary for the active one instead of Text Primary).

The combination gives visual subordination without touching CommanDuctUI. If you later decide the underline itself must go, that's a separate change to CommanDuctUI with its own version bump and changelog entry — do NOT bundle it into this task.

**Rationale:** The right pane's `Triage / Summary / Briefing / Trends / Poll Stats` tabs today have the same visual weight as the top-level `Jobs / Triage Review / Triage Results / Prompt Lab` tabs, creating two equally prominent tab systems. Spec line 147 warns against "multiple competing active indicators." Making the secondary strip smaller and muting its accent color is enough to re-establish the hierarchy.

- [ ] **Step 8.1: Find the right-pane tab bar style**

Run:
```
Grep pattern="TAB_BAR|TabBar.*right|right_pane.*tab|StyleId::TabBar" path=crates/harvester_app/src/platform/ui/layout.rs
```

Identify the `DefineStyle` and `ApplyStyleToControl` commands that set up the right pane's tab bar. Check whether the left and right tab bars currently share a `StyleId::TabBar` or use different ones.

- [ ] **Step 8.2: Add a `TabBarSecondary` style id if needed**

If the left and right tab bars share one style, add a new variant `StyleId::TabBarSecondary` to the `StyleId` enum (grep for `enum StyleId` in the crate to find the definition) and wire the right pane's tab bar to use it via `ApplyStyleToControl`. If they already have separate style ids, reuse the existing one for the right pane — no new variant needed.

- [ ] **Step 8.3: Define the demoted style**

Add a `PlatformCommand::DefineStyle` block for the secondary tab bar style using these values (use the appropriate `FontDescription` shape — grep for existing `FontDescription` literals in `layout.rs` to copy the exact field names):

- Background: same as the primary tab bar (keep tonal continuity).
- Text color (`text_color`): Text Tertiary `#87867F` (inactive tabs derive from this; the handler blends text_inactive automatically).
- Accent color: `#3D3D3A` (Border Default / Surface Overlay — a muted warm neutral that still draws the underline, but no longer competes with `#C96442`).
- Font: `FontDescription` with `size_pt: 9` (primary tab bar is 10pt; verify by grepping for the primary tab bar's font size and adjusting this to be ~10% smaller). **Do not change the font face.**

Do NOT attempt to set `accent_color: None` or zero-width — the handler will still paint 3px of whatever color is passed, so rely on the muted color instead.

- [ ] **Step 8.4: Build and run the tests**

Run: `cargo build -p harvester_app && cargo test -p harvester_app`
Expected: all green. `cargo clippy --all-targets -- -D warnings` should also be clean.

- [ ] **Step 8.5: Visually verify**

Run: `cargo run -p harvester_app`

Expected:
- Top-level tab bar (Jobs/Triage Review/...) still has the full-size orange accent underline.
- Right-pane tab bar (Triage/Summary/...) is visibly smaller and its active underline is a dull warm gray, not orange. The active tab is still legible, but the left-side (primary) tab bar clearly owns the visual hierarchy.

If the difference is too subtle, nudge the font one point smaller or desaturate the accent color further. If the difference is too loud, move the accent color halfway back toward `#C96442`. Stop when the hierarchy reads right.

- [ ] **Step 8.6: clippy, fmt, commit**

```bash
cargo clippy --all-targets -- -D warnings && cargo fmt
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "Demote right-pane secondary tab strip via font + accent muting"
```

**Explicitly not done in this task:** removing the 3px accent line entirely. That is a CommanDuctUI API change, a separate initiative with its own changelog entry, and is out of scope for the visual-spec polish pass.

---

# Spec coverage check

Mapping each VisualDesignSpec.md concern addressed in this plan:

| Spec concern | Line(s) | Addressed by |
|---|---|---|
| Priority color accelerates triage by urgency | 66, 189 | Task 1 |
| Warm neutrals only — no cool blue-grays | 36, 275 | Task 2 |
| Prefer badges/pills over dense inline metadata | 191 | Tasks 3 + 4 |
| Avoid heavy box outlines; prefer tonal contrast | 127-131 | Task 5 |
| Row metadata should not overwhelm scanning | 185-191 | Task 6 |
| Strong selected-state styling using Accent Primary | 178 | Task 7 |
| Avoid multiple competing active indicators | 147 | Task 8 |

Concerns *not* addressed in this plan (because the current UI already meets them):
- Warm dark palette (spec lines 36-47) — the app already uses it.
- Single dominant accent (spec lines 22-24) — the app already has one.
- Flat tabs with accent underline (spec lines 137-148) — already done on the top-level tab bar.
- Primary action visually promoted (spec lines 151-168) — `Poll sources` is already the sole filled primary button.
- Type scale (spec lines 100-115) — reading pane already uses 24/14/12 rhythm.

---

# Self-review notes

- Every code step includes literal code, exact file references, and a test before the implementation where the change has semantic content.
- Tasks 1-4 are ordered: Task 4 depends on Task 3 (RTF shading must exist before backtick-wrapped tags look like chips). Task 2 is independent but grouped with Task 1 because they share the "color system" theme.
- Part B tasks are independent of each other and of Part A (after Part A is in).
- Each task ends with `cargo clippy --all-targets -- -D warnings && cargo fmt` per Agents.md.
- No CLI flags are added to `harvester_batch`, so `scripts/Start-HarvesterBatch.ps1` doesn't need updating.
- No changes to `CommanDuctUI` — all work lives inside `harvester_app` and `harvester_core`. Task 8 explicitly stays within the existing `SetTabBarStyle` API; removing the 3px underline is called out as out-of-scope.
- Reducer purity is untouched — these are rendering and formatting changes.
- Task 4 writes an engineering diary entry per Agents.md guidance.

## Changes applied from plan review (docs/reviews/2026-04-10-visual-spec-ui-polish-plan-review.md)

- **Finding 1 (High, Task 8 CommanDuctUI boundary):** Task 8 rewritten. The accent underline stays — CommanDuctUI's `SetTabBarStyle` API + `tab_bar_handler.rs` unconditionally paint it, so removing it would require a CommanDuctUI change + version bump + CHANGELOG update per Agents.md. The demoted styling now uses only the existing knobs: smaller font, Text Tertiary text, and a muted warm-neutral accent color that keeps the underline visible but stops it competing with the primary accent. Removing the underline entirely is explicitly called out as out-of-scope.
- **Finding 2 (Medium, tag text sanitization):** Task 4 now includes a private `sanitize_tag_for_chip` helper in `preview.rs` that replaces backtick with U+2032 PRIME and CR/LF with spaces, drops empty-after-trim tags, and is exercised by three new regression tests (backtick, CR/LF, empty-tag cases).
- **Finding 3 (Medium, broken markdown links):** All markdown link targets now use `../../crates/...` and `../EngineeringDiary.md` relative to the new plan location at `docs/visual_design/Plan.visual-spec-ui-polish.md`. Bash command paths inside code blocks are unchanged because those execute from the repo root.
- **Finding 4 (Medium, diary references nonexistent plan file):** The diary snippet in Task 4.10 now points at `docs/visual_design/Plan.visual-spec-ui-polish.md`, matching the actual plan location.
- **Finding 5 (Low, Task 6 test command):** Task 6 step 6.3 explains that `render_tests.rs` is included via `#[path = "render_tests.rs"] mod tests;` (module name `tests`, not `render_tests`), and switches to filtering on concrete test names. Task 6 step 6.2 was also revised to account for Task 1's test rewrite removing the old metadata assertion.
- **Finding 6 (Low, superpowers sub-skill mandate):** The plan header is now advisory — the skills are recommended if present but the plan is self-contained and can be executed by any agent or by hand.
