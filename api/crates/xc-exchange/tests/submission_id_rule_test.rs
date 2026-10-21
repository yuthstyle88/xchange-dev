//! #4137 row 5 (exchange site): `TransactionForm::parse` answers a submission id exactly as before the shared
//! rule — the text check at step 2 (sentence of `InputError::Text`), the shape check at step 18 (`INCOMPLETE`).
//! Green before and after #4137. Written by the BE-R6 sub-controller (claude/studio-claude-8); do not edit, delete
//! or weaken.

use pos_error::AppError;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::value_objects::transaction_form::{INCOMPLETE, TransactionForm};

const TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";

/// The fixture's "buy for cash" form (`tests/fixtures/v2_save_form_cases.json`) with `submission_id`.
fn form(submission_id: &str) -> SaveTransaction {
    SaveTransaction {
        customer: ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: "1990-01-31".to_owned(),
            passport_expiry: "2031-12-31".to_owned(),
            mode: "LIVE".to_owned(),
        },
        submission_id: submission_id.to_owned(),
        customer_id: "CX-00000001".to_owned(),
        side: "BUY".to_owned(),
        currency: "USD".to_owned(),
        amount: "100".to_owned(),
        rate: "32.1".to_owned(),
        fee: "0".to_owned(),
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

fn refusal(submission_id: &str) -> String {
    match TransactionForm::parse(&form(submission_id)) {
        Err(AppError::Domain(s)) => s,
        other => panic!("{submission_id:?}: expected a domain refusal, got {other:?}"),
    }
}

#[test]
fn the_exchange_site_answers_as_before() {
    let sixty = "A1-".repeat(20);
    for ok in [
        "sub-0001-aaaaaaaaaaaa",
        "abcdefghij123456",
        sixty.as_str(),
        "  abcdefghij123456 ",
    ] {
        assert!(TransactionForm::parse(&form(ok)).is_ok(), "{ok:?}");
    }
    assert_eq!(refusal("abcdefghij12345"), INCOMPLETE);
    assert_eq!(refusal(&format!("{sixty}x")), TEXT);
    for bad in [
        "abcdefghij_123456",
        "abcdefghij.123456",
        "abcdefghijก123456",
    ] {
        assert_eq!(refusal(bad), INCOMPLETE, "{bad:?}");
    }
    assert_eq!(refusal(""), TEXT);
    assert_eq!(refusal("   "), TEXT);
}
