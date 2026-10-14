//! `PgMembershipMirror` (design §4.1, LY-4 #1402): `MembershipMirror` over `customer_tier_mirror`.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::pii::{IdentityType, LookupHasher, normalize};

use crate::domain::membership_feed::MembershipPage;
use crate::domain::membership_mirror::{MembershipMirror, MirroredTier};

#[derive(FromRow)]
struct TierRow {
    tier_code: String,
    tier_rule_id: Uuid,
    tier_rule_version: i32,
    effective_at: DateTime<Utc>,
    effective_until: Option<DateTime<Utc>>,
}

pub struct PgMembershipMirror {
    pool: PgPool,
    hasher: LookupHasher,
}

impl PgMembershipMirror {
    /// `hasher` is the one `customer_identities.lookup_hash` is written with.
    pub fn new(pool: PgPool, hasher: LookupHasher) -> Self {
        Self { pool, hasher }
    }
}

impl MembershipMirror for PgMembershipMirror {
    fn tier_of<'a>(
        &'a self,
        org: Uuid,
        customer: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<MirroredTier>>> {
        Box::pin(async move {
            let row: Option<TierRow> = sqlx::query_as(
                "SELECT m.tier_code, m.tier_rule_id, m.tier_rule_version, m.effective_at, m.effective_until \
                 FROM customer_identities i \
                 JOIN customer_tier_mirror m \
                   ON m.organization_id = i.organization_id \
                  AND m.phone_lookup_hash = i.lookup_hash \
                 WHERE i.organization_id = $1 \
                   AND i.customer_id = $2 \
                   AND i.type = 'PHONE' \
                   AND i.status = 'ACTIVE' \
                   AND i.verified_at IS NOT NULL",
            )
            .bind(org)
            .bind(customer)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

            Ok(row.map(|r| MirroredTier {
                tier_code: r.tier_code,
                tier_rule_id: r.tier_rule_id,
                tier_rule_version: r.tier_rule_version,
                effective_at: r.effective_at,
                effective_until: r.effective_until,
            }))
        })
    }

    fn apply_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org: Uuid,
        page: &'a MembershipPage,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            for item in &page.items {
                let e164 = if item.phone_key.starts_with('+') {
                    item.phone_key.clone()
                } else {
                    format!("+66{}", item.phone_key)
                };

                let normalized = match normalize(IdentityType::Phone, &e164, None) {
                    Ok(n) => n,
                    Err(_) => continue,
                };

                let phone_lookup_hash = self.hasher.hash(IdentityType::Phone, &normalized);

                sqlx::query(
                    "INSERT INTO customer_tier_mirror ( \
                         organization_id, phone_lookup_hash, tier_code, tier_rule_id, \
                         tier_rule_version, effective_at, effective_until, loyalty_seq, synced_at \
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now()) \
                     ON CONFLICT (organization_id, phone_lookup_hash) \
                     DO UPDATE SET \
                         tier_code = EXCLUDED.tier_code, \
                         tier_rule_id = EXCLUDED.tier_rule_id, \
                         tier_rule_version = EXCLUDED.tier_rule_version, \
                         effective_at = EXCLUDED.effective_at, \
                         effective_until = EXCLUDED.effective_until, \
                         loyalty_seq = EXCLUDED.loyalty_seq, \
                         synced_at = EXCLUDED.synced_at \
                     WHERE EXCLUDED.loyalty_seq > customer_tier_mirror.loyalty_seq",
                )
                .bind(org)
                .bind(&phone_lookup_hash[..])
                .bind(&item.tier_code)
                .bind(item.tier_rule_id)
                .bind(item.tier_rule_version)
                .bind(item.effective_at)
                .bind(item.effective_until)
                .bind(item.seq)
                .execute(&mut **tx)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
            }
            Ok(())
        })
    }
}
