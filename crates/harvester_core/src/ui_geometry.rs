//! UI geometry and layout calculation helpers.
//!
//! This module provides pure functions for computing UI dimensions and
//! clamping values to maintain layout invariants.

/// Clamps the desired left panel width to respect minimum width constraints.
///
/// This function ensures that:
/// - The left panel is at least `min_left_px` wide
/// - The preview panel (right side) is at least `min_preview_px` wide
/// - The splitter's total width (including margins) is accounted for
///
/// The maximum left width is dynamically calculated as:
/// ```text
/// max_left = window_width - min_preview - splitter_total
/// ```
///
/// # Parameters
///
/// - `desired_left_px`: The requested left panel width (e.g., from splitter drag)
/// - `window_width_px`: Total available window width
/// - `min_left_px`: Minimum allowed width for the left panel
/// - `min_preview_px`: Minimum allowed width for the preview (right) panel
/// - `splitter_total_px`: Total width consumed by the splitter (bar width + margins)
///
/// # Returns
///
/// The clamped left panel width that satisfies all constraints.
///
/// # Example
///
/// ```
/// use harvester_core::calc_left_width;
///
/// // Window is 1200px wide, splitter takes 16px total (4px bar + 12px margins)
/// let clamped = calc_left_width(800, 1200, 200, 200, 16);
/// // max_left = 1200 - 200 - 16 = 984
/// // 800 is within [200, 984], so result is 800
/// assert_eq!(clamped, 800);
///
/// // User tries to drag left panel too wide
/// let clamped = calc_left_width(1100, 1200, 200, 200, 16);
/// // 1100 exceeds max_left of 984, clamped to 984
/// assert_eq!(clamped, 984);
///
/// // User tries to drag left panel too narrow
/// let clamped = calc_left_width(100, 1200, 200, 200, 16);
/// // 100 is below min_left of 200, clamped to 200
/// assert_eq!(clamped, 200);
/// ```
pub fn calc_left_width(
    desired_left_px: i32,
    window_width_px: i32,
    min_left_px: i32,
    min_preview_px: i32,
    splitter_total_px: i32,
) -> i32 {
    let max_left = window_width_px - min_preview_px - splitter_total_px;

    // If the window is too small to satisfy both minimums, we prioritize
    // the preview panel minimum and allow the left panel to be smaller than ideal
    if max_left < min_left_px {
        // Constrain to what's physically available
        desired_left_px.min(max_left).max(0)
    } else {
        // Normal case: clamp between min and max
        desired_left_px.clamp(min_left_px, max_left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_left_width_within_bounds() {
        // Desired width is within valid range
        let result = calc_left_width(800, 1200, 200, 200, 16);
        assert_eq!(result, 800);
    }

    #[test]
    fn test_calc_left_width_exceeds_max() {
        // Desired width is too large (would violate min_preview)
        let result = calc_left_width(1100, 1200, 200, 200, 16);
        // max_left = 1200 - 200 - 16 = 984
        assert_eq!(result, 984);
    }

    #[test]
    fn test_calc_left_width_below_min() {
        // Desired width is too small (below min_left)
        let result = calc_left_width(100, 1200, 200, 200, 16);
        assert_eq!(result, 200);
    }

    #[test]
    fn test_calc_left_width_tiny_window() {
        // Window is very small - constraints conflict
        // min_left=200, min_preview=200, splitter=16 => need at least 416px
        // With only 400px, we can't satisfy both minimums
        // The clamp will prefer min_left, but max_left = 400 - 200 - 16 = 184
        let result = calc_left_width(300, 400, 200, 200, 16);
        // max_left = 184, which is less than min_left = 200
        // clamp(300, 200, 184) = 184 (clamps to the max)
        assert_eq!(result, 184);
    }

    #[test]
    fn test_calc_left_width_at_boundaries() {
        // Test exact boundary values
        let result = calc_left_width(200, 1200, 200, 200, 16);
        assert_eq!(result, 200); // At min

        let result = calc_left_width(984, 1200, 200, 200, 16);
        assert_eq!(result, 984); // At max (1200 - 200 - 16)
    }

    #[test]
    fn test_calc_left_width_different_splitter_sizes() {
        // Larger splitter consumes more space
        let result = calc_left_width(900, 1200, 200, 200, 50);
        // max_left = 1200 - 200 - 50 = 950
        assert_eq!(result, 900);

        let result = calc_left_width(970, 1200, 200, 200, 50);
        // 970 > 950, clamped to 950
        assert_eq!(result, 950);
    }
}
