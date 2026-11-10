//! xchange#6569 (`docs/design/xc-counter-app.md` §8.8): leaving the kiosk lock with a one-time code a manager issues
//! at the desk. Written before the implementation (guarded: the lock keeps staff at a cash drawer). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! | Call | Situation | Required result |
//! |---|---|---|
//! | `counterUnlockIssue` | branch manager or Owner, live counter of the session's branch | 200 `{code, expiresAt}`: `code` is `XXXXX-XXXXX`, `expiresAt` 60 s on; one `counter_unlock_tokens` row with `issued_by` = the caller, the code not stored as typed |
//! | `counterUnlockIssue` | a teller | refused `roleNotAllowedHere` (the policy row's refusal), no row (`Permission::CounterUnlock`) |
//! | `counterUnlockIssue` | a counter of another branch | `COUNTER_OTHER_BRANCH`, no row |
//! | `counterUnlockIssue` | a revoked or unknown counter | `COUNTER_REVOKED`, no row |
//! | `kiosk-exit` | the counter the code was issued for, within 60 s | 204; `redeemed_at` set |
//! | `kiosk-exit` | the same code again | 403 `UNLOCK_REFUSED` (one use) |
//! | `kiosk-exit` | 60 s or more after issue | 403 `UNLOCK_REFUSED` |
//! | `kiosk-exit` | another counter's token | 403 `UNLOCK_REFUSED`; the right counter can still use it |
//! | `kiosk-exit` | a code never issued, or not a code | 403 `UNLOCK_REFUSED` |
//! | `kiosk-exit` | a body without `code` | 400 |
//! | `kiosk-exit` | revoked or unknown agent token | 401 `AGENT_TOKEN_INVALID` |
//!
//! Through the server's whole route table with real sessions (`rpc_common`): `OWNER` holds EXECUTIVE, `MANAGER` is a
//! branch manager of MAIN, `FRONT` a teller of MAIN. Codes are issued at the wall clock, so the test clock that
//! `kiosk-exit` reads is set from `Utc::now()`.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, MAIN, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const ORIGIN: &str = "http://desk.test";
const ISSUE: &str = "/api/v3/commands/counter_unlock_issue";
const EXIT: &str = "/api/device-agent/kiosk-exit";
const B2: &str = "B2";

const COUNTER: &str = "a0000000-0000-4000-8000-000000000001";
const OTHER_COUNTER: &str = "a0000000-0000-4000-8000-000000000002";
const OF_B2: &str = "a0000000-0000-4000-8000-0000000000b2";
const REVOKED: &str = "a0000000-0000-4000-8000-0000000000dd";
const UNKNOWN: &str = "a0000000-0000-4000-8000-0000000000ee";

