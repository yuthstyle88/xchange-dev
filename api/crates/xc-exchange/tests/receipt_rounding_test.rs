//! Ticket #5315: the receipt prints the cash rounding, totals the settled figure, and names the staff member.
//! Written before the implementation by the up-front test writer (claude/e2926a49, on the xchange BE lane
//! controller claude/553782's order, 2026-10-28). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Rows of #5315 §3 locked here (pure, no database):
//! - R1: cash with a rounding ≠ 0 prints the row `ปัดเศษเงินสด / Cash rounding` (`+0.20`), right after `THB Paid`,
//!   and `THB Total` is the settled figure (`THB Paid` on BUY, `THB Received` on SELL);
//! - R2: a negative rounding prints `-0.23` (ASCII minus, as every other figure on the receipt);
//! - R3: rounding 0, or non-cash: no rounding row, `THB Total` = settled = principal ∓ fee;
//! - R4: `TransactionForm::with_staff_name` puts the display name in `Staff`; no name keeps the id;
//! - R5: principal ∓ fee + rounding = `THB Total`, to the satang, in every case above;
//! - R6: a form v2 saves (`parse`) has `cash_rounding: None` and keeps v2's total (the principal) and rows.
//!
//! The writer's reading of the ticket (object on #5315 if wrong):
//! - R6 and R3 cannot both hold with one rule for the total: v2 prints the principal as `THB Total` even with a
//!   fee (fixture case "buy with a fee", locked byte for byte by `transaction_form_test.rs`). So the form says
//!   which it is: `cash_rounding: None` = v2 (receipt as today), `Some(r)` = stored exchange (settled total).
//! - `cash_rounding` is a signed `MoneyDelta`, not `Money` as #5315 §4 suggests: `Money` is non-negative and R2
//!   needs `-0.23`.

use chrono::{NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::entities::receipt::receipt_html;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta, Rate, Side, UnitSize};

const ROUNDING_LABEL: &str = "ปัดเศษเงินสด / Cash rounding";

fn money(s: &str) -> Money {
    Money::parse(s).expect("a money figure")
}

/// A COMPLETED-to-be stored exchange of one USD line: `thb` principal, `fee`, `settle` (what the drawer moved).
/// 10 USD at 23.555 gives the principal 235.55 used by the B2 case of #5315 §1.
fn stored(
    side: Side,
    rate: &str,
    thb: &str,
    fee: &str,
    settle: &str,
    method: PaymentMethod,
) -> ExchangeTransaction {
    let now = Utc.with_ymd_and_hms(2026, 10, 27, 20, 30, 0).unwrap();
    ExchangeTransaction {
        queue_ticket_id: None,
        id: TransactionId(Uuid::from_u128(0x5315)),
        organization_id: Uuid::from_u128(1),
        branch_id: BranchId::new("MAIN"),
        mode: Mode::Live,
        status: TransactionStatus::Confirmed,
        document_ref: None,
        quote_id: Some(Uuid::from_u128(2)),
        booking_id: None,
        shift_id: Some("SH-1".to_owned()),
        submission_id: "sub-5315".to_owned(),
        payload_hash: "hash".to_owned(),
        customer_id: "CX-00000001".to_owned(),
        customer_name: "Somchai Jaidee".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: "37ba9f73-0000-4000-8000-000000000001".to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(),
        recorded_at: now,
        recorded_by: "37ba9f73-0000-4000-8000-000000000001".to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-MAIN".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: true,
        lines: vec![TransactionLine {
            id: Uuid::from_u128(3),
            quote_line_id: None,
            shop_side: side,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from_str("10").unwrap()),
            unit_size: UnitSize::ONE,
            rate_snapshot: Rate::parse(rate).unwrap(),
            reference_rate: None,
            thb_amount: money(thb),
            fee_thb: money(fee),
            settle_thb: money(settle),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::from_u128(4),
            payment_no: 1,
            method,
            reference: if method.is_cash() {
                String::new()
            } else {
                "REF-1".to_owned()
            },
            account: "Cash drawer 1".to_owned(),
            slip_file_key: None,
            amount_thb: money(settle),
            bank_fee_thb: Money::ZERO,
            paid_at: now,
        },
        receipts: vec![],
    }
}

fn form_of(t: &ExchangeTransaction) -> TransactionForm {
    TransactionForm::from_transaction(t).expect("a stored exchange's form")
}

fn html_of(form: &TransactionForm) -> String {
    receipt_html(form, "XC-20261028-000001", "XC-B")
}

/// The `<dd>` of the row labelled `label`, if the receipt has one.
fn row<'a>(html: &'a str, label: &str) -> Option<&'a str> {
    let open = format!("<dt>{label}</dt><dd>");
    let at = html.find(&open)? + open.len();
    let end = html[at..].find("</dd>")?;
    Some(&html[at..at + end])
}

