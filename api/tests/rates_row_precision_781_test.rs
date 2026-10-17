//! Ticket #781 — the admin transaction row (`api/src/presentation/dto/v3/transaction_row.rs`,
//! `TransactionRow`) sends each rate as the 8-place string the database holds, and the row's own
//! unit, instead of a rate cut to 4 places with no unit (`transaction_row.rs:256` and `:218` at
//! `27db80f5`: `format!("{:.4}", …)`, so KRW 25.12345678 per 1000 showed `25.1235`).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §3.3 "On the wire": a rate is the 8-place string the database holds (`numeric(20,8)`), e.g.
//!   `"23.50000000"`; one web formatter, `formatRate`, shows it with `/ <unit_size> <code>`;
//! - §4.9 R1 (storage and v3 views speak per `unit_size`), R3 (only the v2-compat wire is per 1;
//!   this row is not on that list), R9 (each row's own `unit_size`; a row with a NULL line id
//!   shows its own stored rate).
//!
//! The four views that answer `TransactionRow` are read through the server's whole route table as
//! the owner, with the harness of `kiosk_http_common` (`Counter`: `MAIN` with an open shift), the
//! way `rates_re4e_readers_test.rs` (#759) reads them: `GET /api/v3/views/admin_transactions`,
//! `admin_slips`, `admin_customer_history` and `admin_overview` (`latest`). The day stored is the
//! same as there (value date 2026-10-06, cash, no fee):
//! - `RE4-JPY` SELL 1,000 JPY at 23.5 per 100, reference 23.01 per 100: `rate` `"23.50000000"`,
//!   `referenceRate` `"23.01000000"`, `unitSize` `"100"`;
//! - `RE4-KRW` BUY 100,000 KRW at 25.12345678 per 1000, reference 25: `"25.12345678"`,
//!   `"25.00000000"`, `"1000"` (today `"25.1235"` and `"25.0000"`, no unit);
//! - `RE4-USD` BUY 100 USD at 32.1 per 1, reference 32.5: `"32.10000000"`, `"32.50000000"`,
//!   `"1"`;
//! - `RE4-MID` BUY 1,000 KRW at 25.125 per 1000, recorded before publications (NULL line id), no
//!   reference: `"25.12500000"`, `null`, `"1000"`.
//!
//! `unitSize` is a decimal string with no trailing zeros. The column is `numeric(20,4)`, so the
//! database's own text is `"1000.0000"`; the row sends `"1000"`, the way `published_rates.rs`
//! sends a line's `unitSize` (`normalize()`).
//!
//! Every other field of the row is unchanged: amount, principal, fee, bank fee, settlement and
//! the reference value, spread and percent are the figures RE-4e pinned.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); export `DATABASE_URL` the same.
//! Nothing here skips. The two `from_record` tests need no database.
//!
//! Signatures and fields this test assumes:
//! - [required] `TransactionRow` (serde camelCase) gains `unit_size: String` → `unitSize`, the
//!   row's `transactions.unit_size` normalized; `rate: String` and `reference_rate:
//!   Option<String>` → `rate`, `referenceRate` at exactly 8 places; no other key added, removed
//!   or renamed;
//! - [required] `TransactionRow::from_record(&TransactionRecord) -> TransactionRow` and the public
//!   fields of `TransactionRecord` (`rate: Rate`, `reference_rate: Option<Rate>`,
//!   `unit_size: UnitSize`, …) as merged at `27db80f5`;
//! - `Rate::parse` keeps at least 4 and at most 8 places (`rate.rs`), so `"32.10000000"` from the
//!   database parses to `32.1000`: the 8 places come from the row's print, not from the column's
//!   text;
//! - the harness and helpers as merged: `kiosk_http_common::Counter`, `rates_re4_common::{d,
//!   line, publish}`, `rpc_common::{Answer, Call}`.

mod common;
mod compat_common;
mod kiosk_http_common;
use kiosk_http_common::rates_re4_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;

use actix_web::http::Method;
use chrono::{NaiveDate, TimeZone, Utc};
use kiosk_http_common::Counter;
use rates_re4_common::{d, line, publish};
use rpc_common::{Answer, Call};
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Rate, UnitSize};
use xchange::presentation::dto::v3::transaction_row::{TransactionRecord, TransactionRow};

const MAIN: &str = "MAIN";
const DAY: &str = "2026-10-06";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

