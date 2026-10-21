//! KC-D — the kiosk cancel's domain: `CancelReason` (the six codes, what staff may send, the
//! note) and `KioskMachine::cancel` (§2's table), the desk `get` of a cancelled request, the
//! `CANCELLED` status and the `KIOSK_CANCELLED` audit kind. Pure; no database.
//! Written before the implementation by the ticket author (claude/studio-claude-10). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md` §2, §3.1 (`CancelReason`), §3.3, §3.4 (the
//! refusal sentences), as amended by PR 4154 (KC-5 `DAY_CLOSED`; `cancel` answers an `Option`).

mod kiosk_common;

use kiosk_common::*;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::value_objects::audit_line::KioskAudit;
use xc_kiosk::domain::kiosk::value_objects::cancel_reason::{CancelReason, CancelReasonError};

use CancelReasonError as RE;
use KioskError as E;
use KioskStatus as S;

const T0: i64 = 1_791_262_800_000;

fn stored(status: KioskStatus, staff: Option<&str>) -> KioskRequest {
    KioskRequest {
        id: "req-1".to_owned(),
        status,
        created: KioskTime::from_epoch_millis(T0),
        staff: staff.map(str::to_owned),
        quote: KioskQuote {
            side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            rate: Rate::parse("35.00").unwrap(),
            unit_size: UnitSize::parse("1").unwrap(),
            line_id: None,
            foreign: Money::from_minor(10_000).unwrap(),
            thb: Money::from_minor(350_000).unwrap(),
            input_amount: Money::from_minor(10_000).unwrap(),
            payment_method: PaymentMethod::Cash,
        },
        transaction: None,
    }
}

fn front(name: &str) -> KioskActor {
    staff(name, KioskRole::Front)
}

fn manager(name: &str) -> KioskActor {
    staff(name, KioskRole::Manager)
}

fn left() -> CancelReason {
    CancelReason::CustomerLeft { note: None }
}

fn cancelled_copy(request: &KioskRequest) -> KioskRequest {
    KioskRequest {
        status: S::Cancelled,
        ..request.clone()
    }
}

// ---- CancelReason ----

#[test]
fn staff_codes_parse_and_print_back() {
    for (code, reason) in [
        ("CUSTOMER_LEFT", CancelReason::CustomerLeft { note: None }),
        (
            "CUSTOMER_CHANGED_MIND",
            CancelReason::CustomerChangedMind { note: None },
        ),
        ("DUPLICATE", CancelReason::Duplicate { note: None }),
    ] {
        assert_eq!(
            CancelReason::parse(code, None),
            Ok(reason.clone()),
            "{code}"
        );
        assert_eq!(reason.as_str(), code);
        assert_eq!(reason.note(), None);
    }
    assert_eq!(
        CancelReason::parse("OTHER", Some("ลูกค้าโทรมายกเลิก")),
        Ok(CancelReason::Other {
            note: "ลูกค้าโทรมายกเลิก".to_owned()
        })
    );
    assert_eq!(
        CancelReason::CustomerCancelled.as_str(),
        "CUSTOMER_CANCELLED"
    );
    assert_eq!(CancelReason::DayClosed.as_str(), "DAY_CLOSED");
    assert_eq!(
        CancelReason::Other {
            note: "x".to_owned()
        }
        .as_str(),
        "OTHER"
    );
}

#[test]
fn a_missing_reason_is_refused_naming_the_field() {
    assert_eq!(CancelReason::parse("", None), Err(RE::ReasonRequired));
    assert_eq!(
        CancelReason::parse("", Some("note")),
        Err(RE::ReasonRequired)
    );
    assert_eq!(
        RE::ReasonRequired.to_string(),
        "กรุณาเลือกเหตุผลการยกเลิก (reason)"
    );
}

