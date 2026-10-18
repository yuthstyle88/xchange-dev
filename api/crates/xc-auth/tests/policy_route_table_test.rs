//! OR-4 (#550): every route and action names its permission (§4.2), the role table says what B1
//! translates (§3.3, §3.4), and a v2 caller becomes the user B1 makes of it.
//!
//! Written before the implementation by the ticket author (design
//! `docs/design/blueprint/org-roles-branches.md` §4.1, §4.2). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! Signatures (`[required]`):
//! - `Policy::need(path: EntryPath, action: &Action) -> Option<Need>` in
//!   `xc_auth::domain::policy::entities::policy`; `Need { Public, Permission(Permission), Custody }`
//!   with `Debug + Clone + Copy + PartialEq + Eq`. `None` = a name the path has no row for.
//!   `Custody` = a row #582 decides; until #582 the custody engine asks the v2 rule.
//! - `Permission` gains `ReferenceRatesRead`: a v3-only row of an OWNER-only route that §4.2 does not
//!   list, given to EXECUTIVE, the B1 translation of OWNER (no role changes). (`BookingsAdmin` went
//!   with v2's booking in BK-11 #1413.)
//! - `xc_auth::domain::access::v3_roles::blueprint_roles(v3: policy Role) -> &'static [Role]` and
//!   `actor_from_v3(v3: &policy Actor, organization_id: &str, since: DateTime<Utc>) -> access::Actor`
//!   (B1; the extractor maps a password session through it until OR-7).
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7 removes `adminSaveRate`
//! (`docs/design/blueprint/rates.md` §9), so its two rows (Rpc and StaffRpc, RatesDraft) move to
//! "no row". Every other row, `rateDraft`'s included, is unchanged.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c): CA-10 retires the `ops*`
//! actions and the direct shift actions with their policy rows (`cash-shift-transfer.md` §6), so on
//! `Rpc` and `StaffRpc` `adminOpenShift`, `adminCloseShift`, `adminMoveStock`, `opsClock`, `ops` and
//! `opsEnable` move to "no row". The `Custody` path's own rows stay (the type stays; no route
//! reaches it). Every other row is unchanged.

mod policy_common;

use policy_common::*;
use xc_auth::domain::access::v3_roles::{actor_from_v3, blueprint_roles};
use xc_auth::domain::policy::value_objects::actor::Actor as V3Actor;
use xc_auth::domain::policy::value_objects::role::Role as V3Role;

fn need(on: EntryPath, name: &str) -> Option<Need> {
    Policy::need(on, &action(name))
}

fn p(permission: Permission) -> Option<Need> {
    Some(Need::Permission(permission))
}

