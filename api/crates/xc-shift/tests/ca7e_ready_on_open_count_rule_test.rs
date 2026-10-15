//! CA-7e: the pure rule of the branch day for a verified OPEN count. Design
//! `docs/design/blueprint/cash-shift-transfer.md` §4.2 and §9 / §9.1's CA-7e rows; the BE lane controller's ruling Q1
//! on #626 (2026-10-15).
//!
//! Written before the implementation by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Signature assumed ([required], §9): `BranchDay::ready_on_open_count(stored: Phase, open_shifts_before: usize) ->
//! Option<Phase>` in `xc_shift::domain::custody::branch_day`. It answers `Some(Phase::Ready)` when the day reads
//! CLOSED (`BranchDay::read_phase(stored, open_shifts_before) == Phase::Closed`) and `None` (leave it as stored)
//! otherwise. `open_shifts_before` is the branch's OPEN shifts before the count's shift is inserted.
//!
//! `Phase::Counting` is deliberately not used: CA-10 removes it (#626 Q5).

use xc_shift::domain::custody::branch_day::BranchDay;
use xc_shift::domain::custody::value_objects::phase::Phase;

#[test]
fn a_day_that_reads_closed_becomes_ready() {
    for (stored, open_before) in [
        (Phase::Closed, 0),
        // a stored CLOSED with a shift open is not a state the code writes, but it still reads CLOSED
        (Phase::Closed, 1),
        // yesterday's day: CUT_OFF with its last shift closed reads CLOSED
        (Phase::CutOff, 0),
    ] {
        assert_eq!(
            BranchDay::read_phase(stored, open_before),
            Phase::Closed,
            "({stored:?}, {open_before}) reads CLOSED"
        );
        assert_eq!(
            BranchDay::ready_on_open_count(stored, open_before),
            Some(Phase::Ready),
            "({stored:?}, {open_before} open shifts) → READY"
        );
    }
}

#[test]
fn any_other_day_is_left_as_stored() {
    for (stored, open_before) in [
        (Phase::Ready, 0),
        (Phase::Ready, 1),
        (Phase::Open, 0),
        (Phase::Open, 2),
        // a CUT_OFF day with another teller's shift still open is not over
        (Phase::CutOff, 1),
        (Phase::CutOff, 3),
    ] {
        assert_eq!(
            BranchDay::ready_on_open_count(stored, open_before),
            None,
            "({stored:?}, {open_before} open shifts) is left as stored"
        );
    }
}
