//! Ticket #757 (RE-4c) — the desk save's LIVE rate check reads the current publication; the
//! save stores the line it was priced from, that line's rate per `unit_size` and the
//! `unit_size`; every THB figure is `amount × rate ÷ unit_size`, HALF_UP once.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Re-expected for RE-7 #604 by the ticket author: the three tests that also wrote a direct-save
//! `rates` row as a decoy no longer write it (rates.md §8 migration 6 drops the table); each
//! still proves the save refuses a rate that is not the current line's.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §3.2: the one arithmetic, `ExchangeArithmetic::thb` (HALF_UP to 2 places once, at the end);
//!   §3.3 rounding;
//! - §4.8: the desk save LIVE check (`save_transaction.rs:316-329`) reads `line_in`; a later
//!   publication never rewrites a stored row;
//! - §4.9 R1 (storage per unit), R2 (every THB figure through `ExchangeArithmetic::thb`),
//!   R3 (the v2 wire, the desk's `rate` included, is per 1), R4 (the v2 desk save is accepted
//!   only when the client's per-1 rate equals the current line's `per_one`, otherwise
//!   `RATE_MISMATCH`; the row then stores the current line), R7 (a save that completes a kiosk
//!   request stores the request's line and rate, not re-checked against a newer publication),
//!   R8 (`transaction_form` judges the fee on R2's figure);
//! - the ticket author's rulings for RE-4c of 2026-10-10 (from RE-4d's findings): R7 extends
//!   to bookings (a desk save that completes a booking stores the booking's line, rate and
//!   unit); a per-1 rate on the v2 wire carries up to 8 + log10(`unit_size`) places, so the
//!   desk save's client rate takes up to 11 decimals and `TOO_MANY_DECIMALS` starts at 12;
//! - the ticket author's further rulings of 2026-10-10: the receipt's "Booked Rate" prints the
//!   per-1 rate the desk sent, exactly (R3); the reference rate is sent per 1 (up to 11 places)
//!   and stored × `unit_size`, per unit, as migration 5 stores it; a kiosk-vouched or booked
//!   save whose client per-1 rate is not the request's or the booking's `per_one` is
//!   `RATE_MISMATCH` and writes nothing; R5 applies to the LIVE desk save (a stale AUTO line
//!   under STOP gets the save's "no rate" answer, `RATE_MISMATCH`);
//! - D-3: the current rate is the line of `branches.current_publication_id` (RE-7 drops
//!   `rates` and the direct save, §8 migration 6);
//! - §8 migration 5 (`transactions.published_rate_line_id`);
//! - §9 RE-4c.
//!
//! The save is driven through `SaveTransactionWorkflow` (the harness of `save_common`), a LIVE
//! exchange of the owner, or of an employee for the kiosk request. Publications are made
//! through the rate engine (`rates_re4_common::publish`). RU-1's register prices JPY per 100
//! and KRW per 1000. A DEMO save (`Mode::Demo`) is the owner's open question O-6 and is not
//! pinned here.
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] a LIVE save's row in `transactions` carries `published_rate_line_id` (migration
//!   5) = the line it was priced from, `rate` = that line's rate per `unit_size`, and
//!   `unit_size` = the line's `unit_size`;
//! - [required] `transactions.principal`, `received`, `paid` and the open shift's expected
//!   balances follow `ExchangeArithmetic::thb` (§3.2);
//! - [required] the refusal is today's sentence `RATE_MISMATCH` (`save_transaction.rs:38`),
//!   `FEE_ABOVE_PRINCIPAL` (`transaction_form.rs:19`) or `TOO_MANY_DECIMALS`
//!   (`transaction_form.rs:15`), as `AppError::Domain`; a refused save writes nothing
//!   (`Shop::everything`);
//! - [required] the save's client `rate` and `reference_rate` texts take up to 11 decimals;
//!   `transactions.reference_rate` = the client's per-1 reference × the line's `unit_size`;
//! - [required] the receipt's `<dt>Booked Rate</dt><dd>…</dd>` is the client's per-1 rate,
//!   unrounded, with at least four places;
//! - [required] `StalePolicy::admits` under the line's policy version (`source_snapshot.
//!   policy_version_id`) decides a LIVE desk save; a stale line is answered `RATE_MISMATCH`;
//! - [required] a booked save redeems the booking when the form's per-1 rate (up to 11 places)
//!   equals `CompatRate::per_one(booking.rate, booking.unit_size)` (`RedeemBooking` and the
//!   redeem comparison in `booking_machine.rs`, ruling of 2026-10-10);
//! - the merged signatures of `SaveTransactionWorkflow::{new, with_steps, save, save_as}`,
//!   `save_as_booked`, `KioskSaveStep::new`, `KioskService::new`, `PgKioskPorts::new`,
//!   `PgKioskAuditLog::new`, `PgDeviceRepository::new`, `BookingSaveStep::new`,
//!   `BookingService::new`, `PgBookingPorts::new`, `PgBookingRepository::new`,
//!   `PgMemberRepository::new`, `UuidBookingIds`, `FileService::open_owned`,
//!   `RateDraftService::{new, create, submit}`, `PublishService::{new, publish}`,
//!   `DraftLine::new`, `LineSide::{Auto, Manual}` and `make_approval_service`, as on the merge
//!   base `d6ce058`.

mod save_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use save_common::rates_re4c_common::{d, line, publish};
use save_common::*;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::application::booking::service::BookingService;
use xc_exchange::infrastructure::booking::booking_ids::UuidBookingIds;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, Side};
use xc_kiosk::application::kiosk::service::KioskService;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xchange::application::workflows::booking_save_step::BookingSaveStep;
use xchange::application::workflows::kiosk_save_step::KioskSaveStep;
use xchange::application::workflows::save_step::{BookingClaim, SaveActor, SaveExtras, SaveStep};
use xchange::infrastructure::bookings::booking_ports::PgBookingPorts;
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use xchange::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use xchange::presentation::http::v3::commands::make_approval_service;

const FEE_ABOVE_PRINCIPAL: &str = "ค่าบริการสูงกว่ายอดซื้อ";
const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
/// The booking step's refusal of a form that is not the booking's (`booking_save_step.rs`).
const NOT_THE_BOOKING: &str = "ข้อมูลรายการไม่ตรงกับการจอง หรือยังไม่ยืนยันลูกค้า";

