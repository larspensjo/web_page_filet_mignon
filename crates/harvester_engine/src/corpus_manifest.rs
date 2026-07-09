use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{json, Value};

use crate::persist::{AtomicFileWriter, PersistError};

pub const CORPUS_MANIFEST_FILENAME: &str = "harvester-corpus.json";
pub const CORPUS_SCHEMA_VERSION: u32 = 1;

pub fn write_corpus_manifest(output_dir: &Path) -> Result<PathBuf, PersistError> {
    let manifest = build_corpus_manifest(&Utc::now().to_rfc3339());
    let content = format!("{manifest}\n");
    AtomicFileWriter::new(output_dir.to_path_buf()).write(CORPUS_MANIFEST_FILENAME, &content)
}

pub fn build_corpus_manifest(written_at_utc: &str) -> Value {
    json!({
        "format": "harvester-corpus",
        "schema_version": CORPUS_SCHEMA_VERSION,
        "written_at_utc": written_at_utc,
        "producer": {
            "name": "harvester",
            "crate": "harvester_engine",
            "crate_version": env!("CARGO_PKG_VERSION")
        },
        "layout": {
            "articles": ["*.md", "linked/*.md"],
            "generated_artifacts": [
                "archive.md",
                "archive-*.md",
                "export.txt",
                "manifest.json",
                "summary_refresh_reports/",
                ".summary_refresh_last.json"
            ],
            "internal_state": [
                ".*.ron",
                "llm_results/",
                "logs/"
            ]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_records_current_schema_version_and_layout() {
        let manifest = build_corpus_manifest("2026-07-09T00:00:00Z");

        assert_eq!(manifest["format"].as_str(), Some("harvester-corpus"));
        assert_eq!(
            manifest["schema_version"].as_u64(),
            Some(CORPUS_SCHEMA_VERSION as u64)
        );
        assert_eq!(
            manifest["layout"]["articles"].as_array().unwrap(),
            &vec![json!("*.md"), json!("linked/*.md")]
        );
        assert!(manifest["layout"]["internal_state"]
            .as_array()
            .unwrap()
            .contains(&json!(".*.ron")));
    }

    #[test]
    fn write_corpus_manifest_creates_output_marker() {
        let dir = tempfile::tempdir().expect("tempdir");

        let path = write_corpus_manifest(dir.path()).expect("write manifest");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(CORPUS_MANIFEST_FILENAME)
        );
        let content = std::fs::read_to_string(path).expect("read manifest");
        let parsed: Value = serde_json::from_str(&content).expect("valid json");
        assert_eq!(
            parsed["schema_version"].as_u64(),
            Some(CORPUS_SCHEMA_VERSION as u64)
        );
    }
}
