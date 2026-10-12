//! EX-3 #1071: A01, the organisation-wide executive overview (design `executive-finance.md` §3, §4, §8.1), served
//! at `GET /api/executive/overview` (and at `/api/v3/views/executive_overview`, see `org_view.rs`).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::application::executive::channel::Channel;
use crate::application::executive::overview::{BranchHead, ExecutiveOverviewService};
use crate::application::executive::ports::{BrokerNotConnected, ExecutiveFilter};
use crate::bootstrap::AppState;
use crate::infrastructure::executive::alert_facts::PgAlertFacts;
use crate::infrastructure::executive::ledger_cash_facts::LedgerCashFacts;
use crate::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;
use crate::presentation::dto::v3::executive_overview::{
    ExecutiveOverviewData, ExecutiveOverviewFilter, OrgViewAnswer,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::org_view::{
    OrgFilter, OrgView, OrgViewEndpoint, validation_failed,
};

pub const ROUTE: &str = "/api/executive/overview";

pub struct ExecutiveOverviewView {
    pool: PgPool,
    service: ExecutiveOverviewService,
}

impl ExecutiveOverviewView {
    pub fn new(pool: PgPool) -> Self {
        let cash = Arc::new(LedgerCashFacts::new(pool.clone()));
        let service = ExecutiveOverviewService::new(
            Arc::new(WideTransactionFacts::new(pool.clone())),
            cash.clone(),
            Arc::new(PgAlertFacts::new(pool.clone(), cash)),
            Arc::new(BrokerNotConnected),
        );
        Self { pool, service }
    }
}

fn scope_ids(scope: &BranchScope) -> Vec<BranchId> {
    match scope {
        BranchScope::Branches(list) => list.clone(),
        // the seam always hands an organisation view its branches by name
        BranchScope::All => Vec::new(),
    }
}

fn date(query: &BTreeMap<String, String>, key: &'static str) -> Result<Option<NaiveDate>, V3Error> {
    match query.get(key).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => NaiveDate::parse_from_str(v, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| validation_failed(key)),
    }
}

impl OrgFilter for ExecutiveFilter {
    fn parse(query: &BTreeMap<String, String>) -> Result<Self, V3Error> {
        let from = date(query, "from")?;
        let to = date(query, "to")?;
        if let (Some(f), Some(t)) = (from, to)
            && f > t
        {
            return Err(validation_failed("to"));
        }
        let currency = match query.get("currency").filter(|v| !v.is_empty()) {
            None => None,
            Some(c) => Some(CurrencyCode::parse(c).map_err(|_| validation_failed("currency"))?),
        };
        let channel = match query.get("channel").filter(|v| !v.is_empty()) {
            None => None,
            Some(c) => Some(Channel::parse(c).ok_or_else(|| validation_failed("channel"))?),
        };
        Ok(ExecutiveFilter {
            from,
            to,
            currency,
            channel,
        })
    }
}

impl OrgView for ExecutiveOverviewView {
    const NAME: &'static str = "executive_overview";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "executiveOverview");
    type Filter = ExecutiveFilter;
    type Data = ExecutiveOverviewData;

    fn versions_in(
        &self,
        scope: &BranchScope,
    ) -> impl Future<Output = AppResult<BTreeMap<BranchId, i64>>> + Send {
        let pool = self.pool.clone();
        let ids: Vec<String> = scope_ids(scope)
            .iter()
            .map(|b| b.as_str().to_owned())
            .collect();
        async move {
            let rows: Vec<(String, i64)> = sqlx::query_as(
                "SELECT branch_id, version FROM branch_versions WHERE branch_id = ANY($1)",
            )
            .bind(ids)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(rows
                .into_iter()
                .map(|(b, v)| (BranchId::new(b), v))
                .collect())
        }
    }

    fn load(
        &self,
        scope: &BranchScope,
        filter: &Self::Filter,
        changed: &[BranchId],
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let scope = scope_ids(scope);
        let changed = changed.to_vec();
        let filter = filter.clone();
        async move {
            let ids: Vec<String> = changed.iter().map(|b| b.as_str().to_owned()).collect();
            let heads: Vec<(String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
                "SELECT b.id, b.name, v.changed_at FROM branches b \
                   LEFT JOIN branch_versions v ON v.branch_id = b.id \
                  WHERE b.id = ANY($1) ORDER BY b.id",
            )
            .bind(ids)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
            let heads: Vec<BranchHead> = heads
                .into_iter()
                .map(|(id, name, as_of)| BranchHead {
                    id: BranchId::new(id),
                    name,
                    as_of,
                })
                .collect();
            let overview = self
                .service
                .overview(&scope, &heads, &filter, &changed)
                .await?;
            Ok(ExecutiveOverviewData::of(&filter, &overview))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    OrgViewEndpoint::register(
        cfg,
        ROUTE,
        Arc::new(ExecutiveOverviewView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    OrgViewAnswer::<()>::export_all(&cfg)?;
    ExecutiveOverviewFilter::export_all(&cfg)?;
    ExecutiveOverviewData::export_all(&cfg)?;
    Ok(())
}
