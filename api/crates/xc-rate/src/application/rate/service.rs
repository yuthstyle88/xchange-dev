//! Rate application service.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};

use crate::domain::branch::repositories::branch_repository::BranchRepository;
use crate::domain::rate::commands::save_rate::SaveRate;
use crate::domain::rate::entities::counter_rate::CounterRate;
use crate::domain::rate::entities::rate_change::RateChange;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::domain::rate::repositories::rate_repository::{
    CurrentRateRow, NewRate, RateRepository, SavedRateRecord,
};
use crate::domain::validation::{
    ABANDONED, EDITED_ELSEWHERE, NO_SUCH_BRANCH, PARTNER_PAIR, REQUEST_CHANGED, SELL_BELOW_BUY,
    cut_to_millis, format_iso, parse_rate, validate_request_id, validate_text,
};
use crate::infrastructure::branch::branch_repository::PgBranchRepository;
use crate::infrastructure::rate::rate_repository::PgRateRepository;

#[derive(Clone)]
pub struct RateService {
    branch_tx: BranchTx,
    source: Arc<dyn RateSource>,
    rate_repo: PgRateRepository,
    branch_repo: PgBranchRepository,
}

fn is_same_rate(
    existing: &SavedRateRecord,
    code: CurrencyCode,
    buy: Rate,
    sell: Rate,
    partner_name: &str,
    partner_cost: Option<Rate>,
) -> bool {
    existing.code.trim() == code.as_str()
        && existing.buy == buy
        && existing.sell == sell
        && existing.partner_name == partner_name
        && existing.partner_cost == partner_cost
}

impl RateService {
    pub fn new(branch_tx: BranchTx, source: Arc<dyn RateSource>) -> Self {
        let rate_repo = PgRateRepository::new(branch_tx.pool().clone());
        let branch_repo = PgBranchRepository::new(branch_tx.pool().clone());
        Self {
            branch_tx,
            source,
            rate_repo,
            branch_repo,
        }
    }

    pub async fn current(&self, branch: &BranchId) -> AppResult<Vec<CounterRate>> {
        if !self.branch_repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        let currencies = self.rate_repo.list_currencies().await?;
        let mut currency_map: HashMap<String, (String, Option<String>)> = currencies
            .into_iter()
            .map(|(code, name_th, symbol)| (code.trim().to_string(), (name_th, symbol)))
            .collect();

        let rates = self.rate_repo.current_rates(branch).await?;
        let mut rate_map: HashMap<String, CurrentRateRow> = HashMap::with_capacity(rates.len());
        for row in rates {
            rate_map.insert(row.code.trim().to_string(), row);
        }

        let mut result = Vec::with_capacity(152);
        for code in &CurrencyCode::all()[1..] {
            let code_str = code.as_str();
            let (name, symbol) = currency_map
                .remove(code_str)
                .unwrap_or_else(|| (code_str.to_string(), None));

            if let Some(row) = rate_map.remove(code_str) {
                result.push(CounterRate {
                    code: *code,
                    name,
                    symbol,
                    buy: Some(row.buy),
                    sell: Some(row.sell),
                    updated_at: Some(row.updated_at),
                    request_id: Some(row.request_id),
                    partner_name: row.partner_name,
                    partner_cost: row.partner_cost,
                    reference: row.reference,
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
        if !self.branch_repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        self.rate_repo.history(branch).await
    }

    pub async fn save_rate(
        &self,
        branch: &BranchId,
        cmd: SaveRate,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        // Step 1: branch exists
        if !self.branch_repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }

        // Step 2: request_id is a Request id
        let trimmed_req = validate_request_id(&cmd.request_id)?.to_string();

        // Step 3: code is Text(3, required)
        let trimmed_code = validate_text(&cmd.code, 3, true)?;

        // Step 4: buy is a Number
        let buy_rate = parse_rate(&cmd.buy)?;

        // Step 5: sell is a Number
        let sell_rate = parse_rate(&cmd.sell)?;

        // Step 6: trimmed code in register (CurrencyCode::parse, exact case), not THB, and sell >= buy
        let parsed_code = match CurrencyCode::parse(trimmed_code) {
            Ok(c) if !c.is_thb() && sell_rate >= buy_rate => c,
            _ => return Err(AppError::Domain(SELL_BELOW_BUY.to_string())),
        };

        // Step 7: request is not Abandoned in this branch
        if self
            .rate_repo
            .is_abandoned(branch.as_str(), &trimmed_req)
            .await?
        {
            return Err(AppError::Domain(ABANDONED.to_string()));
        }

        // Step 8: partner_name is Text(100, not required)
        let trimmed_partner_name = validate_text(&cmd.partner_name, 100, false)?.to_string();

        // Step 9: partner_cost, when Some and not "", is a Number
        let partner_cost_rate = match &cmd.partner_cost {
            Some(cost_str) if !cost_str.is_empty() => Some(parse_rate(cost_str)?),
            _ => None,
        };

        // Step 10: trimmed partner name is empty exactly when there is no partner cost
        if trimmed_partner_name.is_empty() != partner_cost_rate.is_none() {
            return Err(AppError::Domain(PARTNER_PAIR.to_string()));
        }

        // Step 11 check on committed data before asking the source
        let existing_pre = self
            .rate_repo
            .find_by_request(branch.as_str(), &trimmed_req)
            .await?;

        if let Some(existing) = existing_pre {
            if is_same_rate(
                &existing,
                parsed_code,
                buy_rate,
                sell_rate,
                &trimmed_partner_name,
                partner_cost_rate,
            ) {
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
        let code_string = parsed_code.as_str().to_string();
        let rate_repo = self.rate_repo.clone();

        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    // Repeat step 11 inside lock (in case of concurrent identical request)
                    let existing = rate_repo
                        .find_by_request_tx(tx, &branch_id_str, &trimmed_req)
                        .await?;

                    if let Some(existing_row) = existing {
                        if is_same_rate(
                            &existing_row,
                            parsed_code,
                            buy_rate,
                            sell_rate,
                            &trimmed_partner_name,
                            partner_cost_rate,
                        ) {
                            return Ok(());
                        } else {
                            return Err(AppError::Domain(REQUEST_CHANGED.to_string()));
                        }
                    }

                    // Proposed: repeat abandoned check inside lock
                    if rate_repo
                        .is_abandoned_tx(tx, &branch_id_str, &trimmed_req)
                        .await?
                    {
                        return Err(AppError::Domain(ABANDONED.to_string()));
                    }

                    // Step 12: previous_updated equals Printed time of newest row of this code in this branch, or "" when none
                    let newest_updated_at = rate_repo
                        .newest_updated_at(tx, &branch_id_str, &code_string)
                        .await?;

                    let expected_prev = match newest_updated_at {
                        Some(t) => format_iso(t),
                        None => String::new(),
                    };

                    if previous_updated != expected_prev {
                        return Err(AppError::Domain(EDITED_ELSEWHERE.to_string()));
                    }

                    // Insert
                    rate_repo
                        .insert(
                            tx,
                            NewRate {
                                branch_id: &branch_id_str,
                                code: &code_string,
                                buy: buy_rate,
                                sell: sell_rate,
                                partner_name: &trimmed_partner_name,
                                partner_cost: partner_cost_rate,
                                reference: reference_json,
                                updated_at: stored_now,
                                updated_by: &actor_str,
                                request_id: &trimmed_req,
                            },
                        )
                        .await?;

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
        PgRateRepository::find_rate_in(tx, branch, code).await
    }
}
