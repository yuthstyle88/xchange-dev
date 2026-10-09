//! Published rates view (§4.8).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::published_rates::{
    PublishedLineDto, PublishedPublicationDto, PublishedRatesData,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct PublishedRatesBranchView {
    pool: PgPool,
}

impl PublishedRatesBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for PublishedRatesBranchView {
    const NAME: &'static str = "published_rates";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = true;
    type Data = PublishedRatesData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();

        async move {
            let row = sqlx::query(
                "SELECT b.current_publication_id \
                 FROM branches b WHERE b.id = $1",
            )
            .bind(&branch_id)
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let current_pub_id: Option<Uuid> = match row {
                Some(r) => r.get(0),
                None => None,
            };

            let pub_id = match current_pub_id {
                Some(id) => id,
                None => {
                    return Ok(PublishedRatesData {
                        publication: None,
                        source_date: None,
                        stale: false,
                        lines: vec![],
                    });
                }
            };

            let pub_row = sqlx::query(
                "SELECT id, version, published_at FROM rate_publications WHERE id = $1",
            )
            .bind(pub_id)
            .fetch_one(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let pid: Uuid = pub_row.get(0);
            let pver: i32 = pub_row.get(1);
            let pat: DateTime<Utc> = pub_row.get(2);

            let lines_db = sqlx::query(
                "SELECT id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, source_snapshot \
                 FROM published_rate_lines WHERE publication_id = $1 ORDER BY currency_code ASC",
            )
            .bind(pub_id)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut lines = Vec::new();
            let mut is_stale = false;
            let mut source_date = None;
            let now = Utc::now();

            for r in lines_db {
                let line_id: Uuid = r.get(0);
                let currency: String = r.get(1);
                let group: String = r.get(2);
                let unit_size_dec: Decimal = r.get(3);
                let buy_dec: Option<Decimal> = r.get(4);
                let sell_dec: Option<Decimal> = r.get(5);
                let snapshot: Value = r.get(6);

                let unit_size_str = unit_size_dec.normalize().to_string();

                let buy_rate = buy_dec.map(|d| format!("{:.8}", d));
                let sell_rate = sell_dec.map(|d| format!("{:.8}", d));

                if source_date.is_none()
                    && let Some(sd) = snapshot.get("source_date").and_then(|v| v.as_str())
                {
                    source_date = Some(sd.to_string());
                }

                let is_auto = snapshot.get("buy_mode").and_then(|v| v.as_str()) == Some("AUTO")
                    || snapshot.get("sell_mode").and_then(|v| v.as_str()) == Some("AUTO");

                if is_auto {
                    let max_age_hours = snapshot
                        .get("max_reference_age_hours")
                        .and_then(|v| v.as_i64());
                    let fetched_at = snapshot
                        .get("fetched_at")
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc));

                    if let (Some(age), Some(fat)) = (max_age_hours, fetched_at)
                        && now - fat > Duration::hours(age)
                    {
                        is_stale = true;
                    }
                }

                lines.push(PublishedLineDto {
                    currency: currency.trim().to_string(),
                    group,
                    unit_size: unit_size_str,
                    buy_rate,
                    sell_rate,
                    line_id: line_id.to_string(),
                });
            }

            Ok(PublishedRatesData {
                publication: Some(PublishedPublicationDto {
                    id: pid.to_string(),
                    version: pver,
                    published_at: pat.to_rfc3339(),
                }),
                source_date,
                stale: is_stale,
                lines,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(PublishedRatesBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PublishedPublicationDto::export_all(&cfg)?;
    PublishedLineDto::export_all(&cfg)?;
    PublishedRatesData::export_all(&cfg)?;
    Ok(())
}
