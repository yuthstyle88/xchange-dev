//! Ticket CA-1a (`docs/design/xc-counter-app.md` §5.2, §8.2, §8.3): what a counter asks of the server under its agent
//! token. Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! | Call | Situation | Required result |
//! |---|---|---|
//! | `POST /api/device-agent/claim` | no `drawer` key, or `"drawer": null` | accepted (204 when nothing to do); `drawer_transport` and `drawer_target` are NULL afterwards (written as sent, §5.2), a stored drawer included |
//! | `POST /api/device-agent/claim` | a drawer | today's behaviour, stored as sent |
//! | `GET /api/device-agent/info` | every setting NULL | 200 `{agentId, counterName, branchId, capabilities:{cashDrawer:false, scanner:false, kioskLock:true}}` |
//! | `GET /api/device-agent/info` | `cash_drawer=true, scanner=true, kiosk_lock=false` | capabilities `true, true, false`; `counterName` = `device_agents.counter_name` |
//! | `info`, `desk-binding` | revoked or unknown token | 401 `AGENT_TOKEN_INVALID`, as `claim` |
//! | `POST /api/device-agent/desk-binding` | live token | 200 `{nonce, expiresAt}`; `DeskNonce::open(nonce, <the server's PII keys>, now)` gives this agent and branch with `exp = now + 60 s`; `expiresAt` is that instant in RFC 3339 |
//!
//! Through the server's whole route table with the test's clock. Agents are written straight into `device_agents`.
//! The server's PII keys are the process's installed ones (`xc_customer::get_pii_keys`, `bootstrap/mod.rs:436`), which
//! is what `v3/command.rs::current_pii_keys` reads.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::sync::Arc;

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Desk};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use snapshot_common::time;
use xchange::infrastructure::device_agents::desk_binding::DeskNonce;

const ORIGIN: &str = "http://desk.test";
const CLAIM: &str = "/api/device-agent/claim";
const INFO: &str = "/api/device-agent/info";
const DESK_BINDING: &str = "/api/device-agent/desk-binding";
const MAIN: &str = "MAIN";

const AGENT: &str = "a0000000-0000-4000-8000-000000000001";
const TOKEN: &str = "xcda_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const UNKNOWN_TOKEN: &str = "xcda_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const COUNTER_NAME: &str = "เคาน์เตอร์ 1";
const TOKEN_INVALID: &str = "AGENT_TOKEN_INVALID";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn t0() -> DateTime<Utc> {
    time("2026-11-06T03:00:00.000Z")
}

fn at(seconds: i64) -> DateTime<Utc> {
    t0() + Duration::seconds(seconds)
}

struct Room {
    desk: Desk,
    app: App,
}

