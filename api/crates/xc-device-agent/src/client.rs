//! The wire between the agent and the server (§5.4c *Claim*, *Record*, *Identity*) and the
//! trait the runner talks to it through.

use std::future::Future;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CLAIM_MARGIN: Duration = Duration::from_secs(10);
const REPORT_TIMEOUT: Duration = Duration::from_secs(15);

/// The drawer this agent drives, as it reports it at every claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawerInfo {
    /// `CUPS` or `TCP`.
    pub transport: String,
    /// A CUPS queue name, or `host:port`.
    pub target: String,
}

/// `POST /api/device-agent/claim`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequest {
    /// At most 20,000.
    pub wait_ms: u64,
    pub version: String,
    pub build: String,
    pub os: String,
    pub host: String,
    pub drawer: DrawerInfo,
}

/// The 200 answer of a claim. Nothing else is ever sent to the counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claimed {
    pub job_id: String,
    pub lease_id: String,
    /// Base64 (standard alphabet, padded) of the kick bytes.
    pub bytes: String,
    /// How long after the answer arrived the bytes may still be sent.
    pub send_within_ms: u64,
}

/// What happened at the drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    DrawerOpened,
    Failed,
    Expired,
}

/// `POST /api/device-agent/report`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub job_id: String,
    pub lease_id: String,
    pub outcome: Outcome,
    /// Empty for `DRAWER_OPENED`; a code such as `QUEUE_BUSY`, `SEND_FAILED`,
    /// `AGENT_RESTARTED`, `SEND_WINDOW_PASSED` otherwise.
    pub reason: String,
    /// `{"cupsJob": "<queue>-<n>"}` when CUPS took the job; `{}` otherwise.
    pub detail: serde_json::Value,
}

/// `POST /api/device-agent/pair`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub code: String,
    pub host: String,
    pub os: String,
    pub version: String,
    pub build: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairAnswer {
    pub agent_id: String,
    pub token: String,
    pub branch_id: String,
    pub agent_name: String,
}

