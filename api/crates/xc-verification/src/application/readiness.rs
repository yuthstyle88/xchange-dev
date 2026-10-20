//! Verification customer readiness (design §5, §4.3, V12; ticket #1394).

use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    None,
    Pending,
    NeedsUpdate,
    Verified { until: DateTime<Utc> },
    Expired,
}

impl Readiness {
    pub async fn of(pool: &PgPool, customer_id: &str) -> Result<Self, ReadinessError> {
        let row: Option<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT status, expires_at FROM verification_cases \
             WHERE customer_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(customer_id)
        .fetch_optional(pool)
        .await?;

        let Some((status, expires_at)) = row else {
            return Ok(Readiness::None);
        };

        match status.as_str() {
            "DRAFT" => Ok(Readiness::None),
            "SUBMITTED" | "UNDER_REVIEW" => Ok(Readiness::Pending),
            "NEEDS_UPDATE" => Ok(Readiness::NeedsUpdate),
            "VERIFIED" => {
                if let Some(until) = expires_at {
                    Ok(Readiness::Verified { until })
                } else {
                    Ok(Readiness::None)
                }
            }
            "EXPIRED" => Ok(Readiness::Expired),
            "REJECTED" | "REVOKED" => Ok(Readiness::None),
            _ => Ok(Readiness::None),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Pending => "PENDING",
            Self::NeedsUpdate => "NEEDS_UPDATE",
            Self::Verified { .. } => "VERIFIED",
            Self::Expired => "EXPIRED",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadinessError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct ReadinessService {
    pool: PgPool,
}

impl ReadinessService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self::new(pool)
    }

    pub async fn of(&self, customer_id: &str) -> Result<Readiness, ReadinessError> {
        Readiness::of(&self.pool, customer_id).await
    }
}
