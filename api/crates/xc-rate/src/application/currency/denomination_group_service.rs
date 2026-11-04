//! Denomination group application service (§2, §3.2).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_auth::domain::access::{Actor, Role, audit_role};
use xc_kernel::money::{Bound, CurrencyCode, Sign};

use crate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use crate::domain::draft::rate_engine_error::RateEngineError;

fn executive_org_wide(who: &Actor, now: DateTime<Utc>) -> Result<(), RateEngineError> {
    who.roles
        .iter()
        .any(|r| {
            r.role == Role::Executive
                && r.branch.is_none()
                && r.valid_from <= now
                && r.valid_to.is_none_or(|to| to > now)
        })
        .then_some(())
        .ok_or_else(|| {
            RateEngineError::OutOfScope("only OWNER may manage denomination groups".to_string())
        })
}

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
        roles_used: &[Role],
    ) -> Result<(), RateEngineError> {
        let now = Utc::now();
        executive_org_wide(who, now)?;

        // Check if group already exists for this currency
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations \
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 LIMIT 1",
        )
        .bind(&who.organization_id)
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
            "INSERT INTO denominations (organization_id, currency_code, face_value, group_code) \
             VALUES ($1::uuid, $2, NULL, $3)",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let event = AuditEvent {
            actor_id: who.user_id.clone(),
            actor_role: audit_role(roles_used, who, now).to_string(),
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
            .append_in(tx, &who.organization_id, &event)
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Maps a denomination group to a board rate row (instrument_key).
    #[allow(clippy::too_many_arguments)]
    pub async fn map_group(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        code: CurrencyCode,
        group: &GroupCode,
        provider: &str,
        instrument_key: &str,
        roles_used: &[Role],
    ) -> Result<(), RateEngineError> {
        let now = Utc::now();
        executive_org_wide(who, now)?;

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
            "SELECT id FROM denominations \
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 LIMIT 1",
        )
        .bind(&who.organization_id)
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
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 AND provider = $4",
        )
        .bind(&who.organization_id)
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
            "INSERT INTO denomination_group_sources \
             (organization_id, currency_code, group_code, provider, instrument_key, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (organization_id, currency_code, group_code, provider) \
             DO UPDATE SET instrument_key = EXCLUDED.instrument_key, \
                           updated_by = EXCLUDED.updated_by, \
                           updated_at = now()",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .bind(provider)
        .bind(instrument_key)
        .bind(&who.user_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let event = AuditEvent {
            actor_id: who.user_id.clone(),
            actor_role: audit_role(roles_used, who, now).to_string(),
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
            .append_in(tx, &who.organization_id, &event)
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
        roles_used: &[Role],
    ) -> Result<(), RateEngineError> {
        let now = Utc::now();
        executive_org_wide(who, now)?;

        if group.as_str() == "ALL" {
            return Err(RateEngineError::Refused {
                code: "groupAllNotRemovable",
                message: "ไม่สามารถลบกลุ่มนี้ได้".to_string(),
            });
        }

        // Group must exist for this currency
        let exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations \
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 LIMIT 1",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if exists.is_none() {
            return Err(RateEngineError::NotFound(
                "ไม่พบกลุ่มสำหรับสกุลเงินที่ระบุ".to_string(),
            ));
        }

        // Refuse if face values belong to this group
        let has_face_values: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM denominations \
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 AND face_value IS NOT NULL LIMIT 1",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if has_face_values.is_some() {
            return Err(RateEngineError::Refused {
                code: "groupHasFaceValues",
                message: "ไม่สามารถลบกลุ่มที่มีมูลค่าธนบัตรผูกอยู่ได้".to_string(),
            });
        }

        // Refuse if any current publication prices this group
        let is_priced: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM published_rate_lines l \
             JOIN branches b ON l.publication_id = b.current_publication_id \
             WHERE b.organization_id = $1::uuid AND l.currency_code = $2 AND l.denomination_group = $3 \
             LIMIT 1",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if is_priced.is_some() {
            return Err(RateEngineError::Refused {
                code: "groupPricedByPublication",
                message: "ไม่สามารถลบกลุ่มที่มีการประกาศราคาในปัจจุบันได้".to_string(),
            });
        }

        // Delete group sources and face_value IS NULL denomination rows
        sqlx::query(
            "DELETE FROM denomination_group_sources \
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        sqlx::query(
            "DELETE FROM denominations \
             WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 AND face_value IS NULL",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let event = AuditEvent {
            actor_id: who.user_id.clone(),
            actor_role: audit_role(roles_used, who, now).to_string(),
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
            .append_in(tx, &who.organization_id, &event)
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Adds a face value to a currency's group (`denominationNoteAdd`).
    pub async fn add_note(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        code: CurrencyCode,
        group: &GroupCode,
        face: &str,
    ) -> Result<i64, RateEngineError> {
        let now = Utc::now();
        executive_org_wide(who, now)?;

        let face_val = Bound::numeric(20, 4, Sign::Positive)
            .parse(face)
            .map_err(|e| RateEngineError::ValidationFailed(e.to_string()))?;

        let inserted: Option<(i64,)> = sqlx::query_as(
            "INSERT INTO denominations (organization_id, currency_code, group_code, face_value) \
             VALUES ($1::uuid, $2, $3, $4) \
             ON CONFLICT (organization_id, currency_code, group_code, face_value) \
             WHERE face_value IS NOT NULL \
             DO NOTHING \
             RETURNING id",
        )
        .bind(&who.organization_id)
        .bind(code.as_str())
        .bind(group.as_str())
        .bind(face_val)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let Some((id,)) = inserted else {
            let (id,): (i64,) = sqlx::query_as(
                "SELECT id FROM denominations \
                 WHERE organization_id = $1::uuid AND currency_code = $2 AND group_code = $3 AND face_value = $4",
            )
            .bind(&who.organization_id)
            .bind(code.as_str())
            .bind(group.as_str())
            .bind(face_val)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;
            return Ok(id);
        };

        let event = AuditEvent {
            actor_id: who.user_id.clone(),
            actor_role: audit_role(&[Role::Executive], who, now).to_string(),
            scope: "RATE_ENGINE".to_string(),
            action: "denominationNoteAdd".to_string(),
            object_ref: format!("{}/{}", code.as_str(), group.as_str()),
            branch_id: None,
            before: None,
            after: Some(serde_json::json!({
                "id": id,
                "currency": code.as_str(),
                "group": group.as_str(),
                "faceValue": face_val.to_string(),
            })),
        };
        self.audit
            .append_in(tx, &who.organization_id, &event)
            .await
            .map_err(|e| RateEngineError::Internal(e.to_string()))?;

        Ok(id)
    }
}
