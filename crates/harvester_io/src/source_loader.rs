use engine_logging::engine_warn;
use harvester_engine::{SourceRegistry, SourceRegistryValidationError};
use ron::de::{from_str, SpannedError};
use ron::value::RawValue;
use serde::Deserialize;
use std::fs;
use std::io;
use std::path::Path;

/// Load the source registry from the given path.
/// Returns empty registry when the file is missing or invalid.
pub fn load_sources(path: &Path) -> SourceRegistry {
    let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
    match load_sources_from_path(path, config_dir) {
        Ok(registry) => registry,
        Err(SourceLoaderError::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
            SourceRegistry::default()
        }
        Err(err) => {
            engine_warn!("[source-config] failed to load {}: {}", path.display(), err);
            SourceRegistry::default()
        }
    }
}

fn load_sources_from_path(
    path: &Path,
    config_dir: &Path,
) -> Result<SourceRegistry, SourceLoaderError> {
    let contents = fs::read_to_string(path)?;
    load_sources_from_str(&contents, config_dir, path)
}

fn load_sources_from_str(
    contents: &str,
    config_dir: &Path,
    registry_path: &Path,
) -> Result<SourceRegistry, SourceLoaderError> {
    let (registry, skipped) = parse_source_registry(contents)?;
    for warning in skipped {
        engine_warn!(
            "[source-config] path={} {}",
            registry_path.display(),
            warning
        );
    }
    registry.validate()?;
    Ok(registry.with_resolved_paths(config_dir))
}

#[derive(Deserialize)]
#[serde(rename = "SourceRegistry")]
struct RawSourceRegistry {
    sources: Vec<Box<RawValue>>,
}

#[derive(Deserialize)]
#[serde(rename = "SourceConfig")]
struct RawSourceInfo {
    #[serde(default)]
    id: OptionalRawValue,
    source_type: Box<RawValue>,
}

#[derive(Default)]
struct OptionalRawValue(Option<Box<RawValue>>);

impl<'de> Deserialize<'de> for OptionalRawValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Box::<RawValue>::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

fn parse_source_registry(
    contents: &str,
) -> Result<(SourceRegistry, Vec<String>), SourceLoaderError> {
    let raw_registry: RawSourceRegistry = from_str(contents)?;
    let mut sources = Vec::with_capacity(raw_registry.sources.len());
    let mut warnings = Vec::new();

    for (index, raw_source) in raw_registry.sources.into_iter().enumerate() {
        let info: RawSourceInfo = raw_source.into_rust()?;
        let Some(source_type) = source_type_name(&info.source_type) else {
            // Keep malformed supported-entry errors on the original strict path.
            sources.push(raw_source.into_rust::<harvester_engine::SourceConfig>()?);
            continue;
        };

        if !is_supported_source_type_name(source_type) {
            let id = info
                .id
                .0
                .and_then(|id| id.into_rust::<String>().ok())
                .as_deref()
                .map(|id| format!(" id={id}"))
                .unwrap_or_default();
            warnings.push(format!(
                "skipping unsupported source type={source_type} position={}{}",
                index + 1,
                id
            ));
            continue;
        }

        sources.push(raw_source.into_rust::<harvester_engine::SourceConfig>()?);
    }

    Ok((SourceRegistry { sources }, warnings))
}

fn source_type_name(source_type: &RawValue) -> Option<&str> {
    let source_type = source_type.get_ron().trim_start();
    let identifier_length = source_type
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(source_type.len());
    (identifier_length > 0).then(|| &source_type[..identifier_length])
}

fn is_supported_source_type_name(name: &str) -> bool {
    matches!(name, "File" | "CuratedList" | "Rss" | "BraveNews")
}

