//! Pure canonical-key grouping, independent of archive selection.

use std::collections::BTreeMap;

use crate::signal_candidate::canonical_signal_key;

pub mod report;

/// Canonical keys indexed by deterministic group id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalKeyGroups {
    groups: BTreeMap<String, Vec<String>>,
    group_for_key: BTreeMap<String, String>,
}

impl SignalKeyGroups {
    pub fn groups(&self) -> &BTreeMap<String, Vec<String>> {
        &self.groups
    }

    /// Accepts either a raw signal key or its canonical alias.
    pub fn group_id(&self, key: &str) -> Option<&str> {
        self.group_for_key
            .get(&canonical_signal_key(key))
            .map(String::as_str)
    }
}

/// Each distinct canonical key is its own group. Repeated keys and existing
/// frontier-model-access aliases share a group, regardless of input order.
pub fn group_signal_keys<I, K>(keys: I) -> SignalKeyGroups
where
    I: IntoIterator<Item = K>,
    K: AsRef<str>,
{
    let group_for_key: BTreeMap<_, _> = keys
        .into_iter()
        .map(|key| {
            let canonical = canonical_signal_key(key.as_ref());
            (canonical.clone(), canonical)
        })
        .collect();
    let groups = group_for_key
        .keys()
        .map(|key| (key.clone(), vec![key.clone()]))
        .collect();
    SignalKeyGroups {
        groups,
        group_for_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_groups_are_order_independent_and_deduplicate_keys() {
        let a = group_signal_keys(["z-event", "a-event", "z-event"]);
        assert_eq!(a, group_signal_keys(["a-event", "z-event"]));
        assert_eq!(a.groups().len(), 2);
        assert_eq!(a.group_id("z-event"), Some("z-event"));
        assert_eq!(a.group_id("unknown"), None);
    }

    #[test]
    fn existing_alias_is_applied_before_grouping() {
        let keys = [
            "openai-frontier-model-access-restriction",
            "openai-frontier-model-access-blocked",
            "openai-gpt-6-launch",
        ];
        let groups = group_signal_keys(keys);
        assert_eq!(groups.groups().len(), 2);
        assert_eq!(groups.group_id(keys[0]), groups.group_id(keys[1]));
        assert_ne!(groups.group_id(keys[0]), groups.group_id(keys[2]));
    }
}