#[test]
fn codes_staff_may_not_send_are_unknown() {
    for code in [
        "CUSTOMER_CANCELLED",
        "DAY_CLOSED",
        "customer_left",
        "CUSTOMER_LEFT ",
        "LEFT",
        "NO_SHOW",
    ] {
        assert_eq!(
            CancelReason::parse(code, None),
            Err(RE::ReasonUnknown),
            "{code:?}"
        );
    }
    assert_eq!(
        RE::ReasonUnknown.to_string(),
        "เหตุผลการยกเลิกไม่ถูกต้อง (reason)"
    );
}

#[test]
fn other_needs_text_after_trimming() {
    for note in [None, Some(""), Some("   "), Some("\n\t")] {
        assert_eq!(
            CancelReason::parse("OTHER", note),
            Err(RE::NoteRequired),
            "{note:?}"
        );
    }
    assert_eq!(RE::NoteRequired.to_string(), "กรุณาระบุรายละเอียดเหตุผล (note)");
    assert_eq!(
        CancelReason::parse("OTHER", Some("  ซ้ำกับคิว K-1  ")),
        Ok(CancelReason::Other {
            note: "ซ้ำกับคิว K-1".to_owned()
        }),
        "the note is stored trimmed"
    );
}

#[test]
fn a_note_is_optional_trimmed_and_empty_is_none_for_the_other_codes() {
    assert_eq!(
        CancelReason::parse("CUSTOMER_LEFT", Some("  เดินออกไปแล้ว ")),
        Ok(CancelReason::CustomerLeft {
            note: Some("เดินออกไปแล้ว".to_owned())
        })
    );
    assert_eq!(
        CancelReason::parse("DUPLICATE", Some("   ")),
        Ok(CancelReason::Duplicate { note: None })
    );
    assert_eq!(
        CancelReason::parse("CUSTOMER_CHANGED_MIND", Some("")),
        Ok(CancelReason::CustomerChangedMind { note: None })
    );
    assert_eq!(
        CancelReason::parse("DUPLICATE", Some(" K-2 "))
            .unwrap()
            .note(),
        Some("K-2")
    );
}

#[test]
fn a_note_counts_characters_not_bytes_and_stops_at_two_hundred() {
    let at_limit = "ก".repeat(200);
    let over = "ก".repeat(201);
    assert_eq!(
        CancelReason::parse("OTHER", Some(&at_limit)),
        Ok(CancelReason::Other {
            note: at_limit.clone()
        })
    );
    for code in [
        "OTHER",
        "CUSTOMER_LEFT",
        "DUPLICATE",
        "CUSTOMER_CHANGED_MIND",
    ] {
        assert_eq!(
            CancelReason::parse(code, Some(&over)),
            Err(RE::NoteTooLong),
            "{code}"
        );
    }
    assert_eq!(
        CancelReason::parse("OTHER", Some(&format!("  {at_limit}  "))),
        Ok(CancelReason::Other { note: at_limit }),
        "the limit applies after trimming"
    );
    assert_eq!(
        RE::NoteTooLong.to_string(),
        "รายละเอียดเหตุผลต้องไม่เกิน 200 ตัวอักษร (note)"
    );
}

#[test]
fn only_customer_left_is_a_no_show() {
    assert!(CancelReason::CustomerLeft { note: None }.is_no_show());
    assert!(
        CancelReason::CustomerLeft {
            note: Some("x".to_owned())
        }
        .is_no_show()
    );
    for reason in [
        CancelReason::CustomerCancelled,
        CancelReason::CustomerChangedMind { note: None },
        CancelReason::Duplicate { note: None },
        CancelReason::Other {
            note: "x".to_owned(),
        },
        CancelReason::DayClosed,
    ] {
        assert!(!reason.is_no_show(), "{reason:?}");
    }
}

// ---- KioskMachine::cancel (§2) ----

#[test]
fn staff_cancel_a_waiting_request_and_nobody_is_named_its_holder() {
    let waiting = stored(S::Waiting, None);
    for actor in [
        front("front1"),
        manager("mgr1"),
        staff("owner", KioskRole::Owner),
    ] {
        assert_eq!(
            KioskMachine::cancel(Some(&waiting), &actor, &left()),
            Ok(Some(cancelled_copy(&waiting))),
            "{actor:?}"
        );
    }
}

