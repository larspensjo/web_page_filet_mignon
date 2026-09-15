# Harvester Identity Sheet

The Harvester mark is three pages flowing into a funnel: many sources in,
one structured output. Chosen 2026-09-15 from three SVG candidates; the
solid-funnel variant won on 16 px legibility.

## Files

| File | Role |
|------|------|
| `harvester-mark.svg` | Full-colour mark, transparent background. Use in docs and in-app on dark surfaces. |
| `harvester-mark-mono.svg` | One-colour mark using `currentColor`. Use where the mark must follow text colour (light docs, status rows). |
| `harvester-icon.svg` | App icon: the mark on a rounded dark square. Source of truth for every raster icon. |
| `render.py` | Renders the PNG ladder and the multi-size `.ico` into `../generated/` and copies the app icons into `crates/harvester_ui/icons/`. |

Generated outputs are committed so a checkout builds without running the
renderer. Re-run `python assets/identity/source/render.py` after editing any
SVG. It needs Microsoft Edge (headless rasterizer) and Pillow.

## Colours

All colours are design tokens from `docs/visual_design/VisualDesignSpec.md`.

| Element | Token | Hex |
|---------|-------|-----|
| Funnel | Accent Primary | `#c96442` |
| Centre page | Text Primary | `#faf9f5` |
| Outer pages | Text Secondary | `#b0aea5` |
| Icon tile | Surface | `#1e1e1c` |

## Rules

- The funnel is the one accent shape. Do not add a second accent colour.
- Pages stay plain rectangles. No text lines inside them; they vanish below 32 px.
- Keep the 64-unit viewBox and the current padding so exports stay aligned.
- Do not add gradients, glow, shadows, or a wordmark inside the icon tile.
- The mark must never suggest food; the repository name is not part of the identity.
- Prefer the mark left of the "Harvester" title in the app header, small and quiet. No logo in work surfaces.
