//! The two entry paths the 10-06 bundle adds: who may read and save the payment settings,
//! and who may ask the passport drawer.
//!
//! Provided with ticket POL (design `docs/design/xchange-v3.md` §5.5.7 *Policy*, §5.5.2 rows
//! 1, 6, 7 and §5.5.5 rows 1, 4). Written before the implementation. Do not edit, delete or
//! weaken.
//!
//! The answers are v2's, from the 10-06 bundle (`XC-Desk-v2-Passport-Drawer-20261006.zip`,
//! sha256 `6514b4c83c74916edba23d48e6d650051babe567c71e98bc84e7e0574375f570`):
//! - `POST /api/payment-methods` (`server.mjs:8154`, a "user" route: nobody signed in is told
//!   `กรุณาเข้าสู่ระบบ` by the router, `:8289`; then `payment-settings.mjs:47-48`): `list` and
//!   `save` are the only names, anything else is `คำขอไม่ถูกต้อง` to everybody signed in;
//!   `save` is the owner's, anybody else is told `เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้` (403).
//! - `POST /api/drawer-passport` (`server.mjs:8153`, a "user" route; `passport-drawer.mjs:15`):
//!   OWNER, FRONT, BACKEND, MANAGER; anybody else `บัญชีนี้ไม่มีสิทธิ์เปิดลิ้นชัก` (400). The body
//!   carries no action name.
//!
//! Every caller's answer is also in `fixtures/v2_policy_cases.json` (paths `PaymentMethods`,
//! `DrawerPassport`), which `policy_test.rs` replays whole.
//!
//! Moved by the author of OR-4 (#550, `docs/design/blueprint/org-roles-branches.md` §4.2, §4.3)
//! onto the Blueprint's users: the callers are the owner's user (EXECUTIVE + ADMIN, B3) and one
//! user per Blueprint role. One answer changes, C5 (§02 ADMIN "ตั้งค่าระบบ"): `save` is
//! SystemSettings, EXECUTIVE and ADMIN, so ADMIN (ex-SYSTEM) now saves the payment settings. The
//! passport drawer is DrawerPassport, the B1 translation of OWNER, FRONT, BACKEND, MANAGER:
//! EXECUTIVE, TELLER, BRANCH_MANAGER — no change.

mod policy_common;

use policy_common::*;

const B2: &str = "B2";

fn everybody_signed_in() -> Vec<Actor> {
    let mut all = vec![owner()];
    all.extend(BLUEPRINT.into_iter().map(|r| staff(r, B2)));
    all
}

fn label(who: &Actor) -> String {
    who.roles
        .iter()
        .map(|r| r.role.as_str())
        .collect::<Vec<_>>()
        .join("+")
}

// ── payment methods ──────────────────────────────────────────────────────────

#[test]
fn nobody_signed_in_is_asked_to_sign_in_before_the_payment_settings() {
    for name in ["list", "save", "nope", ""] {
        assert_eq!(
            said(ask(None, EntryPath::PaymentMethods, name)),
            (SIGN_IN.to_owned(), 401),
            "{name:?}"
        );
    }
}

#[test]
fn the_payment_settings_are_read_by_anybody_signed_in() {
    for who in everybody_signed_in() {
        assert_eq!(
            ask(Some(&who), EntryPath::PaymentMethods, "list"),
            Ok(()),
            "{}",
            label(&who)
        );
    }
}

#[test]
fn the_payment_settings_are_saved_by_executive_and_admin() {
    // C5: SystemSettings (was: the owner alone)
    assert_eq!(
        ask(Some(&owner()), EntryPath::PaymentMethods, "save"),
        Ok(())
    );
    for r in [Role::Executive, Role::Admin] {
        assert_eq!(
            ask(Some(&staff(r, B2)), EntryPath::PaymentMethods, "save"),
            Ok(()),
            "{}",
            r.as_str()
        );
    }
    for r in BLUEPRINT
        .into_iter()
        .filter(|r| !matches!(r, Role::Executive | Role::Admin))
    {
        assert_eq!(
            said(ask(Some(&staff(r, B2)), EntryPath::PaymentMethods, "save")),
            (PAYMENT_OWNER_ONLY.to_owned(), 403),
            "{}",
            r.as_str()
        );
    }
}

#[test]
fn a_name_the_payment_settings_do_not_know_is_a_bad_request_to_everybody_signed_in() {
    // v2 checks the name (payment-settings.mjs:47) before who may save (:48)
    for who in everybody_signed_in() {
        for name in ["nope", "", "List", "SAVE", "save ", "delete", "history"] {
            assert_eq!(
                said(ask(Some(&who), EntryPath::PaymentMethods, name)),
                (INVALID_REQUEST.to_owned(), 400),
                "{name:?} by {}",
                label(&who)
            );
        }
    }
}

// ── the passport drawer ──────────────────────────────────────────────────────

#[test]
fn the_passport_drawer_is_asked_by_the_desk_alone() {
    let mut desk = vec![owner()];
    desk.extend([Role::Executive, Role::BranchManager, Role::Teller].map(|r| staff(r, B2)));
    for who in &desk {
        assert_eq!(
            ask(Some(who), EntryPath::DrawerPassport, "drawer"),
            Ok(()),
            "{}",
            label(who)
        );
    }
    for r in [
        Role::Admin,
        Role::RateManager,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ] {
        assert_eq!(
            said(ask(
                Some(&staff(r, B2)),
                EntryPath::DrawerPassport,
                "drawer"
            )),
            (DRAWER_ROLE_NOT_ALLOWED.to_owned(), 400),
            "{}",
            r.as_str()
        );
    }
    assert_eq!(
        said(ask(None, EntryPath::DrawerPassport, "drawer")),
        (SIGN_IN.to_owned(), 401)
    );
}

#[test]
fn the_passport_drawer_does_not_read_the_action() {
    let mut callers: Vec<Option<Actor>> = vec![None];
    callers.extend(everybody_signed_in().into_iter().map(Some));
    for who in callers {
        let want = answer(ask(who.as_ref(), EntryPath::DrawerPassport, "drawer"));
        for name in ["", "nope", "list", "save", "accept"] {
            assert_eq!(
                answer(ask(who.as_ref(), EntryPath::DrawerPassport, name)),
                want,
                "{name:?}"
            );
        }
    }
}

// ── the two refusals ─────────────────────────────────────────────────────────

#[test]
fn the_two_new_refusals_say_v2s_sentences_with_v2s_statuses() {
    assert_eq!(
        (
            PolicyError::PaymentOwnerOnly.to_string(),
            PolicyError::PaymentOwnerOnly.status()
        ),
        (PAYMENT_OWNER_ONLY.to_owned(), 403)
    );
    assert_eq!(
        (
            PolicyError::DrawerRoleNotAllowed.to_string(),
            PolicyError::DrawerRoleNotAllowed.status()
        ),
        (DRAWER_ROLE_NOT_ALLOWED.to_owned(), 400)
    );
}

#[test]
fn the_recording_holds_both_paths_for_every_caller() {
    let f = mapped_fixture();
    for (p, names) in [
        ("PaymentMethods", vec!["list", "save", "nope", ""]),
        ("DrawerPassport", vec!["drawer"]),
    ] {
        let rows: Vec<_> = f.cases.iter().filter(|c| c.path == p).collect();
        assert_eq!(rows.len(), 11 * names.len(), "{p}");
        for c in rows {
            assert!(names.contains(&c.action.as_str()), "{p} {:?}", c.action);
        }
    }
}
