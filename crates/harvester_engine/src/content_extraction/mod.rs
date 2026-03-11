pub mod candidate_select;
pub mod diagnostics;
pub mod dom_prune;
pub mod markdown_cleanup;
pub mod pipeline;
pub mod policy;

pub use diagnostics::{BlockDropCounts, CandidateKind, ExtractionDiagnostics, ExtractionOutcome};
pub use pipeline::{ExtractedArticle, ExtractionPipeline};
pub use policy::ExtractionPolicy;
