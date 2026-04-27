# Visual Design Spec

This document defines the target visual system for the Harvester desktop UI. It is the single authoritative reference for both design principles and concrete implementation values.

The target aesthetic is a contemporary, dense expert-tool: calm, restrained, highly scannable, and optimized for long working sessions on a warm dark theme.

## Design Intent

- Preserve information density without making the interface feel cramped.
- Use typography, spacing, and tone to create hierarchy before adding borders.
- Keep the UI dark with warm-toned neutrals throughout.
- Make the active task, selected item, and primary action obvious within a few seconds.

## Core Principles

### 1. Calm density

The application should feel capable and data-rich, not sparse. Dense does not mean noisy. Every visible element must earn its emphasis.

### 2. One accent system

Use one primary accent color for selection, active tabs, progress, and primary actions. Use one reserved warning color only for destructive or high-attention states.

### 3. Hierarchy through type and spacing

Favor contrast in size, weight, spacing, and tone over decorative framing. If two elements are different in importance, that difference should be visible immediately.

### 4. Flat, not ornamental

Do not use raised bevels, recessed grooves, glow effects, or strong simulated lighting. Depth should come from tone and layout, not from skeuomorphic shadow tricks.

## Color System

All neutrals carry warm yellow-brown undertones. No cool blue-grays anywhere in the palette. Avoid pure black (`#000000`) and pure white (`#ffffff`).

### Surfaces

| Token | Hex | Role |
|-------|-----|------|
| Base Dark | `#141413` | App background |
| Surface | `#1e1e1c` | Panel backgrounds |
| Surface Raised | `#30302e` | Reading pane, elevated containers |
| Surface Overlay | `#3d3d3a` | Tooltips, dropdowns, popovers |

Adjacent surfaces should differ subtly but clearly. Use tonal steps to separate header, list pane, and reading pane without relying on heavy borders.

### Text

| Token | Hex | Role |
|-------|-----|------|
| Text Primary | `#faf9f5` | Headings, row titles, primary content |
| Text Secondary | `#b0aea5` | Body text, descriptions |
| Text Tertiary | `#87867f` | Metadata, timestamps, muted labels |
| Text Disabled | `#5e5d59` | Disabled controls, placeholder text |

### Accent

| Token | Hex | Role |
|-------|-----|------|
| Accent Primary | `#c96442` | Active tabs, selection, primary buttons, progress |
| Accent Hover | `#d97757` | Hover state for accent elements |
| Accent Warning | `#b53333` | Destructive actions, stop, high-risk states |

Routine headers should not use warning colors. Status indicators should become vivid only when they require attention. Secondary semantic colors are allowed sparingly where they materially improve scan speed, such as priority levels in triage.

### Borders and Rings

| Token | Hex | Role |
|-------|-----|------|
| Border Default | `#30302e` | Standard panel and card borders |
| Border Subtle | `#2a2a28` | Faint separators where needed |
| Ring Focus | `#c96442` | Focus ring on interactive elements |
| Ring Subtle | `#3d3d3a` | Hover ring on secondary controls |

## Depth and Elevation

Depth comes from tonal differences and ring shadows, not drop shadows or skeuomorphic effects.

| Level | Treatment | Use |
|-------|-----------|-----|
| Flat | No border or shadow | Base Dark background areas |
| Contained | `1px solid` Border Default | Standard panels, cards |
| Ring | `0px 0px 0px 1px` ring shadow | Interactive hover and focus states |
| Whisper | `rgba(0,0,0,0.15) 0px 4px 16px` | Elevated overlays, dropdowns |

Do not use inset shadows, glow effects, or heavy drop shadows.

## Typography

Typography should carry a large share of the visual hierarchy.

### Font family

- UI controls, headings, and reading text: a clean contemporary sans-serif such as Inter, Segoe UI, or system-ui.
- Monospaced type: only where tabular alignment or fixed-width data genuinely benefits from it.

### Type scale

| Role | Size | Weight | Line Height | Color Token |
|------|------|--------|-------------|-------------|
| Page Title | 24px | 600 | 1.25 | Text Primary |
| Section Title | 18px | 600 | 1.30 | Text Primary |
| Row Title | 14px | 500 | 1.25 | Text Primary |
| Body | 14px | 400 | 1.50 | Text Secondary |
| Metadata | 12px | 400 | 1.40 | Text Tertiary |
| Label / Badge | 11px | 500 | 1.25 | varies |

### Guidance

- Use off-white (Text Primary) rather than pure white for body text.
- Reserve bold weight for actual hierarchy, not for routine labels.
- Increase line height in reading views to 1.50 or above.
- Keep long-form reading width controlled to roughly 50 to 75 characters.

## Surfaces and Separation

Major layout regions should be separated primarily through spacing and tonal contrast.

Use:

- Different surface tokens between header, list pane, and reading pane.
- Thin dividers only where panel boundaries are otherwise unclear.
- Interior padding to create breathing room inside panes.

Avoid:

- Heavy box outlines around every region.
- Inset or engraved panel effects.
- Multiple nested frames inside a single pane.

## Tabs

Tabs should be visually light and immediately legible.

Use:

- Flat tab styling.
- Active-state emphasis through underline, accent tint, or stronger text weight.
- Lower-contrast inactive tabs using Text Tertiary.

Avoid:

- Heavy boxed tabs.
- 3D tab treatments.
- Multiple competing active indicators.

