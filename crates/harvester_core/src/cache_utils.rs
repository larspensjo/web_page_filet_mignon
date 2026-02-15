pub fn model_ids_compatible(store_model_id: &str, completion_model_id: &str) -> bool {
    if store_model_id == completion_model_id {
        return true;
    }
    completion_model_id.starts_with(store_model_id)
        && completion_model_id
            .as_bytes()
            .get(store_model_id.len())
            .is_some_and(|b| *b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_model_ids_are_compatible() {
        assert!(model_ids_compatible("gpt-4o-mini", "gpt-4o-mini"));
    }

    #[test]
    fn alias_matches_resolved_variant() {
        assert!(model_ids_compatible(
            "gpt-4o-mini",
            "gpt-4o-mini-2024-07-18",
        ));
    }

    #[test]
    fn incompatible_models_are_rejected() {
        assert!(!model_ids_compatible("gpt-4o-mini", "gpt-4o"));
    }
}
