//! SB-1 (design `docs/design/blueprint/org-roles-branches.md` §14, PR 831): the session branch rules (§14.2) and the
//! "names none" arm of `Authorizer::check` reading the actor's branch (§3.5 amended, §14.3). Written before the
//! implementation by the ticket author (claude/83f682). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Pure: no database. The time is passed in, never read from the clock.
//!
//! Signatures these tests assume (§14.3 [required]):
//! - `xc_auth::domain::access::session_branch::BranchRef { pub id: BranchId, pub code: String, pub name: String }`
//!   with `BranchRef::new(id: &str, code: &str, name: &str) -> BranchRef`, `Clone + Debug + PartialEq + Eq`.
//! - `xc_auth::domain::access::session_branch::SessionBranch`, a unit struct with associated functions:
//!   - `switchable(roles: &[ScopedRole], org_branches: &[BranchRef], now: DateTime<Utc>) -> Vec<BranchRef>`
//!   - `initial(s: &[BranchRef], last: Option<&BranchId>) -> Option<BranchId>`
//!   - `effective(stored: Option<&BranchId>, s: &[BranchRef], last: Option<&BranchId>) -> Option<BranchId>`
//! - `xc_auth::domain::access::actor::Actor` gains `pub branch: Option<BranchId>` (`None` from `Actor::new`, so no
//!   existing caller changes) and `Actor::with_branch(self, branch: Option<BranchId>) -> Actor`.
//! - `Authorizer::check`, `Target`, `Grant`, `Refusal`, `ScopedRole`, `Role` as in `authorizer_test.rs`.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Refusal, Target};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_auth::domain::access::session_branch::{BranchRef, SessionBranch};
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 12, 0, 0).unwrap()
}

fn row(role: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        role,
        branch.map(BranchId::new),
        now() - Duration::days(1),
        None,
    )
}

fn expired(role: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        role,
        branch.map(BranchId::new),
        now() - Duration::days(10),
        Some(now() - Duration::days(1)),
    )
}

/// The organisation's branches, deliberately **not** in code order: ids and codes sort differently.
fn org_branches() -> Vec<BranchRef> {
    vec![
        BranchRef::new("MAIN", "M01", "สาขาหลัก"),
        BranchRef::new("B2", "Z09", "สาขาสอง"),
        BranchRef::new("B3", "A05", "สาขาสาม"),
    ]
}

fn ids(s: &[BranchRef]) -> Vec<&str> {
    s.iter().map(|b| b.id.as_str()).collect()
}

fn b(id: &str) -> BranchId {
    BranchId::new(id)
}

// ── switchable set S (§14.2) ──────────────────────────────────────────────

#[test]
fn an_organisation_wide_row_makes_every_branch_switchable_ordered_by_code() {
    let s = SessionBranch::switchable(&[row(Role::Admin, None)], &org_branches(), now());
    assert_eq!(ids(&s), vec!["B3", "MAIN", "B2"], "A05 < M01 < Z09");
}

#[test]
fn branch_rows_give_exactly_their_branches_ordered_by_code() {
    let roles = [
        row(Role::RateManager, Some("B2")),
        row(Role::RateManager, Some("MAIN")),
    ];
    let s = SessionBranch::switchable(&roles, &org_branches(), now());
    assert_eq!(ids(&s), vec!["MAIN", "B2"]);
}

#[test]
fn two_roles_on_one_branch_give_it_once() {
    let roles = [
        row(Role::Teller, Some("B2")),
        row(Role::RateManager, Some("B2")),
    ];
    let s = SessionBranch::switchable(&roles, &org_branches(), now());
    assert_eq!(ids(&s), vec!["B2"]);
}

#[test]
fn an_expired_row_gives_nothing() {
    let roles = [expired(Role::Admin, None), row(Role::Teller, Some("B2"))];
    let s = SessionBranch::switchable(&roles, &org_branches(), now());
    assert_eq!(
        ids(&s),
        vec!["B2"],
        "the expired org-wide row does not open every branch"
    );
}

#[test]
fn a_row_that_starts_tomorrow_gives_nothing() {
    let later = ScopedRole::new(Role::Teller, Some(b("B3")), now() + Duration::days(1), None);
    let s = SessionBranch::switchable(&[later], &org_branches(), now());
    assert!(s.is_empty(), "{s:?}");
}

#[test]
fn equal_codes_fall_back_to_the_id() {
    let branches = vec![
        BranchRef::new("Y", "SAME", "y"),
        BranchRef::new("X", "SAME", "x"),
    ];
    let s = SessionBranch::switchable(&[row(Role::Admin, None)], &branches, now());
    assert_eq!(ids(&s), vec!["X", "Y"]);
}

#[test]
fn the_names_travel_with_the_ids() {
    let s = SessionBranch::switchable(&[row(Role::Teller, Some("B3"))], &org_branches(), now());
    assert_eq!(s, vec![BranchRef::new("B3", "A05", "สาขาสาม")]);
}

// ── initial branch (R5) ───────────────────────────────────────────────────

#[test]
fn one_branch_is_the_initial_branch_whatever_the_last_choice() {
    let s = vec![BranchRef::new("B2", "Z09", "สาขาสอง")];
    assert_eq!(SessionBranch::initial(&s, Some(&b("MAIN"))), Some(b("B2")));
    assert_eq!(SessionBranch::initial(&s, None), Some(b("B2")));
}

#[test]
fn several_branches_take_the_last_choice_when_it_is_still_switchable() {
    let s = SessionBranch::switchable(&[row(Role::Admin, None)], &org_branches(), now());
    assert_eq!(SessionBranch::initial(&s, Some(&b("B2"))), Some(b("B2")));
}

