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
//!   list, given to EXECUTIVE, the B1 translation of OWNER (no role changes). (v2's bookings-admin permission went with v2's booking in BK-11 #1413.)
//! - (until OR-7.m w2, #1027) `xc_auth::domain::access::v3_roles::{blueprint_roles, actor_from_v3}`, the
//!   B1 mapping of a password session; deleted with password login, and its two tests with it.
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
        (
            E::Kiosk,
            "confirm",
            Some(Need::Permission(Permission::QueueOperate)),
        ),
        (E::Kiosk, "status", Some(Need::Public)),
        (E::Kiosk, "customerReceipt", Some(Need::Public)),
        (E::DrawerPassport, "drawer", p(P::DrawerPassport)),
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
    // §4.2's "Roles after" column, moved to §11.2's seed (`RoleGrants::seed`)
    let has = |r: Role, p: Permission| RoleGrants::seed(r).contains(&p);
    // StaffFile and the bridge's `documents` let BACKEND and MANAGER in (v2): BRANCH_MANAGER reads them
    assert!(has(Role::BranchManager, Permission::IdentityDocumentRead));
    // §11.2: RATE_MANAGER has Documents VIEW (FileRead); ADMIN has no Documents module
    assert!(has(Role::RateManager, Permission::FileRead));
    assert!(!has(Role::Admin, Permission::FileRead));
    // C6: MARKETING never opens identity documents (§02), on no door
    for p in [
        Permission::DocumentsList,
        Permission::FileRead,
        Permission::IdentityDocumentRead,
    ] {
        assert!(!has(Role::Marketing, p), "{p:?}");
    }
    // §11.2 "Branch cash basics": ApprovalsDecide is no-module, seeded on EXECUTIVE, BRANCH_MANAGER, and FINANCE
    assert_eq!(
        BLUEPRINT
            .into_iter()
            .filter(|r| has(*r, Permission::ApprovalsDecide))
            .collect::<Vec<_>>(),
        [Role::Executive, Role::BranchManager, Role::Finance]
    );
    // §11.2: ReferenceRatesRead (RatesStock VIEW) is held by EXECUTIVE, RATE_MANAGER, FINANCE, VERIFICATION_REVIEWER
    assert_eq!(
        BLUEPRINT
            .into_iter()
            .filter(|r| has(*r, Permission::ReferenceRatesRead))
            .collect::<Vec<_>>(),
        [
            Role::Executive,
            Role::RateManager,
            Role::Finance,
            Role::VerificationReviewer
        ]
    );
    // OWNER-only no-module row: EXECUTIVE; and nobody else holds it
    assert_eq!(
        BLUEPRINT
            .into_iter()
            .filter(|r| has(*r, Permission::RequestAbandon))
            .collect::<Vec<_>>(),
        [Role::Executive]
    );
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

// OR-7.m w2 (#1027): `a_v3_role_becomes_the_blueprint_roles_of_its_duty` and
// `a_password_session_becomes_the_user_b1_makes_of_it` pinned `v3_roles` (`blueprint_roles`,
// `actor_from_v3`), which went with password login. The two Blueprint answers the second one also
// checked stay, asked of the Blueprint users directly.
#[test]
fn the_blueprint_users_b1_made_are_answered_as_before() {
    // a BRANCH_MANAGER of B2 (B1's MANAGER) and the owner (EXECUTIVE + ADMIN)
    assert_eq!(
        ask(
            Some(&staff(Role::BranchManager, "B2")),
            EntryPath::Devices,
            "printerOnline"
        ),
        Ok(())
    ); // C7
    assert_eq!(
        ask(Some(&owner()), EntryPath::OwnerRoute, "employees"),
        Ok(())
    );
}