#[test]
fn the_customer_cancels_a_waiting_request_only_with_the_kiosks_own_reason() {
    let waiting = stored(S::Waiting, None);
    assert_eq!(
        KioskMachine::cancel(
            Some(&waiting),
            &KioskActor::Customer,
            &CancelReason::CustomerCancelled
        ),
        Ok(Some(cancelled_copy(&waiting)))
    );
    for reason in [
        left(),
        CancelReason::Duplicate { note: None },
        CancelReason::DayClosed,
    ] {
        assert_eq!(
            KioskMachine::cancel(Some(&waiting), &KioskActor::Customer, &reason),
            Err(E::NotCancellable),
            "{reason:?}"
        );
    }
}

#[test]
fn staff_never_cancel_with_the_customers_reason() {
    let waiting = stored(S::Waiting, None);
    assert_eq!(
        KioskMachine::cancel(
            Some(&waiting),
            &front("front1"),
            &CancelReason::CustomerCancelled
        ),
        Err(E::NotCancellable)
    );
}

#[test]
fn the_customer_cannot_cancel_once_staff_hold_it() {
    for status in [S::Accepted, S::IdChecked] {
        let held = stored(status, Some("front1"));
        assert_eq!(
            KioskMachine::cancel(
                Some(&held),
                &KioskActor::Customer,
                &CancelReason::CustomerCancelled
            ),
            Err(E::NotCancellable),
            "{status:?}"
        );
    }
}

#[test]
fn the_holder_or_a_branch_manager_cancels_a_held_request_and_the_holder_is_kept() {
    for status in [S::Accepted, S::IdChecked] {
        let held = stored(status, Some("front1"));
        let moved = cancelled_copy(&held);
        assert_eq!(
            KioskMachine::cancel(Some(&held), &front("front1"), &left()),
            Ok(Some(moved.clone())),
            "holder, {status:?}"
        );
        assert_eq!(
            KioskMachine::cancel(Some(&held), &manager("mgr1"), &left()),
            Ok(Some(moved.clone())),
            "manager, {status:?}"
        );
        assert_eq!(
            moved.staff.as_deref(),
            Some("front1"),
            "staff keeps meaning who held it, never who cancelled"
        );
    }
}

#[test]
fn another_teller_cannot_cancel_a_held_request() {
    for status in [S::Accepted, S::IdChecked] {
        let held = stored(status, Some("front1"));
        for actor in [
            front("front2"),
            staff("owner", KioskRole::Owner),
            staff("rates1", KioskRole::Rates),
            staff("back1", KioskRole::Backend),
            staff("other1", KioskRole::Other),
        ] {
            assert_eq!(
                KioskMachine::cancel(Some(&held), &actor, &left()),
                Err(E::NotYourRequest),
                "{actor:?} {status:?}"
            );
        }
    }
}

#[test]
fn quoted_expired_and_completed_requests_are_not_cancellable() {
    for (status, staff_name) in [
        (S::Quoted, None),
        (S::Expired, None),
        (S::Completed, Some("front1")),
    ] {
        let request = stored(status, staff_name);
        for actor in [front("front1"), manager("mgr1")] {
            assert_eq!(
                KioskMachine::cancel(Some(&request), &actor, &left()),
                Err(E::NotCancellable),
                "{status:?} {actor:?}"
            );
        }
        assert_eq!(
            KioskMachine::cancel(
                Some(&request),
                &KioskActor::Customer,
                &CancelReason::CustomerCancelled
            ),
            Err(E::NotCancellable),
            "customer, {status:?}"
        );
    }
}

#[test]
fn a_cancelled_request_cancelled_again_is_a_repeat_with_nothing_to_write() {
    for staff_name in [None, Some("front1")] {
        let done = stored(S::Cancelled, staff_name);
        assert_eq!(
            KioskMachine::cancel(Some(&done), &front("front2"), &left()),
            Ok(None)
        );
        assert_eq!(
            KioskMachine::cancel(Some(&done), &manager("mgr1"), &left()),
            Ok(None)
        );
        assert_eq!(
            KioskMachine::cancel(
                Some(&done),
                &KioskActor::Customer,
                &CancelReason::CustomerCancelled
            ),
            Ok(None)
        );
    }
}

