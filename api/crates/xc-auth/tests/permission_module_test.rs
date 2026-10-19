//! Up-front tests for C2 (user-management-rbac design §3.3, §5; ruled table: #564, claude/c81c8986,
//! 2026-10-18T04:26Z). Written before the implementation by the ticket author (claude/dell-claude-4).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `PermissionModule::module` maps each permission to (module, level) or `None`, `PermissionMatrix::of`
//! takes the highest level per module over a role's permissions, `AuditLogsRead` goes to EXECUTIVE and
//! ADMIN, and the design doc's §5 table equals the code (V.9: "fails if this table and the seed disagree").

use std::collections::BTreeMap;

use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::permission_module::{
    Level, Module, PermissionMatrix, PermissionModule,
};
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;

use Level::{Create, Full, View};
use Module::*;

/// The ruled table, one row per permission (all 51). `None` = the no-module permissions (§11.2, §11.3 rule 5).
fn table() -> Vec<(Permission, Option<(Module, Level)>)> {
    use Permission as P;
    vec![
        (P::OverviewRead, Some((Dashboard, View))),
        (P::AlertSettingsSave, Some((Dashboard, Full))),
        (P::RatesHistoryRead, Some((RatesStock, View))),
        (P::ReferenceRatesRead, Some((RatesStock, View))),
        (P::RatesDraft, Some((RatesStock, Create))),
        (P::RatePolicySave, Some((RatesStock, Full))),
        (P::RateAlertSettingsSave, Some((RatesStock, Full))),
        (P::OrgStartingPricesManage, Some((RatesStock, Full))),
        (P::CashManage, Some((Finance, Full))),
        (P::CashTargetSet, Some((Finance, Full))),
        (P::CashLimitsManage, Some((Finance, Full))),
        (P::CashTransferPolicySet, Some((Finance, Full))),
        (P::FinanceReconcile, Some((Finance, Full))),
        (P::ApprovalsDecide, Some((Finance, Full))),
        // §11.2: the desk basics move from FINANCE CREATE to no module (`None`).
        (P::ShiftOperate, None),
        (P::BranchDayOperate, None),
        (P::ShiftCloseCertify, None),
        (P::ExchangeTransact, Some((Customers, Create))),
        (P::BookingsDesk, Some((Customers, Create))),
        (P::QueueOperate, Some((Customers, Create))),
        (P::CustomerHoldPlace, Some((Customers, Create))),
        (P::ReferralAttach, Some((Customers, Create))),
        (P::ReversalRequest, Some((Customers, Create))),
        (P::TransactionRead, Some((Customers, View))),
        (P::ReceiptReprint, Some((Customers, View))),
        (P::ReferralRead, Some((Customers, View))),
        (P::IdentityDocumentRead, Some((Customers, View))),
        (P::ReferralSupersede, Some((Customers, Full))),
        (P::ConsentPolicyManage, Some((Customers, Full))),
        (P::MarketingCampaigns, Some((Customers, Full))),
        (P::MarketingMembers, Some((Customers, Full))),
        (P::LoyaltyRulesManage, Some((Customers, Full))),
        (P::FileRead, Some((Documents, View))),
        // §11.2: the desk document upload permissions move from DOCUMENTS CREATE to no module (`None`).
        (P::ProofUpload, None),
        (P::DrawerPassport, None),
        (P::VerificationReview, Some((Documents, Full))),
        (P::MarketingPerformance, Some((Reports, View))),
        (P::ReportsExport, Some((Reports, Full))),
        (P::UsersManage, Some((UserManagement, Full))),
        (P::BranchesManage, Some((UserManagement, Full))),
        (P::SystemSettings, Some((SystemSettings, Full))),
        // §11.2: the desk device permissions move from SYSTEM_SETTINGS VIEW to no module (`None`).
        (P::DeviceQueue, None),
        (P::DeviceProcess, None),
        (P::DevicePrinterOnline, None),
        (P::AuditLogsRead, Some((AuditLogs, View))),
        (P::SignedIn, None),
        (P::AttendanceClock, None),
        (P::CompatRead, None),
        (P::RequestAbandon, None),
        (P::DocumentsList, None),
        (P::CustomerSelf, None),
    ]
}