#[derive(Debug, thiserror::Error)]
enum SourceLoaderError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] SpannedError),
    #[error(transparent)]
    Validation(#[from] SourceRegistryValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_engine::SourceType;
    use std::fs;
    use tempfile::TempDir;

    fn init_logging() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(engine_logging::initialize_for_tests);
    }

    fn make_registry_ron(path: &str, max: Option<usize>) -> String {
        format!(
            r#"
SourceRegistry(
    sources: [
        SourceConfig(
            id: "{id}",
            source_type: File(path: "{path}"),
            enabled: true,
            max_urls_per_poll: {max:?},
            description: "test",
        ),
    ],
)
"#,
            id = "feed",
            path = path,
            max = max
        )
    }

    #[test]
    fn load_missing_file_returns_empty() {
        init_logging();
        let temp = TempDir::new().expect("temp");
        let registry = load_sources(&temp.path().join("nope.ron"));
        assert!(registry.sources.is_empty());
    }

    #[test]
    fn loads_registry_and_resolves_relative_paths() {
        init_logging();
        let temp = TempDir::new().expect("temp");
        let config_path = temp.path().join("sources.ron");
        let contents = make_registry_ron("incoming.txt", Some(5));
        fs::write(&config_path, contents).expect("write config");

        let registry = load_sources(&config_path);
        assert_eq!(registry.sources.len(), 1);
        if let SourceType::File { path } = &registry.sources[0].source_type {
            assert_eq!(path, &temp.path().join("incoming.txt"));
        } else {
            panic!("expected file source");
        }
    }

    #[test]
    fn duplicate_source_ids_are_rejected() {
        init_logging();
        let ron = r#"
SourceRegistry(
    sources: [
        SourceConfig(
            id: "dup",
            source_type: CuratedList(urls: []),
            enabled: true,
            max_urls_per_poll: None,
            description: "",
        ),
        SourceConfig(
            id: "dup",
            source_type: CuratedList(urls: []),
            enabled: true,
            max_urls_per_poll: None,
            description: "",
        ),
    ],
)
"#;
        let dir = TempDir::new().expect("temp");
        let err =
            load_sources_from_str(ron, dir.path(), &dir.path().join(".sources.ron")).unwrap_err();
        assert!(matches!(err, SourceLoaderError::Validation(..)));
    }

    #[test]
    fn invalid_ron_returns_empty_registry() {
        init_logging();
        let temp = TempDir::new().expect("temp");
        let config_path = temp.path().join("bad.ron");
        fs::write(&config_path, "this is not ron").expect("write");
        let registry = load_sources(&config_path);
        assert!(registry.sources.is_empty());
    }

    #[test]
    fn loads_brave_news_source_from_ron() {
        init_logging();
        let temp = TempDir::new().expect("temp");
        let config_path = temp.path().join("sources.ron");
        let contents = r#"
SourceRegistry(
    sources: [
        SourceConfig(
            id: "brave-test",
            source_type: BraveNews((
                query: "\"AI\" AND \"chips\"",
                api_key_env: "BRAVE_API_KEY",
                count: Some(10),
                freshness: Some("pd"),
            )),
            enabled: true,
            max_urls_per_poll: Some(10),
            description: "test brave source",
        ),
    ],
)
"#;
        fs::write(&config_path, contents).expect("write config");
        let registry = load_sources(&config_path);
        assert_eq!(registry.sources.len(), 1);
        assert!(matches!(
            registry.sources[0].source_type,
            SourceType::BraveNews(_)
        ));
    }

    #[test]
    fn raw_entry_capture_handles_production_shape_and_skips_unknown_types() {
        init_logging();
        let contents =
            include_str!("../tests/fixtures/production_shape_sources_with_unsupported.ron");
        let (registry, warnings) = parse_source_registry(contents).expect("parse registry");

        assert_eq!(registry.sources.len(), 29);
        assert_eq!(
            registry
                .sources
                .iter()
                .filter(|source| matches!(source.source_type, SourceType::BraveNews(_)))
                .count(),
            24
        );
        assert_eq!(
            registry
                .sources
                .iter()
                .filter(|source| matches!(source.source_type, SourceType::Rss { .. }))
                .count(),
            5
        );
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].contains("source type=Script position=30 id=legacy-script"));
        assert!(warnings[1].contains("source type=BraveNewz position=31 id=misspelled-type"));

        let loaded = load_sources_from_str(contents, Path::new("."), Path::new(".sources.ron"))
            .expect("valid real registry entries load after unsupported entries are skipped");
        assert_eq!(loaded.sources.len(), 29);
    }

    #[test]
    fn every_supported_source_variant_is_accepted_by_the_loader() {
        let variants = [
            SourceType::File {
                path: "sample.txt".into(),
            },
            SourceType::CuratedList { urls: vec![] },
            SourceType::Rss {
                feed_url: "https://example.invalid/feed".into(),
            },
            SourceType::BraveNews(harvester_engine::BraveNewsSourceConfig {
                query: "sample".into(),
                api_key_env: "BRAVE_SEARCH_API_KEY".into(),
                count: None,
                freshness: None,
            }),
        ];
        for variant in variants {
            let serialized = ron::to_string(&variant).expect("serialize source variant");
            let raw: Box<RawValue> = ron::from_str(&serialized).expect("read raw source variant");
            assert_eq!(source_type_name(&raw), Some(variant.ron_variant_name()));
            assert!(is_supported_source_type_name(variant.ron_variant_name()));
        }
    }

    #[test]
    fn file_and_curated_list_sources_remain_parseable() {
        let ron = r#"
SourceRegistry(
    sources: [
        SourceConfig(
            id: "file-source",
            source_type: File(path: "feeds.txt"),
            enabled: true,
            max_urls_per_poll: None,
            description: "",
        ),
        SourceConfig(
            id: "curated-source",
            source_type: CuratedList(urls: ["https://example.invalid/article"]),
            enabled: true,
            max_urls_per_poll: None,
            description: "",
        ),
    ],
)
"#;
        let registry = load_sources_from_str(ron, Path::new("."), Path::new(".sources.ron"))
            .expect("File and CuratedList entries should load");
        assert!(matches!(
            registry.sources[0].source_type,
            SourceType::File { .. }
        ));
        assert!(matches!(
            registry.sources[1].source_type,
            SourceType::CuratedList { .. }
        ));
    }
}
