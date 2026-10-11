//! CA-5 (#621 Part 1): the transfer core: the five states, the kinds held as `Arc<dyn TransferEnds>`, and the BRANCH
//! kind's ends and postings (design `docs/design/blueprint/cash-shift-transfer.md` §2 C4–C6, §4.2, §4.5, §4.6, §9.1
//! CA-5).
//!
//! Written before the implementation by claude/adc848e6, on the lane controller's order. Do not edit, delete or weaken
//! these tests; add your own in another file.
//!
//! **This file is the pure half.** It needs no database. The flow half (approval within and above the limit, dispatch
//! beyond the expected balance, the receipt exception over the real tables) waits on the answers to the gaps posted on
//! #621 (issuecomment-6093019876, -6093028038) and lands in this file's second commit.
//!
//! [required] by these tests (ticket §7, design §4.5):
//! - `xc_cash::domain::transfer::cash_transfer`:
//!   - `TransferKind::{Branch, DrawerToSafe, SafeToDrawer, ExternalIn, ExternalOut}`, `Clone + Copy + Debug +
//!     PartialEq + Eq + Hash`, with `as_str(self) -> &'static str` = the `cash_transfers.kind` spelling and
//!     `parse(&str) -> Option<TransferKind>`.
//!   - `TransferStatus::{Draft, Approved, Dispatched, Received, Reconciled}`, `Clone + Copy + Debug + PartialEq + Eq`,
//!     with `as_str` / `parse` as the `cash_transfers.status` spelling, and `ALL: [TransferStatus; 5]` in order.
//!   - `TransferStep::{Approve, Dispatch, Receive, Reconcile}`, `Clone + Copy + Debug + PartialEq + Eq`, with
//!     `ALL: [TransferStep; 4]`.
//!   - `TransferStatus::next(self, step: TransferStep) -> Result<TransferStatus, TransferError>`: the only state
//!     machine. A step from the wrong state is `TransferError::IllegalTransition { from, step }`.
//!   - `TransferError`: `Debug + PartialEq + Display`; it reaches callers as `AppError::Domain(e.to_string())` (the
//!     `CashError` convention).
//!   - `TransferLine { currency: CurrencyCode, denomination_id: i64, face_value: Money, qty_sent: i32, qty_received:
//!     Option<i32> }`.
//!   - `CashTransfer { id: String, kind: TransferKind, from_branch: BranchId, to_branch: Option<BranchId>, from_shift:
//!     Option<String>, to_shift: Option<String>, status: TransferStatus, created_by: String, lines: Vec<TransferLine> }`
//!     (public fields).
//!   - `NewTransfer { kind, from_branch, to_branch, from_shift, to_shift, lines: Vec<NewTransferLine>, note:
//!     Option<String>, idempotency_key: String }`; `NewTransferLine { currency: CurrencyCode, denomination_id: i64, qty:
//!     i32 }`.
//! - `xc_cash::domain::transfer::ends::TransferEnds`, exactly design §4.5 (`kind`, `check_create`, `check_receive`,
//!   `on_dispatch`, `on_receive`), dyn-safe.
//! - `xc_cash::domain::transfer::kinds::TransferKinds`: `TransferKinds::standard() -> TransferKinds` holds every kind
//!   this build has, as `Arc<dyn TransferEnds>`; `get(&self, kind: TransferKind) -> Option<Arc<dyn TransferEnds>>`;
//!   `kinds(&self) -> Vec<TransferKind>` sorted by `as_str`.
//! - Postings use #619's `xc_cash::domain::posting::{CashPlace, PostingRequest}` (design §4.6). One request per
//!   currency; `amount` = Σ qty × `face_value` over that currency's lines; `source` = the transfer's id.
//!   - BRANCH dispatch: `Safe(from_branch)` → `InTransit(id)`, over `qty_sent`.
//!   - BRANCH receipt: `InTransit(id)` → `Safe(to_branch)`, over `qty_received` (the counted notes, C4).

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::domain::posting::{CashPlace, PostingRequest};
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferError, TransferKind, TransferLine,
    TransferStatus, TransferStep,
};
use xc_cash::domain::transfer::ends::TransferEnds;
use xc_cash::domain::transfer::kinds::TransferKinds;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::parse("EUR").unwrap()
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap()
}

/// A user holding `roles`, each on `branch` (None = organisation-wide), valid since 2026-01-01.
fn actor(user: &str, roles: &[(Role, Option<&str>)]) -> Actor {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        user,
        ORG,
        roles
            .iter()
            .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), from, None))
            .collect(),
        None,
    )
}

fn line(
    currency: CurrencyCode,
    denomination_id: i64,
    face: &str,
    sent: i32,
    received: Option<i32>,
) -> TransferLine {
    TransferLine {
        currency,
        denomination_id,
        face_value: money(face),
        qty_sent: sent,
        qty_received: received,
    }
}

