//! The one mapping from a role to its part in custody, pinned (ticket #233).
//!
//! Commit 1 of the move: these rows are today's three mappings written as one
//! table (`docs/design/xchange-v3.md` §5.2, PR 243). They pass before the call
//! sites are re-pointed and must stay green after — the move changes no
//! answer. No database is needed.

use xc_auth::domain::policy::value_objects::role::Role;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xchange::application::custody_role::{custody_role_of, employee_custody_role};

/// Every `Role` keeps the custody part today's two `Role` tables give it.
#[test]
fn every_role_keeps_its_custody_part() {
    let table = [
        (Role::Owner, CustodyRole::Owner),
        (Role::Front, CustodyRole::Front),
        (Role::Finance, CustodyRole::Finance),
        (Role::Monitor, CustodyRole::Monitor),
        (Role::Manager, CustodyRole::Manager),
        (Role::Backend, CustodyRole::Backend),
        // the four roles with no part in custody
        (Role::Rates, CustodyRole::Other),
        (Role::System, CustodyRole::Other),
        (Role::Documents, CustodyRole::Other),
        (Role::DocumentReview, CustodyRole::Other),
    ];
    for (role, want) in table {
        assert_eq!(custody_role_of(role), want, "{role:?}");
    }
}

/// Every staff text as stored keeps the custody part of its `Role` — today's
/// employee-row table, pinned by name.
#[test]
fn every_staff_text_as_stored_is_pinned_by_name() {
    let table = [
        ("FRONT", CustodyRole::Front),
        ("BACKEND", CustodyRole::Backend),
        ("MANAGER", CustodyRole::Manager),
        ("RATES", CustodyRole::Other),
        ("MONITOR", CustodyRole::Monitor),
        ("FINANCE", CustodyRole::Finance),
        ("SYSTEM", CustodyRole::Other),
        ("DOCUMENTS", CustodyRole::Other),
        ("DOCUMENT_REVIEW", CustodyRole::Other),
    ];
    assert_eq!(table.len(), Role::STAFF.len());
    for (stored, want) in table {
        assert_eq!(employee_custody_role(stored), want, "{stored}");
    }
}

/// An employee is never the owner, whatever the row says.
#[test]
fn an_employee_is_never_the_owner() {
    assert_eq!(employee_custody_role("OWNER"), CustodyRole::Other);
}

/// Lower case is not a role text.
#[test]
fn lower_case_is_not_a_role_text() {
    assert_eq!(employee_custody_role("front"), CustodyRole::Other);
}

/// Spaces, the empty text and a role that does not exist are `Other`: the
/// stored text is compared exactly.
#[test]
fn anything_not_exactly_a_staff_role_text_is_other() {
    for stored in ["", " FRONT", "FRONT ", "CASHIER"] {
        assert_eq!(
            employee_custody_role(stored),
            CustodyRole::Other,
            "{stored:?}"
        );
    }
}

/// Drift check: the stored-text path answers the same as the `Role` path for
/// every staff role. It guards the future — a role whose two answers differ
/// cannot pass here, whatever the tables become.
#[test]
fn the_stored_text_path_never_drifts_from_the_role_path() {
    for role in Role::STAFF {
        assert_eq!(
            employee_custody_role(role.as_str()),
            custody_role_of(role),
            "{} drifted",
            role.as_str()
        );
    }
}
