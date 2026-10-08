//! Configuration loading and management.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::infrastructure::devices::printer_models::{PrinterModel, PrinterModels};
use xc_kernel::ids::BranchId;

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
pub struct WebSettings {
    /// Directory holding the pages and the static files. A relative path is
    /// taken from the directory the server is started in.
    pub dir: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct FilesSettings {
    /// Directory the uploaded files are kept in. A relative path is taken from
    /// the directory the server is started in.
    pub dir: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AuthSettings {
    /// Seconds between two purges of expired sessions and past sign-in
    /// attempt windows; the first one runs when the server starts. At least 1.
    pub purge_interval_secs: u64,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            purge_interval_secs: 3600,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DevicesSettings {
    #[serde(rename = "printerModels")]
    pub printer_models: Vec<PrinterModel>,
    #[serde(default, rename = "passportDrawerBranches")]
    pub passport_drawer_branches: Vec<String>,
}

impl Default for DevicesSettings {
    fn default() -> Self {
        Self {
            printer_models: PrinterModels::built_in_models(),
            passport_drawer_branches: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub channel: Channel,
    pub database: DatabaseSettings,
    pub telemetry: TelemetrySettings,
    pub web: WebSettings,
    pub files: FilesSettings,
    #[serde(default)]
    pub auth: AuthSettings,
    #[serde(default)]
    pub devices: DevicesSettings,
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

        let settings: Settings = builder.build()?.try_deserialize()?;
        if settings.auth.purge_interval_secs == 0 {
            return Err(config::ConfigError::Message(
                "auth.purge_interval_secs must be at least 1".to_string(),
            ));
        }
        if let Err(e) = PrinterModels::new(settings.devices.printer_models.clone()) {
            return Err(config::ConfigError::Message(e));
        }
        for branch in &settings.devices.passport_drawer_branches {
            if !BranchId::is_well_formed(branch) {
                return Err(config::ConfigError::Message(format!(
                    "devices.passportDrawerBranches contains invalid branch id `{branch}`"
                )));
            }
        }
        if !settings.database.require_ssl && !is_local_database_host(&settings.database.host) {
            return Err(config::ConfigError::Message(format!(
                "database.require_ssl must be true for the non-local database host `{}` (APP_DATABASE__REQUIRE_SSL=true)",
                settings.database.host
            )));
        }
        Ok(settings)
    }
}

fn is_local_database_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.starts_with('/')
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
