use std::collections::HashMap;

use crate::llm::types::TokenUsage;

/// Pricing per model in microdollars per 1,000,000 tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelPricing {
    pub input_per_million: u64,
    pub output_per_million: u64,
}

impl ModelPricing {
    pub fn new(input_per_million: u64, output_per_million: u64) -> Self {
        Self {
            input_per_million,
            output_per_million,
        }
    }

    pub fn zero() -> Self {
        Self::new(0, 0)
    }

    pub fn cost_microdollars(&self, usage: &TokenUsage) -> u64 {
        let regular_input = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
        let regular_cost = self.cost_component(regular_input, self.input_per_million);
        // Exact 50% discount: ceil(cached * rate / 2_000_000), no rate truncation.
        let cached_cost = (usage.cached_input_tokens as u64 * self.input_per_million + 1_999_999)
            / 2_000_000;
        let output_cost = self.cost_component(usage.output_tokens, self.output_per_million);
        regular_cost
            .saturating_add(cached_cost)
            .saturating_add(output_cost)
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
        reg.insert("gpt-4o-mini", ModelPricing::new(15_000, 15_000));
        reg.insert("gpt-4o", ModelPricing::new(50_000, 80_000));
        reg.insert("gpt-3.5-turbo", ModelPricing::new(6_000, 6_000));
        // TODO: replace with published gpt-5-nano rates once the model is available;
        // using gpt-4o-mini rates as a placeholder.
        reg.insert("gpt-5-nano", ModelPricing::new(15_000, 15_000));
        reg
    }

    pub fn insert(&mut self, model_name: impl Into<String>, pricing: ModelPricing) {
        self.prices.insert(model_name.into(), pricing);
    }

    pub fn get(&self, model_name: &str) -> Option<&ModelPricing> {
        self.prices.get(model_name)
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
}

impl Default for PricingRegistry {
    fn default() -> Self {
        Self::new()
    }
}
