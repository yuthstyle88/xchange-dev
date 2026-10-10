//! Denomination group application service (§2, §3.2).

use std::sync::Arc;

use sqlx::{Postgres, Transaction};
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::money::CurrencyCode;

use crate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use crate::domain::draft::rate_engine_error::RateEngineError;

#[derive(Clone)]
pub struct DenominationGroupService {
    groups: Arc<dyn DenominationGroups>,
    audit: Arc<dyn AuditLog>,
}

impl DenominationGroupService {
    pub fn new(groups: Arc<dyn DenominationGroups>, audit: Arc<dyn AuditLog>) -> Self {
        Self { groups, audit }
    }

    pub fn groups(&self) -> &Arc<dyn DenominationGroups> {
        &self.groups
    }

    /// Creates a denomination group (face_value IS NULL) for a currency.
    pub async fn create_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        code: CurrencyCode,
        group: &GroupCode,
    ) -> Result<(), RateEngineError> {
        if who.role() != Role::Owner {
            return Err(RateEngineError::OutOfScope(
                "only OWNER may manage denomination groups".to_string(),
            ));
        }

        // Check if group already exists for this currency
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations WHERE currency_code = $1 AND group_code = $2 LIMIT 1",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if exists.is_some() {
            // Idempotent: accepted, nothing written, no audit
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO denominations (currency_code, face_value, group_code) VALUES ($1, NULL, $2)",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let event = AuditEvent {
            actor_id: who.id().to_string(),
            actor_role: who.role().as_str().to_string(),
            scope: "RATE_ENGINE".to_string(),
            action: "denominationGroupCreate".to_string(),
            object_ref: format!("{}/{}", code.as_str(), group.as_str()),
            branch_id: None,
            before: None,
            after: Some(serde_json::json!({
                "currency": code.as_str(),
                "group": group.as_str(),
            })),
        };
        self.audit
            .append(tx, &event)
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Maps a denomination group to a board rate row (instrument_key).
    pub async fn map_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        code: CurrencyCode,
        group: &GroupCode,
        provider: &str,
        instrument_key: &str,
    ) -> Result<(), RateEngineError> {
        if who.role() != Role::Owner {
            return Err(RateEngineError::OutOfScope(
                "only OWNER may manage denomination groups".to_string(),
            ));
        }

        let provider_trimmed = provider.trim();
        let key_trimmed = instrument_key.trim();
        if provider_trimmed.is_empty() {
            return Err(RateEngineError::ValidationFailed(
                "provider cannot be empty or blank".to_string(),
            ));
        }
        if key_trimmed.is_empty() {
            return Err(RateEngineError::ValidationFailed(
                "instrument_key cannot be empty or blank".to_string(),
            ));
        }

        // Group must exist for that currency
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations WHERE currency_code = $1 AND group_code = $2 LIMIT 1",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if exists.is_none() {
            return Err(RateEngineError::ValidationFailed(format!(
                "group {} does not exist for currency {}",
                group.as_str(),
                code.as_str()
            )));
        }

        // Check if existing mapping has the same key
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT instrument_key FROM denomination_group_sources \
             WHERE currency_code = $1 AND group_code = $2 AND provider = $3",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .bind(provider)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let old_key = existing.map(|(k,)| k);
        if old_key.as_deref() == Some(instrument_key) {
            // Unchanged: accepted, nothing written, no audit
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO denomination_group_sources (currency_code, group_code, provider, instrument_key, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4, $5, now()) \
             ON CONFLICT (currency_code, group_code, provider) \
             DO UPDATE SET instrument_key = EXCLUDED.instrument_key, \
                           updated_by = EXCLUDED.updated_by, \
                           updated_at = now()",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .bind(provider)
        .bind(instrument_key)
        .bind(who.id())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let event = AuditEvent {
            actor_id: who.id().to_string(),
            actor_role: who.role().as_str().to_string(),
            scope: "RATE_ENGINE".to_string(),
            action: "denominationGroupMap".to_string(),
            object_ref: format!("{}/{}/{}", code.as_str(), group.as_str(), provider),
            branch_id: None,
            before: old_key.map(|k| {
                serde_json::json!({
                    "provider": provider,
                    "instrument_key": k,
                })
            }),
            after: Some(serde_json::json!({
                "provider": provider,
                "instrument_key": instrument_key,
            })),
        };
        self.audit
            .append(tx, &event)
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Removes a denomination group and its board mappings.
    pub async fn remove_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        code: CurrencyCode,
        group: &GroupCode,
    ) -> Result<(), RateEngineError> {
        if who.role() != Role::Owner {
            return Err(RateEngineError::OutOfScope(
                "only OWNER may manage denomination groups".to_string(),
            ));
        }

        if group.as_str() == "ALL" {
            return Err(RateEngineError::ValidationFailed(
                "the ALL group cannot be removed".to_string(),
            ));
        }

        // Group must exist for this currency
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations WHERE currency_code = $1 AND group_code = $2 LIMIT 1",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if exists.is_none() {
            return Err(RateEngineError::ValidationFailed(format!(
                "group {} does not exist for currency {}",
                group.as_str(),
                code.as_str()
            )));
        }

        // Refuse if face values belong to this group
        let has_face_values: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations WHERE currency_code = $1 AND group_code = $2 AND face_value IS NOT NULL LIMIT 1",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if has_face_values.is_some() {
            return Err(RateEngineError::ValidationFailed(
                "cannot remove a group that face values belong to".to_string(),
            ));
        }

        // Refuse if any current publication prices this group
        let is_priced: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM published_rate_lines l \
             JOIN branches b ON l.publication_id = b.current_publication_id \
             WHERE l.currency_code = $1 AND l.denomination_group = $2 \
             LIMIT 1",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if is_priced.is_some() {
            return Err(RateEngineError::ValidationFailed(
                "cannot remove a group that a current publication prices".to_string(),
            ));
        }

        // Delete group sources and face_value IS NULL denomination rows
        sqlx::query(
            "DELETE FROM denomination_group_sources WHERE currency_code = $1 AND group_code = $2",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        sqlx::query(
            "DELETE FROM denominations WHERE currency_code = $1 AND group_code = $2 AND face_value IS NULL",
        )
        .bind(code.as_str())
        .bind(group.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let event = AuditEvent {
            actor_id: who.id().to_string(),
            actor_role: who.role().as_str().to_string(),
            scope: "RATE_ENGINE".to_string(),
            action: "denominationGroupRemove".to_string(),
            object_ref: format!("{}/{}", code.as_str(), group.as_str()),
            branch_id: None,
            before: None,
            after: Some(serde_json::json!({
                "currency": code.as_str(),
                "group": group.as_str(),
            })),
        };
        self.audit
            .append(tx, &event)
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        Ok(())
    }
}
