//! The custody machine: who may hand cash to whom, in which phase, and what it records.
//!
//! Provided with ticket T9. These tests are the specification: make them pass,
//! do not edit, delete or weaken them.

mod custody_common;

use custody_common::*;
use xc_kernel::money::Side;

use CustodyCommand as Cmd;
use CustodyError as E;

fn clock_in() -> Cmd {
    Cmd::Clock(ClockDirection::In)
}

fn clock_out() -> Cmd {
    Cmd::Clock(ClockDirection::Out)
}

fn begin() -> Cmd {
    Cmd::BeginTransaction {
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
    }
}

fn start(d: &Desk) -> Cmd {
    Cmd::StartService {
        equipment_ready: true,
        rates_verified: true,
        rate_updated: d.rates_updated.clone(),
    }
}

fn propose(p: Proposal) -> Cmd {
    Cmd::Propose(Box::new(p))
}

fn sign(id: &str) -> Cmd {
    Cmd::Sign { id: id.into() }
}

fn kinds(events: &[CustodyEvent]) -> Vec<EventKind> {
    events.iter().map(|e| e.kind).collect()
}

fn refused(got: Result<Transition, E>) -> E {
    got.expect_err("must be refused")
}

// ── setting up ───────────────────────────────────────────────────────────────

#[test]
fn a_branch_starts_disabled_closed_and_empty() {
    let s = CustodyState::initial();
    assert!(!s.enabled);
    assert_eq!(s.phase, Phase::Closed);
    assert_eq!(
        (s.safe.is_none(), s.target.is_none(), s.entity.is_none()),
        (true, true, true)
    );
    assert_eq!(s.custodian, None);
    assert!(s.attendance.is_empty() && s.requests.is_empty() && s.holds.is_empty());
    assert!(s.drafts.is_empty() && s.exceptions.is_empty() && s.cutoff.is_none());
}

#[test]
fn only_the_owner_names_the_business_and_enables_and_in_that_order() {
    let mut d = Desk::new();
    let entity = || Cmd::SetEntity {
        company: " XCHANGE Co., Ltd. ".into(),
        site: "Main".into(),
        confirm_separated: true,
    };
    assert_eq!(refused(d.act("owner", Cmd::Enable)), E::EntityMissing);
    assert_eq!(refused(d.act("F1", entity())), E::RoleNotAllowed);
    assert_eq!(refused(d.act("F1", Cmd::Enable)), E::RoleNotAllowed);
    let t = d.ok("owner", entity());
    assert_eq!(kinds(&t.events), [EventKind::SetEntity]);
    let e = d.state.entity.clone().unwrap();
    assert_eq!(
        (e.company.as_str(), e.site.as_str(), e.confirmed_by.as_str()),
        ("XCHANGE Co., Ltd.", "Main", "owner-1")
    );
    assert!(!d.state.enabled);
    let t = d.ok("owner", Cmd::Enable);
    assert_eq!(kinds(&t.events), [EventKind::Enable]);
    assert!(d.state.enabled);
    assert_eq!(refused(d.act("owner", entity())), E::EntityLocked);
}

#[test]
fn the_business_must_be_confirmed_in_full() {
    let mut d = Desk::new();
    let with = |company: &str, site: &str, confirm| Cmd::SetEntity {
        company: company.into(),
        site: site.into(),
        confirm_separated: confirm,
    };
    for cmd in [
        with("  ", "Main", true),
        with("Co", "", true),
        with("Co", "Main", false),
        with(&"c".repeat(151), "Main", true),
        with("Co", &"s".repeat(101), true),
    ] {
        assert_eq!(refused(d.act("owner", cmd)), E::EntityIncomplete);
    }
    d.ok("owner", with(&"c".repeat(150), &"s".repeat(100), true));
}

#[test]
fn custody_is_not_set_up_or_enabled_over_an_open_shift() {
    let mut d = Desk::new();
    d.shift = Some(OpenShift {
        id: "S0".into(),
        expected: bal(&[]),
    });
    let entity = Cmd::SetEntity {
        company: "Co".into(),
        site: "Main".into(),
        confirm_separated: true,
    };
    assert_eq!(refused(d.act("owner", entity.clone())), E::EntityLocked);
    assert_eq!(refused(d.act("owner", Cmd::Enable)), E::ShiftStillOpen);
    d.shift = None;
    d.ok("owner", entity);
    d.ok("owner", Cmd::Enable);
}

#[test]
fn until_the_owner_enables_custody_every_other_command_is_refused() {
    let mut d = Desk::new();
    for cmd in [
        clock_in(),
        begin(),
        Cmd::Cutoff,
        sign("x"),
        propose(proposal(RequestKind::Open, 1)),
    ] {
        assert_eq!(refused(d.act("F1", cmd)), E::NotEnabled);
    }
}