fn branch_transfer(lines: Vec<TransferLine>, status: TransferStatus) -> CashTransfer {
    CashTransfer {
        id: "t-1".to_owned(),
        kind: TransferKind::Branch,
        from_branch: BranchId::new("B1"),
        to_branch: Some(BranchId::new("B2")),
        from_shift: None,
        to_shift: None,
        status,
        created_by: "creator".to_owned(),
        lines,
    }
}

fn new_branch(to: Option<&str>, from_shift: Option<&str>, to_shift: Option<&str>) -> NewTransfer {
    NewTransfer {
        kind: TransferKind::Branch,
        from_branch: BranchId::new("B1"),
        to_branch: to.map(BranchId::new),
        from_shift: from_shift.map(str::to_owned),
        to_shift: to_shift.map(str::to_owned),
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: 1,
            qty: 3,
        }],
        note: None,
        idempotency_key: "k-1".to_owned(),
    }
}

fn branch() -> Arc<dyn TransferEnds> {
    TransferKinds::standard()
        .get(TransferKind::Branch)
        .expect("BRANCH is registered")
}

/// Postings as comparable tuples (from, to, currency, amount, source).
fn flat(reqs: &[PostingRequest]) -> HashSet<(String, String, String, String, String)> {
    reqs.iter()
        .map(|r| {
            (
                format!("{:?}", r.from),
                format!("{:?}", r.to),
                r.currency.as_str().to_owned(),
                format!("{:.2}", r.quantity.value()),
                r.source.source_id.clone(),
            )
        })
        .collect()
}

fn post(
    from: CashPlace,
    to: CashPlace,
    currency: CurrencyCode,
    amount: &str,
) -> (String, String, String, String, String) {
    (
        format!("{from:?}"),
        format!("{to:?}"),
        currency.as_str().to_owned(),
        money(amount).to_string(),
        "t-1".to_owned(),
    )
}

// ── the five states (§06, C4) ─────────────────────────────────────────────

#[test]
fn the_statuses_are_the_blueprints_five_in_order_and_spelled_as_the_table() {
    let spelled: Vec<&str> = TransferStatus::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        spelled,
        ["DRAFT", "APPROVED", "DISPATCHED", "RECEIVED", "RECONCILED"]
    );
    for s in TransferStatus::ALL {
        assert_eq!(TransferStatus::parse(s.as_str()), Some(s));
    }
    for other in ["CANCELLED", "REJECTED", "PENDING", "draft", ""] {
        assert_eq!(
            TransferStatus::parse(other),
            None,
            "no sixth state: {other:?}"
        );
    }
}

#[test]
fn the_four_steps_walk_the_five_states_in_order() {
    let mut s = TransferStatus::Draft;
    for (step, expected) in [
        (TransferStep::Approve, TransferStatus::Approved),
        (TransferStep::Dispatch, TransferStatus::Dispatched),
        (TransferStep::Receive, TransferStatus::Received),
        (TransferStep::Reconcile, TransferStatus::Reconciled),
    ] {
        s = s
            .next(step)
            .unwrap_or_else(|e| panic!("{step:?} from {s:?}: {e}"));
        assert_eq!(s, expected);
    }
}

#[test]
fn every_other_step_from_every_state_is_refused() {
    let legal = [
        (TransferStatus::Draft, TransferStep::Approve),
        (TransferStatus::Approved, TransferStep::Dispatch),
        (TransferStatus::Dispatched, TransferStep::Receive),
        (TransferStatus::Received, TransferStep::Reconcile),
    ];
    let mut refused = 0;
    for from in TransferStatus::ALL {
        for step in TransferStep::ALL {
            if legal.contains(&(from, step)) {
                continue;
            }
            assert_eq!(
                from.next(step),
                Err(TransferError::IllegalTransition { from, step }),
                "{step:?} from {from:?}"
            );
            refused += 1;
        }
    }
    assert_eq!(refused, 5 * 4 - 4, "every pair but the four legal ones");
}

#[test]
fn reconciled_is_final() {
    for step in TransferStep::ALL {
        assert!(TransferStatus::Reconciled.next(step).is_err(), "{step:?}");
    }
}

#[test]
fn an_illegal_jump_reads_as_a_sentence_for_the_caller() {
    let e = TransferStatus::Draft
        .next(TransferStep::Receive)
        .unwrap_err();
    assert!(
        !e.to_string().trim().is_empty(),
        "AppError::Domain carries Display: {e:?}"
    );
}

// ── the kinds, held as Arc<dyn TransferEnds> (§4.5, ♻️) ────────────────────

#[test]
fn the_kinds_are_spelled_as_the_table() {
    let all = [
        (TransferKind::Branch, "BRANCH"),
        (TransferKind::DrawerToSafe, "DRAWER_TO_SAFE"),
        (TransferKind::SafeToDrawer, "SAFE_TO_DRAWER"),
        (TransferKind::ExternalIn, "EXTERNAL_IN"),
        (TransferKind::ExternalOut, "EXTERNAL_OUT"),
    ];
    for (k, text) in all {
        assert_eq!(k.as_str(), text);
        assert_eq!(TransferKind::parse(text), Some(k));
    }
    assert_eq!(TransferKind::parse("SAFE"), None);
}

