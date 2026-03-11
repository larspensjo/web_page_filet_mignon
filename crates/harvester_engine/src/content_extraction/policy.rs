use std::collections::BTreeSet;

/// Policy for DOM node pruning.
#[derive(Debug, Clone)]
pub struct DomPrunePolicy {
    /// HTML tag names to always drop.
    pub drop_tags: BTreeSet<&'static str>,
    /// Class/id attribute tokens that trigger node removal.
    pub drop_attr_tokens: BTreeSet<&'static str>,
    /// Class/id attribute tokens that prevent removal (allow-list override).
    pub allow_attr_tokens: BTreeSet<&'static str>,
}

impl Default for DomPrunePolicy {
    fn default() -> Self {
        Self {
            drop_tags: BTreeSet::from([
                "script", "style", "noscript", "iframe", "template", "nav", "footer", "aside",
                "form",
            ]),
            drop_attr_tokens: BTreeSet::from([
                "share",
                "social",
                "newsletter",
                "subscribe",
                "signup",
                "related",
                "recirculation",
                "comments",
                "comment",
                "promo",
                "advert",
                "advertisement",
                "outbrain",
                "taboola",
                "cookie",
                "consent",
                "pricing",
                "login",
                "signin",
                "paywall",
                "membership",
                "account",
                "register",
            ]),
            allow_attr_tokens: BTreeSet::new(),
        }
    }
}

/// Policy for candidate container selection.
#[derive(Debug, Clone)]
pub struct CandidatePolicy {
    /// Minimum text characters required in a viable candidate.
    pub min_text_chars: usize,
    /// Maximum link density (0..1) for a candidate to be considered.
    pub max_link_density: f64,
    /// Maximum number of candidates to evaluate.
    pub max_candidates: usize,
}

impl Default for CandidatePolicy {
    fn default() -> Self {
        Self {
            min_text_chars: 100,
            max_link_density: 0.5,
            max_candidates: 20,
        }
    }
}

/// Policy for markdown block cleanup.
#[derive(Debug, Clone)]
pub struct MarkdownCleanupPolicy {
    /// Drop blocks that look like CSS-in-JS or stylesheet payloads.
    pub drop_css_blocks: bool,
    /// Drop blocks that look like JS/JSON payloads.
    pub drop_script_payloads: bool,
    /// Drop blocks matching subscription/signup patterns.
    pub drop_subscription_blocks: bool,
    /// Drop blocks matching recirculation/read-more patterns.
    pub drop_recirculation_blocks: bool,
}

impl Default for MarkdownCleanupPolicy {
    fn default() -> Self {
        Self {
            drop_css_blocks: true,
            drop_script_payloads: true,
            drop_subscription_blocks: true,
            drop_recirculation_blocks: true,
        }
    }
}

/// Retention policy for post-cleanup safety checks.
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Minimum retained character count to accept the result.
    pub min_retained_chars: usize,
    /// Minimum ratio of retained to original markdown chars.
    pub min_retention_ratio: f64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            min_retained_chars: 100,
            min_retention_ratio: 0.1,
        }
    }
}

/// Combined extraction policy.
#[derive(Debug, Clone, Default)]
pub struct ExtractionPolicy {
    pub prune: DomPrunePolicy,
    pub candidate: CandidatePolicy,
    pub cleanup: MarkdownCleanupPolicy,
    pub retention: RetentionPolicy,
}
