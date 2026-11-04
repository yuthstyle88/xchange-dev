//! The branch currency set (`docs/design/board-currencies.md`): which currencies a branch handles and what a
//! customer sees of each. Every currency list reads this; prices stay in publications.

use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::currency::denomination_groups::GroupCode;
use crate::domain::draft::rate_engine_error::RateEngineError;

/// `branch_currencies.display` (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Display {
    /// Listed and priced for customers.
    Show,
    /// Listed as "contact staff"; only the desk can quote it.
    Ask,
    /// On no customer screen, and the desk cannot quote it. Held stock can still be counted and transferred.
    Hide,
}

impl Display {
    /// `"SHOW"` · `"ASK"` · `"HIDE"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Display::Show => "SHOW",
            Display::Ask => "ASK",
            Display::Hide => "HIDE",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "SHOW" => Some(Display::Show),
            "ASK" => Some(Display::Ask),
            "HIDE" => Some(Display::Hide),
            _ => None,
        }
    }
}

/// One row of a branch's set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCurrency {
    pub code: CurrencyCode,
    pub group: GroupCode,
    pub display: Display,
    pub sort_order: i32,
}

pub trait BranchCurrencies: Send + Sync {
    /// The branch's rows, by `sort_order`, then code and group.
    fn rows_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, Result<Vec<BranchCurrency>, RateEngineError>>;

    /// The display of one row, or `None` when the branch has no such row.
    fn display_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        code: CurrencyCode,
        group: &'a GroupCode,
    ) -> BoxFuture<'a, Result<Option<Display>, RateEngineError>>;
}