#[test]
fn branch_is_held_as_an_arc_dyn_transfer_ends_keyed_by_its_own_kind() {
    let ends: Arc<dyn TransferEnds> = branch();
    assert_eq!(ends.kind(), TransferKind::Branch);
    let kinds = TransferKinds::standard();
    for k in kinds.kinds() {
        assert_eq!(
            kinds.get(k).expect("listed means held").kind(),
            k,
            "the registry key is the kind() it holds"
        );
    }
    assert!(kinds.kinds().contains(&TransferKind::Branch));
}

// ── BRANCH: ends (§3.1, §4.1) ─────────────────────────────────────────────

#[test]
fn a_branch_transfer_is_created_by_a_branch_manager_or_finance_of_the_source_branch() {
    let t = new_branch(Some("B2"), None, None);
    for who in [
        actor("bm", &[(Role::BranchManager, Some("B1"))]),
        actor("fin", &[(Role::Finance, Some("B1"))]),
        actor("fin-org", &[(Role::Finance, None)]),
    ] {
        assert_eq!(branch().check_create(&who, &t), Ok(()), "{}", who.user_id);
    }
}

#[test]
fn a_branch_transfer_is_not_created_by_a_teller_or_by_a_manager_of_another_branch() {
    let t = new_branch(Some("B2"), None, None);
    for who in [
        actor("teller", &[(Role::Teller, Some("B1"))]),
        actor("bm-b2", &[(Role::BranchManager, Some("B2"))]),
        actor("exec", &[(Role::Executive, None)]),
    ] {
        assert!(branch().check_create(&who, &t).is_err(), "{}", who.user_id);
    }
}

#[test]
fn a_branch_transfer_names_another_branch_and_no_shift() {
    let bm = actor("bm", &[(Role::BranchManager, Some("B1"))]);
    for (what, t) in [
        ("no destination", new_branch(None, None, None)),
        ("to itself", new_branch(Some("B1"), None, None)),
        ("a source drawer", new_branch(Some("B2"), Some("s-1"), None)),
        (
            "a destination drawer",
            new_branch(Some("B2"), None, Some("s-2")),
        ),
    ] {
        assert!(branch().check_create(&bm, &t).is_err(), "{what}");
    }
}

#[test]
fn a_branch_transfer_is_received_by_a_person_of_the_destination_who_is_not_the_creator() {
    let t = branch_transfer(
        vec![line(usd(), 1, "100", 3, None)],
        TransferStatus::Dispatched,
    );
    assert_eq!(
        branch().check_receive(&actor("bm-b2", &[(Role::BranchManager, Some("B2"))]), &t),
        Ok(())
    );
    assert_eq!(
        branch().check_receive(&actor("fin-b2", &[(Role::Finance, Some("B2"))]), &t),
        Ok(())
    );
    for who in [
        actor("bm-b1", &[(Role::BranchManager, Some("B1"))]),
        actor("teller-b2", &[(Role::Teller, Some("B2"))]),
        actor("creator", &[(Role::Finance, None)]),
    ] {
        assert!(branch().check_receive(&who, &t).is_err(), "{}", who.user_id);
    }
}

// ── BRANCH: postings (§4.6, C7) ───────────────────────────────────────────

#[test]
fn a_branch_dispatch_moves_the_sent_notes_from_the_source_safe_into_transit_one_request_per_currency()
 {
    let t = branch_transfer(
        vec![
            line(usd(), 1, "100", 3, None),
            line(usd(), 2, "20", 4, None),
            line(eur(), 7, "50", 2, None),
        ],
        TransferStatus::Approved,
    );
    let from = CashPlace::Safe(BranchId::new("B1"));
    let transit = CashPlace::InTransit("t-1".to_owned());
    assert_eq!(
        flat(&branch().on_dispatch(&t)),
        HashSet::from([
            post(from.clone(), transit.clone(), usd(), "380"),
            post(from, transit, eur(), "100"),
        ])
    );
}

#[test]
fn a_branch_receipt_moves_the_counted_notes_from_transit_into_the_destination_safe() {
    // 3 × 100 USD sent, 2 counted: the receiver's safe gains what was counted (C4: never forced to match)
    let t = branch_transfer(
        vec![line(usd(), 1, "100", 3, Some(2))],
        TransferStatus::Dispatched,
    );
    assert_eq!(
        flat(&branch().on_receive(&t)),
        HashSet::from([post(
            CashPlace::InTransit("t-1".to_owned()),
            CashPlace::Safe(BranchId::new("B2")),
            usd(),
            "200"
        )])
    );
}

#[test]
fn a_currency_whose_counted_total_is_zero_posts_nothing() {
    let t = branch_transfer(
        vec![
            line(usd(), 1, "100", 3, Some(0)),
            line(eur(), 7, "50", 1, Some(1)),
        ],
        TransferStatus::Dispatched,
    );
    let got = flat(&branch().on_receive(&t));
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got.iter().all(|p| p.2 == "EUR"));
}
