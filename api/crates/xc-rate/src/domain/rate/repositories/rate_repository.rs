//! Rate repository trait.

use std::future::Future;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};

use crate::domain::rate::entities::rate_change::RateChange;

#[derive(Debug, Clone, PartialEq)]
pub struct CurrentRateRow {
    pub code: String,
    pub buy: Rate,
    pub sell: Rate,
    pub updated_at: DateTime<Utc>,
    pub request_id: String,
    pub partner_name: String,
    pub partner_cost: Option<Rate>,
    pub reference: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedRateRecord {
    pub code: String,
    pub buy: Rate,
    pub sell: Rate,
    pub partner_name: String,
    pub partner_cost: Option<Rate>,
}

#[derive(Debug, Clone)]
pub struct NewRate<'a> {
    pub branch_id: &'a str,
    pub code: &'a str,
    pub buy: Rate,
    pub sell: Rate,
    pub partner_name: &'a str,
    pub partner_cost: Option<Rate>,
    pub reference: serde_json::Value,
    pub updated_at: DateTime<Utc>,
    pub updated_by: &'a str,
    pub request_id: &'a str,
}

pub trait RateRepository: Send + Sync {
    fn list_currencies(
        &self,
    ) -> impl Future<Output = AppResult<Vec<(String, String, Option<String>)>>> + Send;

    fn current_rates(
        &self,
        branch: &BranchId,
    ) -> impl Future<Output = AppResult<Vec<CurrentRateRow>>> + Send;

    fn history(&self, branch: &BranchId)
    -> impl Future<Output = AppResult<Vec<RateChange>>> + Send;

    fn is_abandoned(
        &self,
        branch_id: &str,
        request_id: &str,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn is_abandoned_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        request_id: &str,
    ) -> impl Future<Output = AppResult<bool>> + Send;

    fn find_by_request(
        &self,
        branch_id: &str,
        request_id: &str,
    ) -> impl Future<Output = AppResult<Option<SavedRateRecord>>> + Send;

    fn find_by_request_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        request_id: &str,
    ) -> impl Future<Output = AppResult<Option<SavedRateRecord>>> + Send;

    fn newest_updated_at(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
        code: &str,
    ) -> impl Future<Output = AppResult<Option<DateTime<Utc>>>> + Send;

    fn insert(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rate: NewRate<'_>,
    ) -> impl Future<Output = AppResult<()>> + Send;

    fn rate_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        code: CurrencyCode,
    ) -> impl Future<Output = AppResult<Option<(Rate, Rate)>>> + Send;
}
