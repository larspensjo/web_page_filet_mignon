# Harvester Visual Identity Asset System - Plan

## Overview

Create a durable visual identity for Harvester that can support the desktop app icon, small UI imagery, splash/startup artwork, documentation graphics, and future release assets.

The candidate icon direction is a strong starting point: documents flowing into a funnel. It matches Harvester's core job of collecting web sources, filtering them, and turning them into structured research output. The production plan is to keep that metaphor, reduce the detail until it survives at small sizes, and build a small asset system around it instead of treating the icon as a one-off generated image.

## Goals

- Establish a recognizable Harvester mark that works as an app icon, favicon, documentation mark, and monochrome symbol.
- Add a restrained set of small in-app visual assets that improve recognition without reducing information density.
- Add a startup/splash image path for the short startup period without making startup slower or visually noisy.
- Keep the identity aligned with `docs/visual_design/VisualDesignSpec.md`.
- Preserve the current app architecture: input -> action -> reducer -> state -> render, with side effects isolated and fed back as actions.
- Keep `CommanDuctUI` generic. Any Harvester-specific asset names, artwork, splash behavior, or generated-image policy should live in Harvester app code and docs, not in toolkit concepts.

## Non-Goals

- Do not redesign the entire UI in this plan.
- Do not introduce a marketing landing page aesthetic into the dense desktop tool.
- Do not add large decorative images inside work surfaces where they would compete with triage, summary, briefing, or prompt-lab content.
- Do not add Harvester-specific concepts to `CommanDuctUI`.
- Do not make the icon depend on tiny lines that disappear below 32 px.
- Do not introduce a second accent system that competes with the existing warm terracotta accent.

## Existing Design Constraints

Source of truth: `docs/visual_design/VisualDesignSpec.md`.

Relevant constraints:

- Target aesthetic: contemporary, dense expert tool.
- Theme: warm dark surfaces, no cool blue-gray palette.
- Primary accent: `#c96442`.
- Accent hover: `#d97757`.
- Surfaces: `#141413`, `#1e1e1c`, `#30302e`, `#3d3d3a`.
- Text: off-white and warm muted grays, not pure white.
- Style: flat, restrained, no glow, no ornamental depth.
- Icons or illustrations should solve recognition problems, not decorate routine workflow surfaces.

## Identity Direction

### Core Metaphor

Use "documents into a funnel" as the primary metaphor.

The mark should communicate:

- collection from many sources
- filtering and extraction
- transformation into one structured output
- editorial/research seriousness
- technical reliability

The mark should not communicate:

- food or restaurant branding
- generic AI chatbot
- cloud SaaS
- playful mascot
- crypto, cyberpunk, or neon aesthetic

### Candidate Icon Refinement Notes

The attached candidate has the right concept and palette direction. The production version should simplify:

- Reduce document line detail.
- Make the funnel silhouette slightly stronger.
- Keep stroke widths consistent.
- Preserve one recognizable copper accent shape.
- Use transparent-background variants for flexible use.
- Make one-color variants early, not as an afterthought.

## Asset Inventory

### Required Assets

- [ ] Primary source SVG for the Harvester mark.
- [ ] Full-color square app icon.
- [ ] Monochrome mark.
- [ ] Dark-background square icon.
- [ ] Light-background square icon.
- [ ] Transparent mark.
- [ ] Windows `.ico` file with multiple embedded sizes.
- [ ] Small favicon-style PNGs: 16, 24, 32, 48, 64 px.
- [ ] Documentation/README banner image.
- [ ] Splash/startup image.

### Optional Assets

- [ ] About-dialog image.
- [ ] Empty-state image for no jobs/no selected article.
- [ ] Small section icons for Jobs, Triage, Summary, Briefing, Trends, Poll Stats, Prompt Lab.
- [ ] Status micro-icons for queued, running, complete, warning, failed, archived.
- [ ] Release-note/social preview image.

## Recommended File Layout

Use a stable asset source folder and generated output folder.

```text
assets/
  identity/
    source/
      harvester-mark.svg
      harvester-mark-mono.svg
      harvester-splash-source.png
      prompts.md
      identity-sheet.md
    generated/
      harvester-icon-16.png
      harvester-icon-24.png
      harvester-icon-32.png
      harvester-icon-48.png
      harvester-icon-64.png
      harvester-icon-128.png
      harvester-icon-256.png
      harvester-icon-512.png
      harvester.ico
      harvester-splash.png
      harvester-readme-banner.png
```

Checklist:

- [ ] Decide whether `assets/identity/generated/` should be committed.
- [ ] Commit source assets even if generated outputs are regenerated.
- [ ] Document the generation process in `assets/identity/source/prompts.md`.
- [ ] Keep the final visual rules in `assets/identity/source/identity-sheet.md`.

## Image Generation Workflow

### Phase 1 - Generate Candidate Families

Generate several families before refining:

- [ ] Family A: close to attached candidate, documents flowing into funnel.
- [ ] Family B: more abstract page layers and selection/crop lines.
- [ ] Family C: stronger app-icon silhouette with fewer document details.
- [ ] Family D: splash/banner composition using the same funnel/document language.

Prompt for app icon candidates:

```text
Minimal vector app icon for Harvester, a desktop research and web harvesting application. 
Documents and web pages flow into a precise filter funnel, becoming structured output. 
Dense expert-tool identity, calm technical reliability, editorial research workflow. 
Strong silhouette, consistent stroke width, readable at 16px and 32px, no text. 
Warm dark theme, muted terracotta copper accent, off-white document strokes, deep warm charcoal background. 
Flat vector style, crisp geometry, restrained, professional desktop software icon.
```

Negative prompt:

```text
no text, no letters, no mascot, no robot, no cloud, no food, no meat, no restaurant, no photorealism, no neon, no glow, no complex shadows, no tiny decorative details, no generic AI sparkle
```

Checklist:

- [ ] Generate at least 20 icon candidates.
- [ ] Discard any candidate that reads as food, cloud, generic AI, or document editor.
- [ ] Keep 3-5 candidates for small-size testing.
- [ ] Save prompt, service settings, seed/reference IDs if available.

### Phase 2 - Select By Small-Size Fitness

For each finalist, test at:

- [ ] 16 x 16
- [ ] 24 x 24
- [ ] 32 x 32
- [ ] 48 x 48
- [ ] 64 x 64
- [ ] 256 x 256

Acceptance criteria:

- [ ] The funnel silhouette is still visible at 16 px.
- [ ] The document cluster is still understandable at 32 px.
- [ ] The icon does not rely on small internal document lines.
- [ ] The mark works in one color.
- [ ] The mark works on both `#141413` and light documentation backgrounds.
- [ ] The mark is recognizable without the app name.

### Phase 3 - Rebuild As Vector

Use the generated image as a reference, then rebuild as clean SVG.

Checklist:

- [ ] Draw the funnel using simple path geometry.
- [ ] Draw 2-3 document/page shapes, not many.
- [ ] Normalize stroke widths.
- [ ] Remove texture, blur, raster shadows, and accidental artifacts.
- [ ] Use named colors that map to design tokens.
- [ ] Produce full-color and monochrome SVGs.
- [ ] Verify the SVG has no embedded raster image.
- [ ] Verify the SVG viewBox is square and has consistent padding.

Recommended SVG rules:

- [ ] Use transparent background for the source mark.
- [ ] Keep a separate square-background composition for app icons.
- [ ] Avoid gradients unless the generated candidate absolutely depends on them.
- [ ] Prefer flat fills and strokes.
- [ ] Use rounded stroke joins only where they improve legibility.

### Phase 4 - Create Production Renders

Generate output files from the SVG source.

Checklist:

- [ ] Export PNG sizes: 16, 24, 32, 48, 64, 128, 256, 512.
- [ ] Create `.ico` containing at least 16, 24, 32, 48, and 256 px layers.
- [ ] Export transparent-mark PNGs for docs.
- [ ] Export dark-square and light-square app icon variants.
- [ ] Export monochrome variants.
- [ ] Compare 16 px and 32 px exports manually.
- [ ] Check the icon on a Windows taskbar/dark title bar if possible.

## In-App Image Plan

The current application is dense and operational. Visual assets should help recognition and state transitions, not decorate every surface.

### Best Candidates For Small In-App Assets

Add icons/pictures only where they speed scanning:

- [ ] Top-left app/window icon.
- [ ] Startup/splash image.
- [ ] Empty state in preview pane when no row is selected.
- [ ] Small icon beside Jobs, Triage Review, Triage Results, Prompt Lab tab labels if the tab control supports it cleanly.
- [ ] Small status icon for important warnings or failure states.
- [ ] Optional small mark in an About dialog.

Avoid:

- [ ] Icons in every article row.
- [ ] Decorative image behind the reading pane.
- [ ] Large illustrations inside Triage Results, Summary, Briefing, Trends, or Poll Stats when content exists.
- [ ] Replacing already-clear priority/category badges with pictorial icons.

### UI Placement Checklist

