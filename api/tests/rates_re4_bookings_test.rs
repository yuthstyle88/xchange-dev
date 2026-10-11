//! Ticket #758 (RE-4d) — a booking is created on the current published line, stores that line,
//! and its confirm refuses when the line moved.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §3.2: `ExchangeArithmetic::thb` (amount × rate ÷ unit_size, HALF_UP once to 2 places);
//! - §4.8: bookings read `line_in` and store `bookings.published_rate_line_id`, `rate`,
//!   `unit_size`; a later publication never rewrites a stored row; "Stale at read time";
//! - §4.9 R1 (storage per unit), R2 (every THB figure through `thb`), R3 (the v2 booking JSON
//!   stays per 1: `rate ÷ unit_size`), R4 (one check before a write, by line id), R5 (stale under
//!   the policy version the line was published under; the refusal is the reader's "no rate"
//!   answer, for bookings today's `RateMoved`), R6 (booking confirm compares the stored line with
//!   the current one and never re-locks; a moved line refuses and the customer books again);
//! - D-3: the current rate is the current publication's line, not the newest `rates` row;
//! - §8 migration 5 (the column; rows written before RE-4 hold NULL); §9 RE-4d;
//! - the ticket author's rulings on #758 (2026-10-10): a per-1 rate on the v2 wire carries up to
//!   8 + log10(unit_size) places, 11 at most (4); stale is checked at create, not at confirm (5).
//!
//! Every request goes through the server's route table (`POST /api/bookings`, the owner) with
//! real sessions (`rpc_common`), as `bookings_http_test.rs` drives it. `/api/bookings` is the v2
//! booking wire (`bookings_http_test.rs` header), so by R3 its rates are per 1 in both directions:
//! `expectedRate` (create and confirm) and `booking.rate` in every answer. A refusal is today's
//! `RateMoved` sentence and writes nothing: the whole row is compared before and after.
//!
//! "A publish between create and confirm" is a real interleaving: create commits, the owner
//! publishes through the rate engine (`rates_re4_common::publish`), then confirm is sent.
//!
//! Stale lines (R5) are seeded with SQL as `quote_common/mod.rs` seeds them: an AUTO line whose
//! reference was fetched 30 h ago, under a policy whose references go stale after 24 h, with the
//! policy version's id in `source_snapshot.policy_version_id`. `PgPublishedRates::line_in`
//! decides `stale` with the wall clock, so no test depends on timing.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `bookings.published_rate_line_id uuid NULL REFERENCES published_rate_lines(id)`
//!   (migration 5) holds the line the booking was created at; `bookings.rate` holds that line's
//!   rate per its `unit_size`; `bookings.unit_size` (RU-1) holds the line's `unit_size`. Confirm
//!   never rewrites the three;
//! - [required] the `create`, `get`, `list` and `confirm` answers keep their keys (T15-f,
//!   `bookings_http_test.rs`); `booking.rate` and `expectedRate` are per 1 (R3);
//! - [required] the refusal sentence `เรทเปลี่ยน กรุณาโหลดใหม่` (T15-a `BookingError::RateMoved`).
//!
//! Re-expected for RE-7 #604 by the ticket author: the decoy `rates` row is gone with the table
//! (rates.md §8 migration 6); every test still proves what it proved, on publications alone.

mod common;
mod compat_common;
mod rates_re4_common;
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use compat_common::Person;
use rates_re4_common::{d, line, publish};
use rpc_common::{Answer, App, Call, Cookies, Desk, sign_in};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use xchange::presentation::http::rpc::RpcClock;

const ORIGIN: &str = "http://desk.test";
const OWNER_ROUTE: &str = "/api/bookings";
const MAIN: &str = "MAIN";
const RATE_MOVED: &str = "เรทเปลี่ยน กรุณาโหลดใหม่";
const CUSTOMER: &str = "C-MAIN-1";
const SECRET: &str = "6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b";

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

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// One `bookings` row as stored: every column a confirm or a refusal could touch.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct Row {
    published_rate_line_id: Option<String>,
    rate: Decimal,
    unit_size: Decimal,
    amount: Decimal,
    status: String,
    expires_at: Option<DateTime<Utc>>,
    confirmed_at: Option<DateTime<Utc>>,
    confirmed_by: Option<String>,
}

