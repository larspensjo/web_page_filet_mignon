//! Tauri-free desktop-window geometry conversions.

/// Converts a physical inner size reported by the host into rounded logical pixels.
pub fn logical_inner_size(
    physical_width: u32,
    physical_height: u32,
    scale_factor: f64,
) -> Option<(i32, i32)> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let width = (f64::from(physical_width) / scale_factor).round();
    let height = (f64::from(physical_height) / scale_factor).round();
    if width < 1.0 || height < 1.0 || width > f64::from(i32::MAX) || height > f64::from(i32::MAX) {
        return None;
    }
    Some((width as i32, height as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_inner_size_converts_to_logical_pixels_at_scaled_dpi() {
        assert_eq!(logical_inner_size(1440, 1080, 1.5), Some((960, 720)));
    }

    #[test]
    fn invalid_scale_factor_is_rejected() {
        assert_eq!(logical_inner_size(960, 720, 0.0), None);
        assert_eq!(logical_inner_size(960, 720, f64::NAN), None);
    }
}
