//! `LedgerCashPostings`: the ledger's implementation of the posting port (LG-2 #924; design
//! `docs/design/blueprint/ledger-inventory.md` §4.1, §4.3–§4.6, §5.1).
//!
//! One `post_in` call is one balanced posting. It is worked out in full before anything is written:
//! the places and positions it moves, their costs, and whether it takes a drawer, a safe or a
//! position below zero. A refusal (`validationFailed`, `stockInsufficient`) therefore writes nothing.
//!
//! `new()` takes no pool, so the statements of LG-1's repositories run here on the caller's
//! transaction, against the same tables and with the same shapes.

use std::collections::HashMap;

use chrono::Utc;
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};
use xc_shift::domain::shift::value_objects::balances::Balances;

use crate::application::ledger_approval::negative_stock::{NEGATIVE_STOCK, NegativeStockSource};
use crate::domain::inventory::repository::MovementType;
use crate::domain::inventory::weighted_average::WeightedAverage;
use crate::domain::ledger::account::AccountKind;
use crate::domain::ledger::posting::{PostingId, SourceRef, SourceType};
use crate::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, LineCost, PlaceActivity, PostingRequest,
    PostingResult, ReversedCost, TransactionLineId,
};
use crate::infrastructure::stock_reservations::active_reserved_in;

/// Who a posting is recorded as made by: the port carries no actor. `[proposed]`
const POSTED_BY: &str = "system";

#[derive(Clone, Default)]
pub struct LedgerCashPostings;

impl LedgerCashPostings {
    pub fn new() -> Self {
        Self
    }
}

fn validation(detail: &str) -> AppError {
    AppError::Domain(format!("validationFailed: {detail}"))
}

fn stock_insufficient(detail: &str) -> AppError {
    AppError::Domain(format!("stockInsufficient: {detail}"))
}

fn is_thb(currency: CurrencyCode) -> bool {
    currency.as_str() == "THB"
}

fn round2(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
}

fn round8(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(8, RoundingStrategy::MidpointAwayFromZero)
}

/// The position's average, when it holds a positive quantity.
fn average(p: WeightedAverage) -> Option<Decimal> {
    (p.quantity > Decimal::ZERO).then(|| round8(p.cost_thb / p.quantity))
}

fn minor(v: Decimal) -> AppResult<i128> {
    (v * Decimal::from(100))
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i128()
        .ok_or_else(|| AppError::Internal("ledger amount out of range".into()))
}

fn money(v: Decimal) -> AppResult<Money> {
    let m = minor(v)?;
    Money::try_from_minor_i128(m)
        .map_err(|_| AppError::Internal(format!("ledger cost is not a money amount: {v}")))
}

fn delta(v: Decimal) -> AppResult<MoneyDelta> {
    Ok(MoneyDelta::from_minor(minor(v)?))
}

/// A `cash_accounts` row, by its natural key (design §3.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AccountKey {
    branch: String,
    currency: CurrencyCode,
    kind: AccountKind,
    shift: Option<String>,
}

/// One `transaction_lines` row's figures this port reads (design §4.3, §4.6).
struct LineRow {
    shop_side: String,
    thb_amount: Decimal,
    cost_basis_thb: Option<Decimal>,
}

/// A position as the posting moves it.
struct Position {
    now: WeightedAverage,
    /// The last movement's `average_after` (§4.5: the cost of a sale from an exactly-zero position).
    last_average: Option<Decimal>,
    /// Active reservation hold on this (branch, currency) under the position lock (SR-6, D-14).
    reserved: Decimal,
}

struct LotPlan {
    id: Uuid,
    branch: String,
    currency: CurrencyCode,
    quantity: Decimal,
    unit_cost_thb: Decimal,
}

struct MovementPlan {
    branch: String,
    currency: CurrencyCode,
    lot_id: Option<Uuid>,
    line: Option<Uuid>,
    movement_type: MovementType,
    quantity_signed: Decimal,
    cost_thb_signed: Decimal,
    average_after: Option<Decimal>,
}

/// Everything one posting writes, worked out before the first write.
struct Plan {
    organization_id: String,
    branch: String,
    positions: HashMap<(String, CurrencyCode), Position>,
    entries: Vec<(AccountKey, Decimal, Decimal)>,
    lots: Vec<LotPlan>,
    movements: Vec<MovementPlan>,
    line_updates: Vec<(Uuid, Decimal, Option<Decimal>)>,
    costs: Vec<LineCost>,
    reversed: Vec<ReversedCost>,
    goes_negative: bool,
    missing_buy_rate: Option<AppError>,
}

