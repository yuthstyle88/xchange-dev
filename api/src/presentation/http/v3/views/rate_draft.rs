//! Single rate draft view (§4.7).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::rate_drafts::{
    RateDraftDetailData, RateDraftDetailItemDto, RateDraftFilter, RateDraftLineDto,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct RateDraftBranchView {
    pool: PgPool,
}

impl RateDraftBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for RateDraftBranchView {
    const NAME: &'static str = "rate_draft";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    const VERSIONED: bool = false;
    type Data = RateDraftDetailData;
    type Filter = RateDraftFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        let draft_id_str = filter.id.clone();

        async move {
            let Some(id_str) = draft_id_str else {
                return Ok(RateDraftDetailData {
                    draft: None,
                    lines: vec![],
                });
            };

            let draft_uuid = match Uuid::from_str(&id_str) {
                Ok(u) => u,
                Err(_) => {
                    return Ok(RateDraftDetailData {
                        draft: None,
                        lines: vec![],
                    });
                }
            };

            let row = sqlx::query(
                "SELECT id, branch_id, status, version, created_by, created_at, updated_at, evaluation \
                 FROM rate_drafts WHERE branch_id = $1 AND id = $2",
            )
            .bind(&branch_id)
            .bind(draft_uuid)
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let draft = match row {
                Some(r) => {
                    let id: Uuid = r.get(0);
                    let b_id: String = r.get(1);
                    let status: String = r.get(2);
                    let version: i32 = r.get(3);
                    let created_by: String = r.get(4);
                    let created_at: DateTime<Utc> = r.get(5);
                    let updated_at: DateTime<Utc> = r.get(6);
                    let evaluation: Option<Value> = r.get(7);

                    Some(RateDraftDetailItemDto {
                        id: id.to_string(),
                        branch_id: b_id,
                        status,
                        version,
                        created_by,
                        created_at: created_at.to_rfc3339(),
                        updated_at: updated_at.to_rfc3339(),
                        evaluation,
                    })
                }
                None => {
                    return Ok(RateDraftDetailData {
                        draft: None,
                        lines: vec![],
                    });
                }
            };

            let lines_db = sqlx::query(
                "SELECT currency_code, buy_mode, sell_mode, manual_buy, manual_sell, reason \
                 FROM rate_draft_lines WHERE draft_id = $1 ORDER BY id",
            )
            .bind(draft_uuid)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut lines = Vec::new();
            for r in lines_db {
                let curr: String = r.get(0);
                let buy_mode: String = r.get(1);
                let sell_mode: String = r.get(2);
                let manual_buy: Option<Decimal> = r.get(3);
                let manual_sell: Option<Decimal> = r.get(4);
                let reason: Option<String> = r.get(5);

                lines.push(RateDraftLineDto {
                    currency: curr.trim().to_string(),
                    buy_mode,
                    sell_mode,
                    manual_buy: manual_buy.map(|d| format!("{:.8}", d)),
                    manual_sell: manual_sell.map(|d| format!("{:.8}", d)),
                    reason,
                });
            }

            Ok(RateDraftDetailData { draft, lines })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(RateDraftBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateDraftFilter::export_all(&cfg)?;
    RateDraftDetailItemDto::export_all(&cfg)?;
    RateDraftLineDto::export_all(&cfg)?;
    RateDraftDetailData::export_all(&cfg)?;
    Ok(())
}
