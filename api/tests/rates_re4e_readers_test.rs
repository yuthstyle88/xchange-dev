//! Ticket #759 (RE-4e) — the admin and report readers turn a stored exchange into baht with the
//! row's own `unit_size`, through `ExchangeArithmetic::thb`: the transaction row
//! (`dto/v3/transaction_row.rs`) as `admin_transactions`, `admin_slips`, `admin_overview` and
//! `admin_customer_history` answer it; the reports' reference (`views/admin_reports.rs`); the
//! cash reference spread (`views/admin_cash.rs`); the overview totals (`views/admin_overview.rs`);
//! the receipt preview (`pages.rs`, `GET /devices/receipt`); and the count line a cash save
//! leaves (`device_service.rs`, `COUNT_CONFIRMED`).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §3.2 "The one arithmetic" (`thb` = amount × rate ÷ unit_size, HALF_UP to 2 places once) and
//!   its table "Callers that change" (transaction row, admin cash, admin reports, devices
//!   COUNT_CONFIRMED, customer page: `amount × rate` → `thb`);
//! - §3.3 (THB rounded HALF_UP to 2 places once);
//! - §4.9 R1 (a row stores the line's rate per `unit_size`, the `unit_size` and the line id),
//!   R2 (every THB figure is `ExchangeArithmetic::thb(quantity, rate, unit_size)`), R9 (admin and
//!   report readers read each row's `unit_size`; a row with a NULL line id shows its own stored
//!   rate);
//! - §8 migration 5 (stored JPY/KRW rates are per `unit_size` after it; "stored principals do not
//!   change, because amount × (rate × u) ÷ u is the same THB"; "Readers": a NULL line id means
//!   "recorded before publications": the row shows its own stored rate);
//! - §9 RE-4e.
//!
//! The rows are put in `transactions` behind the routes, the way RE-4's save (R1) and migration 5
//! leave them, and read through the server's whole route table as the owner, with the harness of
//! `kiosk_http_common` (`Counter`: `MAIN` with an open shift). Rows with a line id carry a real
//! line of a publication made through the rate engine (`rates_re4_common::publish`).
//!
//! The day every reader is asked about (value date 2026-10-06, all cash, no fee, the open shift),
//! hand-computed HALF_UP:
//! - `RE4-JPY` SELL 1,000 JPY at 23.5 per 100 (the JPY line), reference 23.01 per 100:
//!   principal 235.00; reference value 230.10; spread (selling: principal − value) 4.90;
//!   percent 4.90 / 230.10 × 100 = 2.129509.
//! - `RE4-KRW` BUY 100,000 KRW at 25.12345678 per 1000 (the KRW line), reference 25 per 1000:
//!   principal 2512.345678 → 2512.35; value 2500.00; spread (buying: value − principal) −12.35;
//!   percent −0.494000.
//! - `RE4-USD` BUY 100 USD at 32.1 per 1 (the USD line), reference 32.5: principal 3210.00;
//!   value 3250.00; spread 40.00; percent 1.230769.
//! - `RE4-MID` BUY 1,000 KRW at 25.125 per 1000, a row recorded before publications (NULL line
//!   id), no reference: 25.125 is a midpoint, principal 25.13 HALF_UP (banker's rounding gives
//!   25.12; dividing by hand and printing three places gives 25.125).
//! - Reference over the day: 3 with, 1 without; value 5980.10; spread 32.55; percent 0.54.
//! - Overview totals: volume 5982.48; buy principal 5747.48; sell principal 235.00; received
//!   235.00; paid 5747.48; net −5512.48; buy share 96.07; sell share 3.93.
//!
//! Before migration 5 the same day was stored per 1 with `unit_size` 1: JPY 0.235 (reference
//! 0.2301), KRW 0.02512346 (reference 0.025; the column keeps 8 places, and 2512.346 → 2512.35 is
//! the same principal), USD unchanged, the midpoint 0.025125. Migration 5's own conversion (read
//! from the merged file, never retyped) turns them into the rows above; every figure must be the
//! same before and after it.
//!
//! The count line (`COUNT_CONFIRMED.thb`). Ticket author's ruling on #759: R2 applies, because a
//! banknote count is cash. The figure is `ExchangeArithmetic::thb`, HALF_UP to 2 places, equal to
//! the exchange's THB. The device service reads the stored row's `unit_size` by `document_ref`
//! on the save's own transaction (R9).
//! - `the_count_line_divides_by_the_rows_unit_size` compares numbers on products that need no
//!   rounding (1,000 JPY at 23.5 per 100 = 235; 100,000 KRW at 25.1 per 1000 = 2510).
//! - `the_count_line_is_baht_to_two_places_half_up` pins 235 and, for the midpoint 1,000 KRW at
//!   25.125 per 1000, 25.13: at most 2 places, each equal to the row's principal. The value is
//!   compared, not the text, because the device repository writes every amount normalized
//!   (`pg_device_repository.rs` `amounts_json`, outside RE-4e's scope), so 235.00 is stored as
//!   `235`.
//! - `a_save_with_no_stored_row_is_refused_and_leaves_no_line`: a missing row is an error
//!   (`AppError::Internal`), never unit 1, and the device step writes nothing (ruling on #759).
//! - `a_live_rpc_cash_save_of_jpy_counts_the_rows_baht` saves 1,007 JPY through `POST /api/rpc`
//!   `saveTransaction`, the real LIVE save. That proves the row exists when the device step runs.
//!   1,007 at 23.5 per 100 (0.235 per 1) is 236.645, a midpoint, so the count line is 236.65
//!   HALF_UP and equals the saved row's principal. Banker's rounding gives 236.64, and the exact
//!   product is 236.645.
//!
//! Rates are compared as numbers, not as text: how a row prints a rate per 100 is not decided by
//! this ticket.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); export `DATABASE_URL` the same.
//! Nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `xc_kernel::money::ExchangeArithmetic::thb(ForeignAmount, Rate, UnitSize) -> Money`
//!   (RU-1, merged) is the only arithmetic behind every THB figure below (R2);
//! - [required] `transactions.unit_size` (RU-1) and `transactions.published_rate_line_id`
//!   (migration 5, merged `d6ce058`): the unit the row's `rate` and `reference_rate` are per, and
//!   its line (NULL before RE-4);
//! - the view keys as merged: `TransactionRow` (`id`, `rate`, `principal`, `settlement`,
//!   `referenceValue`, `referenceSpread`, `referencePercent`); the reports' `reference`
//!   (`referenced`, `unreferenced`, `value`, `spread`, `percent`); the cash `reference`
//!   (`withReference`, `withoutReference`, `spread`, `percent`); the overview `totals`;
//! - the receipt preview's lines as merged (`devices_receipt_route_test.rs`): `เรท {rate}` and
//!   `{จ่าย|รับ} {thb} THB`;
//! - `DeviceService::new(BranchTx, Arc<dyn SavedExchanges>, Arc<dyn JobIds>)` and
//!   `DeviceService::after_save_in(tx, &BranchId, &SavedExchange, actor, now)` as merged, with
//!   `SavedExchange` unchanged. `device_commands.rs` is outside RE-4e's file scope, so the device
//!   service reads the unit from the exchange's stored row, by `document_ref`, on `tx`;
//! - the RPC action `saveTransaction` (`POST /api/rpc`) with v2's payload, as
//!   `configured_printer_models_test.rs` sends it.