/// The keys of a row as merged at `27db80f5`, plus `unitSize`.
const ROW_KEYS: [&str; 26] = [
    "id",
    "date",
    "created",
    "type",
    "currency",
    "amount",
    "rate",
    "unitSize",
    "principal",
    "fee",
    "bankFee",
    "settlement",
    "method",
    "customer",
    "customerId",
    "staff",
    "status",
    "notes",
    "cancelReason",
    "shiftId",
    "referenceRate",
    "referenceValue",
    "referenceSpread",
    "referencePercent",
    "paymentRef",
    "slipUrl",
];

// ── helpers ──────────────────────────────────────────────────────────────────

fn get(path: &str) -> Call {
    let mut call = Call::post(kiosk_http_common::ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

async fn get_as_owner(counter: &Counter, path: &str) -> Answer {
    let call = get(path).cookie(
        counter
            .cookies
            .get("owner")
            .expect("the owner is signed in"),
    );
    counter.app.send(call).await
}

/// `data` of a view the owner reads.
async fn view(counter: &Counter, path: &str) -> Value {
    let answer = get_as_owner(counter, path).await;
    assert_eq!(answer.status, 200, "GET {path}: {:.400}", answer.text);
    let body = answer.json();
    body.get("data")
        .cloned()
        .unwrap_or_else(|| panic!("GET {path}: data in {body}"))
}

/// One stored exchange of `MAIN` on the open shift, valued [`DAY`], cash, no fee.
struct Ex {
    id: &'static str,
    side: &'static str,
    code: &'static str,
    amount: &'static str,
    rate: &'static str,
    reference: Option<&'static str>,
    unit_size: &'static str,
    principal: &'static str,
}

async fn put(pool: &PgPool, ex: &Ex) {
    let shift: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .fetch_one(pool)
            .await
            .expect("the counter's open shift");
    sqlx::query(
        // CI-4 (#651): a customer belongs to the organisation
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT $1, organization_id, $1, 'Somchai', 'LIVE', now() FROM branches WHERE id = 'MAIN' \
         ON CONFLICT DO NOTHING",
    )
    .bind(format!("CU-{}", ex.id))
    .execute(pool)
    .await
    .expect("insert a customer");
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
            customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
            recorded_by, completed_at) \
         VALUES ($1, 'MAIN', 'LIVE', $2, $1, 'hash', $3, 'Somchai', 'THA', 'owner', 'WALK-IN', '', \
            $4::date, now(), now(), 'owner', now())",
    )
    .bind(ex.id)
    .bind(shift)
    .bind(format!("CU-{}", ex.id))
    .bind(DAY)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("put exchange {}: {e}", ex.id));
    seed_lines(pool, ex).await;
}

/// A payment slip for `id`, so that `admin_slips` lists it.
async fn slip(counter: &Counter, id: &str) {
    let key = counter
        .desk
        .files
        .upload(
            &BranchId::new(MAIN),
            "owner",
            "image/jpeg",
            JPEG,
            Utc::now(),
        )
        .await
        .expect("upload a slip");
    sqlx::query(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) \
         SELECT id, 'SLIP', $2 FROM transactions WHERE document_ref = $1",
    )
    .bind(id)
    .bind(&key)
    .execute(&counter.desk.db.pool)
    .await
    .expect("link the slip");
    // the payment carries its slip, as TX-2's backfill and compat writer write it
    sqlx::query(
        "UPDATE transaction_payments p SET slip_file_key = $2 \
         FROM transactions t WHERE p.transaction_id = t.id AND t.document_ref = $1",
    )
    .bind(id)
    .bind(&key)
    .execute(&counter.desk.db.pool)
    .await
    .expect("put the slip on the payment");
}

/// The midpoint row: recorded before publications, per 1000 (as migration 5 left it).
fn mid() -> Ex {
    Ex {
        id: "RE4-MID",
        side: "BUY",
        code: "KRW",
        amount: "1000",
        rate: "25.125",
        reference: None,
        unit_size: "1000",
        principal: "25.13",
    }
}

