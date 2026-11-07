//! CA-3f (design §2, §4 `kioskLock`, §5.2, §8.2): pure startup decision for kiosk lock and drawer
//! runner.
//!
//! | Paired | Capabilities | Lock | Runner |
//! |---|---|---|---|
//! | false | unknown (`None`) | false | false |
//! | true | unknown (`None`) | false | true |
//! | true | `kiosk_lock: false` | false | true |
//! | true | `kiosk_lock: true` (the server resolves NULL to true) | true | true |

use xc_counter::agent::Capabilities;
use xc_counter::{Startup, startup};

#[test]
fn not_paired_caps_unknown_no_lock_no_runner() {
    assert_eq!(
        startup(false, None),
        Startup {
            lock: false,
            start_runner: false,
        }
    );
}

#[test]
fn paired_caps_unknown_no_lock_and_starts_runner() {
    assert_eq!(
        startup(true, None),
        Startup {
            lock: false,
            start_runner: true,
        }
    );
}

#[test]
fn paired_kiosk_lock_false_unlocks_and_starts_runner() {
    let caps = Capabilities {
        cash_drawer: false,
        scanner: false,
        kiosk_lock: false,
    };
    assert_eq!(
        startup(true, Some(caps)),
        Startup {
            lock: false,
            start_runner: true,
        }
    );
}

#[test]
fn paired_kiosk_lock_true_locks_and_starts_runner() {
    let caps = Capabilities {
        cash_drawer: true,
        scanner: true,
        kiosk_lock: true,
    };
    assert_eq!(
        startup(true, Some(caps)),
        Startup {
            lock: true,
            start_runner: true,
        }
    );
}