#[test]
fn no_request_is_not_found() {
    assert_eq!(
        KioskMachine::cancel(None, &front("front1"), &left()),
        Err(E::RequestNotFound)
    );
}

/// Restated on `feat/4147-kiosk-day-close` by the sub-controller claude/studio-claude-9, replacing
/// `day_close_cancels_only_a_waiting_request`: the owner ruled on 2026-10-21 (option A on #4147,
/// recorded by claude/553782) that `DAY_CLOSED` also cancels a request a teller took but never
/// finished — *"ยกเลิกด้วย เหตุผล ปิดวัน"* (design `queue-day-close.md` §7.0 K-4). The holder
/// check of a held request is unchanged: a manager, which the day-close actor is, passes it.
#[test]
fn day_close_cancels_waiting_accepted_and_id_checked() {
    let waiting = stored(S::Waiting, None);
    assert_eq!(
        KioskMachine::cancel(Some(&waiting), &manager("mgr1"), &CancelReason::DayClosed),
        Ok(Some(cancelled_copy(&waiting)))
    );
    for status in [S::Accepted, S::IdChecked] {
        for holder in ["mgr1", "front2"] {
            let held = stored(status, Some(holder));
            assert_eq!(
                KioskMachine::cancel(Some(&held), &manager("mgr1"), &CancelReason::DayClosed),
                Ok(Some(cancelled_copy(&held))),
                "K-4: a request at the counter is cancelled at day close: {status:?} held by {holder}"
            );
        }
        let held = stored(status, Some("front2"));
        assert_eq!(
            KioskMachine::cancel(Some(&held), &front("front1"), &CancelReason::DayClosed),
            Err(E::NotYourRequest),
            "the holder-or-manager check still applies: {status:?}"
        );
    }
    for status in [S::Quoted, S::Completed] {
        assert_eq!(
            KioskMachine::cancel(
                Some(&stored(status, None)),
                &manager("mgr1"),
                &CancelReason::DayClosed
            ),
            Err(E::NotCancellable),
            "{status:?}"
        );
    }
}

// ---- around the cancel ----

#[test]
fn the_desk_get_of_a_cancelled_request_says_it_was_cancelled() {
    for staff_name in [None, Some("front1")] {
        let done = stored(S::Cancelled, staff_name);
        assert_eq!(
            KioskMachine::check_get(Some(&done), &front("front1")),
            Err(E::KioskCancelled)
        );
    }
    assert_eq!(E::KioskCancelled.to_string(), "คำขอนี้ถูกยกเลิกแล้ว");
}

#[test]
fn a_cancelled_request_is_seen_as_cancelled_whatever_its_age() {
    let done = stored(S::Cancelled, None);
    let identity =
        KioskMachine::identity(Some(&done), KioskTime::from_epoch_millis(T0 + 3_600_000))
            .expect("identity");
    assert_eq!(identity.status, S::Cancelled);
    assert_eq!(identity.status.as_str(), "CANCELLED");
}

#[test]
fn the_refusals_are_worded_as_the_design_lists_them() {
    assert_eq!(E::NotCancellable.to_string(), "คำขอนี้ยกเลิกไม่ได้");
    assert_eq!(E::NotYourRequest.to_string(), "คำขอนี้มีพนักงานคนอื่นรับเรื่องอยู่");
    assert_eq!(
        E::StaleVersion.to_string(),
        "คำขอนี้เปลี่ยนสถานะแล้ว กรุณาโหลดล่าสุด"
    );
    assert_eq!(
        E::CustomerCancelRefused.to_string(),
        "ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน"
    );
}

#[test]
fn the_cancel_audit_line_is_kiosk_cancelled() {
    let what = KioskAudit::Cancelled {
        from: S::Waiting,
        reason: CancelReason::Other {
            note: "x".to_owned(),
        },
    };
    assert_eq!(what.kind(), "KIOSK_CANCELLED");
}
