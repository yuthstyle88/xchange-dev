//! Up-front tests for C4's `IdentityClient::change_password` (BE controller ruling, xchange #564
//! issuecomment-6245742828 (c); design `docs/design/blueprint/user-management-rbac.md`, PR 3083).
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The contract is Identity's own (`108-Plaza/108-platform-services` `d19249f`, `crates/auth/src/api/mod.rs:130-133`,
//! `contract/mod.rs:657-665`, `api/password_reset.rs:719-752`):
//! `POST /auth/password/change`, `Authorization: Bearer <access token>`, body `{ "currentPassword", "newPassword" }`,
//! 200 `{ "changed": true }`; refusals `401 invalid_credentials` and `422 password_policy_violation`.
//!
//! Identity is faked over real HTTP on a local port: nothing here reaches a real Identity.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use xc_auth::domain::identity::identity_client::{IdentityClient, IdentityError};
use xc_auth::infrastructure::identity::HttpIdentityClient;
use xc_auth::infrastructure::identity::identity_settings::IdentitySettings;

const PATH: &str = "/auth/password/change";
const TOKEN: &str = "access-token-of-the-signed-in-person";

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

/// A one-answer Identity: every request gets `(status, body)`; what it received is kept.
struct Fake {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    fn answering(status: u16, body: &str) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let body = body.to_owned();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_owned();
                let path = parts.next().unwrap_or("").to_owned();
                let mut headers = HashMap::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                        break;
                    }
                    if let Some((k, v)) = h.trim_end().split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
                    }
                }
                let len: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut buf = vec![0u8; len];
                let _ = reader.read_exact(&mut buf);
                seen2.lock().unwrap().push(Seen {
                    method,
                    path,
                    headers,
                    body: serde_json::from_slice(&buf).unwrap_or(Value::Null),
                });
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        Fake { url, seen }
    }

    fn client(&self) -> HttpIdentityClient {
        HttpIdentityClient::new(
            IdentitySettings {
                base_url: self.url.clone(),
                client_id: "client-id-1".into(),
                client_secret: "client-secret-1".into(),
                audience: "xchange".into(),
                issuer: "auth-service".into(),
                api_key: Some("api-key-1".into()),
            },
            "x-forwarded-for",
        )
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn change(fake: &Fake) -> Result<(), IdentityError> {
    fake.client()
        .change_password(TOKEN, "old-pass-01", "new-pass-0123", Some("203.0.113.7"))
        .await
}

/// P1: Identity's 200 is `Ok(())`. The call is `POST /auth/password/change` with the person's Bearer token, the
/// camelCase body, the client headers every call carries, and the caller's address.
#[tokio::test]
async fn p1_success_posts_the_bearer_and_both_passwords() {
    let fake = Fake::answering(200, &json!({ "changed": true }).to_string());
    assert_eq!(change(&fake).await, Ok(()));
    let seen = fake.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let s = &seen[0];
    assert_eq!((s.method.as_str(), s.path.as_str()), ("POST", PATH));
    assert_eq!(
        s.headers.get("authorization").map(String::as_str),
        Some(&*format!("Bearer {TOKEN}"))
    );
    assert_eq!(
        s.body,
        json!({ "currentPassword": "old-pass-01", "newPassword": "new-pass-0123" })
    );
    assert_eq!(
        s.headers.get("x-client-id").map(String::as_str),
        Some("client-id-1")
    );
    assert_eq!(
        s.headers.get("x-forwarded-for").map(String::as_str),
        Some("203.0.113.7")
    );
}

/// P2: a wrong current password (or a token Identity refuses) is `InvalidCredentials`.
#[tokio::test]
async fn p2_invalid_credentials_is_invalid_credentials() {
    let fake = Fake::answering(401, &json!({ "error": "invalid_credentials" }).to_string());
    assert_eq!(change(&fake).await, Err(IdentityError::InvalidCredentials));
}

/// P3: a new password Identity's policy refuses is `WeakPassword`.
#[tokio::test]
async fn p3_policy_violation_is_weak_password() {
    let fake = Fake::answering(
        422,
        &json!({ "error": "password_policy_violation" }).to_string(),
    );
    assert_eq!(change(&fake).await, Err(IdentityError::WeakPassword));
}

/// P4: a 5xx, or a 200 that is not JSON, is `Unavailable` on this path, never `Ok`.
#[tokio::test]
async fn p4_server_error_or_unreadable_success_is_unavailable() {
    for (status, body) in [
        (503, json!({ "error": "down" }).to_string()),
        (200, "not json".to_owned()),
    ] {
        let fake = Fake::answering(status, &body);
        match change(&fake).await {
            Err(IdentityError::Unavailable { path, .. }) => assert_eq!(path, PATH, "{status}"),
            other => panic!("{status} {body}: expected Unavailable, got {other:?}"),
        }
    }
}
