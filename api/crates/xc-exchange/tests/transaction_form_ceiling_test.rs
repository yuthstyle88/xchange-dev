//! Tests for the exchange form at exactly the largest accepted principal (ticket #87).
//!
//! Ceiling is 2^53 - 1 = 9,007,199,254,740,991 satang (90,071,992,547,409.91 THB).
//!
//! Arithmetic for the chosen (amount, rate) pairs:
//!
//! ```text
//! 1. Exactly the ceiling (2^53 - 1 = 9,007,199,254,740,991 satang):
//!    amount = 4416505.91 (2 decimals, <= 100,000,000)
//!    rate   = 20394401   (0 decimals, <= 100,000,000)
//!    amount * rate = 4,416,505.91 * 20,394,401 = 90,071,992,547,409.91 THB
//!                  = 9,007,199,254,740,991 satang (diff from 2^53 - 1: 0).
//!
//! 2. One satang above the ceiling (2^53 = 9,007,199,254,740,992 satang):
//!    amount = 1638400       (0 decimals, <= 100,000,000)
//!    rate   = 54975581.3888 (4 decimals, <= 100,000,000)
//!    amount * rate = 1,638,400 * 54,975,581.3888 = 90,071,992,547,409.92 THB
//!                  = 9,007,199,254,740,992 satang (diff from 2^53 - 1: +1 satang).
//!
//! 3. One satang below the ceiling (2^53 - 2 = 9,007,199,254,740,990 satang):
//!    amount = 67108863  (0 decimals, <= 100,000,000)
//!    rate   = 1342177.3 (1 decimal,  <= 100,000,000)
//!    amount * rate = 67,108,863 * 1,342,177.3 = 90,071,992,547,409.90 THB
//!                  = 9,007,199,254,740,990 satang (diff from 2^53 - 1: -1 satang).
//! ```

use pos_error::AppError;

use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;

const TOO_LARGE: &str = "ยอดเงินสูงเกินขอบเขตระบบ";
const CEILING_SATANG: i128 = 9_007_199_254_740_991;

fn good_command() -> SaveTransaction {
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
        fee: "0".to_owned(),
        bank_fee: "0".to_owned(),
        method: "Cash".to_owned(),
        payment_ref: "".to_owned(),
        account: "Cash drawer 1".to_owned(),
        staff: "EMP-001".to_owned(),
        booking: "WALK-IN".to_owned(),
        notes: "".to_owned(),
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
fn a_form_whose_principal_is_exactly_the_ceiling_is_accepted() {
    let mut cmd = good_command();
    cmd.amount = "4416505.91".to_owned();
    cmd.rate = "20394401".to_owned();

    let form = TransactionForm::parse(&cmd)
        .expect("a form whose principal is exactly 2^53 - 1 satang must be accepted");

    assert_eq!(form.quote.principal.minor_i128(), CEILING_SATANG);
    assert_eq!(form.quote.principal.to_string(), "90071992547409.91");
}

#[test]
fn a_form_whose_principal_is_one_satang_above_the_ceiling_is_refused() {
    let mut cmd = good_command();
    cmd.amount = "1638400".to_owned();
    cmd.rate = "54975581.3888".to_owned();

    match TransactionForm::parse(&cmd) {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, TOO_LARGE),
        other => panic!("expected Err(AppError::Domain({TOO_LARGE:?})), got {other:?}"),
    }
}

#[test]
fn a_form_whose_principal_is_one_satang_below_the_ceiling_is_accepted() {
    let mut cmd = good_command();
    cmd.amount = "67108863".to_owned();
    cmd.rate = "1342177.3".to_owned();

    let form = TransactionForm::parse(&cmd)
        .expect("a form whose principal is one satang below the ceiling must be accepted");

    assert_eq!(form.quote.principal.minor_i128(), CEILING_SATANG - 1);
    assert_eq!(form.quote.principal.to_string(), "90071992547409.90");
}
