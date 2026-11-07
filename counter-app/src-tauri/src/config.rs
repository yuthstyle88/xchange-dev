//! The app's config. The server is the one origin the app opens and the bridge answers
//! (design §2). `capabilities/bridge.json` names the same origin; a test keeps the two equal.

use url::Url;

const DEFAULT_SERVER: &str = "https://xc-v3.spacing.cloud";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub server: Url,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: Url::parse(DEFAULT_SERVER).expect("the default server is a valid URL"),
        }
    }
}