Header/title area:

- [ ] Use the app icon in the window/taskbar if platform support exists.
- [ ] Keep the top toolbar visually quiet; do not place a large logo in the work area.

Left pane:

- [ ] Do not add per-row icons to the article list unless a later usability pass proves they improve scan speed.
- [ ] Consider tiny state icons only for non-routine states like failed/warning/archived.

Right pane:

- [ ] Add an empty-state illustration only when no article/result is selected.
- [ ] Keep any empty-state illustration low contrast and below the reading hierarchy.
- [ ] Do not show an illustration behind text.

Footer/status:

- [ ] Do not add decorative imagery.
- [ ] Use existing meters and labels for operational status.

Prompt Lab:

- [ ] Prefer small semantic icons only for actions that otherwise require repeated text scanning.
- [ ] Keep icons generic enough that `CommanDuctUI` remains domain-neutral.

## Splash Image Plan

The startup phase takes a little time, so a splash surface can make launch feel intentional.

### Splash Design

Composition:

- [ ] Use the Harvester mark prominently.
- [ ] Use a dark warm background matching `#141413` or `#1e1e1c`.
- [ ] Include a subtle document-to-funnel motif or soft page layers.
- [ ] Keep text minimal: app name and one short status line if implemented.
- [ ] Avoid marketing copy.
- [ ] Avoid bright glows and decorative gradients.

Splash prompt:

```text
Splash screen image for Harvester, a desktop research and web harvesting tool. 
Warm dark background, refined funnel-and-documents mark, subtle layered web pages flowing into structured output. 
Calm expert software, editorial research, precise extraction, restrained terracotta accent, off-white lines, warm charcoal surfaces. 
Flat vector illustration, minimal detail, no marketing hero style, no text, no glow, no photorealism.
```

Recommended output:

- [ ] `harvester-splash.png`, 1200 x 675 or 1600 x 900.
- [ ] Croppable safe area centered.
- [ ] Separate mark overlay if the platform renders text/status independently.

### Splash Behavior

Implementation should be explicit and testable:

- [ ] Show splash as early as possible after process start.
- [ ] Keep startup hydration and effect scheduling in the existing startup flow.
- [ ] Close splash only after the main window has completed initial UI setup.
- [ ] If startup is fast, either skip splash below a threshold or show it for a short bounded duration only if that does not delay readiness.
- [ ] Log splash lifecycle failures with enough context but never fail app startup because a splash image is missing.

Architecture checklist:

- [ ] Represent splash visibility as app/platform startup behavior, not reducer domain state unless user-visible state transitions require it.
- [ ] If reducer state is needed, model it with explicit actions and keep reducers pure.
- [ ] Keep file loading side effects outside reducers.
- [ ] Keep `CommanDuctUI` changes generic, such as "set window icon" or "show splash bitmap", not "show Harvester splash".
- [ ] Update `CommanDuctUI` version and changelog if generic toolkit APIs change.

## Implementation Phases

### Phase 0 - Decisions And Source Control

Goal: decide identity constraints before generating final assets.

Checklist:

- [ ] Confirm the funnel/documents concept as the primary metaphor.
- [ ] Confirm the production palette uses existing visual-design tokens.
- [ ] Create `assets/identity/source/`.
- [ ] Add `assets/identity/source/prompts.md`.
- [ ] Add `assets/identity/source/identity-sheet.md`.
- [ ] Decide whether generated PNG/ICO files are committed.
- [ ] Decide whether app icon and splash should be part of the first implementation pass or split.

Validation:

- [ ] Documentation-only review.
- [ ] No Rust behavior changes.

### Phase 1 - Finalize The Icon

Goal: produce the source mark and app icon files.

Checklist:

- [ ] Generate candidate families.
- [ ] Select 1 primary candidate and 1 fallback.
- [ ] Rebuild the primary mark as SVG.
- [ ] Export required PNG sizes.
- [ ] Build `harvester.ico`.
- [ ] Test 16/24/32 px readability.
- [ ] Test dark and light backgrounds.
- [ ] Update identity sheet with final colors and usage rules.

Validation:

- [ ] Manual small-size review.
- [ ] Confirm SVG source can regenerate all outputs.
- [ ] Confirm generated files have stable names and sizes.

### Phase 2 - Wire The Application Icon

Goal: make the window/taskbar/executable use the identity mark.

Checklist:

