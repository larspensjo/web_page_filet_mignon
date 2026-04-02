# Visual Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the visual redesign defined in `docs/visual_design/VisualDesignSpec.md` — shifting the app from cool blue-gray accents to a warm neutral palette with terracotta accent, improved typography hierarchy, and better spacing.

**Architecture:** All color/font/spacing changes are isolated to two files: `layout.rs` (theme definitions + layout rules) and `markdown_to_rtf.rs` (RTF reading pane colors). The CommanDuctUI styling API supports per-widget colors, fonts, and margins. It does NOT support border-radius, shadows, alpha, or per-control hover states — those items are deferred to a future CommanDuctUI enhancement pass.

**Tech Stack:** Rust, Win32 via CommanDuctUI, RTF for reading pane rendering.

**Spec reference:** `docs/visual_design/VisualDesignSpec.md`

---

## File Map

| File | Changes |
|------|---------|
| `crates/harvester_app/src/platform/ui/layout.rs` | Rewrite `define_dark_theme_styles()` with new palette; update `SetTabBarStyle` and `SetToggleSwitchStyle` calls; adjust layout margins; split token meter into its own style; add new `StyleId` variants for token meter and section titles |
| `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs` | Update hardcoded RTF colors and heading sizes to match spec |
| `src/CommanDuctUI/src/styling_primitives.rs` | Add new `StyleId` variants: `TokenMeter`, `SectionTitle` |

## Color Mapping: Current to New

This table drives all color changes. Reference it throughout implementation.

| Token | Spec Hex | Spec RGB | Replaces |
|-------|----------|----------|----------|
| Base Dark | `#141413` | `(20, 20, 19)` | `#2E3239` MainWindowBackground |
| Surface | `#1e1e1c` | `(30, 30, 28)` | `#262A2E` PanelBackground |
| Surface Raised | `#30302e` | `(48, 48, 46)` | `#1A1D22` inputs/viewers |
| Surface Overlay | `#3d3d3a` | `(61, 61, 58)` | `#373E47` selected row, `#40444B` splitter |
| Text Primary | `#faf9f5` | `(250, 249, 245)` | `#E0E5EC` primary text |
| Text Secondary | `#b0aea5` | `(176, 174, 165)` | `#8090A0` status bar text |
| Text Tertiary | `#87867f` | `(135, 134, 127)` | `#60656B` disabled items |
| Text Disabled | `#5e5d59` | `(94, 93, 89)` | (new) |
| Accent Primary | `#c96442` | `(201, 100, 66)` | `#0080FF` blue accent, `#FFB347` orange headers |
| Accent Hover | `#d97757` | `(217, 119, 87)` | (new — for future hover support) |
| Accent Warning | `#b53333` | `(181, 51, 51)` | (new — for stop button) |
| Border Default | `#30302e` | `(48, 48, 46)` | (implicit from Surface Raised) |
| Border Subtle | `#2a2a28` | `(42, 42, 40)` | (new) |

---

## Task 1: Add new StyleId variants

**Files:**
- Modify: `src/CommanDuctUI/src/styling_primitives.rs:57-93`

These new variants allow the token meter and section titles to have their own styles, separate from the catch-all `HeaderLabel`.

- [ ] **Step 1: Add TokenMeter and SectionTitle to the StyleId enum**

In `src/CommanDuctUI/src/styling_primitives.rs`, add two new variants to `StyleId`:

```rust
    // TabBar custom control
    TabBar,
    TabBarAccent,
    // Token and status meter
    TokenMeter,
    // Section title (subdued heading, not accent-colored)
    SectionTitle,
```

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Expected: compiles cleanly. The new variants are unused so far — no warnings expected because they'll be used in Task 2.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS (new enum variants may warn as unused; suppress if needed, they'll be consumed in Task 2).

- [ ] **Step 4: Update CommanDuctUI version and changelog**

Per repo rules: "If CommanDuctUI changes, update its version and changelog." Bump the patch version in the CommanDuctUI `Cargo.toml` and add a changelog entry noting the two new `StyleId` variants.

- [ ] **Step 5: Commit**

```bash
git add src/CommanDuctUI/src/styling_primitives.rs src/CommanDuctUI/Cargo.toml src/CommanDuctUI/CHANGELOG.md
git commit -m "feat(ui): add TokenMeter and SectionTitle StyleId variants"
```

---

