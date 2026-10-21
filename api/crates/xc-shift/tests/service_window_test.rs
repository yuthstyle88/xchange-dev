use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;

const OPEN_MS: i64 = 1_000_000_000;
const CLOSE_MS: i64 = OPEN_MS + 3_600_000; // 1 hour window
const CUTOFF_MS: i64 = CLOSE_MS - 600_000; // close - 10 min

fn open_window() -> ServiceWindow {
    ServiceWindow::Open {
        open: CustodyTime::from_epoch_millis(OPEN_MS),
        close: CustodyTime::from_epoch_millis(CLOSE_MS),
    }
}

#[test]
fn row_1_allows_begin_boundaries() {
    let window = open_window();
    assert!(!window.allows_begin(CustodyTime::from_epoch_millis(OPEN_MS - 1)));
    assert!(window.allows_begin(CustodyTime::from_epoch_millis(OPEN_MS)));
    assert!(window.allows_begin(CustodyTime::from_epoch_millis(CUTOFF_MS - 1)));
    assert!(!window.allows_begin(CustodyTime::from_epoch_millis(CUTOFF_MS)));
}

#[test]
fn row_2_allows_save_boundaries() {
    let window = open_window();
    assert!(!window.allows_save(CustodyTime::from_epoch_millis(OPEN_MS - 1)));
    assert!(window.allows_save(CustodyTime::from_epoch_millis(OPEN_MS)));
    assert!(window.allows_save(CustodyTime::from_epoch_millis(CLOSE_MS - 1)));
    assert!(!window.allows_save(CustodyTime::from_epoch_millis(CLOSE_MS)));
}

#[test]
fn row_3_needs_draft_cutoff_boundary() {
    let window = open_window();
    assert!(!window.needs_draft(CustodyTime::from_epoch_millis(CUTOFF_MS - 1)));
    assert!(window.needs_draft(CustodyTime::from_epoch_millis(CUTOFF_MS)));
}

#[test]
fn row_4_not_set_and_closed_windows() {
    let now = CustodyTime::from_epoch_millis(OPEN_MS);

    let not_set = ServiceWindow::NotSet;
    assert!(!not_set.allows_begin(now));
    assert!(!not_set.allows_save(now));
    assert!(not_set.needs_draft(now));

    let closed = ServiceWindow::Closed;
    assert!(!closed.allows_begin(now));
    assert!(!closed.allows_save(now));
    assert!(closed.needs_draft(now));
}