/// A cash exchange of `amount` `currency` at the per-1 `rate` the v2 desk sends, LIVE, fee 0.
fn exchange(
    submission: &str,
    customer: &str,
    passport: &str,
    side: &str,
    currency: &str,
    amount: &str,
    rate: &str,
) -> SaveTransaction {
    let mut cmd = form(submission, customer, passport);
    cmd.side = side.to_owned();
    cmd.currency = currency.to_owned();
    cmd.amount = amount.to_owned();
    cmd.rate = rate.to_owned();
    cmd
}

/// `MAIN` with an open shift holding `opening`, the customer of `person()` and a passport file
/// of the owner. No rate is set: each test publishes its own. Answers (customer id, passport).
async fn counter(shop: &Shop, opening: &[(&str, &str)]) -> (String, String) {
    shop.open_shift(MAIN, opening).await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    (customer, passport)
}

/// One saved exchange as stored.
#[derive(Debug, PartialEq, Eq)]
struct Row {
    line_id: Option<String>,
    rate: Decimal,
    unit_size: Decimal,
    amount: Decimal,
    principal: Decimal,
    received: Decimal,
    paid: Decimal,
}

async fn row(shop: &Shop, document_ref: &str) -> Row {
    type Raw = (
        Option<String>,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
        Decimal,
    );
    let (line_id, rate, unit_size, amount, principal, received, paid): Raw = sqlx::query_as(
        "SELECT published_rate_line_id::text, rate, unit_size, amount, principal, received, paid \
         FROM transactions WHERE document_ref = $1",
    )
    .bind(document_ref)
    .fetch_one(&shop.pool)
    .await
    .unwrap_or_else(|e| panic!("transaction {document_ref}: {e}"));
    Row {
        line_id,
        rate,
        unit_size,
        amount,
        principal,
        received,
        paid,
    }
}

/// The open shift's expected balance of each code in `codes`, as text.
async fn till(shop: &Shop, codes: &[&str]) -> Vec<String> {
    let balances = shop.balances(MAIN).await.expect("the shift is open");
    codes
        .iter()
        .map(|c| {
            balances
                .get(*c)
                .cloned()
                .unwrap_or_else(|| "0.00".to_owned())
        })
        .collect()
}

/// §9 RE-4c: a JPY save at 23.5 per 100 for 1000 JPY gives principal 235.00 (not 23,500.00).
/// The v2 desk sends the per-1 rate it was shown (R3), 0.235; the row stores the line, 23.5 per
/// 100 (R1). The shop sells the yen: the till takes 235.00 THB and gives 1000 JPY.
#[tokio::test]
async fn a_jpy_save_at_23_5_per_100_for_1000_jpy_gives_principal_235() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("JPY", "5000")]).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    let jpy = line(&shop.pool, MAIN, "JPY").await;
    assert_eq!(jpy.unit_size, d("100"), "RU-1 registers JPY per 100");

    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-jpy-sell-000001",
                &customer,
                &passport,
                "SELL",
                "JPY",
                "1000",
                "0.235",
            ),
            at(0),
        )
        .await
        .expect("the save at the current JPY sell rate, per 1, is accepted");

    assert_eq!(
        row(&shop, &saved.document_ref).await,
        Row {
            line_id: Some(jpy.id.clone()),
            rate: d("23.5"),
            unit_size: d("100"),
            amount: d("1000"),
            principal: d("235.00"),
            received: d("235.00"),
            paid: d("0.00"),
        },
        "the line, its rate per 100, 1000 × 23.5 ÷ 100, SELL: principal + fee 0"
    );
    assert_eq!(till(&shop, &["THB", "JPY"]).await, ["100235.00", "4000.00"]);
    shop.finish().await;
}

/// §3.2/§3.3 with KRW per 1000: 100,000 KRW bought at 25.12345 per 1000 (per 1: 0.02512345) is
/// 2512.345 THB. HALF_UP once gives 2512.35 paid out; banker's rounding would give 2512.34.
#[tokio::test]
async fn a_krw_buy_is_priced_per_1000_and_rounded_half_up_once() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("KRW", "25.12345", "26")]).await;
    let krw = line(&shop.pool, MAIN, "KRW").await;
    assert_eq!(krw.unit_size, d("1000"), "RU-1 registers KRW per 1000");

    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-krw-buy-0000001",
                &customer,
                &passport,
                "BUY",
                "KRW",
                "100000",
                "0.02512345",
            ),
            at(0),
        )
        .await
        .expect("the save at the current KRW buy rate, per 1, is accepted");

    assert_eq!(
        row(&shop, &saved.document_ref).await,
        Row {
            line_id: Some(krw.id.clone()),
            rate: d("25.12345"),
            unit_size: d("1000"),
            amount: d("100000"),
            principal: d("2512.35"),
            received: d("0.00"),
            paid: d("2512.35"),
        },
        "100000 × 25.12345 ÷ 1000 = 2512.345, HALF_UP; BUY: principal − fee 0"
    );
    assert_eq!(
        till(&shop, &["THB", "KRW"]).await,
        ["97487.65", "100000.00"]
    );
    shop.finish().await;
}

/// §3.3: 15 JPY at 23.1 per 100 (per 1: 0.231) is 3.465 THB: 3.47 HALF_UP (banker's: 3.46).
#[tokio::test]
async fn a_jpy_principal_at_a_midpoint_is_rounded_half_up() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;

    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-jpy-half-up-001",
                &customer,
                &passport,
                "BUY",
                "JPY",
                "15",
                "0.231",
            ),
            at(0),
        )
        .await
        .expect("the save is accepted");
    let stored = row(&shop, &saved.document_ref).await;
    assert_eq!(
        (stored.rate, stored.unit_size, stored.principal, stored.paid),
        (d("23.1"), d("100"), d("3.47"), d("3.47"))
    );
    assert_eq!(till(&shop, &["THB", "JPY"]).await, ["99996.53", "15.00"]);
    shop.finish().await;
}

