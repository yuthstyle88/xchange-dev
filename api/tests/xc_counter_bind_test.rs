//! Ticket CA-1c (`docs/design/xc-counter-app.md` §5.4, §8.3, §8.4): `POST /api/v3/commands/desk_bind_counter`, body
//! `{nonce}`, binds the caller's staff session to the counter the nonce names. Written before the implementation. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! | Situation | Required result |
//! |---|---|
//! | valid nonce, active counter of the session's branch | 200 `{counterAgentId}`; `user_sessions.counter_agent_id` of that session row = the agent; another session's row untouched |
//! | the counter is of another branch | refused `COUNTER_OTHER_BRANCH`, row unchanged |
//! | the counter is revoked | refused `COUNTER_REVOKED`, row unchanged |
//! | `now >= exp` | refused `NONCE_EXPIRED`, row unchanged |
//! | tampered or garbage nonce | refused `NONCE_INVALID`, row unchanged |
//! | no session | 401 `signIn`, as other v3 commands |
//! | binding again with another counter's nonce | replaces the binding |
//! | `POST /api/auth/refresh` | keeps the binding (`refresh` updates the row in place) |
//! | `POST /api/auth/session/branch` | clears `counter_agent_id` |
//! | `POST /api/auth/logout` | removes the row, so the binding goes with it |
//! | `PgSessionBinding::bound_counter` | the bound agent; `None` when unbound or the row is gone |
//!
//! Sessions are real: signed in at `POST /api/auth/login` against the fake Identity of `identity_signin_common`, as in
//! `session_branch_test.rs`. The nonce is made in the test with `DeskNonce::issue` + `seal` and the server's PII keys
//! (`xc_customer::get_pii_keys`). Every test first asks the command with a garbage nonce, so that it fails on the
//! route (404) until the route exists, not on `DeskNonce`'s `todo!` (CA-1b).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; each test gets its own schema.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::sync::Arc;

use auth_http_common::{Answer, Call, Desk, TestApp, send};
use chrono::{DateTime, Duration, Utc};
use identity_signin_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use xchange::infrastructure::device_agents::desk_binding::DeskNonce;
use xchange::infrastructure::device_agents::session_binding::PgSessionBinding;

const BIND: &str = "/api/v3/commands/desk_bind_counter";
const MAIN: &str = "MAIN";
const B2: &str = "B2";

const COUNTER_1: &str = "c0000000-0000-4000-8000-000000000001";
const COUNTER_2: &str = "c0000000-0000-4000-8000-000000000002";
const COUNTER_OF_B2: &str = "c0000000-0000-4000-8000-0000000000b2";
const COUNTER_REVOKED: &str = "c0000000-0000-4000-8000-0000000000dd";

struct Room {
    desk: Desk,
    identity: FakeIdentity,
    app: App,
    reader: TestApp,
}

async fn counter(pool: &PgPool, id: &str, branch: &str, revoked: bool) {
    sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, paired_at, revoked_at, revoked_by) \
         VALUES ($1::uuid, $2, 'Mac', now(), 'owner', now(), \
         CASE WHEN $3 THEN now() END, CASE WHEN $3 THEN 'owner' END)",
    )
    .bind(id)
    .bind(branch)
    .bind(revoked)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("insert counter {id}: {e}"));
}