mod common;
mod compat_common;
mod kiosk_http_common;
use kiosk_http_common::rates_re4_common;
mod rpc_common;
mod snapshot_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::Method;
use chrono::Utc;
use kiosk_http_common::{Counter, as_who, person};
use pos_error::{AppError, AppResult};
use rates_re4_common::{d, line, publish};
use rpc_common::{Answer, Call};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::ids::BranchId;
use xchange::infrastructure::devices::device_commands::SavedExchange;
use xchange::infrastructure::devices::device_service::DeviceService;
use xchange::infrastructure::devices::job_ids::UuidJobIds;
use xchange::infrastructure::devices::saved_exchanges::{ExchangeState, SavedExchanges};

const MAIN: &str = "MAIN";
const DAY: &str = "2026-10-06";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

/// Migration 5 as merged. Its conversion is the text between the `-- 2.` heading and `-- down:`.
const MIGRATION_5: &str = include_str!("../migrations/20261010125000_published_rate_line_ids.sql");

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
    /// The line it was priced on; `None` for a row recorded before publications.
    line: Option<String>,
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
    let (received, paid) = if ex.side == "SELL" {
        (ex.principal, "0")
    } else {
        ("0", ex.principal)
    };
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, \
            payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, \
            reference_rate, unit_size, published_rate_line_id, principal, fee, bank_fee, \
            received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
            value_date, paid_at, recorded_at, recorded_by, completed_at) \
         VALUES ($1, 'MAIN', 'LIVE', $2, $1, 'hash', $3, 'Somchai', 'THA', $4, $5, $6::numeric, \
            $7::numeric, $8::numeric, $9::numeric, $10::uuid, $11::numeric, 0, 0, \
            $12::numeric, $13::numeric, 'Cash', '', 'Cash drawer', 'owner', 'WALK-IN', '', \
            $14::date, now(), now(), 'owner', now())",
    )
    .bind(ex.id)
    .bind(shift)
    .bind(format!("CU-{}", ex.id))
    .bind(ex.side)
    .bind(ex.code)
    .bind(ex.amount)
    .bind(ex.rate)
    .bind(ex.reference)
    .bind(ex.unit_size)
    .bind(ex.line.as_deref())
    .bind(ex.principal)
    .bind(received)
    .bind(paid)
    .bind(DAY)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("put exchange {}: {e}", ex.id));
    seed_lines(pool, &[ex.id.to_owned()]).await;
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

