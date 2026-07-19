use std::collections::HashMap;

use crate::llm::types::TokenUsage;

/// Pricing per model; internal fields store microdollars per 1,000,000 tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelPricing {
    input_per_million: u64,
    output_per_million: u64,
}

impl ModelPricing {
    /// Creates a new `ModelPricing` with rates specified in dollars per million tokens.
    pub fn new(input_dollars_per_million: f64, output_dollars_per_million: f64) -> Self {
        Self {
            input_per_million: (input_dollars_per_million * 1_000_000.0).round() as u64,
            output_per_million: (output_dollars_per_million * 1_000_000.0).round() as u64,
        }
    }

    pub fn zero() -> Self {
        Self::new(0.0, 0.0)
    }

    pub fn cost_microdollars(&self, usage: &TokenUsage) -> u64 {
        let regular_input = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
        let regular_cost = self.cost_component(regular_input, self.input_per_million);
        // Exact 50% discount: ceil(cached * rate / 2_000_000), no rate truncation.
        let cached_cost = (usage.cached_input_tokens as u64)
            .saturating_mul(self.input_per_million)
            .saturating_add(1_999_999)
            / 2_000_000;
        let output_cost = self.cost_component(usage.output_tokens, self.output_per_million);
        regular_cost
            .saturating_add(cached_cost)
            .saturating_add(output_cost)
    }

    /// Batch API pricing is exactly half of standard input/output pricing.
    /// Batch requests do not receive an additional cached-input tier.
    pub fn batch_cost_microdollars(&self, usage: &TokenUsage) -> u64 {
        let standard_without_cache = self
            .cost_component(usage.input_tokens, self.input_per_million)
            .saturating_add(self.cost_component(usage.output_tokens, self.output_per_million));
        standard_without_cache.div_ceil(2)
    }

    fn cost_component(&self, tokens: u32, per_million: u64) -> u64 {
        if per_million == 0 || tokens == 0 {
            return 0;
        }
        let tokens = tokens as u64;
        tokens
            .saturating_mul(per_million)
            .saturating_add(1_000_000 - 1)
            / 1_000_000
    }
}

/// Registry of pricing information keyed by model name.
#[derive(Clone)]
pub struct PricingRegistry {
    prices: HashMap<String, ModelPricing>,
}

impl PricingRegistry {
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
        }
    }

    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        reg.insert("gpt-5.4", ModelPricing::new(2.50, 15.00));
        reg.insert("gpt-5.4-mini", ModelPricing::new(0.75, 4.50));
        reg.insert("gpt-5.4-nano", ModelPricing::new(0.20, 1.25));
        reg.insert("gpt-5.4-pro", ModelPricing::new(30.00, 180.00));
        reg
    }

    pub fn insert(&mut self, model_name: impl Into<String>, pricing: ModelPricing) {
        self.prices.insert(model_name.into(), pricing);
    }

    pub fn get(&self, model_name: &str) -> Option<&ModelPricing> {
        self.prices.get(model_name).or_else(|| {
            self.prices
                .iter()
                .filter(|(known_name, _)| {
                    model_name.starts_with(known_name.as_str())
                        && model_name
                            .as_bytes()
                            .get(known_name.len())
                            .is_some_and(|b| *b == b'-')
                })
                .max_by_key(|(known_name, _)| known_name.len())
                .map(|(_, pricing)| pricing)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    pub fn model_names(&self) -> Vec<&str> {
        self.prices.keys().map(|s| s.as_str()).collect()
    }

    pub fn cost_microdollars(&self, model_name: &str, usage: &TokenUsage) -> u64 {
        self.get(model_name)
            .map(|pricing| pricing.cost_microdollars(usage))
            .unwrap_or(0)
    }

    pub fn batch_cost_microdollars(&self, model_name: &str, usage: &TokenUsage) -> u64 {
        self.get(model_name)
            .map(|pricing| pricing.batch_cost_microdollars(usage))
            .unwrap_or(0)
    }
}

impl Default for PricingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_returns_exact_match_when_present() {
        let registry = PricingRegistry::with_defaults();
        assert_eq!(
            registry.get("gpt-5.4-mini"),
            Some(&ModelPricing::new(0.75, 4.50))
        );
    }

    #[test]
    fn get_falls_back_to_dated_variant_prefix_match() {
        let registry = PricingRegistry::with_defaults();
        assert_eq!(
            registry.get("gpt-5.4-mini-2026-03-17"),
            Some(&ModelPricing::new(0.75, 4.50))
        );
    }

    #[test]
    fn get_uses_longest_prefix_match() {
        let mut registry = PricingRegistry::new();
        registry.insert("gpt-5.4", ModelPricing::new(2.50, 15.00));
        registry.insert("gpt-5.4-mini", ModelPricing::new(0.75, 4.50));

        assert_eq!(
            registry.get("gpt-5.4-mini-2026-03-17"),
            Some(&ModelPricing::new(0.75, 4.50))
        );
    }

    #[test]
    fn batch_price_is_half_of_standard_without_cached_input_tier() {
        let registry = PricingRegistry::with_defaults();
        let usage = TokenUsage::new(1_000_000, 1_000_000).with_cached_input_tokens(900_000);
        let standard_without_cache = ModelPricing::new(0.75, 4.50)
            .cost_component(usage.input_tokens, 750_000)
            + ModelPricing::new(0.75, 4.50).cost_component(usage.output_tokens, 4_500_000);
        assert_eq!(
            registry.batch_cost_microdollars("gpt-5.4-mini-2026-03-17", &usage),
            standard_without_cache / 2
        );
    }
}
