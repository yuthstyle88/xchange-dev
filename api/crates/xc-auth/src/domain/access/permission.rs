use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// v3-only (#550 OR-4): the owner route `/api/bookings` and the bookings owner actions.
    BookingsAdmin,
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
}
