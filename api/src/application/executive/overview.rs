//! `ExecutiveOverviewService` (design `executive-finance.md` §3, §4, D-5): A01's figures over the branches in
//! scope, from the source ports only.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

use super::drilldown::Drilldown;
use super::metrics::{
    AVAILABLE_CASH, BOOKING_CONVERSION, BUY_VOLUME_THB, COMMISSION_THB, Definition, IN_TRANSIT,
    Metric, MetricUnavailable, REALIZED_MARGIN_THB, REPEAT_CUSTOMERS, SELL_VOLUME_THB, definitions,
    thb,
};
use super::ports::{
    Alert, AlertFacts, CashFacts, CommissionSource, ExchangeFacts, ExecutiveFilter, ShiftStatus,
};

/// What A01 knows of a branch before its figures: its name and when its data last changed (D-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHead {
    pub id: BranchId,
    pub name: String,
    pub as_of: Option<DateTime<Utc>>,
}

/// One row of the branch table (design §4.2, §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRow {
    pub head: BranchHead,
    pub volume_thb: Decimal,
    pub available_cash: Metric,
    pub in_transit: Metric,
    pub shifts: ShiftStatus,
    pub pending_events: i64,
    pub drilldown: Drilldown,
}

/// A01's data: the organisation KPIs over the whole scope, and the rows and alerts of the changed branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub kpis: Vec<Metric>,
    pub branches: Vec<BranchRow>,
    pub alerts: Vec<Alert>,
    pub definitions: &'static [Definition],
}

pub struct ExecutiveOverviewService {
    exchange: Arc<dyn ExchangeFacts>,
    cash: Arc<dyn CashFacts>,
    alerts: Arc<dyn AlertFacts>,
    commission: Arc<dyn CommissionSource>,
}

impl ExecutiveOverviewService {
    pub fn new(
        exchange: Arc<dyn ExchangeFacts>,
        cash: Arc<dyn CashFacts>,
        alerts: Arc<dyn AlertFacts>,
        commission: Arc<dyn CommissionSource>,
    ) -> Self {
        Self {
            exchange,
            cash,
            alerts,
            commission,
        }
    }

    /// The KPIs over `scope` (recomputed whenever any branch changed: repeat customers are not a sum of branches),
    /// and the rows and alerts of `changed`, one row per head in `changed`, in the heads' order.
    pub async fn overview(
        &self,
        scope: &[BranchId],
        heads: &[BranchHead],
        filter: &ExecutiveFilter,
        changed: &[BranchId],
    ) -> AppResult<Overview> {
        let volumes = self.exchange.volumes(scope, filter).await?;
        let repeat = self.exchange.repeat_customers(scope, filter).await?;
        let margin = self.cash.realized_margin(scope, filter).await?;
        let commission = self.commission.commission(scope, filter).await?;

        let kpis = vec![
            Metric::value(
                BUY_VOLUME_THB,
                thb(volumes.buy_thb),
                Some(transactions_drilldown(filter, "BUY")),
            ),
            Metric::value(
                SELL_VOLUME_THB,
                thb(volumes.sell_thb),
                Some(transactions_drilldown(filter, "SELL")),
            ),
            money_metric(REALIZED_MARGIN_THB, margin)?,
            Metric::value(
                REPEAT_CUSTOMERS,
                repeat.to_string(),
                Some(customers_drilldown(filter)),
            ),
            Metric::unavailable(BOOKING_CONVERSION, MetricUnavailable::AwaitsBookingDesign),
            money_metric(COMMISSION_THB, commission)?,
        ];

        let mut branches = Vec::new();
        for head in heads.iter().filter(|h| changed.contains(&h.id)) {
            let side = volumes.by_branch.get(&head.id).cloned().unwrap_or_default();
            let available = self.cash.available_cash(&head.id).await?;
            let in_transit = self.cash.in_transit(&head.id).await?;
            let mut cash_keys = vec![("branch", head.id.as_str().to_owned())];
            if let Some(c) = &filter.currency {
                cash_keys.push(("currency", c.to_string()));
            }
            branches.push(BranchRow {
                volume_thb: side.buy_thb + side.sell_thb,
                available_cash: Metric::value(
                    AVAILABLE_CASH,
                    thb(available),
                    Some(Drilldown::to("admin_cash", cash_keys)),
                ),
                in_transit: Metric::value(IN_TRANSIT, thb(in_transit), None),
                shifts: self.cash.shift_status(&head.id, filter).await?,
                pending_events: self.exchange.unfinished(&head.id).await?,
                drilldown: overview_drilldown(filter, &head.id),
                head: head.clone(),
            });
        }

        let mut alerts = self.alerts.alerts(changed).await?;
        alerts.sort_by(|a, b| {
            (a.branch.as_str(), a.kind, a.since).cmp(&(b.branch.as_str(), b.kind, b.since))
        });

        Ok(Overview {
            kpis,
            branches,
            alerts,
            definitions: definitions(),
        })
    }
}

fn money_metric(key: &'static str, m: Result<Money, MetricUnavailable>) -> AppResult<Metric> {
    match m {
        Ok(money) => {
            let amount = Decimal::from_str(&money.to_string())
                .map_err(|e| AppError::Internal(format!("{key}: {e}")))?;
            Ok(Metric::value(key, thb(amount), None))
        }
        Err(reason) => Ok(Metric::unavailable(key, reason)),
    }
}

/// A01's period, currency and channel keys, in A01's order, for the ones the request set.
fn period_keys(filter: &ExecutiveFilter) -> Vec<(&'static str, String)> {
    let mut keys = Vec::new();
    if let Some(from) = filter.from {
        keys.push(("from", from.to_string()));
    }
    if let Some(to) = filter.to {
        keys.push(("to", to.to_string()));
    }
    keys
}

fn narrowing_keys(filter: &ExecutiveFilter) -> Vec<(&'static str, String)> {
    let mut keys = Vec::new();
    if let Some(c) = &filter.currency {
        keys.push(("currency", c.to_string()));
    }
    if let Some(ch) = filter.channel {
        keys.push(("channel", ch.as_str().to_owned()));
    }
    keys
}

fn transactions_drilldown(filter: &ExecutiveFilter, side: &str) -> Drilldown {
    let mut keys = period_keys(filter);
    keys.push(("type", side.to_owned()));
    keys.push(("status", "COMPLETED".to_owned()));
    keys.extend(narrowing_keys(filter));
    Drilldown::to("admin_transactions", keys)
}

fn customers_drilldown(filter: &ExecutiveFilter) -> Drilldown {
    let mut keys = vec![("repeat", "true".to_owned())];
    keys.extend(period_keys(filter));
    keys.extend(narrowing_keys(filter));
    Drilldown::to("admin_customers", keys)
}

fn overview_drilldown(filter: &ExecutiveFilter, branch: &BranchId) -> Drilldown {
    let mut keys = vec![("branch", branch.as_str().to_owned())];
    keys.extend(period_keys(filter));
    keys.extend(narrowing_keys(filter));
    Drilldown::to("admin_overview", keys)
}