impl Plan {
    async fn position<'p>(
        &'p mut self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        currency: CurrencyCode,
    ) -> AppResult<&'p mut Position> {
        let key = (branch.to_owned(), currency);
        if !self.positions.contains_key(&key) {
            let row = sqlx::query(
                "SELECT quantity, cost_thb FROM inventory_positions \
                 WHERE organization_id = $1::uuid AND branch_id = $2 AND currency = $3 \
                 FOR UPDATE",
            )
            .bind(&self.organization_id)
            .bind(branch)
            .bind(currency.as_str())
            .fetch_optional(&mut **tx)
            .await?;
            let now = match row {
                Some(r) => WeightedAverage {
                    quantity: r.try_get("quantity")?,
                    cost_thb: r.try_get("cost_thb")?,
                },
                None => WeightedAverage::EMPTY,
            };
            let last_average: Option<Decimal> = sqlx::query_scalar(
                "SELECT average_after FROM inventory_movements \
                 WHERE organization_id = $1::uuid AND branch_id = $2 AND currency = $3 \
                   AND average_after IS NOT NULL \
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(&self.organization_id)
            .bind(branch)
            .bind(currency.as_str())
            .fetch_optional(&mut **tx)
            .await?;
            let reserved = if is_thb(currency) {
                Decimal::ZERO
            } else {
                let org = Uuid::parse_str(&self.organization_id).map_err(|e| {
                    AppError::Internal(format!(
                        "invalid organization_id '{}': {e}",
                        self.organization_id
                    ))
                })?;
                active_reserved_in(tx, org, &BranchId::new(branch), currency, Utc::now()).await?
            };
            self.positions.insert(
                key.clone(),
                Position {
                    now,
                    last_average,
                    reserved,
                },
            );
        }
        Ok(self.positions.get_mut(&key).expect("loaded above"))
    }

    /// Adds `q` units that cost `cost` (§4.3 BUY, §4.6 reversed SELL, §4.4 a transfer's receiver).
    /// A position brought back from below zero gets §4.5's correction: one cost-only `ADJUST` on the
    /// short sale's line (Q-4, lane controller's ruling A).
    #[allow(clippy::too_many_arguments)]
    async fn acquire(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        currency: CurrencyCode,
        q: Decimal,
        cost: Decimal,
        line: Option<Uuid>,
        movement_type: MovementType,
    ) -> AppResult<()> {
        let lot_id = Uuid::new_v4();
        let pos = self.position(tx, branch, currency).await?;
        let before = pos.now;
        pos.now = before.acquire(q, cost);
        let average_after = average(pos.now);
        if average_after.is_some() {
            pos.last_average = average_after;
        }
        let after = pos.now;
        self.lots.push(LotPlan {
            id: lot_id,
            branch: branch.to_owned(),
            currency,
            quantity: q,
            unit_cost_thb: round8(cost / q),
        });
        self.movements.push(MovementPlan {
            branch: branch.to_owned(),
            currency,
            lot_id: Some(lot_id),
            line,
            movement_type,
            quantity_signed: q,
            cost_thb_signed: cost,
            average_after,
        });

        if before.quantity < Decimal::ZERO && after.quantity >= Decimal::ZERO {
            // The short quantity was costed provisionally; its real cost is this lot's.
            let short = -before.quantity;
            let provisional = -before.cost_thb;
            let real = round2(short * cost / q);
            let correction = provisional - real;
            if !correction.is_zero() {
                let short_line = self.short_sale_line(tx, branch, currency).await?;
                let pos = self.position(tx, branch, currency).await?;
                pos.now.cost_thb += correction;
                let average_after = average(pos.now);
                if average_after.is_some() {
                    pos.last_average = average_after;
                }
                self.movements.push(MovementPlan {
                    branch: branch.to_owned(),
                    currency,
                    lot_id: None,
                    line: short_line,
                    movement_type: MovementType::Adjust,
                    quantity_signed: Decimal::ZERO,
                    cost_thb_signed: correction,
                    average_after,
                });
            }
        }
        Ok(())
    }

    /// Takes `q` units out and answers the cost moved. At the average (§4.3), or at `at_cost` for a
    /// reversed BUY (§4.6). Taking the last unit moves the whole remaining cost. Beyond the position,
    /// the shortfall is costed at the current average, or at the last `average_after` from zero (§4.5).
    #[allow(clippy::too_many_arguments)]
    async fn dispose(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        currency: CurrencyCode,
        q: Decimal,
        at_cost: Option<Decimal>,
        line: Option<Uuid>,
        movement_type: MovementType,
    ) -> AppResult<Decimal> {
        let (before, last_average, reserved) = {
            let pos = self.position(tx, branch, currency).await?;
            (pos.now, pos.last_average, pos.reserved)
        };
        if reserved > Decimal::ZERO && before.quantity - q < reserved {
            return Err(stock_insufficient(
                "the posting takes a position into another quote's reserved stock",
            ));
        }
        let cost = if before.quantity == q {
            before.cost_thb
        } else if let Some(c) = at_cost {
            c
        } else if before.quantity > q {
            before.dispose(q).0
        } else if before.quantity > Decimal::ZERO {
            let unit = before.cost_thb / before.quantity;
            before.cost_thb + round2((q - before.quantity) * unit)
        } else if before.quantity.is_zero() {
            let unit = match last_average {
                Some(avg) => avg,
                None => self.no_history_unit_cost(tx, branch, currency).await?,
            };
            round2(q * unit)
        } else {
            round2(q * before.cost_thb / before.quantity)
        };
        let pos = self.position(tx, branch, currency).await?;
        pos.now = WeightedAverage {
            quantity: before.quantity - q,
            cost_thb: before.cost_thb - cost,
        };
        let after = pos.now;
        let average_after = if after.quantity > Decimal::ZERO {
            average(after)
        } else if after.quantity.is_zero() {
            // The DISPOSE that empties a position records the average it emptied (lane controller,
            // issuecomment-6106564990 choice 4): §4.5's "the last movement's average_after".
            average(before).or(pos.last_average)
        } else {
            None
        };
        if average_after.is_some() {
            pos.last_average = average_after;
        }
        if after.quantity < Decimal::ZERO {
            self.goes_negative = true;
        }
        self.movements.push(MovementPlan {
            branch: branch.to_owned(),
            currency,
            lot_id: None,
            line,
            movement_type,
            quantity_signed: -q,
            cost_thb_signed: -cost,
            average_after,
        });
        Ok(cost)
    }

    /// Puts `q` units back at the position's average, with no lot: an overage out of `VARIANCE` is no
    /// purchase (§4.4 row 7). From an empty position, at the last `average_after` (§4.5).
    async fn adjust_in(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        currency: CurrencyCode,
        q: Decimal,
    ) -> AppResult<Decimal> {
        let (before, last_average) = {
            let pos = self.position(tx, branch, currency).await?;
            (pos.now, pos.last_average)
        };
        let unit = if before.quantity.is_zero() {
            match last_average {
                Some(avg) => avg,
                None => self.no_history_unit_cost(tx, branch, currency).await?,
            }
        } else {
            before.cost_thb / before.quantity
        };
        let cost = round2(q * unit);
        let pos = self.position(tx, branch, currency).await?;
        pos.now = WeightedAverage {
            quantity: before.quantity + q,
            cost_thb: before.cost_thb + cost,
        };
        let average_after = average(pos.now);
        if average_after.is_some() {
            pos.last_average = average_after;
        }
        self.movements.push(MovementPlan {
            branch: branch.to_owned(),
            currency,
            lot_id: None,
            line: None,
            movement_type: MovementType::Adjust,
            quantity_signed: q,
            cost_thb_signed: cost,
            average_after,
        });
        Ok(cost)
    }

    /// The unit cost when a position has no cost history (`last_average` is `None`): the branch's
    /// current published BUY rate (`round8(buy_rate / unit_size)`). When there is none, records the
    /// `validationFailed` refusal to be raised after the §4.5 negative-stock gate.
    async fn no_history_unit_cost(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        currency: CurrencyCode,
    ) -> AppResult<Decimal> {
        if let Some(unit) =
            published_buy_unit_cost(tx, &self.organization_id, branch, currency).await?
        {
            return Ok(unit);
        }
        if self.missing_buy_rate.is_none() {
            self.missing_buy_rate = Some(validation(&format!(
                "no published BUY rate for {} at branch {branch} to cost stock with no history",
                currency.as_str()
            )));
        }
        Ok(Decimal::ZERO)
    }

    /// The line of the latest disposal that left this position below zero: the one Q-4's `ADJUST`
    /// corrects.
    async fn short_sale_line(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        currency: CurrencyCode,
    ) -> AppResult<Option<Uuid>> {
        if let Some(m) = self.movements.iter().rev().find(|m| {
            m.branch == branch
                && m.currency == currency
                && m.quantity_signed < Decimal::ZERO
                && m.average_after.is_none()
                && m.line.is_some()
        }) {
            return Ok(m.line);
        }
        let line: Option<Option<Uuid>> = sqlx::query_scalar(
            "SELECT transaction_line_id FROM inventory_movements \
             WHERE organization_id = $1::uuid AND branch_id = $2 AND currency = $3 \
               AND quantity_signed < 0 AND average_after IS NULL \
               AND transaction_line_id IS NOT NULL \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(&self.organization_id)
        .bind(branch)
        .bind(currency.as_str())
        .fetch_optional(&mut **tx)
        .await?;
        Ok(line.flatten())
    }
}

