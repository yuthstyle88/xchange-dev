//! C2 (user-management-rbac design §3.3, Q2 A): the read-only module matrix.
//!
//! Nothing reads this to authorise. It exists for the screen, the export and §5.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::permission::Permission;
use super::role::Role;
use super::role_permissions::RolePermissions;

/// V.9's ten modules, in V.9's column order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Module {
    Dashboard,
    Customers,
    Documents,
    RatesStock,
    Finance,
    Risk,
    Reports,
    UserManagement,
    SystemSettings,
    AuditLogs,
}

impl Module {
    pub const ALL: [Module; 10] = [
        Module::Dashboard,
        Module::Customers,
        Module::Documents,
        Module::RatesStock,
        Module::Finance,
        Module::Risk,
        Module::Reports,
        Module::UserManagement,
        Module::SystemSettings,
        Module::AuditLogs,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Module::Dashboard => "DASHBOARD",
            Module::Customers => "CUSTOMERS",
            Module::Documents => "DOCUMENTS",
            Module::RatesStock => "RATES_STOCK",
            Module::Finance => "FINANCE",
            Module::Risk => "RISK",
            Module::Reports => "REPORTS",
            Module::UserManagement => "USER_MANAGEMENT",
            Module::SystemSettings => "SYSTEM_SETTINGS",
            Module::AuditLogs => "AUDIT_LOGS",
        }
    }
}

/// V.9's levels above NONE, ordered `View < Create < Full` (NONE is the absence of a module).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    View,
    Create,
    Full,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::View => "VIEW",
            Level::Create => "CREATE",
            Level::Full => "FULL",
        }
    }
}

pub struct PermissionModule;

impl PermissionModule {
    /// The module and level a permission counts towards, or `None` for the cross-cutting six.
    pub fn module(p: Permission) -> Option<(Module, Level)> {
        use Level::{Create, Full, View};
        use Module::*;
        use Permission as P;
        match p {
            P::OverviewRead => Some((Dashboard, View)),
            P::AlertSettingsSave => Some((Dashboard, Full)),
            P::RatesHistoryRead | P::ReferenceRatesRead => Some((RatesStock, View)),
            P::RatesDraft => Some((RatesStock, Create)),
            P::RatePolicySave | P::RateAlertSettingsSave | P::OrgStartingPricesManage => {
                Some((RatesStock, Full))
            }
            P::CashManage
            | P::CashTargetSet
            | P::CashLimitsManage
            | P::CashTransferPolicySet
            | P::FinanceReconcile
            | P::ApprovalsDecide => Some((Finance, Full)),
            P::ShiftOperate | P::BranchDayOperate | P::ShiftCloseCertify => Some((Finance, Create)),
            P::ExchangeTransact
            | P::BookingsDesk
            | P::QueueOperate
            | P::CustomerHoldPlace
            | P::ReferralAttach
            | P::ReversalRequest => Some((Customers, Create)),
            P::TransactionRead | P::ReceiptReprint | P::ReferralRead | P::IdentityDocumentRead => {
                Some((Customers, View))
            }
            P::BookingsAdmin
            | P::ReferralSupersede
            | P::ConsentPolicyManage
            | P::MarketingCampaigns
            | P::MarketingMembers
            | P::LoyaltyRulesManage => Some((Customers, Full)),
            P::FileRead => Some((Documents, View)),
            P::ProofUpload | P::DrawerPassport => Some((Documents, Create)),
            P::VerificationReview => Some((Documents, Full)),
            P::MarketingPerformance => Some((Reports, View)),
            P::ReportsExport => Some((Reports, Full)),
            P::UsersManage | P::BranchesManage => Some((UserManagement, Full)),
            P::SystemSettings => Some((SystemSettings, Full)),
            P::DeviceQueue | P::DeviceProcess | P::DevicePrinterOnline => {
                Some((SystemSettings, View))
            }
            P::AuditLogsRead => Some((AuditLogs, View)),
            P::SignedIn
            | P::AttendanceClock
            | P::CompatRead
            | P::RequestAbandon
            | P::DocumentsList
            | P::CustomerSelf => None,
        }
    }
}

pub struct PermissionMatrix;

impl PermissionMatrix {
    /// The matrix's rows: the eight staff roles. CUSTOMER is not a row.
    pub const ROLES: [Role; 8] = [
        Role::Executive,
        Role::Admin,
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::VerificationReviewer,
    ];

    /// The highest level among `role`'s permissions in each module; a module it has none in is absent.
    pub fn of(role: Role) -> BTreeMap<Module, Level> {
        if !Self::ROLES.contains(&role) {
            return BTreeMap::new();
        }
        let mut out = BTreeMap::new();
        for p in RolePermissions::of(role) {
            if let Some((module, level)) = PermissionModule::module(*p) {
                out.entry(module)
                    .and_modify(|cur| {
                        if level > *cur {
                            *cur = level;
                        }
                    })
                    .or_insert(level);
            }
        }
        out
    }
}