#[test]
fn several_branches_without_a_usable_last_choice_take_the_first_by_code() {
    let s = SessionBranch::switchable(&[row(Role::Admin, None)], &org_branches(), now());
    assert_eq!(
        SessionBranch::initial(&s, None),
        Some(b("B3")),
        "A05 is first"
    );
    assert_eq!(
        SessionBranch::initial(&s, Some(&b("GONE"))),
        Some(b("B3")),
        "a last choice outside S is ignored"
    );
}

#[test]
fn no_switchable_branch_gives_none() {
    assert_eq!(SessionBranch::initial(&[], Some(&b("B2"))), None);
}

// ── effective branch, read on every request ───────────────────────────────

#[test]
fn a_stored_branch_still_in_s_is_used_as_is() {
    let s = SessionBranch::switchable(&[row(Role::Admin, None)], &org_branches(), now());
    assert_eq!(
        SessionBranch::effective(Some(&b("MAIN")), &s, Some(&b("B2"))),
        Some(b("MAIN")),
        "the session's own choice wins over the user's last choice"
    );
}

#[test]
fn a_stored_branch_no_longer_in_s_is_recomputed_by_the_initial_rule() {
    // the role on B2 was revoked mid-session: S is now MAIN and B3
    let roles = [
        row(Role::RateManager, Some("MAIN")),
        row(Role::RateManager, Some("B3")),
    ];
    let s = SessionBranch::switchable(&roles, &org_branches(), now());
    assert_eq!(
        SessionBranch::effective(Some(&b("B2")), &s, Some(&b("B2"))),
        Some(b("B3"))
    );
    assert_eq!(
        SessionBranch::effective(Some(&b("B2")), &s, Some(&b("MAIN"))),
        Some(b("MAIN"))
    );
}

#[test]
fn no_stored_branch_is_recomputed_by_the_initial_rule() {
    let s = SessionBranch::switchable(&[row(Role::Admin, None)], &org_branches(), now());
    assert_eq!(SessionBranch::effective(None, &s, None), Some(b("B3")));
    assert_eq!(
        SessionBranch::effective(None, &s, Some(&b("B2"))),
        Some(b("B2"))
    );
}

// ── the check reads the actor's branch (§3.5 amended) ─────────────────────

fn actor(roles: Vec<ScopedRole>, branch: Option<&str>) -> Actor {
    Actor::new("user-1", ORG, roles, None).with_branch(branch.map(BranchId::new))
}

fn naming_none() -> Target {
    Target::new(ORG, None, None)
}

#[test]
fn actor_new_carries_no_branch() {
    assert_eq!(Actor::new("user-1", ORG, vec![], None).branch, None);
}

#[test]
fn an_organisation_wide_user_naming_none_gets_the_session_branch_not_main() {
    let a = actor(vec![row(Role::Admin, None)], Some("B2"));
    let grant =
        Authorizer::check(Some(&a), Permission::BranchesManage, &naming_none(), now()).unwrap();
    assert_eq!(grant.branch, b("B2"));
}

#[test]
fn a_several_branch_user_naming_none_gets_the_session_branch_instead_of_a_refusal() {
    let a = actor(
        vec![
            row(Role::RateManager, Some("MAIN")),
            row(Role::RateManager, Some("B2")),
        ],
        Some("B2"),
    );
    let grant = Authorizer::check(Some(&a), Permission::RatesDraft, &naming_none(), now()).unwrap();
    assert_eq!(grant.branch, b("B2"));
}

#[test]
fn a_named_branch_in_scope_still_wins_over_the_session_branch() {
    let a = actor(vec![row(Role::Admin, None)], Some("B2"));
    let named = Target::new(ORG, Some(b("B3")), None);
    let grant = Authorizer::check(Some(&a), Permission::BranchesManage, &named, now()).unwrap();
    assert_eq!(grant.branch, b("B3"));
}

#[test]
fn a_named_branch_out_of_scope_is_still_refused_whatever_the_session_branch() {
    let a = actor(vec![row(Role::RateManager, Some("B2"))], Some("B2"));
    let named = Target::new(ORG, Some(b("B3")), None);
    assert_eq!(
        Authorizer::check(Some(&a), Permission::RatesDraft, &named, now()).unwrap_err(),
        Refusal::OutOfScope
    );
}

#[test]
fn a_session_branch_outside_the_permissions_scope_falls_back_to_the_old_arms() {
    // TELLER on B2 (the session branch), RATE_MANAGER on MAIN only: drafting rates names none
    let a = actor(
        vec![
            row(Role::Teller, Some("B2")),
            row(Role::RateManager, Some("MAIN")),
        ],
        Some("B2"),
    );
    let grant = Authorizer::check(Some(&a), Permission::RatesDraft, &naming_none(), now()).unwrap();
    assert_eq!(
        grant.branch,
        b("MAIN"),
        "the one branch the permission covers, as before §14"
    );
}

#[test]
fn without_a_session_branch_the_old_arms_are_unchanged() {
    let org_wide = actor(vec![row(Role::Admin, None)], None);
    assert_eq!(
        Authorizer::check(
            Some(&org_wide),
            Permission::BranchesManage,
            &naming_none(),
            now()
        )
        .unwrap()
        .branch,
        b("MAIN")
    );
    let several = actor(
        vec![
            row(Role::RateManager, Some("MAIN")),
            row(Role::RateManager, Some("B2")),
        ],
        None,
    );
    assert_eq!(
        Authorizer::check(
            Some(&several),
            Permission::RatesDraft,
            &naming_none(),
            now()
        )
        .unwrap_err(),
        Refusal::OutOfScope
    );
}
