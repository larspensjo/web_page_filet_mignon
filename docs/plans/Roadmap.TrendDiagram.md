# Roadmap trend diagram

My main recommendation is to stop treating this as a generic multi-line chart and make it a "top movers over time" view. Right now the screenshots are hard to parse for two reasons: the renderer is very bare-bones, and the ranking logic favors historically large entities rather than what is interesting now.

The biggest wins, in order:

1. ~~Add axis labels before changing anything else. The view model already sends week labels, but the chart control does not render them at all, so the x-axis currently has no time context. The y-axis also has no numeric labels, which makes the dashed gridlines decorative rather than informative. That is the first fix I'd make.~~ **Done (2026-03-25).** x-axis week labels and y-axis tick values now rendered. Left margin is derived from measured label width. See [chart_handler.rs](../../src/CommanDuctUI/src/controls/chart_handler.rs).

2. ~~Replace the right-side legend with direct end labels for the top 3 to 5 lines, and de-emphasize the rest. The fixed legend column steals width from the plot and forces the user to constantly map color to text. For dense series, a neutral gray for secondary lines plus saturated color only for highlighted lines will read much better than ten equally loud colors. The current chart always uses a fixed 10-color palette and a 130px legend column, which is a big part of the clutter.~~ **Done (2026-03-25).** Legend replaced by endpoint labels with deterministic overlap resolution. Top 2 lines rendered with full color and 2 px pen; remaining 3 lines muted. Visible series reduced from 10 to 5. See [chart_handler.rs](../../src/CommanDuctUI/src/controls/chart_handler.rs) and [render.rs](../../crates/harvester_app/src/platform/ui/render.rs).

3. ~~Change the ranking from "largest over the whole window" to something closer to "most relevant now". In the current core logic, entities are sorted by total count across the whole window, then latest week count. That means a line with one big spike weeks ago can dominate the chart even if it is no longer active. For a Trends tab, I would sort by a recency-weighted score, week-over-week change, or current-week count with a minimum total threshold.~~ **Done (2026-03-25).** Sort replaced with deterministic recency score: `latest*100 + prev1*40 + prev2*20 + total`, tie-break on label ascending. See [trends.rs](../../crates/harvester_core/src/trends.rs).

4. Add a mode switch: `Absolute` vs `Momentum`. Absolute counts are useful, but they bury smaller rising entities. A normalized mode like "index first visible week = 100" or "share of mentions this week" would make the Products screenshot much easier to interpret.

5. ~~Show one short summary above the chart. Something like: `Top products, last 13 weeks, ranked by current momentum.` Without that, users have to infer what the chart is trying to optimize.~~ **Done (2026-03-25).** Static label "Top 5 products by recent activity, last 13 weeks" rendered above the chart in app layout. See [layout.rs](../../crates/harvester_app/src/platform/ui/layout.rs).

## Completed: smallest high-value implementation pass (2026-03-25)

All five items from the original pass recommendation are done:
- ✅ render x-axis week labels and y-axis values
- ✅ reduce visible series from 10 to 5 by default
- ✅ gray out non-focused lines
- ✅ replace the legend with endpoint labels
- ✅ change sort order to favor current activity, not cumulative history

## Remaining ideas

- Configurable time windows (4w, 13w, 26w, 1y)
- `Absolute` vs `Momentum` ranking modes (item 4 above)
- Weighted counts by triage priority
- Alias canonicalization
- Smoothing overlay
- "New entrant" markers
- Annotations for spikes and dips
- CSV export
