//! Ticket #758 (RE-4d) — the booking machine and `PgBookingRepository` on the published line.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §4.8: bookings read `line_in` and store `bookings.published_rate_line_id`, `rate`,
//!   `unit_size`; create/confirm compare line id;
//! - §4.9 R1 (a row stores the line's rate per `unit_size`, its `unit_size` and its id), R3 (the
//!   v2 booking wire is per 1: `CompatRate::per_one(rate, unit_size)`, exact), R4 (one check
//!   before a write, by line id), R6 (confirm compares the stored line with the current one and
//!   never re-locks; a moved line refuses with today's `RateMoved`);
//! - §8 migration 5 (rows written before RE-4 hold a NULL line id); §9 RE-4d.
//!
//! What is pinned here, below the HTTP route (`api/tests/rates_re4_bookings_test.rs` pins it
//! through the route):
//! - `create`: the counter's line is taken when the command's `expected_rate` (the v2 wire,
//!   per 1) is the line's per-1 rate; the per-unit rate on that wire is a moved rate; the
//!   booking's rate is the line's rate per unit.
//! - `confirm`: another line refuses even at the same rate; no current line, or no stored line,
//!   refuses; on the stored line the step confirms with the sent expiry.
//! - `redeem`: the kept v2 desk sends the booking's rate per 1 (R3), so the form matches a JPY
//!   booking at 0.231, not at 23.1.
//! - `PgBookingRepository`: `insert_in` stores the line id, the per-unit rate (8 places) and the
//!   unit; `find` reads them back; `confirm_in` rewrites none of them.
//!
//! The machine is pure: no clock of its own (`now` is given). The repository runs on a schema of
//! its own (`common/mod.rs`); the line a booking names is seeded with SQL, because the column is a
//! foreign key to `published_rate_lines`.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `bookings.published_rate_line_id`, `bookings.rate` (per unit), `bookings.unit_size`
//!   are written at create and never rewritten by confirm;
//! - [proposed] `xc_exchange::domain::booking::repositories::booking_ports::CounterLine
//!   { line_id: uuid::Uuid, rate: Rate, unit_size: UnitSize }` (Clone, Copy, Debug), the published
//!   side `BookingPorts::counter_rate_in(tx, branch, code, side, amount: Option<Money>)` answers
//!   (`Some` at create, whose THB R5 checks; `None` at confirm, ruling 5 on #758);
//! - [proposed] `CreateFacts.counter: Option<CounterLine>`;
//!   `BookingMachine::confirm(&Booking, &ConfirmBooking, Option<CounterLine>, DateTime<Utc>)`;
//!   `ConfirmStep::Confirm { expires_at, .. }`;
//! - [proposed] `Booking { published_rate_line_id: Option<uuid::Uuid>, unit_size: UnitSize, .. }`,
//!   `Booking.rate` per `unit_size`;
//! - [proposed] `BookingRepository::confirm_in(tx, &BranchId, id, expires_at, at, by)`: no rate;
//! - [proposed] `CreateBooking.expected_rate: Decimal`, `ConfirmBooking.expected_rate:
//!   Option<Decimal>`: the v2 wire's per-1 rate, up to 11 places (ruling 4 on #758).

// `common` re-exports the customer types its own tests name; this file names none of them.
#[allow(unused_imports)]
mod common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use common::{MAIN, TestDb, at, branch};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use xc_exchange::domain::booking::commands::confirm_booking::ConfirmBooking;
use xc_exchange::domain::booking::commands::create_booking::CreateBooking;
use xc_exchange::domain::booking::commands::redeem_booking::RedeemBooking;
use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::entities::booking_machine::BookingMachine;
use xc_exchange::domain::booking::entities::booking_steps::{ConfirmStep, CreateFacts, RedeemStep};
use xc_exchange::domain::booking::repositories::booking_ports::CounterLine;
use xc_exchange::domain::booking::repositories::booking_repository::BookingRepository;
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;

const RATE_MOVED: &str = "เรทเปลี่ยน กรุณาโหลดใหม่";
const FORM_MISMATCH: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";

const LINE_A: u128 = 0x0000_0000_0000_4000_8000_0000_0000_00a1;
const LINE_B: u128 = 0x0000_0000_0000_4000_8000_0000_0000_00b2;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 8, 3, 0, 0).unwrap()
}

