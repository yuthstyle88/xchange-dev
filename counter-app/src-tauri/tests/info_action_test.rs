//! CA-3g (design §2, §8.2): pure decision after `GET /api/device-agent/info` for kiosk lock,
//! pair-page navigation and stopping the drawer runner.
//!
//! | Answer | `prev_locked` | `lock` | `show_pair` | `stop_runner` |
//! |---|---|---|---|---|
//! | `Ok(kiosk_lock: true)` | `false` / `true` | `Some(true)` | `false` | `false` |
//! | `Ok(kiosk_lock: false)` | `false` / `true` | `Some(false)` | `false` | `false` |
//! | `Unauthorized` | `false` / `true` | `Some(false)` | `true` | `true` |
//! | `Failed` | `true` | `None` | `false` | `false` |
//! | `Failed` | `false` | `None` | `false` | `false` |

use xc_counter::agent::Capabilities;
use xc_counter::{InfoAction, InfoAnswer, on_info};

fn caps(kiosk_lock: bool) -> Capabilities {
    Capabilities {
        cash_drawer: false,
        scanner: false,
        kiosk_lock,
    }
}

#[test]
fn ok_kiosk_lock_true_locks_on() {
    let expected = InfoAction {
        lock: Some(true),
        show_pair: false,
        stop_runner: false,
    };
    assert_eq!(on_info(false, InfoAnswer::Ok(caps(true))), expected);
    assert_eq!(on_info(true, InfoAnswer::Ok(caps(true))), expected);
}

#[test]
fn ok_kiosk_lock_false_locks_off() {
    let expected = InfoAction {
        lock: Some(false),
        show_pair: false,
        stop_runner: false,
    };
    assert_eq!(on_info(true, InfoAnswer::Ok(caps(false))), expected);
    assert_eq!(on_info(false, InfoAnswer::Ok(caps(false))), expected);
}

#[test]
fn unauthorized_unlocks_shows_pair_and_stops_runner() {
    let expected = InfoAction {
        lock: Some(false),
        show_pair: true,
        stop_runner: true,
    };
    assert_eq!(on_info(true, InfoAnswer::Unauthorized), expected);
    assert_eq!(on_info(false, InfoAnswer::Unauthorized), expected);
}

#[test]
fn failed_while_locked_makes_no_change() {
    assert_eq!(
        on_info(true, InfoAnswer::Failed),
        InfoAction {
            lock: None,
            show_pair: false,
            stop_runner: false,
        }
    );
}

#[test]
fn failed_while_unlocked_makes_no_change() {
    assert_eq!(
        on_info(false, InfoAnswer::Failed),
        InfoAction {
            lock: None,
            show_pair: false,
            stop_runner: false,
        }
    );
}