/// The eight rows, derived from §11.2's seed (`RoleGrants::seed`) and the table above.
fn expected(role: Role) -> BTreeMap<Module, Level> {
    // §11.2: `PermissionMatrix::of` now reads `RoleGrants::of(role.as_str())`, seeded from the owner's V.9 matrix.
    let row: &[(Module, Level)] = match role {
        Role::Executive => &[
            (Dashboard, Full),
            (Customers, Full),
            (Documents, Full),
            (RatesStock, Full),
            (Finance, Full),
            (Reports, Full),
            (UserManagement, Full),
            (SystemSettings, Full),
            (AuditLogs, View),
        ],
        Role::Admin => &[
            (Dashboard, View),
            (Reports, View),
            (UserManagement, Full),
            (SystemSettings, Full),
            (AuditLogs, View),
        ],
        Role::RateManager => &[
            (Dashboard, Full),
            (Customers, Full),
            (Documents, View),
            (RatesStock, Full),
            (Reports, Full),
        ],
        Role::BranchManager => &[
            (Dashboard, Full),
            (Customers, Full),
            (Documents, Full),
            (Reports, View),
        ],
        Role::Teller => &[(Dashboard, View), (Customers, Create), (Documents, View)],
        Role::Finance => &[
            (Dashboard, Full),
            (Customers, View),
            (Documents, View),
            (RatesStock, View),
            (Finance, Full),
            (Reports, Full),
        ],
        Role::Marketing => &[(Customers, Full), (Reports, View)],
        Role::VerificationReviewer => &[
            (Dashboard, Full),
            (Customers, View),
            (Documents, Full),
            (RatesStock, View),
            (Reports, Full),
            (AuditLogs, View),
        ],
        Role::Customer => &[],
    };
    row.iter().copied().collect()
}

