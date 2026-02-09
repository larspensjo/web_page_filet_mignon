use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct BoilerplatePolicy {
    pub link_density_threshold: f64,
    pub min_nav_block_lines: usize,
    pub boundary_scan_lines: usize,
    pub max_removal_ratio: f64,
    pub known_patterns: Vec<String>,
}

impl Default for BoilerplatePolicy {
    fn default() -> Self {
        Self {
            link_density_threshold: 0.6,
            min_nav_block_lines: 3,
            boundary_scan_lines: 20,
            max_removal_ratio: 0.5,
            known_patterns: vec![
                "cookie policy".into(),
                "accept cookies".into(),
                "privacy preferences".into(),
                "all rights reserved".into(),
                "terms of service".into(),
                "copyright".into(),
                "cookies".into(),
                "cookie".into(),
            ],
        }
    }
}

pub struct BoilerplateResult {
    pub filtered_text: String,
    pub removed_line_count: usize,
    pub detected_patterns: Vec<String>,
}

pub fn filter_boilerplate(text: &str, policy: &BoilerplatePolicy) -> BoilerplateResult {
    if text.trim().is_empty() {
        return BoilerplateResult {
            filtered_text: text.to_string(),
            removed_line_count: 0,
            detected_patterns: Vec::new(),
        };
    }

    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    if total_lines < 2 {
        return BoilerplateResult {
            filtered_text: text.to_string(),
            removed_line_count: 0,
            detected_patterns: Vec::new(),
        };
    }

    let boundary_lines = if total_lines < 2 * policy.boundary_scan_lines {
        total_lines / 2
    } else {
        policy.boundary_scan_lines
    }
    .max(1);

    let mut filters = Vec::new();

    filters.extend(collect_boundary_filters(
        &lines[..boundary_lines],
        0,
        policy,
    ));
    filters.extend(collect_boundary_filters(
        &lines[total_lines - boundary_lines..],
        total_lines - boundary_lines,
        policy,
    ));

    let detected_patterns: Vec<String> = filters
        .iter()
        .filter_map(|filter| filter.pattern.clone())
        .collect();

    let mut removed_indices = Vec::new();
    for filter in &filters {
        removed_indices.extend(filter.indices.iter().copied());
    }

    removed_indices.sort_unstable();
    removed_indices.dedup();

    let removal_limit = (total_lines as f64 * policy.max_removal_ratio).floor() as usize;
    if removed_indices.len() > removal_limit {
        return BoilerplateResult {
            filtered_text: text.to_string(),
            removed_line_count: 0,
            detected_patterns,
        };
    }

    let filtered_text = rebuild_without_lines(&lines, &removed_indices);

    BoilerplateResult {
        filtered_text,
        removed_line_count: removed_indices.len(),
        detected_patterns,
    }
}

struct BoundaryFilter {
    indices: Vec<usize>,
    pattern: Option<String>,
}

fn collect_boundary_filters(
    lines: &[&str],
    offset: usize,
    policy: &BoilerplatePolicy,
) -> Vec<BoundaryFilter> {
    let mut filters = Vec::new();
    filters.extend(detect_nav_blocks(lines, offset, policy));
    filters.extend(detect_known_patterns(lines, offset, policy));
    filters.extend(detect_share_widgets(lines, offset));
    filters
}

fn detect_nav_blocks(
    lines: &[&str],
    offset: usize,
    policy: &BoilerplatePolicy,
) -> Vec<BoundaryFilter> {
    let mut filters = Vec::new();
    let mut start = None;

    for (i, line) in lines.iter().enumerate() {
        if link_density(line) >= policy.link_density_threshold {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(begin) = start.take() {
            if i - begin >= policy.min_nav_block_lines {
                filters.push(BoundaryFilter {
                    indices: ((offset + begin)..(offset + i)).collect::<Vec<usize>>(),
                    pattern: Some("navigation block".into()),
                });
            }
        }
    }

    if let Some(begin) = start {
        if lines.len() - begin >= policy.min_nav_block_lines {
            filters.push(BoundaryFilter {
                indices: ((offset + begin)..(offset + lines.len())).collect::<Vec<usize>>(),
                pattern: Some("navigation block".into()),
            });
        }
    }

    filters
}

fn detect_known_patterns(
    lines: &[&str],
    offset: usize,
    policy: &BoilerplatePolicy,
) -> Vec<BoundaryFilter> {
    let mut filters = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let normalized = line.to_lowercase();
        for pattern in &policy.known_patterns {
            if normalized.contains(pattern) {
                filters.push(BoundaryFilter {
                    indices: vec![offset + i],
                    pattern: Some(pattern.clone()),
                });
            }
        }
    }
    filters
}

