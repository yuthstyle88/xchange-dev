//! `SegmentSnapshotService` (design §4.1, §6, §7, D-13, D-14).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::engine::consent_gate::ConsentGate;
use crate::domain::segment::metrics_source::{
    CustomerMetrics, CustomerMetricsSource, MemberMetricsDto,
};
use crate::domain::segment::rule::SegmentRule;

#[derive(Debug, thiserror::Error)]
pub enum SegmentError {
    #[error("validation failed: {0}")]
    ValidationFailed(String),
    #[error("out of scope")]
    OutOfScope,
    #[error("idempotency conflict")]
    IdempotencyConflict,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct SegmentSnapshotService {
    pool: PgPool,
    metrics_source: Arc<dyn CustomerMetricsSource>,
    consent_gate: Arc<dyn ConsentGate>,
}

impl SegmentSnapshotService {
    pub fn new(
        pool: PgPool,
        metrics_source: Arc<dyn CustomerMetricsSource>,
        consent_gate: Arc<dyn ConsentGate>,
    ) -> Self {
        Self {
            pool,
            metrics_source,
            consent_gate,
        }
    }

    pub fn define<'a>(
        &'a self,
        org: Uuid,
        name: &'a str,
        rule: SegmentRule,
        created_by: Uuid,
    ) -> BoxFuture<'a, Result<(Uuid, i32), SegmentError>> {
        Box::pin(async move {
            if name.trim().is_empty() {
                return Err(SegmentError::ValidationFailed(
                    "name cannot be empty".to_string(),
                ));
            }

            let version: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(version), 0) + 1 \
                 FROM segment_definitions \
                 WHERE organization_id = $1 AND name = $2",
            )
            .bind(org)
            .bind(name)
            .fetch_one(&self.pool)
            .await?;

            let id = Uuid::new_v4();
            let rule_json = serde_json::to_value(&rule)
                .map_err(|e| SegmentError::ValidationFailed(e.to_string()))?;

            sqlx::query(
                "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, now())",
            )
            .bind(id)
            .bind(org)
            .bind(name)
            .bind(version)
            .bind(rule_json)
            .bind(created_by)
            .execute(&self.pool)
            .await?;

            Ok((id, version))
        })
    }

    pub async fn rule_of_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: Uuid,
        definition_id: Uuid,
    ) -> Result<SegmentRule, SegmentError> {
        let rule_val: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT rule FROM segment_definitions WHERE id = $1 AND organization_id = $2",
        )
        .bind(definition_id)
        .bind(org)
        .fetch_optional(&mut **tx)
        .await?;

        let Some(rule_val) = rule_val else {
            return Err(SegmentError::NotFound(format!(
                "definition {definition_id} not found"
            )));
        };

        serde_json::from_value(rule_val).map_err(|e| SegmentError::ValidationFailed(e.to_string()))
    }

    pub async fn snapshot_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: Uuid,
        definition_id: Uuid,
        taken_by: Uuid,
        as_of: DateTime<Utc>,
        candidates: &[CustomerMetrics],
        consent_gate: &dyn ConsentGate,
    ) -> Result<(Uuid, usize), SegmentError> {
        let rule = Self::rule_of_in(tx, org, definition_id).await?;
        let mut members = Vec::new();
        for candidate in candidates {
            if !rule.matches(candidate) {
                continue;
            }
            if !consent_gate.marketing_allowed(&candidate.customer_id) {
                continue;
            }
            let metrics_dto = MemberMetricsDto {
                recency_days: candidate.recency_days,
                frequency: candidate.frequency,
                value_thb: format!("{:.2}", candidate.value_thb),
                tier_code: candidate.tier_code.clone(),
            };
            members.push((candidate.customer_id.clone(), metrics_dto));
        }

        let snap_id = Uuid::new_v4();
        let member_count = members.len();

        sqlx::query(
            "INSERT INTO segment_snapshots (id, organization_id, definition_id, as_of, member_count, taken_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now())",
        )
        .bind(snap_id)
        .bind(org)
        .bind(definition_id)
        .bind(as_of)
        .bind(member_count as i32)
        .bind(taken_by)
        .execute(&mut **tx)
        .await?;

        for (cust_id, metrics_dto) in members {
            let metrics_val = serde_json::to_value(&metrics_dto)
                .map_err(|e| SegmentError::ValidationFailed(e.to_string()))?;

            sqlx::query(
                "INSERT INTO segment_snapshot_members (snapshot_id, customer_id, metrics) \
                 VALUES ($1, $2, $3)",
            )
            .bind(snap_id)
            .bind(&cust_id)
            .bind(metrics_val)
            .execute(&mut **tx)
            .await?;
        }

        Ok((snap_id, member_count))
    }

    pub fn snapshot<'a>(
        &'a self,
        org: Uuid,
        definition_id: Uuid,
        taken_by: Uuid,
        as_of: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<(Uuid, usize), SegmentError>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            let rule = Self::rule_of_in(&mut tx, org, definition_id).await?;
            let lookback = rule
                .lookback_months
                .unwrap_or_else(SegmentRule::default_lookback_months);

            let candidates = self
                .metrics_source
                .load_candidate_metrics(org, lookback, as_of)
                .await
                .map_err(|e| SegmentError::ValidationFailed(e.to_string()))?;

            let res = Self::snapshot_in(
                &mut tx,
                org,
                definition_id,
                taken_by,
                as_of,
                &candidates,
                &*self.consent_gate,
            )
            .await?;

            tx.commit().await?;
            Ok(res)
        })
    }
}
