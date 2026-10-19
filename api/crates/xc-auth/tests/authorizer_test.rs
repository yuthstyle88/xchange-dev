//! Ticket #616 (OR-2) — `Authorizer::check` in the order of §3.5, and the permission table it reads. Written before the
//! implementation by the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) §3.1 (roles), §3.2 (scopes; "a
//! role row outside `[valid_from, valid_to)` grants nothing"), §3.4 (`Permission`, `RoleGrants`), §3.5 (the
//! check, in order: no actor → `signIn`; organisation → `outOfScope`; permission and branch → `outOfScope` if the
//! role exists elsewhere, else `roleNotAllowed`; owner of the data → `outOfScope`; and `Grant.branch`), D-4 (a
//! branch outside scope is **refused**, no longer replaced by the actor's own as `policy.rs:43-53` did).
//!
//! Pure: no database. The time is passed in, never read from the clock.
//!
//! Signatures these tests assume (§3.5 gives `Authorizer::check(actor, permission, target) -> Result<Grant,
//! Refusal>`; the time it judges "valid now" against is not named, so it is the last argument here):
//! - `xc_auth::domain::access::authorizer::Authorizer::check(actor: Option<&Actor>, permission: Permission,
//!   target: &Target, now: DateTime<Utc>) -> Result<Grant, Refusal>`, an associated function.
//! - `xc_auth::domain::access::authorizer::Target::new(organization_id: &str, branch: Option<BranchId>,
//!   customer_id: Option<&str>) -> Target`: the organisation, the branch named (if any) and the customer whose
//!   data it is (if any).
//! - `xc_auth::domain::access::authorizer::Grant` is `Debug` and has a public field `branch: BranchId`.
//! - `xc_auth::domain::access::authorizer::Refusal` is an enum with `Debug + PartialEq` and at least the
//!   variants `SignIn`, `OutOfScope`, `RoleNotAllowed`; `Refusal::code(&self) -> &'static str` gives
//!   `"signIn"`, `"outOfScope"`, `"roleNotAllowed"` (§3.5, §11).
//! - `xc_auth::domain::access::actor::Actor::new(user_id: &str, organization_id: &str, roles: Vec<ScopedRole>,
//!   customer_id: Option<&str>) -> Actor` (§4.1 `access::Actor { user_id, organization_id, roles, customer_id }`).
//! - `xc_auth::domain::access::permission::Permission`, an enum with `Clone + Copy + Debug + PartialEq + Eq`,
//!   variant names as §3.4 writes them.
//! - `xc_auth::domain::access::role_grants::RoleGrants::seed(role: Role)` returns a collection with
//!   `contains(&Permission) -> bool` (a slice, a `Vec` or a set).
//! - `ScopedRole::new` and `Role` as in `scope_rule_test.rs`.
//! - Ids are compared as given: `Actor` and `Target` do not parse them.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Grant, Refusal, Target};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;

const ORG: &str = "org-a";
const OTHER_ORG: &str = "org-b";
const MAIN: &str = "MAIN";
const B2: &str = "B2";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 9, 5, 0, 0).unwrap()
}

fn since() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// A row held from the start of 2026 with no end.
fn held(role: Role, at: Option<&str>) -> ScopedRole {
    ScopedRole::new(role, at.map(BranchId::new), since(), None)
}

fn user(roles: Vec<ScopedRole>) -> Actor {
    Actor::new("user-1", ORG, roles, None)
}

fn on(branch: Option<&str>) -> Target {
    Target::new(ORG, branch.map(BranchId::new), None)
}

fn check(actor: &Actor, permission: Permission, target: &Target) -> Result<Grant, Refusal> {
    Authorizer::check(Some(actor), permission, target, now())
}

fn granted_branch(result: Result<Grant, Refusal>, why: &str) -> String {
    match result {
        Ok(grant) => grant.branch.as_str().to_owned(),
        Err(refusal) => panic!("{why}: refused with {refusal:?}"),
    }
}

fn refused(result: Result<Grant, Refusal>, why: &str) -> Refusal {
    match result {
        Ok(grant) => panic!("{why}: granted {grant:?}"),
        Err(refusal) => refusal,
    }
}

// ── 1. no actor ──────────────────────────────────────────────────────────────

#[test]
fn nobody_signed_in_is_told_to_sign_in() {
    let refusal = refused(
        Authorizer::check(None, Permission::ExchangeTransact, &on(Some(MAIN)), now()),
        "no actor",
    );
    assert_eq!(refusal, Refusal::SignIn);
    assert_eq!(refusal.code(), "signIn");
}

// ── 2. organisation ──────────────────────────────────────────────────────────

