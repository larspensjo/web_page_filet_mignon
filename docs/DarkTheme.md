# Specification of the dark theme to use

This design focuses on **tactile depth**.

## The Global Lighting Model
*   **Concept:** Imagine a light source coming from the **Top-Left**.
*   **The Rule:**
    *   **Raised Elements (Buttons):** Light highlight on Top/Left, Dark shadow on Bottom/Right.
    *   **Recessed Elements (Text boxes, List views):** Dark shadow on Top/Left (inside), Light highlight on Bottom/Right (inside).

## The "Soft" Button (Archive / Stop)
Instead of a standard 1px border, you need to draw two soft shadows.
*   **Fill:** Solid color `#2E3239` (same as background).
*   **Shadow 1 (Top-Left):** White (`#FFFFFF`) at ~5-10% opacity, blurred offset (-4, -4).
*   **Shadow 2 (Bottom-Right):** Black (`#000000`) at ~40% opacity, blurred offset (+4, +4).
*   **Active State:** When the user clicks, invert the shadows (Inner Shadow) to make it look physically pressed down.

## The "Channel" Progress Bar
The progress bar shouldn't look like a sticker on top; it should look like a groove cut into the chassis.
*   **The Groove:** Draw an **Inner Shadow** on the container rectangle (Dark shadow Top/Left, Light highlight Bottom/Right).
*   **The Liquid:** Draw a gradient bar inside the groove (e.g., `#00C9FF` to `#0080FF`). Add a slight "Glow" (BoxShadow with the same color) to make it look like neon liquid.

## The Panels (Job List & Preview)
*   **Style:** "Inset" / Engraved.
*   **Implementation:**
    *   Fill the panel background with a slightly darker grey than the window (`#262A2E`).
    *   Apply an **Inner Shadow** only to the Top and Left edges to create the feeling of depth.
    *   Remove standard 1px borders; the shadow defines the edge.

## Typography & Syntax Highlighting
*   Since the UI is dark, pure white text (`#FFFFFF`) causes eye strain (halation).
*   **Base Text:** Use Off-White (`#E0E5EC`).
*   **URLs/Links:** Cyan or Soft Blue.
*   **Headers:** Amber or Soft Orange.
*   **Font:** A monospaced font like *Cascadia Code* or *JetBrains Mono* is essential for the "Preview" pane to keep the data structured.
