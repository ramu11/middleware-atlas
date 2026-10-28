//! Typed configuration loaded from `config/default.toml` with environment overrides.
use serde::Deserialize;
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fmt,
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub application: ApplicationConfig,
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub database: DatabaseConfig,
    pub diagnostics: DiagnosticsConfig,
    #[serde(skip)]
    project_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationConfig {
    pub name: String,
    pub max_case_name_chars: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub web_origin: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub database_filename: String,
    pub max_upload_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    pub max_connections: u32,
    pub busy_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsConfig {
    pub fallback_kind: String,
    pub fallback_product: String,
    pub artifact_rules: Vec<ArtifactRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRule {
    pub kind: String,
    pub product: String,
    #[serde(default)]
    pub contains_any: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub match_mode: MatchMode,
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    #[default]
    Any,
    All,
}

#[derive(Debug)]
struct ConfigError(String);
impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for ConfigError {}

impl AppConfig {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let project_dir = configured_project_dir()?;
        let config_path = configured_config_path(&project_dir);
        let raw = std::fs::read_to_string(&config_path).map_err(|error| {
            ConfigError(format!(
                "Could not read configuration file {}: {error}",
                config_path.display()
            ))
        })?;
        let mut config: Self = toml::from_str(&raw).map_err(|error| {
            ConfigError(format!(
                "Invalid configuration in {}: {error}",
                config_path.display()
            ))
        })?;
        config.project_dir = project_dir;
        config.apply_environment_overrides()?;
        config.validate()?;
        Ok(config)
    }

    pub fn resolve_data_dir(&self) -> PathBuf {
        resolve_path(&self.project_dir, &self.storage.data_dir)
    }

    pub fn classify_artifact(&self, filename: &str) -> (&str, &str) {
        let normalized = filename.to_ascii_lowercase();
        self.diagnostics
            .artifact_rules
            .iter()
            .find(|rule| rule.matches(&normalized))
            .map(|rule| (rule.kind.as_str(), rule.product.as_str()))
            .unwrap_or((
                self.diagnostics.fallback_kind.as_str(),
                self.diagnostics.fallback_product.as_str(),
            ))
    }

    fn apply_environment_overrides(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(value) = nonempty_env("ATLAS_BIND_ADDR") {
            self.server.bind_addr = value;
        }
        if let Some(value) = nonempty_env("ATLAS_WEB_ORIGIN") {
            self.server.web_origin = value;
        }
        if let Some(value) = nonempty_env("ATLAS_DATA_DIR") {
            self.storage.data_dir = PathBuf::from(value);
        }
        parse_override("ATLAS_MAX_UPLOAD_BYTES", &mut self.storage.max_upload_bytes)?;
        parse_override(
            "ATLAS_DB_MAX_CONNECTIONS",
            &mut self.database.max_connections,
        )?;
        parse_override(
            "ATLAS_DB_BUSY_TIMEOUT_SECONDS",
            &mut self.database.busy_timeout_seconds,
        )?;
        Ok(())
    }

    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.application.name.trim().is_empty() {
            return Err(Box::new(ConfigError(
                "application.name cannot be empty".into(),
            )));
        }
        if self.application.max_case_name_chars == 0 || self.application.max_case_name_chars > 4096
        {
            return Err(Box::new(ConfigError(
                "application.max_case_name_chars must be between 1 and 4096".into(),
            )));
        }
        SocketAddr::from_str(&self.server.bind_addr).map_err(|error| {
            ConfigError(format!(
                "server.bind_addr is not a valid socket address: {error}"
            ))
        })?;
        if self.storage.data_dir.as_os_str().is_empty() {
            return Err(Box::new(ConfigError(
                "storage.data_dir cannot be empty".into(),
            )));
        }
        let db_name = Path::new(&self.storage.database_filename);
        if self.storage.database_filename.trim().is_empty()
            || db_name.components().count() != 1
            || db_name.file_name().and_then(OsStr::to_str)
                != Some(self.storage.database_filename.as_str())
            || self.storage.database_filename == "."
            || self.storage.database_filename == ".."
        {
            return Err(Box::new(ConfigError(
                "storage.database_filename must be a filename without directory components".into(),
            )));
        }
        if self.storage.max_upload_bytes == 0 || self.storage.max_upload_bytes > i64::MAX as u64 {
            return Err(Box::new(ConfigError(
                "storage.max_upload_bytes must be between 1 and i64::MAX".into(),
            )));
        }
        if self.database.max_connections == 0 || self.database.max_connections > 100 {
            return Err(Box::new(ConfigError(
                "database.max_connections must be between 1 and 100".into(),
            )));
        }
        if self.database.busy_timeout_seconds == 0 || self.database.busy_timeout_seconds > 300 {
            return Err(Box::new(ConfigError(
                "database.busy_timeout_seconds must be between 1 and 300".into(),
            )));
        }
        if self.diagnostics.fallback_kind.trim().is_empty()
            || self.diagnostics.fallback_product.trim().is_empty()
            || self.diagnostics.artifact_rules.is_empty()
        {
            return Err(Box::new(ConfigError(
                "diagnostics requires a fallback kind/product and at least one artifact rule"
                    .into(),
            )));
        }
        for rule in &self.diagnostics.artifact_rules {
            if rule.kind.trim().is_empty()
                || rule.product.trim().is_empty()
                || (rule.contains_any.is_empty() && rule.extensions.is_empty())
            {
                return Err(Box::new(ConfigError(
                    "each artifact rule must include kind, product, and at least one filename match".into(),
                )));
            }
            if rule
                .extensions
                .iter()
                .any(|extension| !extension.starts_with('.'))
            {
                return Err(Box::new(ConfigError(
                    "artifact rule extensions must start with a dot (for example, .log)".into(),
                )));
            }
            if rule.match_mode == MatchMode::All
                && (rule.contains_any.is_empty() || rule.extensions.is_empty())
            {
                return Err(Box::new(ConfigError(
                    "artifact rules with match_mode = 'all' must define both contains_any and extensions".into(),
                )));
            }
        }
        Ok(())
    }
}

