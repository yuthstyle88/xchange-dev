//! The executive source ports (design `executive-finance.md` §9, D-3): one port per kind of fact, each with
//! today's implementation in `infrastructure/executive/` and its successor later. A01 never changes when a
//! source does.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

use super::channel::Channel;
use super::drilldown::Drilldown;
use super::metrics::MetricUnavailable;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A01's filter (design §4.1). `from`/`to` are business dates; absent means today in each branch's timezone.
/// The branches themselves are the scope every port is called with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutiveFilter {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub currency: Option<CurrencyCode>,
    pub channel: Option<Channel>,
}

/// Completed LIVE volumes of the scope (design §3): gross THB per shop side, and per branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Volumes {
    pub buy_thb: Decimal,
    pub sell_thb: Decimal,
    pub buy_reversed_thb: Decimal,
    pub sell_reversed_thb: Decimal,
    pub by_branch: BTreeMap<BranchId, SideVolumes>,
}

/// One branch's completed LIVE volumes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideVolumes {
    pub buy_thb: Decimal,
    pub sell_thb: Decimal,
}

/// One payment method's completed totals of a branch-day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDayTotals {
    pub method: String,
    pub buy_thb: Decimal,
    pub sell_thb: Decimal,
    pub fees_thb: Decimal,
    pub bank_fees_thb: Decimal,
    /// The COMPLETED transactions whose first payment (`payment_no = 1`) is this method: the same transactions
    /// whose line totals this row carries (FN-1 #1068, ruling Q1).
    pub count: i64,
}

/// A branch-day's completed exchange totals: per payment method, ordered by method; posted reversals apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchDayTotals {
    pub by_method: Vec<MethodDayTotals>,
    pub reversals_thb: Decimal,
}

/// The booking conversion totals over the scope and the filter (design §3, §8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookingConversion {
    pub total: (i64, i64),
    pub by_branch: BTreeMap<BranchId, (i64, i64)>,
}

impl BookingConversion {
    pub fn new(total: (i64, i64), by_branch: BTreeMap<BranchId, (i64, i64)>) -> Self {
        Self { total, by_branch }
    }

    pub fn numerator(&self) -> i64 {
        self.total.0
    }

    pub fn denominator(&self) -> i64 {
        self.total.1
    }

    pub fn completed(&self) -> i64 {
        self.total.0
    }

    pub fn confirmed(&self) -> i64 {
        self.total.1
    }
}

/// The exchange facts A01 and F01 read. Only COMPLETED, LIVE transactions count.
pub trait ExchangeFacts: Send + Sync {
    /// Gross THB volumes per shop side over `scope` and the filter.
    fn volumes<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Volumes>>;

    /// Distinct customers with a completed transaction in the period, in `scope` and the filter, who have an
    /// earlier completed transaction anywhere in the organisation.
    fn repeat_customers<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<i64>>;

    /// Completed LIVE totals of `branch` on business date `date` (what FN-1 #1068 reads).
    fn branch_day_totals<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<BranchDayTotals>>;

    /// The branch's unfinished LIVE transactions (DRAFT, PENDING_APPROVAL, CONFIRMED): design §3 `data_freshness`.
    fn unfinished<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<i64>>;

    /// `unfinished` for every branch in `branches` at once (design §9).
    fn unfinished_by_branch<'a>(
        &'a self,
        branches: &'a [BranchId],
    ) -> BoxFuture<'a, AppResult<BTreeMap<BranchId, i64>>> {
        Box::pin(async move {
            let mut out = BTreeMap::new();
            for b in branches {
                out.insert(b.clone(), self.unfinished(b).await?);
            }
            Ok(out)
        })
    }

    /// Completed / confirmed bookings of the same appointment day (design §3).
    /// Default body answers today's `Unavailable(AwaitsBookingDesign)`.
    fn booking_conversion<'a>(
        &'a self,
        branches: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<BookingConversion, MetricUnavailable>>> {
        let _ = (branches, filter);
        Box::pin(async { Ok(Err(MetricUnavailable::AwaitsBookingDesign)) })
    }
}

/// A branch's shifts for the branch table's ปิดกะ column (design §4.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShiftStatus {
    pub open: i64,
    pub closed_verified: i64,
    pub closed_unverified: i64,
}

/// A branch's cash figures for the branch table (`CashFacts::branch_cash`, design §9).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchCash {
    pub available: Decimal,
    pub in_transit: Decimal,
    pub shifts: ShiftStatus,
}

/// The cash facts A01 reads (design §3 `available_cash`).
pub trait CashFacts: Send + Sync {
    /// Baht on hand less reservations (none exist before #555 phase 6).
    fn available_cash<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Decimal>>;

    /// Baht in transfers dispatched from the branch and not yet received.
    fn in_transit<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Decimal>>;

    /// Open shifts now, and the period's closed shifts by whether their close count is verified.
    fn shift_status<'a>(
        &'a self,
        branch: &'a BranchId,
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<ShiftStatus>>;

    /// `available_cash`, `in_transit` and `shift_status` for every branch in `branches` at once (design §9).
    fn branch_cash<'a>(
        &'a self,
        branches: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<BTreeMap<BranchId, BranchCash>>> {
        Box::pin(async move {
            let mut out = BTreeMap::new();
            for b in branches {
                out.insert(
                    b.clone(),
                    BranchCash {
                        available: self.available_cash(b).await?,
                        in_transit: self.in_transit(b).await?,
                        shifts: self.shift_status(b, filter).await?,
                    },
                );
            }
            Ok(out)
        })
    }

    /// Quantity on hand per currency, as `available_cash` reads it (the low-stock alert reads this).
    fn on_hand_by_currency<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<(CurrencyCode, Decimal)>>>;

    /// Realized margin of the scope, or why it is not available.
    fn realized_margin<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<Money, MetricUnavailable>>>;
}

/// The kinds of alert A01 raises (design §4.3). Wire names are fixed (ruling on #1071 gap 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlertKind {
    RatePending,
    StaleData,
    CountDifference,
    LowStock,
}

impl AlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::RatePending => "ratePending",
            AlertKind::StaleData => "staleData",
            AlertKind::CountDifference => "countDifference",
            AlertKind::LowStock => "lowStock",
        }
    }
}

/// One "ต้องดำเนินการ" item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub kind: AlertKind,
    pub branch: BranchId,
    /// When the item began; `None` when its source holds no time.
    pub since: Option<DateTime<Utc>>,
    pub detail: String,
    pub drilldown: Option<Drilldown>,
}

/// The alerts of the branches in `scope`.
pub trait AlertFacts: Send + Sync {
    fn alerts<'a>(&'a self, scope: &'a [BranchId]) -> BoxFuture<'a, AppResult<Vec<Alert>>>;
}

/// The period's commission expense, or why it is not available.
pub trait CommissionSource: Send + Sync {
    fn commission<'a>(
        &'a self,
        scope: &'a [BranchId],
        filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<Money, MetricUnavailable>>>;
}

/// Today's `CommissionSource`: no Broker seam exists yet, so commission is always `AwaitsBroker`. EX-10 adds the
/// real one.
pub struct BrokerNotConnected;

impl CommissionSource for BrokerNotConnected {
    fn commission<'a>(
        &'a self,
        _scope: &'a [BranchId],
        _filter: &'a ExecutiveFilter,
    ) -> BoxFuture<'a, AppResult<Result<Money, MetricUnavailable>>> {
        Box::pin(async { Ok(Err(MetricUnavailable::AwaitsBroker)) })
    }
}