/// The branch's current published BUY unit cost (`round8(buy_rate / unit_size)`) on the `ALL`
/// denomination group and `ALL` value tier (`docs/design/blueprint/ledger-inventory.md` §4.5, #2794).
async fn published_buy_unit_cost(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: &str,
    branch: &str,
    currency: CurrencyCode,
) -> AppResult<Option<Decimal>> {
    let row = sqlx::query(
        "SELECT l.buy_rate, l.unit_size \
         FROM branches b \
         JOIN rate_publications p ON p.id = b.current_publication_id \
         JOIN published_rate_lines l ON l.publication_id = p.id \
         WHERE b.organization_id = $1::uuid AND b.id = $2 AND l.currency_code = $3 \
           AND l.denomination_group = 'ALL' AND l.value_tier = 'ALL'",
    )
    .bind(organization_id)
    .bind(branch)
    .bind(currency.as_str())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let buy_rate: Option<Decimal> = row.try_get("buy_rate")?;
    let unit_size: Decimal = row.try_get("unit_size")?;
    Ok(buy_rate.map(|buy| round8(buy / unit_size)))
}

/// The branch a place belongs to; `None` for `Customer` and `Bank`, which take the posting's.
async fn branch_of(
    tx: &mut Transaction<'_, Postgres>,
    place: &CashPlace,
) -> AppResult<Option<String>> {
    match place {
        CashPlace::Drawer(shift) => {
            let branch: Option<String> =
                sqlx::query_scalar("SELECT branch_id FROM shifts WHERE id = $1")
                    .bind(shift)
                    .fetch_optional(&mut **tx)
                    .await?;
            branch
                .map(Some)
                .ok_or_else(|| validation("the drawer's shift does not exist"))
        }
        CashPlace::Safe(branch) | CashPlace::Variance(branch) => {
            Ok(Some(branch.as_str().to_owned()))
        }
        CashPlace::InTransit(transfer) => {
            // The IN_TRANSIT account belongs to the sender (§4.4).
            let branch: Option<String> =
                sqlx::query_scalar("SELECT from_branch_id FROM cash_transfers WHERE id::text = $1")
                    .bind(transfer)
                    .fetch_optional(&mut **tx)
                    .await?;
            branch
                .map(Some)
                .ok_or_else(|| validation("the transfer in transit does not exist"))
        }
        CashPlace::Customer | CashPlace::Bank => Ok(None),
    }
}

