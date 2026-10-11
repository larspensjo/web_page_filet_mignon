//! Pure canonical-key grouping, independent of archive selection.

use std::collections::{BTreeMap, BTreeSet};

use crate::signal_candidate::canonical_signal_key;

pub mod report;

// Slug vocabulary is deliberately explicit: changing evidence changes which
// articles are withheld. Real and key-only fixture rows pin these tables.
const COMPOUNDS: &[(&str, &str, &str)] = &[
    ("hon", "hai", "foxconn"),
    ("rocket", "lab", "rocketlab"),
    ("northrop", "grumman", "northropgrumman"),
    ("data", "center", "datacenter"),
    ("data", "centers", "datacenter"),
    ("data", "centre", "datacenter"),
    ("open", "weight", "openweight"),
    ("open", "weights", "openweight"),
    ("white", "house", "whitehouse"),
    ("ge", "vernova", "gevernova"),
    ("bloom", "energy", "bloomenergy"),
    ("blue", "origin", "blueorigin"),
    ("sk", "hynix", "skhynix"),
    ("palo", "alto", "paloalto"),
    ("paloalto", "network", "paloalto"),
    ("paloalto", "networks", "paloalto"),
    ("astera", "lab", "asteralabs"),
    ("astera", "labs", "asteralabs"),
    ("trade", "desk", "tradedesk"),
];
const ALIASES: &[(&str, &str)] = &[("aws", "amazon")];
#[derive(Clone, Copy)]
enum NoiseScope {
    All,
    ActorOnly,
}
use NoiseScope::{ActorOnly, All};

// Access-restriction aliases retain their older prefix vocabulary. Those words
// do not become noise for ordinary same-event evidence.
const NOISE: &[(&str, NoiseScope)] = &[
    ("a", All),
    ("an", All),
    ("and", All),
    ("as", All),
    ("at", All),
    ("by", All),
    ("for", All),
    ("from", All),
    ("in", All),
    ("its", All),
    ("of", All),
    ("on", All),
    ("over", All),
    ("the", All),
    ("to", All),
    ("with", All),
    ("after", All),
    ("amid", All),
    ("up", All),
    ("new", All),
    ("ai", All),
    ("says", All),
    ("reportedly", All),
    ("report", All),
    ("reports", All),
    ("prepares", All),
    ("plans", All),
    ("eyes", All),
    ("set", All),
    ("us", All),
    ("u", All),
    ("s", All),
    ("doe", All),
    ("federal", All),
    ("government", All),
    ("administration", All),
    ("trump", All),
    ("whitehouse", All),
    ("percent", All),
    ("all", ActorOnly),
    ("bar", ActorOnly),
    ("barred", ActorOnly),
    ("bars", ActorOnly),
    ("block", ActorOnly),
    ("blocked", ActorOnly),
    ("blocks", ActorOnly),
    ("commerce", ActorOnly),
    ("department", ActorOnly),
    ("directive", ActorOnly),
    ("foreign", ActorOnly),
    ("foreigners", ActorOnly),
    ("national", ActorOnly),
    ("nationals", ActorOnly),
    ("order", ActorOnly),
    ("ordered", ActorOnly),
    ("orders", ActorOnly),
    ("security", ActorOnly),
    ("users", ActorOnly),
];
const GENERIC_OBJECTS: &[&str] = &["model", "stock", "share", "market", "company", "startup"];
struct EventClass {
    forms: &'static [&'static str],
    shared_objects: usize,
}
const EVENT_CLASSES: &[EventClass] = &[
    EventClass {
        forms: &[
            "loan",
            "loans",
            "lend",
            "lends",
            "lending",
            "lender",
            "financing",
            "finance",
            "financed",
        ],
        shared_objects: 1,
    },
    EventClass {
        forms: &[
            "revenue", "revenues", "sales", "earning", "earnings", "result", "results", "profit",
            "profits",
        ],
        shared_objects: 1,
    },
    EventClass {
        forms: &[
            "pledge",
            "pledges",
            "pledged",
            "commitment",
            "commitments",
            "commit",
            "commits",
            "promise",
            "promises",
        ],
        shared_objects: 1,
    },
    EventClass {
        forms: &[
            "acquire",
            "acquires",
            "acquired",
            "acquisition",
            "buys",
            "buyout",
            "takeover",
        ],
        shared_objects: 1,
    },
    EventClass {
        forms: &[
            "launch",
            "launches",
            "launched",
            "release",
            "releases",
            "released",
            "debut",
            "debuts",
            "unveil",
            "unveils",
            "unveiled",
            "introduces",
            "rollout",
        ],
        shared_objects: 2,
    },
    EventClass {
        forms: &[
            "deal",
            "deals",
            "lease",
            "leases",
            "contract",
            "contracts",
            "agreement",
            "agreements",
        ],
        shared_objects: 2,
    },
    EventClass {
        forms: &[
            "probe",
            "probes",
            "probing",
            "investigation",
            "investigate",
            "investigates",
            "inquiry",
        ],
        shared_objects: 2,
    },
    EventClass {
        forms: &["funding", "round", "series"],
        shared_objects: 2,
    },
];

