//! `cash_accounts` (design §3.1). Scaffold by the ticket author.

use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

/// A cash account's id (`cash_accounts.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CashAccountId(pub Uuid);

/// Where money is: `kind` in `cash_accounts` (design §3.1, §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccountKind {
    Drawer,
    Safe,
    InTransit,
    Customer,
    Bank,
    Variance,
}

impl AccountKind {
    /// The stored value: `DRAWER`, `SAFE`, `IN_TRANSIT`, `CUSTOMER`, `BANK`, `VARIANCE`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Drawer => "DRAWER",
            Self::Safe => "SAFE",
            Self::InTransit => "IN_TRANSIT",
            Self::Customer => "CUSTOMER",
            Self::Bank => "BANK",
            Self::Variance => "VARIANCE",
        }
    }
}

/// One `cash_accounts` row. `shift` is set exactly for a DRAWER (#582 C1); `branch` is `None` for an
/// organisation-wide account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CashAccount {
    pub id: CashAccountId,
    pub branch: Option<BranchId>,
    pub currency: CurrencyCode,
    pub kind: AccountKind,
    pub shift: Option<String>,
}