fn kind_of(place: &CashPlace) -> AccountKind {
    match place {
        CashPlace::Drawer(_) => AccountKind::Drawer,
        CashPlace::Safe(_) => AccountKind::Safe,
        CashPlace::InTransit(_) => AccountKind::InTransit,
        CashPlace::Customer => AccountKind::Customer,
        CashPlace::Bank => AccountKind::Bank,
        CashPlace::Variance(_) => AccountKind::Variance,
    }
}

fn is_inside(place: &CashPlace) -> bool {
    matches!(
        place,
        CashPlace::Drawer(_) | CashPlace::Safe(_) | CashPlace::InTransit(_)
    )
}

fn is_variance(place: &CashPlace) -> bool {
    matches!(place, CashPlace::Variance(_))
}

async fn line_row(
    tx: &mut Transaction<'_, Postgres>,
    line: TransactionLineId,
) -> AppResult<Option<LineRow>> {
    let row = sqlx::query(
        "SELECT shop_side, thb_amount, cost_basis_thb FROM transaction_lines WHERE id = $1",
    )
    .bind(line)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|r| {
        Ok(LineRow {
            shop_side: r.try_get("shop_side")?,
            thb_amount: r.try_get("thb_amount")?,
            cost_basis_thb: r.try_get("cost_basis_thb")?,
        })
    })
    .transpose()
}

/// The checks of §4.1 and §4.6 that need no database.
fn check_requests(req: &[PostingRequest]) -> AppResult<&SourceRef> {
    let source = &req[0].source;
    for r in req {
        if r.source != *source {
            return Err(validation(
                "every request of one posting has the same source",
            ));
        }
        if r.quantity.value() <= Decimal::ZERO {
            return Err(validation("a request moves a positive quantity"));
        }
        if r.from == r.to {
            return Err(validation("a request moves money between two places"));
        }
        if r.line.is_some() && r.reverses.is_some() {
            return Err(validation("line and reverses are not set together"));
        }
        if is_thb(r.currency) && (r.line.is_some() || r.reverses.is_some()) {
            return Err(validation(
                "a THB request carries neither line nor reverses",
            ));
        }
        if r.reverses.is_some() && r.source.source_type != SourceType::Reversal {
            return Err(validation(
                "reverses is sent only with source_type REVERSAL",
            ));
        }
        if (r.line.is_some() || r.reverses.is_some())
            && (is_variance(&r.from) || is_variance(&r.to))
        {
            return Err(validation(
                "line and reverses are not sent on a VARIANCE leg",
            ));
        }
        if (r.line.is_some() || r.reverses.is_some()) && is_inside(&r.from) == is_inside(&r.to) {
            return Err(validation(
                "line and reverses are sent on an exchange leg, between the shop and outside",
            ));
        }
    }
    Ok(source)
}

impl LedgerCashPostings {
    async fn post(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: &[PostingRequest],
        allow_negative: Option<ApprovalRequestId>,
    ) -> AppResult<PostingResult> {
        if req.is_empty() {
            return Ok(PostingResult {
                posting_id: PostingId(Uuid::nil()),
                costs: Vec::new(),
                reversed: Vec::new(),
            });
        }
        let source = check_requests(req)?;

        // The posting's branch: the first place in it that belongs to one.
        let mut places: Vec<(Option<String>, Option<String>)> = Vec::with_capacity(req.len());
        let mut branch: Option<String> = None;
        for r in req {
            let from = branch_of(tx, &r.from).await?;
            let to = branch_of(tx, &r.to).await?;
            if branch.is_none() {
                branch = from.clone().or_else(|| to.clone());
            }
            places.push((from, to));
        }
        let branch =
            branch.ok_or_else(|| validation("a posting names a drawer, safe or transfer"))?;
        let organization_id: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                .bind(&branch)
                .fetch_optional(&mut **tx)
                .await?
                .ok_or_else(|| validation("the posting's branch does not exist"))?;

        // The same source again: the posting is already there, and nothing is written twice.
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM ledger_postings \
             WHERE organization_id = $1::uuid AND source_type = $2 AND source_id = $3 AND source_key = $4",
        )
        .bind(&organization_id)
        .bind(source.source_type.as_str())
        .bind(&source.source_id)
        .bind(&source.source_key)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(id) = existing {
            return replayed(tx, PostingId(id), req).await;
        }

        // The THB the posting moves, for an exchange line with no stored row.
        let thb_total: Decimal = req
            .iter()
            .filter(|r| is_thb(r.currency))
            .map(|r| r.quantity.value())
            .sum();
        let foreign_exchange_legs = req
            .iter()
            .filter(|r| {
                !is_thb(r.currency)
                    && !is_variance(&r.from)
                    && !is_variance(&r.to)
                    && is_inside(&r.from) != is_inside(&r.to)
            })
            .count();

        let mut plan = Plan {
            organization_id: organization_id.clone(),
            branch: branch.clone(),
            positions: HashMap::new(),
            entries: Vec::new(),
            lots: Vec::new(),
            movements: Vec::new(),
            line_updates: Vec::new(),
            costs: Vec::new(),
            reversed: Vec::new(),
            goes_negative: false,
            missing_buy_rate: None,
        };

