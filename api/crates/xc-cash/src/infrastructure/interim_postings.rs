//! Interim implementation of CashPostings on current tables.
//!
//! An `Exchange` source (TX-5 #873, ruling Q9) moves the drawer as today's save does
//! (`ShiftService::apply_delta_in`): below zero is refused with `stockInsufficient`, `shift_balances.expected`
//! moves, and no `stock_movements` row is written. A cash exchange's two drawer legs share one source, and
//! `stock_movements` is `UNIQUE (branch_id, request_id)`. Every other source keeps the interim behaviour below.
//! LG-4's cutover removes this file.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Money};
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;

use crate::domain::ledger::posting::{PostingId, SourceType};
use crate::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, PostingRequest, PostingResult,
};

#[derive(Clone, Default)]
pub struct InterimCashPostings;

impl InterimCashPostings {
    pub fn new() -> Self {
        Self
    }
}

impl CashPostings for InterimCashPostings {
    fn post_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        req: &'a [PostingRequest],
        _allow_negative: Option<ApprovalRequestId>,
    ) -> BoxFuture<'a, AppResult<PostingResult>> {
        Box::pin(async move {
            if req
                .iter()
                .any(|r| r.source.source_type == SourceType::Exchange)
            {
                return post_exchange_in(tx, req).await;
            }
            for r in req {
                let amount_minor = interim_minor(r);
                // Source: where money leaves
                match &r.from {
                    CashPlace::Safe(branch_id) => {
                        let row: Option<(Option<Value>,)> = sqlx::query_as(
                            "SELECT safe FROM ops_state WHERE branch_id = $1 FOR UPDATE",
                        )
                        .bind(branch_id.as_str())
                        .fetch_optional(&mut **tx)
                        .await?;

                        let mut map = match row {
                            Some((Some(Value::Object(m)),)) => m,
                            _ => serde_json::Map::new(),
                        };

                        let cur_key = r.currency.as_str();
                        let current_minor = map.get(cur_key).and_then(|v| v.as_i64()).unwrap_or(0);
                        let new_minor = current_minor - amount_minor;
                        map.insert(cur_key.to_string(), Value::Number(new_minor.into()));

                        sqlx::query("UPDATE ops_state SET safe = $1 WHERE branch_id = $2")
                            .bind(Value::Object(map))
                            .bind(branch_id.as_str())
                            .execute(&mut **tx)
                            .await?;
                    }
                    CashPlace::Drawer(shift_id) => {
                        let amount_dec = Decimal::new(amount_minor, 2);
                        sqlx::query(
                            "UPDATE shift_balances SET expected = expected - $1 WHERE shift_id = $2 AND code = $3",
                        )
                        .bind(amount_dec)
                        .bind(shift_id)
                        .bind(r.currency.as_str())
                        .execute(&mut **tx)
                        .await?;

                        let branch_id: String =
                            sqlx::query_scalar("SELECT branch_id FROM shifts WHERE id = $1")
                                .bind(shift_id)
                                .fetch_one(&mut **tx)
                                .await?;

                        let sm_id = format!("sm_{}", Uuid::new_v4());
                        sqlx::query(
                            "INSERT INTO stock_movements (id, branch_id, shift_id, code, direction, amount, reason, request_id, at, by) \
                             VALUES ($1, $2, $3, $4, 'OUT', $5, 'transfer', $6, now(), 'system')",
                        )
                        .bind(sm_id)
                        .bind(branch_id)
                        .bind(shift_id)
                        .bind(r.currency.as_str())
                        .bind(amount_dec)
                        .bind(&r.source.source_id)
                        .execute(&mut **tx)
                        .await?;
                    }
                    CashPlace::InTransit(_) | CashPlace::Customer | CashPlace::Bank => {}
                }

                // Destination: where money enters
                match &r.to {
                    CashPlace::Safe(branch_id) => {
                        let row: Option<(Option<Value>,)> = sqlx::query_as(
                            "SELECT safe FROM ops_state WHERE branch_id = $1 FOR UPDATE",
                        )
                        .bind(branch_id.as_str())
                        .fetch_optional(&mut **tx)
                        .await?;

                        let mut map = match row {
                            Some((Some(Value::Object(m)),)) => m,
                            _ => serde_json::Map::new(),
                        };

                        let cur_key = r.currency.as_str();
                        let current_minor = map.get(cur_key).and_then(|v| v.as_i64()).unwrap_or(0);
                        let new_minor = current_minor + amount_minor;
                        map.insert(cur_key.to_string(), Value::Number(new_minor.into()));

                        sqlx::query("UPDATE ops_state SET safe = $1 WHERE branch_id = $2")
                            .bind(Value::Object(map))
                            .bind(branch_id.as_str())
                            .execute(&mut **tx)
                            .await?;
                    }
                    CashPlace::Drawer(shift_id) => {
                        let amount_dec = Decimal::new(amount_minor, 2);
                        sqlx::query(
                            "INSERT INTO shift_balances (shift_id, code, opening, expected) \
                             VALUES ($1, $2, 0, $3) \
                             ON CONFLICT (shift_id, code) DO UPDATE SET expected = shift_balances.expected + $3",
                        )
                        .bind(shift_id)
                        .bind(r.currency.as_str())
                        .bind(amount_dec)
                        .execute(&mut **tx)
                        .await?;

                        let branch_id: String =
                            sqlx::query_scalar("SELECT branch_id FROM shifts WHERE id = $1")
                                .bind(shift_id)
                                .fetch_one(&mut **tx)
                                .await?;

                        let sm_id = format!("sm_{}", Uuid::new_v4());
                        sqlx::query(
                            "INSERT INTO stock_movements (id, branch_id, shift_id, code, direction, amount, reason, request_id, at, by) \
                             VALUES ($1, $2, $3, $4, 'IN', $5, 'transfer', $6, now(), 'system')",
                        )
                        .bind(sm_id)
                        .bind(branch_id)
                        .bind(shift_id)
                        .bind(r.currency.as_str())
                        .bind(amount_dec)
                        .bind(&r.source.source_id)
                        .execute(&mut **tx)
                        .await?;
                    }
                    CashPlace::InTransit(_) | CashPlace::Customer | CashPlace::Bank => {}
                }
            }
            Ok(PostingResult {
                posting_id: PostingId(Uuid::new_v4()),
                costs: Vec::new(),
                reversed: Vec::new(),
            })
        })
    }

    fn expected_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>> {
        Box::pin(async move {
            match place {
                CashPlace::Drawer(shift_id) => {
                    let rows: Vec<(String, String)> = sqlx::query_as(
                        "SELECT code, expected::text FROM shift_balances WHERE shift_id = $1",
                    )
                    .bind(shift_id)
                    .fetch_all(&mut **tx)
                    .await?;

                    let mut pairs = Vec::new();
                    for (code_str, exp_str) in rows {
                        if let Ok(code) = CurrencyCode::parse(code_str.trim()) {
                            if let Ok(money) = Money::parse(&exp_str) {
                                if !money.is_zero() {
                                    pairs.push((code, money));
                                }
                            }
                        }
                    }
                    Ok(Balances::of(&pairs))
                }
                CashPlace::Safe(branch_id) => {
                    let row: Option<(Option<Value>,)> =
                        sqlx::query_as("SELECT safe FROM ops_state WHERE branch_id = $1")
                            .bind(branch_id.as_str())
                            .fetch_optional(&mut **tx)
                            .await?;

                    let mut pairs = Vec::new();
                    if let Some((Some(Value::Object(map)),)) = row {
                        for (cur_str, val) in map {
                            if let Ok(code) = CurrencyCode::parse(cur_str.trim()) {
                                if let Some(minor) = val.as_i64() {
                                    if let Ok(money) = Money::from_minor(minor) {
                                        if !money.is_zero() {
                                            pairs.push((code, money));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Balances::of(&pairs))
                }
                _ => Ok(Balances::of(&[])),
            }
        })
    }
}

/// An exchange's legs as today's save moves the drawer: the net change per drawer and currency, refused below
/// zero before anything is written, then added to `shift_balances.expected`.
async fn post_exchange_in(
    tx: &mut Transaction<'_, Postgres>,
    req: &[PostingRequest],
) -> AppResult<PostingResult> {
    let mut deltas: Vec<(String, CurrencyCode, i64)> = Vec::new();
    let mut add = |shift: &str, currency: CurrencyCode, minor: i64| match deltas
        .iter_mut()
        .find(|(s, c, _)| s == shift && *c == currency)
    {
        Some((_, _, d)) => *d += minor,
        None => deltas.push((shift.to_owned(), currency, minor)),
    };
    for r in req {
        let minor = interim_minor(r);
        if let CashPlace::Drawer(shift) = &r.from {
            add(shift, r.currency, -minor);
        }
        if let CashPlace::Drawer(shift) = &r.to {
            add(shift, r.currency, minor);
        }
    }
    let mut held = Vec::with_capacity(deltas.len());
    for (shift, currency, delta) in &deltas {
        let expected: Option<Decimal> = sqlx::query_scalar(
            "SELECT expected FROM shift_balances WHERE shift_id = $1 AND code = $2 FOR UPDATE",
        )
        .bind(shift)
        .bind(currency.as_str())
        .fetch_optional(&mut **tx)
        .await?;
        if expected.unwrap_or_default() + Decimal::new(*delta, 2) < Decimal::ZERO {
            return Err(AppError::Domain(format!(
                "stockInsufficient: {}",
                ShiftError::Insufficient(*currency)
            )));
        }
        held.push(expected.is_some());
    }
    // An UPDATE where the balance exists: `shift_balances.expected >= 0` is checked on an INSERT's proposed row
    // before ON CONFLICT, so a withdrawal cannot go through an upsert.
    for ((shift, currency, delta), held) in deltas.iter().zip(held) {
        let sql = if held {
            "UPDATE shift_balances SET expected = expected + $3 WHERE shift_id = $1 AND code = $2"
        } else {
            "INSERT INTO shift_balances (shift_id, code, opening, expected) VALUES ($1, $2, 0, $3)"
        };
        sqlx::query(sql)
            .bind(shift)
            .bind(currency.as_str())
            .bind(Decimal::new(*delta, 2))
            .execute(&mut **tx)
            .await?;
    }
    Ok(PostingResult {
        posting_id: PostingId(Uuid::new_v4()),
        costs: Vec::new(),
        reversed: Vec::new(),
    })
}

/// The interim tables hold hundredths: the request's quantity in hundredths, rounded half away from
/// zero, as the old `Money` amount was.
fn interim_minor(r: &PostingRequest) -> i64 {
    (r.quantity.value() * Decimal::from(100))
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .unwrap_or(i64::MAX)
}
