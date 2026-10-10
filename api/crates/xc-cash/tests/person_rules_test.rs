//! CA-2 (#619 Part 1): `PersonRules`, the cash rules about *which person* rather than which role (design
//! `docs/design/blueprint/cash-shift-transfer.md` §3, §4.3, class `PersonRules` §5; §9.1 row CA-2).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests:
//! - `xc_cash::domain::person_rules::PersonRules` with the associated functions
//!   `verifier_is_not_counter(counter: &str, verifier: &Actor)`, `receiver_verifies_handover(receiver: &str, actor:
//!   &Actor)` and `teller_owns_shift(shift_teller: &str, actor: &Actor)`, each `Result<(), CashError>`. The `&str`
//!   is a `users.id`; the actor's id is `Actor.user_id` (`xc-auth/src/domain/access/actor.rs`).
//! - `xc_cash::domain::error::CashError` with `SameCounter`, `NotReceiver`, `NotOwnShift`, deriving `Debug` and
//!   `PartialEq`.
//!
//! The role × permission matrix is PR 589's test, not this ticket's (design §9.1).

use chrono::{TimeZone, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::domain::error::CashError;
use xc_cash::domain::person_rules::PersonRules;

const ORG: &str = "00000000-0000-4000-8000-00000000aa01";
const ALICE: &str = "00000000-0000-4000-8000-0000000000a1";
const BOB: &str = "00000000-0000-4000-8000-0000000000b2";

fn actor(user_id: &str, roles: &[Role]) -> Actor {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        user_id,
        ORG,
        roles
            .iter()
            .map(|r| ScopedRole::new(*r, None, from, None))
            .collect(),
        None,
    )
}

// ── verifier ≠ counter ──────────────────────────────────────────────────────

#[test]
fn a_count_verified_by_someone_else_passes() {
    assert_eq!(
        PersonRules::verifier_is_not_counter(ALICE, &actor(BOB, &[Role::BranchManager])),
        Ok(())
    );
}

#[test]
fn a_count_verified_by_its_counter_is_refused() {
    assert_eq!(
        PersonRules::verifier_is_not_counter(ALICE, &actor(ALICE, &[Role::BranchManager])),
        Err(CashError::SameCounter)
    );
}

#[test]
fn an_executive_or_branch_manager_is_not_exempt_from_counter_is_not_verifier() {
    for roles in [
        &[Role::Executive][..],
        &[Role::BranchManager][..],
        &[Role::Executive, Role::Admin, Role::BranchManager][..],
    ] {
        assert_eq!(
            PersonRules::verifier_is_not_counter(ALICE, &actor(ALICE, roles)),
            Err(CashError::SameCounter),
            "{roles:?}"
        );
    }
}

// ── the receiver verifies a hand-over ──────────────────────────────────────

#[test]
fn a_hand_over_verified_by_its_receiver_passes() {
    assert_eq!(
        PersonRules::receiver_verifies_handover(BOB, &actor(BOB, &[Role::Teller])),
        Ok(())
    );
}

#[test]
fn a_hand_over_verified_by_anyone_but_its_receiver_is_refused() {
    for roles in [
        &[Role::Teller][..],
        &[Role::BranchManager][..],
        &[Role::Executive][..],
    ] {
        assert_eq!(
            PersonRules::receiver_verifies_handover(BOB, &actor(ALICE, roles)),
            Err(CashError::NotReceiver),
            "{roles:?}"
        );
    }
}

// ── a teller acts on their own shift ───────────────────────────────────────

#[test]
fn a_teller_acting_on_their_own_shift_passes() {
    assert_eq!(
        PersonRules::teller_owns_shift(ALICE, &actor(ALICE, &[Role::Teller])),
        Ok(())
    );
}

#[test]
fn a_teller_acting_on_another_tellers_shift_is_refused() {
    for roles in [&[Role::Teller][..], &[Role::BranchManager][..]] {
        assert_eq!(
            PersonRules::teller_owns_shift(ALICE, &actor(BOB, roles)),
            Err(CashError::NotOwnShift),
            "{roles:?}"
        );
    }
}

// ── the three refusals are three different answers ─────────────────────────

#[test]
fn the_three_refusals_are_distinct() {
    assert_ne!(CashError::SameCounter, CashError::NotReceiver);
    assert_ne!(CashError::SameCounter, CashError::NotOwnShift);
    assert_ne!(CashError::NotReceiver, CashError::NotOwnShift);
}