## Task 2: Rewrite define_dark_theme_styles with new palette

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/layout.rs:833-1160`

Replace every color value in `define_dark_theme_styles()` using the mapping table above. This is the core palette change.

- [ ] **Step 1: Replace MainWindowBackground**

Change `#2E3239` to Base Dark `(0x14, 0x14, 0x13)`:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::MainWindowBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 2: Replace PanelBackground**

Change bg `#262A2E` to Surface `(0x1e, 0x1e, 0x1c)`, text `#E0E5EC` to Text Primary `(0xfa, 0xf9, 0xf5)`:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::PanelBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 3: Replace StatusBarBackground**

Change bg to Base Dark `(0x14, 0x14, 0x13)`, text to Text Secondary `(0xb0, 0xae, 0xa5)`:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::StatusBarBackground,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0xb0,
                g: 0xae,
                b: 0xa5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 4: Replace DefaultText**

Change bg to Base Dark `(0x14, 0x14, 0x13)`, text to Text Primary `(0xfa, 0xf9, 0xf5)`:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultText,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 5: Replace HeaderLabel — change from orange to Accent Primary (terracotta)**

This is a key change: headers go from bright orange `#FFB347` to terracotta `#c96442`, and the background becomes Surface `(0x1e, 0x1e, 0x1c)`:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::HeaderLabel,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xc9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 6: Replace DefaultInput**

Change bg to Surface Raised `(0x30, 0x30, 0x2e)`, text to Text Primary:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultInput,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2e,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 7: Replace DefaultButton, RadioButton, CheckBox**

All three share the same palette. Change bg to Surface `(0x1e, 0x1e, 0x1c)`, text to Text Primary:

```rust
    // DefaultButton
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::DefaultButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
    // RadioButton
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::RadioButton,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
    // CheckBox
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::CheckBox,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 8: Replace TabBar and TabBarAccent**

TabBar bg to Base Dark, text to Text Primary. TabBarAccent from blue `#0080FF` to Accent Primary `(0xc9, 0x64, 0x42)`:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TabBar,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TabBarAccent,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xc9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 9: Replace TreeView and TreeView selection styles**

TreeView bg to Surface, text to Text Primary. Selected row bg to Surface Overlay `(0x3d, 0x3d, 0x3a)`. Selection accent from blue to Accent Primary:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeView,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeItemDisabled,
        style: ControlStyle {
            text_color: Some(Color {
                r: 0x87,
                g: 0x86,
                b: 0x7f,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeViewSelectedRow,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x3d,
                g: 0x3d,
                b: 0x3a,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TreeViewSelectionAccent,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0xc9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 10: Replace ViewerMonospace and ViewerReadable**

ViewerMonospace: bg to Surface Raised, text from cyan `#00C9FF` to Text Primary (monospace content shouldn't scream). ViewerReadable: bg to Surface Raised, text to Text Primary, font stays Segoe UI but bump size from 11 to 12 for readability:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ViewerMonospace,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2e,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            font: Some(FontDescription {
                name: Some("Cascadia Code".to_string()),
                size: Some(10),
                weight: Some(FontWeight::Normal),
            }),
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ViewerReadable,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2e,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(12),
                weight: Some(FontWeight::Normal),
            }),
        },
    });
```

- [ ] **Step 11: Replace ProgressBar, Splitter, ComboBox**

ProgressBar: bg to Surface Raised, bar color (text_color) to Accent Primary instead of cyan. Splitter: bg to Border Default `(0x30, 0x30, 0x2e)`. ComboBox: bg to Surface Raised, text to Text Primary:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ProgressBar,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2e,
            }),
            text_color: Some(Color {
                r: 0xc9,
                g: 0x64,
                b: 0x42,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::Splitter,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2e,
            }),
            ..Default::default()
        },
    });
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::ComboBox,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x30,
                g: 0x30,
                b: 0x2e,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 12: Add TokenMeter style definition**

This new style makes the token label use Text Secondary instead of the accent color, so it stops competing with content:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::TokenMeter,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x14,
                g: 0x14,
                b: 0x13,
            }),
            text_color: Some(Color {
                r: 0xb0,
                g: 0xae,
                b: 0xa5,
            }),
            ..Default::default()
        },
    });
```

- [ ] **Step 13: Add SectionTitle style definition**

Section titles (like "Triage Results | Since checkpoint") get Text Primary on Surface bg, without the accent color used for header labels:

```rust
    commands.push(PlatformCommand::DefineStyle {
        style_id: StyleId::SectionTitle,
        style: ControlStyle {
            background_color: Some(Color {
                r: 0x1e,
                g: 0x1e,
                b: 0x1c,
            }),
            text_color: Some(Color {
                r: 0xfa,
                g: 0xf9,
                b: 0xf5,
            }),
            font: Some(FontDescription {
                name: Some("Segoe UI".to_string()),
                size: Some(11),
                weight: Some(FontWeight::Bold),
            }),
        },
    });
```

- [ ] **Step 14: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 15: Commit**

```bash
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "feat(ui): rewrite dark theme palette to warm neutrals with terracotta accent"
```

---

## Task 3: Update inline SetTabBarStyle and SetToggleSwitchStyle calls

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/layout.rs:170-339` (inline style calls in `initial_commands`)

The `initial_commands` function has three `SetTabBarStyle` calls and one `SetToggleSwitchStyle` call with hardcoded colors that must match the new palette.

- [ ] **Step 1: Update all three SetTabBarStyle calls**

For `TAB_BAR_LEFT`, `TAB_BAR_RIGHT`, and `TAB_BAR_TRENDS`, change the inline colors:

```rust
    commands.push(PlatformCommand::SetTabBarStyle {
        window_id,
        control_id: TAB_BAR_LEFT, // repeat for TAB_BAR_RIGHT and TAB_BAR_TRENDS
        background_color: Color {
            r: 0x14,
            g: 0x14,
            b: 0x13,
        },
        text_color: Color {
            r: 0xfa,
            g: 0xf9,
            b: 0xf5,
        },
        accent_color: Color {
            r: 0xc9,
            g: 0x64,
            b: 0x42,
        },
        font: None,
    });
```

Apply the same background/text/accent to all three tab bar controls.

- [ ] **Step 2: Update SetToggleSwitchStyle**

Change the toggle switch colors to match the warm palette:

```rust
    commands.push(PlatformCommand::SetToggleSwitchStyle {
        window_id,
        control_id: TS_JOBS_SCOPE,
        background: Color {
            r: 0x14,
            g: 0x14,
            b: 0x13,
        },
        pill_off: Color {
            r: 0x3d,
            g: 0x3d,
            b: 0x3a,
        },
        pill_on: Color {
            r: 0xc9,
            g: 0x64,
            b: 0x42,
        },
        knob: Color {
            r: 0xfa,
            g: 0xf9,
            b: 0xf5,
        },
        text: Color {
            r: 0xb0,
            g: 0xae,
            b: 0xa5,
        },
    });
```

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "feat(ui): update tab bar and toggle switch inline colors to warm palette"
```

---

## Task 4: Reassign style applications — token meter and section titles

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/layout.rs:2268-2310` (style application section)

Currently `LABEL_TOKEN_PROGRESS` uses `HeaderLabel` (orange/terracotta). The spec says to quiet the token meter. Also, some labels that are section titles should use `SectionTitle` instead of `HeaderLabel`.

- [ ] **Step 1: Change LABEL_TOKEN_PROGRESS to use TokenMeter style**

Find the `ApplyStyleToControl` for `LABEL_TOKEN_PROGRESS` and change from `HeaderLabel` to `TokenMeter`:

```rust
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TOKEN_PROGRESS,
        style_id: StyleId::TokenMeter,
    });
```

- [ ] **Step 2: Change section headers to use SectionTitle style**

Change `LABEL_JOBS_HEADER`, `LABEL_PREVIEW_HEADER`, and `LABEL_TRENDS_DESCRIPTION` from `HeaderLabel` to `SectionTitle`:

```rust
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_PREVIEW_HEADER,
        style_id: StyleId::SectionTitle,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_JOBS_HEADER,
        style_id: StyleId::SectionTitle,
    });
    commands.push(PlatformCommand::ApplyStyleToControl {
        window_id,
        control_id: LABEL_TRENDS_DESCRIPTION,
        style_id: StyleId::SectionTitle,
    });
```

Keep `LABEL_INPUT_HINT` and `LABEL_PROMPT_LAB_STATUS` on `HeaderLabel` — these benefit from the accent color to draw attention to interactive areas.

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "feat(ui): quiet token meter and separate section titles from accent headers"
```

---

