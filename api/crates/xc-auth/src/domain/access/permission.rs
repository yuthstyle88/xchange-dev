use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Permission {
    OverviewRead,
    RatesDraft,
    RatesHistoryRead,
    RatePolicySave,
    BranchesManage,
    UsersManage,
    ApprovalsDecide,
    ShiftOperate,
    ShiftCloseCertify,
    BookingsDesk,
    ExchangeTransact,
    TransactionRead,
    ReceiptReprint,
    ReversalRequest,
    CashManage,
    QueueOperate,
    SystemSettings,
    CompatRead,
    RequestAbandon,
    DocumentsList,
    FileRead,
    ProofUpload,
    DeviceQueue,
    DeviceProcess,
    DevicePrinterOnline,
    DrawerPassport,
    FinanceReconcile,
    VerificationReview,
    IdentityDocumentRead,
    MarketingCampaigns,
    MarketingMembers,
    MarketingPerformance,
    CustomerSelf,
    AttendanceClock,
    CustomerHoldPlace,
    CashTargetSet,
    BranchDayOperate,
    SignedIn,
    /// v3-only (#550 OR-4): the owner route `/api/reference-rates`.
    ReferenceRatesRead,
    /// Consent policy manage permission (§3, §5.1, CI-6).
    ConsentPolicyManage,
    /// RA-3 (#981, O-2): attach a referral code to an exchange. TELLER and EXECUTIVE.
    ReferralAttach,
    /// RA-3 (#981, O-2): replace an ACTIVE referral code, with a reason. EXECUTIVE only.
    ReferralSupersede,
    /// RA-3 (#981, O-2): see the referral attributions of a branch. EXECUTIVE and TELLER.
    ReferralRead,
    /// LG-3b (#926, ledger design §11 Q-2): set a branch's `cash_limits`. FINANCE and ADMIN only,
    /// not BRANCH_MANAGER, whose own counts the limit decides on.
    CashLimitsManage,
    /// EX-1 (#1070, executive-finance design §7.1, owner decision E-2): export a report or a table.
    /// EXECUTIVE and FINANCE.
    ReportsExport,
    /// EX-1 (#1070, §7.1, owner decision E-6): edit the alert thresholds. EXECUTIVE and ADMIN.
    AlertSettingsSave,
    /// CA-0p (#1272, cash-shift-transfer blueprint §3.1, §9): set cash-transfer limit policy. EXECUTIVE only.
    CashTransferPolicySet,
    /// RA-2b (#1429, rate-alerts design §3): configure the organisation's rate alert settings. EXECUTIVE only.
    RateAlertSettingsSave,
    /// SP-1 (#1585, rates design §12.6, §12.10): enter org starting prices. EXECUTIVE only.
    OrgStartingPricesManage,
    /// PT-14 (#3968, pricing-tiers design §PT-14, line 210): refresh market rates (`pricing.market:refresh`).
    MarketRatesRefresh,
    /// LY-6 (#1411, membership-points design §3, §4.6): draft and submit loyalty rule versions. EXECUTIVE only.
    LoyaltyRulesManage,
    /// C2 (user-management-rbac design §3.3): read the account audit and the login history. EXECUTIVE and ADMIN.
    AuditLogsRead,
    /// RBAC §11.9: decide a `CASH_TRANSFER` within the limit, on a row covering `from_branch`. Seeded on BRANCH_MANAGER.
    ApproveCashTransferWithinLimit,
    /// RBAC §11.9: decide a `CASH_TRANSFER` above the limit or with no limit, any row. Seeded on EXECUTIVE.
    ApproveCashTransferAboveLimit,
    /// RBAC §11.9: decide a `COUNT_DIFFERENCE` on a row covering its branch. Seeded on FINANCE.
    ApproveCountDifference,
    /// RBAC §11.9: decide a `NEGATIVE_STOCK` on a row covering its branch. Seeded on BRANCH_MANAGER.
    ApproveNegativeStock,
    /// RBAC §11.9: decide a `CAMPAIGN_VERSION`, any row. Seeded on EXECUTIVE.
    ApproveCampaignVersion,
    /// RBAC §11.9: decide a `RATE_PUBLICATION`, any row. Seeded on EXECUTIVE.
    ApproveRatePublication,
    /// RBAC §11.9: decide an `EXCHANGE_TRANSACTION` on a row covering its branch. Seeded on BRANCH_MANAGER.
    ApproveExchangeTransaction,
    /// RBAC §11.9: decide a `TRANSACTION_REVERSAL` on a row covering its branch. Seeded on FINANCE.
    ApproveTransactionReversal,
    /// RBAC §11.9: decide a `LOYALTY_RULE_VERSION`, any row. Seeded on EXECUTIVE.
    ApproveLoyaltyRuleVersion,
}

impl Permission {
    /// Every permission, in declaration order (RBAC §11: the seed, the guardrails and the matrix view walk it).
    pub const ALL: [Permission; 61] = [
        Permission::OverviewRead,
        Permission::RatesDraft,
        Permission::RatesHistoryRead,
        Permission::RatePolicySave,
        Permission::BranchesManage,
        Permission::UsersManage,
        Permission::ApprovalsDecide,
        Permission::ShiftOperate,
        Permission::ShiftCloseCertify,
        Permission::BookingsDesk,
        Permission::ExchangeTransact,
        Permission::TransactionRead,
        Permission::ReceiptReprint,
        Permission::ReversalRequest,
        Permission::CashManage,
        Permission::QueueOperate,
        Permission::SystemSettings,
        Permission::CompatRead,
        Permission::RequestAbandon,
        Permission::DocumentsList,
        Permission::FileRead,
        Permission::ProofUpload,
        Permission::DeviceQueue,
        Permission::DeviceProcess,
        Permission::DevicePrinterOnline,
        Permission::DrawerPassport,
        Permission::FinanceReconcile,
        Permission::VerificationReview,
        Permission::IdentityDocumentRead,
        Permission::MarketingCampaigns,
        Permission::MarketingMembers,
        Permission::MarketingPerformance,
        Permission::CustomerSelf,
        Permission::AttendanceClock,
        Permission::CustomerHoldPlace,
        Permission::CashTargetSet,
        Permission::BranchDayOperate,
        Permission::SignedIn,
        Permission::ReferenceRatesRead,
        Permission::ConsentPolicyManage,
        Permission::ReferralAttach,
        Permission::ReferralSupersede,
        Permission::ReferralRead,
        Permission::CashLimitsManage,
        Permission::ReportsExport,
        Permission::AlertSettingsSave,
        Permission::CashTransferPolicySet,
        Permission::RateAlertSettingsSave,
        Permission::OrgStartingPricesManage,
        Permission::MarketRatesRefresh,
        Permission::LoyaltyRulesManage,
        Permission::AuditLogsRead,
        Permission::ApproveCashTransferWithinLimit,
        Permission::ApproveCashTransferAboveLimit,
        Permission::ApproveCountDifference,
        Permission::ApproveNegativeStock,
        Permission::ApproveCampaignVersion,
        Permission::ApproveRatePublication,
        Permission::ApproveExchangeTransaction,
        Permission::ApproveTransactionReversal,
        Permission::ApproveLoyaltyRuleVersion,
    ];
}
