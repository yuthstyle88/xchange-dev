//! Rate application service.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, NumberError, Rate};

use crate::domain::rate::commands::save_rate::SaveRate;
use crate::domain::rate::entities::counter_rate::CounterRate;
use crate::domain::rate::entities::rate_change::RateChange;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::domain::validation::{
    ABANDONED, BAD_NUMBER, EDITED_ELSEWHERE, NO_SUCH_BRANCH, PARTNER_PAIR, REQUEST_CHANGED,
    SELL_BELOW_BUY, TOO_MANY_DECIMALS, cut_to_millis, format_iso, validate_request_id,
    validate_text,
};

type RateRow = (
    String,
    Decimal,
    Decimal,
    DateTime<Utc>,
    String,
    String,
    String,
    Option<Decimal>,
    Option<serde_json::Value>,
);

#[derive(Clone)]
pub struct RateService {
    branch_tx: BranchTx,
    source: Arc<dyn RateSource>,
}

type SavedRate = (String, Decimal, Decimal, String, Option<Decimal>);

fn is_same_rate(
    existing: &SavedRate,
    code: CurrencyCode,
    buy: Rate,
    sell: Rate,
    partner_name: &str,
    partner_cost: Option<Rate>,
) -> AppResult<bool> {
    let (ex_code, ex_buy, ex_sell, ex_pname, ex_pcost) = existing;
    if ex_code.trim() != code.as_str() {
        return Ok(false);
    }
    let db_buy = Rate::parse(&ex_buy.normalize().to_string())
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if db_buy != buy {
        return Ok(false);
    }
    let db_sell = Rate::parse(&ex_sell.normalize().to_string())
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if db_sell != sell {
        return Ok(false);
    }
    if ex_pname != partner_name {
        return Ok(false);
    }
    let db_cost = ex_pcost
        .map(|c| Rate::parse(&c.normalize().to_string()))
        .transpose()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if db_cost != partner_cost {
        return Ok(false);
    }
    Ok(true)
}

impl RateService {
    pub fn new(branch_tx: BranchTx, source: Arc<dyn RateSource>) -> Self {
        Self { branch_tx, source }
    }

    pub async fn current(&self, branch: &BranchId) -> AppResult<Vec<CounterRate>> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                .bind(branch.as_str())
                .fetch_one(self.branch_tx.pool())
                .await
                .map_err(AppError::Database)?;

