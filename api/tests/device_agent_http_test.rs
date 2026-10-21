//! Row AG-2 (`docs/design/xchange-v3.md` §5.3, §5.4c *Identity*, *Security*, *Claim* steps
//! 1–2): pairing, the bearer token and the rules that keep browsers out of
//! `POST /api/device-agent/{pair,claim,report}`. Written before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Through the server's whole route table with the test's clock. Agents are written straight
//! into `device_agents` (row AG-1's table): a pairing code is stored as the SHA-256 (lower-case
//! hex) of its 10 characters, upper case, without the dash (`AgentToken::code_hash`, AG-1); a
//! token as the SHA-256 of its whole text. Nothing here depends on row AG-4. This file
//! compiles on `main` and fails there on results.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

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
const PAIR: &str = "/api/device-agent/pair";
const CLAIM: &str = "/api/device-agent/claim";
const REPORT: &str = "/api/device-agent/report";
const MAIN: &str = "MAIN";

const AGENT: &str = "a0000000-0000-4000-8000-000000000001";
const AGENT_NAME: &str = "Mac เคาน์เตอร์ 1";
/// The code as the owner's page shows it, and its canonical form.
const CODE: &str = "7K3QD-9XW2M";
const CODE_CANONICAL: &str = "7K3QD9XW2M";
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

fn at(seconds: i64) -> DateTime<Utc> {
    t0() + Duration::seconds(seconds)
}

struct Room {
    desk: Desk,
    app: App,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        desk.clock.set(t0());
        Room { desk, app }
    }

    /// An agent of MAIN added by the owner at t0 with the code `CODE`, valid 10 minutes.
    async fn pending_agent(&self) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, \
             pair_code_hash, pair_code_expires_at) VALUES ($1::uuid, 'MAIN', $2, $3, 'owner', $4, $5)",
        )
        .bind(AGENT)
        .bind(AGENT_NAME)
        .bind(t0())
        .bind(sha256_hex(CODE_CANONICAL))
        .bind(at(600))
        .execute(&self.desk.db.pool)
        .await
        .expect("an agent added by the owner (row AG-1's table)");
    }

    /// An agent of MAIN already paired with `TOKEN`.
    async fn paired_agent(&self) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, \
             paired_at) VALUES ($1::uuid, 'MAIN', $2, $3, 'owner', $4, $3)",
        )
        .bind(AGENT)
        .bind(AGENT_NAME)
        .bind(t0())
        .bind(sha256_hex(TOKEN))
        .execute(&self.desk.db.pool)
        .await
        .expect("a paired agent (row AG-1's table)");
    }

    async fn sql(&self, statement: &str) {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }

    async fn agent_text(&self, column: &str) -> Option<String> {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {column}::text FROM device_agents WHERE id = $1::uuid"
        )))
        .bind(AGENT)
        .fetch_one(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("read {column}: {e}"))
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
fn program(path: &str, body: Value, address: &str) -> Call {
    Call::post(ORIGIN, path, body.to_string())
        .without("origin")
        .without("x-xchange-request")
        .header("x-real-ip", address)
}

fn with_token(call: Call, token: &str) -> Call {
    call.header("authorization", &format!("Bearer {token}"))
}

fn pair_body(code: &str) -> Value {
    json!({ "code": code, "host": "counter-1.local", "os": "macos-14.6-arm64",
            "version": "1.0.0", "build": "abc1234" })
}

fn claim_body() -> Value {
    json!({ "waitMs": 0, "version": "1.0.0", "build": "abc1234", "os": "macos-14.6-arm64",
            "host": "counter-1.local", "drawer": { "transport": "TCP", "target": "192.168.1.50:9100" } })
}

fn error_of(answer: &Answer) -> String {
    answer.json()["error"].as_str().unwrap_or("").to_owned()
}

// ── pairing ──────────────────────────────────────────────────────────────────

