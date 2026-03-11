/// Per-reason counts of markdown blocks dropped during cleanup.
#[derive(Debug, Clone, Default)]
pub struct BlockDropCounts {
    pub css_blocks: usize,
    pub script_payloads: usize,
    pub subscription_blocks: usize,
    pub recirculation_blocks: usize,
}

impl BlockDropCounts {
    pub fn total(&self) -> usize {
        self.css_blocks + self.script_payloads + self.subscription_blocks + self.recirculation_blocks
    }
}

/// Which container type was selected as the article candidate.
#[derive(Debug, Clone)]
pub enum CandidateKind {
    Article,
    Main,
    RoleMain,
    ContentClass(String),
    Body,
}

/// Describes the extraction path taken.
#[derive(Debug, Clone)]
pub enum ExtractionOutcome {
    BestCandidate { score: f64, pruned_nodes: usize },
    LessPrunedFallback { reason: String },
    LegacyFallback { reason: String },
}

/// Diagnostics produced by a single extraction run.
#[derive(Debug, Clone, Default)]
pub struct ExtractionDiagnostics {
    pub candidate_kind: Option<CandidateKind>,
    pub candidate_score: Option<f64>,
    pub pruned_by_tag: usize,
    pub pruned_by_attr: usize,
    pub cleanup_blocks_dropped: BlockDropCounts,
    pub original_html_bytes: usize,
    pub pre_cleanup_markdown_bytes: usize,
    pub final_markdown_bytes: usize,
    pub retention_ratio: Option<f64>,
    pub outcome: Option<ExtractionOutcome>,
}

impl ExtractionDiagnostics {
    pub fn pruned_nodes(&self) -> usize {
        self.pruned_by_tag + self.pruned_by_attr
    }
}