fn detect_share_widgets(lines: &[&str], offset: usize) -> Vec<BoundaryFilter> {
    let mut filters = Vec::new();
    let names = ["facebook", "x", "twitter", "linkedin"];
    for (i, line) in lines.iter().enumerate() {
        if line.len() <= 40 {
            let lower = line.to_lowercase();
            for name in names {
                if lower.contains(name) {
                    filters.push(BoundaryFilter {
                        indices: vec![offset + i],
                        pattern: Some("share widget".into()),
                    });
                }
            }
        }
    }
    filters
}

fn link_density(line: &str) -> f64 {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let link_tokens = tokens
        .iter()
        .filter(|token| token.contains('[') && token.contains("]("))
        .count();
    link_tokens as f64 / tokens.len() as f64
}

fn rebuild_without_lines(lines: &[&str], remove: &[usize]) -> String {
    let removal_set: HashSet<usize> = remove.iter().copied().collect();
    let mut result = String::new();
    let mut first = true;

    for (idx, line) in lines.iter().enumerate() {
        if removal_set.contains(&idx) {
            continue;
        }
        if !first {
            result.push('\n');
        }
        result.push_str(line);
        first = false;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_markdown(lines: &[&str]) -> String {
        lines.join("\n")
    }

    #[test]
    fn removes_nav_block_at_start() {
        let policy = BoilerplatePolicy::default();
        let input = build_markdown(&[
            "[home](#)",
            "[about](#)",
            "[contact](#)",
            "",
            "Article begins",
            "Body line 1",
            "Body line 2",
            "Body line 3",
        ]);
        let result = filter_boilerplate(&input, &policy);
        assert!(!result.filtered_text.contains("home"));
        assert!(result.filtered_text.contains("Article begins"));
    }

    #[test]
    fn preserves_middle_cookies() {
        let policy = BoilerplatePolicy::default();
        let input = build_markdown(&["Article line", "cookies are good", "more content"]);
        let result = filter_boilerplate(&input, &policy);
        assert_eq!(result.filtered_text, input);
        assert!(result.detected_patterns.is_empty());
    }

    #[test]
    fn detects_cookie_banner_at_end() {
        let policy = BoilerplatePolicy::default();
        let input = build_markdown(&[
            "Content",
            "More content",
            "Accept cookies to continue",
            "We use cookies",
        ]);
        let result = filter_boilerplate(&input, &policy);
        assert!(!result.filtered_text.contains("cookies"));
        assert!(
            result
                .detected_patterns
                .contains(&"cookie policy".to_string())
                || result
                    .detected_patterns
                    .contains(&"accept cookies".to_string())
        );
    }

    #[test]
    fn rejects_over_removal() {
        let policy = BoilerplatePolicy {
            boundary_scan_lines: 2,
            max_removal_ratio: 0.1,
            ..BoilerplatePolicy::default()
        };
        let input = build_markdown(&["footer", "dd", "xx"]);
        let result = filter_boilerplate(&input, &policy);
        assert_eq!(result.filtered_text, input);
    }

    #[test]
    fn short_document_boundary_window() {
        let policy = BoilerplatePolicy {
            boundary_scan_lines: 5,
            ..BoilerplatePolicy::default()
        };
        let input = build_markdown(&["top nav", "cookie notice", "content", "footer"]);
        let result = filter_boilerplate(&input, &policy);
        assert!(!result.filtered_text.contains("cookie"));
    }

    #[test]
    fn empty_input_returns_empty() {
        let policy = BoilerplatePolicy::default();
        let result = filter_boilerplate("", &policy);
        assert!(result.filtered_text.is_empty());
        assert_eq!(result.removed_line_count, 0);
    }
}
