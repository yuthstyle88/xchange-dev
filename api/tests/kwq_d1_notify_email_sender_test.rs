//! KWQ-D1 — `NotifyEmailSender` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D21,
//! §4.1 "E-mail", §4.5 guard): Notify `POST /api/v1/notifications`, `channel: "email"`, against a
//! fake Notify on 127.0.0.1. The JSON field names are Notify's `CreateNotificationRequest`
//! (`recipient.address`, `subject`, `body`, `html_body`, `idempotency_key`); `201` ⇒ `Sent`,
//! `200` ⇒ `AlreadySent`; unset env, a 5xx or no server ⇒ `emailUnavailable`.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use xc_customer::domain::email_sender::{
    EMAIL_UNAVAILABLE, EmailMessage, EmailOutcome, EmailSender,
};
use xchange::infrastructure::notify::NotifyEmailSender;

fn message() -> EmailMessage {
    EmailMessage {
        to: "alice@example.com".to_owned(),
        subject: "Your booking".to_owned(),
        text: "plain body".to_owned(),
        html: "<p>html body</p>".to_owned(),
        idempotency_key: "copy:booking:BK-1:abcd".to_owned(),
    }
}

/// A fake Notify answering one request with `status`; returns (base url, the request text).
async fn fake_notify(status: u16) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = socket.read(&mut buf).await.expect("read");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..pos]).to_ascii_lowercase();
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                if data.len() >= pos + 4 + len {
                    break;
                }
            }
        }
        let reason = match status {
            200 => "OK",
            201 => "Created",
            _ => "Error",
        };
        let body = r#"{"id":"n-1"}"#;
        let resp = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(resp.as_bytes()).await.expect("write");
        String::from_utf8_lossy(&data).into_owned()
    });
    (base, handle)
}

fn body_of(request: &str) -> Value {
    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
    serde_json::from_str(body).unwrap_or_else(|_| panic!("a JSON body: {body}"))
}

#[test]
fn the_payload_carries_notifys_field_names_and_nothing_else() {
    let p = NotifyEmailSender::payload(&message());
    assert_eq!(
        p,
        json!({
            "channel": "email",
            "recipient": { "address": "alice@example.com" },
            "subject": "Your booking",
            "body": "plain body",
            "html_body": "<p>html body</p>",
            "idempotency_key": "copy:booking:BK-1:abcd",
        })
    );
    let keys: BTreeSet<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(
        !keys.contains("phone") && !keys.contains("text"),
        "not the SMS shape"
    );
}

#[tokio::test]
async fn created_is_sent_and_the_request_is_notifys_shape_with_the_key_header() {
    let (base, server) = fake_notify(201).await;
    let sender = NotifyEmailSender::new(Some(base), Some("secret-key".to_owned()));
    assert!(sender.is_configured());
    assert_eq!(
        sender.send(&message()).await.expect("sent"),
        EmailOutcome::Sent
    );
    let req = server.await.expect("the fake saw one request");
    assert!(req.starts_with("POST /api/v1/notifications "), "{req}");
    assert!(
        req.to_ascii_lowercase().contains("x-api-key: secret-key"),
        "the key header: {req}"
    );
    assert_eq!(body_of(&req), NotifyEmailSender::payload(&message()));
}

#[tokio::test]
async fn ok_is_already_sent() {
    let (base, server) = fake_notify(200).await;
    let sender = NotifyEmailSender::new(Some(format!("{base}/")), Some("k".to_owned()));
    assert_eq!(
        sender.send(&message()).await.expect("answered"),
        EmailOutcome::AlreadySent
    );
    let req = server.await.unwrap();
    assert!(
        req.starts_with("POST /api/v1/notifications "),
        "a trailing slash is trimmed: {req}"
    );
}

#[tokio::test]
async fn a_server_error_or_no_server_is_email_unavailable() {
    let (base, _server) = fake_notify(502).await;
    let sender = NotifyEmailSender::new(Some(base), Some("k".to_owned()));
    let err = sender.send(&message()).await.unwrap_err();
    assert!(err.to_string().contains(EMAIL_UNAVAILABLE), "{err}");

    // nothing listens on a closed port
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let sender = NotifyEmailSender::new(
        Some(format!("http://127.0.0.1:{port}")),
        Some("k".to_owned()),
    );
    let err = sender.send(&message()).await.unwrap_err();
    assert!(err.to_string().contains(EMAIL_UNAVAILABLE), "{err}");
}

#[tokio::test]
async fn unset_or_empty_settings_are_not_configured_and_send_nothing() {
    for (url, key) in [
        (None, None),
        (Some("http://127.0.0.1:9".to_owned()), None),
        (None, Some("k".to_owned())),
        (Some(String::new()), Some("k".to_owned())),
        (Some("http://127.0.0.1:9".to_owned()), Some(String::new())),
    ] {
        let sender = NotifyEmailSender::new(url.clone(), key.clone());
        assert!(!sender.is_configured(), "{url:?} {key:?}");
        let err = sender.send(&message()).await.unwrap_err();
        assert!(
            err.to_string().contains(EMAIL_UNAVAILABLE),
            "{url:?} {key:?}: {err}"
        );
    }
}
