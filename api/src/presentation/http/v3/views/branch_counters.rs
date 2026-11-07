//! Ticket CA-1d (`docs/design/xc-counter-app.md` §4, §5.2, §5.4, §8.4): `GET /api/v3/views/branch_counters?branch=`.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::SecondsFormat;
use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::settings::cash_from_browser;
use crate::bootstrap::AppState;
use crate::infrastructure::device_agents::agent_repository::DeviceAgent;
use crate::infrastructure::device_agents::counter_store::CounterStore;
use crate::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_counters.ts")]
pub struct BranchCounterRow {
    pub agent_id: String,
    pub name: String,
    pub paired_at: Option<String>,
    pub last_seen_at: Option<String>,
    pub cash_drawer: Option<bool>,
    pub scanner: Option<bool>,
    pub kiosk_lock: Option<bool>,
}

impl From<DeviceAgent> for BranchCounterRow {
    fn from(a: DeviceAgent) -> Self {
        Self {
            agent_id: a.id,
            name: a.counter_name,
            paired_at: a
                .paired_at
                .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
            last_seen_at: a
                .last_seen_at
                .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
            cash_drawer: a.cash_drawer,
            scanner: a.scanner,
            kiosk_lock: a.kiosk_lock,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_counters.ts")]
pub struct BranchCountersData {
    pub counters: Vec<BranchCounterRow>,
    pub cash_from_browser: Option<bool>,
    #[ts(type = "number")]
    pub cash_from_browser_version: i64,
}

pub struct BranchCountersView {
    pool: PgPool,
    agents: PgAgentRepository,
}

impl BranchCountersView {
    pub fn new(pool: PgPool) -> Self {
        let agents = PgAgentRepository::new(pool.clone());
        Self { pool, agents }
    }
}

impl BranchView for BranchCountersView {
    const NAME: &'static str = "branch_counters";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "branchCounters");
    const VERSIONED: bool = true;
    type Data = BranchCountersData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let agents = self.agents.clone();
        let branch = branch.clone();
        async move {
            let counters = agents
                .counters(&branch)
                .await?
                .into_iter()
                .map(BranchCounterRow::from)
                .collect();
            let mut tx = pool.begin().await.map_err(AppError::Database)?;
            let (cash_from_browser, cash_from_browser_version) =
                cash_from_browser::read_in(&mut tx, &branch).await?;
            Ok(BranchCountersData {
                counters,
                cash_from_browser,
                cash_from_browser_version,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(BranchCountersView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchCounterRow::export_all(&cfg)?;
    BranchCountersData::export_all(&cfg)?;
    Ok(())
}
