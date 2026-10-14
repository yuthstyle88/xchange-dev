//! Ticket #769 — booking create judges a stale line by the policy it was published under, read
//! through `PublishedRates::stale_policy_in`; a stale line whose policy cannot be found is a
//! server error and writes nothing.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.8 "Stale at read time" (the policy version the
//! line was published under; `StalePolicy::admits`: STOP refuses, LIMIT refuses above
//! `stale_limit_thb`, NULL admits), §4.9 R5 (stale is checked at create; *a stale line whose
//! policy cannot be found must be an error*; a refusal is the reader's "no rate" answer, for
//! bookings today's `RateMoved`). Ticket #769 §7 (booking create: no line, so `RateMoved`), §9.
//!
//! Every request goes through the server's route table (`POST /api/bookings`, the owner) with
//! real sessions (`rpc_common`), as `rates_re4_bookings_test.rs` drives it. `/api/bookings` answers
//! `AppError::Internal` with 500 (`presentation/http/bookings.rs`, `server_error`). Stale lines are
//! seeded with SQL by `stale_policy_769_common` (why: see its header).
//!
//! Already pinned before this ticket, not repeated here (`rates_re4_bookings_test.rs`):
//! `a_stale_line_is_refused_under_the_policy_it_was_published_under` (STOP: 400 `RateMoved`,
//! nothing written), `under_limit_a_stale_line_is_booked_up_to_the_limit_in_half_up_thb` (LIMIT:
//! 231.00 booked under a 231.00 limit, 36.13 refused under 36.12).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `PublishedRates::stale_policy_in(tx, line: &PublishedSide) ->
//!   AppResult<StalePolicy>` (ticket #769 §7), which booking create calls for a stale line; this
//!   file pins its effect through the route;
//! - the `/api/bookings` create payload and `{saved, booking}` answer of
//!   `rates_re4_bookings_test.rs` (RE-4d, merged); `bookings.published_rate_line_id`.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;
mod stale_policy_769_common;

use chrono::{DateTime, Duration, Utc};
use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, sign_in};
use serde_json::{Value, json};
use sqlx::PgPool;
use stale_policy_769_common::{
    Names, id_of, policy, policy_rows, publish_auto, reference_age, unfindable,
};

const ORIGIN: &str = "http://desk.test";
const OWNER_ROUTE: &str = "/api/v2/bookings";
const MAIN: &str = "MAIN";
/// USD per 1: BUY 36.25, SELL 36.75.
const USD: (&str, &str, &str, &str) = ("USD", "1", "36.25", "36.75");

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

fn owner() -> Person {
    Person {
        key: "OWNER".to_string(),
        kind: "owner".to_string(),
        id: "owner".to_string(),
        username: "owner".to_string(),
        name: "Owner".to_string(),
        role: "OWNER".to_string(),
        branch_id: None,
        must_change: false,
    }
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an instant")
        .with_timezone(&Utc)
}

impl Room {
    /// `MAIN` with an open shift; the owner signed in; no publication yet (as
    /// `rates_re4_bookings_test.rs` opens it).
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = vec![owner()];
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, \
                 open_request_id, teller_id) \
             VALUES ('SH-MAIN', 'MAIN', 'OPEN', $1, 'owner', NULL, NULL, 'open-SH-MAIN', \
                 (SELECT u.id FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS \
                   (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' \
                    AND r.branch_id IS NULL)))",
        )
        .bind(at("2026-10-06T04:00:00.000Z"))
        .execute(&desk.db.pool)
        .await
        .expect("insert into shifts");
        desk.clock.set(at("2026-10-06T05:00:00.000Z"));
        Room { desk, app, cookies }
    }

    fn pool(&self) -> PgPool {
        self.desk.db.pool.clone()
    }

    async fn create(&self, request_id: &str, amount: Value, rate: Value) -> Answer {
        self.desk.clock.advance(Duration::seconds(1));
        let payload = json!({
            "branchId": MAIN,
            "requestId": request_id,
            "name": "Somchai Jaidee",
            "phone": "081-234-5678",
            "type": "BUY",
            "currency": "USD",
            "amount": amount,
            "expectedRate": rate
        });
        let body = json!({ "action": "create", "payload": payload }).to_string();
        let call = Call::post(ORIGIN, OWNER_ROUTE, body).cookie(&self.cookies["OWNER"]);
        self.app.send(call).await
    }

    async fn bookings(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM v2_bookings")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("count bookings")
    }

    /// The booking's id from a `{saved: true, booking}` answer, and the line it stored.
    async fn saved(&self, answer: &Answer, what: &str) -> Option<String> {
        assert_eq!(answer.status, 200, "{what}: {}", answer.text);
        let body = answer.json();
        assert_eq!(body["saved"], json!(true), "{what}: {}", answer.text);
        let id = body["booking"]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{what}: the booking's id: {}", answer.text))
            .to_owned();
        sqlx::query_scalar("SELECT published_rate_line_id::text FROM v2_bookings WHERE id = $1")
            .bind(&id)
            .fetch_one(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{what}: booking {id}: {e}"))
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// §9 "Stale, snapshot without `policy_version_id`" and "Stale, policy row missing": create
/// answers 500 and writes nothing (today it books the stale price). The same line made fresh is
/// booked: no lookup for a line that is not stale (§9 "Line not stale").
#[actix_web::test]
async fn a_stale_line_whose_policy_cannot_be_found_is_a_server_error_and_books_nothing() {
    let room = Room::open().await;
    let pool = room.pool();
    for (n, (names, label)) in unfindable().into_iter().enumerate() {
        assert_eq!(policy_rows(&pool, &names).await, 0, "the setup: {label}");
        let ids = publish_auto(&pool, MAIN, &names, 30, &[USD]).await;
        let before = room.bookings().await;
        let answer = room
            .create(&format!("s769-unfound-{n}"), json!(1), json!(36.25))
            .await;
        assert_eq!(answer.status, 500, "{label}: {:.300}", answer.text);
        assert_eq!(room.bookings().await, before, "{label}: nothing written");

        let line = id_of(&ids, "USD");
        reference_age(&pool, &line, 1).await;
        let answer = room
            .create(&format!("s769-fresh-{n}"), json!(1), json!(36.25))
            .await;
        assert_eq!(
            room.saved(&answer, &format!("fresh, {label}")).await,
            Some(line),
            "{label}: booked at the fresh line"
        );
        assert_eq!(room.bookings().await, before + 1, "{label}");
    }
    room.finish().await;
}

/// §9 "Stale, `on_stale` NULL": a stale line under a policy with no `on_stale` is booked at any
/// amount, at that line.
#[actix_web::test]
async fn a_stale_line_under_a_null_policy_is_booked() {
    let room = Room::open().await;
    let pool = room.pool();
    let none = policy(&pool, MAIN, None, None).await;
    let ids = publish_auto(&pool, MAIN, &Names::Policy(none), 30, &[USD]).await;
    let answer = room.create("s769-null-1", json!(1000), json!(36.25)).await;
    assert_eq!(
        room.saved(&answer, "stale under NULL").await,
        Some(id_of(&ids, "USD"))
    );
    assert_eq!(room.bookings().await, 1);
    room.finish().await;
}