/// Why a call did not get its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401: the token is unknown or revoked — re-pair needed.
    Unauthorized,
    /// 429, with `Retry-After` in seconds.
    RateLimited { retry_after_secs: u64 },
    /// Any other answer that is not the expected one.
    Rejected { status: u16, error: String },
    /// No answer at all: connection, TLS, timeout.
    Connection(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "unauthorized (401)"),
            Self::RateLimited { retry_after_secs } => {
                write!(f, "rate limited (429), retry after {retry_after_secs}s")
            }
            Self::Rejected { status, error } => write!(f, "rejected with {status}: {error}"),
            Self::Connection(err) => write!(f, "connection error: {err}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// The server as the runner sees it. `HttpAgentApi` is the real one (rustls, HTTPS only,
/// `http://` refused except `localhost`); tests put a fake behind it.
pub trait AgentApi {
    fn pair(
        &self,
        request: &PairRequest,
    ) -> impl Future<Output = Result<PairAnswer, ApiError>> + Send;

    /// `Ok(None)` is the 204: nothing to do within `wait_ms`.
    fn claim(
        &self,
        request: &ClaimRequest,
    ) -> impl Future<Output = Result<Option<Claimed>, ApiError>> + Send;

    fn report(&self, report: &Report) -> impl Future<Output = Result<(), ApiError>> + Send;
}

/// The HTTPS client talking to `/api/device-agent/*`.
#[derive(Clone)]
pub struct HttpAgentApi {
    pub server_url: String,
    token: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for HttpAgentApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpAgentApi")
            .field("server_url", &self.server_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl HttpAgentApi {
    /// Connects to `server_url`. Refuses `http://` unless the host is `localhost` or `127.0.0.1`.
    pub fn new(server_url: impl Into<String>, token: impl Into<String>) -> Result<Self, ApiError> {
        let server_url = server_url.into();
        Self::validate_url(&server_url)?;
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| ApiError::Connection(e.to_string()))?;
        Ok(Self {
            server_url,
            token: token.into(),
            client,
        })
    }

    pub fn for_pairing(server_url: impl Into<String>) -> Result<Self, ApiError> {
        Self::new(server_url, "")
    }

    pub fn with_token(&self, token: impl Into<String>) -> Self {
        Self {
            server_url: self.server_url.clone(),
            token: token.into(),
            client: self.client.clone(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn validate_url(url_str: &str) -> Result<reqwest::Url, ApiError> {
        let url = reqwest::Url::parse(url_str).map_err(|e| ApiError::Connection(e.to_string()))?;
        match url.scheme() {
            "https" => Ok(url),
            "http" => {
                let host = url.host_str().unwrap_or("");
                if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]" {
                    Ok(url)
                } else {
                    Err(ApiError::Connection(
                        "http:// refused for non-local host; https:// required".to_owned(),
                    ))
                }
            }
            other => Err(ApiError::Connection(format!(
                "unsupported URL scheme {other}: https required"
            ))),
        }
    }

    fn redact_error(&self, err_text: String) -> String {
        if !self.token.is_empty() {
            err_text.replace(&self.token, "<redacted>")
        } else {
            err_text
        }
    }
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> u64 {
    if let Some(val) = headers.get("retry-after") {
        if let Ok(s) = val.to_str() {
            if let Ok(secs) = s.trim().parse::<u64>() {
                return secs;
            }
        }
    }
    1
}

impl AgentApi for HttpAgentApi {
    async fn pair(&self, request: &PairRequest) -> Result<PairAnswer, ApiError> {
        Self::validate_url(&self.server_url)?;
        let base = self.server_url.trim_end_matches('/');
        let url = format!("{base}/api/device-agent/pair");

        let response = self
            .client
            .post(&url)
            .json(request)
            .send()
            .await
            .map_err(|e| ApiError::Connection(self.redact_error(e.to_string())))?;

        let status = response.status();
        if status == reqwest::StatusCode::OK {
            response
                .json::<PairAnswer>()
                .await
                .map_err(|e| ApiError::Connection(self.redact_error(e.to_string())))
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err(ApiError::Unauthorized)
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = parse_retry_after(response.headers());
            Err(ApiError::RateLimited { retry_after_secs })
        } else {
            let error = response.text().await.unwrap_or_default();
            Err(ApiError::Rejected {
                status: status.as_u16(),
                error: self.redact_error(error),
            })
        }
    }

    async fn claim(&self, request: &ClaimRequest) -> Result<Option<Claimed>, ApiError> {
        Self::validate_url(&self.server_url)?;
        let base = self.server_url.trim_end_matches('/');
        let url = format!("{base}/api/device-agent/claim");

        let timeout = Duration::from_millis(request.wait_ms) + CLAIM_MARGIN;
        let mut req = self.client.post(&url).timeout(timeout);
        if !self.token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.token));
        }

        let response = req
            .json(request)
            .send()
            .await
            .map_err(|e| ApiError::Connection(self.redact_error(e.to_string())))?;

        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            Ok(None)
        } else if status == reqwest::StatusCode::OK {
            let claimed = response
                .json::<Claimed>()
                .await
                .map_err(|e| ApiError::Connection(self.redact_error(e.to_string())))?;
            Ok(Some(claimed))
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err(ApiError::Unauthorized)
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = parse_retry_after(response.headers());
            Err(ApiError::RateLimited { retry_after_secs })
        } else {
            let error = response.text().await.unwrap_or_default();
            Err(ApiError::Rejected {
                status: status.as_u16(),
                error: self.redact_error(error),
            })
        }
    }

    async fn report(&self, report: &Report) -> Result<(), ApiError> {
        Self::validate_url(&self.server_url)?;
        let base = self.server_url.trim_end_matches('/');
        let url = format!("{base}/api/device-agent/report");

        let mut req = self.client.post(&url).timeout(REPORT_TIMEOUT);
        if !self.token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.token));
        }

        let response = req
            .json(report)
            .send()
            .await
            .map_err(|e| ApiError::Connection(self.redact_error(e.to_string())))?;

        let status = response.status();
        if status == reqwest::StatusCode::OK || status == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else if status == reqwest::StatusCode::UNAUTHORIZED {
            Err(ApiError::Unauthorized)
        } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after_secs = parse_retry_after(response.headers());
            Err(ApiError::RateLimited { retry_after_secs })
        } else if status.is_server_error() {
            let body = response.text().await.unwrap_or_default();
            let detail = if body.is_empty() {
                format!("server error {}", status.as_u16())
            } else {
                format!("server error {}: {}", status.as_u16(), body)
            };
            Err(ApiError::Connection(self.redact_error(detail)))
        } else {
            let error = response.text().await.unwrap_or_default();
            Err(ApiError::Rejected {
                status: status.as_u16(),
                error: self.redact_error(error),
            })
        }
    }
}