const TOKEN: &str = "xcda_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const OTHER_TOKEN: &str = "xcda_1111111111111111111111111111111111111111111111111111111111111111";
const UNKNOWN_TOKEN: &str = "xcda_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// MAIN with two paired counters and a revoked one; B2 with one counter.
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        desk.clock.set(Utc::now());
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'สาขาสอง')")
            .bind(B2)
            .execute(&desk.db.pool)
            .await
            .expect("insert B2");
        let room = Room { desk, app, cookies };
        room.counter(COUNTER, MAIN, Some(TOKEN)).await;
        room.counter(OTHER_COUNTER, MAIN, Some(OTHER_TOKEN)).await;
        room.counter(OF_B2, B2, None).await;
        room.counter(REVOKED, MAIN, None).await;
        sqlx::query(
            "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        )
        .bind(REVOKED)
        .execute(&room.desk.db.pool)
        .await
        .expect("revoke a counter");
        let answer = room.issue("MANAGER", "not-a-counter").await;
        assert_ne!(
            answer.status, 404,
            "#6569: {ISSUE} is not served yet: {}",
            answer.text
        );
        room
    }

    async fn counter(&self, id: &str, branch: &str, token: Option<&str>) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, paired_at) \
             VALUES ($1::uuid, $2, 'Mac', now(), 'owner', $3, now())",
        )
        .bind(id)
        .bind(branch)
        .bind(token.map(sha256_hex))
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert counter {id}: {e}"));
    }

    async fn issue(&self, who: &str, agent_id: &str) -> Answer {
        self.app
            .send(
                Call::post(ORIGIN, ISSUE, json!({ "agentId": agent_id }).to_string())
                    .header("idempotency-key", &uuid::Uuid::new_v4().to_string())
                    .cookie(&self.cookies[who]),
            )
            .await
    }

    /// The code a 200 from `counterUnlockIssue` carries.
    async fn code_for(&self, who: &str, agent_id: &str) -> String {
        let answer = self.issue(who, agent_id).await;
        assert_eq!(answer.status, 200, "issue: {}", answer.text);
        output_of(&answer)["code"]
            .as_str()
            .expect("a code")
            .to_owned()
    }

    async fn exit_at(&self, at: DateTime<Utc>, token: &str, body: Value) -> Answer {
        self.desk.clock.set(at);
        self.app
            .send(
                Call::post(ORIGIN, EXIT, body.to_string())
                    .without("origin")
                    .without("x-xchange-request")
                    .header("x-real-ip", "10.1.1.1")
                    .header("authorization", &format!("Bearer {token}")),
            )
            .await
    }

    async fn exit(&self, token: &str, code: &str) -> Answer {
        self.exit_at(Utc::now(), token, json!({ "code": code }))
            .await
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM counter_unlock_tokens")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("count the codes")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The v3 error code, or the device route's `error`.
fn error_of(answer: &Answer) -> String {
    let body = answer.json();
    body["error"]["code"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .unwrap_or("")
        .to_owned()
}

/// The `{code, expiresAt}` of a v3 command's 200, wherever the envelope puts it.
fn output_of(answer: &Answer) -> Value {
    let body = answer.json();
    if body.get("data").is_some() {
        body["data"].clone()
    } else {
        body
    }
}

// ── issue ───────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn a_branch_manager_gets_a_code_good_for_60_seconds_and_one_row() {
    let room = Room::open().await;
    let before = Utc::now();
    let answer = room.issue("MANAGER", COUNTER).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let out = output_of(&answer);
    let code = out["code"].as_str().expect("code");
    assert_eq!(code.len(), 11, "XXXXX-XXXXX: {code}");
    assert_eq!(&code[5..6], "-", "XXXXX-XXXXX: {code}");
    let expires: DateTime<Utc> = out["expiresAt"]
        .as_str()
        .expect("expiresAt")
        .parse()
        .expect("RFC 3339");
    assert!(expires >= before + Duration::seconds(60));
    assert!(expires <= Utc::now() + Duration::seconds(60));

    let (agent, issued_by, stored): (String, String, String) = sqlx::query_as(
        "SELECT agent_id::text, issued_by::text, code_hash FROM counter_unlock_tokens",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("one row");
    assert_eq!(agent, COUNTER);
    let manager = one_of_each_role()
        .into_iter()
        .find(|p| p.key == "MANAGER")
        .expect("MANAGER");
    assert_eq!(issued_by, manager.id);
    assert!(!stored.contains(&code.replace('-', "")), "the code is not stored as typed");
    room.finish().await;
}

#[actix_web::test]
async fn the_owner_gets_a_code() {
    let room = Room::open().await;
    let answer = room.issue("OWNER", COUNTER).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(room.rows().await, 1);
    room.finish().await;
}

#[actix_web::test]
async fn a_teller_is_refused_and_nothing_is_written() {
    let room = Room::open().await;
    let answer = room.issue("FRONT", COUNTER).await;
    assert_ne!(answer.status, 200, "{}", answer.text);
    assert_eq!(error_of(&answer), "roleNotAllowedHere", "{}", answer.text);
    assert_eq!(room.rows().await, 0);
    room.finish().await;
}

#[actix_web::test]
async fn another_branch_s_counter_is_refused() {
    let room = Room::open().await;
    let answer = room.issue("MANAGER", OF_B2).await;
    assert_eq!(error_of(&answer), "COUNTER_OTHER_BRANCH", "{}", answer.text);
    assert_eq!(room.rows().await, 0);
    room.finish().await;
}

#[actix_web::test]
async fn a_revoked_or_unknown_counter_is_refused() {
    let room = Room::open().await;
    for agent in [REVOKED, UNKNOWN] {
        let answer = room.issue("MANAGER", agent).await;
        assert_eq!(error_of(&answer), "COUNTER_REVOKED", "{agent}: {}", answer.text);
    }
    assert_eq!(room.rows().await, 0);
    room.finish().await;
}

// ── kiosk-exit ──────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_counter_uses_its_code_once() {
    let room = Room::open().await;
    let code = room.code_for("MANAGER", COUNTER).await;

    let first = room.exit(TOKEN, &code).await;
    assert_eq!(first.status, 204, "{}", first.text);
    let redeemed: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT redeemed_at FROM counter_unlock_tokens")
            .fetch_one(&room.desk.db.pool)
            .await
            .expect("the row");
    assert!(redeemed.is_some(), "redeemed_at is set");

    let again = room.exit(TOKEN, &code).await;
    assert_eq!(again.status, 403, "{}", again.text);
    assert_eq!(error_of(&again), "UNLOCK_REFUSED");
    room.finish().await;
}

#[actix_web::test]
async fn the_code_typed_without_the_dash_or_in_lower_case_still_works() {
    let room = Room::open().await;
    let code = room.code_for("MANAGER", COUNTER).await;
    let typed = code.replace('-', "").to_lowercase();
    let answer = room.exit(TOKEN, &typed).await;
    assert_eq!(answer.status, 204, "{}", answer.text);
    room.finish().await;
}

#[actix_web::test]
async fn an_expired_code_is_refused() {
    let room = Room::open().await;
    let code = room.code_for("MANAGER", COUNTER).await;
    let answer = room
        .exit_at(
            Utc::now() + Duration::seconds(61),
            TOKEN,
            json!({ "code": code }),
        )
        .await;
    assert_eq!(answer.status, 403, "{}", answer.text);
    assert_eq!(error_of(&answer), "UNLOCK_REFUSED");
    room.finish().await;
}

#[actix_web::test]
async fn another_counter_cannot_use_the_code_and_the_right_one_still_can() {
    let room = Room::open().await;
    let code = room.code_for("MANAGER", COUNTER).await;

    let wrong = room.exit(OTHER_TOKEN, &code).await;
    assert_eq!(wrong.status, 403, "{}", wrong.text);
    assert_eq!(error_of(&wrong), "UNLOCK_REFUSED");

    let right = room.exit(TOKEN, &code).await;
    assert_eq!(right.status, 204, "{}", right.text);
    room.finish().await;
}

#[actix_web::test]
async fn a_code_never_issued_or_not_a_code_is_refused() {
    let room = Room::open().await;
    room.code_for("MANAGER", COUNTER).await;
    for code in ["ABCDE-FGHJK", "1234", ""] {
        let answer = room.exit(TOKEN, code).await;
        assert_eq!(answer.status, 403, "{code:?}: {}", answer.text);
        assert_eq!(error_of(&answer), "UNLOCK_REFUSED");
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_body_without_a_code_is_a_bad_request() {
    let room = Room::open().await;
    let answer = room.exit_at(Utc::now(), TOKEN, json!({})).await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    room.finish().await;
}

#[actix_web::test]
async fn an_unknown_agent_token_is_401() {
    let room = Room::open().await;
    let code = room.code_for("MANAGER", COUNTER).await;
    let answer = room.exit(UNKNOWN_TOKEN, &code).await;
    assert_eq!(answer.status, 401, "{}", answer.text);
    assert_eq!(error_of(&answer), "AGENT_TOKEN_INVALID");
    room.finish().await;
}