fn tomorrow() -> DateTime<Utc> {
    now() + Duration::days(1)
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap()
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).unwrap()
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap()
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn counter(id: u128, r: &str, u: &str) -> CounterLine {
    CounterLine {
        line_id: Uuid::from_u128(id),
        rate: rate(r),
        unit_size: unit(u),
    }
}

/// The sentence of a refusal; panics when the step was taken.
fn refused<T>(got: Result<T, BookingError>) -> String {
    match got {
        Ok(_) => panic!("must be refused"),
        Err(e) => e.to_string(),
    }
}

fn taken<T>(got: Result<T, BookingError>) -> T {
    match got {
        Ok(v) => v,
        Err(e) => panic!("must be taken, was refused: {e}"),
    }
}

/// BUY 1,000 JPY at 23.1 per 100 on `line`, made an hour ago.
fn jpy_booking(
    status: BookingStatus,
    member: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
    line: Option<Uuid>,
) -> Booking {
    Booking {
        id: "BK-1".to_owned(),
        branch_id: branch(MAIN),
        name: "Somchai Jaidee".to_owned(),
        phone: "081-234-5678".to_owned(),
        member_id: member.map(str::to_owned),
        side: Side::Buy,
        currency: code("JPY"),
        amount: money("1000"),
        rate: rate("23.1"),
        unit_size: unit("100"),
        published_rate_line_id: line,
        status,
        created_at: now() - Duration::hours(1),
        created_by: "owner".to_owned(),
        expires_at,
        confirmed_at: None,
        confirmed_by: None,
        submission_id: None,
        transaction_ref: None,
    }
}

fn create_jpy(expected: &str) -> CreateBooking {
    CreateBooking {
        request_id: "REQ-1".to_owned(),
        name: "Somchai Jaidee".to_owned(),
        phone: "081-234-5678".to_owned(),
        side: Side::Buy,
        currency: code("JPY"),
        amount: money("1000"),
        expected_rate: dec(expected),
        customer_id: None,
    }
}

fn facts(line: Option<CounterLine>) -> CreateFacts {
    CreateFacts {
        rules: BookingRules {
            enabled: true,
            minutes: 30,
        },
        open_shift: true,
        counter: line,
        member_id: None,
    }
}

fn confirm_cmd(expected: &str) -> ConfirmBooking {
    ConfirmBooking {
        id: "BK-1".to_owned(),
        expires_at: Some(tomorrow()),
        expected_rate: Some(dec(expected)),
    }
}

// ── create (R1, R3) ──────────────────────────────────────────────────────────

/// R1 + R3: the screen's per-1 0.231 is the JPY line's 23.1 per 100, so the line is taken and
/// the booking's rate is the line's, per unit.
#[test]
fn create_takes_the_line_when_the_v2_rate_is_its_per_1_rate() {
    let line = counter(LINE_A, "23.1", "100");
    let made = taken(BookingMachine::create(
        &create_jpy("0.231"),
        &facts(Some(line)),
        now(),
    ));
    assert_eq!(made.status, BookingStatus::Confirmed);
    assert_eq!(made.rate, rate("23.1"), "per 100 JPY, the line's rate");
    assert_eq!(made.expires_at, Some(now() + Duration::minutes(30)));
}

/// R3: the per-unit 23.1 on the per-1 wire is not the counter's rate: a moved rate.
#[test]
fn create_refuses_the_per_unit_rate_on_the_v2_wire() {
    let line = counter(LINE_A, "23.1", "100");
    assert_eq!(
        refused(BookingMachine::create(
            &create_jpy("23.1"),
            &facts(Some(line)),
            now()
        )),
        RATE_MOVED
    );
    assert_eq!(
        refused(BookingMachine::create(
            &create_jpy("0.231"),
            &facts(None),
            now()
        )),
        RATE_MOVED,
        "no line for the currency and side"
    );
}

// ── confirm (R4, R6) ─────────────────────────────────────────────────────────