/// R3/R4: the v2 desk speaks per 1. The line's own per-100 figure, 23.5, sent as the rate is
/// not the current line's `per_one` (0.235), so it is refused and nothing is written.
#[tokio::test]
async fn a_rate_per_unit_sent_by_the_desk_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("JPY", "5000")]).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-jpy-per-unit-01",
                &customer,
                &passport,
                "SELL",
                "JPY",
                "1000",
                "23.5",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    shop.finish().await;
}

/// R4: the side's own rate. A BUY at the line's per-1 SELL rate is refused.
#[tokio::test]
async fn a_buy_at_the_sell_rate_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("JPY", "5000")]).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-jpy-wrong-side1",
                &customer,
                &passport,
                "BUY",
                "JPY",
                "1000",
                "0.235",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    shop.finish().await;
}

/// §9 RE-4c `RATE_MISMATCH` (R4): the client quoted version 1's USD 32.1; version 2 says 32.2.
/// The save at 32.1 is refused and writes nothing; at 32.2 it is accepted and stores version
/// 2's line.
#[tokio::test]
async fn a_rate_from_a_publication_that_is_no_longer_current_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("USD", "1000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.2", "33")]).await;
    let usd = line(&shop.pool, MAIN, "USD").await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-moved-00001",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.1",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );

    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-moved-00002",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.2",
            ),
            at(1),
        )
        .await
        .expect("the save at the current rate is accepted");
    assert_eq!(
        row(&shop, &saved.document_ref).await,
        Row {
            line_id: Some(usd.id.clone()),
            rate: d("32.2"),
            unit_size: d("1"),
            amount: d("100"),
            principal: d("3220.00"),
            received: d("0.00"),
            paid: d("3220.00"),
        }
    );
    shop.finish().await;
}

/// D-3: the counter rate is the publication's 32.5. A save at 32.1 (before RE-7, the newer
/// direct save's rate) is refused; at 32.5 it is accepted.
#[tokio::test]
async fn the_save_checks_the_publication_not_the_direct_save() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("USD", "1000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.5", "33")]).await;
    let usd = line(&shop.pool, MAIN, "USD").await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-direct-0001",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.1",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );

    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-direct-0002",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.5",
            ),
            at(1),
        )
        .await
        .expect("the save at the published rate is accepted");
    let stored = row(&shop, &saved.document_ref).await;
    assert_eq!(stored.line_id.as_deref(), Some(usd.id.as_str()));
    assert_eq!(stored.rate, d("32.5"));
    shop.finish().await;
}

/// D-3 (§9 RE-4c): with no publication the currency has no current line, so a LIVE save is
/// refused `RATE_MISMATCH` and writes nothing.
#[tokio::test]
async fn without_a_publication_a_live_save_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("USD", "1000")]).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-nopub-00001",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.1",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    shop.finish().await;
}

/// D-3: a currency the current publication does not list has no counter rate.
#[tokio::test]
async fn a_currency_the_publication_does_not_list_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("EUR", "1000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-eur-unlisted-01",
                &customer,
                &passport,
                "BUY",
                "EUR",
                "100",
                "35.5",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    shop.finish().await;
}

/// §4.8 / R4: the stored line is the current line, also when a republish keeps the rate; and a
/// later publication never rewrites a stored row.
#[tokio::test]
async fn a_save_stores_the_current_line_and_keeps_it() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("USD", "1000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let first = line(&shop.pool, MAIN, "USD").await;
    let one = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-keep-000001",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.1",
            ),
            at(0),
        )
        .await
        .expect("the first save is accepted");

    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let second = line(&shop.pool, MAIN, "USD").await;
    assert_ne!(first.id, second.id, "a republish makes a new line");
    let two = shop
        .save(
            MAIN,
            exchange(
                "re4c-usd-keep-000002",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.1",
            ),
            at(1),
        )
        .await
        .expect("the second save is accepted");

    publish(&shop.pool, MAIN, &[("USD", "40", "41")]).await;
    let kept_one = row(&shop, &one.document_ref).await;
    let kept_two = row(&shop, &two.document_ref).await;
    assert_eq!(
        (kept_one.line_id.as_deref(), kept_one.rate),
        (Some(first.id.as_str()), d("32.1")),
        "kept after versions 2 and 3"
    );
    assert_eq!(
        (kept_two.line_id.as_deref(), kept_two.rate),
        (Some(second.id.as_str()), d("32.1")),
        "the line current at the save, kept after version 3"
    );
    shop.finish().await;
}

/// R4 "one check before a write": a repeat of a saved submission writes nothing, so a
/// publication since the first sending does not refuse it. It is answered from the stored
/// exchange, which keeps its line.
#[tokio::test]
async fn a_repeat_after_a_newer_publication_is_answered_from_the_stored_exchange() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("USD", "1000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let first = line(&shop.pool, MAIN, "USD").await;
    let cmd = exchange(
        "re4c-usd-repeat-0001",
        &customer,
        &passport,
        "BUY",
        "USD",
        "100",
        "32.1",
    );
    let saved = shop
        .save(MAIN, cmd.clone(), at(0))
        .await
        .expect("the first sending is accepted");

    publish(&shop.pool, MAIN, &[("USD", "32.2", "33")]).await;
    let before = shop.everything().await;
    let again = shop
        .save(MAIN, cmd, at(5))
        .await
        .expect("the repeat is answered");
    assert_eq!(again, saved, "the same exchange");
    assert_eq!(shop.everything().await, before, "the repeat wrote nothing");
    assert_eq!(
        row(&shop, &saved.document_ref).await.line_id.as_deref(),
        Some(first.id.as_str())
    );
    shop.finish().await;
}