        for (r, (from_branch, to_branch)) in req.iter().zip(places) {
            let q = r.quantity.value();
            let from_branch = from_branch.unwrap_or_else(|| branch.clone());
            let to_branch = to_branch.unwrap_or_else(|| branch.clone());
            let valuation = if is_thb(r.currency) {
                // THB is not inventory and moves at face value (§3.1, §4.3).
                q
            } else if is_variance(&r.from) || is_variance(&r.to) {
                // A difference is no sale and no purchase: an `ADJUST` at the average (§4.4 rows 6, 7).
                match (is_inside(&r.from), is_inside(&r.to)) {
                    (true, false) => {
                        plan.dispose(
                            tx,
                            &from_branch,
                            r.currency,
                            q,
                            None,
                            None,
                            MovementType::Adjust,
                        )
                        .await?
                    }
                    (false, true) => plan.adjust_in(tx, &to_branch, r.currency, q).await?,
                    _ => Decimal::ZERO,
                }
            } else {
                match (is_inside(&r.from), is_inside(&r.to)) {
                    (true, true) if from_branch == to_branch => {
                        // A move inside one branch changes no position and no cost (§3.2).
                        Decimal::ZERO
                    }
                    (true, true) => {
                        // Cost travels with the money: out at the sender's average, in as a lot at
                        // that unit cost (§4.4).
                        let cost = plan
                            .dispose(
                                tx,
                                &from_branch,
                                r.currency,
                                q,
                                None,
                                None,
                                MovementType::TransferOut,
                            )
                            .await?;
                        plan.acquire(
                            tx,
                            &to_branch,
                            r.currency,
                            q,
                            cost,
                            None,
                            MovementType::TransferIn,
                        )
                        .await?;
                        cost
                    }
                    (false, true) => {
                        let place = &to_branch;
                        if let Some(reversed) = r.reverses {
                            let original = reversed_line(tx, reversed, "SELL").await?;
                            plan.acquire(
                                tx,
                                place,
                                r.currency,
                                q,
                                original,
                                None,
                                MovementType::Acquire,
                            )
                            .await?;
                            plan.reversed.push(ReversedCost {
                                reverses: reversed,
                                cost_thb: money(original)?,
                                difference_thb: delta(Decimal::ZERO)?,
                            });
                            original
                        } else {
                            let row = match r.line {
                                Some(line) => line_row(tx, line).await?,
                                None => None,
                            };
                            let cost = exchange_thb(&row, thb_total, foreign_exchange_legs)?;
                            plan.acquire(
                                tx,
                                place,
                                r.currency,
                                q,
                                cost,
                                r.line,
                                MovementType::Acquire,
                            )
                            .await?;
                            if let Some(line) = r.line {
                                // A BUY recognises no margin (§4.3).
                                if row.is_some() {
                                    plan.line_updates.push((line, cost, None));
                                }
                                plan.costs.push(LineCost {
                                    line,
                                    cost_basis_thb: money(cost)?,
                                    realized_margin_thb: None,
                                });
                            }
                            cost
                        }
                    }
                    (true, false) => {
                        let place = &from_branch;
                        if let Some(reversed) = r.reverses {
                            let original = reversed_line(tx, reversed, "BUY").await?;
                            let moved = plan
                                .dispose(
                                    tx,
                                    place,
                                    r.currency,
                                    q,
                                    Some(original),
                                    None,
                                    MovementType::Dispose,
                                )
                                .await?;
                            plan.reversed.push(ReversedCost {
                                reverses: reversed,
                                cost_thb: money(original)?,
                                difference_thb: delta(moved - original)?,
                            });
                            // The foreign entries carry ±C (§4.6).
                            original
                        } else {
                            let row = match r.line {
                                Some(line) => line_row(tx, line).await?,
                                None => None,
                            };
                            let cost = plan
                                .dispose(
                                    tx,
                                    place,
                                    r.currency,
                                    q,
                                    None,
                                    r.line,
                                    MovementType::Dispose,
                                )
                                .await?;
                            if let Some(line) = r.line {
                                let thb_amount =
                                    exchange_thb(&row, thb_total, foreign_exchange_legs)?;
                                let margin = thb_amount - cost;
                                if row.is_some() {
                                    plan.line_updates.push((line, cost, Some(margin)));
                                }
                                plan.costs.push(LineCost {
                                    line,
                                    cost_basis_thb: money(cost)?,
                                    realized_margin_thb: Some(delta(margin)?),
                                });
                            }
                            cost
                        }
                    }
                    (false, false) => Decimal::ZERO,
                }
            };
            let from_key = AccountKey {
                branch: from_branch,
                currency: r.currency,
                kind: kind_of(&r.from),
                shift: shift_of(&r.from),
            };
            let to_key = AccountKey {
                branch: to_branch,
                currency: r.currency,
                kind: kind_of(&r.to),
                shift: shift_of(&r.to),
            };
            plan.entries.push((from_key, -q, -valuation));
            plan.entries.push((to_key, q, valuation));
        }

        // §4.5: a drawer or safe below zero, as a position below zero, needs an approved request.
        let mut moved: HashMap<AccountKey, Decimal> = HashMap::new();
        for (key, q, _) in &plan.entries {
            if matches!(key.kind, AccountKind::Drawer | AccountKind::Safe) {
                *moved.entry(key.clone()).or_insert(Decimal::ZERO) += *q;
            }
        }
        for (key, change) in &moved {
            if *change < Decimal::ZERO
                && place_balance(tx, &organization_id, key).await? + *change < Decimal::ZERO
            {
                plan.goes_negative = true;
            }
        }
        if plan.goes_negative && !self.allows_negative(tx, source, allow_negative).await? {
            return Err(stock_insufficient(
                "the posting takes a drawer, a safe or a position below zero",
            ));
        }
        if let Some(err) = plan.missing_buy_rate {
            return Err(err);
        }