#[test]
fn every_route_and_action_names_its_permission() {
    use EntryPath as E;
    use Permission as P;
    // §4.2, row by row
    let rows: Vec<(EntryPath, &str, Option<Need>)> = vec![
        (E::Rpc, "adminSnapshot", p(P::CompatRead)),
        (E::Rpc, "adminDeskConfig", p(P::CompatRead)),
        (E::Rpc, "getConnectionStatus", p(P::CompatRead)),
        (E::StaffRpc, "adminSnapshot", p(P::CompatRead)),
        (E::StaffRpc, "getConnectionStatus", p(P::CompatRead)),
        (E::StaffRpc, "adminDeskConfig", p(P::ExchangeTransact)),
        (E::StaffDesk, "desk", p(P::ExchangeTransact)),
        (E::StaffRpc, "reserveCustomer", p(P::ExchangeTransact)),
        (E::StaffRpc, "saveTransaction", p(P::ExchangeTransact)),
        (E::Rpc, "reserveCustomer", p(P::ExchangeTransact)),
        (E::Rpc, "saveTransaction", p(P::ExchangeTransact)),
        (E::Kiosk, "manage", p(P::RatesDraft)),
        (E::Kiosk, "configure", p(P::RatesDraft)),
        (E::Rpc, "adminCreateBranch", p(P::BranchesManage)),
        (E::OwnerRoute, "employees", p(P::UsersManage)),
        (E::OwnerRoute, "list", p(P::UsersManage)),
        (E::DeviceSetup, "", p(P::SystemSettings)),
        (E::Devices, "setPrinter", p(P::SystemSettings)),
        (E::Devices, "setDrawer", p(P::SystemSettings)),
        (E::PaymentMethods, "save", p(P::SystemSettings)),
        (E::Rpc, "adminAbandonRequest", p(P::RequestAbandon)),
        (E::StaffRpc, "documents", p(P::IdentityDocumentRead)),
        (E::StaffFile, "file", p(P::IdentityDocumentRead)),
        (E::UserRoute, "documents", p(P::DocumentsList)),
        (E::File, "file", p(P::FileRead)),
        (E::Upload, "upload", p(P::ProofUpload)),
        (E::StaffUpload, "upload", p(P::ProofUpload)),
        (E::UserRoute, "receipt", p(P::SignedIn)),
        (E::UserRoute, "admin", p(P::SignedIn)),
        (E::UserRoute, "desk", p(P::SignedIn)),
        (E::Devices, "snapshot", p(P::SignedIn)),
        (E::Devices, "nope", p(P::SignedIn)),
        (E::Kiosk, "notifications", p(P::SignedIn)),
        (E::Kiosk, "list", p(P::SignedIn)),
        (E::PaymentMethods, "list", p(P::SignedIn)),
        (E::Devices, "queue", p(P::DeviceQueue)),
        (E::Devices, "process", p(P::DeviceProcess)),
        (E::Devices, "retry", p(P::DeviceProcess)),
        (E::Devices, "drawerObservation", p(P::DeviceProcess)),
        (E::Devices, "printerOnline", p(P::DevicePrinterOnline)),
        (E::Kiosk, "identityChecked", p(P::QueueOperate)),
        (E::Kiosk, "publishReceipt", p(P::QueueOperate)),
        (E::Kiosk, "get", p(P::QueueOperate)),
        (E::Kiosk, "accept", p(P::QueueOperate)),
        (E::Kiosk, "rates", Some(Need::Public)),
        (E::Kiosk, "quote", Some(Need::Public)),
        (E::Kiosk, "confirm", Some(Need::Public)),
        (E::Kiosk, "status", Some(Need::Public)),
        (E::Kiosk, "customerReceipt", Some(Need::Public)),
        (E::DrawerPassport, "drawer", p(P::DrawerPassport)),
        (E::StaffBookings, "list", p(P::BookingsDesk)),
        (E::StaffBookings, "get", p(P::BookingsDesk)),
        (E::StaffBookings, "create", p(P::BookingsDesk)),
        (E::StaffBookings, "confirm", p(P::BookingsDesk)),
        (E::StaffBookings, "cancel", p(P::BookingsDesk)),
        // RE-3's rows (`docs/design/blueprint/rates.md` §6), translated
        (E::Rpc, "rateDraft", p(P::RatesDraft)),
        (E::StaffRpc, "rateDraft", p(P::RatesDraft)),
        (E::Rpc, "ratePolicySave", p(P::RatePolicySave)),
        (E::Rpc, "approvalDecide", p(P::ApprovalsDecide)),
        (E::Rpc, "approvalRead", p(P::ApprovalsDecide)),
        // OWNER-only routes §4.2 does not list: v3-only permissions, EXECUTIVE's
        (
            E::OwnerRoute,
            "/api/reference-rates",
            p(P::ReferenceRatesRead),
        ),
        // #582's
        (E::Custody, "opsClock", Some(Need::Custody)),
        (E::Custody, "opsNope", Some(Need::Custody)),
        // no row
        // CA-10 #626 retires these (cash-shift-transfer.md §6)
        (E::Rpc, "adminOpenShift", None),
        (E::Rpc, "adminCloseShift", None),
        (E::Rpc, "adminMoveStock", None),
        (E::StaffRpc, "opsClock", None),
        (E::StaffRpc, "ops", None),
        (E::StaffRpc, "opsEnable", None),
        (E::StaffRpc, "adminOpenShift", None),
        // RE-7 #604 removes adminSaveRate (rates.md §9): it was RatesDraft on both paths
        (E::Rpc, "adminSaveRate", None),
        (E::StaffRpc, "adminSaveRate", None),
        (E::Rpc, "documents", None),
        (E::Rpc, "opsClock", None),
        (E::Rpc, "nope", None),
        (E::StaffRpc, "nope", None),
        (E::StaffRpc, "Ops", None),
        (E::StaffRpc, "adminCreateBranch", None),
        (E::Kiosk, "nope", None),
        (E::PaymentMethods, "nope", None),
        (E::StaffBookings, "nope", None),
    ];
    let mut wrong = Vec::new();
    for (on, name, want) in rows {
        let got = need(on, name);
        if got != want {
            wrong.push(format!(
                "{} {name:?}: want {want:?}, got {got:?}",
                path_name(on)
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_role_table_holds_what_b1_translates() {
    // §4.2's "Roles after" column, where §3.4's first table (OR-2) said otherwise
    let has = |r: Role, p: Permission| RolePermissions::of(r).contains(&p);
    // StaffFile and the bridge's `documents` let BACKEND and MANAGER in (v2): BRANCH_MANAGER reads them
    assert!(has(Role::BranchManager, Permission::IdentityDocumentRead));
    // /files refuses RATES and SYSTEM (v2: Not found): RATE_MANAGER and ADMIN hold no FileRead
    assert!(!has(Role::RateManager, Permission::FileRead));
    assert!(!has(Role::Admin, Permission::FileRead));
    // C6: MARKETING never opens identity documents (§02), on no door
    for p in [
        Permission::DocumentsList,
        Permission::FileRead,
        Permission::IdentityDocumentRead,
    ] {
        assert!(!has(Role::Marketing, p), "{p:?}");
    }
    // #874 (amended by its ticket author claude/f03345fe, per lane controller 2026-10-10): the
    // branch's manager and FINANCE (RG4 of #682, reversals) decide what each type's policy lets it (T2's over-limit exchange); rates stay
    // the owner's through `RatePublicationPolicy`
    assert_eq!(
        BLUEPRINT
            .into_iter()
            .filter(|r| has(*r, Permission::ApprovalsDecide))
            .collect::<Vec<_>>(),
        [Role::Executive, Role::BranchManager, Role::Finance]
    );
    // OWNER-only rows: EXECUTIVE; and nobody else holds them
    for p in [Permission::ReferenceRatesRead, Permission::RequestAbandon] {
        assert_eq!(
            BLUEPRINT
                .into_iter()
                .filter(|r| has(*r, p))
                .collect::<Vec<_>>(),
            [Role::Executive],
            "{p:?}"
        );
    }
    // #602's ruling: RATE_MANAGER saves a policy version (the command keeps it to the percents)
    assert_eq!(
        BLUEPRINT
            .into_iter()
            .filter(|r| has(*r, Permission::RatePolicySave))
            .collect::<Vec<_>>(),
        [Role::Executive, Role::RateManager]
    );
    // the rows §4.2 keeps as they are
    assert!(has(Role::RateManager, Permission::DocumentsList));
    assert!(has(Role::Admin, Permission::DocumentsList));
    assert!(has(Role::Executive, Permission::FileRead));
}

#[test]
fn a_v3_role_becomes_the_blueprint_roles_of_its_duty() {
    // §3.3 (B1, B3)
    let table = [
        (V3Role::Owner, vec![Role::Executive, Role::Admin]),
        (V3Role::Front, vec![Role::Teller]),
        (V3Role::Backend, vec![Role::BranchManager]),
        (V3Role::Manager, vec![Role::BranchManager]),
        (V3Role::Rates, vec![Role::RateManager]),
        (V3Role::Finance, vec![Role::Finance]),
        (V3Role::System, vec![Role::Admin]),
        (V3Role::Monitor, vec![Role::VerificationReviewer]),
        (V3Role::Documents, vec![Role::VerificationReviewer]),
        (V3Role::DocumentReview, vec![Role::VerificationReviewer]),
    ];
    for (v3, want) in table {
        assert_eq!(blueprint_roles(v3), want.as_slice(), "{}", v3.as_str());
    }
}

#[test]
fn a_password_session_becomes_the_user_b1_makes_of_it() {
    // the owner: EXECUTIVE + ADMIN, organisation-wide; an employee: their role(s) on their branch
    let o = actor_from_v3(&V3Actor::owner("owner"), ORG, since());
    assert_eq!(o.user_id, "owner");
    assert_eq!(o.organization_id, ORG);
    assert_eq!(o.customer_id, None);
    assert_eq!(
        o.roles,
        [
            ScopedRole::new(Role::Executive, None, since(), None),
            ScopedRole::new(Role::Admin, None, since(), None)
        ]
    );
    let e = V3Actor::staff("E-1", V3Role::Manager, branch("B2")).expect("a staff role");
    let m = actor_from_v3(&e, ORG, since());
    assert_eq!(m.user_id, "E-1");
    assert_eq!(
        m.roles,
        [ScopedRole::new(
            Role::BranchManager,
            Some(branch("B2")),
            since(),
            None
        )]
    );
    // and that user is answered as the mapped fixture answers its v2 caller
    assert_eq!(ask(Some(&m), EntryPath::Devices, "printerOnline"), Ok(())); // C7
    assert_eq!(ask(Some(&o), EntryPath::OwnerRoute, "employees"), Ok(()));
}
