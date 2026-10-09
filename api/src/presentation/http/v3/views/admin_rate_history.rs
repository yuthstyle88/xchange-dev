//! The `admin_rate_history` view (§7, §12.14).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_rate_history::{
    AdminRateHistoryRow, BotReference, BotReferenceRow,
};
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

fn format_decimal_4(dec: Decimal) -> String {
    let rounded =
        dec.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    format!("{rounded:.4}")
}

fn parse_and_format_rate(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Number(n) => {
            if let Ok(d) = Decimal::from_str(&n.to_string()) {
                format_decimal_4(d)
            } else {
                String::new()
            }
        }
        serde_json::Value::String(s) => {
            if let Ok(d) = Decimal::from_str(s) {
                format_decimal_4(d)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

fn parse_bot_reference(ref_val: Option<serde_json::Value>) -> Option<BotReference> {
    let val = ref_val?;
    let fetched_at = val.get("fetchedAt").and_then(|v| v.as_str())?.to_string();
    let rows_arr = val.get("rows").and_then(|v| v.as_array())?;
    let rows = rows_arr
        .iter()
        .map(|r| {
            let code = r
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let date = r
                .get("date")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let unit = r.get("unit").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            let buying_sight = r
                .get("buyingSight")
                .map(parse_and_format_rate)
                .unwrap_or_default();
            let buying_transfer = r
                .get("buyingTransfer")
                .map(parse_and_format_rate)
                .unwrap_or_default();
            let selling = r
                .get("selling")
                .map(parse_and_format_rate)
                .unwrap_or_default();
            BotReferenceRow {
                code,
                date,
                unit,
                buying_sight,
                buying_transfer,
                selling,
            }
        })
        .collect();
    Some(BotReference { fetched_at, rows })
}

type HistoryDbRow = (
    i64,
    String,
    Decimal,
    Decimal,
    DateTime<Utc>,
    String,
    String,
    Option<Decimal>,
    Option<serde_json::Value>,
);

pub struct AdminRateHistoryPagedView {
    pool: PgPool,
}

impl AdminRateHistoryPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for AdminRateHistoryPagedView {
    const NAME: &'static str = "admin_rate_history";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = AdminRateHistoryRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        let before_id = before.and_then(|b| b.parse::<i64>().ok());
        let limit = limit as i64;

        async move {
            let rows = if let Some(bid) = before_id {
                sqlx::query_as::<_, HistoryDbRow>(
                    "SELECT id, code, buy, sell, updated_at, updated_by, partner_name, partner_cost, reference \
                     FROM rates \
                     WHERE branch_id = $1 AND id < $2 \
                     ORDER BY id DESC \
                     LIMIT $3",
                )
                .bind(&branch_id)
                .bind(bid)
                .bind(limit)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            } else {
                sqlx::query_as::<_, HistoryDbRow>(
                    "SELECT id, code, buy, sell, updated_at, updated_by, partner_name, partner_cost, reference \
                     FROM rates \
                     WHERE branch_id = $1 \
                     ORDER BY id DESC \
                     LIMIT $2",
                )
                .bind(&branch_id)
                .bind(limit)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            };

            let result = rows
                .into_iter()
                .map(
                    |(
                        id,
                        code,
                        buy,
                        sell,
                        updated_at,
                        updated_by,
                        partner_name,
                        partner_cost,
                        reference,
                    )| {
                        AdminRateHistoryRow {
                            id: id.to_string(),
                            code: code.trim().to_string(),
                            buy: format_decimal_4(buy),
                            sell: format_decimal_4(sell),
                            updated: updated_at
                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                            actor: updated_by,
                            partner_name,
                            partner_cost: partner_cost.map(format_decimal_4),
                            bot_reference: parse_bot_reference(reference),
                        }
                    },
                )
                .collect();

            Ok(result)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    ViewEndpoint::register_paged(cfg, Arc::new(AdminRateHistoryPagedView::new(pool)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BotReferenceRow::export_all(&cfg)?;
    BotReference::export_all(&cfg)?;
    AdminRateHistoryRow::export_all(&cfg)?;
    Ok(())
}