/// An organisation-wide EXECUTIVE holding the permission is still refused in another organisation.
#[test]
fn another_organisation_is_out_of_scope() {
    let executive = user(vec![held(Role::Executive, None)]);
    for branch in [None, Some(MAIN)] {
        let refusal = refused(
            check(
                &executive,
                Permission::OverviewRead,
                &Target::new(OTHER_ORG, branch.map(BranchId::new), None),
            ),
            "another organisation",
        );
        assert_eq!(refusal, Refusal::OutOfScope, "branch {branch:?}");
        assert_eq!(refusal.code(), "outOfScope");
    }
}

/// Step 2 comes before step 3: in another organisation a permission the actor does not hold is `outOfScope`,
/// not `roleNotAllowed`.
#[test]
fn the_organisation_is_checked_before_the_permission() {
    let teller = user(vec![held(Role::Teller, Some(MAIN))]);
    assert_eq!(
        refused(
            check(
                &teller,
                Permission::UsersManage,
                &Target::new(OTHER_ORG, Some(BranchId::new(MAIN)), None),
            ),
            "another organisation, a permission TELLER does not have",
        ),
        Refusal::OutOfScope
    );
}

// ── 3. permission and branch ─────────────────────────────────────────────────

#[test]
fn a_branch_in_scope_is_the_branch_granted() {
    let teller = user(vec![held(Role::Teller, Some(MAIN))]);
    assert_eq!(
        granted_branch(
            check(&teller, Permission::ExchangeTransact, &on(Some(MAIN))),
            "TELLER at MAIN on MAIN"
        ),
        MAIN
    );
}

/// D-4: the branch named outside scope is refused, and **not** replaced by the actor's own branch.
#[test]
fn a_branch_outside_scope_is_refused_not_overridden() {
    let teller = user(vec![held(Role::Teller, Some(MAIN))]);
    let refusal = refused(
        check(&teller, Permission::ExchangeTransact, &on(Some(B2))),
        "TELLER at MAIN names B2",
    );
    assert_eq!(refusal, Refusal::OutOfScope);
    assert_eq!(refusal.code(), "outOfScope");

    // ADMIN scoped to a branch (ex-SYSTEM, §3.3) manages users only there (C4, §4.2).
    let admin = user(vec![held(Role::Admin, Some(MAIN))]);
    assert_eq!(
        granted_branch(
            check(&admin, Permission::UsersManage, &on(Some(MAIN))),
            "ADMIN at MAIN on MAIN"
        ),
        MAIN
    );
    assert_eq!(
        refused(
            check(&admin, Permission::UsersManage, &on(Some(B2))),
            "ADMIN at MAIN names B2"
        ),
        Refusal::OutOfScope
    );
}

/// RATE_MANAGER on two branches works on each of them and on no third.
#[test]
fn several_branches_cover_each_of_them() {
    let rates = user(vec![
        held(Role::RateManager, Some(MAIN)),
        held(Role::RateManager, Some(B2)),
    ]);
    for branch in [MAIN, B2] {
        assert_eq!(
            granted_branch(
                check(&rates, Permission::RatesDraft, &on(Some(branch))),
                "RATE_MANAGER on its branch"
            ),
            branch
        );
    }
    assert_eq!(
        refused(
            check(&rates, Permission::RatesDraft, &on(Some("B3"))),
            "a third branch"
        ),
        Refusal::OutOfScope
    );
}

/// No role of the actor carries the permission anywhere: `roleNotAllowed`.
#[test]
fn a_permission_no_role_carries_is_role_not_allowed() {
    let teller = user(vec![held(Role::Teller, Some(MAIN))]);
    let refusal = refused(
        check(&teller, Permission::UsersManage, &on(Some(MAIN))),
        "TELLER asks to manage users",
    );
    assert_eq!(refusal, Refusal::RoleNotAllowed);
    assert_eq!(refusal.code(), "roleNotAllowed");
}

/// §3.1 MARKETING "never opens identity documents" (§02); §3.4 `IdentityDocumentRead` is TELLER and
/// VERIFICATION_REVIEWER only.
#[test]
fn marketing_never_reads_identity_documents() {
    assert!(!RoleGrants::seed(Role::Marketing).contains(&Permission::IdentityDocumentRead));
    assert!(RoleGrants::seed(Role::Teller).contains(&Permission::IdentityDocumentRead));
    assert!(
        RoleGrants::seed(Role::VerificationReviewer).contains(&Permission::IdentityDocumentRead)
    );

    let marketing = user(vec![held(Role::Marketing, None)]);
    for branch in [None, Some(MAIN)] {
        assert_eq!(
            refused(
                check(&marketing, Permission::IdentityDocumentRead, &on(branch)),
                "MARKETING asks for identity documents"
            ),
            Refusal::RoleNotAllowed,
            "branch {branch:?}"
        );
    }
    // Its own permissions are granted.
    assert!(check(&marketing, Permission::MarketingCampaigns, &on(None)).is_ok());
}

