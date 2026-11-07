//! CA-3b (design §2 "Kiosk lock"): which shortcuts the lock stops. How each is stopped is in
//! `src/kiosk.rs`. Written before the implementation. Do not edit, delete or weaken; add tests in
//! a file of your own.
//!
//! | Shortcut | locked | unlocked |
//! |---|---|---|
//! | Cmd+Q, Cmd+W, Cmd+Tab, Cmd+H, Cmd+M, Cmd+Option+Esc | stopped | not stopped |

use xc_counter::kiosk::{Shortcut, intercepts};

const ALL: [Shortcut; 6] = [
    Shortcut::CmdQ,
    Shortcut::CmdW,
    Shortcut::CmdTab,
    Shortcut::CmdH,
    Shortcut::CmdM,
    Shortcut::CmdOptionEsc,
];

#[test]
fn locked_stops_every_shortcut() {
    for shortcut in ALL {
        assert!(intercepts(shortcut, true), "{shortcut:?}");
    }
}

#[test]
fn unlocked_stops_none() {
    for shortcut in ALL {
        assert!(!intercepts(shortcut, false), "{shortcut:?}");
    }
}