## Task 5: Update RTF reading pane colors and typography

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/markdown_to_rtf.rs:7-10` (color constants)

The RTF converter has hardcoded RGB values that must match the new palette.

- [ ] **Step 1: Update the RTF color constants**

Replace the three color constants:

```rust
const BODY_FONT_SIZE_HALF_POINTS: usize = 24; // was 22 (11pt) -> 24 (12pt) for readability
const COLOR_BODY_TEXT_RTF: &str = "\\red250\\green249\\blue245;"; // Text Primary #faf9f5
const COLOR_BACKGROUND_RTF: &str = "\\red48\\green48\\blue46;";  // Surface Raised #30302e
const COLOR_LINK_RTF: &str = "\\red217\\green119\\blue87;}";     // Accent Hover #d97757
```

Rationale:
- Body text: `#E0E5EC` (cool) -> `#faf9f5` (warm Text Primary)
- Background: `#1A1D22` (cool) -> `#30302e` (warm Surface Raised, matching ViewerReadable bg)
- Link color: `#58A6FF` (blue) -> `#d97757` (Accent Hover — slightly lighter than terracotta for legibility on dark bg)
- Font size: 22 half-points (11pt) -> 24 half-points (12pt), matching the ViewerReadable font size bump

- [ ] **Step 2: Update heading sizes for clearer hierarchy**

In the `handle_start_tag` function, adjust heading sizes to create more separation between H1/H2/H3. Change the heading size match (around line 136-140):

```rust
            let size = match level {
                HeadingLevel::H1 => 40,  // was 36 — Page Title equivalent
                HeadingLevel::H2 => 32,  // unchanged — Section Title
                _ => 26,                 // was 28 — closer to body, less visual noise
            };
```

Also increase the spacing after headings for better breathing room. Change the format string:

```rust
            rtf.push_str(&format!("\\pard\\sa160\\sb80\\b\\fs{size} "));
```

(Was `\\sa120\\sb60` — increased to `\\sa160\\sb80` for more visual separation per spec.)

- [ ] **Step 3: Update the RTF heading end to use new font size**

The heading end resets to `BODY_FONT_SIZE_HALF_POINTS` — since we changed the constant, this happens automatically. No code change needed here, but verify the constant is referenced (line ~185):

```rust
        TagEnd::Heading(_) => {
            rtf.push_str(&format!(
                "\\b0\\fs{BODY_FONT_SIZE_HALF_POINTS}\\par\\pard\\sa60\\sb0 "
            ));
        }
```

This already references the constant, so the size change propagates.

- [ ] **Step 4: Update tests that check hardcoded RTF values**

The test `headings_render_as_distinct_bold_blocks` (around line 251) checks for specific `\\fs` values:

```rust
    #[test]
    fn headings_render_as_distinct_bold_blocks() {
        let rtf = convert_markdown_to_rtf("# H1\n## H2\n### H3");
        assert!(rtf.contains("\\pard\\sa160\\sb80\\b\\fs40 H1"));
        assert!(rtf.contains("\\pard\\sa160\\sb80\\b\\fs32 H2"));
        assert!(rtf.contains("\\pard\\sa160\\sb80\\b\\fs26 H3"));
    }
```

Also update `heading_after_list_starts_on_new_paragraph` test:

```rust
    #[test]
    fn heading_after_list_starts_on_new_paragraph() {
        let rtf = convert_markdown_to_rtf("- one\n- two\n\n## Brave");
        assert!(rtf.contains("\\pard \\par \\pard\\sa160\\sb80\\b\\fs32 Brave"));
    }
```

And update any other tests that assert on the old color table values. Search for `\\red224\\green229\\blue236` and `\\red26\\green29\\blue34` in tests and replace with the new values.

- [ ] **Step 5: Run tests**

Run: `cargo test -p harvester_app -- markdown_to_rtf`
Expected: all tests PASS with updated assertions.

- [ ] **Step 6: Commit**

```bash
git add crates/harvester_app/src/platform/ui/markdown_to_rtf.rs
git commit -m "feat(ui): update RTF reading pane colors and typography to match warm palette"
```

---

## Task 6: Adjust layout margins for better spacing

**Files:**
- Modify: `crates/harvester_app/src/platform/ui/layout.rs:1225-1602` (layout rules in `build_layout_rules`)

The spec calls for more breathing room inside panes, better spacing around buttons, and more padding in the reading pane.

- [ ] **Step 1: Increase reading pane inner padding**