impl Room {
    /// `MAIN` with an open shift and a LIVE customer `C-MAIN-1`; the owner signed in; no
    /// publication yet. (Before RE-7 a direct-save USD 36.25 / 36.75 sat in `rates` as a decoy the
    /// bookings must not read; RE-7 #604 drops the table, so there is nothing left to decoy.)
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
        sqlx::query(
            // CI-4 (#651): a customer belongs to the organisation
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, organization_id, $1, 'Somchai Jaidee', 'LIVE', $2 FROM branches WHERE id = 'MAIN'",
        )
        .bind(CUSTOMER)
        .bind(at("2026-10-06T04:00:00.000Z"))
        .execute(&desk.db.pool)
        .await
        .expect("insert into customers");
        desk.clock.set(at("2026-10-06T05:00:00.000Z"));
        Room { desk, app, cookies }
    }

    fn pool(&self) -> PgPool {
        self.desk.db.pool.clone()
    }

    async fn act(&self, action: &str, payload: Value) -> Answer {
        self.desk.clock.advance(Duration::seconds(1));
        let body = json!({ "action": action, "payload": payload }).to_string();
        let call = Call::post(ORIGIN, OWNER_ROUTE, body).cookie(&self.cookies["OWNER"]);
        self.app.send(call).await
    }

    async fn create(&self, payload: Value) -> Answer {
        self.act("create", payload).await
    }

    /// The member code of `C-MAIN-1`; answers the member's id.
    async fn member(&self) -> String {
        let answer = self
            .act(
                "member",
                json!({ "branchId": MAIN, "customerId": CUSTOMER, "identityConfirmed": true,
                        "phone": "081-234-5678", "memberSecret": SECRET }),
            )
            .await;
        assert_eq!(answer.status, 200, "issue the member: {}", answer.text);
        assert_eq!(answer.json()["saved"], json!(true), "{}", answer.text);
        let list = self.list().await;
        list["members"][0]["id"]
            .as_str()
            .unwrap_or_else(|| panic!("the member's id: {list}"))
            .to_owned()
    }

    async fn confirm(&self, id: &str, expires_at: DateTime<Utc>, expected: Value) -> Answer {
        self.act(
            "confirm",
            json!({ "branchId": MAIN, "id": id, "expiresAt": stamp(expires_at),
                    "expectedRate": expected }),
        )
        .await
    }

    async fn get(&self, id: &str) -> Value {
        let answer = self.act("get", json!({ "branchId": MAIN, "id": id })).await;
        assert_eq!(answer.status, 200, "get {id}: {}", answer.text);
        answer.json()
    }

    async fn list(&self) -> Value {
        let answer = self.act("list", json!({ "branchId": MAIN })).await;
        assert_eq!(answer.status, 200, "list: {}", answer.text);
        answer.json()
    }

    async fn bookings(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM bookings")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("count bookings")
    }

    async fn row(&self, id: &str) -> Row {
        sqlx::query_as(
            "SELECT published_rate_line_id::text AS published_rate_line_id, rate, unit_size, \
                    amount, status, expires_at, confirmed_at, confirmed_by \
             FROM bookings WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("booking {id}: {e}"))
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn walk_in(request_id: &str, currency: &str, amount: Value, rate: Value) -> Value {
    json!({
        "branchId": MAIN,
        "requestId": request_id,
        "name": "Somchai Jaidee",
        "phone": "081-234-5678",
        "type": "BUY",
        "currency": currency,
        "amount": amount,
        "expectedRate": rate
    })
}

fn for_member(request_id: &str, currency: &str, amount: Value, rate: Value) -> Value {
    let mut payload = walk_in(request_id, currency, amount, rate);
    payload["customerId"] = json!(CUSTOMER);
    payload
}

/// The booking's id from a `{saved: true, booking}` answer.
fn saved(answer: &Answer, what: &str) -> String {
    assert_eq!(answer.status, 200, "{what}: {}", answer.text);
    let body = answer.json();
    assert_eq!(body["saved"], json!(true), "{what}: {}", answer.text);
    body["booking"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: the booking's id: {}", answer.text))
        .to_owned()
}

/// A JSON number's text (`0.231`, `36.5`): rates print as numbers in their shortest form.
fn number_text(value: &Value, what: &str) -> String {
    assert!(value.is_number(), "{what}: a JSON number, got {value}");
    value.to_string()
}

fn line_id(line: &rates_re4_common::Line) -> Option<String> {
    Some(line.id.clone())
}

// ── stale lines (R5), seeded as quote_common seeds them ─────────────────────

/// A policy version of `branch` with `on_stale` (`None` = NULL) and `stale_limit_thb`, whose
/// references go stale after 24 h. Answers its id.
async fn policy(
    pool: &PgPool,
    branch: &str,
    on_stale: Option<&str>,
    limit: Option<&str>,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO rate_policy_versions (id, organization_id, branch_id, version, \
             buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours, on_stale, \
             stale_limit_thb, valid_from, valid_to, approved_by, reason, source_version) \
         SELECT $1::uuid, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_policy_versions WHERE branch_id = b.id), 0) + 1, \
             0, 0, '{}'::jsonb, 24, $3, $4::numeric, now() - interval '1 day', NULL, \
             'test', 'test', NULL \
         FROM branches b WHERE b.id = $2",
    )
    .bind(&id)
    .bind(branch)
    .bind(on_stale)
    .bind(limit)
    .execute(pool)
    .await
    .expect("insert into rate_policy_versions");
    id
}

