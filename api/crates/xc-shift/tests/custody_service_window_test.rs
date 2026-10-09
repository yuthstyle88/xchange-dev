//! OR-6 (`docs/design/blueprint/org-roles-branches.md` §6, §10): the custody machine's service
//! window is the branch's own opening hours, not 11:00–23:00 in Bangkok.
//!
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! The machine is told the window through `CustodyFacts::service_window` (a `ServiceWindow`
//! of instants, already resolved from the branch's timezone). The rules (§6 table):
//! - beginning a transaction and starting the service: within `[open, close − 10 min)`;
//! - a new save: within `[open, close)`, and from `close − 10 min` only for a transaction
//!   begun before (a matching draft), as the 22:50 cut-off does today;
//! - `NotSet` or `Closed`: the service does not open.
//!
//! The branch here is in Asia/Tokyo (UTC+9, no daylight saving) with hours 10:00–20:00, so a
//! machine that still reads the Bangkok clock (Tokyo 10:00 is Bangkok 08:00, Tokyo 19:51 is
//! Bangkok 17:51) answers these tests wrongly.
//!
//! This file does not compile before OR-6: `ServiceWindow` and the `service_window` field are
//! what the ticket adds.

mod custody_common;

use chrono::{FixedOffset, TimeZone};
use custody_common::*;
use xc_kernel::money::Side;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;

use CustodyError as E;

/// `hh:mm` on `day` (of October 2026) in Asia/Tokyo.
fn tokyo_on(day: u32, hhmm: &str) -> CustodyTime {
    let (h, m) = hhmm.split_once(':').expect("hh:mm");
    let tokyo = FixedOffset::east_opt(9 * 3600).expect("UTC+9");
    let t = tokyo
        .with_ymd_and_hms(2026, 10, day, h.parse().unwrap(), m.parse().unwrap(), 0)
        .single()
        .expect("a time in Tokyo");
    CustodyTime::from_epoch_millis(t.timestamp_millis())
}

/// `hh:mm` on 2026-10-07 in Asia/Tokyo, the day after the desk's history.
fn tokyo(hhmm: &str) -> CustodyTime {
    tokyo_on(7, hhmm)
}

/// The branch's window on 2026-10-07: 10:00–20:00 in Tokyo.
fn ten_to_eight() -> ServiceWindow {
    ServiceWindow::Open {
        open: tokyo("10:00"),
        close: tokyo("20:00"),
    }
}

/// The desk's facts, told the given window.
fn told(d: &Desk, window: ServiceWindow) -> CustodyFacts {
    CustodyFacts {
        service_window: window,
        ..d.facts()
    }
}

fn start(d: &Desk) -> CustodyCommand {
    CustodyCommand::StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: d.rates_updated.clone(),
    }
}

fn begin() -> CustodyCommand {
    CustodyCommand::BeginTransaction {
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
    }
}

fn start_at(d: &Desk, window: ServiceWindow, now: CustodyTime) -> Result<Transition, E> {
    CustodyMachine::apply(&d.state, &start(d), &actor("BE1"), &told(d, window), now)
}

fn begin_at(d: &Desk, window: ServiceWindow, now: CustodyTime) -> Result<Transition, E> {
    CustodyMachine::apply(&d.state, &begin(), &actor("F1"), &told(d, window), now)
}

fn save(draft: Option<&str>) -> SaveAttempt {
    SaveAttempt {
        customer_id: "CX-00000001".into(),
        draft_id: draft.map(str::to_string),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
        prior: PriorSave::None,
    }
}

fn gate(d: &Desk, s: &SaveAttempt, window: ServiceWindow, now: CustodyTime) -> Result<(), E> {
    CustodyMachine::check_save(&d.state, s, &actor("F1"), &told(d, window), now)
}

/// `Desk::ready()` with the service opened by BE1 at 12:00 in Tokyo inside `window`.
fn opened(window: ServiceWindow, at: CustodyTime) -> Desk {
    let mut d = Desk::ready();
    let t = start_at(&d, window, at).expect("the service opens inside its window");
    assert_eq!(t.state.phase, Phase::Open);
    d.state = t.state;
    d
}

