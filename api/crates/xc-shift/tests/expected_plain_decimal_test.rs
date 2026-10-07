//! Ticket #147 — an `expected` text that is not a plain decimal. Written before the fix. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Closing a shift carries the count the screen showed (`expected`, one entry per currency). A
//! text entry is compared with what the server holds after it was read as a number — by a
//! general decimal parser, so `1_0`, `+10`, `.5` and `5.` equal the figures 10, 10, 0.5 and 5 and
//! the close goes through. The rule is "a plain decimal" (`xc_kernel::money::plain_decimal`) with
//! the surrounding spaces trimmed as today; a text that is not one is the same refusal as a
//! count that changed. A JSON number is not a text and keeps its path.
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

/// The text is read leniently as the very figure the server holds, so only the rule can refuse it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expected_text_that_is_not_a_plain_decimal_is_a_count_that_changed() {
    let db = TestDb::start().await;
    for (n, (text, usd)) in [("1_0", "10"), ("+10", "10"), (".5", "0.5"), ("5.", "5")]
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

/// Holds before the fix and after it: a plain text equal to the figure, with spaces around it as
/// today, and a JSON number, close the shift.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plain_text_and_a_number_close_the_shift() {
    let db = TestDb::start().await;
    for (n, claimed) in [json!("10.00"), json!(" 10 "), json!("10"), json!(10)]
        .into_iter()
        .enumerate()
    {
        let shown = claimed.to_string();
        let (closed, still_open) = close_claiming(&db, &format!("plain{n}"), "10", claimed).await;
        closed.unwrap_or_else(|e| panic!("expected {shown} against 10: {e:?}"));
        assert!(!still_open, "the shift is closed ({shown})");
    }
    db.drop_schema().await;
}