/// Ruling of 2026-10-10 (R3: a per-1 rate carries up to 8 + log10(`unit_size`) places): a KRW
/// line at 25.12345678 per 1000 is 0.02512345678 per 1, eleven places. The desk sends that, the
/// save is accepted, and the row stores 25.12345678 with unit 1000. 100,000 KRW is
/// 2512.345678 THB, 2512.35 paid out.
#[tokio::test]
async fn a_krw_line_of_eight_places_is_saved_at_its_eleven_place_per_1_rate() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("KRW", "25.12345678", "26")]).await;
    let krw = line(&shop.pool, MAIN, "KRW").await;

    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-krw-11-places-1",
                &customer,
                &passport,
                "BUY",
                "KRW",
                "100000",
                "0.02512345678",
            ),
            at(0),
        )
        .await
        .expect("an eleven-place per-1 rate equal to the line's is accepted");
    assert_eq!(
        row(&shop, &saved.document_ref).await,
        Row {
            line_id: Some(krw.id.clone()),
            rate: d("25.12345678"),
            unit_size: d("1000"),
            amount: d("100000"),
            principal: d("2512.35"),
            received: d("0.00"),
            paid: d("2512.35"),
        }
    );
    assert_eq!(
        till(&shop, &["THB", "KRW"]).await,
        ["97487.65", "100000.00"]
    );
    shop.finish().await;
}

/// Ruling of 2026-10-10: `TOO_MANY_DECIMALS` starts at 12 places. A client rate of twelve
/// places is refused with it, before the rate is compared, and nothing is written; eleven
/// places that are not the line's are `RATE_MISMATCH`.
#[tokio::test]
async fn a_client_rate_of_twelve_places_is_too_many_decimals() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("KRW", "25.12345678", "26")]).await;

    let before = shop.everything().await;
    for (submission, rate, sentence) in [
        ("re4c-krw-12-places-1", "0.025123456781", TOO_MANY_DECIMALS),
        ("re4c-krw-11-places-2", "0.02512345679", RATE_MISMATCH),
    ] {
        let refused = shop
            .save(
                MAIN,
                exchange(
                    submission, &customer, &passport, "BUY", "KRW", "100000", rate,
                ),
                at(0),
            )
            .await;
        assert_eq!(refusal(refused), sentence, "{rate}");
    }
    assert_eq!(
        shop.everything().await,
        before,
        "the refused saves wrote nothing"
    );
    shop.finish().await;
}

/// The `Booked Rate` cell of a saved exchange's receipt.
async fn booked_rate(shop: &Shop, saved: &SavedTransaction) -> String {
    let key = saved
        .receipt_url
        .strip_prefix("/files/")
        .expect("a file address");
    let receipt = shop
        .files
        .open_owned(key, OWNER)
        .await
        .expect("read the receipt")
        .expect("the receipt is a stored file");
    let html = String::from_utf8(receipt.bytes).expect("a receipt is text");
    let (_, after) = html
        .split_once("<dt>Booked Rate</dt><dd>")
        .unwrap_or_else(|| panic!("no Booked Rate in {html}"));
    after
        .split_once("</dd>")
        .expect("the cell ends")
        .0
        .to_owned()
}

/// Ruling of 2026-10-10 (R3): the receipt's "Booked Rate" is the per-1 rate the desk sent,
/// exactly, in v2's print format of at least four places (`32.1` as `32.1000`, pinned by
/// `transaction_form_test`): KRW 0.02512345678 (eleven places, not rounded to eight), JPY
/// `0.2350` (not 23.5 per 100).
#[tokio::test]
async fn the_receipt_books_the_per_1_rate_the_desk_sent() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("JPY", "5000")]).await;
    publish(
        &shop.pool,
        MAIN,
        &[("KRW", "25.12345678", "26"), ("JPY", "23.1", "23.5")],
    )
    .await;

    let krw = shop
        .save(
            MAIN,
            exchange(
                "re4c-receipt-krw-001",
                &customer,
                &passport,
                "BUY",
                "KRW",
                "100000",
                "0.02512345678",
            ),
            at(0),
        )
        .await
        .expect("the KRW save is accepted");
    assert_eq!(booked_rate(&shop, &krw).await, "0.02512345678");

    let jpy = shop
        .save(
            MAIN,
            exchange(
                "re4c-receipt-jpy-001",
                &customer,
                &passport,
                "SELL",
                "JPY",
                "1000",
                "0.235",
            ),
            at(1),
        )
        .await
        .expect("the JPY save is accepted");
    assert_eq!(
        booked_rate(&shop, &jpy).await,
        "0.2350",
        "the per-1 JPY rate, not 23.5"
    );
    shop.finish().await;
}

/// Ruling of 2026-10-10: the reference rate travels like the rate. The desk sends JPY's
/// reference per 1, 0.230; the row stores it per 100, 23.0, beside the rate 23.5 per 100.
#[tokio::test]
async fn a_reference_rate_sent_per_1_is_stored_per_unit() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000"), ("JPY", "5000")]).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;

    let mut cmd = exchange(
        "re4c-reference-jpy-1",
        &customer,
        &passport,
        "SELL",
        "JPY",
        "1000",
        "0.235",
    );
    cmd.reference_rate = Some("0.230".to_owned());
    let saved = shop
        .save(MAIN, cmd, at(0))
        .await
        .expect("the save is accepted");
    let (rate, reference, unit_size): (Decimal, Option<Decimal>, Decimal) = sqlx::query_as(
        "SELECT rate, reference_rate, unit_size FROM transactions WHERE document_ref = $1",
    )
    .bind(&saved.document_ref)
    .fetch_one(&shop.pool)
    .await
    .expect("read the exchange");
    assert_eq!(
        (rate, reference, unit_size),
        (d("23.5"), Some(d("23.0")), d("100")),
        "both per 100"
    );
    shop.finish().await;
}

