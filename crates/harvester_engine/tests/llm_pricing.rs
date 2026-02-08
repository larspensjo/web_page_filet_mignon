use harvester_engine::llm::{PricingRegistry, TokenUsage};

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
