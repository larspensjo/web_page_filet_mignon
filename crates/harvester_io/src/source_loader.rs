use engine_logging::engine_warn;
use harvester_engine::{SourceRegistry, SourceRegistryValidationError};
use ron::de::{from_str, SpannedError};
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
    load_sources_from_str(&contents, config_dir)
}

fn load_sources_from_str(
    contents: &str,
    config_dir: &Path,
) -> Result<SourceRegistry, SourceLoaderError> {
    let registry: SourceRegistry = from_str(contents)?;
    registry.validate()?;
    Ok(registry.with_resolved_paths(config_dir))
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
        let err = load_sources_from_str(ron, dir.path()).unwrap_err();
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
}
