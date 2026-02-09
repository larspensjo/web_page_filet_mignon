mod boilerplate;
mod budget;
mod derive;
mod normalize;
mod truncation;
mod types;

pub use boilerplate::{BoilerplatePolicy, BoilerplateResult};
pub use budget::{
    compute_prompt_overhead, ContentBudget, PreparedCollection, PreparedInput, NONCE_OVERHEAD_BYTES,
};
pub use derive::{derive_clean_text, ContentPrepConfig};
pub use normalize::NormalizationPolicy;
pub use truncation::{truncate_to_budget, TRUNCATION_MARKER};
pub use types::{CleanText, CleanTextReport, TruncationBoundary};