/// The day of the header: three rows on their published lines, the midpoint row from before
/// publications.
async fn the_day(pool: &PgPool) {
    publish(
        pool,
        MAIN,
        &[
            ("JPY", "23", "23.5"),
            ("KRW", "25.12345678", "26"),
            ("USD", "32.1", "32.6"),
        ],
    )
    .await;
    let jpy = line(pool, MAIN, "JPY").await;
    let krw = line(pool, MAIN, "KRW").await;
    let usd = line(pool, MAIN, "USD").await;
    assert_eq!(jpy.unit_size, d("100"), "the register's JPY unit");
    assert_eq!(krw.unit_size, d("1000"), "the register's KRW unit");
    assert_eq!(usd.unit_size, d("1"), "the register's USD unit");
    for ex in [
        Ex {
            id: "RE4-JPY",
            side: "SELL",
            code: "JPY",
            amount: "1000",
            rate: "23.5",
            reference: Some("23.01"),
            unit_size: "100",
            principal: "235.00",
        },
        Ex {
            id: "RE4-KRW",
            side: "BUY",
            code: "KRW",
            amount: "100000",
            rate: "25.12345678",
            reference: Some("25"),
            unit_size: "1000",
            principal: "2512.35",
        },
        Ex {
            id: "RE4-USD",
            side: "BUY",
            code: "USD",
            amount: "100",
            rate: "32.1",
            reference: Some("32.5"),
            unit_size: "1",
            principal: "3210.00",
        },
        mid(),
    ] {
        put(pool, &ex).await;
    }
}

fn rows(data: &Value) -> &Vec<Value> {
    data.as_array()
        .or_else(|| data["rows"].as_array())
        .unwrap_or_else(|| panic!("a page of rows: {data}"))
}

fn row<'a>(data: &'a Value, id: &str) -> &'a Value {
    rows(data)
        .iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("{id} in {data}"))
}

/// The fields of one row this ticket changes, and the figures it must leave as they are.
fn shown(r: &Value) -> Value {
    json!({
        "currency": r["currency"],
        "amount": r["amount"],
        "rate": r["rate"],
        "unitSize": r["unitSize"],
        "referenceRate": r["referenceRate"],
        "principal": r["principal"],
        "fee": r["fee"],
        "bankFee": r["bankFee"],
        "settlement": r["settlement"],
        "referenceValue": r["referenceValue"],
        "referenceSpread": r["referenceSpread"],
        "referencePercent": r["referencePercent"],
    })
}

fn expected(id: &str) -> Value {
    match id {
        "RE4-JPY" => json!({
            "currency": "JPY", "amount": "1000.00",
            "rate": "23.50000000", "unitSize": "100", "referenceRate": "23.01000000",
            "principal": "235.00", "fee": "0.00", "bankFee": "0.00", "settlement": "235.00",
            "referenceValue": "230.10", "referenceSpread": "4.90",
            "referencePercent": "2.129509",
        }),
        "RE4-KRW" => json!({
            "currency": "KRW", "amount": "100000.00",
            "rate": "25.12345678", "unitSize": "1000", "referenceRate": "25.00000000",
            "principal": "2512.35", "fee": "0.00", "bankFee": "0.00", "settlement": "2512.35",
            "referenceValue": "2500.00", "referenceSpread": "-12.35",
            "referencePercent": "-0.494000",
        }),
        "RE4-USD" => json!({
            "currency": "USD", "amount": "100.00",
            "rate": "32.10000000", "unitSize": "1", "referenceRate": "32.50000000",
            "principal": "3210.00", "fee": "0.00", "bankFee": "0.00", "settlement": "3210.00",
            "referenceValue": "3250.00", "referenceSpread": "40.00",
            "referencePercent": "1.230769",
        }),
        "RE4-MID" => json!({
            "currency": "KRW", "amount": "1000.00",
            "rate": "25.12500000", "unitSize": "1000", "referenceRate": null,
            "principal": "25.13", "fee": "0.00", "bankFee": "0.00", "settlement": "25.13",
            "referenceValue": null, "referenceSpread": null, "referencePercent": null,
        }),
        other => panic!("no expected row for {other}"),
    }
}

const IDS: [&str; 4] = ["RE4-JPY", "RE4-KRW", "RE4-USD", "RE4-MID"];

fn keys(r: &Value) -> BTreeSet<String> {
    r.as_object()
        .unwrap_or_else(|| panic!("a row is an object: {r}"))
        .keys()
        .cloned()
        .collect()
}

fn row_keys() -> BTreeSet<String> {
    ROW_KEYS.iter().map(|k| k.to_string()).collect()
}

async fn transactions(counter: &Counter) -> Value {
    view(
        counter,
        &format!("/api/v3/views/admin_transactions?branch={MAIN}&from={DAY}&to={DAY}"),
    )
    .await
}