/// Publish `lines` (AUTO or MANUAL) as the next publication of `branch`, as the owner, through
/// the rate engine, as `rates_re4_common::publish` does for MANUAL lines.
async fn publish_lines(shop: &Shop, branch_id: &str, lines: Vec<DraftLine>) {
    // OR-7.2 #1021: `create`, `submit` and `publish` take the owner's B1-mapped user, EXECUTIVE + ADMIN organisation-wide.
    let owner_user = xc_auth::domain::access::Actor::new(
        "owner",
        "00000000-0000-0000-0000-000000000001",
        vec![
            xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Executive,
                None,
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                None,
            ),
            xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Admin,
                None,
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                None,
            ),
        ],
        None,
    );
    let b = BranchId::new(branch_id);
    let drafts = RateDraftService::new(shop.pool.clone(), make_approval_service(shop.pool.clone()));
    let publisher = PublishService::new(shop.pool.clone());
    let mut tx = shop.pool.begin().await.expect("begin");
    let draft = drafts
        .create(&mut tx, &b, &owner_user, lines)
        .await
        .unwrap_or_else(|e| panic!("create the draft: {}", e.code()));
    tx.commit().await.expect("commit the draft");
    let mut tx = shop.pool.begin().await.expect("begin");
    drafts
        .submit(&mut tx, &b, &owner_user, &draft.id, &unique("re4c-submit"))
        .await
        .unwrap_or_else(|e| panic!("submit the draft: {}", e.code()));
    tx.commit().await.expect("commit the submit");
    let expected: Option<i32> = sqlx::query_scalar(
        "SELECT p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch_id)
    .fetch_optional(&shop.pool)
    .await
    .expect("read the current version");
    let mut tx = shop.pool.begin().await.expect("begin");
    publisher
        .publish(&mut tx, &b, &owner_user, &draft.id, expected)
        .await
        .unwrap_or_else(|e| panic!("publish the draft: {}", e.code()));
    tx.commit().await.expect("commit the publish");
}

/// R5 (ruling of 2026-10-10): the branch's policy keeps a reference for one hour and stops on a
/// stale one. USD is published AUTO from a Bank of Thailand reference fetched three hours ago,
/// so its line is stale: the save at its rate gets the save's "no rate" answer,
/// `RATE_MISMATCH`, and writes nothing. EUR in the same publication is MANUAL, never stale,
/// and saves.
#[tokio::test]
async fn a_stale_auto_line_under_stop_is_refused_and_a_manual_line_saves() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    let batch: String = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches \
           (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), 'BOT', (now() - interval '3 hours')::date, \
                 now() - interval '3 hours', $1, 'COMPLETE') RETURNING id::text",
    )
    .bind(unique("checksum"))
    .fetch_one(&shop.pool)
    .await
    .expect("insert a reference batch");
    sqlx::query(
        "INSERT INTO rate_reference_lines \
           (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1::uuid, 'USD', 'BOT', 1, 32.1, 32.45, '{}'::jsonb)",
    )
    .bind(&batch)
    .execute(&shop.pool)
    .await
    .expect("insert a reference line");
    sqlx::query(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
            approved_by, reason, source_version) \
         VALUES (gen_random_uuid(), $1, 1, 0, 0, '{}'::jsonb, 1, 'STOP', NULL, \
                 now() - interval '1 day', NULL, 'owner', 'seeded by the RE-4c test', NULL)",
    )
    .bind(MAIN)
    .execute(&shop.pool)
    .await
    .expect("insert a policy version");
    let manual = |text: &str| LineSide::Manual(Rate::parse(text).expect("a rate"));
    let code = |text: &str| CurrencyCode::parse(text).expect("a currency");
    publish_lines(
        &shop,
        MAIN,
        vec![
            DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None),
            DraftLine::new(
                code("EUR"),
                manual("35.5"),
                manual("36.25"),
                Some("set by hand".to_owned()),
            ),
        ],
    )
    .await;
    let usd = line(&shop.pool, MAIN, "USD").await;
    assert_eq!(
        usd.buy,
        Some(d("32.1")),
        "AUTO from the reference, no adjustment"
    );

    let before = shop.everything().await;
    let refused = shop
        .save(
            MAIN,
            exchange(
                "re4c-stale-usd-00001",
                &customer,
                &passport,
                "BUY",
                "USD",
                "100",
                "32.1",
            ),
            at(0),
        )
        .await;
    assert_eq!(refusal(refused), RATE_MISMATCH, "a stale line under STOP");
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );

    let eur = line(&shop.pool, MAIN, "EUR").await;
    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-stale-eur-00001",
                &customer,
                &passport,
                "BUY",
                "EUR",
                "100",
                "35.5",
            ),
            at(1),
        )
        .await
        .expect("the MANUAL line of the same publication saves");
    assert_eq!(
        row(&shop, &saved.document_ref).await.line_id.as_deref(),
        Some(eur.id.as_str())
    );
    shop.finish().await;
}

/// R8: the fee is judged on R2's figure. 1000 JPY bought at 23.5 per 100 (per 1: 0.235) is
/// 235.00 THB: a fee of 235.01 is above it and refused, writing nothing; a fee of 235.00 is
/// accepted and leaves 0.00 to pay out. (A figure taken as `amount × rate per 100` would be
/// 23,500.00 and accept both.)
#[tokio::test]
async fn the_fee_is_judged_on_the_per_unit_figure() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.5", "24")]).await;
    let jpy = line(&shop.pool, MAIN, "JPY").await;

    let mut above = exchange(
        "re4c-jpy-fee-above01",
        &customer,
        &passport,
        "BUY",
        "JPY",
        "1000",
        "0.235",
    );
    above.fee = "235.01".to_owned();
    let before = shop.everything().await;
    assert_eq!(
        refusal(shop.save(MAIN, above, at(0)).await),
        FEE_ABOVE_PRINCIPAL
    );
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );

    let mut equal = exchange(
        "re4c-jpy-fee-equal01",
        &customer,
        &passport,
        "BUY",
        "JPY",
        "1000",
        "0.235",
    );
    equal.fee = "235".to_owned();
    let saved = shop
        .save(MAIN, equal, at(1))
        .await
        .expect("a fee equal to the principal is accepted");
    assert_eq!(
        row(&shop, &saved.document_ref).await,
        Row {
            line_id: Some(jpy.id.clone()),
            rate: d("23.5"),
            unit_size: d("100"),
            amount: d("1000"),
            principal: d("235.00"),
            received: d("0.00"),
            paid: d("0.00"),
        }
    );
    assert_eq!(till(&shop, &["THB", "JPY"]).await, ["100000.00", "1000.00"]);
    shop.finish().await;
}

// ------------------------------------------------------- the kiosk-vouched save (R7)

/// The employee holding the kiosk request: an id, as the kiosk stores it.
const FRONT: &str = "employee-7";

/// A reference-rate source that is never asked here.
struct NoBank;

impl RateSource for NoBank {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        Box::pin(async { panic!("the bank is not asked by a save") })
    }
}