/// Publish AUTO lines `(code, unit_size, buy, sell)` whose reference was fetched 30 h ago,
/// under `policy`, as `branch`'s next version, and point the branch at it. Answers the line
/// ids by currency.
async fn publish_stale(
    pool: &PgPool,
    branch: &str,
    policy: &str,
    lines: &[(&str, &str, &str, &str)],
) -> Vec<(String, String)> {
    let request = uuid::Uuid::new_v4().to_string();
    let publication = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1::uuid, organization_id, 'RATE_PUBLICATION', id, $2, 'test', '1', \
             '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $4",
    )
    .bind(&request)
    .bind(&publication)
    .bind(format!("re4d-stale-{publication}"))
    .bind(branch)
    .execute(pool)
    .await
    .expect("insert into approval_requests");
    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
         SELECT $1::uuid, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
             now(), $2::uuid, NULL, 'test' \
         FROM branches b WHERE b.id = $3",
    )
    .bind(&publication)
    .bind(&request)
    .bind(branch)
    .execute(pool)
    .await
    .expect("insert into rate_publications");
    let fetched_at = (Utc::now() - Duration::hours(30)).to_rfc3339();
    let mut ids = Vec::new();
    for (code, unit, buy, sell) in lines {
        let id = uuid::Uuid::new_v4().to_string();
        let snapshot = json!({
            "currency": code,
            "buy_mode": "AUTO",
            "sell_mode": "AUTO",
            "fetched_at": fetched_at,
            "source_date": &fetched_at[..10],
            "max_reference_age_hours": 24,
            "policy_version_id": policy,
        });
        sqlx::query(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, \
                 currency_code, denomination_group, unit_size, buy_rate, sell_rate, \
                 source_snapshot) \
             SELECT $1::uuid, organization_id, $2::uuid, $3, 'ALL', $4::numeric, $5::numeric, \
                 $6::numeric, $7::jsonb \
             FROM rate_publications WHERE id = $2::uuid",
        )
        .bind(&id)
        .bind(&publication)
        .bind(*code)
        .bind(*unit)
        .bind(*buy)
        .bind(*sell)
        .bind(snapshot.to_string())
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("insert the stale {code} line: {e}"));
        ids.push(((*code).to_owned(), id));
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(branch)
        .execute(pool)
        .await
        .expect("point the branch at the stale publication");
    ids
}

