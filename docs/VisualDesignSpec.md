# Visual Design Spec

Date: 2026-04-02

This document defines the target visual system for the Harvester desktop UI. It replaces the older dark-theme guidance that emphasized tactile depth, recessed grooves, and heavy shadow modeling.

The new direction is a contemporary, dense expert-tool aesthetic: calm, restrained, highly scannable, and optimized for long working sessions.

## Design Intent

- Preserve information density without making the interface feel cramped.
- Use typography, spacing, and tone to create hierarchy before adding borders.
- Keep the UI dark, but avoid the ornamental depth effects of neumorphism or heavy beveling.
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

### Neutrals

- App background: a deep neutral charcoal.
- Panel surfaces: one or two slightly lighter neutral steps.
- Reading surface: optionally one step lighter than adjacent tool surfaces.
- Borders: low-contrast neutral lines only where needed.

Guidance:

- Adjacent surfaces should differ subtly but clearly.
- Avoid pure black and pure white.
- Default text should use a soft off-white to reduce eye strain.

### Accent colors

- Primary accent: used for active tabs, selected states, focused controls, primary buttons, and progress.
- Warning accent: used for stop, destructive, or high-risk actions only.
- Secondary semantic colors: allowed only where they materially improve scan speed, such as priority levels in triage.

Guidance:

- Routine headers should not use warning colors.
- Avoid multiple unrelated bright accents on the same screen.
- Status indicators should become more vivid only when they require attention.

## Surfaces and Separation

Major layout regions should be separated primarily through spacing and tonal contrast.

Use:

- Slightly different background tones between header, list pane, and reading pane.
- Thin dividers only where panel boundaries are otherwise unclear.
- Interior padding to create breathing room inside panes.

Avoid:

- Heavy box outlines around every region.
- Inset or engraved panel effects.
- Multiple nested frames inside a single pane.

## Typography

Typography should carry a large share of the hierarchy.

### Type roles

- Page title: clearly dominant.
- Section title: distinct from page title, still prominent.
- Row title: primary text in lists.
- Metadata: secondary in size, weight, or contrast.
- Body text: optimized for sustained reading.

Guidance:

- Use off-white rather than pure white for body text.
- Reserve bold weight for actual hierarchy, not for routine labels.
- Increase line height in reading views.
- Keep long-form reading width controlled rather than spanning the full pane.

### Font direction

- UI controls and reading text should use a clean, contemporary UI face.
- Monospaced type should be used only where tabular alignment or fixed-width data genuinely benefits from it.
- Do not force monospaced typography across the full reading experience.

## Tabs

Tabs should be visually light and immediately legible.

Use:

- Flat tab styling.
- Active-state emphasis through underline, tint, fill, or stronger text weight.
- Lower-contrast inactive tabs.

Avoid:

- Heavy boxed tabs.
- 3D tab treatments.
- Multiple competing active indicators.

## Buttons and Actions

Buttons should clearly express priority and intent.

Use:

- One primary filled action per context when applicable.
- Secondary actions as outline or ghost treatments.
- Warning/destructive actions with reserved warning styling.
- Subtle corner radius to soften the interface without making it playful.

Avoid:

- Giving every button equal visual weight.
- Thick hard borders on all actions.
- Soft-shadow tactile button styling.

## Lists and Triage Rows

Lists are scan surfaces, not paragraph surfaces.

Use:

- Slightly increased row padding for readability.
- Clear separation between row title and supporting metadata.
- Distinct visual treatment for priority, category, and tags.
- Strong selected-state styling.

Recommended row structure:

- Priority badge.
- Category label.
- Short title.
- Secondary metadata or tags only if they do not overwhelm scanning.

Guidance:

- Use semantic color sparingly to accelerate triage.
- Prefer badges or pills over dense bracketed inline metadata.
- Move overflow detail such as long tag lists or URLs out of the default row view when practical.

## Reading Pane

The reading pane should behave like an editorial surface inside a tool.

Use:

- Clear distinction between document title, section headings, and body text.
- Comfortable line height.
- Generous inner padding.
- Constrained line length where possible.

Avoid:

- Treating the reading pane like a raw data console unless the content truly requires that mode.
- Full-width text blocks with minimal margins.

## Status Indicators and Progress

Status information should remain visible without dominating the screen.

Use:

- A single clearly labeled token or budget meter.
- Muted default presentation.
- Color escalation only near important thresholds.

Avoid:

- Bright status components competing with core content.
- Separate numeric and bar treatments that feel redundant or visually noisy.
- Glows, neon liquid effects, or ornamental progress styling.

## Spacing System

Use a consistent spacing rhythm.

Recommended base increments:

- 8 px for compact internal spacing.
- 12 px for row and control spacing.
- 16 px to 24 px for pane padding and major separations.

Guidance:

- Dense interfaces still need breathing room at panel edges.
- Increase spacing around controls before increasing border strength.

## State Model

Interactive states must be easy to distinguish.

Support clear visual treatment for:

- Default
- Hover
- Focus
- Active
- Selected
- Disabled
- Warning or destructive

State changes should be visible through tone, accent, or fill changes rather than through heavy animation or dramatic shadow shifts.

## Quick Visual Checklist

When evaluating any screen, ask:

1. Is the active task obvious within two seconds?
2. Is there only one dominant accent system?
3. Are list rows scannable before they are fully read?
4. Does the reading surface support comfortable long-form reading?
5. Are status elements informative without stealing focus?
6. Are borders solving a real problem, or compensating for weak spacing and hierarchy?

## Non-Goals

This design system should not drift toward:

- Neumorphism
- Heavy skeuomorphic depth
- Decorative glow effects
- Retro terminal aesthetics applied indiscriminately
- Bright multi-accent dashboards unless the screen is truly an operational monitoring view

## Inspiration
Can use a color scheme from https://github.com/VoltAgent/awesome-design-md.

## Summary

The target UI should feel like a modern professional desktop tool: dense, serious, and efficient, but visually calm. The interface should communicate structure through typography, spacing, and restrained color, not through bevels, grooves, or excessive chrome.
