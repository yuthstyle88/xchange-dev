//! R-device-claim up-front tests (#2054 row 9, design `docs/design/blueprint/bounded-inputs.md` §4 R-* row,
//! "`device_agent.rs` claim `transport` (non-v3: … with its own refusal)") — written before the implementation by
//! claude/studio-claude-2 on the order of the xchange BE lane controller claude/56a74fcd. Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! `POST /api/device-agent/claim` stores `drawer.transport` in `device_agents.drawer_transport`, which is
//! `CHECK (drawer_transport IN ('CUPS', 'TCP'))` (`migrations/20261008120000_device_agents.sql:24`). Today any other
//! string reaches that CHECK and the claim answers 500. It must answer the route's own refusal for a bad body,
//! `400 {"error":"BAD_REQUEST"}` with `cache-control: no-store` (`http/device_agent.rs:69-73`), and write nothing.
//!
//! The harness is `agent_claim_test.rs`'s. Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Desk};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const CLAIM: &str = "/api/device-agent/claim";
const MAIN: &str = "MAIN";
const AGENT: &str = "a0000000-0000-4000-8000-000000000001";
const TOKEN: &str = "xcda_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn t0() -> DateTime<Utc> {
    time("2026-10-08T03:00:00.000Z")
}

struct Shop {
    desk: Desk,
    app: App,
}

impl Shop {
    /// MAIN with a paired agent whose last claim said CUPS / `POS80_`.
    async fn open() -> Shop {
        let desk = Desk::open().await;
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, \
             paired_at, last_seen_at, agent_version, agent_build, agent_os, agent_host, \
             drawer_transport, drawer_target) VALUES ($1::uuid, $2, 'Mac 1', $3, 'owner', $4, $3, $3, \
             '1.0.0', 'abc1234', 'macos-14.6-arm64', 'counter-1.local', 'CUPS', 'POS80_')",
        )
        .bind(AGENT)
        .bind(MAIN)
        .bind(t0() - Duration::hours(1))
        .bind(sha256_hex(TOKEN))
        .execute(&desk.db.pool)
        .await
        .expect("an agent of MAIN");
        let app = rpc_app!(desk);
        Shop { desk, app }
    }

    async fn claim(&self, transport: &str) -> Answer {
        self.desk.clock.set(t0());
        let body = json!({ "waitMs": 0, "version": "1.0.1", "build": "def5678", "os": "macos-14.6-arm64",
                           "host": "counter-1.local", "drawer": { "transport": transport, "target": "10.0.0.5:9100" } });
        self.app
            .send(
                Call::post(ORIGIN, CLAIM, body.to_string())
                    .without("origin")
                    .without("x-xchange-request")
                    .header("authorization", &format!("Bearer {TOKEN}"))
                    .header("x-real-ip", "10.1.1.1"),
            )
            .await
    }

    /// `(drawer_transport, drawer_target, agent_version, claim_count)` of the agent.
    async fn agent(&self) -> (Option<String>, Option<String>, Option<String>, i32) {
        sqlx::query_as(
            "SELECT drawer_transport, drawer_target, agent_version, claim_count FROM device_agents \
             WHERE id = $1::uuid",
        )
        .bind(AGENT)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("read the agent")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn is_bad_request(a: &Answer, what: &str) {
    assert_eq!(a.status, 400, "{what}: {}", a.text);
    let body: Value = a.json();
    assert_eq!(
        body,
        json!({ "error": "BAD_REQUEST" }),
        "{what}: the route's own refusal"
    );
    assert_eq!(a.header("cache-control"), Some("no-store"), "{what}");
}

/// Both allowed values are taken and stored as sent (keep).
#[actix_web::test]
async fn cups_and_tcp_are_accepted_and_stored() {
    let shop = Shop::open().await;
    let a = shop.claim("TCP").await;
    assert_eq!(a.status, 204, "TCP: {}", a.text);
    assert_eq!(shop.agent().await.0.as_deref(), Some("TCP"));
    let a = shop.claim("CUPS").await;
    assert_eq!(a.status, 204, "CUPS: {}", a.text);
    assert_eq!(shop.agent().await.0.as_deref(), Some("CUPS"));
    shop.finish().await;
}

/// Any other transport is the route's 400 `BAD_REQUEST` (today: 500 at the CHECK).
#[actix_web::test]
async fn an_unknown_transport_is_bad_request_not_a_server_error() {
    let shop = Shop::open().await;
    for t in ["USB", "cups", "tcp", "", " TCP", "CUPS ", "SERIAL"] {
        let a = shop.claim(t).await;
        is_bad_request(&a, &format!("{t:?}"));
    }
    shop.finish().await;
}

/// A refused claim writes nothing: the agent keeps its last transport, target, version and claim count.
#[actix_web::test]
async fn a_refused_transport_writes_nothing() {
    let shop = Shop::open().await;
    let before = shop.agent().await;
    let a = shop.claim("USB").await;
    is_bad_request(&a, "USB");
    assert_eq!(shop.agent().await, before, "the agent row is untouched");
    shop.finish().await;
}
