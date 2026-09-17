//! Jev input-token pricing.

/// Computes microdollars for input tokens at a rate expressed in
/// microdollars per million input tokens. Output tokens are not priced.
pub fn jev_cost_microdollars(input_tokens: u64, input_microdollars_per_million: u64) -> u64 {
    let numerator = u128::from(input_tokens) * u128::from(input_microdollars_per_million);
    numerator.div_ceil(1_000_000) as u64
}