// ── attendance ───────────────────────────────────────────────────────────────

#[test]
fn clocking_in_twice_is_one_attendance_and_one_event() {
    let mut d = Desk::enabled();
    let before = d.state.attendance.len();
    let t = d.ok("F1", clock_in());
    assert!(t.events.is_empty());
    assert_eq!(d.state.attendance.len(), before);
    let mine: Vec<_> = d
        .state
        .attendance
        .iter()
        .filter(|a| a.employee_id == "F1")
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].role, mine[0].clock_out),
        (CustodyRole::Front, None)
    );
}

#[test]
fn a_new_attendance_takes_the_id_it_was_given() {
    let mut d = Desk::enabled();
    d.ok("F1", clock_out());
    let id = d.facts().new_id;
    let t = d.ok("F1", clock_in());
    assert_eq!(kinds(&t.events), [EventKind::ClockIn]);
    let a = d.state.attendance.last().unwrap();
    assert_eq!((a.id.as_str(), a.employee_id.as_str()), (id.as_str(), "F1"));
    assert_eq!(a.clock_in, CustodyTime::from_epoch_millis(d.now));
}

#[test]
fn the_owner_has_no_attendance_and_nobody_clocks_out_who_is_not_in() {
    let mut d = Desk::enabled();
    assert_eq!(refused(d.act("owner", clock_in())), E::EmployeeRequired);
    assert_eq!(refused(d.act("F3", clock_out())), E::NoClockInToClose);
    let t = d.ok("F2", clock_out());
    assert_eq!(kinds(&t.events), [EventKind::ClockOut]);
    assert_eq!(refused(d.act("F2", clock_out())), E::NoClockInToClose);
}

#[test]
fn whoever_is_not_clocked_in_can_do_nothing_else() {
    let mut d = Desk::open();
    assert_eq!(refused(d.act("F3", begin())), E::NotClockedIn);
    assert_eq!(refused(d.act("owner", Cmd::Cutoff)), E::NotClockedIn);
    d.ok("BE2", clock_out());
    assert_eq!(refused(d.act("BE2", Cmd::Cutoff)), E::NotClockedIn);
}

// ── serving ──────────────────────────────────────────────────────────────────

#[test]
fn backend_opens_the_service_between_eleven_and_ten_to_eleven_at_night() {
    let mut d = Desk::ready();
    let go = start(&d);
    assert_eq!(refused(d.act("F1", go.clone())), E::RoleNotAllowed);
    assert!(matches!(
        refused(d.act_at("BE1", go.clone(), bangkok("10:59"))),
        E::ServiceNotReady { .. }
    ));
    assert!(matches!(
        refused(d.act_at("BE1", go.clone(), bangkok("22:50"))),
        E::ServiceNotReady { .. }
    ));
    let current = d.rates_updated.clone();
    for (ready, verified, updated) in [
        (false, true, current.as_str()),
        (true, false, current.as_str()),
        (true, true, "2026-10-05T03:00:00.000Z"),
    ] {
        let cmd = Cmd::StartService {
            equipment_ready: ready,
            rates_verified: verified,
            rate_updated: updated.into(),
        };
        assert_eq!(refused(d.act("BE1", cmd)), E::ServiceChecksIncomplete);
    }
    let t = d.act_at("BE1", go.clone(), bangkok("22:49")).unwrap();
    assert_eq!(kinds(&t.events), [EventKind::ServiceOpen]);
    assert_eq!(d.state.phase, Phase::Open);
    assert_eq!(
        d.state.service_opened_at,
        Some(CustodyTime::from_epoch_millis(bangkok("22:49")))
    );
    assert!(matches!(
        refused(d.act("BE1", go)),
        E::ServiceNotReady { .. }
    )); // only from Ready
}

// ── holding a customer ───────────────────────────────────────────────────────