impl Room {
    /// MAIN with two active counters and a revoked one, and B2 with one counter.
    async fn open() -> Room {
        let keys =
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .expect("the test PII key parses");
        let _ = xc_customer::install_pii_keys(Arc::new(keys));
        let desk = Desk::start().await;
        let org = organization(&desk.pool).await;
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, code) VALUES ('B2', 'สาขาสอง', $1::uuid, 'Z09')",
        )
        .bind(&org)
        .execute(&desk.pool)
        .await
        .expect("insert B2");
        counter(&desk.pool, COUNTER_1, MAIN, false).await;
        counter(&desk.pool, COUNTER_2, MAIN, false).await;
        counter(&desk.pool, COUNTER_OF_B2, B2, false).await;
        counter(&desk.pool, COUNTER_REVOKED, MAIN, true).await;
        let identity = FakeIdentity::start();
        let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
        let reader = desk_app_without_custody!(desk);
        let room = Room {
            desk,
            identity,
            app,
            reader,
        };
        let answer = send(&room.reader, room.bind_call(None, "garbage")).await;
        assert_ne!(
            answer.status, 404,
            "CA-1c: {BIND} is not served yet: {}",
            answer.text
        );
        room
    }

    /// The teller of MAIN (and of B2, to switch to), signed in; the session cookie's value.
    async fn teller_a(&self) -> String {
        bound_user(
            &self.desk.pool,
            SUB_A,
            "ACTIVE",
            "Nok",
            &[("TELLER", Some(MAIN)), ("TELLER", Some(B2))],
        )
        .await;
        self.sign_in(VALID_A).await
    }

    async fn teller_b(&self) -> String {
        bound_user(
            &self.desk.pool,
            SUB_B,
            "ACTIVE",
            "Mai",
            &[("TELLER", Some(MAIN))],
        )
        .await;
        self.sign_in(VALID_B).await
    }

    async fn sign_in(&self, identity_token: &str) -> String {
        self.identity
            .login_answers("/auth/login", identity_token, REFRESH_1);
        let reply = self
            .app
            .send(Req::post(
                "/api/auth/login",
                json!({ "identifier": "+66812345678", "password": "pw-from-the-browser-01" }),
            ))
            .await;
        assert_eq!(reply.status, 200, "sign-in: {}", reply.text);
        reply.session()
    }

    fn bind_call(&self, session: Option<&str>, nonce: &str) -> Call {
        let call = Call::post(BIND, json!({ "nonce": nonce }))
            .header("idempotency-key", &uuid::Uuid::new_v4().to_string());
        match session {
            Some(token) => call.session(token),
            None => call,
        }
    }

    async fn bind(&self, session: &str, nonce: &str) -> Answer {
        send(&self.reader, self.bind_call(Some(session), nonce)).await
    }

    /// `user_sessions.counter_agent_id` of the session row, which must exist.
    async fn bound(&self, session: &str) -> Option<String> {
        sqlx::query_scalar("SELECT counter_agent_id::text FROM user_sessions WHERE hash = $1")
            .bind(sha256_hex(session))
            .fetch_one(&self.desk.pool)
            .await
            .expect("the session row")
    }

    async fn rows_of(&self, session: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM user_sessions WHERE hash = $1")
            .bind(sha256_hex(session))
            .fetch_one(&self.desk.pool)
            .await
            .expect("count the session rows")
    }

    async fn finish(self) {
        let Room { desk, reader, .. } = self;
        drop(reader);
        desk.drop_schema().await;
    }
}

fn sealed_at(agent: &str, branch: &str, at: DateTime<Utc>) -> String {
    let keys = xc_customer::get_pii_keys().expect("the server's PII keys");
    DeskNonce::issue(agent, branch, at).seal(&keys)
}

/// A nonce good right now.
fn sealed(agent: &str, branch: &str) -> String {
    sealed_at(agent, branch, Utc::now())
}

fn code(answer: &Answer) -> String {
    let body: Value = answer.json();
    body["error"]["code"].as_str().unwrap_or("").to_owned()
}

// ── binding ─────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn a_valid_nonce_binds_this_session_and_no_other() {
    let room = Room::open().await;
    let session = room.teller_a().await;
    let other = room.teller_b().await;

    let answer = room.bind(&session, &sealed(COUNTER_1, MAIN)).await;

    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json(), json!({ "counterAgentId": COUNTER_1 }));
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    assert_eq!(room.bound(&other).await, None, "another session's row");
    room.finish().await;
}

/// §5.4: "The app re-binds … on each desk load", and a replay inside the 60 s binds the same counter again.
#[actix_web::test]
async fn the_same_nonce_again_binds_the_same_counter() {
    let room = Room::open().await;
    let session = room.teller_a().await;
    let nonce = sealed(COUNTER_1, MAIN);

    let first = room.bind(&session, &nonce).await;
    let second = room.bind(&session, &nonce).await;

    assert_eq!(first.status, 200, "{}", first.text);
    assert_eq!(second.status, 200, "{}", second.text);
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    room.finish().await;
}

