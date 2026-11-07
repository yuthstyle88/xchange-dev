//! CA-3d. The counter's agent inside the app: its identity, the release gate, the drawer runner
//! and the desk's binding nonce (design §8.2, §5.4).

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use url::Url;
use xc_device_agent::client::{ApiError, DrawerInfo, HttpAgentApi};
use xc_device_agent::config::{BUILD, VERSION, current_host, current_os, load_config};
use xc_device_agent::cups_transport::CupsTransport;
use xc_device_agent::journal::Journal;
use xc_device_agent::runner::{AgentConfig, AgentRunner, run_forever};
use xc_device_agent::tcp_transport::TcpTransport9100;
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

use crate::keychain::TokenStore;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const SERVER_TOO_OLD_MSG: &str = "เซิร์ฟเวอร์ยังไม่รองรับเคาน์เตอร์";

/// What each capability resolves to on the server (§8.2): NULL is already false, false, true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub cash_drawer: bool,
    pub scanner: bool,
    pub kiosk_lock: bool,
}

/// `GET /api/device-agent/info`, 200 (§8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub agent_id: String,
    pub counter_name: String,
    pub branch_id: String,
    pub capabilities: Capabilities,
}

/// `xcCounter.info()` (§3): the server's answer plus this build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterInfo {
    pub agent_id: String,
    pub counter_name: String,
    pub branch_id: String,
    pub capabilities: Capabilities,
    pub app_version: String,
    pub build: String,
}

/// `POST /api/device-agent/desk-binding`, 200 (§8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeskBinding {
    pub nonce: String,
    pub expires_at: String,
}

/// What the app does after asking `GET /api/device-agent/info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// 200: start the runner.
    Run,
    /// 404: a server older than CA-1. Show "เซิร์ฟเวอร์ยังไม่รองรับเคาน์เตอร์" and claim nothing.
    ServerTooOld,
    /// 401: the token is unknown or revoked.
    RePair,
    /// Anything else: ask again later, claiming nothing meanwhile.
    Retry,
}

/// The release gate (§8.2). Pure. The table is at the top of `tests/gate_test.rs`.
pub fn gate(status: u16) -> Gate {
    match status {
        200 => Gate::Run,
        404 => Gate::ServerTooOld,
        401 => Gate::RePair,
        _ => Gate::Retry,
    }
}

fn load_bearer_token(tokens: &dyn TokenStore) -> Result<String, ApiError> {
    match tokens.load() {
        Ok(Some(t)) if !t.is_empty() => Ok(t),
        Ok(_) => Err(ApiError::Unauthorized),
        Err(e) => Err(ApiError::Connection(e.to_string())),
    }
}

fn build_client(server: &Url) -> Result<reqwest::Client, ApiError> {
    HttpAgentApi::validate_url(server.as_str())?;
    reqwest::Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| ApiError::Connection(e.to_string()))
}

fn redact(token: &str, text: String) -> String {
    if token.is_empty() {
        text
    } else {
        text.replace(token, "<redacted>")
    }
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> u64 {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1)
}

/// `GET /api/device-agent/info`, with the status for [`gate`]. The body is `Some` only on 200.
pub async fn fetch_info(
    server: &Url,
    tokens: &dyn TokenStore,
) -> Result<(u16, Option<ServerInfo>), ApiError> {
    let token = load_bearer_token(tokens)?;
    let client = build_client(server)?;
    let base = server.as_str().trim_end_matches('/');
    let url = format!("{base}/api/device-agent/info");

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| ApiError::Connection(redact(&token, e.to_string())))?;

    let status = response.status().as_u16();
    if status == 200 {
        let info = response
            .json::<ServerInfo>()
            .await
            .map_err(|e| ApiError::Connection(redact(&token, e.to_string())))?;
        Ok((200, Some(info)))
    } else {
        Ok((status, None))
    }
}

