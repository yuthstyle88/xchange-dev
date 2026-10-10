//! Consent application service (design `customer-identity-consent.md` §4.1, §4.3).

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::consent::{Channel, ConsentError, Purpose};
use crate::domain::consent_text::ConsentText;

#[derive(Debug, Clone)]
pub struct Registration {
    pub user_id: String,
    pub display_name: String,
    pub lang: String,
    pub marketing: Vec<Channel>,
    pub evidence_ref: String,
}

#[derive(Debug, Clone)]
pub struct ConsentChoice {
    pub purpose: Purpose,
    pub channel: Option<Channel>,
    pub granted: bool,
    pub lang: String,
    pub evidence_ref: String,
    pub recorded_by: Option<String>,
}

#[derive(Clone)]
pub struct ConsentService {
    pool: PgPool,
}

impl ConsentService {
    pub fn pg(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn publish(
        &self,
        org: &str,
        published_by: Option<&str>,
        purpose: Purpose,
        lang: &str,
        body: &str,
        now: DateTime<Utc>,
    ) -> AppResult<ConsentText> {
        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;
        let text = Self::publish_in(&mut tx, org, published_by, purpose, lang, body, now).await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(text)
    }

    pub async fn publish_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: &str,
        published_by: Option<&str>,
        purpose: Purpose,
        lang: &str,
        body: &str,
        now: DateTime<Utc>,
    ) -> AppResult<ConsentText> {
        let org_id = Uuid::parse_str(org).map_err(|e| AppError::Domain(e.to_string()))?;
        let pub_by = published_by
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|e| AppError::Domain(e.to_string()))?;

        let next_version: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM consent_texts \
             WHERE organization_id = $1 AND purpose = $2 AND lang = $3",
        )
        .bind(org_id)
        .bind(purpose.as_str())
        .bind(lang)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO consent_texts ( \
                 organization_id, purpose, version, lang, body, published_by, published_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(org_id)
        .bind(purpose.as_str())
        .bind(next_version)
        .bind(lang)
        .bind(body)
        .bind(pub_by)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(ConsentText {
            purpose,
            lang: lang.to_owned(),
            version: next_version,
            body: body.to_owned(),
            published_by: published_by.map(|s| s.to_owned()),
            published_at: now,
        })
    }

    pub async fn register(
        &self,
        org: &str,
        reg: Registration,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        let org_id = Uuid::parse_str(org).map_err(|e| AppError::Domain(e.to_string()))?;
        let user_id = Uuid::parse_str(&reg.user_id).map_err(|e| AppError::Domain(e.to_string()))?;

        let terms_version: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(version) FROM consent_texts \
             WHERE organization_id = $1 AND purpose = 'TERMS' AND lang = $2",
        )
        .bind(org_id)
        .bind(&reg.lang)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let terms_ver = terms_version.ok_or(ConsentError::TextMissing)?;

        let marketing_ver = if !reg.marketing.is_empty() {
            let m_ver: Option<i32> = sqlx::query_scalar(
                "SELECT MAX(version) FROM consent_texts \
                 WHERE organization_id = $1 AND purpose = 'MARKETING' AND lang = $2",
            )
            .bind(org_id)
            .bind(&reg.lang)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;

            Some(m_ver.ok_or(ConsentError::TextMissing)?)
        } else {
            None
        };

        let mut tx = self.pool.begin().await.map_err(AppError::Database)?;

        let next_val: i64 = sqlx::query_scalar(
            "INSERT INTO counters (key, value) VALUES ('customer-LIVE', 1) \
             ON CONFLICT (key) DO UPDATE SET value = counters.value + 1 WHERE counters.value < 999999999 \
             RETURNING value",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        let customer_id = format!("CX-{next_val:08}");

        sqlx::query(
            "INSERT INTO customers ( \
                 id, organization_id, customer_no, display_name, preferred_language, \
                 status, mode, user_id, created_at \
             ) VALUES ($1, $2, $1, $3, $4, 'ACTIVE', 'LIVE', $5, $6)",
        )
        .bind(&customer_id)
        .bind(org_id)
        .bind(&reg.display_name)
        .bind(&reg.lang)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        sqlx::query(
            "INSERT INTO consents ( \
                 id, organization_id, customer_id, purpose, channel, policy_version, \
                 lang, granted, effective_at, evidence_ref, recorded_by, created_at \
             ) VALUES ($1, $2, $3, 'TERMS', NULL, $4, $5, true, $6, $7, NULL, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(org_id)
        .bind(&customer_id)
        .bind(terms_ver)
        .bind(&reg.lang)
        .bind(now)
        .bind(&reg.evidence_ref)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        if let Some(m_ver) = marketing_ver {
            for ch in &reg.marketing {
                sqlx::query(
                    "INSERT INTO consents ( \
                         id, organization_id, customer_id, purpose, channel, policy_version, \
                         lang, granted, effective_at, evidence_ref, recorded_by, created_at \
                     ) VALUES ($1, $2, $3, 'MARKETING', $4, $5, $6, true, $7, $8, NULL, $7)",
                )
                .bind(Uuid::new_v4())
                .bind(org_id)
                .bind(&customer_id)
                .bind(ch.as_str())
                .bind(m_ver)
                .bind(&reg.lang)
                .bind(now)
                .bind(&reg.evidence_ref)
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
            }
        }

        tx.commit().await.map_err(AppError::Database)?;

        Ok(customer_id)
    }

    pub async fn record(
        &self,
        customer_id: &str,
        choice: ConsentChoice,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let org_id: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM customers WHERE id = $1")
                .bind(customer_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;

        let org_id = org_id.ok_or_else(|| AppError::NotFound {
            entity: "customer",
            id: customer_id.to_string(),
        })?;

        let version: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(version) FROM consent_texts \
             WHERE organization_id = $1 AND purpose = $2 AND lang = $3",
        )
        .bind(org_id)
        .bind(choice.purpose.as_str())
        .bind(&choice.lang)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let version = version.ok_or(ConsentError::TextMissing)?;

        let rec_by = choice
            .recorded_by
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|e| AppError::Domain(e.to_string()))?;

        sqlx::query(
            "INSERT INTO consents ( \
                 id, organization_id, customer_id, purpose, channel, policy_version, \
                 lang, granted, effective_at, evidence_ref, recorded_by, created_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $9)",
        )
        .bind(Uuid::new_v4())
        .bind(org_id)
        .bind(customer_id)
        .bind(choice.purpose.as_str())
        .bind(choice.channel.map(|c| c.as_str()))
        .bind(version)
        .bind(&choice.lang)
        .bind(choice.granted)
        .bind(now)
        .bind(&choice.evidence_ref)
        .bind(rec_by)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    pub async fn marketing_allowed(&self, customer_id: &str, channel: Channel) -> AppResult<bool> {
        let granted: Option<bool> = sqlx::query_scalar(
            "SELECT granted FROM consents \
             WHERE customer_id = $1 AND purpose = 'MARKETING' AND channel = $2 \
             ORDER BY effective_at DESC, created_at DESC \
             LIMIT 1",
        )
        .bind(customer_id)
        .bind(channel.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(granted.unwrap_or(false))
    }
}