/// §3.2: "A role row outside `[valid_from, valid_to)` grants nothing." The window is half-open.
#[test]
fn a_role_row_outside_its_validity_grants_nothing() {
    let from = now() - Duration::hours(1);
    let to = now() + Duration::hours(1);
    let teller = user(vec![ScopedRole::new(
        Role::Teller,
        Some(BranchId::new(MAIN)),
        from,
        Some(to),
    )]);
    let target = on(Some(MAIN));
    let at = |when: DateTime<Utc>| {
        Authorizer::check(Some(&teller), Permission::ExchangeTransact, &target, when)
    };

    assert!(at(from).is_ok(), "valid from `valid_from` itself");
    assert!(at(now()).is_ok(), "valid inside the window");
    assert!(
        at(to - Duration::seconds(1)).is_ok(),
        "valid until just before `valid_to`"
    );
    assert!(at(from - Duration::seconds(1)).is_err(), "not yet valid");
    assert!(at(to).is_err(), "`valid_to` itself is outside");
    assert!(at(to + Duration::days(30)).is_err(), "long expired");

    // A row with no end is valid from its start on.
    let open = user(vec![ScopedRole::new(
        Role::Teller,
        Some(BranchId::new(MAIN)),
        from,
        None,
    )]);
    assert!(Authorizer::check(Some(&open), Permission::ExchangeTransact, &target, now()).is_ok());
    assert!(
        Authorizer::check(
            Some(&open),
            Permission::ExchangeTransact,
            &target,
            from - Duration::seconds(1)
        )
        .is_err()
    );
}

/// An expired row does not widen the scope: the valid row's branch is the only one granted.
#[test]
fn an_expired_row_covers_no_branch() {
    let teller = user(vec![
        held(Role::Teller, Some(MAIN)),
        ScopedRole::new(
            Role::Teller,
            Some(BranchId::new(B2)),
            since(),
            Some(now() - Duration::days(1)),
        ),
    ]);
    assert!(check(&teller, Permission::ExchangeTransact, &on(Some(MAIN))).is_ok());
    assert!(check(&teller, Permission::ExchangeTransact, &on(Some(B2))).is_err());
}

// ── 4. owner of the data ─────────────────────────────────────────────────────

/// §3.5 step 4: for `CustomerSelf`, the target's customer must be the actor's.
#[test]
fn a_customer_reading_another_customers_data_is_out_of_scope() {
    let customer = Actor::new(
        "user-c1",
        ORG,
        vec![held(Role::Customer, None)],
        Some("cust-1"),
    );
    let own = Target::new(ORG, None, Some("cust-1"));
    let other = Target::new(ORG, None, Some("cust-2"));

    assert!(
        Authorizer::check(Some(&customer), Permission::CustomerSelf, &own, now()).is_ok(),
        "own data"
    );
    let refusal = refused(
        Authorizer::check(Some(&customer), Permission::CustomerSelf, &other, now()),
        "another customer's data",
    );
    assert_eq!(refusal, Refusal::OutOfScope);
    assert_eq!(refusal.code(), "outOfScope");

    // A CUSTOMER user not linked to any customer owns nobody's data.
    let unlinked = Actor::new("user-c0", ORG, vec![held(Role::Customer, None)], None);
    assert_eq!(
        refused(
            Authorizer::check(Some(&unlinked), Permission::CustomerSelf, &own, now()),
            "no customer of its own"
        ),
        Refusal::OutOfScope
    );
}

// ── Grant.branch ─────────────────────────────────────────────────────────────

/// §3.5: an organisation-wide user who names no branch gets `MAIN` (v3's rule); one who names a branch gets it.
#[test]
fn an_organisation_wide_user_works_on_the_branch_named_and_on_main_when_none_is() {
    let executive = user(vec![held(Role::Executive, None)]);
    assert_eq!(
        granted_branch(
            check(&executive, Permission::OverviewRead, &on(None)),
            "EXECUTIVE names no branch"
        ),
        MAIN
    );
    assert_eq!(
        granted_branch(
            check(&executive, Permission::OverviewRead, &on(Some(B2))),
            "EXECUTIVE names B2"
        ),
        B2
    );
}

/// §3.5: a user whose roles cover exactly one branch, and who names none, gets that branch.
#[test]
fn a_one_branch_user_who_names_none_gets_that_branch() {
    let teller = user(vec![held(Role::Teller, Some(B2))]);
    assert_eq!(
        granted_branch(
            check(&teller, Permission::ExchangeTransact, &on(None)),
            "TELLER at B2 names no branch"
        ),
        B2
    );
}
