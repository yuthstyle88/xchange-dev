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
    /// `binding-quote.md` D-13 (O-1 A): issue a binding quote (`bindingQuoteIssue`). No module; seeded on TELLER and
    /// BRANCH_MANAGER only. BQ-7 adds it to `ALL`, `RoleGrants::seed` and `SEED_ADDITIONS` (version 6).
    BindingQuoteIssue,
    /// Staff corrections SC-D2 (`docs/design/staff-corrections.md`): close another employee's forgotten clock-in
    /// from an earlier day (`attendanceCloseFor`, view `open_attendance`), on a row covering the branch. Seeded on
    /// BRANCH_MANAGER, EXECUTIVE and ADMIN.
    AttendanceCloseFor,
    /// `ledger-inventory.md` §13.3: decide a `LEDGER_ADJUSTMENT` on a row covering the adjustment's branch (the
    /// payload's, not the requester's). Seeded on FINANCE only. LA-1 adds it to `ALL`, `APPROVAL_KINDS`,
    /// `RoleGrants::seed` and `SEED_ADDITIONS` (version 8).
    ApproveLedgerAdjustment,
    /// #5733 (owner ruling A on #5683; `pricing-formula.md` §5 F-2 *"EXECUTIVE เท่านั้น"*): save the organisation's
    /// price formula (`pricingFormulaSave`) and preview it at scope `ORGANISATION`. No module, so RatesStock at Full
    /// (RATE_MANAGER) never implies it; seeded on EXECUTIVE only at seed v9, and editable on the `/users` matrix.
    PricingFormulaSave,
    /// RBAC §11.11 (#6004): read any business setting. No module; seeded on every built-in staff group at seed v10.
    #[serde(rename = "settingsRead")]
    SettingsRead,
    /// RBAC §11.11 (#6004): save branch-level business settings (`bookingPolicySet`, `slotRuleSet`, `slotCapacitySet`,
    /// `branchExchangeLimitSave`). No module; seeded on EXECUTIVE only at seed v10.
    #[serde(rename = "branchSettingsSave")]
    BranchSettingsSave,
    /// Backdated keying (`docs/design/backdated-keying.md` §3; owner rule R1 of 2026-11-04 *"Owner and Manager roles
    /// only"*): key an exchange at its real past time with no shift (`transactionBackdate`). No module; seeded on
    /// EXECUTIVE (which the Owner holds) and BRANCH_MANAGER only, at seed v11.
    #[serde(rename = "exchangeBackdate")]
    ExchangeBackdate,
    /// XC Counter kiosk exit (`docs/design/xc-counter-app.md` §8.8; owner decision on xchange#6569, 2026-11-09):
    /// issue the one-time code that unlocks a locked counter (`counterUnlockIssue`). No module; seeded on EXECUTIVE
    /// (which the Owner holds) and BRANCH_MANAGER only, at seed v12.
    #[serde(rename = "counterUnlock")]
    CounterUnlock,
    /// The Owner's pricing rules (`docs/design/blueprint/owner-set-values.md` §2; xchange#6555): save a new version
    /// of the organisation's pricing rules (`pricingRulesSave`). No module; seeded on EXECUTIVE only, at seed v13.
    #[serde(rename = "pricingRulesSave")]
    PricingRulesSave,
}

impl Permission {
    /// Every permission, in declaration order (RBAC §11: the seed, the guardrails and the matrix view walk it).
    pub const ALL: [Permission; 70] = [
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
        Permission::BindingQuoteIssue,
        Permission::AttendanceCloseFor,
        Permission::ApproveLedgerAdjustment,
        Permission::PricingFormulaSave,
        Permission::SettingsRead,
        Permission::BranchSettingsSave,
        Permission::ExchangeBackdate,
        Permission::CounterUnlock,
        Permission::PricingRulesSave,
    ];
}