#[test]
fn c2_every_permission_maps_to_its_ruled_module_and_level() {
    let rows = table();
    assert_eq!(rows.len(), 51, "the table covers all 51 permissions");
    let wrong: Vec<String> = rows
        .iter()
        .filter(|(p, want)| PermissionModule::module(*p) != *want)
        .map(|(p, want)| {
            format!(
                "{p:?}: want {want:?}, got {:?}",
                PermissionModule::module(*p)
            )
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} permissions map wrongly:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn c2_exactly_the_cross_cutting_six_map_to_no_module() {
    let none: Vec<Permission> = table()
        .into_iter()
        .map(|(p, _)| p)
        .filter(|p| PermissionModule::module(*p).is_none())
        .collect();
    // §11.2: the cross-cutting six plus the desk basics and the five desk device/document permissions have no module.
    assert_eq!(
        none,
        vec![
            Permission::ShiftOperate,
            Permission::BranchDayOperate,
            Permission::ShiftCloseCertify,
            Permission::ProofUpload,
            Permission::DrawerPassport,
            Permission::DeviceQueue,
            Permission::DeviceProcess,
            Permission::DevicePrinterOnline,
            Permission::SignedIn,
            Permission::AttendanceClock,
            Permission::CompatRead,
            Permission::RequestAbandon,
            Permission::DocumentsList,
            Permission::CustomerSelf,
        ]
    );
}

#[test]
fn c2_each_staff_role_row_is_the_highest_level_per_module() {
    for role in PermissionMatrix::ROLES {
        assert_eq!(
            PermissionMatrix::of(role),
            expected(role),
            "row {}",
            role.as_str()
        );
    }
}

#[test]
fn c2_the_rows_are_the_eight_staff_roles_and_customer_has_none() {
    assert_eq!(
        PermissionMatrix::ROLES.map(Role::as_str),
        [
            "EXECUTIVE",
            "ADMIN",
            "RATE_MANAGER",
            "BRANCH_MANAGER",
            "TELLER",
            "FINANCE",
            "MARKETING",
            "VERIFICATION_REVIEWER"
        ]
    );
    assert!(PermissionMatrix::of(Role::Customer).is_empty());
}

#[test]
fn c2_audit_logs_read_holders_per_section_11_2() {
    // §11.2: AUDIT_LOGS VIEW is held by EXECUTIVE, ADMIN, and VERIFICATION_REVIEWER
    for role in PermissionMatrix::ROLES.into_iter().chain([Role::Customer]) {
        let holds = RoleGrants::seed(role).contains(&Permission::AuditLogsRead);
        let want = matches!(
            role,
            Role::Executive | Role::Admin | Role::VerificationReviewer
        );
        assert_eq!(holds, want, "{} holds AuditLogsRead", role.as_str());
    }
}

#[test]
fn c2_no_role_goes_above_view_on_audit_logs_and_risk_is_empty() {
    for role in PermissionMatrix::ROLES {
        let row = PermissionMatrix::of(role);
        assert!(
            row.get(&AuditLogs).is_none_or(|l| *l == View),
            "{} AUDIT_LOGS",
            role.as_str()
        );
        assert!(!row.contains_key(&Risk), "{} has RISK", role.as_str());
    }
}

#[test]
fn c2_role_permission_seed_counts_per_section_11_2() {
    // §11.2: Permission counts by role for the seed
    let counts: Vec<(&str, usize)> = PermissionMatrix::ROLES
        .into_iter()
        .chain([Role::Customer])
        .map(|r| (r.as_str(), RoleGrants::seed(r).len()))
        .collect();
    assert_eq!(
        counts,
        vec![
            ("EXECUTIVE", 47),
            ("ADMIN", 10),
            ("RATE_MANAGER", 30),
            ("BRANCH_MANAGER", 31),
            ("TELLER", 21),
            ("FINANCE", 22),
            ("MARKETING", 5),
            ("VERIFICATION_REVIEWER", 17),
            ("CUSTOMER", 2),
        ]
    );
}

#[test]
fn c2_levels_order_view_create_full() {
    assert!(View < Create && Create < Full);
    assert_eq!(
        [View, Create, Full].map(Level::as_str),
        ["VIEW", "CREATE", "FULL"]
    );
}

/// §5 of `docs/design/blueprint/user-management-rbac.md` holds the matrix as a markdown table: the header
/// `| Role | DASHBOARD | … | AUDIT_LOGS |` (`Module::ALL` order), one row per `PermissionMatrix::ROLES`
/// entry named by `Role::as_str`, each cell `Level::as_str` or `—`. It must equal the code.
#[test]
fn c2_the_design_docs_section_5_table_equals_the_code() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/design/blueprint/user-management-rbac.md"
    );
    let doc = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let section = doc
        .split("\n## ")
        .find(|s| s.starts_with("5."))
        .expect("a `## 5.` section");
    let header: Vec<&str> = std::iter::once("Role")
        .chain(Module::ALL.map(Module::as_str))
        .collect();
    let mut lines = section
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('|'));
    let cells = |l: &str| -> Vec<String> {
        l.trim_matches('|')
            .split('|')
            .map(|c| c.trim().trim_matches('`').to_string())
            .collect()
    };
    let head = lines
        .by_ref()
        .find(|l| cells(l).first().is_some_and(|c| c == "Role"))
        .expect("the §5 table's header row");
    assert_eq!(cells(head), header, "§5 header");
    let rows: Vec<Vec<String>> = lines
        .map(cells)
        .filter(|c| !c[0].starts_with('-') && !c[0].starts_with(':'))
        .collect();
    assert_eq!(
        rows.len(),
        PermissionMatrix::ROLES.len(),
        "§5 has one row per staff role: {rows:?}"
    );
    for (role, row) in PermissionMatrix::ROLES.into_iter().zip(rows) {
        let m = PermissionMatrix::of(role);
        let want: Vec<String> = std::iter::once(role.as_str().to_string())
            .chain(
                Module::ALL
                    .iter()
                    .map(|md| m.get(md).map_or("—", |l| l.as_str()).to_string()),
            )
            .collect();
        assert_eq!(row, want, "§5 row {}", role.as_str());
    }
}
