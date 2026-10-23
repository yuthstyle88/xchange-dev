//! CA-7 (#623), stage 1: the kept branch day rebound to the Blueprint's roles (design
//! `docs/design/blueprint/cash-shift-transfer.md` C1, C9, §3.2, §4.2 "branch day", §4.3, §9.1 row CA-7).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file. Stage 2 (the save route and the re-expected v2 fixtures) comes after #755
//! and #767 merge (lane controller's ruling, #623 issuecomment-6093028939).
//!
//! [required] by these tests, in `xc_shift::domain::custody::branch_day::BranchDay` (stateless, no I/O):
//! - `start_service(phase: Phase, actor: &Actor, at: &Target, now: DateTime<Utc>, window: ServiceWindow,
//!   utc_offset_minutes: i32) -> Result<Phase, CustodyError>`: `Authorizer::check(actor, BranchDayOperate, at, now)`
//!   first, a refusal → `CustodyError::RoleNotAllowed`; then READY → OPEN, any other phase →
//!   `CustodyError::ServiceNotReady { window, utc_offset_minutes }`, the branch's own hours as given (#755's
//!   per-branch sentence; lane controller's ruling, #623 issuecomment-6094523727). The window itself is checked
//!   outside `BranchDay` (`BranchHours`).
//! - `cut_off(phase, actor, at, now) -> Result<Phase, CustodyError>`: the same check; OPEN → CUT_OFF, any other phase →
//!   `CustodyError::DayNotOpen` (a new variant of `custody_error.rs`).
//! - `read_phase(stored: Phase, open_shifts: usize) -> Phase`: CUT_OFF with no OPEN shift reads CLOSED (§4.2 "last
//!   open shift CLOSE verified"); every other pair reads as stored.
//! - `check_save(phase, saver: &Actor, own_shift: Option<&str>, drafts: &[TxDraft], save: &SaveAttempt,
//!   window_needs_draft: bool) -> Result<(), CustodyError>`, for a first save (`PriorSave::None`):
//!   - no OPEN shift of the saver's own → `NoOpenShift` (C1, §4.3);
//!   - a phase other than OPEN or CUT_OFF → `SaveClosed`;
//!   - after cut-off, or inside the window's draft-only part, the saver needs an OPEN draft of their own (`TxDraft.
//!     employee_id` = the saver's `user_id`) on their own shift, matching the save's id, side, currency and amount;
//!     none → `SaveClosed` (the kept rule of `custody_machine.rs:1194-1214`);
//!   - no branch custodian: two tellers, each on their own shift, both save (C1).
//!
//! No role literal is asked for: the role table is §11.2 (`RoleGrants::seed`), where only BRANCH_MANAGER holds
//! `BranchDayOperate`.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c; ruling Q5, cash-shift-transfer.md §4.2): `COUNTING`
//! goes with the request half that was its only writer, so it is no longer among the phases tried here.

use chrono::{DateTime, TimeZone, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole, Target};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xc_shift::domain::custody::branch_day::BranchDay;
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::value_objects::custody_error::CustodyError;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ALICE: &str = "00000000-0000-4000-8000-0000000000a1";
const BOB: &str = "00000000-0000-4000-8000-0000000000b2";
const MANAGER: &str = "00000000-0000-4000-8000-0000000000c3";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";

const EVERY_PHASE: [Phase; 4] = [Phase::Closed, Phase::Ready, Phase::Open, Phase::CutOff];

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 6, 0, 0).unwrap()
}

/// B01's hours today: 10:00–22:50 Bangkok (UTC+7), given in UTC.
fn hours() -> ServiceWindow {
    ServiceWindow::Open {
        open: CustodyTime::from_epoch_millis(
            Utc.with_ymd_and_hms(2026, 10, 10, 3, 0, 0)
                .unwrap()
                .timestamp_millis(),
        ),
        close: CustodyTime::from_epoch_millis(
            Utc.with_ymd_and_hms(2026, 10, 10, 15, 50, 0)
                .unwrap()
                .timestamp_millis(),
        ),
    }
}

const BANGKOK: i32 = 7 * 60;

