//! XC Counter, ticket CA-1e (`docs/design/xc-counter-app.md` §4, §5.2, §5.4, §8.5): rule A and the drawer target as
//! one pure decision, and the capabilities `info()` answers. No database. Written before the implementation. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! `cash_payout(seat, cash_from_browser)`:
//!
//! | #  | Seat | Counter `cash_drawer` | `cashFromBrowser` | Expected |
//! |----|------|-----------------------|-------------------|----------|
//! | P1 | bound to X | `Some(true)` | any | `Allowed { target: Some(X) }` |
//! | P2 | bound to X | `Some(false)` | any | `Refused` (rule A) |
//! | P3 | bound to X | `None` (not set = off) | any | `Refused` |
//! | P4 | bound to X, X revoked | `Some(true)` | any | `Refused` (a revoked counter has no drawer) |
//! | P5 | bound to an id that names no counter | — | any | `Refused` |
//! | P6 | unbound | — | `None` (not set = on) | `Allowed { target: None }` |
//! | P7 | unbound | — | `Some(true)` | `Allowed { target: None }` |
//! | P8 | unbound | — | `Some(false)` | `Refused` |
//!
//! `Capabilities::of(agent)`: `None` → cash drawer off, scanner off, kiosk lock on; a set value is taken as is (C1–C3).

use chrono::{TimeZone, Utc};
use xchange::infrastructure::device_agents::agent_repository::DeviceAgent;
use xchange::infrastructure::device_agents::counter_policy::{
    CASH_FROM_BROWSER, COUNTER_NO_DRAWER, COUNTER_NO_DRAWER_MESSAGE, Capabilities, CashPayout,
    Seat, cash_payout,
};

const X: &str = "c0000000-0000-4000-8000-00000000000a";

fn counter(
    cash_drawer: Option<bool>,
    scanner: Option<bool>,
    kiosk_lock: Option<bool>,
) -> DeviceAgent {
    let at = Utc
        .with_ymd_and_hms(2026, 11, 6, 3, 0, 0)
        .single()
        .expect("an instant");
    DeviceAgent {
        id: X.to_owned(),
        branch_id: "MAIN".to_owned(),
        name: "Mac".to_owned(),
        created_at: at,
        created_by: "owner".to_owned(),
        pair_code_expires_at: None,
        paired_at: Some(at),
        revoked_at: None,
        revoked_by: None,
        last_seen_at: Some(at),
        agent_version: Some("2.0.0".to_owned()),
        agent_build: None,
        agent_os: None,
        agent_host: None,
        drawer_transport: None,
        drawer_target: None,
        counter_name: "เคาน์เตอร์ 1".to_owned(),
        cash_drawer,
        scanner,
        kiosk_lock,
    }
}

fn allowed(target: Option<&str>) -> CashPayout {
    CashPayout::Allowed {
        target_agent_id: target.map(str::to_owned),
    }
}

#[test]
fn the_names_are_the_design_s() {
    assert_eq!(COUNTER_NO_DRAWER, "COUNTER_NO_DRAWER");
    assert_eq!(
        COUNTER_NO_DRAWER_MESSAGE,
        "เคาน์เตอร์นี้ไม่มีลิ้นชักเงิน — ส่งไปชำระที่เคาน์เตอร์ที่มีลิ้นชัก"
    );
    assert_eq!(CASH_FROM_BROWSER, "cashFromBrowser");
}

#[test]
fn a_bound_session_pays_cash_only_at_a_drawer_counter() {
    for browser in [None, Some(true), Some(false)] {
        let on = counter(Some(true), None, None);
        assert_eq!(
            cash_payout(Seat::Bound(Some(&on)), browser),
            allowed(Some(X)),
            "P1 {browser:?}"
        );
        let off = counter(Some(false), None, None);
        assert_eq!(
            cash_payout(Seat::Bound(Some(&off)), browser),
            CashPayout::Refused,
            "P2 {browser:?}"
        );
        let unset = counter(None, None, None);
        assert_eq!(
            cash_payout(Seat::Bound(Some(&unset)), browser),
            CashPayout::Refused,
            "P3 {browser:?}"
        );
        let mut revoked = counter(Some(true), None, None);
        revoked.revoked_at = revoked.paired_at;
        assert_eq!(
            cash_payout(Seat::Bound(Some(&revoked)), browser),
            CashPayout::Refused,
            "P4 {browser:?}"
        );
        assert_eq!(
            cash_payout(Seat::Bound(None), browser),
            CashPayout::Refused,
            "P5 {browser:?}"
        );
    }
}

#[test]
fn an_unbound_session_follows_the_branch_setting() {
    assert_eq!(
        cash_payout(Seat::Unbound, None),
        allowed(None),
        "P6: not set = on"
    );
    assert_eq!(cash_payout(Seat::Unbound, Some(true)), allowed(None), "P7");
    assert_eq!(
        cash_payout(Seat::Unbound, Some(false)),
        CashPayout::Refused,
        "P8"
    );
}

#[test]
fn capabilities_resolve_not_set_to_the_safe_value() {
    assert_eq!(
        Capabilities::of(&counter(None, None, None)),
        Capabilities {
            cash_drawer: false,
            scanner: false,
            kiosk_lock: true
        },
        "C1"
    );
    assert_eq!(
        Capabilities::of(&counter(Some(true), Some(true), Some(false))),
        Capabilities {
            cash_drawer: true,
            scanner: true,
            kiosk_lock: false
        },
        "C2"
    );
    assert_eq!(
        Capabilities::of(&counter(Some(false), Some(false), Some(true))),
        Capabilities {
            cash_drawer: false,
            scanner: false,
            kiosk_lock: true
        },
        "C3"
    );
}
