//! `XC_RECOGNIZE_*` (design `docs/design/recognize-passport-read.md` §6).

use std::time::Duration;

use crate::application::passport_read::AiProvider;

/// `XC_RECOGNIZE_TIMEOUT_MS` when unset or blank.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(60_000);

/// Where and how xchange calls Recognize. `Debug` never prints the token.
#[derive(Clone, PartialEq, Eq)]
pub struct RecognizeSettings {
    pub base_url: String,
    pub provider: AiProvider,
    pub model: Option<String>,
    pub timeout: Duration,
    token: String,
}

/// Why `Settings.recognize` is not usable. `Display` names the variable, never its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognizeConfigError {
    /// Neither `XC_RECOGNIZE_BASE_URL` nor `XC_RECOGNIZE_TOKEN` is set: a server without OCR.
    Unset,
    /// One of the two is set and this one is not.
    Missing { variable: &'static str },
    /// This variable's value is not one the design allows.
    Invalid { variable: &'static str },
}

impl std::fmt::Display for RecognizeConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unset => write!(
                f,
                "XC_RECOGNIZE_BASE_URL and XC_RECOGNIZE_TOKEN are not set"
            ),
            Self::Missing { variable } => write!(f, "{variable} is not set"),
            Self::Invalid { variable } => write!(f, "{variable} is not valid"),
        }
    }
}

impl std::error::Error for RecognizeConfigError {}

/// `Settings.recognize` when nothing was read (serde's default for the skipped field).
pub fn unset() -> Result<RecognizeSettings, RecognizeConfigError> {
    Err(RecognizeConfigError::Unset)
}

impl RecognizeSettings {
    /// Builds the settings from the five raw environment values (design §6 table).
    pub fn from_raw(
        base_url: Option<&str>,
        token: Option<&str>,
        provider: Option<&str>,
        model: Option<&str>,
        timeout_ms: Option<&str>,
    ) -> Result<Self, RecognizeConfigError> {
        fn trimmed(opt: Option<&str>) -> Option<&str> {
            opt.map(str::trim).filter(|s| !s.is_empty())
        }
        let base_url = trimmed(base_url);
        let token = trimmed(token);

        let (base_url, token) = match (base_url, token) {
            (None, None) => return Err(RecognizeConfigError::Unset),
            (Some(_), None) => {
                return Err(RecognizeConfigError::Missing {
                    variable: "XC_RECOGNIZE_TOKEN",
                });
            }
            (None, Some(_)) => {
                return Err(RecognizeConfigError::Missing {
                    variable: "XC_RECOGNIZE_BASE_URL",
                });
            }
            (Some(u), Some(t)) => (u, t),
        };

        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(RecognizeConfigError::Invalid {
                variable: "XC_RECOGNIZE_BASE_URL",
            });
        }

        let provider = match trimmed(provider) {
            None | Some("gemini") => AiProvider::Gemini,
            Some("ollama") => AiProvider::Ollama,
            Some(_) => {
                return Err(RecognizeConfigError::Invalid {
                    variable: "XC_RECOGNIZE_PROVIDER",
                });
            }
        };

        let model = trimmed(model).map(str::to_owned);

        let timeout = match trimmed(timeout_ms) {
            None => DEFAULT_TIMEOUT,
            Some(raw) => {
                let ms: u64 = raw.parse().map_err(|_| RecognizeConfigError::Invalid {
                    variable: "XC_RECOGNIZE_TIMEOUT_MS",
                })?;
                if !(1..=600_000).contains(&ms) {
                    return Err(RecognizeConfigError::Invalid {
                        variable: "XC_RECOGNIZE_TIMEOUT_MS",
                    });
                }
                Duration::from_millis(ms)
            }
        };

        Ok(Self {
            base_url: base_url.to_owned(),
            provider,
            model,
            timeout,
            token: token.to_owned(),
        })
    }

    /// The bearer token Recognize issued to xchange.
    pub fn token(&self) -> &str {
        &self.token
    }
}

fn redact_userinfo(url: &str) -> &str {
    if let Some((_, rest)) = url.split_once("://") {
        let authority_end = rest.find('/').unwrap_or(rest.len());
        if rest[..authority_end].contains('@') {
            return "<redacted>";
        }
    }
    url
}

impl std::fmt::Debug for RecognizeSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecognizeSettings")
            .field("base_url", &redact_userinfo(&self.base_url))
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}