/// The JPY, KRW and USD lines every row of the day with a line id is priced on.
async fn publish_the_day(pool: &PgPool) {
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
}

/// The day of the header, as RE-4 stores it: three rows on their lines, the midpoint row from
/// before publications. Publishes the lines first.
async fn the_day(pool: &PgPool) {
    publish_the_day(pool).await;
    let jpy = line(pool, MAIN, "JPY").await;
    let krw = line(pool, MAIN, "KRW").await;
    let usd = line(pool, MAIN, "USD").await;
    assert_eq!(jpy.unit_size, d("100"), "the register's JPY unit");
    assert_eq!(krw.unit_size, d("1000"), "the register's KRW unit");
    assert_eq!(jpy.sell, Some(d("23.5")));
    assert_eq!(krw.buy, Some(d("25.12345678")));
    assert_eq!(usd.buy, Some(d("32.1")));
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
            line: Some(jpy.id),
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
            line: Some(krw.id),
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
            line: Some(usd.id),
        },
        mid(),
    ] {
        put(pool, &ex).await;
    }
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
        line: None,
    }
}

/// The same day as the pre-RE-4 binary stored it: per 1, `unit_size` 1, no line.
async fn the_day_before_migration_5(pool: &PgPool) {
    for ex in [
        Ex {
            id: "RE4-JPY",
            side: "SELL",
            code: "JPY",
            amount: "1000",
            rate: "0.235",
            reference: Some("0.2301"),
            unit_size: "1",
            principal: "235.00",
            line: None,
        },
        Ex {
            id: "RE4-KRW",
            side: "BUY",
            code: "KRW",
            amount: "100000",
            rate: "0.02512346",
            reference: Some("0.025"),
            unit_size: "1",
            principal: "2512.35",
            line: None,
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
            line: None,
        },
        Ex {
            id: "RE4-MID",
            side: "BUY",
            code: "KRW",
            amount: "1000",
            rate: "0.025125",
            reference: None,
            unit_size: "1",
            principal: "25.13",
            line: None,
        },
    ] {
        put(pool, &ex).await;
    }
}