fn record(
    currency: &str,
    amount: &str,
    rate: &str,
    reference: Option<&str>,
    unit_size: &str,
) -> TransactionRecord {
    TransactionRecord {
        document_ref: format!("XC-781-{currency}"),
        recorded_at: Utc.with_ymd_and_hms(2026, 10, 6, 3, 4, 5).unwrap(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        side: "BUY".into(),
        currency: currency.into(),
        amount: d(amount),
        rate: Rate::parse(rate).expect("a valid rate"),
        reference_rate: reference.map(|r| Rate::parse(r).expect("a valid reference rate")),
        unit_size: UnitSize::parse(unit_size).expect("a valid unit size"),
        fee: d("0.00"),
        bank_fee: d("0.00"),
        method: "Cash".into(),
        payment_ref: String::new(),
        customer_id: "CU-781".into(),
        customer_name: "Somchai".into(),
        staff: "owner".into(),
        notes: String::new(),
        shift_id: None,
        slip_key: None,
    }
}

fn wire(record: &TransactionRecord) -> Value {
    serde_json::to_value(TransactionRow::from_record(record)).expect("a row serializes")
}

// ── the row, built from a record (no database) ───────────────────────────────

/// §3.3: KRW 25.12345678 per 1000 is sent as `"25.12345678"`, its reference 25 as
/// `"25.00000000"`, and the unit as `"1000"`, even when the record's unit carries the column's
/// four zero places (`numeric(20,4)` text `"1000.0000"`).
#[test]
fn a_row_sends_a_krw_rate_at_eight_places_with_its_unit_of_1000() {
    let r = wire(&record(
        "KRW",
        "100000",
        "25.12345678",
        Some("25.00000000"),
        "1000.0000",
    ));
    assert_eq!(r["rate"], json!("25.12345678"), "{r}");
    assert_eq!(r["referenceRate"], json!("25.00000000"), "{r}");
    assert_eq!(r["unitSize"], json!("1000"), "{r}");
    assert_eq!(r["principal"], json!("2512.35"), "{r}");
    assert_eq!(r["referenceValue"], json!("2500.00"), "{r}");
    assert_eq!(r["referenceSpread"], json!("-12.35"), "{r}");
    assert_eq!(r["referencePercent"], json!("-0.494000"), "{r}");
    assert_eq!(keys(&r), row_keys(), "{r}");
}

/// §3.3: a rate the database holds as `"32.10000000"` parses to `32.1000` (`Rate::parse` keeps at
/// least 4 places), and the row still sends all 8: `"32.10000000"`, unit `"1"`. No reference:
/// `referenceRate` is `null`.
#[test]
fn a_row_sends_a_usd_rate_at_eight_places_with_its_unit_of_1() {
    let r = wire(&record("USD", "100", "32.10000000", None, "1.0000"));
    assert_eq!(r["rate"], json!("32.10000000"), "{r}");
    assert_eq!(r["referenceRate"], Value::Null, "{r}");
    assert_eq!(r["unitSize"], json!("1"), "{r}");
    assert_eq!(r["principal"], json!("3210.00"), "{r}");
    assert_eq!(r["amount"], json!("100.00"), "{r}");
    assert_eq!(r["settlement"], json!("3210.00"), "{r}");
    assert_eq!(r["referenceValue"], Value::Null, "{r}");
    assert_eq!(keys(&r), row_keys(), "{r}");
}

// ── the views ────────────────────────────────────────────────────────────────

/// `admin_transactions`: each row's `rate` and `referenceRate` are the stored 8-place strings and
/// `unitSize` is the row's own unit; every other figure is RE-4e's.
#[actix_web::test]
async fn admin_transactions_rows_carry_the_stored_rate_and_the_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    let data = transactions(&counter).await;
    for id in IDS {
        let r = row(&data, id);
        assert_eq!(shown(r), expected(id), "{id}: {r}");
        assert_eq!(keys(r), row_keys(), "{id}: {r}");
    }
    counter.finish().await;
}