## Buttons and Actions

Buttons should clearly express priority and intent.

### Button hierarchy

| Variant | Background | Text | Border | Radius |
|---------|-----------|------|--------|--------|
| Primary | Accent Primary | Text Primary | none | 6px |
| Secondary | transparent | Text Secondary | `1px solid` Border Default | 6px |
| Ghost | transparent | Text Secondary | none | 6px |
| Destructive | Accent Warning | Text Primary | none | 6px |

### Guidance

- Establish one clear primary action per context and visually demote the rest.
- Add spacing between bottom-bar actions and visually separate destructive actions from constructive ones.
- Support clear visual treatment for default, hover, focus, active, selected, and disabled states.
- State changes should be visible through tone, accent, or fill changes rather than through heavy animation or dramatic shadow shifts.

## Links

Links should feel clearly interactive without introducing a second accent system or adding visual noise to dense reading surfaces.

### Link styling

| State | Color | Decoration | Notes |
|-------|-------|------------|-------|
| Default | `#c96442` | none | Use Accent Primary for standalone links and key metadata links |
| Hover | `#d97757` | underline | Hover should strengthen affordance without changing layout |
| Focus | `#d97757` | underline + focus ring | Use Ring Focus for keyboard focus |
| Active | `#d97757` | underline | Keep the pressed state subtle and flat |
| Disabled / Unavailable | `#5e5d59` | none | Use only when a link-shaped control cannot currently be activated |

### Guidance

- Use links for lightweight navigation actions embedded in content or metadata, not for primary workflow actions.
- Prefer link-styled controls near the content they act on, such as an article source line at the top of the reading pane.
- In reading surfaces, avoid persistent underlines by default; reserve underlines for hover and focus so text remains calm and scannable.
- In metadata-heavy areas, pair Text Tertiary labels with an Accent Primary link target rather than coloring the whole line.
- Do not introduce a separate visited-link color unless the screen materially benefits from browsing history; the default system should preserve one accent language.

Avoid:

- Bright blue web-style links that break the warm palette.
- Underlining large volumes of body text by default.
- Styling primary actions as links when they should read as buttons.
- Adding icons, glow, or ornamental treatments unless they solve a real recognition problem.

## Lists and Triage Rows

Lists are scan surfaces, not paragraph surfaces.

Use:

- Slightly increased row padding for readability.
- Clear separation between row title and supporting metadata.
- Distinct visual treatment for priority, category, and tags.
- Strong selected-state styling using Accent Primary.

Recommended row structure:

- Priority badge with semantic color.
- Category label.
- Short title.
- Secondary metadata or tags only if they do not overwhelm scanning.

Guidance:

- Use semantic color sparingly to accelerate triage by urgency.
- Prefer badges or pills over dense bracketed inline metadata.
- Move overflow detail such as long tag lists or URLs out of the default row view.
- Align row structure so titles start from a common visual column.

## Reading Pane

The reading pane should behave like an editorial surface inside a tool.

Use:

- Clear distinction between document title, section headings, and body text via the type scale.
- Comfortable line height (1.50+).
- Generous inner padding (16-24px).
- Constrained line length of 50 to 75 characters where possible.
- Surface Raised background to differentiate from adjacent list pane.

Avoid:

- Treating the reading pane as a raw data console unless the content truly requires it.
- Full-width text blocks with minimal margins.

## Status Indicators and Progress

Status information should remain visible without dominating the screen.

Use:

- A single clearly labeled token or budget meter.
- Muted default presentation using Text Tertiary.
- Color escalation to Accent Primary or Accent Warning only near important thresholds.

Avoid:

- Bright status components competing with core content.
- Separate numeric and bar treatments that feel redundant.
- Glows, neon effects, or ornamental progress styling.

## Spacing System

Use a consistent spacing rhythm based on 4px increments.

| Increment | Use |
|-----------|-----|
| 4px | Tight internal gaps |
| 8px | Compact internal spacing |
| 12px | Row and control spacing |
| 16px | Standard pane padding |
| 24px | Major pane padding, section separation |

Dense interfaces still need breathing room at panel edges. Increase spacing around controls before increasing border strength.

## Border Radius

Use a restrained radius scale. Cap at 8px.

| Radius | Use |
|--------|-----|
| 2px | Inline badges, tiny elements |
| 4px | Standard buttons, inputs, small cards |
| 6px | Primary buttons, prominent controls |
| 8px | Cards, containers, panels |

Avoid large rounded treatments (12px+). The tool should feel precise, not playful.

## Quick Visual Checklist

When evaluating any screen, ask:

1. Is the active task obvious within two seconds?
2. Is there only one dominant accent system?
3. Are list rows scannable before they are fully read?
4. Does the reading surface support comfortable long-form reading?
5. Are status elements informative without stealing focus?
6. Are borders solving a real problem, or compensating for weak spacing and hierarchy?
7. Do all neutrals maintain warm undertones?

## Non-Goals

This design system should not drift toward:

- Neumorphism
- Heavy skeuomorphic depth
- Decorative glow effects
- Retro terminal aesthetics applied indiscriminately
- Bright multi-accent dashboards unless the screen is truly an operational monitoring view
- Cool blue-gray palettes

## Summary

The target UI should feel like a modern professional desktop tool: dense, serious, and efficient, but visually calm. The interface should communicate structure through typography, spacing, and restrained warm color, not through bevels, grooves, or excessive chrome.
