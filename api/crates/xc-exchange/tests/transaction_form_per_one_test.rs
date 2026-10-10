//! Ticket #757 (RE-4c) — the desk save's client rate is the v2 wire's per-1 rate, which carries
//! up to 8 + log10(`unit_size`) places: `TransactionForm::parse` takes up to 11 decimals in
//! `rate` and refuses 12 with `TOO_MANY_DECIMALS`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §3.3 (a published rate has 8 places per
//! `unit_size`), §4.9 R3 (the v2-compat wire is per 1: `rate ÷ unit_size`, exact) and R4 (the
//! save accepts the client's per-1 rate only when it equals the current line's `per_one`); the
//! ticket author's ruling for RE-4c of 2026-10-10 (11 decimals accepted, 12 refused, the
//! register's largest `unit_size` being 1000), and the further ruling of 2026-10-10: the
//! reference rate is sent per 1 the same way. What the save does with the rate is pinned
//! through the save in `api/tests/rates_re4_desk_save_test.rs`.
//!
//! No database is used here; nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `TransactionForm::parse(&SaveTransaction) -> AppResult<TransactionForm>` accepts
//!   a `rate` and a `reference_rate` text of up to 11 decimals and refuses 12 with
//!   `TOO_MANY_DECIMALS`
//!   (`transaction_form.rs:15`) as `AppError::Domain`;
//! - `SaveTransaction` and `ReserveCustomer` as on the merge base `d6ce058`.

use pos_error::{AppError, AppResult};
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;

const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";

fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// A purchase of 100,000 KRW for cash at the per-1 `rate`.
fn krw_at(rate: &str) -> SaveTransaction {
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
        currency: "KRW".to_owned(),
        amount: "100000".to_owned(),
        rate: rate.to_owned(),
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

/// 25.12345678 per 1000 is 0.02512345678 per 1: eleven places, and nine and ten too.
#[test]
fn a_per_1_rate_of_up_to_eleven_places_is_taken() {
    for rate in ["0.02512345678", "0.0251234567", "0.025123456", "32.1"] {
        TransactionForm::parse(&krw_at(rate))
            .unwrap_or_else(|e| panic!("{rate} is a per-1 rate the wire can carry: {e:?}"));
    }
}

/// Twelve places is more than any register unit gives: refused, with v2's sentence.
#[test]
fn a_rate_of_twelve_places_is_too_many_decimals() {
    for rate in ["0.025123456781", "32.100000000001"] {
        assert_eq!(
            refusal(TransactionForm::parse(&krw_at(rate))),
            TOO_MANY_DECIMALS,
            "{rate}"
        );
    }
}

/// The reference rate is sent per 1 too: eleven places are taken, twelve refused.
#[test]
fn a_per_1_reference_rate_takes_eleven_places_and_refuses_twelve() {
    let with_reference = |reference: &str| {
        let mut cmd = krw_at("0.02512345678");
        cmd.reference_rate = Some(reference.to_owned());
        cmd
    };
    TransactionForm::parse(&with_reference("0.02500000001"))
        .unwrap_or_else(|e| panic!("an eleven-place reference is taken: {e:?}"));
    assert_eq!(
        refusal(TransactionForm::parse(&with_reference("0.025000000001"))),
        TOO_MANY_DECIMALS
    );
}