pub(crate) fn is_noise_token(token: &str) -> bool {
    NOISE
        .iter()
        .any(|(word, scope)| *word == token && matches!(scope, All))
}

pub(crate) fn is_actor_noise_token(token: &str) -> bool {
    NOISE.iter().any(|(word, _)| *word == token)
}

fn event_class(token: &str) -> Option<usize> {
    EVENT_CLASSES
        .iter()
        .position(|class| class.forms.contains(&token))
}

fn singular(token: &str) -> &str {
    if token.len() > 3
        && token.ends_with('s')
        && !["ss", "us", "is"]
            .iter()
            .any(|ending| token.ends_with(ending))
    {
        &token[..token.len() - 1]
    } else {
        token
    }
}

fn integer(token: &str) -> bool {
    !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit())
}

/// The quantity grammar is ASCII digits followed by zero to two ASCII letters.
fn quantity(token: &str) -> bool {
    let digits = token.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0
        && token.len() - digits <= 2
        && token[digits..]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase())
}

fn year(token: &str) -> Option<String> {
    if let Some(number) = token.strip_prefix("fy") {
        return match number.len() {
            2 if integer(number) => Some(format!("20{number}")),
            4 if integer(number) => Some(number.into()),
            _ => None,
        };
    }
    (token.len() == 4 && integer(token) && (token.starts_with("19") || token.starts_with("20")))
        .then(|| token.into())
}

fn period(token: &str) -> Option<&str> {
    let token = if token.starts_with("fq") {
        &token[1..]
    } else {
        token
    };
    matches!(token, "q1" | "q2" | "q3" | "q4" | "h1" | "h2").then_some(token)
}

fn amount_suffix(token: &str) -> bool {
    matches!(
        token,
        "b" | "bn" | "m" | "mn" | "k" | "w" | "kw" | "mw" | "gw"
    )
}

fn decimal_amount_tail(token: &str) -> bool {
    let digits = token.bytes().take_while(u8::is_ascii_digit).count();
    quantity(token)
        && !integer(token)
        // Multi-digit b suffixes are ambiguous model sizes (4-70b, 4-12b).
        && (digits == 1 || (&token[digits..] != "b" && amount_suffix(&token[digits..])))
}

fn version_base(token: &str) -> bool {
    !quantity(token)
        && year(token).is_none()
        && period(token).is_none()
        && !matches!(token, "quarter" | "quarterly" | "half")
        && !is_noise_token(token)
        && event_class(token).is_none()
}

fn version_component(token: &str) -> String {
    let digits = token.bytes().take_while(u8::is_ascii_digit).count();
    let number = token[..digits].trim_start_matches('0');
    format!(
        "{}{}",
        if number.is_empty() { "0" } else { number },
        &token[digits..]
    )
}

#[derive(Debug, Default)]
struct Evidence {
    lead: Option<String>,
    objects: BTreeSet<String>,
    classes: BTreeSet<usize>,
    periods: BTreeSet<String>,
    years: BTreeSet<String>,
    versions: BTreeMap<String, BTreeSet<String>>,
}