#[test]
fn front_or_monitor_holds_a_customer_and_monitor_releases() {
    let mut d = Desk::open();
    let hold = |reason: &str| Cmd::Hold {
        customer_id: "CX-00000001".into(),
        reason: reason.into(),
    };
    assert_eq!(refused(d.act("FIN1", hold("x"))), E::RoleNotAllowed);
    assert_eq!(refused(d.act("F1", hold("  "))), E::HoldInvalid);
    d.customer_known = false;
    assert_eq!(refused(d.act("F1", hold("x"))), E::HoldInvalid);
    d.customer_known = true;
    let new_id = d.facts().new_id;
    let t = d.ok("F1", hold("passport photo unclear"));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::HoldCustomer,
            subject: Some("CX-00000001".into())
        }]
    );
    let h = d.state.holds[0].clone();
    assert_eq!(
        (h.id.as_str(), h.by.as_str(), h.status),
        (new_id.as_str(), "F1", HoldStatus::Open)
    );

    let release = |id: &str, note: &str| Cmd::Release {
        hold_id: id.into(),
        note: note.into(),
    };
    assert_eq!(
        refused(d.act("F1", release(&h.id, "ok"))),
        E::RoleNotAllowed
    );
    assert_eq!(
        refused(d.act("MON1", release(&h.id, " "))),
        E::ReleaseInvalid
    );
    assert_eq!(
        refused(d.act("MON1", release("nope", "ok"))),
        E::ReleaseInvalid
    );
    let t = d.ok("MON1", release(&h.id, "checked the original"));
    assert_eq!(
        t.events,
        [CustodyEvent {
            kind: EventKind::ReleaseCustomer,
            subject: Some(h.id.clone())
        }]
    );
    let h = &d.state.holds[0];
    assert_eq!(
        (h.status, h.reviewed_by.as_deref(), h.note.as_deref()),
        (
            HoldStatus::Released,
            Some("MON1"),
            Some("checked the original")
        )
    );
    assert_eq!(
        refused(d.act("MON1", release(&h.id.clone(), "again"))),
        E::ReleaseInvalid
    );
}

// ── the gate on a LIVE save (v2 server.mjs:6868-6877) ────────────────────────
// Not in the fixture: v2 reads the wall clock inside this check.

fn save(d: &Desk, draft: Option<&str>) -> SaveAttempt {
    let _ = d;
    SaveAttempt {
        customer_id: "CX-00000001".into(),
        draft_id: draft.map(str::to_string),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
        prior: PriorSave::None,
    }
}

fn gate(d: &Desk, who: &str, s: &SaveAttempt, hhmm: &str) -> Result<(), E> {
    CustodyMachine::check_save(
        &d.state,
        s,
        &actor(who),
        &d.facts(),
        CustodyTime::from_epoch_millis(bangkok(hhmm)),
    )
}

#[test]
fn while_custody_is_off_a_save_is_not_gated() {
    let d = Desk::new();
    assert_eq!(gate(&d, "RT1", &save(&d, None), "03:00"), Ok(()));
}

#[test]
fn a_customer_on_hold_is_not_served_until_released() {
    let mut d = Desk::open();
    d.ok(
        "MON1",
        Cmd::Hold {
            customer_id: "CX-00000001".into(),
            reason: "check".into(),
        },
    );
    let s = save(&d, None);
    assert_eq!(gate(&d, "F1", &s, "12:00"), Err(E::CustomerOnHold));
    assert_eq!(gate(&d, "F2", &s, "03:00"), Err(E::CustomerOnHold)); // said first
    let other = SaveAttempt {
        customer_id: "CX-00000002".into(),
        ..s.clone()
    };
    assert_eq!(gate(&d, "F1", &other, "12:00"), Ok(()));
    let hold = d.state.holds[0].id.clone();
    d.ok(
        "MON1",
        Cmd::Release {
            hold_id: hold,
            note: "fine".into(),
        },
    );
    assert_eq!(gate(&d, "F1", &s, "12:00"), Ok(()));
}

#[test]
fn a_retry_of_a_save_is_judged_by_what_is_already_stored() {
    let mut d = Desk::open();
    d.ok(
        "MON1",
        Cmd::Hold {
            customer_id: "CX-00000001".into(),
            reason: "check".into(),
        },
    );
    let stored = |prior| SaveAttempt {
        prior,
        ..save(&d, None)
    };
    // Already saved by this Front: let through whatever the hour, the phase or a hold —
    // the save itself will answer with the stored result.
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::SavedByActor), "03:00"),
        Ok(())
    );
    assert_eq!(
        gate(&d, "F2", &stored(PriorSave::SavedByActor), "03:00"),
        Ok(())
    ); // not asked who holds the drawer
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::SavedByOther), "12:00"),
        Err(E::SaveNotOwnRetry)
    );
    assert_eq!(
        gate(&d, "MGR1", &stored(PriorSave::SavedByActor), "12:00"),
        Err(E::SaveNotOwnRetry)
    );
    // Stored but not finished: the hours and the phase are not asked again, the rest is.
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::Unfinished), "03:00"),
        Err(E::CustomerOnHold)
    );
    let hold = d.state.holds[0].id.clone();
    d.ok(
        "MON1",
        Cmd::Release {
            hold_id: hold,
            note: "fine".into(),
        },
    );
    let stored = |prior| SaveAttempt {
        prior,
        ..save(&d, None)
    };
    assert_eq!(
        gate(&d, "F1", &stored(PriorSave::Unfinished), "03:00"),
        Ok(())
    );
    assert_eq!(
        gate(&d, "F2", &stored(PriorSave::Unfinished), "03:00"),
        Err(E::SaveNotCustodian)
    );
}