/// Applies migration 5's conversion, as merged, to the rows already stored.
async fn convert_as_migration_5(pool: &PgPool) {
    let start = MIGRATION_5
        .find("-- 2.")
        .expect("migration 5 has its conversion step");
    let end = MIGRATION_5
        .find("-- down:")
        .expect("migration 5 has a down note");
    // BK-1's M1 renames v2's `bookings` to `v2_bookings`; the conversion's booking step targets it there.
    let conversion = MIGRATION_5[start..end].replace("UPDATE bookings", "UPDATE v2_bookings");
    assert!(
        conversion.contains("UPDATE transactions"),
        "the conversion converts transactions: {conversion}"
    );
    sqlx::raw_sql(AssertSqlSafe(conversion))
        .execute(pool)
        .await
        .expect("apply migration 5's conversion");
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

fn number(value: &Value) -> Decimal {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    d(&text)
}

/// The figures of one transaction row that depend on the rate.
fn figures(r: &Value) -> Value {
    json!({
        "principal": r["principal"],
        "settlement": r["settlement"],
        "referenceValue": r["referenceValue"],
        "referenceSpread": r["referenceSpread"],
        "referencePercent": r["referencePercent"],
    })
}

fn expected_row(id: &str) -> Value {
    match id {
        "RE4-JPY" => json!({ "principal": "235.00", "settlement": "235.00",
            "referenceValue": "230.10", "referenceSpread": "4.90",
            "referencePercent": "2.129509" }),
        "RE4-KRW" => json!({ "principal": "2512.35", "settlement": "2512.35",
            "referenceValue": "2500.00", "referenceSpread": "-12.35",
            "referencePercent": "-0.494000" }),
        "RE4-USD" => json!({ "principal": "3210.00", "settlement": "3210.00",
            "referenceValue": "3250.00", "referenceSpread": "40.00",
            "referencePercent": "1.230769" }),
        "RE4-MID" => json!({ "principal": "25.13", "settlement": "25.13",
            "referenceValue": null, "referenceSpread": null, "referencePercent": null }),
        other => panic!("no expected row for {other}"),
    }
}

const IDS: [&str; 4] = ["RE4-JPY", "RE4-KRW", "RE4-USD", "RE4-MID"];

async fn transactions(counter: &Counter) -> Value {
    view(
        counter,
        &format!("/api/v3/views/admin_transactions?branch={MAIN}&from={DAY}&to={DAY}"),
    )
    .await
}

async fn reports_reference(counter: &Counter) -> Value {
    view(
        counter,
        &format!("/api/v3/views/admin_reports?branch={MAIN}&from={DAY}&to={DAY}"),
    )
    .await["reference"]
        .clone()
}

async fn overview_totals(counter: &Counter) -> Value {
    view(
        counter,
        &format!("/api/v3/views/admin_overview?branch={MAIN}&from={DAY}&to={DAY}"),
    )
    .await["totals"]
        .clone()
}

async fn cash_reference(counter: &Counter) -> Value {
    view(counter, &format!("/api/v3/views/admin_cash?branch={MAIN}")).await["reference"].clone()
}

async fn receipt(counter: &Counter, id: &str) -> String {
    let path = format!("/devices/receipt?branch={MAIN}&ref={id}");
    let answer = get_as_owner(counter, &path).await;
    assert_eq!(answer.status, 200, "GET {path}: {:.400}", answer.text);
    answer.text
}

fn expected_reports_reference() -> Value {
    json!({ "referenced": 3, "unreferenced": 1, "value": "5980.10", "spread": "32.55",
            "percent": "0.54" })
}

fn expected_cash_reference() -> Value {
    json!({ "withReference": 3, "withoutReference": 1, "spread": "32.55", "percent": "0.54" })
}

/// The totals of the overview that depend on the rate, and the counts.
fn totals_figures(t: &Value) -> Value {
    json!({
        "volume": t["volume"], "buyPrincipal": t["buyPrincipal"],
        "sellPrincipal": t["sellPrincipal"], "received": t["received"], "paid": t["paid"],
        "net": t["net"], "buyShare": t["buyShare"], "sellShare": t["sellShare"],
        "buyCount": t["buyCount"], "sellCount": t["sellCount"],
    })
}

fn expected_totals() -> Value {
    json!({
        "volume": "5982.48", "buyPrincipal": "5747.48", "sellPrincipal": "235.00",
        "received": "235.00", "paid": "5747.48", "net": "-5512.48", "buyShare": "96.07",
        "sellShare": "3.93", "buyCount": 3, "sellCount": 1,
    })
}

// ── the transaction row ──────────────────────────────────────────────────────

/// `transaction_row.rs` through `admin_transactions`: JPY 1,000 at 23.5 per 100 gives 235.00,
/// KRW per 1000 gives 2512.35, and each reference value divides by the row's own unit (R2, R9).
#[actix_web::test]
async fn the_transaction_row_divides_by_the_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    let data = transactions(&counter).await;
    for id in IDS {
        assert_eq!(figures(row(&data, id)), expected_row(id), "{id}: {data}");
    }
    // the row shows its own stored rate, per its unit (a rate of four places or fewer, so the
    // number is the same whichever width the row prints)
    assert_eq!(number(&row(&data, "RE4-JPY")["rate"]), d("23.5"), "{data}");
    assert_eq!(number(&row(&data, "RE4-USD")["rate"]), d("32.1"), "{data}");
    counter.finish().await;
}

/// The one HALF_UP midpoint: 1,000 KRW at 25.125 per 1000 = 25.125 THB exactly, which is
/// 25.13 HALF_UP (banker's rounding would give 25.12, three places 25.125).
#[actix_web::test]
async fn a_midpoint_principal_rounds_half_up_once() {
    let counter = Counter::open(&[]).await;
    put(&counter.desk.db.pool, &mid()).await;
    let data = transactions(&counter).await;
    let r = row(&data, "RE4-MID");
    assert_eq!(r["principal"], json!("25.13"), "{r}");
    assert_eq!(r["settlement"], json!("25.13"), "{r}");
    let page = receipt(&counter, "RE4-MID").await;
    assert!(
        page.contains("จ่าย 25.13 THB"),
        "the receipt pays 25.13 THB: {page}"
    );
    counter.finish().await;
}

/// The other views that answer `TransactionRow` read the row's `unit_size` in their own SELECT:
/// `admin_slips` (a row with a slip), `admin_customer_history` (a customer's rows) and the
/// overview's `latest`.
#[actix_web::test]
async fn slips_customer_history_and_the_overview_latest_divide_by_the_rows_unit_size() {
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
        assert_eq!(
            figures(row(&slips, id)),
            expected_row(id),
            "slips {id}: {slips}"
        );
    }

    for id in IDS {
        let history = view(
            &counter,
            &format!("/api/v3/views/admin_customer_history?branch={MAIN}&customer=CU-{id}"),
        )
        .await;
        assert_eq!(
            figures(row(&history, id)),
            expected_row(id),
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
            figures(row(&overview["latest"], id)),
            expected_row(id),
            "overview latest {id}: {overview}"
        );
    }
    counter.finish().await;
}

