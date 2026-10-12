//! Rate alert evaluator (AL-4, design `docs/design/blueprint/rate-alerts.md` §5, §7).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CustomerSide, Side};
use xc_kernel::notifications::{NotificationObject, NotificationRequest, Notifications};
use xc_rate::domain::publication::freshness::Freshness;

use crate::domain::crossing_detector::{AlertState, CrossingDetector};
use crate::domain::repository::RateAlertRepository;
use crate::domain::source_policy::{AlertSourcePolicy, ObservePolicy, SideMode, SideSnapshot};

pub struct RateAlertEvaluator {
    pool: PgPool,
    notifications: Arc<dyn Notifications>,
    repository: Arc<dyn RateAlertRepository>,
}

impl RateAlertEvaluator {
    pub fn new(
        pool: PgPool,
        notifications: Arc<dyn Notifications>,
        repository: Arc<dyn RateAlertRepository>,
    ) -> Self {
        Self {
            pool,
            notifications,
            repository,
        }
    }

    pub fn with_pool(pool: PgPool, notifications: Arc<dyn Notifications>) -> Self {
        let repo =
            Arc::new(crate::infrastructure::repository::PgRateAlertRepository::new(pool.clone()));
        Self::new(pool, notifications, repo)
    }

    pub async fn evaluate_publication(
        &self,
        branch: &BranchId,
        publication: Uuid,
    ) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        let pub_row =
            sqlx::query("SELECT version, published_at FROM rate_publications WHERE id = $1")
                .bind(publication)
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?;

        let Some(pub_row) = pub_row else {
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(());
        };

        let version: i32 = pub_row.get("version");
        let published_at: DateTime<Utc> = pub_row.get("published_at");

        let alerts = self
            .repository
            .lock_due_in(&mut tx, branch, version)
            .await?;
        if alerts.is_empty() {
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(());
        }

        let line_rows = sqlx::query(
            "SELECT id, currency_code, unit_size, buy_rate, sell_rate, source_snapshot \
             FROM published_rate_lines WHERE publication_id = $1",
        )
        .bind(publication)
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        struct PubLine {
            id: Uuid,
            unit_size: Decimal,
            buy_rate: Option<Decimal>,
            sell_rate: Option<Decimal>,
            source_snapshot: serde_json::Value,
        }

        let mut lines_by_currency: HashMap<String, PubLine> = HashMap::new();
        for row in line_rows {
            let code: String = row.get("currency_code");
            lines_by_currency.insert(
                code.trim().to_string(),
                PubLine {
                    id: row.get("id"),
                    unit_size: row.get("unit_size"),
                    buy_rate: row.get("buy_rate"),
                    sell_rate: row.get("sell_rate"),
                    source_snapshot: row.get("source_snapshot"),
                },
            );
        }

        let mut policy_by_org: HashMap<Uuid, AlertSourcePolicy> = HashMap::new();

        for alert in alerts {
            let policy = match policy_by_org.get(&alert.organization_id) {
                Some(p) => *p,
                None => {
                    let settings = self
                        .repository
                        .settings_in(&mut tx, alert.organization_id)
                        .await?;
                    let observe_policy = settings
                        .map(|s| s.observe_policy)
                        .unwrap_or(ObservePolicy::AllFresh);
                    let p = AlertSourcePolicy::new(observe_policy);
                    policy_by_org.insert(alert.organization_id, p);
                    p
                }
            };

            let Some(line) = lines_by_currency.get(alert.currency.as_str()) else {
                sqlx::query(
                    "UPDATE rate_alerts SET last_publication_version = $1, updated_at = now() WHERE id = $2"
                )
                .bind(version)
                .bind(alert.id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
                continue;
            };

            if line.unit_size != alert.unit_size.value() {
                sqlx::query(
                    "UPDATE rate_alerts SET \
                     enabled = false, \
                     disabled_reason = 'UNIT_CHANGED', \
                     last_publication_version = $1, \
                     updated_at = now() \
                     WHERE id = $2",
                )
                .bind(version)
                .bind(alert.id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
                continue;
            }

            let (rate_opt, mode_key) = match alert.shop_side {
                Side::Buy => (line.buy_rate, "buy_mode"),
                Side::Sell => (line.sell_rate, "sell_mode"),
            };

            let Some(rate) = rate_opt else {
                sqlx::query(
                    "UPDATE rate_alerts SET last_publication_version = $1, updated_at = now() WHERE id = $2"
                )
                .bind(version)
                .bind(alert.id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
                continue;
            };

            let mode =
                if line.source_snapshot.get(mode_key).and_then(|v| v.as_str()) == Some("MANUAL") {
                    SideMode::Manual
                } else {
                    SideMode::Auto
                };

            let freshness =
                Freshness::of_side(&line.source_snapshot, alert.shop_side, published_at);
            let side_snapshot = SideSnapshot { rate, mode };
            let obs = policy.observes(&side_snapshot, &freshness);

            let state = AlertState {
                condition: alert.condition,
                target: alert.target,
                cooldown_seconds: alert.cooldown_seconds,
                relation: alert.relation,
                episode_no: alert.episode_no,
                last_triggered_at: alert.last_triggered_at,
            };

            let (new_state, maybe_fire) = CrossingDetector::observe(state, obs, published_at);

            if let Some(fire) = maybe_fire {
                let event_id = Uuid::new_v4();
                sqlx::query(
                    "INSERT INTO rate_alert_events \
                     (id, organization_id, alert_id, published_rate_line_id, observed_rate, trigger_at, episode_no, created_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, now())"
                )
                .bind(event_id)
                .bind(alert.organization_id)
                .bind(alert.id.as_uuid())
                .bind(line.id)
                .bind(fire.observed_rate)
                .bind(fire.trigger_at)
                .bind(fire.episode_no)
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;

                let recipient_user_id: Option<Uuid> =
                    sqlx::query_scalar("SELECT user_id FROM customers WHERE id = $1")
                        .bind(&alert.customer_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(AppError::Database)?;

                let user_id = recipient_user_id.ok_or_else(|| {
                    AppError::Internal(format!(
                        "customer user_id not found for {}",
                        alert.customer_id
                    ))
                })?;

                let customer_side_str = match alert.shop_side {
                    Side::Buy => CustomerSide::CustomerSell.as_str(),
                    Side::Sell => CustomerSide::CustomerBuy.as_str(),
                };

                let mut params = BTreeMap::new();
                params.insert("currency".to_string(), alert.currency.as_str().to_string());
                params.insert("side".to_string(), customer_side_str.to_string());
                params.insert("relation".to_string(), alert.condition.as_str().to_string());

                let object = NotificationObject {
                    object_type: "rate_alert_event".to_string(),
                    object_id: event_id.to_string(),
                };

                let event_id_str = format!("rate_alert:{event_id}");

                let req = NotificationRequest::new(
                    user_id.to_string(),
                    event_id_str,
                    "rate_alert_crossed",
                    Some(object),
                    params,
                )
                .map_err(|e| AppError::Internal(format!("notification request error: {e}")))?;

                self.notifications.notify_in(&mut tx, &req).await?;
            }

            sqlx::query(
                "UPDATE rate_alerts SET \
                 relation = $1, \
                 episode_no = $2, \
                 last_triggered_at = $3, \
                 last_publication_version = $4, \
                 updated_at = now() \
                 WHERE id = $5",
            )
            .bind(new_state.relation.as_str())
            .bind(new_state.episode_no)
            .bind(new_state.last_triggered_at)
            .bind(version)
            .bind(alert.id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        }

        tx.commit().await.map_err(AppError::Database)?;
        Ok(())
    }
}