/// The answer of `xcCounter.info()`.
pub async fn counter_info(server: &Url, tokens: &dyn TokenStore) -> Result<CounterInfo, ApiError> {
    let (status, info) = fetch_info(server, tokens).await?;
    match (gate(status), info) {
        (Gate::Run, Some(info)) => Ok(CounterInfo {
            agent_id: info.agent_id,
            counter_name: info.counter_name,
            branch_id: info.branch_id,
            capabilities: info.capabilities,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            build: BUILD.to_owned(),
        }),
        (Gate::ServerTooOld, _) => Err(ApiError::Rejected {
            status: 404,
            error: SERVER_TOO_OLD_MSG.to_owned(),
        }),
        (Gate::RePair, _) => {
            let _ = tokens.clear();
            Err(ApiError::Unauthorized)
        }
        (_, _) => Err(ApiError::Rejected {
            status,
            error: format!("unexpected status {status}"),
        }),
    }
}

/// `POST /api/device-agent/desk-binding`.
pub async fn desk_binding(server: &Url, tokens: &dyn TokenStore) -> Result<DeskBinding, ApiError> {
    let token = load_bearer_token(tokens)?;
    let client = build_client(server)?;
    let base = server.as_str().trim_end_matches('/');
    let url = format!("{base}/api/device-agent/desk-binding");

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| ApiError::Connection(redact(&token, e.to_string())))?;

    let status = response.status();
    if status == reqwest::StatusCode::OK {
        response
            .json::<DeskBinding>()
            .await
            .map_err(|e| ApiError::Connection(redact(&token, e.to_string())))
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        let _ = tokens.clear();
        Err(ApiError::Unauthorized)
    } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Err(ApiError::RateLimited {
            retry_after_secs: parse_retry_after(response.headers()),
        })
    } else {
        let error = response.text().await.unwrap_or_default();
        Err(ApiError::Rejected {
            status: status.as_u16(),
            error: redact(&token, error),
        })
    }
}

/// The drawer this counter has configured locally, or `None` (§5.2): `None` claims with no
/// `drawer` key.
pub fn local_drawer(config_dir: &Path) -> Option<DrawerInfo> {
    load_config(&config_dir.join("config.json"))
        .ok()
        .map(|cfg| cfg.drawer)
}

enum LocalTransport {
    Cups(CupsTransport),
    Tcp(TcpTransport9100),
    None,
}

impl LocalTransport {
    fn from_drawer(drawer: Option<&DrawerInfo>) -> Self {
        match drawer {
            Some(d) if d.transport.eq_ignore_ascii_case("CUPS") => {
                Self::Cups(CupsTransport::new(&d.target))
            }
            Some(d) => {
                let (host, port) = match d.target.split_once(':') {
                    Some((h, p)) => (h.to_owned(), p.parse::<u16>().unwrap_or(9100)),
                    None => (d.target.clone(), 9100),
                };
                Self::Tcp(TcpTransport9100::new(host, port))
            }
            None => Self::None,
        }
    }
}

impl DrawerTransport for LocalTransport {
    async fn send(&self, bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        match self {
            Self::Cups(t) => t.send(bytes).await,
            Self::Tcp(t) => t.send(bytes).await,
            Self::None => Err(TransportError::Failed(
                "no local drawer configured".to_owned(),
            )),
        }
    }
}