impl Evidence {
    fn from_key(key: &str) -> Self {
        let raw: Vec<_> = key
            .split('-')
            .filter(|token| !token.is_empty())
            .map(str::to_lowercase)
            .collect();
        let mut tokens: Vec<String> = Vec::new();
        for token in &raw {
            tokens.push(
                ALIASES
                    .iter()
                    .find(|(alias, _)| token == alias)
                    .map_or(token.as_str(), |(_, folded)| *folded)
                    .to_string(),
            );
            // Successive folds absorb three-word names such as Palo Alto Networks.
            while tokens.len() >= 2 {
                let end = tokens.len();
                let Some((_, _, folded)) = COMPOUNDS
                    .iter()
                    .find(|(a, b, _)| tokens[end - 2] == *a && tokens[end - 1] == *b)
                else {
                    break;
                };
                tokens.truncate(end - 2);
                tokens.push((*folded).into());
            }
        }
        let mut evidence = Self::default();
        let mut remaining = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].as_str();
            // Consume unambiguous amounts before versions; large b-suffixed
            // sizes instead leave the preceding model generation intact.
            if integer(token)
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| decimal_amount_tail(next))
            {
                index += 2;
                continue;
            }
            if integer(token)
                && tokens.get(index + 1).is_some_and(|next| integer(next))
                && tokens.get(index + 2).is_some_and(|next| next == "billion")
            {
                index += 3;
                continue;
            }
            if let Some(marker) = period(token) {
                evidence.periods.insert(marker.into());
                remaining.push(
                    (if marker.starts_with('q') {
                        "quarter"
                    } else {
                        "half"
                    })
                    .to_string(),
                );
                index += 1;
                continue;
            }
            if let Some(year) = year(token) {
                evidence.years.insert(year);
                index += 1;
                continue;
            }
            if quantity(token) && index > 0 && version_base(&tokens[index - 1]) {
                let digits = token.bytes().take_while(u8::is_ascii_digit).count();
                if integer(token) || !amount_suffix(&token[digits..]) {
                    let base = singular(&tokens[index - 1]).to_string();
                    let mut parts = Vec::new();
                    loop {
                        parts.push(version_component(&tokens[index]));
                        index += 1;
                        if !integer(token)
                            || !tokens
                                .get(index)
                                .is_some_and(|next| integer(next) && year(next).is_none())
                        {
                            break;
                        }
                    }
                    evidence
                        .versions
                        .entry(base)
                        .or_default()
                        .insert(parts.join("."));
                    continue;
                }
            }
            index += 1;
            match token {
                "quarter" | "quarterly" => {
                    remaining.push("quarter".into());
                    continue;
                }
                _ => {}
            }
            if is_noise_token(token) || quantity(token) {
                continue;
            }
            if let Some(class) = event_class(token) {
                evidence.classes.insert(class);
            } else {
                remaining.push(singular(token).into());
            }
        }
        let lead_index = remaining
            .iter()
            .position(|token| !matches!(token.as_str(), "quarter" | "half"));
        evidence.lead = lead_index.map(|index| remaining[index].clone());
        for object in remaining {
            if evidence.lead.as_ref() != Some(&object)
                && !GENERIC_OBJECTS.contains(&object.as_str())
            {
                evidence.objects.insert(object);
            }
        }
        evidence
    }

    fn matches(&self, other: &Self) -> bool {
        fn conflicts<T: Ord>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> bool {
            !a.is_empty() && !b.is_empty() && a != b
        }
        if self.lead.is_none()
            || self.lead != other.lead
            || conflicts(&self.periods, &other.periods)
            || conflicts(&self.years, &other.years)
            || self.versions.iter().any(|(base, values)| {
                other
                    .versions
                    .get(base)
                    .is_some_and(|other| conflicts(values, other))
            })
        {
            return false;
        }
        let shared = self.objects.intersection(&other.objects).count();
        self.classes
            .intersection(&other.classes)
            .any(|class| shared >= EVENT_CLASSES[*class].shared_objects)
            || (shared >= 3
                && (self.classes.is_empty()
                    || other.classes.is_empty()
                    || !self.classes.is_disjoint(&other.classes)))
    }
}

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

/// Canonical keys are bucketed by lead and processed in ascending byte order.
/// A key joins the earliest group whose every member matches it. This complete
/// linkage prevents transitive chains; a newly arriving early key can regroup
/// existing keys. Repeated keys and existing access aliases are canonicalized first.
pub fn group_signal_keys<I, K>(keys: I) -> SignalKeyGroups
where
    I: IntoIterator<Item = K>,
    K: AsRef<str>,
{
    let canonical_keys: BTreeSet<_> = keys
        .into_iter()
        .map(|key| canonical_signal_key(key.as_ref()))
        .collect();
    let mut buckets: BTreeMap<_, Vec<_>> = BTreeMap::new();
    let mut groups = BTreeMap::new();
    let mut group_for_key = BTreeMap::new();
    for key in canonical_keys {
        let evidence = Evidence::from_key(&key);
        if let Some(lead) = &evidence.lead {
            buckets
                .entry(lead.clone())
                .or_default()
                .push((key, evidence));
        } else {
            group_for_key.insert(key.clone(), key.clone());
            groups.insert(key.clone(), vec![key]);
        }
    }
    for bucket in buckets.into_values() {
        let mut complete_groups: Vec<Vec<(String, Evidence)>> = Vec::new();
        for (key, evidence) in bucket {
            if let Some(group) = complete_groups
                .iter_mut()
                .find(|group| group.iter().all(|(_, member)| evidence.matches(member)))
            {
                group.push((key, evidence));
            } else {
                complete_groups.push(vec![(key, evidence)]);
            }
        }
        for group in complete_groups {
            let id = group[0].0.clone();
            let members = group
                .into_iter()
                .map(|(key, _)| {
                    group_for_key.insert(key.clone(), id.clone());
                    key
                })
                .collect();
            groups.insert(id, members);
        }
    }
    SignalKeyGroups {
        groups,
        group_for_key,
    }
}