#[actix_web::test]
async fn binding_again_with_another_counters_nonce_replaces_it() {
    let room = Room::open().await;
    let session = room.teller_a().await;

    let first = room.bind(&session, &sealed(COUNTER_1, MAIN)).await;
    assert_eq!(first.status, 200, "{}", first.text);
    let second = room.bind(&session, &sealed(COUNTER_2, MAIN)).await;

    assert_eq!(second.status, 200, "{}", second.text);
    assert_eq!(second.json(), json!({ "counterAgentId": COUNTER_2 }));
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_2));
    room.finish().await;
}

// ── refusals: the row stays as it was ───────────────────────────────────────

/// Bound to `COUNTER_1` first, so that "row unchanged" is seen on a value, not on NULL.
async fn bound_session(room: &Room) -> String {
    let session = room.teller_a().await;
    let answer = room.bind(&session, &sealed(COUNTER_1, MAIN)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    session
}

#[actix_web::test]
async fn a_counter_of_another_branch_is_refused() {
    let room = Room::open().await;
    let session = bound_session(&room).await;

    let answer = room.bind(&session, &sealed(COUNTER_OF_B2, B2)).await;

    assert_eq!(code(&answer), "COUNTER_OTHER_BRANCH", "{}", answer.text);
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    room.finish().await;
}

/// §5.4: the agent's `branch_id` is what counts, not the branch the nonce says. A nonce the server signed cannot say a
/// different branch today; this pins that the row is read rather than the nonce believed.
#[actix_web::test]
async fn the_branch_is_the_counters_own_not_the_one_in_the_nonce() {
    let room = Room::open().await;
    let session = bound_session(&room).await;

    let answer = room.bind(&session, &sealed(COUNTER_OF_B2, MAIN)).await;

    assert_eq!(code(&answer), "COUNTER_OTHER_BRANCH", "{}", answer.text);
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    room.finish().await;
}

/// A session that switched branch is judged by its new branch (`session_service.rs:402`'s column).
#[actix_web::test]
async fn the_sessions_branch_is_the_one_it_is_on_now() {
    let room = Room::open().await;
    let session = room.teller_a().await;
    let switched = room
        .app
        .send(Req::post("/api/auth/session/branch", json!({ "branch": B2 })).with_session(&session))
        .await;
    assert_eq!(switched.status, 200, "{}", switched.text);

    let refused = room.bind(&session, &sealed(COUNTER_1, MAIN)).await;
    assert_eq!(code(&refused), "COUNTER_OTHER_BRANCH", "{}", refused.text);
    assert_eq!(room.bound(&session).await, None);

    let answer = room.bind(&session, &sealed(COUNTER_OF_B2, B2)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_OF_B2));
    room.finish().await;
}

#[actix_web::test]
async fn a_revoked_counter_is_refused() {
    let room = Room::open().await;
    let session = bound_session(&room).await;

    let answer = room.bind(&session, &sealed(COUNTER_REVOKED, MAIN)).await;

    assert_eq!(code(&answer), "COUNTER_REVOKED", "{}", answer.text);
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    room.finish().await;
}

