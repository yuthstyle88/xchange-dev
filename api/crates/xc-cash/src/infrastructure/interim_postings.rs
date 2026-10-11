//! Interim implementation of CashPostings on current tables.

use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Money};
use xc_shift::domain::shift::value_objects::balances::Balances;

use crate::domain::posting::{CashPlace, CashPostings, PostingRequest};

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
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            for r in req {
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
                        let new_minor = current_minor - r.amount.minor();
                        map.insert(cur_key.to_string(), Value::Number(new_minor.into()));

                        sqlx::query("UPDATE ops_state SET safe = $1 WHERE branch_id = $2")
                            .bind(Value::Object(map))
                            .bind(branch_id.as_str())
                            .execute(&mut **tx)
                            .await?;
                    }
                    CashPlace::Drawer(shift_id) => {
                        let amount_dec = Decimal::new(r.amount.minor(), 2);
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
                        .bind(&r.source)
                        .execute(&mut **tx)
                        .await?;
                    }
                    CashPlace::InTransit(_) | CashPlace::Outside => {}
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
                        let new_minor = current_minor + r.amount.minor();
                        map.insert(cur_key.to_string(), Value::Number(new_minor.into()));

                        sqlx::query("UPDATE ops_state SET safe = $1 WHERE branch_id = $2")
                            .bind(Value::Object(map))
                            .bind(branch_id.as_str())
                            .execute(&mut **tx)
                            .await?;
                    }
                    CashPlace::Drawer(shift_id) => {
                        let amount_dec = Decimal::new(r.amount.minor(), 2);
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
                        .bind(&r.source)
                        .execute(&mut **tx)
                        .await?;
                    }
                    CashPlace::InTransit(_) | CashPlace::Outside => {}
                }
            }
            Ok(())
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