Change `PANEL_PREVIEW` margins from `(6, 6, 6, 6)` to `(8, 12, 8, 12)`:

```rust
        LayoutRule {
            control_id: PANEL_PREVIEW,
            parent_control_id: None,
            dock_style: DockStyle::Fill,
            order: 310,
            fixed_size: None,
            margin: (8, 12, 8, 12),
        },
```

- [ ] **Step 2: Add inner margins to viewer controls**

The RichEdit viewers currently have `(0, 0, 0, 0)` margins. Add padding so text doesn't touch the edges. For `VIEWER_TRIAGE`, `VIEWER_PREVIEW`, `VIEWER_BRIEFING`, `VIEWER_POLL_STATS`:

```rust
        LayoutRule {
            control_id: VIEWER_TRIAGE,
            parent_control_id: Some(PANEL_TAB_TRIAGE),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (4, 8, 4, 8),
        },
```

Apply the same `(4, 8, 4, 8)` margin to all four viewer controls.

- [ ] **Step 3: Increase button spacing**

Current buttons have `margin: (6, 6, 6, 0)` and fixed_size 160. Increase left margin on BUTTON_STOP to separate it from the constructive actions:

```rust
        LayoutRule {
            control_id: BUTTON_STOP,
            parent_control_id: Some(PANEL_BUTTONS),
            dock_style: DockStyle::Left,
            order: 0,
            fixed_size: Some(140),
            margin: (6, 16, 6, 8),  // extra right margin to separate from constructive buttons
        },
```

For the remaining buttons, add slight left margin:

```rust
        // BUTTON_BRIEFING, BUTTON_TRIAGE, BUTTON_POLL_SOURCES, BUTTON_OPEN_BROWSER
        margin: (6, 4, 6, 4),
```

- [ ] **Step 4: Increase left panel margins**

Change `PANEL_LEFT` from `(6, 6, 6, 6)` to `(8, 8, 8, 8)`:

```rust
        LayoutRule {
            control_id: PANEL_LEFT,
            parent_control_id: None,
            dock_style: DockStyle::Left,
            order: 200,
            fixed_size: Some(left_panel_width),
            margin: (8, 8, 8, 8),
        },
```

- [ ] **Step 5: Increase status bar padding**

Change `LABEL_STATUS` margin from `(6, 6, 6, 6)` to `(8, 12, 8, 12)`:

```rust
        LayoutRule {
            control_id: LABEL_STATUS,
            parent_control_id: Some(PANEL_BOTTOM),
            dock_style: DockStyle::Fill,
            order: 0,
            fixed_size: None,
            margin: (8, 12, 8, 12),
        },
```

- [ ] **Step 6: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 7: Commit**

```bash
git add crates/harvester_app/src/platform/ui/layout.rs
git commit -m "feat(ui): increase layout margins for better spacing and breathing room"
```

---

## Task 7: Final verification and clippy

**Files:**
- All modified files

- [ ] **Step 1: Run full build**

Run: `cargo build`
Expected: PASS

- [ ] **Step 2: Run all tests**

Run: `cargo test`
Expected: all tests PASS. If any tests assert on specific color values (e.g., in render.rs tests), update them.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS with no warnings.

- [ ] **Step 4: Commit any remaining fixes**

If tests or clippy required adjustments, commit them:

```bash
git add -A
git commit -m "fix(ui): address test and clippy issues from visual redesign"
```

---

## Deferred: CommanDuctUI Enhancements (Not in this plan)

These spec items cannot be implemented without extending CommanDuctUI's `ControlStyle`:

| Spec Item | Required Enhancement |
|-----------|---------------------|
| Border radius (4-8px) | Add `border_radius: Option<u32>` to `ControlStyle`; implement in Win32 via `CreateRoundRectRgn` or `SetWindowRgn` |
| Ring shadows (`0px 0px 0px 1px`) | Add shadow/ring properties; implement via custom paint in GDI |
| Per-control hover states | Add hover color fields or a state-based style model |
| Internal padding | Add `padding` field to `ControlStyle` (distinct from layout margins) |
| Alpha/opacity | Extend `Color` to RGBA; implement via `AlphaBlend` or layered windows |
| Button variants (primary/secondary/ghost/destructive) | Add more `StyleId` variants and apply per-button, or add button-specific style commands |

These should be tackled as a separate CommanDuctUI feature pass after the palette/typography/spacing changes are visually validated.
