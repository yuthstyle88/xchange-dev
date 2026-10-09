//! Non-counter roles get RoleNotAllowed on EntryPath::StaffBookings whatever the action.
//!
//! Ticket T15-d rework (#410): the role check comes first.
//!
//! Moved by the author of OR-4 (#550, `docs/design/blueprint/org-roles-branches.md` §4.3) onto the
//! Blueprint's users: every role but TELLER is outside the counter (C9 adds BRANCH_MANAGER).

mod policy_common;

use policy_common::*;

const B2: &str = "B2";

const NON_COUNTER_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Finance,
    Role::Marketing,
    Role::Customer,
    Role::VerificationReviewer,
];

#[test]
fn non_counter_roles_get_role_not_allowed_for_every_action_category() {
    let counter_action = "list";
    let owner_action = "settings";
    let unknown_name = "nope";

    for r in NON_COUNTER_ROLES {
        for name in [counter_action, owner_action, unknown_name] {
            assert_eq!(
                said(ask(Some(&staff(r, B2)), EntryPath::StaffBookings, name)),
                (ROLE_NOT_ALLOWED.to_owned(), 400),
                "role {} on action {name:?} must get RoleNotAllowed (400)",
                r.as_str()
            );
        }
    }
}
