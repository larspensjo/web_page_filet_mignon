# Look-and-Feel Redesign Brief

Date: 2026-04-02

Scope: This brief is based only on the application screenshot. It intentionally does not rely on source code, implementation details, or project documentation.

## Overall Assessment

The application appears capable, information-dense, and oriented toward expert users. The current look-and-feel communicates "serious internal tool" more strongly than "modern desktop product." Relative to contemporary dense productivity applications such as Linear, Figma side panels, Raycast, Notion Calendar, and modern observability dashboards, the main issue is not missing functionality. The main issue is visual hierarchy.

Too many elements carry similar visual weight at the same time: panel outlines, tab bars, headings, button rows, colored accents, dense list rows, and large reading surfaces. The result is functional but visually tiring. A redesign should preserve density and speed while reducing friction, clarifying emphasis, and making the interface feel more intentional.

## Design Direction

Target a "dense expert desktop tool" aesthetic rather than a sparse consumer layout. The goal is not to remove information. The goal is to make the information easier to scan and the application easier to trust at a glance.

Reference characteristics from contemporary applications:

- Fewer hard separators and more spatial grouping.
- A restrained accent color system with one clear primary accent.
- Stronger typography hierarchy instead of relying on borders and bold text everywhere.
- More deliberate use of row spacing, muted secondary metadata, and selection states.
- Buttons and tabs that feel lighter, clearer, and more stateful.
- Editorial reading surfaces with controlled line length.
- Dense lists that prioritize skimming over exhaustive in-row detail.

## Five Improvement Suggestions

### 1. Reduce visual chrome and separator density

The current screen uses many borders, divider lines, framed regions, and enclosed controls. That creates a boxed-in feeling and makes the interface look older than it is.

Recommendation:

- Remove non-essential outlines around major panels and inner regions.
- Use background tone, spacing, and subtle elevation to distinguish sections instead of drawing every boundary.
- Reserve strong borders for active, selected, or interactive elements only.

Expected outcome:

The UI will feel calmer, more contemporary, and less mechanically partitioned without losing structure.

### 2. Consolidate the color language

The screenshot shows several competing accents: purple window chrome, orange headers, cyan progress, blue tab emphasis, and white text highlights. Modern interfaces usually establish one primary accent and let neutrals do most of the compositional work.

Recommendation:

- Choose a single primary accent color for active tabs, primary actions, and progress indicators.
- Use one semantic warning/emphasis color only when necessary, rather than for routine headers.
- Shift more non-interactive framing elements into a narrow neutral range.
- Use varying dark neutral surfaces to create depth between toolbars, list panes, and reading panes.
- Soften the token meter and progress treatment so it informs without dominating the screen.

Expected outcome:

The application will feel more cohesive and visually confident. Users will also find it easier to tell what is actionable versus what is merely decorative or structural.

### 3. Rebuild hierarchy through typography

The interface relies heavily on panel structure and contrast blocks, while the type system appears relatively flat. The right-hand reading pane contains large amounts of content, but the hierarchy between title, section header, summary, and body copy could be clearer.

Recommendation:

- Define a clearer type scale for page title, section title, row title, metadata, and body text.
- Increase line height in reading views to improve sustained readability.
- Reduce the use of bold text for routine labels so that emphasis is meaningful.
- Treat metadata as secondary through smaller size or lower contrast rather than crowding it into the same visual tier as primary content.
- Constrain the reading column width so the body text does not stretch too far across wide panes.
- Increase the separation between the main page title and subordinate section headings.

Expected outcome:

Users will be able to identify structure faster and read long-form briefings with less fatigue.

### 4. Make the triage list more scannable

The left pane appears powerful but dense to the point of strain. Each row includes multiple signals, tags, and long text fragments, yet the rows are packed tightly and look visually similar.

Recommendation:

- Increase row padding slightly to reduce crowding.
- Separate primary text from secondary metadata more clearly.
- Lower the contrast of tags and supporting metadata so the core item label leads.
- Strengthen the selected-row treatment so it is unmistakable.
- Consider limiting the number of simultaneously prominent signals per row and demoting the rest.
- Represent priority, category, and tags as distinct visual elements rather than as a continuous line of bracketed text.
- Use semantic color for priority levels so the list can be scanned by urgency before it is read in detail.
- Consider showing only priority, category, and a short title in the default row view, with longer tags and URLs deferred to selection, hover, or the detail pane.
- Align row structure with a fixed priority area so titles start from a common visual column.
- Use subtle row separators or alternating row tone only if spacing alone is insufficient.

Expected outcome:

The triage workflow will feel faster because users can visually skim for patterns instead of parsing each row in sequence.

### 5. Modernize controls, tabs, and action emphasis

The bottom action row and multiple tab bands feel functional but heavy. Buttons look similar in importance, and state changes do not strongly shape the interface.

Recommendation:

- Establish one clear primary action per context and visually demote the rest.
- Use lighter button treatments for secondary actions.
- Increase clarity of active, hover, pressed, and disabled states.
- Simplify tab styling so active context is obvious without high visual weight.
- Reconsider whether every command needs permanent exposure, or whether some actions can move into overflow or context-specific surfaces.
- Flatten tab styling and rely on underline, tint, or fill to indicate the active tab instead of heavy boxed treatments.
- Add spacing between bottom-bar actions and visually separate stop-oriented actions from constructive workflow actions.
- If one action is dominant in the workflow, render it as a filled primary button and keep the others as outline or ghost variants.

Expected outcome:

The interface will feel less like a control panel and more like a modern task-focused workspace.

### 6. Reframe the token meter and status signaling

The token count and progress bar are highly visible relative to the rest of the interface. In the screenshot they compete with the actual working surfaces for attention, even though they appear to be supporting status information rather than the main task.

Recommendation:

- Merge the number and progress indicator into one clearly labeled status component.
- Reduce the visual intensity of the meter until usage becomes notable.
- Consider moving this element into a status-bar treatment or otherwise integrating it more quietly into the header.
- Use threshold-based color changes only when the metric becomes important enough to deserve interruption.

Expected outcome:

Users will still have access to operational budget information without having the header feel like a monitoring dashboard.

## Practical Visual Spec

If redesign work begins, use the following guardrails:

- Keep the dark theme, but narrow the contrast range between adjacent panels.
- Prefer 8 px or 12 px spacing increments for a more deliberate rhythm.
- Use one accent color plus neutrals and one reserved alert color.
- Limit strong borders to selected, focused, and interactive states.
- Treat reading panes as editorial surfaces with more line height and stronger text hierarchy.
- Treat data lists as scan surfaces with clearer selected states and quieter metadata.
- Add more interior padding to major panes and around bottom controls.
- Use subtle corner radius on interactive controls to soften the older desktop feel without making the tool feel playful.
- Prefer badges or pills over bracketed inline metadata where visual parsing matters.
- Keep body text in a readable measure of roughly 50 to 75 characters when practical.
- Differentiate adjacent surfaces through depth and tone before adding more divider lines.

## Suggested Priority Order

1. Simplify color and border usage.
2. Improve type hierarchy in the briefing pane.
3. Make triage rows easier to scan.
4. Rebalance tabs, buttons, and status indicators.
5. Fine-tune spacing, states, and polish.

## Quick Wins

If only a small visual pass is possible, prioritize these changes first:

1. Add semantic styling to priority labels in the triage list.
2. Differentiate row title text from tags and metadata.
3. Add padding around the right-hand reading pane and bottom action bar.
4. Flatten the tabs and make the active state clearer.
5. Quiet the token meter so it stops competing with the main content.

## Success Criteria

The redesign is successful if the application:

- Feels calmer without feeling less capable.
- Makes the active task more obvious within two seconds of opening the app.
- Improves scan speed in the triage pane.
- Improves readability in the briefing pane.
- Uses fewer competing visual accents while preserving clarity.
- Makes priorities and selection state recognizable before the user reads the full row text.
- Keeps status information available without making it the focal point of the header.

## Summary

The current UI already communicates seriousness and functionality. The redesign opportunity is to retain its density and power while replacing visual heaviness with clearer hierarchy, stronger restraint, and more deliberate emphasis. That is the difference between a competent internal tool and a contemporary professional desktop application.