#[cfg(test)]
pub(crate) const REGROUPING_KEYS: [&str; 5] = [
    "actor-alpha-loan-ab-ac-ak",
    "actor-beta-loan-ab-bd",
    "actor-charlie-loan-ac-ck",
    "actor-delta-loan-bd",
    "actor-aardvark-loan-ak-ck",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_groups_are_order_independent_and_deduplicate_keys() {
        let a = group_signal_keys(["z-event", "a-event", "z-event"]);
        assert_eq!(a, group_signal_keys(["a-event", "z-event"]));
        assert_eq!(a.groups().len(), 2);
        assert_eq!(a.group_id("z-event"), Some("z-event"));
        assert_eq!(a.group_id("unknown"), None);
    }

    #[test]
    fn normalization_extracts_lead_objects_classes_and_markers() {
        let evidence = Evidence::from_key("--US-DOE-loans-Hon-Hai-q3-2026-revenues-ai-data-centers-47-percent-stocks-sb1246-ai5-mi450--");
        assert_eq!(evidence.lead.as_deref(), Some("foxconn"));
        assert_eq!(
            evidence.objects,
            ["quarter", "datacenter", "sb1246", "ai5", "mi450"]
                .map(String::from)
                .into()
        );
        assert_eq!(evidence.classes, [0, 1].into());
        assert_eq!(evidence.periods, ["q3".into()].into());
        assert_eq!(evidence.years, ["2026".into()].into());
        let half = Evidence::from_key("trump-white-house-aws-h1-fy2031-loan-nuclear");
        assert_eq!(half.lead.as_deref(), Some("amazon"));
        assert_eq!(half.objects, ["half".into(), "nuclear".into()].into());
        assert_eq!(half.periods, ["h1".into()].into());
        assert_eq!(half.years, ["2031".into()].into());
        assert_eq!(
            Evidence::from_key("actor-fy2100-revenue-target").years,
            ["2100".into()].into()
        );
        assert_eq!(
            Evidence::from_key("rocket-lab-loan-nuclear")
                .lead
                .as_deref(),
            Some("rocketlab")
        );
        assert_eq!(
            Evidence::from_key("doe-loan-vistra-nuclear")
                .lead
                .as_deref(),
            Some("vistra")
        );
        assert!(Evidence::from_key("us-doe-ai-loan-4b").lead.is_none());
    }

    #[test]
    fn plural_objects_and_quantities_follow_the_explicit_grammar() {
        let evidence = Evidence::from_key(
            "actor-loan-plants-glass-status-analysis-4b-47-100m-2gw-48a-sb1246-ai5-mi450",
        );
        assert_eq!(
            evidence.objects,
            ["plant", "glass", "status", "analysis", "sb1246", "ai5", "mi450"]
                .map(String::from)
                .into()
        );
        let version = Evidence::from_key("google-gemini-4-launch-pricing");
        assert_eq!(version.versions["gemini"], ["4".into()].into());
        let amount = Evidence::from_key("vistra-4-2b-loan-nuclear");
        assert!(amount.versions.is_empty());
        assert_eq!(amount.objects, ["nuclear".into()].into());
        assert!(Evidence::from_key("actor-loan-4-nuclear")
            .versions
            .is_empty());
    }

    #[test]
    fn extended_markers_preserve_versions_before_sizes_and_rounded_amounts() {
        for (key, base, version) in [
            ("google-gemini-03-06-flash-launch", "gemini", "3.6"),
            ("openai-gpt-6-1-astra-rollout", "gpt", "6.1"),
            ("meta-llama-4-70b-open-weights-release", "llama", "4"),
            ("google-gemma-4-12b-edge-release", "gemma", "4"),
            ("openai-gpt-4o-voice-launch", "gpt", "4o"),
            ("openai-gpt-04o-voice-launch", "gpt", "4o"),
            ("actor-model-00o-voice-launch", "model", "0o"),
        ] {
            let evidence = Evidence::from_key(key);
            assert_eq!(evidence.versions[base], [version.into()].into(), "{key}");
            assert_eq!(evidence.versions.len(), 1, "{key}");
        }
        for key in [
            "us-lends-vistra-4-2-billion-nuclear",
            "vistra-doe-4-2b-loan-nuclear",
            "vistra-4b-loan-nuclear",
            "actor-4-25bn-loan-nuclear",
            "actor-4-25gw-loan-nuclear",
        ] {
            let evidence = Evidence::from_key(key);
            assert!(evidence.versions.is_empty(), "{key}");
            assert_eq!(evidence.objects, ["nuclear".into()].into(), "{key}");
        }
        let fiscal = Evidence::from_key("marvell-fq2-fy27-revenue-guidance");
        assert_eq!(fiscal.periods, ["q2".into()].into());
        assert_eq!(fiscal.years, ["2027".into()].into());
    }

    #[test]
    fn period_objects_cannot_be_the_lead() {
        for key in [
            "q3-earnings-nvidia",
            "fq3-earnings-nvidia",
            "quarterly-earnings-nvidia",
            "h1-earnings-nvidia",
        ] {
            assert_eq!(
                Evidence::from_key(key).lead.as_deref(),
                Some("nvidia"),
                "{key}"
            );
        }
        for key in ["q3-earnings", "h1-earnings", "quarterly-earnings"] {
            assert!(Evidence::from_key(key).lead.is_none(), "{key}");
        }
    }

    #[test]
    fn explicit_markers_veto_only_when_both_are_present() {
        for (a, b) in [
            (
                "actor-q2-2026-revenue-server",
                "actor-q3-2026-revenue-server",
            ),
            ("actor-h1-revenue-server", "actor-h2-revenue-server"),
            (
                "actor-q3-2025-revenue-server",
                "actor-q3-2026-revenue-server",
            ),
            (
                "actor-crew-12-launch-iss-docking",
                "actor-crew-13-launch-iss-docking",
            ),
            (
                "actor-model-5-launch-pricing-tiers",
                "actor-model-6-launch-pricing-tiers",
            ),
        ] {
            assert_eq!(group_signal_keys([a, b]).groups().len(), 2, "{a} / {b}");
        }
        for (a, b) in [
            (
                "actor-q3-2026-revenue-server",
                "actor-quarterly-revenue-server",
            ),
            (
                "actor-crew-12-launch-iss-docking",
                "actor-crew-launch-iss-docking",
            ),
            (
                "actor-crew-12-launch-iss-docking",
                "actor-other-13-launch-iss-docking",
            ),
            ("actor-fy2031-revenue-target", "actor-2031-revenue-target"),
        ] {
            assert_eq!(group_signal_keys([a, b]).groups().len(), 1, "{a} / {b}");
        }
    }

    #[test]
    fn three_object_fallback_refuses_disjoint_event_classes() {
        for (a, b, count) in [
            (
                "actor-blackwell-ultra-gpu-delay",
                "actor-blackwell-ultra-gpu-begin",
                1,
            ),
            (
                "actor-launch-blackwell-ultra-gpu",
                "actor-blackwell-ultra-gpu-begin",
                1,
            ),
            (
                "actor-launch-blackwell-ultra-gpu",
                "actor-probe-blackwell-ultra-gpu",
                2,
            ),
            (
                "actor-blackwell-ultra-delay",
                "actor-blackwell-ultra-begin",
                2,
            ),
            (
                "actor-launch-model-stock-share",
                "actor-release-model-stock-share",
                2,
            ),
        ] {
            assert_eq!(group_signal_keys([a, b]).groups().len(), count, "{a} / {b}");
        }
    }

    #[test]
    fn event_classes_require_their_own_shared_object_threshold() {
        for class in EVENT_CLASSES {
            let a = format!("actor-{}-nuclear-turbine", class.forms[0]);
            let b = format!("actor-{}-nuclear-turbine-detail", class.forms[1]);
            assert_eq!(group_signal_keys([&a, &b]).groups().len(), 1, "{a} / {b}");
            let insufficient = if class.shared_objects == 1 {
                "solar-panel"
            } else {
                "nuclear-panel"
            };
            let b = format!("actor-{}-{insufficient}", class.forms[1]);
            assert_eq!(group_signal_keys([&a, &b]).groups().len(), 2, "{a} / {b}");
        }
        for token in ["investment", "expansion", "raise", "target"] {
            assert!(Evidence::from_key(&format!("actor-{token}-nuclear"))
                .classes
                .is_empty());
        }
    }

    #[test]
    fn complete_linkage_refuses_a_chain_and_is_permutation_invariant() {
        let keys = [
            "actor-alpha-loan-nuclear",
            "actor-beta-loan-nuclear-solar",
            "actor-charlie-loan-solar",
        ];
        let expected = group_signal_keys(keys);
        assert_eq!(expected.groups().len(), 2);
        assert_eq!(expected.group_id(keys[0]), expected.group_id(keys[1]));
        assert_ne!(expected.group_id(keys[0]), expected.group_id(keys[2]));
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            assert_eq!(group_signal_keys(order.map(|i| keys[i])), expected);
        }
    }

    #[test]
    fn early_key_regroups_three_groups_into_two() {
        let [a, b, c, d, k] = REGROUPING_KEYS;
        for (left, right, matches) in [
            (a, b, true),
            (a, c, true),
            (a, d, false),
            (a, k, true),
            (b, c, false),
            (b, d, true),
            (b, k, false),
            (c, d, false),
            (c, k, true),
            (d, k, false),
        ] {
            assert_eq!(
                Evidence::from_key(left).matches(&Evidence::from_key(right)),
                matches,
                "{left} / {right}"
            );
        }
        let before = group_signal_keys([a, b, c, d]);
        assert_eq!(before.groups().len(), 3);
        assert_eq!(before.group_id(a), before.group_id(b));
        let after = group_signal_keys([a, b, c, d, k]);
        assert_eq!(after.groups().len(), 2);
        assert_eq!(after.groups()[k], vec![k, a, c]);
        assert_eq!(after.groups()[b], vec![b, d]);
        // All 120 permutations, including duplicate keys, must yield the same map.
        fn permutations(keys: &mut [&str], start: usize, expected: &SignalKeyGroups) {
            if start == keys.len() {
                assert_eq!(&group_signal_keys(keys.iter().chain(keys.iter())), expected);
            } else {
                for i in start..keys.len() {
                    keys.swap(start, i);
                    permutations(keys, start + 1, expected);
                    keys.swap(start, i);
                }
            }
        }
        let mut permuted = REGROUPING_KEYS;
        permutations(&mut permuted, 0, &after);
    }

    #[test]
    fn every_rule_table_entry_has_vocabulary_coverage_witnesses() {
        let fixture: report::ReportExpectations = serde_json::from_str(include_str!(
            "../tests/fixtures/signal_key_merge/rule-table-cases.json"
        ))
        .unwrap();
        assert_eq!(fixture.synthetic.len(), fixture.synthetic_merge.len());
        let keys: Vec<_> = fixture
            .synthetic_merge
            .iter()
            .flat_map(|case| &case.keys)
            .collect();
        let has_token = |token: &str| {
            keys.iter()
                .any(|key| key.split('-').any(|part| part == token))
        };
        for token in NOISE
            .iter()
            .map(|(word, _)| word)
            .chain(GENERIC_OBJECTS)
            .chain(EVENT_CLASSES.iter().flat_map(|class| class.forms))
        {
            assert!(has_token(token), "missing fixture witness: {token}");
        }
        for (a, b, _) in COMPOUNDS {
            assert!(
                keys.iter().any(|key| key.contains(&format!("-{a}-{b}"))),
                "missing compound witness: {a} {b}"
            );
        }
        for (alias, _) in ALIASES {
            assert!(has_token(alias), "missing alias witness: {alias}");
        }
        for case in &fixture.synthetic_merge {
            assert!(
                fixture
                    .synthetic
                    .iter()
                    .any(|boundary| boundary.name == format!("{} boundary", case.name)),
                "missing boundary: {}",
                case.name
            );
        }
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
        for (prefix, scope) in NOISE {
            let prefixed = format!("{prefix}-openai-frontier-model-access-blocked");
            assert_eq!(canonical_signal_key(&prefixed), keys[0], "prefix {prefix}");
            if matches!(scope, ActorOnly) {
                assert!(
                    !is_noise_token(prefix),
                    "alias prefix must remain ordinary evidence: {prefix}"
                );
            }
        }
    }
}