/// R6: a PENDING member booking made on line A. The current line B has the very same rate, and
/// confirm is still refused: the line id is compared, not the rate.
#[test]
fn confirm_refuses_another_line_even_at_the_same_rate() {
    let pending = jpy_booking(
        BookingStatus::Pending,
        Some("MB-1"),
        None,
        Some(Uuid::from_u128(LINE_A)),
    );
    for (label, current) in [
        (
            "another line, the same rate",
            Some(counter(LINE_B, "23.1", "100")),
        ),
        (
            "another line, another rate",
            Some(counter(LINE_B, "23.2", "100")),
        ),
        ("no current line", None),
    ] {
        assert_eq!(
            refused(BookingMachine::confirm(
                &pending,
                &confirm_cmd("0.231"),
                current,
                now()
            )),
            RATE_MOVED,
            "{label}"
        );
    }
}

/// R6: on the line stored at create, confirm is taken with the sent expiry. The step names no
/// new rate to write (whatever it carries, the stored rate is kept: the repository test below).
#[test]
fn confirm_on_the_stored_line_takes_the_sent_expiry() {
    let pending = jpy_booking(
        BookingStatus::Pending,
        Some("MB-1"),
        None,
        Some(Uuid::from_u128(LINE_A)),
    );
    match taken(BookingMachine::confirm(
        &pending,
        &confirm_cmd("0.231"),
        Some(counter(LINE_A, "23.1", "100")),
        now(),
    )) {
        ConfirmStep::Confirm { expires_at, .. } => assert_eq!(expires_at, tomorrow()),
        ConfirmStep::Unchanged { .. } => panic!("a PENDING member booking must be confirmed"),
    }
}

/// R4 + §8 migration 5: a booking written before RE-4 has no stored line; no current line is it.
#[test]
fn confirm_refuses_a_booking_with_no_stored_line() {
    let before_re4 = jpy_booking(BookingStatus::Pending, Some("MB-1"), None, None);
    assert_eq!(
        refused(BookingMachine::confirm(
            &before_re4,
            &confirm_cmd("0.231"),
            Some(counter(LINE_A, "23.1", "100")),
            now()
        )),
        RATE_MOVED
    );
}

// ── redeem (R3) ──────────────────────────────────────────────────────────────

/// R3: the kept v2 desk computes `amount × rate` per 1 and sends the booking's rate per 1. The
/// form of a JPY booking at 23.1 per 100 matches at 0.231; at 23.1 it does not.
#[test]
fn redeem_compares_the_desks_rate_per_1() {
    let booking = jpy_booking(
        BookingStatus::Confirmed,
        None,
        Some(tomorrow()),
        Some(Uuid::from_u128(LINE_A)),
    );
    let form = |r: &str| RedeemBooking {
        booking_id: "BK-1".to_owned(),
        submission_id: "S-1".to_owned(),
        live: true,
        side: Side::Buy,
        currency: code("JPY"),
        amount: money("1000"),
        // amended for #757 (RE-4c, rates.md §4.9 R7): the form's rate is the exact per-1 Decimal
        rate: Decimal::from_str(r).expect("a decimal"),
        form_booking: "BK-1".to_owned(),
        customer_id: "C-1".to_owned(),
        customer_confirmed: true,
    };
    assert!(matches!(
        taken(BookingMachine::redeem(
            &booking,
            None,
            &form("0.231"),
            now()
        )),
        RedeemStep::Use { .. }
    ));
    assert!(matches!(
        taken(BookingMachine::redeem(
            &booking,
            None,
            &form("0.2310"),
            now()
        )),
        RedeemStep::Use { .. }
    ));
    assert_eq!(
        refused(BookingMachine::redeem(&booking, None, &form("23.1"), now())),
        FORM_MISMATCH,
        "23.1 is per 100"
    );
}

// ── the repository (R1, R6) ──────────────────────────────────────────────────