/// A live code answers the token once; only its hash is stored and the code is used up.
#[actix_web::test]
async fn pairing_with_a_live_code_answers_the_token_once() {
    let room = Room::open().await;
    room.pending_agent().await;
    let answer = room
        .send_at(at(60), program(PAIR, pair_body(CODE), "10.1.1.1"))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let body = answer.json();
    assert_eq!(body["agentId"], AGENT);
    assert_eq!(body["branchId"], MAIN);
    assert_eq!(body["agentName"], AGENT_NAME);
    let token = body["token"].as_str().expect("a token").to_owned();
    let hex = token.strip_prefix("xcda_").expect("xcda_ prefix");
    assert!(
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "{token}"
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    assert_eq!(
        room.agent_text("token_hash").await,
        Some(sha256_hex(&token))
    );
    assert_eq!(
        room.agent_text("pair_code_hash").await,
        None,
        "the code is used up"
    );
    assert!(room.agent_text("paired_at").await.is_some());
    assert_eq!(
        room.agent_text("agent_host").await.as_deref(),
        Some("counter-1.local")
    );

    let again = room
        .send_at(at(61), program(PAIR, pair_body(CODE), "10.1.1.1"))
        .await;
    assert_eq!(
        (again.status, error_of(&again).as_str()),
        (400, "PAIRING_CODE_INVALID"),
        "single use"
    );

    let claim = room
        .send_at(
            at(62),
            with_token(program(CLAIM, claim_body(), "10.1.1.1"), &token),
        )
        .await;
    assert_eq!(claim.status, 204, "the new token claims: {}", claim.text);
    room.finish().await;
}

/// A code is typed without its dash or in lower case; unknown, expired and used codes all get
/// the same answer.
#[actix_web::test]
async fn codes_are_read_loosely_and_refused_with_one_answer() {
    let room = Room::open().await;
    room.pending_agent().await;
    for (code, t) in [("ZZZZZ-ZZZZZ", 1), ("not a code", 2), (CODE, 600)] {
        let answer = room
            .send_at(at(t), program(PAIR, pair_body(code), "10.1.1.2"))
            .await;
        assert_eq!(
            (answer.status, answer.text.as_str()),
            (400, "{\"error\":\"PAIRING_CODE_INVALID\"}"),
            "{code} at {t} s (the code expires at 600 s)"
        );
    }
    let answer = room
        .send_at(at(10), program(PAIR, pair_body("7k3qd9xw2m"), "10.1.1.2"))
        .await;
    assert_eq!(answer.status, 200, "lower case, no dash: {}", answer.text);
    room.finish().await;
}

/// At most 10 pairing attempts per address in 15 minutes, right or wrong; another address is
/// counted on its own.
#[actix_web::test]
async fn pairing_attempts_are_limited_per_address() {
    let room = Room::open().await;
    room.pending_agent().await;
    for i in 0..10 {
        let answer = room
            .send_at(at(1), program(PAIR, pair_body("ZZZZZ-ZZZZZ"), "10.9.9.9"))
            .await;
        assert_eq!(answer.status, 400, "attempt {i}: {}", answer.text);
    }
    let answer = room
        .send_at(at(2), program(PAIR, pair_body(CODE), "10.9.9.9"))
        .await;
    assert_eq!(
        (answer.status, error_of(&answer).as_str()),
        (429, "RATE_LIMITED"),
        "the 11th, even with the right code"
    );
    let answer = room
        .send_at(at(3), program(PAIR, pair_body(CODE), "10.9.9.8"))
        .await;
    assert_eq!(answer.status, 200, "another address: {}", answer.text);
    room.finish().await;
}

/// A new code for the same agent re-pairs it: the old token stops working.
#[actix_web::test]
async fn a_re_pair_stops_the_old_token() {
    let room = Room::open().await;
    room.paired_agent().await;
    room.sql(&format!(
        "UPDATE device_agents SET pair_code_hash = '{}', pair_code_expires_at = '{}' WHERE id = '{AGENT}'",
        sha256_hex(CODE_CANONICAL),
        at(600).to_rfc3339()
    ))
    .await;
    let answer = room
        .send_at(at(10), program(PAIR, pair_body(CODE), "10.1.1.3"))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let old = room
        .send_at(
            at(11),
            with_token(program(CLAIM, claim_body(), "10.1.1.3"), TOKEN),
        )
        .await;
    assert_eq!(old.status, 401, "{}", old.text);
    room.finish().await;
}

// ── the token ────────────────────────────────────────────────────────────────

/// No token, a wrong one, or a revoked agent's: 401 on both routes that need it.
#[actix_web::test]
async fn a_missing_wrong_or_revoked_token_is_401() {
    let room = Room::open().await;
    room.paired_agent().await;
    let wrong = "xcda_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    for path in [CLAIM, REPORT] {
        let body = if path == CLAIM {
            claim_body()
        } else {
            json!({ "jobId": "x", "leaseId": "y", "outcome": "FAILED", "reason": "", "detail": {} })
        };
        let none = room
            .send_at(at(1), program(path, body.clone(), "10.2.2.2"))
            .await;
        assert_eq!(none.status, 401, "{path} without a token: {}", none.text);
        let bad = room
            .send_at(
                at(1),
                with_token(program(path, body.clone(), "10.2.2.2"), wrong),
            )
            .await;
        assert_eq!(bad.status, 401, "{path} with a wrong token: {}", bad.text);
        let basic = room
            .send_at(
                at(1),
                program(path, body, "10.2.2.2").header("authorization", &format!("Basic {TOKEN}")),
            )
            .await;
        assert_eq!(basic.status, 401, "{path}: bearer only");
    }
    let ok = room
        .send_at(
            at(2),
            with_token(program(CLAIM, claim_body(), "10.2.2.3"), TOKEN),
        )
        .await;
    assert_eq!(ok.status, 204, "{}", ok.text);
    room.sql(&format!(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = '{AGENT}'"
    ))
    .await;
    let revoked = room
        .send_at(
            at(3),
            with_token(program(CLAIM, claim_body(), "10.2.2.3"), TOKEN),
        )
        .await;
    assert_eq!(revoked.status, 401, "{}", revoked.text);
    room.finish().await;
}