// ── a row from before publications ───────────────────────────────────────────

/// R9 / §8 migration 5 "Readers": a row with a NULL line id shows its own stored rate and its
/// THB from it, not the current publication's line for its currency (KRW is published at 26 per
/// 1000 to buy here; the row was bought at 25.125 per 1000).
#[actix_web::test]
async fn a_row_with_no_line_shows_its_own_stored_rate() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(&pool, MAIN, &[("KRW", "26", "27")]).await;
    put(&pool, &mid()).await;

    let data = transactions(&counter).await;
    let r = row(&data, "RE4-MID");
    assert_eq!(number(&r["rate"]), d("25.125"), "its own stored rate: {r}");
    assert_eq!(figures(r), expected_row("RE4-MID"), "{r}");

    let page = receipt(&counter, "RE4-MID").await;
    assert!(page.contains("เรท 25.125"), "its own stored rate: {page}");
    assert!(page.contains("จ่าย 25.13 THB"), "{page}");

    let still: Option<String> = sqlx::query_scalar(
        "SELECT published_rate_line_id::text FROM transactions WHERE document_ref = 'RE4-MID'",
    )
    .fetch_one(&pool)
    .await
    .expect("read the row");
    assert_eq!(still, None, "a reader links no line to an old row");
    counter.finish().await;
}

// ── reports, cash, overview, the receipt ─────────────────────────────────────

/// `admin_reports.rs`: the reference value and spread of the day divide by each row's unit.
#[actix_web::test]
async fn the_reports_reference_divides_by_each_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    assert_eq!(
        reports_reference(&counter).await,
        expected_reports_reference()
    );
    counter.finish().await;
}

/// `admin_cash.rs`: the open shift's reference spread divides by each row's unit.
#[actix_web::test]
async fn the_cash_reference_spread_divides_by_each_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    assert_eq!(cash_reference(&counter).await, expected_cash_reference());
    counter.finish().await;
}

/// `admin_overview.rs`: the totals sum each row's `thb`.
#[actix_web::test]
async fn the_overview_totals_divide_by_each_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    assert_eq!(
        totals_figures(&overview_totals(&counter).await),
        expected_totals()
    );
    counter.finish().await;
}

/// `pages.rs` (`GET /devices/receipt`): JPY 1,000 at 23.5 per 100 receives 235 THB, KRW 100,000
/// at 25.12345678 per 1000 pays 2,512.35 THB.
#[actix_web::test]
async fn the_receipt_preview_divides_by_the_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    the_day(&counter.desk.db.pool).await;
    let jpy = receipt(&counter, "RE4-JPY").await;
    assert!(jpy.contains("รับ 235 THB"), "{jpy}");
    let krw = receipt(&counter, "RE4-KRW").await;
    assert!(krw.contains("จ่าย 2,512.35 THB"), "{krw}");
    counter.finish().await;
}

// ── migration 5 leaves the day's baht where it was ───────────────────────────

/// §8 migration 5: "stored principals do not change, because amount × (rate × u) ÷ u is the same
/// THB". The day stored per 1 reads the same figures before migration 5's conversion and after
/// it, in every reader; and the conversion did convert (so the comparison is not vacuous).
#[actix_web::test]
async fn migration_5_leaves_the_thb_figures_of_a_mixed_day_unchanged() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    the_day_before_migration_5(&pool).await;

    let mut before = Vec::new();
    for _ in 0..2 {
        let data = transactions(&counter).await;
        let mut seen = serde_json::Map::new();
        for id in IDS {
            assert_eq!(figures(row(&data, id)), expected_row(id), "{id}: {data}");
            seen.insert(id.to_owned(), figures(row(&data, id)));
        }
        let reference = reports_reference(&counter).await;
        assert_eq!(reference, expected_reports_reference());
        let cash = cash_reference(&counter).await;
        assert_eq!(cash, expected_cash_reference());
        let totals = totals_figures(&overview_totals(&counter).await);
        assert_eq!(totals, expected_totals());
        let page = receipt(&counter, "RE4-JPY").await;
        assert!(page.contains("รับ 235 THB"), "{page}");
        before.push(json!({ "rows": seen, "reports": reference, "cash": cash,
                            "totals": totals }));

        if before.len() == 1 {
            convert_as_migration_5(&pool).await;
            let converted: Vec<(String, Decimal, Option<Decimal>, Decimal)> = sqlx::query_as(
                "SELECT document_ref, rate, reference_rate, unit_size FROM transactions \
                 ORDER BY document_ref",
            )
            .fetch_all(&pool)
            .await
            .expect("read the converted rows");
            assert_eq!(
                converted,
                vec![
                    ("RE4-JPY".into(), d("23.5"), Some(d("23.01")), d("100")),
                    ("RE4-KRW".into(), d("25.12346"), Some(d("25")), d("1000")),
                    ("RE4-MID".into(), d("25.125"), None, d("1000")),
                    ("RE4-USD".into(), d("32.1"), Some(d("32.5")), d("1")),
                ],
                "migration 5 converted the JPY and KRW rows"
            );
        }
    }
    assert_eq!(before[0], before[1], "the same day, before and after");
    counter.finish().await;
}