/// One published line of `MAIN` (`code`, `unit`, buy and sell), in a publication of its own.
/// Answers its id.
async fn seed_line(db: &TestDb, currency: &str, unit_size: &str, buy: &str, sell: &str) -> Uuid {
    let request = Uuid::new_v4();
    let publication = Uuid::new_v4();
    let line = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1::uuid, organization_id, 'RATE_PUBLICATION', id, $2, 'test', '1', \
             '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $4",
    )
    .bind(request.to_string())
    .bind(publication.to_string())
    .bind(format!("re4d-{publication}"))
    .bind(MAIN)
    .execute(&db.pool)
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
    .bind(publication.to_string())
    .bind(request.to_string())
    .bind(MAIN)
    .execute(&db.pool)
    .await
    .expect("insert into rate_publications");
    sqlx::query(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT $1::uuid, organization_id, $2::uuid, $3, 'ALL', $4::numeric, $5::numeric, \
             $6::numeric, '{\"buy_mode\": \"MANUAL\", \"sell_mode\": \"MANUAL\"}'::jsonb \
         FROM rate_publications WHERE id = $2::uuid",
    )
    .bind(line.to_string())
    .bind(publication.to_string())
    .bind(currency)
    .bind(unit_size)
    .bind(buy)
    .bind(sell)
    .execute(&db.pool)
    .await
    .expect("insert into published_rate_lines");
    line
}

/// `(published_rate_line_id, rate, unit_size)` as stored.
async fn columns(db: &TestDb, id: &str) -> (Option<String>, Decimal, Decimal) {
    sqlx::query_as(
        "SELECT published_rate_line_id::text, rate, unit_size FROM bookings WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("booking {id}: {e}"))
}

/// R1: `insert_in` stores the line, its rate per 100 at 8 places and the unit; `find` reads them
/// back. R6: `confirm_in` sets the status, the expiry and who confirmed, and keeps the three.
#[tokio::test]
async fn insert_in_stores_the_line_and_confirm_in_keeps_it() {
    let db = TestDb::start().await;
    let repo = PgBookingRepository::new(db.pool.clone());
    let line = seed_line(&db, "JPY", "100", "23.12345678", "23.5").await;
    sqlx::query(
        // CI-4 (#651): a customer belongs to the organisation
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'C-1', organization_id, 'C-1', 'Somchai Jaidee', 'LIVE', $2 FROM branches WHERE id = $1",
    )
    .bind(MAIN)
    .bind(at(0))
    .execute(&db.pool)
    .await
    .expect("insert into customers");
    sqlx::query(
        "INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, \
             verified_by) \
         VALUES ('MB-1', $1, 'C-1', 'Somchai Jaidee', '081-234-5678', 'h', $2, 'owner')",
    )
    .bind(MAIN)
    .bind(at(0))
    .execute(&db.pool)
    .await
    .expect("insert into members");

    let pending = Booking {
        rate: rate("23.12345678"),
        ..jpy_booking(BookingStatus::Pending, Some("MB-1"), None, Some(line))
    };
    let mut tx = db.pool.begin().await.expect("begin");
    repo.insert_in(&mut tx, &pending, "REQ-1", "hash-1")
        .await
        .expect("insert_in");
    tx.commit().await.expect("commit");

    assert_eq!(
        columns(&db, "BK-1").await,
        (Some(line.to_string()), dec("23.12345678"), dec("100")),
        "the line, its rate per 100 and the unit (R1)"
    );
    let found = repo
        .find(&branch(MAIN), "BK-1")
        .await
        .expect("find")
        .expect("BK-1 is stored");
    assert_eq!(found.published_rate_line_id, Some(line));
    assert_eq!(found.rate, rate("23.12345678"));
    assert_eq!(found.unit_size, unit("100"));

    let mut tx = db.pool.begin().await.expect("begin");
    let changed = repo
        .confirm_in(
            &mut tx,
            &branch(MAIN),
            "BK-1",
            tomorrow(),
            at(60),
            "FRONT-2",
        )
        .await
        .expect("confirm_in");
    tx.commit().await.expect("commit");
    assert!(changed, "a PENDING booking is confirmed");
    assert_eq!(
        columns(&db, "BK-1").await,
        (Some(line.to_string()), dec("23.12345678"), dec("100")),
        "confirm rewrites none of the three (R6)"
    );
    let confirmed = repo
        .find(&branch(MAIN), "BK-1")
        .await
        .expect("find")
        .expect("BK-1 is stored");
    assert_eq!(confirmed.status, BookingStatus::Confirmed);
    assert_eq!(confirmed.expires_at, Some(tomorrow()));
    assert_eq!(confirmed.confirmed_by.as_deref(), Some("FRONT-2"));
    db.finish().await;
}
