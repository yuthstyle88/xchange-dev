//! Rate publications view (§4.7).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rate_publications::{
    RatePublicationLine, RatePublicationRow, RateSides,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

pub struct RatePublicationsPagedView {
    pool: PgPool,
}

impl RatePublicationsPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type LineKey = (String, String, String);

struct RawLine {
    unit_size: String,
    sides: RateSides,
}

impl PagedView for RatePublicationsPagedView {
    const NAME: &'static str = "rate_publications";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Row = RatePublicationRow;
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

        async move {
            let cur_row = sqlx::query("SELECT current_publication_id FROM branches WHERE id = $1")
                .bind(&branch_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

            let current_pub_id: Option<Uuid> = cur_row.and_then(|r| r.get(0));

            let rows = sqlx::query(
                "SELECT p.id, p.branch_id, p.version, p.published_at, p.published_by, \
                        p.approved_request_id, p.draft_id, \
                        (SELECT prev.id FROM rate_publications prev \
                         WHERE prev.branch_id = p.branch_id AND prev.version < p.version \
                         ORDER BY prev.version DESC LIMIT 1) AS prev_id \
                 FROM rate_publications p \
                 WHERE p.branch_id = $1 \
                   AND ($2::text IS NULL OR p.version < ( \
                       SELECT b.version FROM rate_publications b \
                       WHERE b.id::text = $2 AND b.branch_id = $1 \
                   )) \
                 ORDER BY p.version DESC \
                 LIMIT $3",
            )
            .bind(&branch_id)
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut pub_ids = Vec::with_capacity(rows.len() * 2);
            for r in &rows {
                pub_ids.push(r.get::<Uuid, _>(0));
                if let Some(prev_id) = r.get::<Option<Uuid>, _>(7) {
                    pub_ids.push(prev_id);
                }
            }
            pub_ids.sort_unstable();
            pub_ids.dedup();

            let mut by_pub: HashMap<Uuid, BTreeMap<LineKey, RawLine>> = HashMap::new();
            if !pub_ids.is_empty() {
                let line_rows = sqlx::query(
                    "SELECT publication_id, currency_code, denomination_group, value_tier, \
                            unit_size, buy_rate, sell_rate \
                     FROM published_rate_lines \
                     WHERE publication_id = ANY($1) \
                     ORDER BY currency_code ASC, denomination_group ASC, value_tier ASC",
                )
                .bind(&pub_ids)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;

                for lr in line_rows {
                    let pub_id: Uuid = lr.get(0);
                    let currency: String = lr.get::<String, _>(1).trim().to_string();
                    let group: String = lr.get(2);
                    let value_tier: String = lr.get(3);
                    let unit_size: Decimal = lr.get(4);
                    let buy_rate: Option<Decimal> = lr.get(5);
                    let sell_rate: Option<Decimal> = lr.get(6);

                    by_pub.entry(pub_id).or_default().insert(
                        (currency, group, value_tier),
                        RawLine {
                            unit_size: unit_size.normalize().to_string(),
                            sides: RateSides {
                                buy: buy_rate.map(|d| format!("{:.8}", d)),
                                sell: sell_rate.map(|d| format!("{:.8}", d)),
                            },
                        },
                    );
                }
            }

            let empty = BTreeMap::new();
            let mut publications = Vec::with_capacity(rows.len());
            for r in rows {
                let id: Uuid = r.get(0);
                let b_id: String = r.get(1);
                let version: i32 = r.get(2);
                let published_at: DateTime<Utc> = r.get(3);
                let published_by: String = r.get(4);
                let approved_request_id: Uuid = r.get(5);
                let draft_id: Option<Uuid> = r.get(6);
                let prev_id: Option<Uuid> = r.get(7);

                let after_map = by_pub.get(&id).unwrap_or(&empty);
                let before_map = prev_id.and_then(|pid| by_pub.get(&pid)).unwrap_or(&empty);

                let keys: BTreeSet<&LineKey> = before_map.keys().chain(after_map.keys()).collect();
                let mut lines = Vec::with_capacity(keys.len());
                for key in keys {
                    let before_line = before_map.get(key);
                    let after_line = after_map.get(key);
                    let unit_size = after_line
                        .or(before_line)
                        .expect("line exists in before or after")
                        .unit_size
                        .clone();
                    lines.push(RatePublicationLine {
                        currency: key.0.clone(),
                        group: key.1.clone(),
                        value_tier: key.2.clone(),
                        unit_size,
                        before: before_line.map(|l| l.sides.clone()),
                        after: after_line.map(|l| l.sides.clone()),
                    });
                }

                let state = if Some(id) == current_pub_id {
                    "PUBLISHED"
                } else {
                    "SUPERSEDED"
                };

                publications.push(RatePublicationRow {
                    id: id.to_string(),
                    branch_id: b_id,
                    version,
                    published_at: published_at.to_rfc3339(),
                    published_by,
                    approved_request_id: approved_request_id.to_string(),
                    draft_id: draft_id.map(|u| u.to_string()),
                    state: state.to_string(),
                    lines,
                });
            }

            Ok(publications)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(RatePublicationsPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateSides::export_all(&cfg)?;
    RatePublicationLine::export_all(&cfg)?;
    RatePublicationRow::export_all(&cfg)?;
    Ok(())
}