impl Room {
    /// A branch's counter, paired with `TOKEN`, every setting not set.
    async fn open() -> Room {
        // One key per process, whichever test of the binary got here first: the server reads the same one.
        let keys =
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("the test PII key parses");
        let _ = xc_customer::install_pii_keys(Arc::new(keys));
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        desk.clock.set(t0());
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, paired_at, \
             counter_name) VALUES ($1::uuid, 'MAIN', 'Mac 1', $2, 'owner', $3, $2, $4)",
        )
        .bind(AGENT)
        .bind(t0())
        .bind(sha256_hex(TOKEN))
        .bind(COUNTER_NAME)
        .execute(&desk.db.pool)
        .await
        .expect("a paired counter");
        Room { desk, app }
    }

    async fn sql(&self, statement: &str) {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }

    async fn revoke(&self) {
        self.sql(&format!(
            "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = '{AGENT}'"
        ))
        .await;
    }

    /// (`drawer_transport`, `drawer_target`) as stored.
    async fn drawer(&self) -> (Option<String>, Option<String>) {
        sqlx::query_as(
            "SELECT drawer_transport, drawer_target FROM device_agents WHERE id = $1::uuid",
        )
        .bind(AGENT)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("the counter's row")
    }

    async fn send_at(&self, t: DateTime<Utc>, call: Call) -> Answer {
        self.desk.clock.set(t);
        self.app.send(call).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// A call as a program sends it: no `Origin`, no marker header, no cookie.
fn program(path: &str, body: Value, token: &str) -> Call {
    Call::post(ORIGIN, path, body.to_string())
        .without("origin")
        .without("x-xchange-request")
        .header("x-real-ip", "10.1.1.1")
        .header("authorization", &format!("Bearer {token}"))
}

fn get_info(token: &str) -> Call {
    let mut call = program(INFO, json!({}), token);
    call.method = Method::GET;
    call.body = Vec::new();
    call
}

/// A claim's body with `drawer` as given; `None` leaves the key out.
fn claim_body(drawer: Option<Value>) -> Value {
    let mut body = json!({ "waitMs": 0, "version": "1.0.0", "build": "abc1234", "os": "macos-14.6-arm64",
                           "host": "counter-1.local" });
    if let Some(drawer) = drawer {
        body["drawer"] = drawer;
    }
    body
}

fn a_drawer() -> Value {
    json!({ "transport": "TCP", "target": "192.168.1.50:9100" })
}

fn the_drawer_stored() -> (Option<String>, Option<String>) {
    (Some("TCP".to_owned()), Some("192.168.1.50:9100".to_owned()))
}

fn error_of(answer: &Answer) -> String {
    answer.json()["error"].as_str().unwrap_or("").to_owned()
}

// ── claim: a counter with no drawer configured locally ──────────────────────

/// §5.2: `ClaimRequest.drawer` is optional. A counter that sends no `drawer` key claims like any other.
#[actix_web::test]
async fn a_claim_with_no_drawer_key_is_accepted_and_stores_no_drawer() {
    let room = Room::open().await;
    let answer = room
        .send_at(at(1), program(CLAIM, claim_body(None), TOKEN))
        .await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    assert_eq!(room.drawer().await, (None, None));
    room.finish().await;
}

/// §8.2: `"drawer": null` means the same as no key.
#[actix_web::test]
async fn a_claim_with_a_null_drawer_is_accepted_and_stores_no_drawer() {
    let room = Room::open().await;
    let answer = room
        .send_at(at(1), program(CLAIM, claim_body(Some(Value::Null)), TOKEN))
        .await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    assert_eq!(room.drawer().await, (None, None));
    room.finish().await;
}

/// §5.2: written as sent, NULL included (today's `COALESCE` keeps the old value). A counter whose drawer was taken
/// away no longer reports one.
#[actix_web::test]
async fn a_claim_with_no_drawer_clears_the_drawer_stored_before() {
    let room = Room::open().await;
    let first = room
        .send_at(at(1), program(CLAIM, claim_body(Some(a_drawer())), TOKEN))
        .await;
    assert_eq!(first.status, 204, "{}", first.text);
    assert_eq!(room.drawer().await, the_drawer_stored());

    let second = room
        .send_at(at(2), program(CLAIM, claim_body(None), TOKEN))
        .await;
    assert_eq!(second.status, 204, "{}", second.text);
    assert_eq!(room.drawer().await, (None, None), "written as sent");
    room.finish().await;
}

/// Today's behaviour, which today's agent relies on (§5.2 "its behaviour is unchanged").
#[actix_web::test]
async fn a_claim_with_a_drawer_stores_it_as_sent() {
    let room = Room::open().await;
    let answer = room
        .send_at(at(1), program(CLAIM, claim_body(Some(a_drawer())), TOKEN))
        .await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    assert_eq!(room.drawer().await, the_drawer_stored());
    room.finish().await;
}

// ── info ────────────────────────────────────────────────────────────────────

/// §5.2, §8.2: not set resolves to its safe value: drawer off, scanner off, kiosk lock on.
#[actix_web::test]
async fn info_with_every_setting_not_set_answers_the_safe_values() {
    let room = Room::open().await;
    let answer = room.send_at(at(1), get_info(TOKEN)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json(),
        json!({
            "agentId": AGENT,
            "counterName": COUNTER_NAME,
            "branchId": MAIN,
            "capabilities": { "cashDrawer": false, "scanner": false, "kioskLock": true },
        })
    );
    room.finish().await;
}

/// §8.2: the owner's values, each resolved on its own; a name change shows on the next read, with no reinstall (§4).
#[actix_web::test]
async fn info_answers_the_owners_settings_and_the_counters_name() {
    let room = Room::open().await;
    room.sql(&format!(
        "UPDATE device_agents SET cash_drawer = true, scanner = true, kiosk_lock = false, \
         counter_name = 'เคาน์เตอร์ 9' WHERE id = '{AGENT}'"
    ))
    .await;
    let answer = room.send_at(at(1), get_info(TOKEN)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json(),
        json!({
            "agentId": AGENT,
            "counterName": "เคาน์เตอร์ 9",
            "branchId": MAIN,
            "capabilities": { "cashDrawer": true, "scanner": true, "kioskLock": false },
        })
    );
    room.finish().await;
}

/// A setting saved `false` is the owner's `false`, not "not set": kiosk lock off stays off, and a drawer and a scanner
/// saved off answer off.
#[actix_web::test]
async fn info_keeps_false_apart_from_not_set() {
    let room = Room::open().await;
    room.sql(&format!(
        "UPDATE device_agents SET cash_drawer = false, scanner = false, kiosk_lock = true WHERE id = '{AGENT}'"
    ))
    .await;
    let answer = room.send_at(at(1), get_info(TOKEN)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        answer.json()["capabilities"],
        json!({ "cashDrawer": false, "scanner": false, "kioskLock": true })
    );
    room.finish().await;
}

// ── a token that is no good ─────────────────────────────────────────────────

/// §8.2: a revoked or unknown token answers as `claim` does.
#[actix_web::test]
async fn info_and_desk_binding_refuse_an_unknown_token_as_claim_does() {
    let room = Room::open().await;
    let claim = room
        .send_at(
            at(1),
            program(CLAIM, claim_body(Some(a_drawer())), UNKNOWN_TOKEN),
        )
        .await;
    assert_eq!(
        (claim.status, error_of(&claim).as_str()),
        (401, TOKEN_INVALID),
        "claim, the rule the others follow: {}",
        claim.text
    );
    let info = room.send_at(at(2), get_info(UNKNOWN_TOKEN)).await;
    assert_eq!(
        (info.status, error_of(&info).as_str()),
        (401, TOKEN_INVALID),
        "info: {}",
        info.text
    );
    let binding = room
        .send_at(at(3), program(DESK_BINDING, json!({}), UNKNOWN_TOKEN))
        .await;
    assert_eq!(
        (binding.status, error_of(&binding).as_str()),
        (401, TOKEN_INVALID),
        "desk-binding: {}",
        binding.text
    );
    room.finish().await;
}

#[actix_web::test]
async fn info_and_desk_binding_refuse_a_revoked_token_as_claim_does() {
    let room = Room::open().await;
    room.revoke().await;
    let info = room.send_at(at(1), get_info(TOKEN)).await;
    assert_eq!(
        (info.status, error_of(&info).as_str()),
        (401, TOKEN_INVALID),
        "info: {}",
        info.text
    );
    let binding = room
        .send_at(at(2), program(DESK_BINDING, json!({}), TOKEN))
        .await;
    assert_eq!(
        (binding.status, error_of(&binding).as_str()),
        (401, TOKEN_INVALID),
        "desk-binding: {}",
        binding.text
    );
    room.finish().await;
}

/// No bearer token at all is the same refusal.
#[actix_web::test]
async fn info_and_desk_binding_refuse_no_token() {
    let room = Room::open().await;
    let mut info = get_info(TOKEN);
    info.headers.remove("authorization");
    let answer = room.send_at(at(1), info).await;
    assert_eq!(answer.status, 401, "info: {}", answer.text);

    let binding = program(DESK_BINDING, json!({}), TOKEN).without("authorization");
    let answer = room.send_at(at(2), binding).await;
    assert_eq!(answer.status, 401, "desk-binding: {}", answer.text);
    room.finish().await;
}

// ── desk-binding ────────────────────────────────────────────────────────────

/// §5.4, §8.3: a nonce the server itself can open, for this counter and its branch, good for 60 s from the moment it
/// was asked for (the route's clock), and `expiresAt` says that instant.
#[actix_web::test]
async fn desk_binding_answers_a_sealed_nonce_good_for_60_seconds() {
    let room = Room::open().await;
    let asked_at = at(5);
    let answer = room
        .send_at(asked_at, program(DESK_BINDING, json!({}), TOKEN))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let body = answer.json();
    let nonce = body["nonce"].as_str().expect("a nonce");

    let keys = xc_customer::get_pii_keys().expect("the server's PII keys");
    let opened = DeskNonce::open(nonce, &keys, asked_at).expect("the server's own nonce opens");
    let exp = asked_at + Duration::seconds(60);
    assert_eq!(
        opened,
        DeskNonce {
            agent_id: AGENT.to_owned(),
            branch_id: MAIN.to_owned(),
            exp: exp.timestamp(),
        }
    );

    let expires_at = body["expiresAt"].as_str().expect("expiresAt");
    assert_eq!(
        DateTime::parse_from_rfc3339(expires_at)
            .unwrap_or_else(|e| panic!("{expires_at} is RFC 3339: {e}"))
            .with_timezone(&Utc),
        exp,
        "expiresAt is the nonce's exp"
    );
    room.finish().await;
}

/// §8.3: the 60 s are real. At `exp` the nonce is refused `Expired`, one second before it still opens.
#[actix_web::test]
async fn the_nonce_desk_binding_answers_stops_opening_at_its_expiry() {
    let room = Room::open().await;
    let asked_at = at(5);
    let answer = room
        .send_at(asked_at, program(DESK_BINDING, json!({}), TOKEN))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let body = answer.json();
    let nonce = body["nonce"].as_str().expect("a nonce");
    let keys = xc_customer::get_pii_keys().expect("the server's PII keys");
    assert!(
        DeskNonce::open(nonce, &keys, asked_at + Duration::seconds(59)).is_ok(),
        "still good one second before exp"
    );
    assert_eq!(
        DeskNonce::open(nonce, &keys, asked_at + Duration::seconds(60))
            .expect_err("now >= exp")
            .code(),
        "NONCE_EXPIRED"
    );
    room.finish().await;
}