/// The other views that answer `TransactionRow`: `admin_slips` (rows with a slip),
/// `admin_customer_history` (a customer's rows) and the overview's `latest`.
#[actix_web::test]
async fn slips_customer_history_and_the_overview_latest_carry_the_stored_rate_and_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    slip(&counter, "RE4-JPY").await;
    slip(&counter, "RE4-KRW").await;

    let slips = view(
        &counter,
        &format!("/api/v3/views/admin_slips?branch={MAIN}&from={DAY}&to={DAY}"),
    )
    .await;
    for id in ["RE4-JPY", "RE4-KRW"] {
        assert_eq!(shown(row(&slips, id)), expected(id), "slips {id}: {slips}");
    }

    for id in IDS {
        let history = view(
            &counter,
            &format!("/api/v3/views/admin_customer_history?branch={MAIN}&customer=CU-{id}"),
        )
        .await;
        assert_eq!(
            shown(row(&history, id)),
            expected(id),
            "customer history {id}: {history}"
        );
    }

    let overview = view(
        &counter,
        &format!("/api/v3/views/admin_overview?branch={MAIN}&from={DAY}&to={DAY}"),
    )
    .await;
    for id in IDS {
        assert_eq!(
            shown(row(&overview["latest"], id)),
            expected(id),
            "overview latest {id}: {overview}"
        );
    }
    counter.finish().await;
}

/// R9: a row with a NULL line id shows its own stored rate and its own unit, never the current
/// publication's line nor the register's `currencies.unit_size`. KRW is published here at 26 per
/// 1000 to buy (register unit 1000). `RE4-MID` was bought at 25.125 per 1000; `RE4-OLD` is a KRW
/// row stored per 1 with unit 1 (the shape before migration 5), so its unit is `"1"` although the
/// register says 1000.
#[actix_web::test]
async fn a_row_with_no_line_shows_its_own_stored_rate_and_unit() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(&pool, MAIN, &[("KRW", "26", "27")]).await;
    assert_eq!(line(&pool, MAIN, "KRW").await.unit_size, d("1000"));
    put(&pool, &mid()).await;
    put(
        &pool,
        &Ex {
            id: "RE4-OLD",
            side: "BUY",
            code: "KRW",
            amount: "100000",
            rate: "0.02512346",
            reference: Some("0.025"),
            unit_size: "1",
            principal: "2512.35",
        },
    )
    .await;

    let data = transactions(&counter).await;
    assert_eq!(shown(row(&data, "RE4-MID")), expected("RE4-MID"), "{data}");
    let old = row(&data, "RE4-OLD");
    assert_eq!(
        shown(old),
        json!({
            "currency": "KRW", "amount": "100000.00",
            "rate": "0.02512346", "unitSize": "1", "referenceRate": "0.02500000",
            "principal": "2512.35", "fee": "0.00", "bankFee": "0.00", "settlement": "2512.35",
            "referenceValue": "2500.00", "referenceSpread": "-12.35",
            "referencePercent": "-0.494000",
        }),
        "{old}"
    );
    counter.finish().await;
}

/// The `transaction_lines` + `transaction_payments` rows of `ex` (TX-9 #677: the figures live on
/// the line and the payment only; `transactions` keeps the header). Cash, no fee: the settled THB
/// is the principal.
async fn seed_lines(pool: &sqlx::PgPool, ex: &Ex) {
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, quote_line_id, \
            shop_side, currency, denomination_id, quantity, unit_size, rate_snapshot, \
            reference_rate, thb_amount, fee_thb, settle_thb, cost_basis_thb, \
            realized_margin_thb, line_no, created_at, updated_at) \
         SELECT gen_random_uuid(), t.organization_id, t.id, NULL, $2, $3, NULL, $4::numeric, \
            $5::numeric, $6::numeric, $7::numeric, $8::numeric, 0, $8::numeric, NULL, NULL, 1, \
            t.recorded_at, t.recorded_at \
         FROM transactions t WHERE t.document_ref = $1",
    )
    .bind(ex.id)
    .bind(ex.side)
    .bind(ex.code)
    .bind(ex.amount)
    .bind(ex.unit_size)
    .bind(ex.rate)
    .bind(ex.reference)
    .bind(ex.principal)
    .execute(pool)
    .await
    .expect("seed transaction_lines");
    sqlx::query(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, \
            method, reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
            created_at, updated_at) \
         SELECT gen_random_uuid(), t.organization_id, t.id, 1, 'Cash', '', 'Cash drawer', NULL, \
            $2::numeric, 0, t.paid_at, t.recorded_at, t.recorded_at \
         FROM transactions t WHERE t.document_ref = $1",
    )
    .bind(ex.id)
    .bind(ex.principal)
    .execute(pool)
    .await
    .expect("seed transaction_payments");
}