        write(tx, &plan, source).await
    }

    /// `allow_negative` names an APPROVED `NEGATIVE_STOCK` request for this source, decided at the
    /// source's current state (§5.1; lane controller's rulings issuecomment-6097791219 choice 4 and
    /// issuecomment-6097817311).
    async fn allows_negative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        source: &SourceRef,
        allow_negative: Option<ApprovalRequestId>,
    ) -> AppResult<bool> {
        let Some(request) = allow_negative else {
            return Ok(false);
        };
        let row = sqlx::query(
            "SELECT type, status, object_id, object_version_hash FROM approval_requests \
             WHERE id::text = $1",
        )
        .bind(request.as_str())
        .fetch_optional(&mut **tx)
        .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let type_name: String = row.try_get("type")?;
        let status: String = row.try_get("status")?;
        let object_id: String = row.try_get("object_id")?;
        let hash: String = row.try_get("object_version_hash")?;
        if type_name != NEGATIVE_STOCK || status != "APPROVED" || object_id != source.source_id {
            return Ok(false);
        }
        Ok(source_hash(tx, &object_id).await? == hash)
    }
}

fn shift_of(place: &CashPlace) -> Option<String> {
    match place {
        CashPlace::Drawer(shift) => Some(shift.clone()),
        _ => None,
    }
}

/// An exchange line's THB: its `thb_amount`, or, for a line with no stored row, the THB the posting
/// moves when it has one foreign leg.
fn exchange_thb(
    row: &Option<LineRow>,
    thb_total: Decimal,
    foreign_legs: usize,
) -> AppResult<Decimal> {
    match row {
        Some(r) => Ok(r.thb_amount),
        None if foreign_legs == 1 => Ok(thb_total),
        None => Err(validation(
            "a line with no stored row is valued only in a posting with one foreign leg",
        )),
    }
}

/// The reversed line's frozen cost `C`, refused when the line is missing, is not of `side`, or was
/// not completed through the ledger (§4.6).
async fn reversed_line(
    tx: &mut Transaction<'_, Postgres>,
    line: TransactionLineId,
    side: &str,
) -> AppResult<Decimal> {
    let row = line_row(tx, line)
        .await?
        .ok_or_else(|| validation("the reversed line does not exist"))?;
    if row.shop_side != side {
        return Err(validation(
            "a reversed BUY leaves the shop and a reversed SELL comes back into it",
        ));
    }
    row.cost_basis_thb
        .ok_or_else(|| validation("the reversed line has no cost_basis_thb"))
}

/// Σ `quantity_signed` of the account `key`, if it exists yet.
async fn place_balance(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: &str,
    key: &AccountKey,
) -> AppResult<Decimal> {
    let balance: Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(e.quantity_signed), 0) FROM ledger_entries e \
         JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.organization_id = $1::uuid AND a.branch_id = $2 AND a.currency = $3 \
           AND a.kind = $4 AND a.shift_id IS NOT DISTINCT FROM $5",
    )
    .bind(organization_id)
    .bind(&key.branch)
    .bind(key.currency.as_str())
    .bind(key.kind.as_str())
    .bind(key.shift.as_deref())
    .fetch_one(&mut **tx)
    .await?;
    Ok(balance)
}

/// Finds or creates the account (design §3.1: `INSERT … ON CONFLICT DO NOTHING RETURNING`, then
/// `SELECT`), as LG-1's `PgLedgerRepository::account_in`.
async fn account_id(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: &str,
    key: &AccountKey,
) -> AppResult<Uuid> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, created_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, now()) \
         ON CONFLICT (organization_id, branch_id, currency, kind, shift_id) DO NOTHING \
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(organization_id)
    .bind(&key.branch)
    .bind(key.currency.as_str())
    .bind(key.kind.as_str())
    .bind(key.shift.as_deref())
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = inserted {
        return Ok(id);
    }
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM cash_accounts \
         WHERE organization_id = $1::uuid AND branch_id IS NOT DISTINCT FROM $2 AND currency = $3 \
           AND kind = $4 AND shift_id IS NOT DISTINCT FROM $5",
    )
    .bind(organization_id)
    .bind(&key.branch)
    .bind(key.currency.as_str())
    .bind(key.kind.as_str())
    .bind(key.shift.as_deref())
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

