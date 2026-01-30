use chrono::NaiveDate;

/// How confident we are in a heuristic age estimate for a link.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeEstimateConfidence {
    Low,
    Medium,
    High,
}

/// Source that produced an age estimate for a link.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeEstimateSource {
    UrlPattern,
    AnchorText,
    DownloadedMetadata,
}

/// A heuristic estimate of when a link was published or last updated.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgeEstimate {
    pub date: NaiveDate,
    pub confidence: AgeEstimateConfidence,
    pub source: AgeEstimateSource,
}

const YEAR_MIN: i32 = 1900;
const YEAR_MAX: i32 = 2100;

pub fn guess_age_from_url(url: &str) -> Option<AgeEstimate> {
    let candidate = find_date_in_url(url)?;
    Some(AgeEstimate {
        date: candidate,
        confidence: AgeEstimateConfidence::High,
        source: AgeEstimateSource::UrlPattern,
    })
}

fn find_date_in_url(url: &str) -> Option<NaiveDate> {
    let cleaned = strip_query_and_fragment(url);
    find_date_in_slash_segments(cleaned)
        .or_else(|| find_date_in_hyphen_pattern(cleaned))
        .or_else(|| find_date_in_compact_pattern(cleaned))
}

fn strip_query_and_fragment(url: &str) -> &str {
    let without_fragment = url.split('#').next().unwrap_or(url);
    without_fragment.split('?').next().unwrap_or(without_fragment)
}

fn find_date_in_slash_segments(input: &str) -> Option<NaiveDate> {
    let segments: Vec<&str> = input.split('/').collect();
    for window in segments.windows(3) {
        if let (Some(year), Some(month), Some(day)) = (
            parse_year_segment(window[0]),
            parse_month_segment(window[1]),
            parse_day_segment(window[2]),
        ) {
            if let Some(date) = date_from_parts(year, month, day) {
                return Some(date);
            }
        }
    }
    None
}

fn parse_year_segment(segment: &str) -> Option<i32> {
    parse_fixed_digits(segment, 4).map(|value| value as i32)
}

fn parse_month_segment(segment: &str) -> Option<u32> {
    parse_fixed_digits(segment, 2)
}

fn parse_day_segment(segment: &str) -> Option<u32> {
    parse_leading_digits(segment, 2)
}

fn find_date_in_hyphen_pattern(input: &str) -> Option<NaiveDate> {
    let bytes = input.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    for start in 0..=bytes.len() - 10 {
        if has_digit_before(bytes, start) || has_digit_after(bytes, start + 10) {
            continue;
        }
        if !is_digit(bytes[start])
            || !is_digit(bytes[start + 1])
            || !is_digit(bytes[start + 2])
            || !is_digit(bytes[start + 3])
            || bytes[start + 4] != b'-'
            || !is_digit(bytes[start + 5])
            || !is_digit(bytes[start + 6])
            || bytes[start + 7] != b'-'
            || !is_digit(bytes[start + 8])
            || !is_digit(bytes[start + 9])
        {
            continue;
        }
        let year = parse_digits(&bytes[start..start + 4]).map(|value| value as i32);
        let month = parse_digits(&bytes[start + 5..start + 7]);
        let day = parse_digits(&bytes[start + 8..start + 10]);
        if let (Some(year), Some(month), Some(day)) = (year, month, day) {
            if let Some(date) = date_from_parts(year, month, day) {
                return Some(date);
            }
        }
    }
    None
}

fn find_date_in_compact_pattern(input: &str) -> Option<NaiveDate> {
    let bytes = input.as_bytes();
    if bytes.len() < 8 {
        return None;
    }
    for start in 0..=bytes.len() - 8 {
        if has_digit_before(bytes, start) || has_digit_after(bytes, start + 8) {
            continue;
        }
        if !bytes[start..start + 8].iter().all(|byte| is_digit(*byte)) {
            continue;
        }
        let year = parse_digits(&bytes[start..start + 4]).map(|value| value as i32);
        let month = parse_digits(&bytes[start + 4..start + 6]);
        let day = parse_digits(&bytes[start + 6..start + 8]);
        if let (Some(year), Some(month), Some(day)) = (year, month, day) {
            if let Some(date) = date_from_parts(year, month, day) {
                return Some(date);
            }
        }
    }
    None
}

fn parse_fixed_digits(segment: &str, len: usize) -> Option<u32> {
    if segment.len() != len {
        return None;
    }
    let bytes = segment.as_bytes();
    if !bytes.iter().all(|byte| is_digit(*byte)) {
        return None;
    }
    parse_digits(bytes)
}

fn parse_leading_digits(segment: &str, len: usize) -> Option<u32> {
    let bytes = segment.as_bytes();
    if bytes.len() < len {
        return None;
    }
    if !bytes[..len].iter().all(|byte| is_digit(*byte)) {
        return None;
    }
    parse_digits(&bytes[..len])
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    let mut value: u32 = 0;
    for byte in bytes {
        if !is_digit(*byte) {
            return None;
        }
        value = value.saturating_mul(10).saturating_add((byte - b'0') as u32);
    }
    Some(value)
}

fn is_digit(byte: u8) -> bool {
    byte.is_ascii_digit()
}

fn has_digit_before(bytes: &[u8], index: usize) -> bool {
    index > 0 && is_digit(bytes[index - 1])
}

fn has_digit_after(bytes: &[u8], index: usize) -> bool {
    index < bytes.len() && is_digit(bytes[index])
}

fn date_from_parts(year: i32, month: u32, day: u32) -> Option<NaiveDate> {
    if !(YEAR_MIN..=YEAR_MAX).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_date(url: &str, expected: NaiveDate) {
        let estimate = guess_age_from_url(url).expect("expected date estimate");
        assert_eq!(estimate.date, expected);
        assert_eq!(estimate.confidence, AgeEstimateConfidence::High);
        assert_eq!(estimate.source, AgeEstimateSource::UrlPattern);
    }

    #[test]
    fn guess_age_from_url_parses_slash_pattern() {
        assert_date(
            "https://example.com/2024/01/05/article",
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
        );
    }

    #[test]
    fn guess_age_from_url_parses_slash_pattern_with_extension() {
        assert_date(
            "https://example.com/2024/01/05.html",
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
        );
    }

    #[test]
    fn guess_age_from_url_parses_hyphen_pattern() {
        assert_date(
            "https://example.com/news/2023-12-31/breaking.html",
            NaiveDate::from_ymd_opt(2023, 12, 31).unwrap(),
        );
    }

    #[test]
    fn guess_age_from_url_parses_hyphen_pattern_with_time_suffix() {
        assert_date(
            "https://example.com/news/2024-01-05T12:34:56",
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
        );
    }

    #[test]
    fn guess_age_from_url_parses_compact_pattern() {
        assert_date(
            "https://example.com/20240105/article",
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
        );
    }

    #[test]
    fn guess_age_from_url_ignores_invalid_date() {
        assert!(guess_age_from_url("https://example.com/2023/02/30/").is_none());
        assert!(guess_age_from_url("https://example.com/2023-13-01/").is_none());
    }

    #[test]
    fn guess_age_from_url_requires_digit_boundaries() {
        assert!(guess_age_from_url("https://example.com/202401051234").is_none());
    }
}
