use harvester_engine::llm::{ModelPricing, PricingRegistry, TokenUsage};

#[test]
fn default_pricing_matches_expected_costs() {
    let registry = PricingRegistry::with_defaults();
    let usage = TokenUsage::new(10_000, 5_000);

    let microdollars = registry.cost_microdollars("gpt-3.5-turbo", &usage);
    assert_eq!(microdollars, 90);
}

#[test]
fn unknown_model_returns_zero_cost() {
    let registry = PricingRegistry::with_defaults();
    let usage = TokenUsage::new(1_000_000, 1_000_000);

    assert_eq!(registry.cost_microdollars("nonexistent", &usage), 0);
}

#[test]
fn cost_with_cached_tokens_applies_exact_half_rate() {
    // Use an odd rate (15_001) to verify that we do NOT truncate the rate before dividing.
    // If we had halved the rate via integer division (15_001 / 2 = 7_500), cached tokens
    // would cost 7_500 per million instead of the correct 7_500.5 (rounded up to 7_501).
    let pricing = ModelPricing::new(15_001, 60_000);

    // Regular input: 1_000_000 tokens, no cache.
    let regular_usage = TokenUsage::new(1_000_000, 0);
    let regular_cost = pricing.cost_microdollars(&regular_usage);

    // Cached: same 1_000_000 tokens, all served from cache.
    let cached_usage = TokenUsage::new(1_000_000, 0).with_cached_input_tokens(1_000_000);
    let cached_cost = pricing.cost_microdollars(&cached_usage);

    // Cached cost must be exactly half of regular cost (ceiling applied correctly).
    // regular: ceil(1_000_000 * 15_001 / 1_000_000) = 15_001
    // cached:  ceil(1_000_000 * 15_001 / 2_000_000) = ceil(7_500.5) = 7_501
    assert_eq!(regular_cost, 15_001);
    assert_eq!(cached_cost, 7_501);
    assert!(cached_cost < regular_cost);
}

#[test]
fn cost_with_no_cached_tokens_unchanged() {
    // Existing behavior: zero cached tokens means full input rate applies.
    let pricing = ModelPricing::new(15_000, 60_000);
    let usage = TokenUsage::new(1_000_000, 0);
    let cost = pricing.cost_microdollars(&usage);
    assert_eq!(cost, 15_000);
}

#[test]
fn default_registry_contains_gpt5_nano() {
    let registry = PricingRegistry::with_defaults();
    assert!(
        registry.get("gpt-5-nano").is_some(),
        "gpt-5-nano must be present in the default pricing registry"
    );
}

#[test]
fn cost_calculation_saturates_on_overflow() {
    let mut registry = PricingRegistry::new();
    registry.insert(
        "expensive",
        harvester_engine::llm::ModelPricing::new(u64::MAX, u64::MAX),
    );

    let usage = TokenUsage::new(u32::MAX, u32::MAX);
    let cost = registry.cost_microdollars("expensive", &usage);
    assert!(cost < u64::MAX);
}