/// The save workflow with the kiosk's step, as `saveTransaction` registers it, on the real
/// kiosk service of this shop.
fn kiosk_flow(shop: &Shop) -> SaveTransactionWorkflow {
    let branch_tx = BranchTx::new(Transactor::new(shop.pool.clone()));
    let audit: Arc<dyn KioskAuditLog> = Arc::new(PgKioskAuditLog::new(PgDeviceRepository::new(
        shop.pool.clone(),
    )));
    let ports = Arc::new(PgKioskPorts::new(
        shop.pool.clone(),
        audit,
        Arc::new(NoBank),
    ));
    let kiosk = KioskService::new(branch_tx.clone(), ports);
    let steps: Vec<Arc<dyn SaveStep>> = vec![Arc::new(KioskSaveStep::new(kiosk))];
    SaveTransactionWorkflow::new(branch_tx, shop.shifts.clone(), shop.files.clone())
        .with_steps(steps)
}

/// A kiosk request `id` that `FRONT` accepted and whose passport they checked: the customer
/// buys `amount` JPY in cash, quoted from `line_id` at `rate` per `unit_size` for `thb`. RE-4b
/// writes these three columns at the kiosk quote (§4.9 R1); the row is written here so that
/// this test does not depend on the kiosk's quote.
async fn checked_request(
    shop: &Shop,
    id: &str,
    amount: &str,
    (line_id, rate, unit_size): (&str, &str, &str),
    thb: &str,
) {
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
         rate, foreign_amount, thb, input_amount, payment_method, client_ip, unit_size, \
         published_rate_line_id) \
         VALUES ($1, $2, 'ID_CHECKED', now(), $3, 'BUY', 'JPY', $4::numeric, $5::numeric, \
         $6::numeric, $5::numeric, 'Cash', '203.0.113.7', $7::numeric, $8::uuid)",
    )
    .bind(MAIN)
    .bind(id)
    .bind(FRONT)
    .bind(rate)
    .bind(amount)
    .bind(thb)
    .bind(unit_size)
    .bind(line_id)
    .execute(&shop.pool)
    .await
    .expect("insert a checked kiosk request");
}

/// Re-pinned for TX-6 by the test-writer, A3 (kiosk) (design quote-transaction.md §4.6.1 A3 and D-5; lane
/// controller's ruling on the republish, #674), replacing R7 (§9 RE-4c, ruling of 2026-10-10): a kiosk save uses
/// the request's own quote, and a quote is valid only while its publication is the branch's current one. The
/// customer was quoted version 1's JPY 23.1 per 100 at the kiosk (through the quote service, channel KIOSK, as
/// TX-7 asks); version 2 says 23.3 since. The save at the request's per-1 rate 0.231 is refused `quoteExpired`,
/// writes nothing, and the request stays `ID_CHECKED`.
#[tokio::test]
async fn a_kiosk_save_after_a_republish_is_quote_expired() {
    let shop = Shop::start().await;
    shop.open_shift(MAIN, &[("THB", "100000")]).await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    let quoted = line(&shop.pool, MAIN, "JPY").await;
    checked_request(
        &shop,
        "q-re4c-1",
        "10000",
        (quoted.id.as_str(), "23.1", "100"),
        "2310.00",
    )
    .await;
    quote_kiosk_request(
        &shop.pool,
        MAIN,
        "q-re4c-1",
        "JPY",
        Side::Buy,
        "10000",
        at(30),
    )
    .await;
    publish(&shop.pool, MAIN, &[("JPY", "23.3", "23.7")]).await;
    let current = line(&shop.pool, MAIN, "JPY").await;
    assert_ne!(current.id, quoted.id);

    let front = SaveActor {
        id: FRONT.to_owned(),
        username: "front7".to_owned(),
        employee_id: Some(FRONT.to_owned()),
        role: Role::Front,
    };
    let extras = SaveExtras {
        kiosk_request_id: Some("q-re4c-1".to_owned()),
        ..SaveExtras::default()
    };
    let before = shop.everything().await;
    let refused = tokio::time::timeout(
        MUST_FINISH,
        kiosk_flow(&shop).save_as(
            &branch(MAIN),
            exchange(
                "re4c-kiosk-jpy-00001",
                &customer,
                &passport,
                "BUY",
                "JPY",
                "10000",
                "0.231",
            ),
            &extras,
            &front,
            at(60),
        ),
    )
    .await
    .expect("a save finishes");
    let sentence = refusal(refused);
    assert!(
        sentence.starts_with("quoteExpired"),
        "decision A3 (kiosk) with D-5: {sentence}"
    );
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    let (status, document): (String, Option<String>) = sqlx::query_as(
        "SELECT status, document_ref FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
    )
    .bind(MAIN)
    .bind("q-re4c-1")
    .fetch_one(&shop.pool)
    .await
    .expect("read the request");
    assert_eq!((status.as_str(), document.as_deref()), ("ID_CHECKED", None));
    shop.finish().await;
}

/// Ruling of 2026-10-10 (R7): a save completing a kiosk request at a per-1 rate that is not
/// the request's `per_one` is `RATE_MISMATCH` and writes nothing — even at the current
/// publication's rate (0.233 per 1 where the request was quoted 0.231). The request stays
/// checked.
#[tokio::test]
async fn a_kiosk_vouched_save_at_another_rate_is_refused() {
    let shop = Shop::start().await;
    shop.open_shift(MAIN, &[("THB", "100000")]).await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    let quoted = line(&shop.pool, MAIN, "JPY").await;
    checked_request(
        &shop,
        "q-re4c-2",
        "10000",
        (quoted.id.as_str(), "23.1", "100"),
        "2310.00",
    )
    .await;
    publish(&shop.pool, MAIN, &[("JPY", "23.3", "23.7")]).await;

    let front = SaveActor {
        id: FRONT.to_owned(),
        username: "front7".to_owned(),
        employee_id: Some(FRONT.to_owned()),
        role: Role::Front,
    };
    let extras = SaveExtras {
        kiosk_request_id: Some("q-re4c-2".to_owned()),
        ..SaveExtras::default()
    };
    let before = shop.everything().await;
    let refused = tokio::time::timeout(
        MUST_FINISH,
        kiosk_flow(&shop).save_as(
            &branch(MAIN),
            exchange(
                "re4c-kiosk-jpy-00002",
                &customer,
                &passport,
                "BUY",
                "JPY",
                "10000",
                "0.233",
            ),
            &extras,
            &front,
            at(60),
        ),
    )
    .await
    .expect("a save finishes");
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    let status: String =
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE branch_id = $1 AND id = $2")
            .bind(MAIN)
            .bind("q-re4c-2")
            .fetch_one(&shop.pool)
            .await
            .expect("read the request");
    assert_eq!(status, "ID_CHECKED");
    shop.finish().await;
}