/// `THB Total` as printed, without the thousands separators and the unit.
fn total(html: &str) -> String {
    row(html, "THB Total")
        .expect("every receipt has a THB Total")
        .trim_end_matches(" THB")
        .replace(',', "")
}

/// A printed figure (`235.75`, `+0.20`, `-0.23`) in satang.
fn minor(text: &str) -> i128 {
    let mut d = Decimal::from_str(text.trim_start_matches('+'))
        .unwrap_or_else(|_| panic!("a figure: {text}"));
    d.rescale(2);
    d.mantissa()
}

/// R5: principal ∓ fee + rounding = THB Total, read from the printed receipt.
fn assert_the_figures_add_up(html: &str, side: Side) {
    let principal = minor(row(html, "THB Principal").expect("principal row"));
    let fee = minor(row(html, "Customer Fee").expect("fee row"));
    let rounding = row(html, ROUNDING_LABEL).map(minor).unwrap_or(0);
    let want = match side {
        Side::Buy => principal - fee + rounding,
        Side::Sell => principal + fee + rounding,
    };
    assert_eq!(
        minor(&total(html)),
        want,
        "R5: principal ∓ fee + rounding = THB Total\n{html}"
    );
}

// ── R1 ──────────────────────────────────────────────────────────────────────

/// The B2 case of #5315 §1: cash BUY of 235.55, settled 235.75.
#[test]
fn r1_a_cash_buy_rounded_up_prints_the_rounding_row_and_totals_what_was_paid() {
    let t = stored(
        Side::Buy,
        "23.555",
        "235.55",
        "0",
        "235.75",
        PaymentMethod::Cash,
    );
    let form = form_of(&t);
    assert_eq!(form.cash_rounding, Some(MoneyDelta::from_minor(20)));

    let html = html_of(&form);
    assert_eq!(row(&html, ROUNDING_LABEL), Some("+0.20"), "{html}");
    assert!(
        html.contains(&format!(
            "<dt>THB Paid</dt><dd>235.75</dd></div><div class=\"row\"><dt>{ROUNDING_LABEL}</dt><dd>+0.20</dd></div>"
        )),
        "the rounding row comes right after THB Paid: {html}"
    );
    assert_eq!(total(&html), "235.75", "THB Total is THB Paid");
    assert_eq!(row(&html, "THB Principal"), Some("235.55"));
    assert_the_figures_add_up(&html, Side::Buy);
}

#[test]
fn r1_a_cash_buy_with_a_fee_and_a_rounding_totals_what_was_paid() {
    let t = stored(
        Side::Buy,
        "321",
        "3210.00",
        "25.50",
        "3184.75",
        PaymentMethod::Cash,
    );
    let form = form_of(&t);
    assert_eq!(form.cash_rounding, Some(MoneyDelta::from_minor(25)));

    let html = html_of(&form);
    assert_eq!(row(&html, ROUNDING_LABEL), Some("+0.25"));
    assert_eq!(
        row(&html, "THB Total"),
        Some("3,184.75 THB"),
        "grouped by thousands as today"
    );
    assert_the_figures_add_up(&html, Side::Buy);
}

// ── R2 ──────────────────────────────────────────────────────────────────────

#[test]
fn r2_a_cash_sell_rounded_down_prints_a_negative_row_and_totals_what_was_received() {
    let t = stored(
        Side::Sell,
        "23.555",
        "235.55",
        "0",
        "235.32",
        PaymentMethod::Cash,
    );
    let form = form_of(&t);
    assert_eq!(form.cash_rounding, Some(MoneyDelta::from_minor(-23)));

    let html = html_of(&form);
    assert_eq!(row(&html, ROUNDING_LABEL), Some("-0.23"), "{html}");
    assert_eq!(row(&html, "THB Received"), Some("235.32"));
    assert_eq!(total(&html), "235.32", "THB Total is THB Received");
    assert_the_figures_add_up(&html, Side::Sell);
}

// ── R3 ──────────────────────────────────────────────────────────────────────

#[test]
fn r3_cash_with_no_rounding_has_no_row_and_totals_principal_less_fee() {
    let t = stored(
        Side::Buy,
        "321",
        "3210.00",
        "25.50",
        "3184.50",
        PaymentMethod::Cash,
    );
    let form = form_of(&t);
    assert_eq!(form.cash_rounding, Some(MoneyDelta::from_minor(0)));

    let html = html_of(&form);
    assert_eq!(row(&html, ROUNDING_LABEL), None, "no rounding row: {html}");
    assert!(!html.contains("Cash rounding"));
    assert_eq!(total(&html), "3184.50", "THB Total is THB Paid");
    assert_the_figures_add_up(&html, Side::Buy);
}

