use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use harvester_core::AppViewModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BodyKey {
    Preview,
    TriageMarkdown,
    SummaryMarkdown,
    PollStatsMarkdown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyRef {
    pub key: BodyKey,
    pub content_hash: String,
    pub byte_len: usize,
}
pub type BodyTable = BTreeMap<BodyKey, String>;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEnvelope {
    pub generation: u64,
    pub schema_version: u32,
    pub view: serde_json::Value,
    pub fatal_message: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedSnapshot {
    pub schema_version: u32,
    pub view: serde_json::Value,
}

impl ProjectedSnapshot {
    pub fn with_generation(self, generation: u64) -> SnapshotEnvelope {
        SnapshotEnvelope {
            generation,
            schema_version: self.schema_version,
            view: self.view,
            fatal_message: None,
        }
    }
}

impl SnapshotEnvelope {
    pub fn fatal(generation: u64, message: String) -> Self {
        Self {
            generation,
            schema_version: crate::IPC_SCHEMA_VERSION,
            view: serde_json::Value::Null,
            fatal_message: Some(message),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyResponse {
    pub content_hash: String,
    pub text: String,
}

pub fn project(view: &AppViewModel) -> (ProjectedSnapshot, BodyTable) {
    let mut value = serde_json::to_value(view)
        .expect("AppViewModel serialization is an IPC contract and must not fail");
    let mut bodies = BodyTable::new();
    remove_pointer(&mut value, "/left_pane/prompt_lab");
    remove_pointer(&mut value, "/briefing_preview");
    remove_pointer(&mut value, "/right_pane/briefing_markdown");
    replace_body(&mut value, &mut bodies, "/preview_text", BodyKey::Preview);
    replace_body(
        &mut value,
        &mut bodies,
        "/right_pane/triage_markdown",
        BodyKey::TriageMarkdown,
    );
    replace_body(
        &mut value,
        &mut bodies,
        "/right_pane/summary_markdown",
        BodyKey::SummaryMarkdown,
    );
    replace_body(
        &mut value,
        &mut bodies,
        "/right_pane/poll_stats_markdown",
        BodyKey::PollStatsMarkdown,
    );
    (
        ProjectedSnapshot {
            schema_version: crate::IPC_SCHEMA_VERSION,
            view: value,
        },
        bodies,
    )
}

fn remove_pointer(value: &mut serde_json::Value, pointer: &str) {
    let (parent, key) = pointer
        .rsplit_once('/')
        .expect("projection pointers always name a field");
    let parent = if parent.is_empty() {
        value
    } else {
        value
            .pointer_mut(parent)
            .expect("AppViewModel IPC projection pointer must exist")
    };
    parent
        .as_object_mut()
        .expect("AppViewModel IPC projection parent must be an object")
        .remove(key);
}

fn replace_body(value: &mut serde_json::Value, table: &mut BodyTable, pointer: &str, key: BodyKey) {
    let slot = value
        .pointer_mut(pointer)
        .expect("AppViewModel IPC projection pointer must exist");
    let text = slot.as_str().map(str::to_string);
    *slot = serde_json::to_value(body_ref(table, key, text.as_deref()))
        .expect("BodyRef serialization cannot fail");
}

fn body_ref(table: &mut BodyTable, key: BodyKey, text: Option<&str>) -> Option<BodyRef> {
    let text = text?;
    let content_hash = hash(text);
    let body_ref = BodyRef {
        key,
        content_hash,
        byte_len: text.len(),
    };
    table.insert(key, text.to_string());
    Some(body_ref)
}
pub fn fetch_body(table: &BodyTable, key: BodyKey) -> Option<BodyResponse> {
    table.get(&key).map(|text| BodyResponse {
        content_hash: hash(text),
        text: text.clone(),
    })
}
pub(crate) fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::AppViewModel;
    use std::collections::BTreeSet;

    #[test]
    fn project_preserves_the_full_view_except_for_the_explicit_contract_paths() {
        let view = AppViewModel {
            preview_text: Some("preview".to_string()),
            right_pane: harvester_core::RightPaneView {
                triage_markdown: Some("triage".to_string()),
                summary_markdown: Some("summary".to_string()),
                poll_stats_markdown: Some("stats".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let raw = serde_json::to_value(&view).unwrap();
        let (projected, table) = project(&view);
        let envelope = projected.with_generation(42);
        assert_eq!(
            table.keys().copied().collect::<Vec<_>>(),
            vec![
                BodyKey::Preview,
                BodyKey::TriageMarkdown,
                BodyKey::SummaryMarkdown,
                BodyKey::PollStatsMarkdown
            ]
        );
        let raw_paths = value_paths(&raw);
        let envelope_paths = value_paths(&envelope.view);
        let stripped_roots = [
            "/left_pane/prompt_lab",
            "/briefing_preview",
            "/right_pane/briefing_markdown",
        ];
        let expected_removed = raw_paths
            .iter()
            .filter(|path| {
                stripped_roots
                    .iter()
                    .any(|root| path.as_str() == *root || path.starts_with(&format!("{root}/")))
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            raw_paths
                .difference(&envelope_paths)
                .cloned()
                .collect::<BTreeSet<_>>(),
            expected_removed
        );

        let replaced = BTreeSet::from([
            "/preview_text".to_string(),
            "/right_pane/triage_markdown".to_string(),
            "/right_pane/summary_markdown".to_string(),
            "/right_pane/poll_stats_markdown".to_string(),
        ]);
        let raw_leaf_paths = leaf_paths(&raw);
        let changed = raw_leaf_paths
            .iter()
            .filter(|path| {
                envelope.view.pointer(path).is_some()
                    && raw.pointer(path) != envelope.view.pointer(path)
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(changed, replaced);
        for path in raw_leaf_paths
            .iter()
            .filter(|path| envelope.view.pointer(path).is_some() && !replaced.contains(*path))
        {
            assert_eq!(raw.pointer(path), envelope.view.pointer(path), "{path}");
        }
        assert_eq!(
            fetch_body(&table, BodyKey::SummaryMarkdown),
            Some(BodyResponse {
                content_hash: hash("summary"),
                text: "summary".to_string(),
            })
        );
    }

    #[test]
    fn default_view_serializes_and_projection_is_pure() {
        let view = AppViewModel::default();
        assert!(serde_json::to_value(&view).is_ok());
        assert_eq!(project(&view), project(&view));
    }

    fn value_paths(value: &serde_json::Value) -> BTreeSet<String> {
        let mut paths = BTreeSet::new();
        collect_paths(value, "", &mut paths);
        paths
    }

    fn leaf_paths(value: &serde_json::Value) -> BTreeSet<String> {
        value_paths(value)
            .into_iter()
            .filter(|path| {
                !matches!(
                    value.pointer(path),
                    Some(serde_json::Value::Array(_) | serde_json::Value::Object(_))
                )
            })
            .collect()
    }

    fn collect_paths(value: &serde_json::Value, path: &str, paths: &mut BTreeSet<String>) {
        if !path.is_empty() {
            paths.insert(path.to_string());
        }
        match value {
            serde_json::Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    collect_paths(item, &format!("{path}/{index}"), paths);
                }
            }
            serde_json::Value::Object(fields) => {
                for (key, field) in fields {
                    collect_paths(
                        field,
                        &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                        paths,
                    );
                }
            }
            _ => {}
        }
    }
}
