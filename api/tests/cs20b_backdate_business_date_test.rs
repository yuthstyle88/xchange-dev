//! CS-20b up-front tests (lane controller's ruling on #564, issuecomment-6633086725: one business-date rule): a
//! backdated row's `value_date` must be `ShiftPlan::business_date_of(paid_at)` under the branch's plan, not the
//! calendar date of `paid_at`. Written by the ticket author claude/35271802 — do not edit, delete or weaken.
//!
//! `BackdateSale::run_in` checks the window first (step 1). Every request here sends THB, which step 2 always refuses
//! with `validationFailed:currency`; so that refusal proves the window let the dates through, and a
//! `validationFailed:valueDate` refusal proves it did not. Nothing is written in either case.
//!
//! MAIN keeps the migration's defaults (A 06–14, B 14–22, C 22–06, cutover 06:00, Asia/Bangkok). Instants are UTC;
//! Bangkok is UTC+7. Needs `TEST_DATABASE_URL`.

#[allow(unused_imports)]
mod save_common;
mod shift_plan_common;

use chrono::{DateTime, NaiveDate, Utc};
use save_common::*;
use shift_plan_common::clear_plan;
use xc_auth::domain::access::StandingKind;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side};
use xchange::application::workflows::exchange::backdate::{BackdateRequest, BackdateSale};
use xchange::application::workflows::exchange::create::CreatePayment;
use xchange::application::workflows::save_step::SaveActor;

const WINDOW_PASSED: &str = "validationFailed:currency:";
const VALUE_DATE_REFUSED: &str = "validationFailed:valueDate:";

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a date")
}

fn owner() -> SaveActor {
    SaveActor {
        id: OWNER.to_owned(),
        username: OWNER.to_owned(),
        employee_id: None,
        standing: StandingKind::Owner,
    }
}

/// The sentence `run_in` refuses a THB backdated BUY with, paid at `paid_at` and dated `value_date`, at `now`.
async fn refusal_at(shop: &Shop, paid_at: &str, value_date: &str, now: &str) -> String {
    let req = BackdateRequest {
        branch_id: BranchId::new(MAIN),
        customer_id: "C-NONE".to_owned(),
        side: Side::Buy,
        currency: CurrencyCode::THB,
        quantity: ForeignAmount::from_decimal(rust_decimal::Decimal::from(100)),
        rate: Rate::parse("1").expect("a rate"),
        fee_thb: Money::ZERO,
        expected_settle_thb: None,
        payment: CreatePayment {
            method: PaymentMethod::Cash,
            reference: String::new(),
            account: String::new(),
            slip_key: None,
            bank_fee_thb: Money::ZERO,
            paid_at: instant(paid_at),
        },
        value_date: date(value_date),
        passport_key: None,
        notes: String::new(),
    };
    let now: DateTime<Utc> = instant(now);
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = BackdateSale::new(shop.pool.clone(), shop.files.clone())
        .run_in(&mut tx, req, &owner(), now)
        .await;
    tx.rollback().await.expect("rollback");
    refusal(got)
}

fn assert_starts(got: &str, prefix: &str, row: &str) {
    assert!(
        got.starts_with(prefix),
        "{row}: expected `{prefix}…`, got `{got}`"
    );
}

/// D1: paid 5 Nov 02:00 Bangkok (before the 06:00 cutover) belongs to business date 4 Nov.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d1_a_two_am_deal_is_dated_the_previous_business_date() {
    let shop = Shop::start().await;
    // now 5 Nov 10:00 Bangkok
    let got = refusal_at(
        &shop,
        "2026-11-04T19:00:00Z",
        "2026-11-04",
        "2026-11-05T03:00:00Z",
    )
    .await;
    assert_starts(&got, WINDOW_PASSED, "D1");
    shop.finish().await;
}

/// D2: the calendar date of that deal, 5 Nov, is refused, naming `valueDate`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d2_the_calendar_date_of_a_two_am_deal_is_refused() {
    let shop = Shop::start().await;
    let got = refusal_at(
        &shop,
        "2026-11-04T19:00:00Z",
        "2026-11-05",
        "2026-11-05T03:00:00Z",
    )
    .await;
    assert_starts(&got, VALUE_DATE_REFUSED, "D2");
    shop.finish().await;
}

/// D3: at 5 Nov 03:00 Bangkok the business date is still 4 Nov, so a 02:00 deal dated 4 Nov is not in the future.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d3_before_the_cutover_today_is_still_the_previous_business_date() {
    let shop = Shop::start().await;
    let got = refusal_at(
        &shop,
        "2026-11-04T19:00:00Z",
        "2026-11-04",
        "2026-11-04T20:00:00Z",
    )
    .await;
    assert_starts(&got, WINDOW_PASSED, "D3");
    shop.finish().await;
}

/// D4: from the cutover on, the business date is the calendar date (5 Nov 07:00 Bangkok → 5 Nov).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d4_after_the_cutover_the_business_date_is_the_calendar_date() {
    let shop = Shop::start().await;
    let got = refusal_at(
        &shop,
        "2026-11-05T00:00:00Z",
        "2026-11-05",
        "2026-11-05T03:00:00Z",
    )
    .await;
    assert_starts(&got, WINDOW_PASSED, "D4");
    let got = refusal_at(
        &shop,
        "2026-11-05T00:00:00Z",
        "2026-11-04",
        "2026-11-05T03:00:00Z",
    )
    .await;
    assert_starts(&got, VALUE_DATE_REFUSED, "D4 previous day");
    shop.finish().await;
}

/// D5: a branch with no live template falls back to the calendar date in its timezone, the rule the
/// `shifts_business_date_default` trigger applies to a shift without a template (owner Q6 = A).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn d5_without_a_plan_the_calendar_date_is_the_business_date() {
    let shop = Shop::start().await;
    clear_plan(&shop.pool, MAIN).await;
    let got = refusal_at(
        &shop,
        "2026-11-04T19:00:00Z",
        "2026-11-05",
        "2026-11-05T03:00:00Z",
    )
    .await;
    assert_starts(&got, WINDOW_PASSED, "D5");
    shop.finish().await;
}