/// Make the line's reference `hours` old, as if that much time had passed since it was fetched
/// (`line_in` decides `stale` from `source_snapshot.fetched_at` and the wall clock).
async fn reference_age(pool: &PgPool, line: &str, hours: i64) {
    let fetched_at = (Utc::now() - Duration::hours(hours)).to_rfc3339();
    sqlx::query(
        "UPDATE published_rate_lines \
         SET source_snapshot = jsonb_set(source_snapshot, '{fetched_at}', to_jsonb($2::text)) \
         WHERE id = $1::uuid",
    )
    .bind(line)
    .bind(fetched_at)
    .execute(pool)
    .await
    .expect("age the line's reference");
}

fn id_of(ids: &[(String, String)], code: &str) -> String {
    ids.iter()
        .find(|(c, _)| c == code)
        .map(|(_, id)| id.clone())
        .unwrap_or_else(|| panic!("{code} was published"))
}

// ── create (R1, R3, D-3) ─────────────────────────────────────────────────────

/// D-3, R1: 36.25 (the retired direct save's rate before RE-7) is not the counter rate; the
/// publication's 36.5 is. A create at 36.25 is refused and writes nothing; at 36.5 it stores the USD line, its rate and
/// its unit, and answers the rate it stored.
#[actix_web::test]
async fn a_booking_is_created_at_the_published_line_and_stores_it() {
    let room = Room::open().await;
    let pool = room.pool();
    publish(
        &pool,
        MAIN,
        &[("USD", "36.5", "37"), ("JPY", "23.1", "23.5")],
    )
    .await;
    let usd = line(&pool, MAIN, "USD").await;

    room.create(walk_in("re4d-usd-1", "USD", json!(1000), json!(36.25)))
        .await
        .assert_refused(400, RATE_MOVED, "a rate other than the line");
    assert_eq!(room.bookings().await, 0, "the refusal wrote nothing");

    let answer = room
        .create(walk_in("re4d-usd-2", "USD", json!(1000), json!(36.5)))
        .await;
    let id = saved(&answer, "create at the published rate");
    assert_eq!(
        number_text(&answer.json()["booking"]["rate"], "the answer's rate"),
        "36.5"
    );
    let row = room.row(&id).await;
    assert_eq!(
        row.published_rate_line_id,
        line_id(&usd),
        "the line it was created at"
    );
    assert_eq!(row.rate, d("36.5"), "the line's buy rate");
    assert_eq!(row.unit_size, d("1"), "the line's unit");
    assert_eq!(row.amount, d("1000"));
    assert_eq!(row.status, "CONFIRMED", "a walk-in is confirmed at create");
    room.finish().await;
}

