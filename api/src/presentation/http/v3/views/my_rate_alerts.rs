//! RA-2b (#1429): `GET /api/v3/views/my_rate_alerts` reads the caller's own rate alerts.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_rate::domain::publication::freshness::Freshness;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rate_alerts::{
    MyRateAlertItem, MyRateAlertsData, RateAlertLastEvent,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

#[derive(sqlx::FromRow)]
struct AlertDbRow {
    id: String,
    version: i32,
    currency_code: String,
    shop_side: String,
    condition: String,
    target: Decimal,
    enabled: bool,
    branch_id: String,
    unit_size: Decimal,
    relation: String,
    expires_at: Option<DateTime<Utc>>,
    current_rate: Option<Decimal>,
    published_at: Option<DateTime<Utc>>,
    source_snapshot: Option<serde_json::Value>,
    last_event_observed_rate: Option<Decimal>,
    last_event_trigger_at: Option<DateTime<Utc>>,
    last_event_episode_no: Option<i32>,
}

pub struct MyRateAlertsView {
    pool: PgPool,
}

impl MyRateAlertsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for MyRateAlertsView {
    const NAME: &'static str = "my_rate_alerts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "myRateAlerts");
    type Data = MyRateAlertsData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let customer_id = who.customer_id.clone();
        async move {
            let Some(cid) = customer_id else {
                return Ok(MyRateAlertsData { alerts: vec![] });
            };

            let rows: Vec<AlertDbRow> = sqlx::query_as(
                "SELECT \
                    ra.id::text AS id, \
                    ra.version, \
                    ra.currency_code, \
                    ra.shop_side, \
                    ra.condition, \
                    ra.target, \
                    ra.enabled, \
                    ra.branch_id, \
                    ra.unit_size, \
                    ra.relation, \
                    ra.expires_at, \
                    CASE \
                        WHEN ra.shop_side = 'BUY' THEN prl.buy_rate \
                        WHEN ra.shop_side = 'SELL' THEN prl.sell_rate \
                        ELSE NULL \
                    END AS current_rate, \
                    rp.published_at, \
                    prl.source_snapshot, \
                    le.observed_rate AS last_event_observed_rate, \
                    le.trigger_at AS last_event_trigger_at, \
                    le.episode_no AS last_event_episode_no \
                 FROM rate_alerts ra \
                 LEFT JOIN branches b ON b.id = ra.branch_id \
                 LEFT JOIN rate_publications rp ON rp.id = b.current_publication_id \
                 LEFT JOIN published_rate_lines prl \
                    ON prl.publication_id = rp.id \
                   AND prl.currency_code = ra.currency_code \
                   AND prl.denomination_group = 'ALL' \
                   AND prl.unit_size = ra.unit_size \
                   AND prl.tier_lower_exclusive_thb IS NULL \
                 LEFT JOIN LATERAL ( \
                    SELECT observed_rate, trigger_at, episode_no \
                    FROM rate_alert_events \
                    WHERE alert_id = ra.id \
                    ORDER BY episode_no DESC \
                    LIMIT 1 \
                 ) le ON true \
                 WHERE ra.customer_id = $1 AND ra.deleted_at IS NULL \
                 ORDER BY ra.created_at DESC",
            )
            .bind(cid)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let now = Utc::now();
            let alerts = rows
                .into_iter()
                .map(|row| {
                    let side = match row.shop_side.as_str() {
                        "BUY" => Side::Buy,
                        _ => Side::Sell,
                    };
                    let customer_side = match row.shop_side.as_str() {
                        "SELL" => "CUSTOMER_BUY",
                        "BUY" => "CUSTOMER_SELL",
                        other => other,
                    };
                    let (current_rate, rate_published_at) =
                        match (row.current_rate, row.published_at) {
                            (Some(rate), Some(pub_at)) => {
                                (Some(format!("{:.8}", rate)), Some(pub_at.to_rfc3339()))
                            }
                            _ => (None, None),
                        };
                    let rate_stale = if current_rate.is_some() {
                        Some(
                            row.source_snapshot
                                .as_ref()
                                .map(|snap| Freshness::of_side(snap, side, now).stale)
                                .unwrap_or(false),
                        )
                    } else {
                        None
                    };
                    let (expires_at, expired) = match row.expires_at {
                        Some(exp) => (Some(exp.to_rfc3339()), exp <= now),
                        None => (None, false),
                    };
                    let last_event = match (
                        row.last_event_observed_rate,
                        row.last_event_trigger_at,
                        row.last_event_episode_no,
                    ) {
                        (Some(rate), Some(trigger_at), Some(episode_no)) => {
                            Some(RateAlertLastEvent {
                                observed_rate: format!("{:.8}", rate),
                                trigger_at: trigger_at.to_rfc3339(),
                                episode_no,
                            })
                        }
                        _ => None,
                    };

                    MyRateAlertItem {
                        id: row.id,
                        version: i64::from(row.version),
                        currency: row.currency_code,
                        customer_side: customer_side.to_string(),
                        condition: row.condition,
                        target: row.target.to_string(),
                        enabled: row.enabled,
                        branch: row.branch_id,
                        unit_size: row.unit_size.to_string(),
                        current_rate,
                        rate_published_at,
                        relation: row.relation,
                        expires_at,
                        expired,
                        rate_stale,
                        last_event,
                    }
                })
                .collect();

            Ok(MyRateAlertsData { alerts })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(MyRateAlertsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateAlertLastEvent::export_all(&cfg)?;
    MyRateAlertItem::export_all(&cfg)?;
    MyRateAlertsData::export_all(&cfg)?;
    Ok(())
}