#[test]
fn r3_a_non_cash_sell_has_no_row_and_totals_principal_plus_fee() {
    let t = stored(
        Side::Sell,
        "100",
        "1000.00",
        "10.00",
        "1010.00",
        PaymentMethod::PromptPay,
    );
    let form = form_of(&t);
    let html = html_of(&form);
    assert_eq!(row(&html, ROUNDING_LABEL), None, "{html}");
    assert_eq!(total(&html), "1010.00", "THB Total is THB Received");
    assert_the_figures_add_up(&html, Side::Sell);
}

/// R3 "no other row changes": with no fee and no rounding, a stored exchange's receipt is the one a form without
/// a rounding (`None`) prints, byte for byte.
#[test]
fn r3_with_no_fee_and_no_rounding_nothing_but_the_figures_already_there_is_printed() {
    let t = stored(
        Side::Buy,
        "23.555",
        "235.55",
        "0",
        "235.55",
        PaymentMethod::Cash,
    );
    let form = form_of(&t);
    let as_today = TransactionForm {
        cash_rounding: None,
        ..form.clone()
    };
    assert_eq!(html_of(&form), html_of(&as_today));
}

// ── R4 ──────────────────────────────────────────────────────────────────────

#[test]
fn r4_the_staff_row_shows_the_display_name() {
    let t = stored(
        Side::Buy,
        "23.555",
        "235.55",
        "0",
        "235.75",
        PaymentMethod::Cash,
    );
    let form = form_of(&t).with_staff_name(Some("Malee S."));
    assert_eq!(form.staff, "Malee S.");
    assert_eq!(row(&html_of(&form), "Staff"), Some("Malee S."));
}

#[test]
fn r4_with_no_name_the_staff_row_keeps_the_id_and_is_never_empty() {
    let t = stored(
        Side::Buy,
        "23.555",
        "235.55",
        "0",
        "235.75",
        PaymentMethod::Cash,
    );
    for name in [None, Some(""), Some("   ")] {
        let form = form_of(&t).with_staff_name(name);
        assert_eq!(form.staff, t.staff_id, "name {name:?} keeps the id");
        assert_eq!(
            row(&html_of(&form), "Staff"),
            Some(t.staff_id.as_str()),
            "name {name:?}"
        );
    }
}

#[test]
fn r4_a_name_is_escaped_like_every_other_value() {
    let t = stored(
        Side::Buy,
        "23.555",
        "235.55",
        "0",
        "235.75",
        PaymentMethod::Cash,
    );
    let form = form_of(&t).with_staff_name(Some("A&B <C>"));
    assert_eq!(row(&html_of(&form), "Staff"), Some("A&amp;B &lt;C&gt;"));
}

// ── R6 ──────────────────────────────────────────────────────────────────────

/// The fixture's "buy with a fee" (`fixtures/v2_save_form_cases.json`): v2 prints the principal as the total.
fn v2_buy_with_a_fee() -> SaveTransaction {
    SaveTransaction {
        customer: ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: "1990-01-31".to_owned(),
            passport_expiry: "2031-12-31".to_owned(),
            mode: "LIVE".to_owned(),
        },
        submission_id: "sub-0001-aaaaaaaaaaaa".to_owned(),
        customer_id: "CX-00000001".to_owned(),
        side: "BUY".to_owned(),
        currency: "USD".to_owned(),
        amount: "100".to_owned(),
        rate: "32.1".to_owned(),
        fee: "25.5".to_owned(),
        bank_fee: "0".to_owned(),
        method: "Cash".to_owned(),
        payment_ref: String::new(),
        account: "Cash drawer 1".to_owned(),
        staff: "EMP-001".to_owned(),
        booking: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: "2026-10-06".to_owned(),
        paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
        payment_confirmed: true,
        identity_confirmed: true,
        passport: Some("11111111-1111-4111-8111-111111111111".to_owned()),
        slip: None,
        reference_rate: None,
    }
}

#[test]
fn r6_a_v2_form_has_no_rounding_and_keeps_v2s_total_and_rows() {
    let form = TransactionForm::parse(&v2_buy_with_a_fee()).expect("a form v2 saves");
    assert_eq!(form.cash_rounding, None);

    let html = html_of(&form);
    assert_eq!(row(&html, ROUNDING_LABEL), None);
    assert_eq!(row(&html, "THB Paid"), Some("3184.50"));
    assert_eq!(
        row(&html, "THB Total"),
        Some("3,210.00 THB"),
        "v2 prints the principal"
    );
    assert_eq!(row(&html, "Staff"), Some("EMP-001"), "v2's typed staff");
}