/// F1 begins a transaction at `at`; the draft's id.
fn draft_at(d: &mut Desk, window: ServiceWindow, at: CustodyTime) -> String {
    let t = begin_at(d, window, at).expect("a transaction begins inside the window");
    let id = t.id.clone().expect("the draft id");
    d.state = t.state;
    id
}

#[test]
fn the_service_starts_from_opening_until_ten_minutes_before_closing_in_the_branch_timezone() {
    let d = Desk::ready();
    let w = ten_to_eight();
    for hhmm in ["09:59", "19:50", "19:51", "19:59", "20:00", "23:00"] {
        assert_eq!(
            start_at(&d, w, tokyo(hhmm)).map(|t| t.state.phase),
            Err(E::ServiceNotReady),
            "a start at {hhmm} Tokyo is outside [10:00, 19:50)"
        );
    }
    for hhmm in ["10:00", "12:00", "19:49"] {
        let t = start_at(&d, w, tokyo(hhmm))
            .unwrap_or_else(|e| panic!("a start at {hhmm} Tokyo is inside [10:00, 19:50): {e:?}"));
        assert_eq!(t.state.phase, Phase::Open);
        assert_eq!(t.state.service_opened_at, Some(tokyo(hhmm)));
    }
}

#[test]
fn a_transaction_begins_from_opening_until_ten_minutes_before_closing() {
    let w = ten_to_eight();
    let d = opened(w, tokyo("12:00"));
    for hhmm in ["09:59", "19:50", "19:51", "20:00"] {
        assert_eq!(
            begin_at(&d, w, tokyo(hhmm)).map(|t| t.id),
            Err(E::OutsideServiceHours),
            "a begin at {hhmm} Tokyo is outside [10:00, 19:50)"
        );
    }
    for hhmm in ["10:00", "19:49"] {
        let t = begin_at(&d, w, tokyo(hhmm))
            .unwrap_or_else(|e| panic!("a begin at {hhmm} Tokyo is inside [10:00, 19:50): {e:?}"));
        assert!(t.id.is_some(), "an accepted begin names its draft");
    }
}

#[test]
fn a_new_save_stops_ten_minutes_before_closing_and_a_begun_one_at_closing() {
    let w = ten_to_eight();
    let mut d = opened(w, tokyo("10:00"));
    let fresh = save(None);
    assert_eq!(gate(&d, &fresh, w, tokyo("09:59")), Err(E::SaveClosed));
    assert_eq!(gate(&d, &fresh, w, tokyo("10:00")), Ok(()));
    assert_eq!(gate(&d, &fresh, w, tokyo("19:49")), Ok(()));
    // the needs-draft cut-off is close − 10 min of this branch, not 22:50 in Bangkok
    assert_eq!(gate(&d, &fresh, w, tokyo("19:50")), Err(E::SaveClosed));
    assert_eq!(gate(&d, &fresh, w, tokyo("19:59")), Err(E::SaveClosed));

    let draft = draft_at(&mut d, w, tokyo("19:45"));
    let begun = save(Some(&draft));
    assert_eq!(gate(&d, &begun, w, tokyo("19:50")), Ok(()));
    assert_eq!(gate(&d, &begun, w, tokyo("19:59")), Ok(()));
    assert_eq!(gate(&d, &begun, w, tokyo("20:00")), Err(E::SaveClosed));
    assert_eq!(gate(&d, &begun, w, tokyo("20:01")), Err(E::SaveClosed));
    assert_eq!(gate(&d, &begun, w, tokyo("09:59")), Err(E::SaveClosed));
}

#[test]
fn a_window_across_midnight_is_read_as_one_window() {
    // 18:00 on the 7th to 02:00 on the 8th, Tokyo (`BranchHours::window` turns a close at or
    // before the open into the next day's)
    let w = ServiceWindow::Open {
        open: tokyo_on(7, "18:00"),
        close: tokyo_on(8, "02:00"),
    };
    let ready = Desk::ready();
    assert_eq!(
        start_at(&ready, w, tokyo_on(7, "17:59")).map(|t| t.state.phase),
        Err(E::ServiceNotReady)
    );
    assert_eq!(
        start_at(&ready, w, tokyo_on(8, "01:50")).map(|t| t.state.phase),
        Err(E::ServiceNotReady)
    );
    let mut d = opened(w, tokyo_on(8, "01:49"));
    let draft = draft_at(&mut d, w, tokyo_on(8, "01:49"));
    assert_eq!(gate(&d, &save(None), w, tokyo_on(8, "01:30")), Ok(()));
    assert_eq!(
        gate(&d, &save(None), w, tokyo_on(8, "01:50")),
        Err(E::SaveClosed)
    );
    assert_eq!(
        gate(&d, &save(Some(&draft)), w, tokyo_on(8, "01:59")),
        Ok(())
    );
    assert_eq!(
        gate(&d, &save(Some(&draft)), w, tokyo_on(8, "02:00")),
        Err(E::SaveClosed)
    );
}

