//! Shared by the custody tests: the figures, the people and the clock they are written in.
//! Provided with ticket T9 — do not edit.
//!
//! Re-expected for CA-10 #626 by the ticket author (ruling Q7 A, design `cash-shift-transfer.md`
//! §6): the custody command engine (`CustodyMachine::apply`, `CustodyCommand`, the request model)
//! is retired, and with it the v2 fixture conversions and the desk that drove the machine. What
//! stays is what the surviving suites use.
#![allow(dead_code, unused_imports)]

use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};
pub use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;
pub use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
pub use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
pub use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;

// ── figures ──────────────────────────────────────────────────────────────────

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is a registered code"))
}

/// `"12.5"` → 1250 hundredths. At most two decimals, no sign.
pub fn money(text: &str) -> Money {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let minor: i128 = format!("{whole}{frac:0<2}").parse().expect("digits");
    MoneyDelta::from_minor(minor)
        .to_money()
        .expect("not negative")
}

// ── people ───────────────────────────────────────────────────────────────────

/// `owner` is the owner session; the others are employees whose id is their name here:
/// `F1`‥ Front, `FIN1`‥ Finance, `MON1` Monitor, `MGR1` Manager, `BE1`‥ Backend, `RT1` Rates.
pub fn actor(name: &str) -> CustodyActor {
    if name == "owner" {
        return CustodyActor {
            id: "owner-1".into(),
            employee_id: None,
        };
    }
    match name.trim_end_matches(|c: char| c.is_ascii_digit()) {
        "F" | "FIN" | "MON" | "MGR" | "BE" | "RT" => {}
        other => panic!("unknown actor {other}"),
    }
    CustodyActor {
        id: name.into(),
        employee_id: Some(name.into()),
    }
}

// ── the clock ────────────────────────────────────────────────────────────────

pub const NOON: i64 = 1_791_262_800_000; // 2026-10-06 12:00 in Bangkok

/// `"11:00"` in Bangkok on 2026-10-06, as epoch milliseconds.
pub fn bangkok(hhmm: &str) -> i64 {
    let (h, m) = hhmm.split_once(':').expect("hh:mm");
    let minutes: i64 = h.parse::<i64>().unwrap() * 60 + m.parse::<i64>().unwrap();
    NOON + (minutes - 12 * 60) * 60_000
}

pub fn v2_hours() -> ServiceWindow {
    ServiceWindow::Open {
        open: CustodyTime::from_epoch_millis(bangkok("11:00")),
        close: CustodyTime::from_epoch_millis(bangkok("23:00")),
    }
}
