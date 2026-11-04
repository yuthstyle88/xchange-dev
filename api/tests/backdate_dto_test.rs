//! Unit tests for BK-10 (`docs/design/backdated-keying.md` §9):
//! `TransactionDto::with_backdate` JSON serialization.

use chrono::{TimeZone, Utc};
use serde_json::Value;
use xc_exchange::domain::transaction::backdate_mark::BackdateMark;
use xchange::presentation::dto::v3::transaction::{TransactionDto, TransactionPaymentDto};

fn sample_transaction_dto() -> TransactionDto {
    TransactionDto {
        id: "tx-test-id".to_owned(),
        branch_id: "branch-1".to_owned(),
        mode: "LIVE".to_owned(),
        status: "COMPLETED".to_owned(),
        document_ref: Some("XC-20261104-000001".to_owned()),
        quote_id: None,
        customer_id: "cust-1".to_owned(),
        approval_request_id: None,
        cancel_reason: None,
        cancelled_at: None,
        recorded_at: "2026-11-04T13:23:00+07:00".to_owned(),
        lines: vec![],
        payment: TransactionPaymentDto {
            method: "Cash".to_owned(),
            reference: "".to_owned(),
            account: "".to_owned(),
            amount_thb: "1000.00".to_owned(),
            bank_fee_thb: "0.00".to_owned(),
            paid_at: "2026-11-04T13:23:00+07:00".to_owned(),
        },
        backdate: None,
    }
}

#[test]
fn none_leaves_backdate_absent_from_json() {
    let dto = sample_transaction_dto().with_backdate(None);
    let val: Value = serde_json::to_value(&dto).expect("TransactionDto serialises");
    assert!(
        val.get("backdate").is_none(),
        "None leaves backdate absent from JSON: {val}"
    );
}

#[test]
fn some_gives_both_keys() {
    let entered_at = Utc.with_ymd_and_hms(2026, 11, 4, 14, 15, 0).unwrap();
    let mark = BackdateMark {
        entered_at,
        entered_by: "user-admin-1".to_owned(),
    };
    let dto = sample_transaction_dto().with_backdate(Some(mark));
    let val: Value = serde_json::to_value(&dto).expect("TransactionDto serialises");
    let backdate = val
        .get("backdate")
        .expect("Some mark must produce `backdate` key in JSON");

    assert_eq!(
        backdate.get("enteredAt").and_then(|v| v.as_str()),
        Some("2026-11-04T14:15:00+00:00")
    );
    assert_eq!(
        backdate.get("enteredBy").and_then(|v| v.as_str()),
        Some("user-admin-1")
    );
}
