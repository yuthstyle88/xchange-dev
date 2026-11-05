//! Up-front tests for CS-20 (design `docs/design/configurable-shifts.md` §2.10): `ReversalWindow::is_open_until` is
//! open strictly before the deadline the caller computed. Written by the ticket author claude/35271802 — the
//! implementer must not edit, delete or weaken these.

use chrono::{DateTime, Duration, Utc};
use xc_exchange::domain::reversal::window::ReversalWindow;

fn deadline() -> DateTime<Utc> {
    // the end of the 4 Nov business day under the defaults: 5 Nov 06:00 Bangkok
    DateTime::parse_from_rfc3339("2026-11-04T23:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// W1: one second before the deadline the window is open.
#[test]
fn w1_open_just_before_the_deadline() {
    assert!(ReversalWindow::is_open_until(
        deadline(),
        deadline() - Duration::seconds(1)
    ));
}

/// W2: at the deadline itself it is closed.
#[test]
fn w2_closed_at_the_deadline() {
    assert!(!ReversalWindow::is_open_until(deadline(), deadline()));
}

/// W3: after it, closed.
#[test]
fn w3_closed_after_the_deadline() {
    assert!(!ReversalWindow::is_open_until(
        deadline(),
        deadline() + Duration::hours(3)
    ));
}
