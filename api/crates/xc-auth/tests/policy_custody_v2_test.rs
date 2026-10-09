//! The custody rows of `policy_test.rs`, moved here verbatim by the author of OR-4 (#550, design
//! `docs/design/blueprint/org-roles-branches.md` §4.2 "Custody ops*" and "Rpc adminOpenShift,
//! adminCloseShift" → #582, §4.3 "move to #582's tickets").
//!
//! OR-4 does not re-express them: who may do each custody step is #582's table (PR 590). Until
//! #582's tickets replace them, the custody engine still asks the v2 rule
//! `Policy::allow(&v2 Actor, path, action)`, and these assertions are that rule's, unchanged
//! (`policy_test.rs` at `8234e77`: `:310`, `:523`, `:720`, `:744`, `:762`, and the custody lines of
//! `:417`, `:437`, `:481`). Do not edit, delete or weaken; #582's tickets retire this file.

#![allow(dead_code)]

use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;

const B2: &str = "B2";

const EMPLOYEE_AUTH: &str = "EMPLOYEE_AUTH_REQUIRED";
const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";
const ROLE_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
const ROLE_NOT_ALLOWED_HERE: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";
const OWNER_ENABLES: &str = "เจ้าของเป็นผู้เปิดใช้ขั้นตอน";
const SIGN_NOT_ALLOWED: &str = "ไม่มีสิทธิ์ลงชื่อ";
const BACKEND_USES_COMMIT: &str = "Backend ใช้ปุ่มตรวจและบันทึกงาน";
const EMPLOYEE_ACCOUNT: &str = "ต้องใช้บัญชีพนักงาน";
const CLOCK_IN_FIRST: &str = "ลงเวลาเข้าก่อนทำรายการ";

fn owner() -> Actor {
    Actor::owner("owner")
}

fn staff(r: Role, own: &str) -> Actor {
    Actor::staff(format!("E-{}", r.as_str()), r, BranchId::new(own)).expect("a staff role")
}

fn front() -> Actor {
    staff(Role::Front, B2)
}

fn ask(who: Option<&Actor>, on: EntryPath, name: &str) -> Result<(), PolicyError> {
    Policy::allow(who, on, &Action::new(name))
}

fn said(got: Result<(), PolicyError>) -> (String, u16) {
    let e = got.expect_err("must be refused");
    (e.to_string(), e.status())
}

fn allowed_roles(on: EntryPath, name: &str) -> Vec<&'static str> {
    Role::STAFF
        .into_iter()
        .filter(|r| ask(Some(&staff(*r, B2)), on, name).is_ok())
        .map(Role::as_str)
        .collect()
}

#[test]
fn opening_and_closing_a_shift_depends_on_the_path() {
    for name in ["adminOpenShift", "adminCloseShift"] {
        assert_eq!(allowed_roles(EntryPath::Rpc, name), ["BACKEND", "MANAGER"]);
        assert!(allowed_roles(EntryPath::StaffRpc, name).is_empty());
    }
    // (its two `adminMoveStock` lines are OR-4's row C3: `policy_test.rs`
    // `moving_stock_is_branch_manager_and_finance`)
}

#[test]
fn the_owner_opens_and_closes_shifts_on_the_owner_rpc() {
    // from `the_owner_may_do_all_eleven_actions_of_the_owner_rpc` (`policy_test.rs:397`)
    for name in ["adminOpenShift", "adminCloseShift"] {
        assert_eq!(ask(Some(&owner()), EntryPath::Rpc, name), Ok(()), "{name}");
    }
}

#[test]
fn the_staff_bridge_lets_through_whatever_starts_with_ops() {
    // the custody module, not the bridge, decides those — including names it does not know
    for r in Role::STAFF {
        let e = staff(r, B2);
        for name in ["opsClock", "opsCommit", "opsNope", "ops", "opsenable"] {
            assert_eq!(ask(Some(&e), EntryPath::StaffRpc, name), Ok(()), "{name}");
        }
        // except the two the owner alone starts, which the bridge refuses in its own words
        for name in ["opsEnable", "opsSetEntity"] {
            assert_eq!(
                said(ask(Some(&e), EntryPath::StaffRpc, name)),
                (OWNER_ENABLES.to_owned(), 400)
            );
        }
        // and /api/rpc lets no employee near any of them
        for name in ["opsClock", "ops", "opsEnable"] {
            assert_eq!(
                said(ask(Some(&e), EntryPath::Rpc, name)),
                (ROLE_NOT_ALLOWED_HERE.to_owned(), 400)
            );
        }
    }
}