// ------------------------------------------------------- the booked save (R7, bookings)

/// The booking save step on the real booking service of this shop, as `saveTransaction`
/// registers it.
fn booking_flow(shop: &Shop) -> SaveTransactionWorkflow {
    let branch_tx = BranchTx::new(Transactor::new(shop.pool.clone()));
    let bookings = BookingService::new(
        branch_tx.clone(),
        Arc::new(PgBookingRepository::new(shop.pool.clone())),
        Arc::new(PgMemberRepository::new(shop.pool.clone())),
        Arc::new(PgBookingPorts::new(shop.pool.clone())),
        Arc::new(UuidBookingIds),
    );
    let steps: Vec<Arc<dyn SaveStep>> = vec![Arc::new(BookingSaveStep::new(bookings))];
    SaveTransactionWorkflow::new(branch_tx, shop.shifts.clone(), shop.files.clone())
        .with_steps(steps)
}

/// A confirmed walk-in booking `id` of 100 USD bought at `rate`, quoted from `line_id` per 1.
/// RE-4d writes the line and unit at booking create (§4.9 R1); the row is written here so that
/// this test does not depend on the booking's create.
async fn confirmed_booking(shop: &Shop, id: &str, line_id: &str, rate: &str) {
    sqlx::query(
        "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, type, code, \
         amount, rate, status, created_at, created_by, expires_at, confirmed_at, confirmed_by, \
         unit_size, published_rate_line_id) \
         VALUES ($1, $2, $3, 'hash', 'Walk In', '0800000000', 'BUY', 'USD', 100, $4::numeric, \
         'CONFIRMED', '2026-10-06T00:00:00Z', 'owner', '2026-10-07T00:00:00Z', \
         '2026-10-06T00:00:00Z', 'owner', 1, $5::uuid)",
    )
    .bind(id)
    .bind(MAIN)
    .bind(unique("booking-request"))
    .bind(rate)
    .bind(line_id)
    .execute(&shop.pool)
    .await
    .expect("insert a confirmed booking");
}

/// Re-expected for TX-6 by the test-writer, decision A3 (design quote-transaction.md §4.6.1 and §6.4, #674; the client-rate check at quote comes first, lane controller's ruling issuecomment-6116917556), replacing the ruling of 2026-10-10 (R7 extended to
/// bookings): a booking no longer sets the rate; a booked save is quoted at the current published
/// line. The customer booked version 1's USD 32.1; version 2 says 32.2 since. The save at the booked
/// 32.1 is refused `RATE_MISMATCH`, writes nothing, and the booking stays `CONFIRMED`.
#[tokio::test]
async fn a_booked_save_at_the_booked_rate_off_the_current_line_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let booked = line(&shop.pool, MAIN, "USD").await;
    confirmed_booking(&shop, "BK-re4c-0001", &booked.id, "32.1").await;
    publish(&shop.pool, MAIN, &[("USD", "32.2", "33")]).await;
    assert_ne!(line(&shop.pool, MAIN, "USD").await.id, booked.id);

    let mut cmd = exchange(
        "re4c-booked-usd-0001",
        &customer,
        &passport,
        "BUY",
        "USD",
        "100",
        "32.1",
    );
    cmd.booking = "BK-re4c-0001".to_owned();
    let owner = SaveActor {
        id: OWNER.to_owned(),
        username: OWNER.to_owned(),
        employee_id: None,
        role: Role::Owner,
    };
    let claim = BookingClaim {
        booking_id: "BK-re4c-0001".to_owned(),
        customer_confirmed: true,
    };
    let before = shop.everything().await;
    let refused = tokio::time::timeout(
        MUST_FINISH,
        booking_flow(&shop).save_as_booked(
            &branch(MAIN),
            cmd,
            &SaveExtras::default(),
            Some(claim),
            &owner,
            at(60),
        ),
    )
    .await
    .expect("a save finishes");
    assert_eq!(refusal(refused), RATE_MISMATCH, "decision A3");
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    let status: String = sqlx::query_scalar("SELECT status FROM v2_bookings WHERE id = $1")
        .bind("BK-re4c-0001")
        .fetch_one(&shop.pool)
        .await
        .expect("read the booking");
    assert_eq!(status, "CONFIRMED");
    shop.finish().await;
}

/// Re-expected for TX-6 by the test-writer, decision A3 (design quote-transaction.md §4.6.1 and §6.4, #674; the client-rate check at quote comes first, lane controller's ruling issuecomment-6116917556), replacing the ruling of 2026-10-10 (R7 for bookings):
/// a save completing a booking at the current publication's rate (32.2 where the booking holds 32.1)
/// passes the quote, and the booking step refuses it as not the booking's form. It writes nothing,
/// and the booking stays confirmed.
#[tokio::test]
async fn a_booked_save_at_another_rate_is_refused() {
    let shop = Shop::start().await;
    let (customer, passport) = counter(&shop, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("USD", "32.1", "33")]).await;
    let booked = line(&shop.pool, MAIN, "USD").await;
    confirmed_booking(&shop, "BK-re4c-0002", &booked.id, "32.1").await;
    publish(&shop.pool, MAIN, &[("USD", "32.2", "33")]).await;

    let mut cmd = exchange(
        "re4c-booked-usd-0002",
        &customer,
        &passport,
        "BUY",
        "USD",
        "100",
        "32.2",
    );
    cmd.booking = "BK-re4c-0002".to_owned();
    let owner = SaveActor {
        id: OWNER.to_owned(),
        username: OWNER.to_owned(),
        employee_id: None,
        role: Role::Owner,
    };
    let claim = BookingClaim {
        booking_id: "BK-re4c-0002".to_owned(),
        customer_confirmed: true,
    };
    let before = shop.everything().await;
    let refused = tokio::time::timeout(
        MUST_FINISH,
        booking_flow(&shop).save_as_booked(
            &branch(MAIN),
            cmd,
            &SaveExtras::default(),
            Some(claim),
            &owner,
            at(60),
        ),
    )
    .await
    .expect("a save finishes");
    assert_eq!(
        refusal(refused),
        NOT_THE_BOOKING,
        "decision A3: the booking step refuses it"
    );
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    let status: String = sqlx::query_scalar("SELECT status FROM v2_bookings WHERE id = $1")
        .bind("BK-re4c-0002")
        .fetch_one(&shop.pool)
        .await
        .expect("read the booking");
    assert_eq!(status, "CONFIRMED");
    shop.finish().await;
}