- [ ] Locate the current Windows icon path, resource build path, or missing resource setup.
- [ ] Add the `.ico` file to the app resource pipeline.
- [ ] Ensure `WindowConfig` or platform initialization can apply the icon.
- [ ] Keep any generic icon-setting support inside `CommanDuctUI` free of Harvester terms.
- [ ] Add a focused test where practical for emitted platform command/resource selection.

Validation:

- [ ] `cargo build`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo fmt`
- [ ] Manual launch confirms window/taskbar icon appears.

### Phase 3 - Add Splash Startup

Goal: show an intentional startup image while the app hydrates.

Checklist:

- [ ] Measure current perceived startup points: process start, window creation, initial commands, `SignalMainWindowUISetupComplete`, `ShowWindow`.
- [ ] Choose splash implementation: platform splash window, main-window pre-content panel, or static startup overlay.
- [ ] Prefer the smallest implementation that does not disrupt existing startup flow.
- [ ] Add image-loading failure handling.
- [ ] Close or hide splash after main UI setup completes.
- [ ] Keep startup logs clear enough to diagnose missing/invalid splash assets.

Validation:

- [ ] Startup without splash file still succeeds.
- [ ] Startup with splash file shows image and then reveals the main window.
- [ ] `cargo build`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo fmt`
- [ ] Manual startup check on a cold run.

### Phase 4 - Add Small In-App Visuals

Goal: use visual identity sparingly inside the application.

Checklist:

- [ ] Add an empty-state illustration for the preview pane only when no selection exists.
- [ ] Consider small generic tab icons only if the toolkit supports them cleanly.
- [ ] Add status icons only for high-attention states, not routine rows.
- [ ] Keep all Harvester-specific asset choices in `harvester_app`.
- [ ] If `CommanDuctUI` needs image-control support, implement it generically and update version/changelog.
- [ ] Add rendering tests for commands emitted by empty-state/status views where practical.

Validation:

- [ ] Existing dense workflows still show content first.
- [ ] Empty-state imagery never overlaps text.
- [ ] Dark-theme contrast remains consistent.
- [ ] `cargo build`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo fmt`
- [ ] Manual screenshot review of Jobs, Triage Results, Summary, Briefing, Trends, Poll Stats, and Prompt Lab.

### Phase 5 - Documentation And Maintenance

Goal: make the identity reusable by future work.

Checklist:

- [ ] Add final prompt history to `assets/identity/source/prompts.md`.
- [ ] Add final asset rules to `assets/identity/source/identity-sheet.md`.
- [ ] Update `docs/visual_design/VisualDesignSpec.md` with a short identity-assets section if the identity becomes accepted.
- [ ] Update `docs/EngineeringDiary.md` after implementation with concrete changed files and lessons learned.
- [ ] Document regeneration commands for SVG -> PNG/ICO.

Validation:

- [ ] A future contributor can regenerate assets from source instructions.
- [ ] The UI design spec and identity sheet do not conflict.

## Risk Register

| Risk | Mitigation |
|------|------------|
| Generated icon has too much detail | Rebuild as SVG and test at 16/24/32 px before committing. |
| Icon reads as food because of "filet" association | Keep the funnel/document metaphor dominant; avoid meat, knife, plate, or restaurant shapes. |
| Splash makes startup slower | Show splash only while existing startup work happens; never delay readiness just to display it. |
| In-app images reduce scan speed | Limit imagery to startup, empty states, and high-attention status states. |
| Toolkit boundary is violated | Keep Harvester-specific assets and naming in `harvester_app`; only add generic image/window-icon primitives to `CommanDuctUI`. |
| Palette drifts from visual spec | Use existing warm dark and accent tokens as source colors. |
| Raster-only source blocks future scaling | Make SVG the source of truth for the mark. |

## Acceptance Checklist

Identity:

- [ ] The primary mark works at 16 px, 32 px, and 512 px.
- [ ] The mark works in full-color and monochrome.
- [ ] The mark is recognizable without text.
- [ ] The mark aligns with the warm dark visual system.
- [ ] The mark does not look like food, generic AI, cloud sync, or a document editor.

Application:

- [ ] The window/taskbar icon uses the production `.ico`.
- [ ] Splash startup is optional, non-blocking, and failure-tolerant.
- [ ] Empty-state or small UI images improve recognition without competing with content.
- [ ] No Harvester-specific concept is added to `CommanDuctUI`.
- [ ] Any `CommanDuctUI` change updates its version and changelog.

Engineering:

- [ ] Source assets and prompts are committed or otherwise documented.
- [ ] Generated assets are reproducible.
- [ ] Regression tests cover any emitted platform commands or reducer-visible behavior that changes.
- [ ] `cargo build` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo fmt` has been run.