        if !exists {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        let currencies = sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT code, name_th, symbol FROM currencies WHERE code != 'THB'",
        )
        .fetch_all(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        let mut currency_map: HashMap<String, (String, Option<String>)> = currencies
            .into_iter()
            .map(|(code, name_th, symbol)| (code.trim().to_string(), (name_th, symbol)))
            .collect();

        let rates = sqlx::query_as::<_, (
            String,
            Decimal,
            Decimal,
            DateTime<Utc>,
            String,
            String,
            Option<Decimal>,
            Option<serde_json::Value>,
        )>(
            "SELECT DISTINCT ON (code) code, buy, sell, updated_at, request_id, partner_name, partner_cost, reference FROM rates WHERE branch_id = $1 ORDER BY code, id DESC",
        )
        .bind(branch.as_str())
        .fetch_all(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        let mut rate_map = HashMap::new();
        for (code, buy_dec, sell_dec, updated_at, request_id, partner_name, cost_dec, reference) in
            rates
        {
            let buy = Rate::parse(&buy_dec.normalize().to_string())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let sell = Rate::parse(&sell_dec.normalize().to_string())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let partner_cost = cost_dec
                .map(|c| Rate::parse(&c.normalize().to_string()))
                .transpose()
                .map_err(|e| AppError::Internal(e.to_string()))?;

            rate_map.insert(
                code.trim().to_string(),
                (
                    buy,
                    sell,
                    updated_at,
                    request_id,
                    partner_name,
                    partner_cost,
                    reference,
                ),
            );
        }

        let mut result = Vec::with_capacity(152);
        for code in &CurrencyCode::all()[1..] {
            let code_str = code.as_str();
            let (name, symbol) = currency_map
                .remove(code_str)
                .unwrap_or_else(|| (code_str.to_string(), None));

            if let Some((
                buy,
                sell,
                updated_at,
                request_id,
                partner_name,
                partner_cost,
                reference,
            )) = rate_map.remove(code_str)
            {
                result.push(CounterRate {
                    code: *code,
                    name,
                    symbol,
                    buy: Some(buy),
                    sell: Some(sell),
                    updated_at: Some(updated_at),
                    request_id: Some(request_id),
                    partner_name,
                    partner_cost,
                    reference,
                });
            } else {
                result.push(CounterRate {
                    code: *code,
                    name,
                    symbol,
                    buy: None,
                    sell: None,
                    updated_at: None,
                    request_id: None,
                    partner_name: String::new(),
                    partner_cost: None,
                    reference: None,
                });
            }
        }

        Ok(result)
    }

    pub async fn history(&self, branch: &BranchId) -> AppResult<Vec<RateChange>> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                .bind(branch.as_str())
                .fetch_one(self.branch_tx.pool())
                .await
                .map_err(AppError::Database)?;

        if !exists {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        let rows = sqlx::query_as::<_, RateRow>(
            "SELECT code, buy, sell, updated_at, updated_by, request_id, partner_name, partner_cost, reference FROM rates WHERE branch_id = $1 ORDER BY id DESC",
        )
        .bind(branch.as_str())
        .fetch_all(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        let mut changes = Vec::with_capacity(rows.len());
        for (
            r_code,
            buy_dec,
            sell_dec,
            updated_at,
            updated_by,
            request_id,
            partner_name,
            cost_dec,
            reference,
        ) in rows
        {
            let code = CurrencyCode::parse(r_code.trim())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let buy = Rate::parse(&buy_dec.normalize().to_string())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let sell = Rate::parse(&sell_dec.normalize().to_string())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let partner_cost = cost_dec
                .map(|c| Rate::parse(&c.normalize().to_string()))
                .transpose()
                .map_err(|e| AppError::Internal(e.to_string()))?;

            changes.push(RateChange {
                code,
                buy,
                sell,
                updated_at,
                updated_by,
                request_id,
                partner_name,
                partner_cost,
                reference,
            });
        }

        Ok(changes)
    }

    pub async fn save_rate(
        &self,
        branch: &BranchId,
        cmd: SaveRate,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        // Step 1: branch exists
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                .bind(branch.as_str())
                .fetch_one(self.branch_tx.pool())
                .await
                .map_err(AppError::Database)?;

        if !exists {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        // Step 2: request_id is a Request id
        let trimmed_req = validate_request_id(&cmd.request_id)?.to_string();

        // Step 3: code is Text(3, required)
        let trimmed_code = validate_text(&cmd.code, 3, true)?;

        // Step 4: buy is a Number
        let buy_rate = Rate::parse(&cmd.buy).map_err(|e| match e {
            NumberError::TooManyDecimals => AppError::Domain(TOO_MANY_DECIMALS.to_string()),
            _ => AppError::Domain(BAD_NUMBER.to_string()),
        })?;

        // Step 5: sell is a Number
        let sell_rate = Rate::parse(&cmd.sell).map_err(|e| match e {
            NumberError::TooManyDecimals => AppError::Domain(TOO_MANY_DECIMALS.to_string()),
            _ => AppError::Domain(BAD_NUMBER.to_string()),
        })?;

        // Step 6: trimmed code in register (CurrencyCode::parse, exact case), not THB, and sell >= buy
        let parsed_code = match CurrencyCode::parse(trimmed_code) {
            Ok(c) if !c.is_thb() && sell_rate >= buy_rate => c,
            _ => return Err(AppError::Domain(SELL_BELOW_BUY.to_string())),
        };

        // Step 7: request is not Abandoned in this branch
        let abandoned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
        )
        .bind(branch.as_str())
        .bind(&trimmed_req)
        .fetch_one(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        if abandoned {
            return Err(AppError::Domain(ABANDONED.to_string()));
        }

        // Step 8: partner_name is Text(100, not required)
        let trimmed_partner_name = validate_text(&cmd.partner_name, 100, false)?.to_string();

        // Step 9: partner_cost, when Some and not "", is a Number
        let partner_cost_rate = match &cmd.partner_cost {
            Some(cost_str) if !cost_str.is_empty() => {
                let cost = Rate::parse(cost_str).map_err(|e| match e {
                    NumberError::TooManyDecimals => AppError::Domain(TOO_MANY_DECIMALS.to_string()),
                    _ => AppError::Domain(BAD_NUMBER.to_string()),
                })?;
                Some(cost)
            }
            _ => None,
        };

        // Step 10: trimmed partner name is empty exactly when there is no partner cost
        if trimmed_partner_name.is_empty() != partner_cost_rate.is_none() {
            return Err(AppError::Domain(PARTNER_PAIR.to_string()));
        }

        // Step 11 check on committed data before asking the source
        let existing_pre = sqlx::query_as::<_, (
            String,
            Decimal,
            Decimal,
            String,
            Option<Decimal>,
        )>(
            "SELECT code, buy, sell, partner_name, partner_cost FROM rates WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch.as_str())
        .bind(&trimmed_req)
        .fetch_optional(self.branch_tx.pool())
        .await
        .map_err(AppError::Database)?;

        if let Some(existing) = existing_pre {
            if is_same_rate(
                &existing,
                parsed_code,
                buy_rate,
                sell_rate,
                &trimmed_partner_name,
                partner_cost_rate,
            )? {
                return Ok(());
            } else {
                return Err(AppError::Domain(REQUEST_CHANGED.to_string()));
            }
        }

        // Ask the reference-rate source outside the lock
        let ref_rates = self.source.fetch().await?;

        let code_str = parsed_code.as_str();
        let matching_rows: Vec<serde_json::Value> = ref_rates
            .rows
            .into_iter()
            .filter(|r| r.get("code").and_then(|c| c.as_str()) == Some(code_str))
            .collect();

        let reference_json = serde_json::json!({
            "source": ref_rates.source,
            "fetchedAt": ref_rates.fetched_at,
            "kind": ref_rates.kind,
            "rows": matching_rows,
        });

        let branch_id_str = branch.as_str().to_string();
        let actor_str = actor.to_string();
        let stored_now = cut_to_millis(now);
        let previous_updated = cmd.previous_updated;

        let buy_dec = Decimal::from_str(&buy_rate.to_string())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let sell_dec = Decimal::from_str(&sell_rate.to_string())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let partner_cost_dec = partner_cost_rate
            .map(|r| Decimal::from_str(&r.to_string()))
            .transpose()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    // Repeat step 11 inside lock (in case of concurrent identical request)
                    let existing = sqlx::query_as::<_, (
                        String,
                        Decimal,
                        Decimal,
                        String,
                        Option<Decimal>,
                    )>(
                        "SELECT code, buy, sell, partner_name, partner_cost FROM rates WHERE branch_id = $1 AND request_id = $2",
                    )
                    .bind(&branch_id_str)
                    .bind(&trimmed_req)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    if let Some(existing_row) = existing {
                        if is_same_rate(
                            &existing_row,
                            parsed_code,
                            buy_rate,
                            sell_rate,
                            &trimmed_partner_name,
                            partner_cost_rate,
                        )? {
                            return Ok(());
                        } else {
                            return Err(AppError::Domain(REQUEST_CHANGED.to_string()));
                        }
                    }

                    // Proposed: repeat abandoned check inside lock
                    let abandoned: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
                    )
                    .bind(&branch_id_str)
                    .bind(&trimmed_req)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    if abandoned {
                        return Err(AppError::Domain(ABANDONED.to_string()));
                    }

                    // Step 12: previous_updated equals Printed time of newest row of this code in this branch, or "" when none
                    let newest_updated_at: Option<DateTime<Utc>> = sqlx::query_scalar(
                        "SELECT updated_at FROM rates WHERE branch_id = $1 AND code = $2 ORDER BY id DESC LIMIT 1",
                    )
                    .bind(&branch_id_str)
                    .bind(code_str)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    let expected_prev = match newest_updated_at {
                        Some(t) => format_iso(t),
                        None => String::new(),
                    };

                    if previous_updated != expected_prev {
                        return Err(AppError::Domain(EDITED_ELSEWHERE.to_string()));
                    }

                    // Insert
                    sqlx::query(
                        "INSERT INTO rates (branch_id, code, buy, sell, partner_name, partner_cost, reference, updated_at, updated_by, request_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                    )
                    .bind(&branch_id_str)
                    .bind(code_str)
                    .bind(buy_dec)
                    .bind(sell_dec)
                    .bind(&trimmed_partner_name)
                    .bind(partner_cost_dec)
                    .bind(reference_json)
                    .bind(stored_now)
                    .bind(&actor_str)
                    .bind(&trimmed_req)
                    .execute(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    Ok(())
                })
            })
            .await
    }

    pub async fn rate_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        code: CurrencyCode,
    ) -> AppResult<Option<(Rate, Rate)>> {
        let row = sqlx::query_as::<_, (Decimal, Decimal)>(
            "SELECT buy, sell FROM rates WHERE branch_id = $1 AND code = $2 ORDER BY id DESC LIMIT 1",
        )
        .bind(branch.as_str())
        .bind(code.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((buy_dec, sell_dec)) => {
                let buy = Rate::parse(&buy_dec.normalize().to_string())
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                let sell = Rate::parse(&sell_dec.normalize().to_string())
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                Ok(Some((buy, sell)))
            }
            None => Ok(None),
        }
    }
}
