use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(String);

impl SourceId {
    pub fn new(value: impl AsRef<str>) -> Result<Self, SourceIdError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(SourceIdError::Empty);
        }
        for ch in value.chars() {
            match ch {
                'a'..='z' | '0'..='9' | '-' | '_' | '.' => {}
                _ => return Err(SourceIdError::InvalidCharacter(ch)),
            }
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SourceId {
    fn default() -> Self {
        Self("default".to_string())
    }
}

impl Serialize for SourceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        SourceId::new(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BraveNewsSourceConfig {
    pub query: String,
    pub api_key_env: String,
    /// Request size — how many results to fetch from Brave (1..=50).
    /// This is NOT the emit cap; `max_urls_per_poll` on `SourceConfig` controls that.
    pub count: Option<usize>,
    /// Freshness filter: `pd` (past day), `pw` (past week), `pm` (past month),
    /// `py` (past year), or `YYYY-MM-DDtoYYYY-MM-DD`.
    pub freshness: Option<String>,
}

/// Discriminant for a source type — used in poll stats and summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    Rss,
    Brave,
    File,
    Curated,
    Script,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    File { path: PathBuf },
    Script { command: String, args: Vec<String> },
    CuratedList { urls: Vec<String> },
    Rss { feed_url: String },
    BraveNews(BraveNewsSourceConfig),
}

impl SourceType {
    pub fn kind(&self) -> SourceKind {
        match self {
            SourceType::File { .. } => SourceKind::File,
            SourceType::Script { .. } => SourceKind::Script,
            SourceType::CuratedList { .. } => SourceKind::Curated,
            SourceType::Rss { .. } => SourceKind::Rss,
            SourceType::BraveNews(_) => SourceKind::Brave,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: SourceId,
    pub source_type: SourceType,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub max_urls_per_poll: Option<usize>,
    #[serde(default)]
    pub description: String,
}

fn default_enabled() -> bool {
    true
}

impl SourceConfig {
    pub fn resolve_paths(&self, config_dir: &Path) -> Self {
        let source_type = self.source_type.resolve_paths(config_dir);
        SourceConfig {
            source_type,
            ..self.clone()
        }
    }
}

impl SourceType {
    fn resolve_paths(&self, config_dir: &Path) -> Self {
        match self {
            SourceType::File { path } => SourceType::File {
                path: resolve_config_path(config_dir, path),
            },
            SourceType::Script { command, args } => SourceType::Script {
                command: command.clone(),
                args: args.clone(),
            },
            SourceType::CuratedList { urls } => SourceType::CuratedList { urls: urls.clone() },
            SourceType::Rss { feed_url } => SourceType::Rss {
                feed_url: feed_url.clone(),
            },
            SourceType::BraveNews(_) => self.clone(),
        }
    }
}

fn resolve_config_path(config_dir: &Path, candidate: &Path) -> PathBuf {
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        config_dir.join(candidate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceRegistry {
    pub sources: Vec<SourceConfig>,
}

impl SourceRegistry {
    pub fn validate(&self) -> Result<(), SourceRegistryValidationError> {
        let mut seen = HashSet::new();
        for source in &self.sources {
            if !seen.insert(source.id.clone()) {
                return Err(SourceRegistryValidationError::DuplicateSourceId(
                    source.id.clone(),
                ));
            }

            if let SourceType::Rss { feed_url } = &source.source_type {
                let trimmed = feed_url.trim();
                if trimmed.is_empty() {
                    return Err(SourceRegistryValidationError::InvalidFeedUrl {
                        source_id: source.id.clone(),
                        reason: "feed_url cannot be empty".to_string(),
                    });
                }

                Url::parse(trimmed).map_err(|err| {
                    SourceRegistryValidationError::InvalidFeedUrl {
                        source_id: source.id.clone(),
                        reason: err.to_string(),
                    }
                })?;
            }

            if let SourceType::BraveNews(cfg) = &source.source_type {
                if cfg.query.trim().is_empty() {
                    return Err(SourceRegistryValidationError::InvalidBraveConfig {
                        source_id: source.id.clone(),
                        reason: "query cannot be empty".to_string(),
                    });
                }
                if cfg.api_key_env.trim().is_empty() {
                    return Err(SourceRegistryValidationError::InvalidBraveConfig {
                        source_id: source.id.clone(),
                        reason: "api_key_env cannot be empty".to_string(),
                    });
                }
                if let Some(count) = cfg.count {
                    if !(1..=50).contains(&count) {
                        return Err(SourceRegistryValidationError::InvalidBraveConfig {
                            source_id: source.id.clone(),
                            reason: format!("count must be 1..=50, got {}", count),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn with_resolved_paths(self, config_dir: &Path) -> Self {
        SourceRegistry {
            sources: self
                .sources
                .into_iter()
                .map(|source| source.resolve_paths(config_dir))
                .collect(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceIdError {
    #[error("source id cannot be empty")]
    Empty,
    #[error("source id contains invalid character '{0}'")]
    InvalidCharacter(char),
}

#[derive(Debug, thiserror::Error)]
pub enum SourceRegistryValidationError {
    #[error("duplicate source id: {0}")]
    DuplicateSourceId(SourceId),
    #[error("rss source '{source_id}' has invalid feed url: {reason}")]
    InvalidFeedUrl { source_id: SourceId, reason: String },
    #[error("brave source '{source_id}' config invalid: {reason}")]
    InvalidBraveConfig { source_id: SourceId, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_id_accepts_valid_name() {
        let id = SourceId::new("abc-123_xyz").expect("parsed");
        assert_eq!(id.as_str(), "abc-123_xyz");
    }

    #[test]
    fn source_id_rejects_invalid_chars() {
        assert!(matches!(
            SourceId::new("bad id"),
            Err(SourceIdError::InvalidCharacter(' '))
        ));
        assert!(matches!(
            SourceId::new("!bang"),
            Err(SourceIdError::InvalidCharacter('!'))
        ));
    }

    #[test]
    fn source_registry_detects_duplicate_ids() {
        let registry = SourceRegistry {
            sources: vec![
                SourceConfig {
                    id: SourceId::new("one").unwrap(),
                    source_type: SourceType::CuratedList { urls: vec![] },
                    enabled: true,
                    max_urls_per_poll: None,
                    description: String::new(),
                },
                SourceConfig {
                    id: SourceId::new("one").unwrap(),
                    source_type: SourceType::CuratedList { urls: vec![] },
                    enabled: true,
                    max_urls_per_poll: None,
                    description: String::new(),
                },
            ],
        };
        assert!(matches!(
            registry.validate(),
            Err(SourceRegistryValidationError::DuplicateSourceId(_))
        ));
    }

    #[test]
    fn source_registry_resolves_relative_file_paths() {
        let config_dir = Path::new("config");
        let registry = SourceRegistry {
            sources: vec![SourceConfig {
                id: SourceId::new("file").unwrap(),
                source_type: SourceType::File {
                    path: PathBuf::from("feed.txt"),
                },
                enabled: true,
                max_urls_per_poll: None,
                description: String::new(),
            }],
        };
        let resolved = registry.with_resolved_paths(config_dir);
        if let SourceType::File { path } = &resolved.sources[0].source_type {
            assert_eq!(path, &config_dir.join("feed.txt"));
        } else {
            panic!("expected file source");
        }
    }

    #[test]
    fn rss_source_registry_accepts_valid_feed() {
        let registry = SourceRegistry {
            sources: vec![SourceConfig {
                id: SourceId::new("rss").unwrap(),
                source_type: SourceType::Rss {
                    feed_url: "https://example.com/feed".to_string(),
                },
                enabled: true,
                max_urls_per_poll: None,
                description: String::new(),
            }],
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn rss_source_registry_rejects_empty_feed_url() {
        let registry = SourceRegistry {
            sources: vec![SourceConfig {
                id: SourceId::new("rss").unwrap(),
                source_type: SourceType::Rss {
                    feed_url: "".to_string(),
                },
                enabled: true,
                max_urls_per_poll: None,
                description: String::new(),
            }],
        };

        assert!(matches!(
            registry.validate(),
            Err(SourceRegistryValidationError::InvalidFeedUrl { .. })
        ));
    }

    #[test]
    fn rss_source_registry_detects_bad_url() {
        let registry = SourceRegistry {
            sources: vec![SourceConfig {
                id: SourceId::new("rss").unwrap(),
                source_type: SourceType::Rss {
                    feed_url: "not-a-url".to_string(),
                },
                enabled: true,
                max_urls_per_poll: None,
                description: String::new(),
            }],
        };

        assert!(matches!(
            registry.validate(),
            Err(SourceRegistryValidationError::InvalidFeedUrl { .. })
        ));
    }

    fn brave_source(query: &str, api_key_env: &str, count: Option<usize>) -> SourceRegistry {
        SourceRegistry {
            sources: vec![SourceConfig {
                id: SourceId::new("brave").unwrap(),
                source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                    query: query.to_string(),
                    api_key_env: api_key_env.to_string(),
                    count,
                    freshness: None,
                }),
                enabled: true,
                max_urls_per_poll: None,
                description: String::new(),
            }],
        }
    }

    #[test]
    fn brave_news_source_round_trips_through_ron() {
        let config = SourceConfig {
            id: SourceId::new("brave-test").unwrap(),
            source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                query: "\"AI\" AND \"data center\"".to_string(),
                api_key_env: "BRAVE_API_KEY".to_string(),
                count: Some(10),
                freshness: Some("pd".to_string()),
            }),
            enabled: true,
            max_urls_per_poll: Some(10),
            description: "test".to_string(),
        };

        let ron_str = ron::to_string(&config).expect("serialize");
        let parsed: SourceConfig = ron::from_str(&ron_str).expect("deserialize");
        assert_eq!(parsed, config);
    }

    #[test]
    fn brave_news_rejects_empty_query() {
        assert!(brave_source("", "KEY", None).validate().is_err());
    }

    #[test]
    fn brave_news_rejects_empty_api_key_env() {
        assert!(brave_source("test", "", None).validate().is_err());
    }

    #[test]
    fn brave_news_rejects_count_over_50() {
        assert!(brave_source("test", "KEY", Some(51)).validate().is_err());
    }

    #[test]
    fn brave_news_rejects_count_zero() {
        assert!(brave_source("test", "KEY", Some(0)).validate().is_err());
    }

    #[test]
    fn brave_news_accepts_valid_config() {
        assert!(brave_source("AI news", "BRAVE_API_KEY", Some(10))
            .validate()
            .is_ok());
    }

    #[test]
    fn brave_news_resolve_paths_is_noop() {
        let config = SourceConfig {
            id: SourceId::new("brave").unwrap(),
            source_type: SourceType::BraveNews(BraveNewsSourceConfig {
                query: "test".to_string(),
                api_key_env: "KEY".to_string(),
                count: None,
                freshness: None,
            }),
            enabled: true,
            max_urls_per_poll: None,
            description: String::new(),
        };
        let resolved = config.resolve_paths(Path::new("/tmp"));
        assert_eq!(resolved, config);
    }

    #[test]
    fn rss_resolve_paths_is_noop() {
        let config = SourceConfig {
            id: SourceId::new("rss").unwrap(),
            source_type: SourceType::Rss {
                feed_url: "https://example.com/feed".to_string(),
            },
            enabled: true,
            max_urls_per_poll: None,
            description: String::new(),
        };

        let resolved = config.resolve_paths(Path::new("/tmp"));
        if let SourceType::Rss { feed_url } = resolved.source_type {
            assert_eq!(feed_url, "https://example.com/feed");
        } else {
            panic!("expected rss source");
        }
    }
}