#[test]
fn the_owner_starts_custody_on_its_own_path() {
    // the custody lines of `the_owner_is_let_through_wherever_a_signed_in_caller_is` (`:417`)
    let o = owner();
    for name in ["opsSetEntity", "opsEnable"] {
        assert_eq!(ask(Some(&o), EntryPath::Custody, name), Ok(()), "{name}");
    }
    // the custody line of `the_owner_is_not_an_employee` (`:437`)
    assert_eq!(
        said(ask(Some(&o), EntryPath::StaffRpc, "opsClock")),
        (EMPLOYEE_AUTH.to_owned(), 401)
    );
    // the custody line of `an_unknown_action_is_refused_in_the_words_of_its_path` (`:481`)
    assert_eq!(
        said(ask(
            Some(&staff(Role::Manager, B2)),
            EntryPath::Custody,
            "opsNope"
        )),
        (INVALID_REQUEST.to_owned(), 400)
    );
}

#[test]
fn each_custody_action_has_its_roles() {
    let table: [(&str, &[&str]); 12] = [
        ("opsBeginTransaction", &["FRONT"]),
        ("opsTarget", &["FINANCE"]),
        ("opsStartService", &["BACKEND"]),
        ("opsCutoff", &["BACKEND"]),
        ("opsPropose", &["FRONT", "FINANCE"]),
        ("opsHold", &["FRONT", "MONITOR"]),
        ("opsRelease", &["MONITOR"]),
        ("opsReject", &["BACKEND", "MONITOR"]),
        ("opsSign", &["FRONT", "MANAGER", "MONITOR", "FINANCE"]),
        ("opsDispatch", &["BACKEND"]),
        ("opsCommit", &["BACKEND"]),
        ("opsReconcile", &["FINANCE"]),
    ];
    for (name, roles) in table {
        assert_eq!(allowed_roles(EntryPath::Custody, name), roles, "{name}");
    }
    assert_eq!(allowed_roles(EntryPath::Custody, "opsClock").len(), 9);
    assert!(allowed_roles(EntryPath::Custody, "opsEnable").is_empty());
    assert!(allowed_roles(EntryPath::Custody, "opsSetEntity").is_empty());
}

#[test]
fn signing_is_refused_in_three_different_sentences() {
    let sign = |who: &Actor| said(ask(Some(who), EntryPath::Custody, "opsSign"));
    assert_eq!(
        sign(&staff(Role::Backend, B2)),
        (BACKEND_USES_COMMIT.to_owned(), 400)
    );
    for r in [
        Role::Rates,
        Role::System,
        Role::Documents,
        Role::DocumentReview,
    ] {
        assert_eq!(sign(&staff(r, B2)), (SIGN_NOT_ALLOWED.to_owned(), 400));
    }
    assert_eq!(sign(&owner()), (CLOCK_IN_FIRST.to_owned(), 400));
}

#[test]
fn the_owner_starts_custody_and_does_nothing_else_in_it() {
    let o = owner();
    assert_eq!(
        said(ask(Some(&o), EntryPath::Custody, "opsClock")),
        (EMPLOYEE_ACCOUNT.to_owned(), 400)
    );
    for name in ["opsTarget", "opsPropose", "opsCommit", "opsNope"] {
        assert_eq!(
            said(ask(Some(&o), EntryPath::Custody, name)),
            (CLOCK_IN_FIRST.to_owned(), 400),
            "{name}"
        );
    }
    assert_eq!(
        said(ask(Some(&front()), EntryPath::Custody, "opsEnable")),
        (ROLE_NOT_ALLOWED.to_owned(), 400)
    );
}