// ── the count line ───────────────────────────────────────────────────────────

/// No exchange table is asked here.
struct NoExchanges;

impl SavedExchanges for NoExchanges {
    fn find<'a>(
        &'a self,
        _branch: &'a BranchId,
        _reference: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<Option<ExchangeState>>> + Send + 'a>> {
        Box::pin(async { Ok(None) })
    }
}

/// `device_service.rs` `after_save_in`: a cash save's `COUNT_CONFIRMED` line values the foreign
/// cash at the row's rate per its unit: 1,000 JPY at 23.5 per 100 is 235, 100,000 KRW at 25.1
/// per 1000 is 2510, 100 USD at 32.1 is 3210 (compared as numbers).
#[actix_web::test]
async fn the_count_line_divides_by_the_rows_unit_size() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(
        &pool,
        MAIN,
        &[
            ("JPY", "23", "23.5"),
            ("KRW", "25.1", "26"),
            ("USD", "32.1", "32.6"),
        ],
    )
    .await;
    let cases = [
        (
            "RE4-DEV-JPY",
            "SELL",
            "JPY",
            "1000",
            "23.5",
            "100",
            "235.00",
            "235",
        ),
        (
            "RE4-DEV-KRW",
            "BUY",
            "KRW",
            "100000",
            "25.1",
            "1000",
            "2510.00",
            "2510",
        ),
        (
            "RE4-DEV-USD",
            "BUY",
            "USD",
            "100",
            "32.1",
            "1",
            "3210.00",
            "3210",
        ),
    ];
    let service = DeviceService::new(
        counter.desk.db.branch_tx(),
        Arc::new(NoExchanges),
        Arc::new(UuidJobIds),
    );
    for (id, side, code, amount, rate, unit, principal, thb) in cases {
        let side_line = line(&pool, MAIN, code).await;
        put(
            &pool,
            &Ex {
                id,
                side,
                code,
                amount,
                rate,
                reference: None,
                unit_size: unit,
                principal,
                line: Some(side_line.id),
            },
        )
        .await;
        let saved = SavedExchange {
            cash: true,
            submission_id: format!("sub-{id}"),
            document_ref: id.to_owned(),
            currency: code.to_owned(),
            amount: d(amount),
            rate: d(rate),
            passport_key: None,
            slip_key: None,
        };
        let main = BranchId::new(MAIN);
        let inner = main.clone();
        let svc = service.clone();
        counter
            .desk
            .db
            .branch_tx()
            .run(&main, move |tx| {
                Box::pin(async move {
                    svc.after_save_in(tx, &inner, &saved, "owner", Utc::now())
                        .await
                })
            })
            .await
            .unwrap_or_else(|e| panic!("the lines of the save {id}: {e:?}"));
        let line_thb: String = sqlx::query_scalar(
            "SELECT detail->>'thb' FROM device_events \
             WHERE kind = 'COUNT_CONFIRMED' AND ref = $1",
        )
        .bind(format!("sub-{id}"))
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("the count line of {id}: {e}"));
        assert_eq!(
            d(&line_thb),
            d(thb),
            "{id}: {amount} {code} at {rate} per {unit}"
        );
    }
    counter.finish().await;
}