fn on(branch: &str, user: &str, role: Role) -> Actor {
    Actor::new(
        user,
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
}

fn at(branch: &str) -> Target {
    Target::new(ORG, Some(BranchId::new(branch)), None)
}

fn usd(amount: &str) -> ForeignAmount {
    let m = Money::parse(amount).expect("a money string");
    ForeignAmount::from_decimal(rust_decimal::Decimal::from_i128_with_scale(
        m.minor_i128(),
        2,
    ))
}

fn save(draft: Option<&str>) -> SaveAttempt {
    SaveAttempt {
        customer_id: "CX-00000001".to_owned(),
        draft_id: draft.map(str::to_owned),
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").expect("USD"),
        amount: usd("100"),
        prior: PriorSave::None,
    }
}

fn draft(id: &str, owner: &str, shift: &str) -> TxDraft {
    TxDraft {
        id: id.to_owned(),
        shift_id: shift.to_owned(),
        employee_id: owner.to_owned(),
        side: Side::Buy,
        currency: CurrencyCode::parse("USD").expect("USD"),
        amount: usd("100"),
        at: CustodyTime::from_epoch_millis(now().timestamp_millis() - 60_000),
        status: DraftStatus::Open,
        document_ref: None,
    }
}

// ── start service, cut-off: BRANCH_MANAGER through BranchDayOperate (§3.2) ───

#[test]
fn a_branch_manager_starts_service_from_ready() {
    assert_eq!(
        BranchDay::start_service(
            Phase::Ready,
            &on(B01, MANAGER, Role::BranchManager),
            &at(B01),
            now(),
            hours(),
            BANGKOK
        ),
        Ok(Phase::Open)
    );
}

#[test]
fn every_role_but_branch_manager_is_refused_start_service_and_cut_off() {
    for role in [
        Role::Executive,
        Role::Admin,
        Role::RateManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::VerificationReviewer,
        Role::Customer,
    ] {
        let who = on(B01, MANAGER, role);
        assert_eq!(
            BranchDay::start_service(Phase::Ready, &who, &at(B01), now(), hours(), BANGKOK),
            Err(CustodyError::RoleNotAllowed),
            "start {}",
            role.as_str()
        );
        assert_eq!(
            BranchDay::cut_off(Phase::Open, &who, &at(B01), now()),
            Err(CustodyError::RoleNotAllowed),
            "cut-off {}",
            role.as_str()
        );
    }
}

#[test]
fn a_branch_manager_of_another_branch_is_refused() {
    let elsewhere = on(B02, MANAGER, Role::BranchManager);
    assert_eq!(
        BranchDay::start_service(Phase::Ready, &elsewhere, &at(B01), now(), hours(), BANGKOK),
        Err(CustodyError::RoleNotAllowed)
    );
    assert_eq!(
        BranchDay::cut_off(Phase::Open, &elsewhere, &at(B01), now()),
        Err(CustodyError::RoleNotAllowed)
    );
}

#[test]
fn the_access_check_comes_before_the_phase_check() {
    let teller = on(B01, ALICE, Role::Teller);
    assert_eq!(
        BranchDay::start_service(Phase::Closed, &teller, &at(B01), now(), hours(), BANGKOK),
        Err(CustodyError::RoleNotAllowed)
    );
    assert_eq!(
        BranchDay::cut_off(Phase::Closed, &teller, &at(B01), now()),
        Err(CustodyError::RoleNotAllowed)
    );
}

#[test]
fn service_starts_only_from_ready() {
    let manager = on(B01, MANAGER, Role::BranchManager);
    for phase in EVERY_PHASE {
        let expected = if phase == Phase::Ready {
            Ok(Phase::Open)
        } else {
            Err(CustodyError::ServiceNotReady {
                window: hours(),
                utc_offset_minutes: BANGKOK,
            })
        };
        assert_eq!(
            BranchDay::start_service(phase, &manager, &at(B01), now(), hours(), BANGKOK),
            expected,
            "{}",
            phase.as_str()
        );
    }
}

#[test]
fn cut_off_is_taken_only_from_open() {
    let manager = on(B01, MANAGER, Role::BranchManager);
    for phase in EVERY_PHASE {
        let expected = if phase == Phase::Open {
            Ok(Phase::CutOff)
        } else {
            Err(CustodyError::DayNotOpen)
        };
        assert_eq!(
            BranchDay::cut_off(phase, &manager, &at(B01), now()),
            expected,
            "{}",
            phase.as_str()
        );
    }
}

#[test]
fn the_day_not_open_refusal_is_its_own_answer() {
    assert_ne!(
        CustodyError::DayNotOpen,
        CustodyError::ServiceNotReady {
            window: hours(),
            utc_offset_minutes: BANGKOK,
        }
    );
    assert_ne!(CustodyError::DayNotOpen, CustodyError::SaveClosed);
    assert!(!CustodyError::DayNotOpen.to_string().trim().is_empty());
}

// ── the day closes when the last open shift is closed (§4.2) ─────────────────

#[test]
fn cut_off_with_no_open_shift_reads_closed() {
    assert_eq!(BranchDay::read_phase(Phase::CutOff, 0), Phase::Closed);
}

#[test]
fn cut_off_with_an_open_shift_stays_cut_off() {
    assert_eq!(BranchDay::read_phase(Phase::CutOff, 1), Phase::CutOff);
    assert_eq!(BranchDay::read_phase(Phase::CutOff, 3), Phase::CutOff);
}

#[test]
fn every_other_phase_reads_as_stored() {
    for phase in [Phase::Closed, Phase::Ready, Phase::Open] {
        for open in [0, 1, 2] {
            assert_eq!(
                BranchDay::read_phase(phase, open),
                phase,
                "{} with {open}",
                phase.as_str()
            );
        }
    }
}

// ── the save: the saver's own shift, no custodian (C1); drafts after cut-off ─

#[test]
fn a_teller_with_no_open_shift_of_their_own_is_refused_no_open_shift() {
    for phase in [Phase::Open, Phase::CutOff] {
        assert_eq!(
            BranchDay::check_save(
                phase,
                &on(B01, ALICE, Role::Teller),
                None,
                &[],
                &save(None),
                false
            ),
            Err(CustodyError::NoOpenShift),
            "{}",
            phase.as_str()
        );
    }
}

#[test]
fn two_tellers_each_on_their_own_shift_both_save_while_open() {
    assert_eq!(
        BranchDay::check_save(
            Phase::Open,
            &on(B01, ALICE, Role::Teller),
            Some("SH-A"),
            &[],
            &save(None),
            false
        ),
        Ok(())
    );
    assert_eq!(
        BranchDay::check_save(
            Phase::Open,
            &on(B01, BOB, Role::Teller),
            Some("SH-B"),
            &[],
            &save(None),
            false
        ),
        Ok(()),
        "C1: there is no branch custodian"
    );
}

#[test]
fn a_save_outside_open_or_cut_off_is_closed() {
    let alice = on(B01, ALICE, Role::Teller);
    let drafts = [draft("D1", ALICE, "SH-A")];
    for phase in [Phase::Closed, Phase::Ready] {
        assert_eq!(
            BranchDay::check_save(
                phase,
                &alice,
                Some("SH-A"),
                &drafts,
                &save(Some("D1")),
                false
            ),
            Err(CustodyError::SaveClosed),
            "{}",
            phase.as_str()
        );
    }
}

#[test]
fn after_cut_off_a_save_needs_the_savers_own_open_draft() {
    let alice = on(B01, ALICE, Role::Teller);
    assert_eq!(
        BranchDay::check_save(Phase::CutOff, &alice, Some("SH-A"), &[], &save(None), false),
        Err(CustodyError::SaveClosed),
        "no draft"
    );
    assert_eq!(
        BranchDay::check_save(
            Phase::CutOff,
            &alice,
            Some("SH-A"),
            &[draft("D1", ALICE, "SH-A")],
            &save(Some("D1")),
            false
        ),
        Ok(()),
        "her own OPEN draft on her own shift"
    );
}

#[test]
fn after_cut_off_another_persons_draft_does_not_count() {
    assert_eq!(
        BranchDay::check_save(
            Phase::CutOff,
            &on(B01, ALICE, Role::Teller),
            Some("SH-A"),
            &[draft("D1", BOB, "SH-A")],
            &save(Some("D1")),
            false
        ),
        Err(CustodyError::SaveClosed)
    );
}

#[test]
fn after_cut_off_a_draft_on_another_shift_does_not_count() {
    assert_eq!(
        BranchDay::check_save(
            Phase::CutOff,
            &on(B01, ALICE, Role::Teller),
            Some("SH-A"),
            &[draft("D1", ALICE, "SH-OLD")],
            &save(Some("D1")),
            false
        ),
        Err(CustodyError::SaveClosed)
    );
}

#[test]
fn after_cut_off_a_completed_or_mismatched_draft_does_not_count() {
    let alice = on(B01, ALICE, Role::Teller);
    let mut done = draft("D1", ALICE, "SH-A");
    done.status = DraftStatus::Completed;
    let mut other_amount = draft("D2", ALICE, "SH-A");
    other_amount.amount = usd("250");
    let mut other_side = draft("D3", ALICE, "SH-A");
    other_side.side = Side::Sell;
    for (d, what) in [
        (done, "completed"),
        (other_amount, "amount"),
        (other_side, "side"),
    ] {
        let id = d.id.clone();
        assert_eq!(
            BranchDay::check_save(
                Phase::CutOff,
                &alice,
                Some("SH-A"),
                &[d],
                &save(Some(&id)),
                false
            ),
            Err(CustodyError::SaveClosed),
            "{what}"
        );
    }
    assert_eq!(
        BranchDay::check_save(
            Phase::CutOff,
            &alice,
            Some("SH-A"),
            &[draft("D9", ALICE, "SH-A")],
            &save(Some("D1")),
            false
        ),
        Err(CustodyError::SaveClosed),
        "the save names another draft"
    );
}

#[test]
fn while_open_the_windows_draft_only_part_also_needs_an_own_draft() {
    let alice = on(B01, ALICE, Role::Teller);
    assert_eq!(
        BranchDay::check_save(Phase::Open, &alice, Some("SH-A"), &[], &save(None), true),
        Err(CustodyError::SaveClosed)
    );
    assert_eq!(
        BranchDay::check_save(
            Phase::Open,
            &alice,
            Some("SH-A"),
            &[draft("D1", ALICE, "SH-A")],
            &save(Some("D1")),
            true
        ),
        Ok(())
    );
}

#[test]
fn the_not_ready_refusal_carries_the_branchs_own_hours_as_given() {
    let manager = on(B02, MANAGER, Role::BranchManager);
    for (window, offset) in [
        (ServiceWindow::Closed, 420),
        (ServiceWindow::NotSet, 0),
        (hours(), 330),
    ] {
        assert_eq!(
            BranchDay::start_service(Phase::Open, &manager, &at(B02), now(), window, offset),
            Err(CustodyError::ServiceNotReady {
                window,
                utc_offset_minutes: offset,
            }),
            "{window:?} {offset}"
        );
    }
}