/// Writes the plan: accounts, the posting and its entries, lots, movements (each moving its
/// position, as `PgInventoryRepository::record_in`), and the frozen cost on exchange lines.
async fn write(
    tx: &mut Transaction<'_, Postgres>,
    plan: &Plan,
    source: &SourceRef,
) -> AppResult<PostingResult> {
    let now = Utc::now();
    let posting_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, 'POSTED', $7, $8)",
    )
    .bind(posting_id)
    .bind(&plan.organization_id)
    .bind(&plan.branch)
    .bind(source.source_type.as_str())
    .bind(&source.source_id)
    .bind(&source.source_key)
    .bind(now)
    .bind(POSTED_BY)
    .execute(&mut **tx)
    .await?;

    let mut accounts: HashMap<AccountKey, Uuid> = HashMap::new();
    for (key, q, valuation) in &plan.entries {
        let account = match accounts.get(key) {
            Some(id) => *id,
            None => {
                let id = account_id(tx, &plan.organization_id, key).await?;
                accounts.insert(key.clone(), id);
                id
            }
        };
        sqlx::query(
            "INSERT INTO ledger_entries (posting_id, account_id, currency, quantity_signed, valuation_thb, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(posting_id)
        .bind(account)
        .bind(key.currency.as_str())
        .bind(*q)
        .bind(*valuation)
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }

    for lot in &plan.lots {
        sqlx::query(
            "INSERT INTO inventory_lots (id, organization_id, branch_id, currency, denomination_id, acquired_at, quantity, unit_cost_thb, source_posting_id) \
             VALUES ($1, $2::uuid, $3, $4, NULL, $5, $6, $7, $8)",
        )
        .bind(lot.id)
        .bind(&plan.organization_id)
        .bind(&lot.branch)
        .bind(lot.currency.as_str())
        .bind(now)
        .bind(lot.quantity)
        .bind(lot.unit_cost_thb)
        .bind(posting_id)
        .execute(&mut **tx)
        .await?;
    }

    for m in &plan.movements {
        sqlx::query(
            "INSERT INTO inventory_movements (organization_id, branch_id, currency, lot_id, posting_id, transaction_line_id, movement_type, quantity_signed, cost_thb_signed, average_after, created_at) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(&plan.organization_id)
        .bind(&m.branch)
        .bind(m.currency.as_str())
        .bind(m.lot_id)
        .bind(posting_id)
        .bind(m.line)
        .bind(m.movement_type.as_str())
        .bind(m.quantity_signed)
        .bind(m.cost_thb_signed)
        .bind(m.average_after)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
             VALUES ($1::uuid, $2, $3, $4, $5, 1) \
             ON CONFLICT (organization_id, branch_id, currency) \
             DO UPDATE SET \
                 quantity = inventory_positions.quantity + EXCLUDED.quantity, \
                 cost_thb = inventory_positions.cost_thb + EXCLUDED.cost_thb, \
                 version = inventory_positions.version + 1",
        )
        .bind(&plan.organization_id)
        .bind(&m.branch)
        .bind(m.currency.as_str())
        .bind(m.quantity_signed)
        .bind(m.cost_thb_signed)
        .execute(&mut **tx)
        .await?;
    }

    for (line, cost, margin) in &plan.line_updates {
        sqlx::query(
            "UPDATE transaction_lines SET cost_basis_thb = $2, realized_margin_thb = $3, updated_at = now() \
             WHERE id = $1",
        )
        .bind(line)
        .bind(cost)
        .bind(margin)
        .execute(&mut **tx)
        .await?;
    }

    Ok(PostingResult {
        posting_id: PostingId(posting_id),
        costs: plan.costs.clone(),
        reversed: plan.reversed.clone(),
    })
}

/// The answer for a source already posted: its id, and the cost frozen on each stored line.
/// `[proposed]`: a reversal's `ReversedCost` is not answered again; its caller recorded it the
/// first time, in the same transaction as the posting.
async fn replayed(
    tx: &mut Transaction<'_, Postgres>,
    posting_id: PostingId,
    req: &[PostingRequest],
) -> AppResult<PostingResult> {
    let mut costs = Vec::new();
    for r in req {
        let Some(line) = r.line else { continue };
        let row = sqlx::query(
            "SELECT cost_basis_thb, realized_margin_thb FROM transaction_lines WHERE id = $1",
        )
        .bind(line)
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(row) = row {
            let cost: Option<Decimal> = row.try_get("cost_basis_thb")?;
            let margin: Option<Decimal> = row.try_get("realized_margin_thb")?;
            if let Some(cost) = cost {
                costs.push(LineCost {
                    line,
                    cost_basis_thb: money(cost)?,
                    realized_margin_thb: margin.map(delta).transpose()?,
                });
            }
        }
    }
    Ok(PostingResult {
        posting_id,
        costs,
        reversed: Vec::new(),
    })
}

/// The source's state for a `NEGATIVE_STOCK` request: an exchange's status and lines, or a
/// transfer's status and lines (design §5.1). Its cost and margin are left out: the posting writes
/// them.
async fn source_hash(tx: &mut Transaction<'_, Postgres>, object_id: &str) -> AppResult<String> {
    let hash: String = sqlx::query_scalar(
        "SELECT md5( \
           coalesce((SELECT 'transaction:' || t.status || '|' || coalesce(string_agg( \
               concat_ws(':', l.id, l.line_no, l.shop_side, l.currency, l.quantity, l.unit_size, \
                         l.rate_snapshot, l.thb_amount, l.fee_thb, l.settle_thb), \
               ',' ORDER BY l.line_no, l.id), '') \
             FROM transactions t LEFT JOIN transaction_lines l ON l.transaction_id = t.id \
             WHERE t.id::text = $1 GROUP BY t.status), '') \
           || '#' || \
           coalesce((SELECT 'transfer:' || ct.status || '|' || coalesce(string_agg( \
               concat_ws(':', cl.currency, cl.denomination_id, cl.qty_sent, cl.qty_received), \
               ',' ORDER BY cl.currency, cl.denomination_id), '') \
             FROM cash_transfers ct LEFT JOIN cash_transfer_lines cl ON cl.transfer_id = ct.id \
             WHERE ct.id::text = $1 GROUP BY ct.status), ''))",
    )
    .bind(object_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(hash)
}

impl NegativeStockSource for LedgerCashPostings {
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move { source_hash(tx, object_id).await })
    }
}