impl ArtifactRule {
    fn matches(&self, normalized_filename: &str) -> bool {
        let contains_match = self.contains_any.iter().any(|needle| {
            !needle.is_empty() && normalized_filename.contains(&needle.to_ascii_lowercase())
        });
        let extension_match = self.extensions.iter().any(|extension| {
            !extension.is_empty() && normalized_filename.ends_with(&extension.to_ascii_lowercase())
        });
        match self.match_mode {
            MatchMode::Any => contains_match || extension_match,
            MatchMode::All => contains_match && extension_match,
        }
    }
}

fn configured_project_dir() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(value) = nonempty_env("ATLAS_PROJECT_DIR") {
        let path = expand_home(Path::new(&value));
        if !path.is_absolute() {
            return Err(Box::new(ConfigError(
                "ATLAS_PROJECT_DIR must be an absolute path so relative storage paths remain predictable".into(),
            )));
        }
        return Ok(path);
    }
    // CARGO_MANIFEST_DIR is .../<project>/crates/atlas-api; two parents up is the repo root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            Box::new(ConfigError(
                "Could not infer the Middleware Atlas project root; set ATLAS_PROJECT_DIR".into(),
            )) as Box<dyn Error>
        })
}

fn configured_config_path(project_dir: &Path) -> PathBuf {
    let configured = nonempty_env("ATLAS_CONFIG_FILE")
        .map(|value| PathBuf::from(value))
        .unwrap_or_else(|| PathBuf::from("config/default.toml"));
    resolve_path(project_dir, &configured)
}

fn resolve_path(project_dir: &Path, configured: &Path) -> PathBuf {
    let expanded = expand_home(configured);
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        project_dir.join(expanded)
    };
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn expand_home(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if value == "~" {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn parse_override<T>(name: &str, target: &mut T) -> Result<(), Box<dyn Error>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    if let Some(value) = nonempty_env(name) {
        *target = value.parse().map_err(|error| {
            ConfigError(format!(
                "Environment variable {name} has invalid value {value:?}: {error}"
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, MatchMode};
    use std::path::PathBuf;

    #[test]
    fn checked_in_configuration_parses_and_classifies_products() {
        let project_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .to_path_buf();
        let raw = std::fs::read_to_string(project_dir.join("config/default.toml")).unwrap();
        let mut config: AppConfig = toml::from_str(&raw).unwrap();
        config.project_dir = project_dir;
        config.validate().unwrap();
        assert_eq!(
            config.classify_artifact("java_pid.hprof"),
            ("heap_dump", "Java / JVM")
        );
        assert_eq!(
            config.classify_artifact("kafka-server.log"),
            ("kafka_log", "Apache Kafka / MirrorMaker 2")
        );
        assert_eq!(
            config.classify_artifact("sosreport-host.tar.xz"),
            ("sosreport", "Red Hat Enterprise Linux")
        );
        assert_eq!(
            config.classify_artifact("thread-dump.txt"),
            ("thread_dump", "Java / JVM")
        );
        assert_eq!(
            config.classify_artifact("unknown.data"),
            ("other", "Other / Unclassified")
        );
    }

    #[test]
    fn data_directory_is_anchored_to_project_root() {
        let mut config: AppConfig =
            toml::from_str(include_str!("../../../config/default.toml")).unwrap();
        let project_dir = std::env::temp_dir().join("middleware-atlas-project-test");
        config.project_dir = project_dir.clone();
        config.storage.data_dir = PathBuf::from("data");
        assert_eq!(config.resolve_data_dir(), project_dir.join("data"));
    }

    #[test]
    fn artifact_rules_can_require_both_token_and_extension() {
        let config: AppConfig =
            toml::from_str(include_str!("../../../config/default.toml")).unwrap();
        let rule = config
            .diagnostics
            .artifact_rules
            .iter()
            .find(|rule| rule.match_mode == MatchMode::All)
            .unwrap();
        assert!(rule.matches("server-thread.log"));
        assert!(!rule.matches("server-thread.zip"));
        assert!(!rule.matches("server.log"));
    }
}