/// A confirmed walk-in booking `id` of 100,000 KRW bought at `rate` per 1000, quoted from
/// `line_id`, written as RE-4d's create writes it (§4.9 R1).
async fn confirmed_krw_booking(shop: &Shop, id: &str, line_id: &str, rate: &str) {
    sqlx::query(
        "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, type, code, \
         amount, rate, status, created_at, created_by, expires_at, confirmed_at, confirmed_by, \
         unit_size, published_rate_line_id) \
         VALUES ($1, $2, $3, 'hash', 'Walk In', '0800000000', 'BUY', 'KRW', 100000, \
         $4::numeric, 'CONFIRMED', '2026-10-06T00:00:00Z', 'owner', '2026-10-07T00:00:00Z', \
         '2026-10-06T00:00:00Z', 'owner', 1000, $5::uuid)",
    )
    .bind(id)
    .bind(MAIN)
    .bind(unique("booking-request"))
    .bind(rate)
    .bind(line_id)
    .execute(&shop.pool)
    .await
    .expect("insert a confirmed KRW booking");
}

/// The booked desk save of 100,000 KRW at the per-1 `rate`, completing booking `id`.
async fn booked_krw_save(
    shop: &Shop,
    (customer, passport): (&str, &str),
    id: &str,
    submission: &str,
    rate: &str,
) -> AppResult<SavedTransaction> {
    let mut cmd = exchange(submission, customer, passport, "BUY", "KRW", "100000", rate);
    cmd.booking = id.to_owned();
    let owner = SaveActor {
        id: OWNER.to_owned(),
        username: OWNER.to_owned(),
        employee_id: None,
        role: Role::Owner,
    };
    let claim = BookingClaim {
        booking_id: id.to_owned(),
        customer_confirmed: true,
    };
    tokio::time::timeout(
        MUST_FINISH,
        booking_flow(shop).save_as_booked(
            &branch(MAIN),
            cmd,
            &SaveExtras::default(),
            Some(claim),
            &owner,
            at(60),
        ),
    )
    .await
    .expect("a save finishes")
}

/// The customer of `person()` and a passport file of the owner, in `MAIN`.
async fn counter_prepared(shop: &Shop) -> (String, String) {
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    (customer, passport)
}

/// The status of booking `id`.
async fn booking_status(shop: &Shop, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM v2_bookings WHERE id = $1")
        .bind(id)
        .fetch_one(&shop.pool)
        .await
        .expect("read the booking")
}

/// Ruling of 2026-10-10 (R3, R7): a booking on a KRW line of 25.12345678 per 1000 has the
/// per-1 rate 0.02512345678, eleven places. The booked desk save at that rate redeems the
/// booking (it ends `USED`, the schema's redeemed state) and stores the booking's line,
/// 25.12345678 and unit 1000; 100,000 KRW is 2512.35 THB.
#[tokio::test]
async fn a_booked_save_redeems_a_krw_booking_at_its_11_place_per_1_rate() {
    let shop = Shop::start().await;
    shop.open_shift(MAIN, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("KRW", "25.12345678", "26")]).await;
    let booked = line(&shop.pool, MAIN, "KRW").await;
    confirmed_krw_booking(&shop, "BK-re4c-krw-01", &booked.id, "25.12345678").await;

    let (customer, passport) = counter_prepared(&shop).await;
    let saved = booked_krw_save(
        &shop,
        (&customer, &passport),
        "BK-re4c-krw-01",
        "re4c-booked-krw-0001",
        "0.02512345678",
    )
    .await
    .expect("the booked save at the booking's per-1 rate is accepted");

    assert_eq!(
        row(&shop, &saved.document_ref).await,
        Row {
            line_id: Some(booked.id.clone()),
            rate: d("25.12345678"),
            unit_size: d("1000"),
            amount: d("100000"),
            principal: d("2512.35"),
            received: d("0.00"),
            paid: d("2512.35"),
        },
        "the booking's line, rate and unit"
    );
    assert_eq!(booking_status(&shop, "BK-re4c-krw-01").await, "USED");
    shop.finish().await;
}

/// R7: the same booked save one place off in the eleventh decimal (0.02512345679) is not the
/// booking's `per_one`: `RATE_MISMATCH`, nothing written, the booking still `CONFIRMED`.
#[tokio::test]
async fn a_booked_krw_save_off_in_the_eleventh_place_is_refused() {
    let shop = Shop::start().await;
    shop.open_shift(MAIN, &[("THB", "100000")]).await;
    publish(&shop.pool, MAIN, &[("KRW", "25.12345678", "26")]).await;
    let booked = line(&shop.pool, MAIN, "KRW").await;
    confirmed_krw_booking(&shop, "BK-re4c-krw-02", &booked.id, "25.12345678").await;
    let (customer, passport) = counter_prepared(&shop).await;

    let before = shop.everything().await;
    let refused = booked_krw_save(
        &shop,
        (&customer, &passport),
        "BK-re4c-krw-02",
        "re4c-booked-krw-0002",
        "0.02512345679",
    )
    .await;
    assert_eq!(refusal(refused), RATE_MISMATCH);
    assert_eq!(
        shop.everything().await,
        before,
        "the refused save wrote nothing"
    );
    assert_eq!(booking_status(&shop, "BK-re4c-krw-02").await, "CONFIRMED");
    shop.finish().await;
}
