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

pub use xc_auth::infrastructure::identity::IdentitySettings;

/// The official BoT API (`docs/design/rate-feed-connector.md` §8.4): `APP_BOT__API_TOKEN`,
/// `APP_BOT__API_BASE`.
// scaffold (#1066): the field shapes only. The implementation gives `api_base` its default
// (`https://gateway.api.bot.or.th/Stat-ExchangeRate/v2`) and makes `load_from` take
// `api_token` raw from the environment (empty = absent).
#[derive(Clone, serde::Deserialize)]
#[serde(default)]
pub struct BotSettings {
    pub api_token: Option<String>,
    pub api_base: String,
}

impl Default for BotSettings {
    fn default() -> Self {
        Self {
            api_token: None,
            api_base: "https://gateway.api.bot.or.th/Stat-ExchangeRate/v2".to_string(),
        }
    }
}

impl std::fmt::Debug for BotSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotSettings")
            .field("api_token", &self.api_token.as_ref().map(|_| "<redacted>"))
            .field("api_base", &self.api_base)
            .finish()
    }
}

/// Where the Broker attribution handler sends (RA-2 #980, design `broker-seam.md` §5.4). Read
/// from `XC_BROKER_BASE_URL` and `XC_BROKER_SERVICE_TOKEN`; `None` when either is unset or blank,
/// and then no handler is registered. `Debug` never prints the token.
#[derive(Clone, PartialEq, Eq)]
pub struct BrokerSettings {
    pub base_url: String,
    pub service_token: String,
}

impl std::fmt::Debug for BrokerSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerSettings")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

pub use xc_loyalty::infrastructure::config::LoyaltyConfig as LoyaltySettings;

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
    #[serde(default)]
    pub identity: Option<IdentitySettings>,
    #[serde(default)]
    pub bot: BotSettings,
    #[serde(skip)]
    pub broker: Option<BrokerSettings>,
    #[serde(skip)]
    pub loyalty: Option<LoyaltySettings>,
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

        let identity_base_url = env_map.get("IDENTITY_BASE_URL");
        let identity_client_id = env_map.get("IDENTITY_CLIENT_ID");
        let identity_client_secret = env_map.get("IDENTITY_CLIENT_SECRET");
        let identity_audience = env_map.get("IDENTITY_AUDIENCE");
        let identity_issuer = env_map.get("IDENTITY_ISSUER");
        let identity_api_key = env_map.get("IDENTITY_API_KEY");

        let required = [
            ("IDENTITY_BASE_URL", identity_base_url),
            ("IDENTITY_CLIENT_ID", identity_client_id),
            ("IDENTITY_CLIENT_SECRET", identity_client_secret),
            ("IDENTITY_AUDIENCE", identity_audience),
            ("IDENTITY_ISSUER", identity_issuer),
        ];
        let count = required.iter().filter(|(_, v)| v.is_some()).count();

        let identity = if count == 5 {
            let api_key = identity_api_key.cloned();
            Some(IdentitySettings {
                base_url: identity_base_url.unwrap().clone(),
                client_id: identity_client_id.unwrap().clone(),
                client_secret: identity_client_secret.unwrap().clone(),
                audience: identity_audience.unwrap().clone(),
                issuer: identity_issuer.unwrap().clone(),
                api_key,
            })
        } else if count == 0 && identity_api_key.is_none() {
            None
        } else {
            return Err(config::ConfigError::Message(
                "IDENTITY_* variables set in part: IDENTITY_BASE_URL, IDENTITY_CLIENT_ID, IDENTITY_CLIENT_SECRET, IDENTITY_AUDIENCE, IDENTITY_ISSUER must all be set together".to_string()
            ));
        };

        let broker = match (
            env_map.get("XC_BROKER_BASE_URL"),
            env_map.get("XC_BROKER_SERVICE_TOKEN"),
        ) {
            (Some(base_url), Some(service_token))
                if !base_url.trim().is_empty() && !service_token.trim().is_empty() =>
            {
                Some(BrokerSettings {
                    base_url: base_url.trim().to_owned(),
                    service_token: service_token.trim().to_owned(),
                })
            }
            _ => None,
        };

        let loyalty = LoyaltySettings::from_raw(
            env_map.get("XC_LOYALTY_BASE_URL").map(|s| s.as_str()),
            env_map.get("XC_LOYALTY_TENANT_ID").map(|s| s.as_str()),
            env_map.get("XC_LOYALTY_WEBHOOK_SECRET").map(|s| s.as_str()),
            env_map.get("XC_LOYALTY_TIMEOUT_MS").map(|s| s.as_str()),
        );

        let bot_token = env_map
            .get("APP_BOT__API_TOKEN")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

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

        let mut env_map_for_builder = env_map;
        env_map_for_builder.remove("APP_BOT__API_TOKEN");

        builder = builder.add_source(
            config::Environment::with_prefix("APP")
                .prefix_separator("_")
                .separator("__")
                .try_parsing(true)
                .source(Some(env_map_for_builder)),
        );

        let mut settings: Settings = builder.build()?.try_deserialize()?;
        settings.identity = identity;
        settings.broker = broker;
        settings.loyalty = loyalty;
        settings.bot.api_token = bot_token;
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

    pub const MISSING_IDENTITY_API_KEY_NOTICE: &'static str =
        "IDENTITY_API_KEY is not set; calls to identity may fail closed";

    pub fn identity_api_key_notice(&self) -> Option<&'static str> {
        if self
            .identity
            .as_ref()
            .is_some_and(|id| id.api_key.is_none())
        {
            Some(Self::MISSING_IDENTITY_API_KEY_NOTICE)
        } else {
            None
        }
    }
}

fn is_local_database_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.starts_with('/')
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
