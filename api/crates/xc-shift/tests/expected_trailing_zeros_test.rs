//! Ticket #252 — an `expected` figure written with zeros past the second decimal. Written before
//! the fix. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Closing a shift carries the count the screen showed (`expected`, one entry per currency). A
//! numeric text claims the value it names (#48, rule 4 of the close), so `"250.000"` claims the
//! same 250 as `"250.00"`, `"250"` and `250`. Today a text with more than two decimals is refused
//! as a count that changed even when every extra decimal is zero. A figure whose extra decimals
//! are not all zero names a different amount and stays refused.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.

mod shift_service_common;

use serde_json::{Value, json};
use shift_service_common::{TestDb, at, refusal};
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::commands::close_shift::CloseShift;
use xc_shift::domain::shift::commands::open_shift::OpenShift;

const EXPECTED_CHANGED: &str = "ยอดเปลี่ยนระหว่างตรวจนับ กรุณารีเฟรชและตรวจสอบอีกครั้ง";

fn rid(label: &str, n: usize) -> String {
    format!("req-{label}-{n:04}-000000000000")
}

/// A shift opened with `usd` dollars, then closed with the count equal to it and `expected`
/// naming `claimed` for USD. Gives the answer of the close and whether the shift is still open.
async fn close_claiming(
    db: &TestDb,
    label: &str,
    usd: &str,
    claimed: Value,
) -> (pos_error::AppResult<()>, bool) {
    let branch: BranchId = db.branch(label).await;
    let service = db.service();
    let opened = service
        .open(
            &branch,
            OpenShift::from_payload(
                &json!({ "requestId": rid(label, 0), "opening": { "USD": usd } }),
            )
            .expect("a well-formed open payload"),
            "opener",
            at(0, 0),
        )
        .await
        .expect("open the shift");
    let closed = service
        .close(
            &branch,
            CloseShift::from_payload(&json!({
                "requestId": rid(label, 1), "shiftId": opened.id,
                "counted": { "USD": usd }, "expected": { "USD": claimed }, "note": "",
            }))
            .expect("a well-formed close payload"),
            "supervisor",
            at(60, 0),
        )
        .await
        .map(|_| ());
    let still_open = service
        .active(&branch)
        .await
        .expect("read the open shift")
        .is_some();
    (closed, still_open)
}

/// Red before the fix: every extra decimal is zero, so the text claims exactly what the server
/// holds and the shift closes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expected_text_with_trailing_zeros_past_two_decimals_closes_the_shift() {
    let db = TestDb::start().await;
    for (n, (text, usd)) in [
        ("250.000", "250"),
        ("250.0000000000", "250"),
        ("12.500", "12.5"),
        ("0.000", "0"),
        (" 250.000 ", "250"),
    ]
    .into_iter()
    .enumerate()
    {
        let (closed, still_open) =
            close_claiming(&db, &format!("zeros{n}"), usd, json!(text)).await;
        closed.unwrap_or_else(|e| panic!("expected {text:?} against {usd}: {e:?}"));
        assert!(!still_open, "the shift is closed ({text:?})");
    }
    db.drop_schema().await;
}

/// Holds before the fix and after it: a text whose third decimal is not zero names an amount no
/// balance can hold, so it is a count that changed — never rounded or cut to two decimals.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expected_text_with_a_non_zero_third_decimal_is_a_count_that_changed() {
    let db = TestDb::start().await;
    for (n, (text, usd)) in [
        ("250.001", "250"),
        ("250.004", "250"),
        ("249.999", "250"),
        ("250.0000001", "250"),
        ("0.001", "0"),
    ]
    .into_iter()
    .enumerate()
    {
        let (closed, still_open) =
            close_claiming(&db, &format!("third{n}"), usd, json!(text)).await;
        assert_eq!(
            refusal(closed),
            EXPECTED_CHANGED,
            "expected {text:?} against {usd}: not the amount the server holds"
        );
        assert!(
            still_open,
            "a refused close leaves the shift open ({text:?})"
        );
    }
    db.drop_schema().await;
}

/// Holds before the fix and after it: a JSON number keeps its own path. Read from the text
/// `250.000` it is the number 250 and closes the shift; `250.004` is a different amount.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expected_number_written_with_three_decimals_keeps_its_path() {
    let db = TestDb::start().await;
    let same: Value = serde_json::from_str("250.000").expect("a JSON number");
    let (closed, still_open) = close_claiming(&db, "num0", "250", same).await;
    closed.unwrap_or_else(|e| panic!("expected 250.000 (a number) against 250: {e:?}"));
    assert!(!still_open, "the shift is closed (250.000 as a number)");

    let other: Value = serde_json::from_str("250.004").expect("a JSON number");
    let (closed, still_open) = close_claiming(&db, "num1", "250", other).await;
    assert_eq!(
        refusal(closed),
        EXPECTED_CHANGED,
        "expected 250.004 (a number) against 250"
    );
    assert!(
        still_open,
        "a refused close leaves the shift open (250.004)"
    );
    db.drop_schema().await;
}

/// Holds before the fix and after it: the forms #147 refuses stay refused when they also carry
/// zeros past the second decimal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_text_that_is_not_a_plain_decimal_stays_refused_with_trailing_zeros() {
    let db = TestDb::start().await;
    for (n, (text, usd)) in [
        ("+250.000", "250"),
        ("250.000.0", "250"),
        ("-0.000", "0"),
        ("2_50.000", "250"),
        ("250.000e0", "250"),
    ]
    .into_iter()
    .enumerate()
    {
        let (closed, still_open) = close_claiming(&db, &format!("odd{n}"), usd, json!(text)).await;
        assert_eq!(
            refusal(closed),
            EXPECTED_CHANGED,
            "expected {text:?} against {usd}: not a plain decimal"
        );
        assert!(
            still_open,
            "a refused close leaves the shift open ({text:?})"
        );
    }
    db.drop_schema().await;
}
