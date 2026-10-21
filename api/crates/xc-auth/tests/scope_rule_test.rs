//! Ticket #616 (OR-2) — the Blueprint roles and the scope rule of a granted role. Written before the implementation by the
//! ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/org-roles-branches.md` (#550, merged f0ec3ca) §3.1 (the roles, verbatim from
//! Blueprint §02 and §15), §3.2 (what `branch_id` may be), §4.1 (`access::Role`, `ScopedRole`, new files beside
//! the old types), §7 (`ScopeRule::check(role, branch, rows)`).
//!
//! Pure: no database, no clock.
//!
//! Signatures these tests assume (§4.1 and §7 name the types and files, not the exact Rust):
//! - `xc_auth::domain::access::role::Role`, an enum with `Clone + Copy + Debug + PartialEq + Eq`, variants
//!   `Executive`, `Admin`, `RateManager`, `BranchManager`, `Teller`, `Finance`, `Marketing`, `Customer`,
//!   `VerificationReviewer`; `Role::as_str(self) -> &'static str` (the §3.1 "v3 wire" spelling) and
//!   `Role::parse(&str) -> Option<Role>` (the old v2 `Role` had them too; it is gone since OR-7.m w2).
//! - `xc_auth::domain::access::scoped_role::ScopedRole::new(role: Role, branch: Option<BranchId>,
//!   valid_from: DateTime<Utc>, valid_to: Option<DateTime<Utc>>) -> ScopedRole`; `branch: None` is the whole
//!   organisation (D-2).
//! - `xc_auth::domain::access::scope_rule::ScopeRule::check(role: Role, branch: Option<&BranchId>,
//!   existing: &[ScopedRole]) -> Result<(), E>` with `E: Debug`, an associated function. `existing` are the
//!   rows the user already holds; the call asks whether one more row `(role, branch)` may be granted.
//!
//! Note: §3.1 lists nine roles (the Blueprint's nine minus PARTNER, plus VERIFICATION_REVIEWER), although D-3
//! and §4.1 call them "ten". These tests follow the table: the nine of §3.1.

use chrono::{DateTime, TimeZone, Utc};
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scope_rule::ScopeRule;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;

const MAIN: &str = "MAIN";
const B2: &str = "B2";
const B3: &str = "B3";

fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

fn since() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// A row held from the start of 2026 with no end.
fn held(role: Role, at: Option<&str>) -> ScopedRole {
    ScopedRole::new(role, at.map(branch), since(), None)
}

fn may_grant(role: Role, at: Option<&str>, existing: &[ScopedRole]) -> bool {
    let at = at.map(branch);
    ScopeRule::check(role, at.as_ref(), existing).is_ok()
}

// ── the roles, as §3.1 spells them ───────────────────────────────────────────

/// §3.1 "v3 wire" column, every row but PARTNER.
const NINE: [(Role, &str); 9] = [
    (Role::Executive, "EXECUTIVE"),
    (Role::Admin, "ADMIN"),
    (Role::RateManager, "RATE_MANAGER"),
    (Role::BranchManager, "BRANCH_MANAGER"),
    (Role::Teller, "TELLER"),
    (Role::Finance, "FINANCE"),
    (Role::Marketing, "MARKETING"),
    (Role::Customer, "CUSTOMER"),
    (Role::VerificationReviewer, "VERIFICATION_REVIEWER"),
];

#[test]
fn roles_are_spelled_as_the_blueprint_spells_them() {
    for (role, wire) in NINE {
        assert_eq!(role.as_str(), wire, "{role:?} is written {wire}");
        assert_eq!(
            Role::parse(wire),
            Some(role),
            "{wire} reads back as {role:?}"
        );
    }
}