/// Runs `xc_device_agent::runner::run_forever` until `stop` fires. Called only after [`gate`]
/// answered [`Gate::Run`], so no build sends `drawer: None` to a server that does not take it.
pub async fn run_agent(
    server: &Url,
    tokens: &dyn TokenStore,
    drawer: Option<DrawerInfo>,
    journal: &Path,
    stop: watch::Receiver<bool>,
) -> Result<(), ApiError> {
    let (status, _) = fetch_info(server, tokens).await?;
    match gate(status) {
        Gate::Run => {
            let token = load_bearer_token(tokens)?;
            let api = HttpAgentApi::new(server.as_str(), token)?;
            let transport = LocalTransport::from_drawer(drawer.as_ref());
            let journal_path = if journal.is_dir() {
                journal.join("journal.jsonl")
            } else {
                journal.to_path_buf()
            };
            let journal_handle =
                Journal::open(journal_path).map_err(|e| ApiError::Connection(e.to_string()))?;
            let config = AgentConfig {
                version: VERSION.to_owned(),
                build: BUILD.to_owned(),
                os: current_os(),
                host: current_host(),
                drawer,
                wait_ms: 20_000,
            };
            let runner = AgentRunner::new(api, transport, journal_handle, config);
            run_forever(runner, stop).await;
            Ok(())
        }
        Gate::ServerTooOld => Err(ApiError::Rejected {
            status: 404,
            error: SERVER_TOO_OLD_MSG.to_owned(),
        }),
        Gate::RePair => {
            let _ = tokens.clear();
            Err(ApiError::Unauthorized)
        }
        Gate::Retry => Err(ApiError::Rejected {
            status,
            error: format!("retry after status {status}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::TokenError;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct TestStore(Mutex<Option<String>>);

    impl TokenStore for TestStore {
        fn load(&self) -> Result<Option<String>, TokenError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, token: &str) -> Result<(), TokenError> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }
        fn clear(&self) -> Result<(), TokenError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    async fn mock_server(
        status: u16,
        body: &'static str,
    ) -> (Url, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}", addr.port())).unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            req
        });
        (url, handle)
    }

    #[tokio::test]
    async fn fetch_and_counter_info_parses_200_and_sends_bearer() {
        let body = r#"{"agentId":"a1","counterName":"c1","branchId":"b1","capabilities":{"cashDrawer":true,"scanner":false,"kioskLock":true}}"#;
        let (url, handle) = mock_server(200, body).await;
        let store = TestStore::default();
        store.save("secret-tok").unwrap();

        let info = counter_info(&url, &store).await.unwrap();
        let req = handle.await.unwrap();
        assert!(req.contains("GET /api/device-agent/info"));
        assert!(req.contains("Bearer secret-tok"));
        assert_eq!(info.agent_id, "a1");
        assert_eq!(info.counter_name, "c1");
        assert_eq!(info.branch_id, "b1");
        assert!(info.capabilities.cash_drawer);
        assert!(!info.capabilities.scanner);
        assert!(info.capabilities.kiosk_lock);
        assert_eq!(info.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!info.build.is_empty());
    }

    #[tokio::test]
    async fn desk_binding_returns_nonce() {
        let body = r#"{"nonce":"n-123","expiresAt":"2026-11-06T05:00:00Z"}"#;
        let (url, handle) = mock_server(200, body).await;
        let store = TestStore::default();
        store.save("secret-tok").unwrap();

        let binding = desk_binding(&url, &store).await.unwrap();
        let req = handle.await.unwrap();
        assert!(req.contains("POST /api/device-agent/desk-binding"));
        assert!(req.contains("Bearer secret-tok"));
        assert_eq!(binding.nonce, "n-123");
        assert_eq!(binding.expires_at, "2026-11-06T05:00:00Z");
    }

    #[tokio::test]
    async fn run_agent_404_reports_thai_message_and_401_clears_token() {
        let (url_404, _) = mock_server(404, "{}").await;
        let store = TestStore::default();
        store.save("secret-tok").unwrap();
        let (_tx, rx) = watch::channel(false);
        let res_404 = run_agent(&url_404, &store, None, Path::new("/tmp"), rx.clone()).await;
        assert_eq!(
            res_404,
            Err(ApiError::Rejected {
                status: 404,
                error: "เซิร์ฟเวอร์ยังไม่รองรับเคาน์เตอร์".to_owned(),
            })
        );
        assert_eq!(store.load().unwrap(), Some("secret-tok".to_owned()));

        let (url_401, _) = mock_server(401, "{}").await;
        let res_401 = run_agent(&url_401, &store, None, Path::new("/tmp"), rx).await;
        assert_eq!(res_401, Err(ApiError::Unauthorized));
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn local_drawer_reads_config_when_present() {
        let dir = std::env::temp_dir().join(format!("xc-counter-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(local_drawer(&dir), None);

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"server":"https://xc-v3.spacing.cloud","drawer":{"transport":"CUPS","target":"Receipt_Printer"}}"#,
        )
        .unwrap();
        assert_eq!(
            local_drawer(&dir),
            Some(DrawerInfo {
                transport: "CUPS".to_owned(),
                target: "Receipt_Printer".to_owned(),
            })
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