/// The count line's baht is `ExchangeArithmetic::thb` (ruling on #759, R2): 2 places, HALF_UP at a
/// midpoint, equal to the row's principal. 1,000 JPY at 23.5 per 100 is 235. 1,000 KRW at 25.125
/// per 1000 is 25.125 exactly, which gives 25.13 (banker's rounding gives 25.12; the exact product
/// is 25.125, three places).
#[actix_web::test]
async fn the_count_line_is_baht_to_two_places_half_up() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(
        &pool,
        MAIN,
        &[("JPY", "23", "23.5"), ("KRW", "25.125", "26")],
    )
    .await;
    let service = DeviceService::new(
        counter.desk.db.branch_tx(),
        Arc::new(NoExchanges),
        Arc::new(UuidJobIds),
    );
    let cases = [
        (
            "RE4-CNT-JPY",
            "SELL",
            "JPY",
            "1000",
            "23.5",
            "100",
            "235.00",
        ),
        (
            "RE4-CNT-KRW",
            "BUY",
            "KRW",
            "1000",
            "25.125",
            "1000",
            "25.13",
        ),
    ];
    for (id, side, code, amount, rate, unit, thb) in cases {
        let side_line = line(&pool, MAIN, code).await;
        put(
            &pool,
            &Ex {
                id,
                side,
                code,
                amount,
                rate,
                reference: None,
                unit_size: unit,
                principal: thb,
                line: Some(side_line.id),
            },
        )
        .await;
        let saved = SavedExchange {
            cash: true,
            submission_id: format!("sub-{id}"),
            document_ref: id.to_owned(),
            currency: code.to_owned(),
            amount: d(amount),
            rate: d(rate),
            passport_key: None,
            slip_key: None,
        };
        let main = BranchId::new(MAIN);
        let inner = main.clone();
        let svc = service.clone();
        counter
            .desk
            .db
            .branch_tx()
            .run(&main, move |tx| {
                Box::pin(async move {
                    svc.after_save_in(tx, &inner, &saved, "owner", Utc::now())
                        .await
                })
            })
            .await
            .unwrap_or_else(|e| panic!("the lines of the save {id}: {e:?}"));
        let (line_thb, principal): (String, String) = sqlx::query_as(
            "SELECT e.detail->>'thb', t.principal::text FROM device_events e \
             JOIN transactions t ON t.document_ref = $2 \
             WHERE e.kind = 'COUNT_CONFIRMED' AND e.ref = $1",
        )
        .bind(format!("sub-{id}"))
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("the count line of {id}: {e}"));
        let counted = d(&line_thb);
        assert_eq!(
            counted,
            d(thb),
            "{id}: {amount} {code} at {rate} per {unit}"
        );
        assert!(counted.scale() <= 2, "{id}: at most 2 places: {line_thb}");
        assert_eq!(
            counted,
            d(&principal),
            "{id}: the count is the exchange's THB"
        );
    }
    counter.finish().await;
}

/// The real LIVE save (`POST /api/rpc` `saveTransaction`, cash, counted) of 1,007 JPY bought at
/// the counter rate. The device step runs on the save's transaction and finds the row just
/// written. 1,007 × 0.235 = 236.645 exactly, so its count line is 236.65 HALF_UP, the saved row's
/// principal, whether the row stores 0.235 per 1 (before RE-4c) or 23.5 per 100 (after RE-4c).
/// Banker's rounding gives 236.64, and the exact product is 236.645. The rate is the publication's
/// 23.5 / 25 per 100. (Re-expected for RE-7 #604 by the ticket author: before RE-7 it was also set
/// through `adminSaveRate`, so a pre-RE-4c save path priced it too; RE-7 removes that action and
/// the `rates` it wrote, and the publication is the only counter rate.)
#[actix_web::test]
async fn a_live_rpc_cash_save_of_jpy_counts_the_rows_baht() {
    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's; FRONT (TELLER under B1) saves
    let teller = person("FR1", "FRONT", 1);
    let counter = Counter::open(std::slice::from_ref(&teller)).await;
    let pool = counter.desk.db.pool.clone();
    publish(
        &pool,
        MAIN,
        &[("JPY", "23.5", "25"), ("USD", "32.1", "32.6")],
    )
    .await;
    sqlx::query(
        // CI-4 (#651): a customer belongs to the organisation
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'cust-re4e', organization_id, 'cust-re4e', 'Somchai Jaidee', 'LIVE', now() \
         FROM branches WHERE id = 'MAIN'",
    )
    .execute(&pool)
    .await
    .expect("insert the customer");
    // CI-4 (#651): the passport is the customer's encrypted identity (#554 §4.1)
    snapshot_common::seed_passport_identity(&pool, "cust-re4e", "AA1234567", "THA").await;
    let passport = counter
        .desk
        .files
        .upload(
            // C2: the teller who saves uploads the passport (a save attaches its saver's files)
            &BranchId::new(MAIN),
            &teller.id,
            "image/jpeg",
            JPEG,
            Utc::now(),
        )
        .await
        .expect("upload a passport");
    let submission = "00000000-0000-4000-8000-0000000e0002";
    let call = Call::action(
        &counter.origin,
        "/api/rpc",
        "saveTransaction",
        json!({
                "branchId": MAIN, "submissionId": submission, "customerId": "cust-re4e",
                "name": "Somchai Jaidee", "passportNo": "AA1234567", "nationality": "THA",
                "birthDate": "", "passportExpiry": "", "mode": "LIVE", "type": "BUY",
                "currency": "JPY", "amount": "1007", "rate": "0.235", "fee": "0",
                "bankFee": "0", "method": "Cash", "ref": "", "account": "Cash drawer 1",
                "staff": "desk", "booking": "WALK-IN", "notes": "", "valueDate": DAY,
                "paidAt": "2026-10-06T05:30:00.000Z", "paymentConfirmed": true,
                "identityConfirmed": true, "countConfirmed": true,
                "passport": { "uploadKey": passport },
        }),
    );
    // C2: FRONT saves
    let answer = counter
        .app
        .send(as_who(call, &counter.cookies, "FR1"))
        .await;
    assert_eq!(answer.status, 200, "saveTransaction: {}", answer.text);
    let answer = answer.json();
    assert_eq!(answer["saved"], json!(true), "{answer}");
    let (line_thb, principal): (String, String) = sqlx::query_as(
        "SELECT e.detail->>'thb', t.principal::text FROM device_events e \
         JOIN transactions t ON t.submission_id = e.ref \
         WHERE e.kind = 'COUNT_CONFIRMED' AND e.ref = $1",
    )
    .bind(submission)
    .fetch_one(&pool)
    .await
    .expect("the save's count line and its row");
    assert_eq!(principal, "236.65", "1,007 JPY at 23.5 per 100, HALF_UP");
    let counted = d(&line_thb);
    assert!(counted.scale() <= 2, "at most 2 places: {line_thb}");
    assert_eq!(counted, d(&principal), "the count is the saved row's baht");
    counter.finish().await;
}