/// Every half hour of the 7th in Tokyo.
fn whole_day() -> Vec<CustodyTime> {
    (0..48)
        .map(|half| tokyo(&format!("{:02}:{:02}", half / 2, (half % 2) * 30)))
        .collect()
}

#[test]
fn a_branch_whose_hours_are_not_set_never_opens_the_service() {
    let ready = Desk::ready();
    for now in whole_day() {
        assert_eq!(
            start_at(&ready, ServiceWindow::NotSet, now).map(|t| t.state.phase),
            Err(E::ServiceNotReady),
            "no hours set: no start at {now:?}"
        );
    }
    // a service opened while hours were set: once they are not, nothing new begins or saves
    let w = ten_to_eight();
    let mut d = opened(w, tokyo("12:00"));
    let draft = draft_at(&mut d, w, tokyo("12:05"));
    for now in whole_day() {
        assert_eq!(
            begin_at(&d, ServiceWindow::NotSet, now).map(|t| t.id),
            Err(E::OutsideServiceHours),
            "no hours set: no begin at {now:?}"
        );
        assert_eq!(
            gate(&d, &save(None), ServiceWindow::NotSet, now),
            Err(E::SaveClosed)
        );
        assert_eq!(
            gate(&d, &save(Some(&draft)), ServiceWindow::NotSet, now),
            Err(E::SaveClosed)
        );
    }
}

#[test]
fn a_closed_day_never_opens_the_service() {
    let ready = Desk::ready();
    let w = ten_to_eight();
    let mut d = opened(w, tokyo("12:00"));
    let draft = draft_at(&mut d, w, tokyo("12:05"));
    for now in whole_day() {
        assert_eq!(
            start_at(&ready, ServiceWindow::Closed, now).map(|t| t.state.phase),
            Err(E::ServiceNotReady),
            "a closed day: no start at {now:?}"
        );
        assert_eq!(
            begin_at(&d, ServiceWindow::Closed, now).map(|t| t.id),
            Err(E::OutsideServiceHours),
            "a closed day: no begin at {now:?}"
        );
        assert_eq!(
            gate(&d, &save(None), ServiceWindow::Closed, now),
            Err(E::SaveClosed)
        );
        assert_eq!(
            gate(&d, &save(Some(&draft)), ServiceWindow::Closed, now),
            Err(E::SaveClosed)
        );
    }
}

#[test]
fn the_old_bangkok_hours_are_one_window_among_others() {
    // a branch in Bangkok with 11:00–23:00 keeps today's answers (D7, v2 parity)
    let bkk = |hhmm: &str| CustodyTime::from_epoch_millis(bangkok(hhmm));
    let w = ServiceWindow::Open {
        open: bkk("11:00"),
        close: bkk("23:00"),
    };
    let ready = Desk::ready();
    assert_eq!(
        start_at(&ready, w, bkk("10:59")).map(|t| t.state.phase),
        Err(E::ServiceNotReady)
    );
    assert_eq!(
        start_at(&ready, w, bkk("22:50")).map(|t| t.state.phase),
        Err(E::ServiceNotReady)
    );
    assert!(start_at(&ready, w, bkk("22:49")).is_ok());
    // … and the same instants are outside a Tokyo 10:00–20:00 window of that day
    let tokyo_6 = ServiceWindow::Open {
        open: tokyo_on(6, "10:00"),
        close: tokyo_on(6, "20:00"),
    };
    assert_eq!(
        start_at(&ready, tokyo_6, bkk("22:49")).map(|t| t.state.phase),
        Err(E::ServiceNotReady)
    );
}