/// Failed token checks are limited per address: 30 in 15 minutes, then 429.
#[actix_web::test]
async fn failed_token_checks_are_limited_per_address() {
    let room = Room::open().await;
    let wrong = "xcda_eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    for i in 0..30 {
        let answer = room
            .send_at(
                at(1),
                with_token(program(CLAIM, claim_body(), "10.3.3.3"), wrong),
            )
            .await;
        assert_eq!(answer.status, 401, "check {i}");
    }
    let answer = room
        .send_at(
            at(1),
            with_token(program(CLAIM, claim_body(), "10.3.3.3"), wrong),
        )
        .await;
    assert_eq!(
        (answer.status, error_of(&answer).as_str()),
        (429, "RATE_LIMITED")
    );
    room.finish().await;
}

// ── browsers stay out; programs need no same-origin marker ──────────────────

/// A request carrying `Origin` or `Cookie` is refused 400 `NOT_FOR_BROWSERS` on all three
/// routes, before anything else is looked at; the routes send no CORS header.
#[actix_web::test]
async fn a_request_from_a_browser_is_refused() {
    let room = Room::open().await;
    room.paired_agent().await;
    for path in [PAIR, CLAIM, REPORT] {
        for (name, value) in [("origin", ORIGIN), ("cookie", "__Host-xc_owner=abc")] {
            let call =
                with_token(program(path, claim_body(), "10.4.4.4"), TOKEN).header(name, value);
            let answer = room.send_at(at(1), call).await;
            assert_eq!(
                (answer.status, answer.text.as_str()),
                (400, "{\"error\":\"NOT_FOR_BROWSERS\"}"),
                "{path} with {name}"
            );
            assert_eq!(answer.header("access-control-allow-origin"), None);
        }
    }
    room.finish().await;
}

/// The routes are called by a program: no `X-Xchange-Request` marker is needed, and the
/// same-origin rule does not refuse them.
#[actix_web::test]
async fn the_agent_routes_need_no_marker_header() {
    let room = Room::open().await;
    room.paired_agent().await;
    let answer = room
        .send_at(
            at(1),
            with_token(program(CLAIM, claim_body(), "10.5.5.5"), TOKEN),
        )
        .await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    room.finish().await;
}

/// The token gives the branch; a claim naming another one is refused 403
/// `AGENT_BRANCH_MISMATCH`, with a line on the token's branch.
#[actix_web::test]
async fn a_claim_naming_another_branch_is_refused_with_a_line() {
    let room = Room::open().await;
    room.paired_agent().await;
    let mut body = claim_body();
    body["branchId"] = json!("B9");
    let answer = room
        .send_at(at(1), with_token(program(CLAIM, body, "10.6.6.6"), TOKEN))
        .await;
    assert_eq!(
        (answer.status, error_of(&answer).as_str()),
        (403, "AGENT_BRANCH_MISMATCH")
    );
    let lines: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM device_events WHERE branch_id = 'MAIN' AND actor = $1",
    )
    .bind(format!("agent:{AGENT}"))
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("count lines");
    assert_eq!(lines, 1, "one line on the token's branch");
    let mut same = claim_body();
    same["branchId"] = json!(MAIN);
    let answer = room
        .send_at(at(2), with_token(program(CLAIM, same, "10.6.6.6"), TOKEN))
        .await;
    assert_eq!(
        answer.status, 204,
        "naming its own branch is fine: {}",
        answer.text
    );
    room.finish().await;
}

/// At most 60 claims per agent per minute; the 61st is 429 `RATE_LIMITED` with `Retry-After`.
#[actix_web::test]
async fn claims_are_limited_to_sixty_a_minute() {
    let room = Room::open().await;
    room.paired_agent().await;
    for i in 0..60 {
        let answer = room
            .send_at(
                at(i / 2),
                with_token(program(CLAIM, claim_body(), "10.7.7.7"), TOKEN),
            )
            .await;
        assert_eq!(answer.status, 204, "claim {i}: {}", answer.text);
    }
    let answer = room
        .send_at(
            at(40),
            with_token(program(CLAIM, claim_body(), "10.7.7.7"), TOKEN),
        )
        .await;
    assert_eq!(
        (answer.status, error_of(&answer).as_str()),
        (429, "RATE_LIMITED")
    );
    let retry: u64 = answer
        .header("retry-after")
        .expect("Retry-After")
        .parse()
        .expect("whole seconds");
    assert_eq!(retry, 20, "the window opened at 0 s and lasts 60 s");
    let answer = room
        .send_at(
            at(60),
            with_token(program(CLAIM, claim_body(), "10.7.7.7"), TOKEN),
        )
        .await;
    assert_eq!(answer.status, 204, "a new minute: {}", answer.text);
    room.finish().await;
}
