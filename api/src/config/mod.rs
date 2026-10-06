//! Configuration loading and management.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use pos_infra::config::settings::{DatabaseSettings, TelemetrySettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Staging,
    Release,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Staging => "staging",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApplicationSettings {
    pub host: String,
    pub port: u16,
    pub workers: usize,
    pub shutdown_timeout_secs: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub channel: Channel,
    pub database: DatabaseSettings,
    pub telemetry: TelemetrySettings,
}

impl Settings {
    pub fn load() -> Result<Self, config::ConfigError> {
        let config_dir = std::env::var("APP_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("configuration"));
        Self::load_from(&config_dir, std::env::vars())
    }

    pub fn load_from<I>(config_dir: &Path, env: I) -> Result<Self, config::ConfigError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let env_map: HashMap<String, String> = env.into_iter().collect();

        let environment = env_map
            .get("APP_ENVIRONMENT")
            .map(|s| s.as_str())
            .unwrap_or("development");

        match environment {
            "development" | "staging" | "production" => {}
            bad => {
                return Err(config::ConfigError::Message(format!(
                    "unknown APP_ENVIRONMENT value `{bad}` (must be `development`, `staging`, or `production`)"
                )));
            }
        }

        let base_path = config_dir.join("base.yaml");
        if !base_path.exists() {
            return Err(config::ConfigError::NotFound(format!(
                "base.yaml is required but missing at {}",
                base_path.display()
            )));
        }

        let env_path = config_dir.join(format!("{environment}.yaml"));

        let mut builder = config::Config::builder().add_source(config::File::from(base_path));

        if env_path.exists() {
            builder = builder.add_source(config::File::from(env_path).required(false));
        }

        builder = builder.add_source(
            config::Environment::with_prefix("APP")
                .prefix_separator("_")
                .separator("__")
                .try_parsing(true)
                .source(Some(env_map)),
        );

        builder.build()?.try_deserialize()
    }
}
