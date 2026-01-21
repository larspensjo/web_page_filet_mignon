use sha2::{Digest, Sha256};

/// Windows-safe, deterministic filename: `{sanitized_title}--{short_hash(url)}.md`
pub fn deterministic_filename(title: Option<&str>, url: &str) -> String {
    let sanitized = sanitize_title(title.unwrap_or("untitled"));
    let hash = short_hash(url);
    format!("{sanitized}--{hash}.md")
}

fn sanitize_title(input: &str) -> String {
    let mut cleaned: String = input
        .chars()
        .map(|c| {
            if is_forbidden(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    cleaned = cleaned.trim_matches(&['_', ' ', '.'][..]).to_string();
    if cleaned.is_empty() {
        cleaned = "untitled".to_string();
    }
    // Collapse multiple underscores
    let mut compacted = String::with_capacity(cleaned.len());
    let mut prev_underscore = false;
    for c in cleaned.chars() {
        if c == '_' {
            if !prev_underscore {
                compacted.push(c);
            }
            prev_underscore = true;
        } else {
            compacted.push(c);
            prev_underscore = false;
        }
    }
    let mut final_name = compacted;
    if final_name.len() > 80 {
        final_name.truncate(80);
    }
    if is_reserved_windows_name(&final_name) {
        final_name.push('_');
    }
    final_name
}

fn is_forbidden(c: char) -> bool {
    matches!(c,
        '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\u{1F}'
    )
}

fn is_reserved_windows_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name))
}

fn short_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(8);
    for byte in digest.iter().take(4) {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}