/// R1 + R3 with JPY per 100: 1,000 JPY at 23.1 per 100 is booked with the v2 wire's per-1 rate
/// 0.231. The row stores 23.1 and unit_size 100 (per unit); every answer prints 0.231 (per 1).
/// A later publication rewrites neither the row nor what is printed.
#[actix_web::test]
async fn a_jpy_booking_stores_the_rate_per_100_and_prints_it_per_1() {
    let room = Room::open().await;
    let pool = room.pool();
    publish(&pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    let jpy = line(&pool, MAIN, "JPY").await;
    assert_eq!(jpy.unit_size, d("100"), "the register's JPY unit (RU-1)");

    let answer = room
        .create(walk_in("re4d-jpy-1", "JPY", json!(1000), json!(0.231)))
        .await;
    let id = saved(&answer, "create a JPY booking at the per-1 rate");
    assert_eq!(
        number_text(&answer.json()["booking"]["rate"], "create's answer"),
        "0.231",
        "the v2 booking JSON is per 1 (R3)"
    );
    let stored = room.row(&id).await;
    assert_eq!(stored.published_rate_line_id, line_id(&jpy));
    assert_eq!(stored.rate, d("23.1"), "stored per 100 JPY (R1)");
    assert_eq!(stored.unit_size, d("100"));
    assert_eq!(stored.amount, d("1000"));

    publish(&pool, MAIN, &[("JPY", "23.2", "23.6")]).await;
    assert_eq!(room.row(&id).await, stored, "version 2 rewrites nothing");
    assert_eq!(
        number_text(&room.get(&id).await["rate"], "get"),
        "0.231",
        "get prints the stored rate per 1"
    );
    let list = room.list().await;
    let listed = list["bookings"]
        .as_array()
        .expect("bookings")
        .iter()
        .find(|b| b["id"] == json!(id))
        .unwrap_or_else(|| panic!("{id} is listed: {list}"))
        .clone();
    assert_eq!(number_text(&listed["rate"], "list"), "0.231");
    room.finish().await;
}

/// R3: the v2 wire is per 1, so the per-100 23.1 is not the counter's rate for JPY. It is refused
/// as a moved rate and writes nothing; the same booking at the per-1 0.231 is taken.
#[actix_web::test]
async fn a_per_unit_rate_on_the_v2_booking_wire_is_refused() {
    let room = Room::open().await;
    let pool = room.pool();
    publish(&pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    let jpy = line(&pool, MAIN, "JPY").await;

    room.create(walk_in("re4d-jpy-2", "JPY", json!(1000), json!(23.1)))
        .await
        .assert_refused(400, RATE_MOVED, "23.1 is per 100, the wire is per 1");
    assert_eq!(room.bookings().await, 0, "the refusal wrote nothing");

    let id = saved(
        &room
            .create(walk_in("re4d-jpy-3", "JPY", json!(1000), json!(0.231)))
            .await,
        "the same booking per 1",
    );
    let row = room.row(&id).await;
    assert_eq!(row.published_rate_line_id, line_id(&jpy));
    assert_eq!(row.rate, d("23.1"));
    assert_eq!(row.unit_size, d("100"));
    room.finish().await;
}

/// D-3: a currency the publication has no line for cannot be booked; nor can anything before the
/// branch's first publication.
#[actix_web::test]
async fn a_currency_without_a_published_line_cannot_be_booked() {
    let room = Room::open().await;

    room.create(walk_in("re4d-none-1", "USD", json!(1000), json!(36.25)))
        .await
        .assert_refused(400, RATE_MOVED, "no publication yet");

    publish(&room.pool(), MAIN, &[("JPY", "23.1", "23.5")]).await;
    room.create(walk_in("re4d-none-2", "USD", json!(1000), json!(36.25)))
        .await
        .assert_refused(400, RATE_MOVED, "USD has no line");
    assert_eq!(room.bookings().await, 0, "the refusals wrote nothing");
    room.finish().await;
}

// ── confirm (R4, R6) ─────────────────────────────────────────────────────────

/// R6 with a real interleaving: a member's PENDING booking is made on USD line 1 at 36.5; the
/// owner then publishes 36.6. Confirm is refused, whether the screen sends the stored rate or the
/// new one, and writes nothing: no re-lock at 36.6. A third publication back at 36.5 is a new
/// line, so confirm is still refused (the line id is compared, not the rate).
#[actix_web::test]
async fn confirm_refuses_when_the_line_moved_and_writes_nothing() {
    let room = Room::open().await;
    let pool = room.pool();
    room.member().await;
    publish(&pool, MAIN, &[("USD", "36.5", "37")]).await;
    let first = line(&pool, MAIN, "USD").await;

    let id = saved(
        &room
            .create(for_member("re4d-m-1", "USD", json!(1000), json!(36.5)))
            .await,
        "a member's booking",
    );
    let before = room.row(&id).await;
    assert_eq!(before.status, "PENDING");
    assert_eq!(before.published_rate_line_id, line_id(&first));
    assert_eq!(before.rate, d("36.5"));

    publish(&pool, MAIN, &[("USD", "36.6", "37")]).await;
    let expires = room.desk.clock.now() + Duration::hours(2);
    for expected in [json!(36.5), json!(36.6)] {
        room.confirm(&id, expires, expected.clone())
            .await
            .assert_refused(
                400,
                RATE_MOVED,
                &format!("line moved, expectedRate {expected}"),
            );
        assert_eq!(room.row(&id).await, before, "the refusal wrote nothing");
    }

    publish(&pool, MAIN, &[("USD", "36.5", "37")]).await;
    let third = line(&pool, MAIN, "USD").await;
    assert_ne!(third.id, first.id, "a publication makes new lines");
    room.confirm(&id, expires, json!(36.5))
        .await
        .assert_refused(400, RATE_MOVED, "back at 36.5, on another line");
    assert_eq!(room.row(&id).await, before, "the refusal wrote nothing");
    assert_eq!(room.bookings().await, 1, "no booking was added");
    room.finish().await;
}

/// R6: with no publication since create, confirm sets the status and the sent expiry and keeps
/// the line, the per-unit rate and the unit stored at create. JPY per 100, per 1 on the wire.
#[actix_web::test]
async fn confirm_on_the_line_it_was_created_at_keeps_what_create_stored() {
    let room = Room::open().await;
    let pool = room.pool();
    room.member().await;
    publish(&pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    let jpy = line(&pool, MAIN, "JPY").await;

    let id = saved(
        &room
            .create(for_member("re4d-m-2", "JPY", json!(1000), json!(0.231)))
            .await,
        "a member's JPY booking",
    );
    let created = room.row(&id).await;
    assert_eq!(created.status, "PENDING");

    let expires = room.desk.clock.now() + Duration::hours(2);
    let answer = room.confirm(&id, expires, json!(0.231)).await;
    assert_eq!(answer.status, 200, "confirm: {}", answer.text);
    assert_eq!(answer.json()["saved"], json!(true), "{}", answer.text);

    let confirmed = room.row(&id).await;
    assert_eq!(confirmed.status, "CONFIRMED");
    assert_eq!(confirmed.expires_at, Some(expires), "the sent expiry");
    assert_eq!(
        confirmed.published_rate_line_id,
        line_id(&jpy),
        "the line of create"
    );
    assert_eq!(confirmed.rate, d("23.1"), "the per-unit rate of create");
    assert_eq!(confirmed.unit_size, d("100"));
    assert_eq!(confirmed.amount, created.amount);
    assert_eq!(number_text(&room.get(&id).await["rate"], "get"), "0.231");
    room.finish().await;
}

/// R4 + §8 migration 5: a PENDING booking written before RE-4 holds a NULL line id. No current
/// line is that line, so its confirm is refused and writes nothing, though the current USD line,
/// the direct save and the screen all say its rate, 36.25.
#[actix_web::test]
async fn a_booking_written_before_re4_cannot_be_confirmed() {
    let room = Room::open().await;
    let pool = room.pool();
    let member = room.member().await;
    publish(&pool, MAIN, &[("USD", "36.25", "36.75")]).await;
    sqlx::query(
        "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
             type, code, amount, rate, status, created_at, created_by, unit_size, \
             published_rate_line_id) \
         VALUES ('BK-OLD', 'MAIN', 'req-old', 'h', 'Somchai Jaidee', '081-234-5678', $1, 'BUY', \
             'USD', 1000, 36.25, 'PENDING', $2, 'owner', 1, NULL)",
    )
    .bind(&member)
    .bind(at("2026-10-06T04:30:00.000Z"))
    .execute(&pool)
    .await
    .expect("a booking from before RE-4");
    let before = room.row("BK-OLD").await;

    let expires = room.desk.clock.now() + Duration::hours(2);
    room.confirm("BK-OLD", expires, json!(36.25))
        .await
        .assert_refused(400, RATE_MOVED, "a NULL line is not the current line");
    assert_eq!(
        room.row("BK-OLD").await,
        before,
        "the refusal wrote nothing"
    );
    room.finish().await;
}

// ── stale (R5, R2) ───────────────────────────────────────────────────────────

/// R5: a stale line under a STOP policy is refused with the "no rate" answer and writes nothing,
/// though the line, the direct save and the screen all say 36.25.
/// The policy is the version the line was published under: a later version that would admit it
/// changes nothing.
#[actix_web::test]
async fn a_stale_line_is_refused_under_the_policy_it_was_published_under() {
    let room = Room::open().await;
    let pool = room.pool();
    let stop = policy(&pool, MAIN, Some("STOP"), None).await;
    publish_stale(&pool, MAIN, &stop, &[("USD", "1", "36.25", "36.75")]).await;

    room.create(walk_in("re4d-stale-1", "USD", json!(1), json!(36.25)))
        .await
        .assert_refused(400, RATE_MOVED, "stale under STOP");
    assert_eq!(room.bookings().await, 0, "the refusal wrote nothing");

    policy(&pool, MAIN, None, None).await;
    room.create(walk_in("re4d-stale-2", "USD", json!(1), json!(36.25)))
        .await
        .assert_refused(400, RATE_MOVED, "a later policy does not govern the line");
    assert_eq!(room.bookings().await, 0, "the refusal wrote nothing");
    room.finish().await;
}

/// R5 + R2 under LIMIT: the THB compared with `stale_limit_thb` is `thb(amount, rate,
/// unit_size)`, HALF_UP once.
/// - USD 1 at 36.125 is 36.125 → 36.13 HALF_UP (banker's rounding gives 36.12): refused under a
///   36.12 limit, booked under a 36.13 one.
/// - JPY at 23.1 per 100: 1,000 is 231.00 and is booked under a 231.00 limit (a rate not divided
///   by the unit would be 23,100); 1,000.05 is 231.01155 → 231.01 and is refused.
#[actix_web::test]
async fn under_limit_a_stale_line_is_booked_up_to_the_limit_in_half_up_thb() {
    let room = Room::open().await;
    let pool = room.pool();

    let tight = policy(&pool, MAIN, Some("LIMIT"), Some("36.12")).await;
    publish_stale(&pool, MAIN, &tight, &[("USD", "1", "36.125", "37")]).await;
    room.create(walk_in("re4d-limit-1", "USD", json!(1), json!(36.125)))
        .await
        .assert_refused(400, RATE_MOVED, "36.125 rounds HALF_UP to 36.13 > 36.12");
    assert_eq!(room.bookings().await, 0, "the refusal wrote nothing");

    let enough = policy(&pool, MAIN, Some("LIMIT"), Some("36.13")).await;
    let usd = publish_stale(&pool, MAIN, &enough, &[("USD", "1", "36.125", "37")]).await;
    let id = saved(
        &room
            .create(walk_in("re4d-limit-2", "USD", json!(1), json!(36.125)))
            .await,
        "36.13 is within 36.13",
    );
    let row = room.row(&id).await;
    assert_eq!(row.published_rate_line_id, Some(id_of(&usd, "USD")));
    assert_eq!(row.rate, d("36.125"));

    let jpy_limit = policy(&pool, MAIN, Some("LIMIT"), Some("231.00")).await;
    let jpy = publish_stale(&pool, MAIN, &jpy_limit, &[("JPY", "100", "23.1", "23.5")]).await;
    let id = saved(
        &room
            .create(walk_in("re4d-limit-3", "JPY", json!(1000), json!(0.231)))
            .await,
        "1,000 JPY at 23.1 per 100 is 231.00",
    );
    let row = room.row(&id).await;
    assert_eq!(row.published_rate_line_id, Some(id_of(&jpy, "JPY")));
    assert_eq!(row.rate, d("23.1"));
    assert_eq!(row.unit_size, d("100"));

    room.create(walk_in("re4d-limit-4", "JPY", json!(1000.05), json!(0.231)))
        .await
        .assert_refused(400, RATE_MOVED, "231.01 > 231.00");
    assert_eq!(room.bookings().await, 2, "the refusal wrote nothing");
    room.finish().await;
}

/// R3 and the ticket author's ruling 4 on #758: a per-1 rate carries up to 8 + log10(unit_size)
/// places, 11 for the register's largest unit. KRW at 25.12345678 per 1000 is booked with the
/// v2 wire's 0.02512345678 and stored per 1000; every answer prints the 11 places.
#[actix_web::test]
async fn a_krw_line_per_1000_is_booked_at_its_11_place_per_1_rate() {
    let room = Room::open().await;
    let pool = room.pool();
    publish(&pool, MAIN, &[("KRW", "25.12345678", "25.5")]).await;
    let krw = line(&pool, MAIN, "KRW").await;
    assert_eq!(krw.unit_size, d("1000"), "the register's KRW unit (RU-1)");

    let answer = room
        .create(walk_in(
            "re4d-krw-1",
            "KRW",
            json!(100000),
            json!(0.02512345678),
        ))
        .await;
    let id = saved(&answer, "create at the 11-place per-1 rate");
    assert_eq!(
        number_text(&answer.json()["booking"]["rate"], "create's answer"),
        "0.02512345678"
    );
    let row = room.row(&id).await;
    assert_eq!(row.published_rate_line_id, line_id(&krw));
    assert_eq!(row.rate, d("25.12345678"), "stored per 1000 KRW (R1)");
    assert_eq!(row.unit_size, d("1000"));
    assert_eq!(row.amount, d("100000"));
    assert_eq!(
        number_text(&room.get(&id).await["rate"], "get"),
        "0.02512345678"
    );

    room.create(walk_in(
        "re4d-krw-2",
        "KRW",
        json!(100000),
        json!(0.0251234568),
    ))
    .await
    .assert_refused(
        400,
        RATE_MOVED,
        "a per-1 rate rounded to 10 places is another rate",
    );
    assert_eq!(room.bookings().await, 1, "the refusal wrote nothing");
    room.finish().await;
}

/// The ticket author's ruling 5 on #758: R5 applies when the booking is created; confirm compares
/// line identity only (R6). A line fresh at create, under a STOP policy, that goes stale before
/// confirm still confirms, and the row keeps that line.
#[actix_web::test]
async fn a_line_that_goes_stale_between_create_and_confirm_still_confirms() {
    let room = Room::open().await;
    let pool = room.pool();
    room.member().await;
    let stop = policy(&pool, MAIN, Some("STOP"), None).await;
    let usd = id_of(
        &publish_stale(&pool, MAIN, &stop, &[("USD", "1", "36.25", "36.75")]).await,
        "USD",
    );
    reference_age(&pool, &usd, 1).await;

    let id = saved(
        &room
            .create(for_member("re4d-fresh-1", "USD", json!(1000), json!(36.25)))
            .await,
        "fresh at create",
    );
    reference_age(&pool, &usd, 30).await;
    room.create(walk_in("re4d-stale-3", "USD", json!(1000), json!(36.25)))
        .await
        .assert_refused(
            400,
            RATE_MOVED,
            "the line is stale now: a new booking is refused",
        );

    let expires = room.desk.clock.now() + Duration::hours(2);
    let answer = room.confirm(&id, expires, json!(36.25)).await;
    assert_eq!(answer.status, 200, "confirm a stale line: {}", answer.text);
    assert_eq!(answer.json()["saved"], json!(true), "{}", answer.text);
    let row = room.row(&id).await;
    assert_eq!(row.status, "CONFIRMED");
    assert_eq!(row.published_rate_line_id, Some(usd), "the line of create");
    assert_eq!(row.rate, d("36.25"));
    room.finish().await;
}