/// PARTNER is Broker-Platform's (§3.1, #564 §1); the v3 names are not Blueprint roles (B1 maps them, §3.3).
#[test]
fn a_role_the_blueprint_does_not_have_is_no_role() {
    for word in [
        "PARTNER",
        "OWNER",
        "FRONT",
        "BACKEND",
        "MANAGER",
        "RATES",
        "SYSTEM",
        "MONITOR",
        "DOCUMENTS",
        "DOCUMENT_REVIEW",
        "teller",
        "",
    ] {
        assert_eq!(Role::parse(word), None, "{word:?} is not a Blueprint role");
    }
}

// ── §3.2: the rows the brief names ────────────────────────────────────────────

/// §3.2 EXECUTIVE: `branch_id NULL` **required** ("every branch of the organisation").
#[test]
fn executive_must_be_organisation_wide() {
    assert!(
        may_grant(Role::Executive, None, &[]),
        "EXECUTIVE over the whole organisation"
    );
    assert!(
        !may_grant(Role::Executive, Some(MAIN), &[]),
        "EXECUTIVE scoped to one branch is refused"
    );
    assert!(
        !may_grant(Role::Executive, Some(B2), &[held(Role::Executive, None)]),
        "nor added as a branch row beside the organisation-wide one"
    );
}

/// §3.2 BRANCH_MANAGER ("own branch") and TELLER: no NULL, and one branch only.
#[test]
fn branch_manager_and_teller_take_exactly_one_branch() {
    for role in [Role::BranchManager, Role::Teller] {
        assert!(may_grant(role, Some(MAIN), &[]), "{role:?} at one branch");
        assert!(
            !may_grant(role, None, &[]),
            "{role:?} over the whole organisation is refused"
        );
        assert!(
            !may_grant(role, Some(B2), &[held(role, Some(MAIN))]),
            "{role:?} at a second branch is refused"
        );
    }
}

/// §3.2 RATE_MANAGER ("assigned branches"): no NULL, several branches, one row per branch.
#[test]
fn rate_manager_can_hold_several_branches() {
    assert!(may_grant(Role::RateManager, Some(MAIN), &[]));
    assert!(
        may_grant(
            Role::RateManager,
            Some(B2),
            &[held(Role::RateManager, Some(MAIN))]
        ),
        "a second branch"
    );
    assert!(
        may_grant(
            Role::RateManager,
            Some(B3),
            &[
                held(Role::RateManager, Some(MAIN)),
                held(Role::RateManager, Some(B2))
            ]
        ),
        "a third branch"
    );
    assert!(
        !may_grant(Role::RateManager, None, &[]),
        "RATE_MANAGER over the whole organisation is refused"
    );
}

// ── §3.2: the other rows of the table ─────────────────────────────────────────

/// §3.2 ADMIN and VERIFICATION_REVIEWER: NULL allowed, and several branches.
#[test]
fn admin_and_verification_reviewer_may_be_organisation_wide_or_on_several_branches() {
    for role in [Role::Admin, Role::VerificationReviewer] {
        assert!(may_grant(role, None, &[]), "{role:?} organisation-wide");
        assert!(may_grant(role, Some(MAIN), &[]), "{role:?} at one branch");
        assert!(
            may_grant(role, Some(B2), &[held(role, Some(MAIN))]),
            "{role:?} at a second branch"
        );
    }
}

/// §3.2 FINANCE ("branches, by grant"): no NULL, several branches.
#[test]
fn finance_takes_branches_and_never_the_whole_organisation() {
    assert!(!may_grant(Role::Finance, None, &[]));
    assert!(may_grant(Role::Finance, Some(MAIN), &[]));
    assert!(may_grant(
        Role::Finance,
        Some(B2),
        &[held(Role::Finance, Some(MAIN))]
    ));
}

/// §3.2 MARKETING: NULL allowed ("the data marketing needs", not a branch).
/// CUSTOMER: not branch-scoped, own data only.
#[test]
fn marketing_and_customer_hold_no_branch() {
    assert!(may_grant(Role::Marketing, None, &[]));
    assert!(may_grant(Role::Customer, None, &[]));
    assert!(
        !may_grant(Role::Customer, Some(MAIN), &[]),
        "CUSTOMER is not branch-scoped"
    );
}