impl CashPostings for LedgerCashPostings {
    fn post_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        req: &'a [PostingRequest],
        allow_negative: Option<ApprovalRequestId>,
    ) -> BoxFuture<'a, AppResult<PostingResult>> {
        Box::pin(async move { self.post(tx, req, allow_negative).await })
    }

    /// Σ of the place's account entries per currency (§4.2, L-4). A balance below zero, or one
    /// with more than 2 decimal places, reads as nothing: `Balances` holds `Money` (lane controller,
    /// issuecomment-6106564990), and §4.5 refuses a drawer or safe below zero.
    fn expected_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>> {
        Box::pin(async move {
            let kind = kind_of(place);
            let rows: Vec<(String, Decimal)> = match place {
                CashPlace::Drawer(shift) => {
                    sqlx::query_as(
                        "SELECT a.currency::text, SUM(e.quantity_signed) FROM ledger_entries e \
                         JOIN cash_accounts a ON a.id = e.account_id \
                         WHERE a.kind = $1 AND a.shift_id = $2 GROUP BY a.currency",
                    )
                    .bind(kind.as_str())
                    .bind(shift)
                    .fetch_all(&mut **tx)
                    .await?
                }
                CashPlace::Safe(_) | CashPlace::InTransit(_) | CashPlace::Variance(_) => {
                    let branch = branch_of(tx, place).await?;
                    sqlx::query_as(
                        "SELECT a.currency::text, SUM(e.quantity_signed) FROM ledger_entries e \
                         JOIN cash_accounts a ON a.id = e.account_id \
                         WHERE a.kind = $1 AND a.branch_id = $2 GROUP BY a.currency",
                    )
                    .bind(kind.as_str())
                    .bind(branch)
                    .fetch_all(&mut **tx)
                    .await?
                }
                CashPlace::Customer | CashPlace::Bank => {
                    sqlx::query_as(
                        "SELECT a.currency::text, SUM(e.quantity_signed) FROM ledger_entries e \
                         JOIN cash_accounts a ON a.id = e.account_id \
                         WHERE a.kind = $1 GROUP BY a.currency",
                    )
                    .bind(kind.as_str())
                    .fetch_all(&mut **tx)
                    .await?
                }
            };
            let mut pairs = Vec::new();
            for (code, sum) in rows {
                let sum = sum.normalize();
                if sum <= Decimal::ZERO || sum.scale() > 2 {
                    continue;
                }
                let Ok(code) = CurrencyCode::parse(code.trim()) else {
                    continue;
                };
                if let Ok(m) = Money::try_from_minor_i128(minor(sum)?) {
                    pairs.push((code, m));
                }
            }
            Ok(Balances::of(&pairs))
        })
    }

    fn activity_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Option<PlaceActivity>>> {
        Box::pin(async move {
            let kind = kind_of(place);
            let rows: Vec<(String, Decimal)> = match place {
                CashPlace::Drawer(shift) => {
                    sqlx::query_as(
                        "SELECT a.currency::text, SUM(e.quantity_signed) FROM ledger_entries e \
                         JOIN cash_accounts a ON a.id = e.account_id \
                         WHERE a.kind = $1 AND a.shift_id = $2 \
                         GROUP BY a.currency ORDER BY a.currency",
                    )
                    .bind(kind.as_str())
                    .bind(shift)
                    .fetch_all(&mut **tx)
                    .await?
                }
                CashPlace::Safe(_) | CashPlace::InTransit(_) | CashPlace::Variance(_) => {
                    let branch = branch_of(tx, place).await?;
                    sqlx::query_as(
                        "SELECT a.currency::text, SUM(e.quantity_signed) FROM ledger_entries e \
                         JOIN cash_accounts a ON a.id = e.account_id \
                         WHERE a.kind = $1 AND a.branch_id = $2 \
                         GROUP BY a.currency ORDER BY a.currency",
                    )
                    .bind(kind.as_str())
                    .bind(branch)
                    .fetch_all(&mut **tx)
                    .await?
                }
                CashPlace::Customer | CashPlace::Bank => {
                    sqlx::query_as(
                        "SELECT a.currency::text, SUM(e.quantity_signed) FROM ledger_entries e \
                         JOIN cash_accounts a ON a.id = e.account_id \
                         WHERE a.kind = $1 \
                         GROUP BY a.currency ORDER BY a.currency",
                    )
                    .bind(kind.as_str())
                    .fetch_all(&mut **tx)
                    .await?
                }
            };
            if rows.is_empty() {
                return Ok(None);
            }
            let mut pairs = Vec::with_capacity(rows.len());
            for (code, sum) in rows {
                let code = CurrencyCode::parse(code.trim()).map_err(|e| {
                    AppError::Internal(format!("invalid currency code in cash_accounts: {e}"))
                })?;
                pairs.push((code, delta(sum)?));
            }
            pairs.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
            Ok(Some(pairs))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_minor_rounds_positive_half_up() {
        let v = Decimal::from_str("0.125").unwrap();
        assert_eq!(minor(v).unwrap(), 13);
    }

    #[test]
    fn test_minor_rounds_negative_half_away_from_zero() {
        let v = Decimal::from_str("-0.125").unwrap();
        assert_eq!(minor(v).unwrap(), -13);
    }

    #[test]
    fn test_minor_rounds_below_half_down() {
        let v = Decimal::from_str("0.124").unwrap();
        assert_eq!(minor(v).unwrap(), 12);
    }

    #[test]
    fn test_minor_whole_satang() {
        let v = Decimal::from_str("1.00").unwrap();
        assert_eq!(minor(v).unwrap(), 100);
    }
}
