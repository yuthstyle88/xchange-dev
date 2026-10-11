//! Rate application service: the counter rates a branch shows, from its current publication.
//!
//! RE-7 #604 (rates.md §8 migration 6, §9) retired the direct save (`adminSaveRate`,
//! `save_rate`), the `rates` table and its history: a counter rate is set only by publishing.

use std::collections::HashMap;
use std::sync::Arc;

use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::branch::repositories::branch_repository::BranchRepository;
use crate::domain::rate::entities::counter_rate::CounterRate;
use crate::domain::rate::entities::rate_change::RateChange;
use crate::domain::rate::entities::rate_source::RateSource;
use crate::domain::rate::repositories::rate_repository::{CurrentRateRow, RateRepository};
use crate::domain::validation::NO_SUCH_BRANCH;
use crate::infrastructure::branch::branch_repository::PgBranchRepository;
use crate::infrastructure::rate::rate_repository::PgRateRepository;

#[derive(Clone)]
pub struct RateService {
    rate_repo: PgRateRepository,
    branch_repo: PgBranchRepository,
}

impl RateService {
    /// `_source` was the reference-rate source of the direct save; nothing asks it here since
    /// RE-7, and it stays in the signature for the callers that build a service with it.
    pub fn new(branch_tx: BranchTx, _source: Arc<dyn RateSource>) -> Self {
        let rate_repo = PgRateRepository::new(branch_tx.pool().clone());
        let branch_repo = PgBranchRepository::new(branch_tx.pool().clone());
        Self {
            rate_repo,
            branch_repo,
        }
    }

    /// Every currency of the register but THB, in register order, with the branch's current
    /// publication per 1 unit where it publishes the currency: `updated_at` is the
    /// publication's `published_at`, the partner is `source_snapshot.partner`'s `name` / `cost`.
    /// No request id and no BoT reference: only a direct save carried those.
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
        let mut rate_map: HashMap<String, CurrentRateRow> = rates
            .into_iter()
            .map(|row| (row.code.clone(), row))
            .collect();

        let mut result = Vec::with_capacity(152);
        for code in &CurrencyCode::all()[1..] {
            let code_str = code.as_str();
            let (name, symbol) = currency_map
                .remove(code_str)
                .unwrap_or_else(|| (code_str.to_string(), None));

            result.push(match rate_map.remove(code_str) {
                Some(row) => CounterRate {
                    code: *code,
                    name,
                    symbol,
                    buy: row.buy,
                    sell: row.sell,
                    updated_at: Some(row.updated_at),
                    request_id: None,
                    partner_name: row.partner_name,
                    partner_cost: row.partner_cost,
                    reference: None,
                },
                None => CounterRate {
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
                },
            });
        }

        Ok(result)
    }

    /// The branch's direct saves, newest first: none since RE-7 (rates.md R10: `rateHistory`
    /// is `[]`; a rate changes only by publication, whose history is the publications). A
    /// branch that does not exist is refused, as `current` refuses it.
    pub async fn history(&self, branch: &BranchId) -> AppResult<Vec<RateChange>> {
        if !self.branch_repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }
        Ok(Vec::new())
    }
}