/// Ruling on #759 (R9): the device service reads the stored row's `unit_size`. With no row for
/// the document the step is refused with `AppError::Internal`, never read as unit 1, and it
/// writes no line and no job. In the real save the row always exists
/// (`a_live_rpc_cash_save_of_jpy_counts_the_rows_baht`).
#[actix_web::test]
async fn a_save_with_no_stored_row_is_refused_and_leaves_no_line() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let service = DeviceService::new(
        counter.desk.db.branch_tx(),
        Arc::new(NoExchanges),
        Arc::new(UuidJobIds),
    );
    let saved = SavedExchange {
        cash: true,
        submission_id: "sub-RE4-NO-ROW".to_owned(),
        document_ref: "RE4-NO-ROW".to_owned(),
        currency: "USD".to_owned(),
        amount: d("100"),
        rate: d("32.1"),
        passport_key: None,
        slip_key: None,
    };
    let main = BranchId::new(MAIN);
    let inner = main.clone();
    let got = counter
        .desk
        .db
        .branch_tx()
        .run(&main, move |tx| {
            Box::pin(async move {
                service
                    .after_save_in(tx, &inner, &saved, "owner", Utc::now())
                    .await
            })
        })
        .await;
    assert!(
        matches!(got, Err(AppError::Internal(_))),
        "a missing row is an internal error: {got:?}"
    );
    let (events, jobs): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM device_events \
                  WHERE ref IN ('sub-RE4-NO-ROW', 'RE4-NO-ROW')), \
                (SELECT count(*) FROM device_jobs WHERE ref = 'RE4-NO-ROW')",
    )
    .fetch_one(&pool)
    .await
    .expect("count the device lines");
    assert_eq!((events, jobs), (0, 0), "no line and no job");
    counter.finish().await;
}

/// The `transaction_lines` + `transaction_payments` rows of the wide-only exchanges `refs`, as
/// TX-2's backfill writes them (migration 20261111170000 steps 7 and 8): an exchange seeded
/// straight into `transactions` would otherwise have no line, a shape that cannot exist after
/// TX-2 (#670), and readers moved onto the lines by TX-8 (#676) would not see it.
async fn seed_lines(pool: &sqlx::PgPool, refs: &[String]) {
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, quote_line_id, \
            shop_side, currency, denomination_id, quantity, unit_size, rate_snapshot, \
            reference_rate, thb_amount, fee_thb, settle_thb, cost_basis_thb, \
            realized_margin_thb, line_no, created_at, updated_at) \
         SELECT gen_random_uuid(), t.organization_id, t.id, NULL, t.type, t.code, NULL, \
            t.amount, t.unit_size, t.rate, t.reference_rate, t.principal, t.fee, \
            CASE t.type WHEN 'BUY' THEN t.paid ELSE t.received END, NULL, NULL, 1, \
            t.recorded_at, t.recorded_at \
         FROM transactions t WHERE t.document_ref = ANY($1)",
    )
    .bind(refs)
    .execute(pool)
    .await
    .expect("seed transaction_lines");
    sqlx::query(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, \
            method, reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
            created_at, updated_at) \
         SELECT gen_random_uuid(), t.organization_id, t.id, 1, t.method, t.payment_ref, \
            t.account, \
            (SELECT file_key FROM transaction_files f \
             WHERE f.transaction_id = t.id AND f.kind = 'SLIP' LIMIT 1), \
            CASE t.type WHEN 'BUY' THEN t.paid ELSE t.received END, t.bank_fee, t.paid_at, \
            t.recorded_at, t.recorded_at \
         FROM transactions t WHERE t.document_ref = ANY($1)",
    )
    .bind(refs)
    .execute(pool)
    .await
    .expect("seed transaction_payments");
}