/// `now >= exp`: a nonce issued 60 s ago has `exp` = now, so it is already out, and one issued long ago too.
#[actix_web::test]
async fn an_expired_nonce_is_refused() {
    let room = Room::open().await;
    let session = bound_session(&room).await;

    for ago in [60, 3_600] {
        let nonce = sealed_at(COUNTER_2, MAIN, Utc::now() - Duration::seconds(ago));
        let answer = room.bind(&session, &nonce).await;
        assert_eq!(
            code(&answer),
            "NONCE_EXPIRED",
            "issued {ago} s ago: {}",
            answer.text
        );
        assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_garbage_or_tampered_nonce_is_refused() {
    let room = Room::open().await;
    let session = bound_session(&room).await;
    let good = sealed(COUNTER_2, MAIN);
    let (without_mac, _) = good.rsplit_once('.').expect("the wire form has a MAC");
    let other_key =
        xc_kernel::pii::PiiKeys::parse("test-2:HxweHRwbGhkYFxYVFBMSERAPDg0MCwoJCAcGBQQDAgE=")
            .expect("a second key parses");

    for (label, nonce) in [
        ("garbage", "garbage".to_owned()),
        ("empty", String::new()),
        ("no MAC", without_mac.to_owned()),
        // The MAC of COUNTER_2's nonce over COUNTER_1's id.
        ("another agent", good.replacen(COUNTER_2, COUNTER_1, 1)),
        ("another version", good.replacen("v1.", "v2.", 1)),
        (
            "signed with another key",
            DeskNonce::issue(COUNTER_2, MAIN, Utc::now()).seal(&other_key),
        ),
    ] {
        let answer = room.bind(&session, &nonce).await;
        assert_eq!(code(&answer), "NONCE_INVALID", "{label}: {}", answer.text);
        assert_eq!(
            room.bound(&session).await.as_deref(),
            Some(COUNTER_1),
            "{label}"
        );
    }
    room.finish().await;
}

/// No session ⇒ as other v3 commands.
#[actix_web::test]
async fn no_session_is_refused_as_other_v3_commands() {
    let room = Room::open().await;
    let answer = send(&room.reader, room.bind_call(None, &sealed(COUNTER_1, MAIN))).await;
    assert_eq!(answer.status, 401, "{}", answer.text);
    assert_eq!(code(&answer), "signIn", "{}", answer.text);
    room.finish().await;
}

// ── how long a binding lasts ────────────────────────────────────────────────

/// §5.4: `refresh` updates the row in place, so the binding survives.
#[actix_web::test]
async fn a_token_refresh_keeps_the_binding() {
    let room = Room::open().await;
    let session = bound_session(&room).await;
    room.identity
        .login_answers("/auth/refresh", VALID_A, REFRESH_2);

    let reply = room
        .app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&session))
        .await;

    assert_eq!(reply.status, 200, "refresh: {}", reply.text);
    assert_eq!(room.bound(&session).await.as_deref(), Some(COUNTER_1));
    room.finish().await;
}

/// §8.5: `switch_branch` changes `branch_id` in place, so a binding left behind would point at another branch's
/// counter.
#[actix_web::test]
async fn switching_branch_clears_the_binding() {
    let room = Room::open().await;
    let session = bound_session(&room).await;

    let reply = room
        .app
        .send(Req::post("/api/auth/session/branch", json!({ "branch": B2 })).with_session(&session))
        .await;

    assert_eq!(reply.status, 200, "switch: {}", reply.text);
    assert_eq!(room.bound(&session).await, None);
    room.finish().await;
}

/// §5.4: sign-out deletes the row, and the binding with it.
#[actix_web::test]
async fn signing_out_removes_the_row() {
    let room = Room::open().await;
    let session = bound_session(&room).await;

    let reply = room
        .app
        .send(Req::post("/api/auth/logout", json!({})).with_session(&session))
        .await;

    assert_eq!(reply.status, 200, "logout: {}", reply.text);
    assert_eq!(room.rows_of(&session).await, 0);
    room.finish().await;
}

/// A new sign-in writes a new row, which starts unbound (§5.4 "a new sign-in writes a new row").
#[actix_web::test]
async fn a_new_sign_in_starts_unbound() {
    let room = Room::open().await;
    let first = bound_session(&room).await;

    let second = room.sign_in(VALID_A).await;

    assert_ne!(first, second);
    assert_eq!(room.bound(&second).await, None);
    room.finish().await;
}

// ── PgSessionBinding ────────────────────────────────────────────────────────

#[actix_web::test]
async fn bound_counter_answers_the_bound_agent_and_none_when_unbound_or_gone() {
    let room = Room::open().await;
    let binding = PgSessionBinding::new(room.desk.pool.clone());
    let session = room.teller_a().await;
    let hash = sha256_hex(&session);

    assert_eq!(
        binding.bound_counter(&hash).await.expect("read"),
        None,
        "unbound"
    );

    let answer = room.bind(&session, &sealed(COUNTER_1, MAIN)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(
        binding.bound_counter(&hash).await.expect("read").as_deref(),
        Some(COUNTER_1),
        "bound"
    );

    sqlx::query("DELETE FROM user_sessions WHERE hash = $1")
        .bind(&hash)
        .execute(&room.desk.pool)
        .await
        .expect("delete the row");
    assert_eq!(
        binding.bound_counter(&hash).await.expect("read"),
        None,
        "the row is gone"
    );
    room.finish().await;
}
