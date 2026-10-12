//! Phone verification repository over PostgreSQL (CU-P1, design §6.3).

use chrono::{DateTime, Utc};
use sqlx::{Error, PgPool, Postgres, Transaction};
use uuid::Uuid;

pub struct ChallengeDbRow {
    pub organization_id: Uuid,
    pub customer_id: String,
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub code_hash: Vec<u8>,
    pub attempts: i32,
    pub phone_encrypted: Vec<u8>,
    pub key_id: String,
    pub phone_lookup_hash: Vec<u8>,
}

pub struct InsertChallengeParams<'a> {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub customer_id: &'a str,
    pub branch_id: &'a str,
    pub phone_encrypted: &'a [u8],
    pub key_id: &'a str,
    pub phone_lookup_hash: &'a [u8],
    pub code_hash: &'a [u8],
    pub expires_at: DateTime<Utc>,
    pub teller_user_id: Uuid,
    pub created_at: DateTime<Utc>,
}

pub struct InsertIdentityParams<'a> {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub customer_id: &'a str,
    pub encrypted_value: &'a [u8],
    pub key_id: &'a str,
    pub lookup_hash: &'a [u8],
    pub now: DateTime<Utc>,
}

pub struct PgPhoneVerificationRepository {
    pool: PgPool,
}

impl PgPhoneVerificationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Selects customer preferred_language with `FOR UPDATE` row lock.
    pub async fn lock_customer(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer_id: &str,
        org_id: Uuid,
    ) -> Result<Option<String>, Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT preferred_language FROM customers WHERE id = $1 AND organization_id = $2 FOR UPDATE",
        )
        .bind(customer_id)
        .bind(org_id)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.map(|(lang,)| lang))
    }

    pub async fn count_customer_challenges(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        customer_id: &str,
        since: DateTime<Utc>,
    ) -> Result<i64, Error> {
        sqlx::query_scalar(
            "SELECT count(*) FROM phone_verifications \
             WHERE organization_id = $1 AND customer_id = $2 AND created_at >= $3",
        )
        .bind(org_id)
        .bind(customer_id)
        .bind(since)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn count_phone_challenges(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        phone_lookup_hash: &[u8],
        since: DateTime<Utc>,
    ) -> Result<i64, Error> {
        sqlx::query_scalar(
            "SELECT count(*) FROM phone_verifications \
             WHERE organization_id = $1 AND phone_lookup_hash = $2 AND created_at >= $3",
        )
        .bind(org_id)
        .bind(phone_lookup_hash)
        .bind(since)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn count_teller_challenges(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        teller_user_id: Uuid,
        since: DateTime<Utc>,
    ) -> Result<i64, Error> {
        sqlx::query_scalar(
            "SELECT count(*) FROM phone_verifications \
             WHERE organization_id = $1 AND requested_by = $2 AND created_at >= $3",
        )
        .bind(org_id)
        .bind(teller_user_id)
        .bind(since)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn fetch_branch_name(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch_id: &str,
    ) -> Result<Option<String>, Error> {
        sqlx::query_scalar("SELECT name FROM branches WHERE id = $1")
            .bind(branch_id)
            .fetch_optional(&mut **tx)
            .await
    }

    pub async fn supersede_pending(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        customer_id: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let result = sqlx::query(
            "UPDATE phone_verifications \
             SET status = 'SUPERSEDED', decided_at = $1 \
             WHERE organization_id = $2 AND customer_id = $3 AND status = 'PENDING'",
        )
        .bind(now)
        .bind(org_id)
        .bind(customer_id)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn insert_challenge(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        p: InsertChallengeParams<'_>,
    ) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO phone_verifications ( \
                 id, organization_id, customer_id, branch_id, phone_encrypted, \
                 key_id, phone_lookup_hash, code_hash, expires_at, attempts, \
                 status, requested_by, created_at \
             ) VALUES ( \
                 $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13 \
             )",
        )
        .bind(p.id)
        .bind(p.organization_id)
        .bind(p.customer_id)
        .bind(p.branch_id)
        .bind(p.phone_encrypted)
        .bind(p.key_id)
        .bind(p.phone_lookup_hash)
        .bind(p.code_hash)
        .bind(p.expires_at)
        .bind(0)
        .bind("PENDING")
        .bind(p.teller_user_id)
        .bind(p.created_at)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn fetch_challenge(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge_id: Uuid,
    ) -> Result<Option<ChallengeDbRow>, Error> {
        let row_opt = sqlx::query(
            "SELECT organization_id, customer_id, status, expires_at, code_hash, attempts, \
                    phone_encrypted, key_id, phone_lookup_hash \
             FROM phone_verifications WHERE id = $1",
        )
        .bind(challenge_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row_opt {
            Some(row) => {
                use sqlx::Row;
                Ok(Some(ChallengeDbRow {
                    organization_id: row.try_get("organization_id")?,
                    customer_id: row.try_get("customer_id")?,
                    status: row.try_get("status")?,
                    expires_at: row.try_get("expires_at")?,
                    code_hash: row.try_get("code_hash")?,
                    attempts: row.try_get("attempts")?,
                    phone_encrypted: row.try_get("phone_encrypted")?,
                    key_id: row.try_get("key_id")?,
                    phone_lookup_hash: row.try_get("phone_lookup_hash")?,
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn lock_challenge(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge_id: Uuid,
    ) -> Result<Option<ChallengeDbRow>, Error> {
        let row_opt = sqlx::query(
            "SELECT organization_id, customer_id, status, expires_at, code_hash, attempts, \
                    phone_encrypted, key_id, phone_lookup_hash \
             FROM phone_verifications WHERE id = $1 FOR UPDATE",
        )
        .bind(challenge_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row_opt {
            Some(row) => {
                use sqlx::Row;
                Ok(Some(ChallengeDbRow {
                    organization_id: row.try_get("organization_id")?,
                    customer_id: row.try_get("customer_id")?,
                    status: row.try_get("status")?,
                    expires_at: row.try_get("expires_at")?,
                    code_hash: row.try_get("code_hash")?,
                    attempts: row.try_get("attempts")?,
                    phone_encrypted: row.try_get("phone_encrypted")?,
                    key_id: row.try_get("key_id")?,
                    phone_lookup_hash: row.try_get("phone_lookup_hash")?,
                }))
            }
            None => Ok(None),
        }
    }

    pub async fn atomic_record_wrong_code(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge_id: Uuid,
        max_attempts: i32,
        now: DateTime<Utc>,
    ) -> Result<Option<(i32, String)>, Error> {
        sqlx::query_as(
            "UPDATE phone_verifications \
             SET attempts = attempts + 1, \
                 status = CASE WHEN attempts + 1 >= $1 THEN 'LOCKED' ELSE status END, \
                 decided_at = CASE WHEN attempts + 1 >= $1 THEN $2 ELSE decided_at END \
             WHERE id = $3 AND status = 'PENDING' \
             RETURNING attempts, status",
        )
        .bind(max_attempts)
        .bind(now)
        .bind(challenge_id)
        .fetch_optional(&mut **tx)
        .await
    }

    pub async fn find_active_phone_identity(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        lookup_hash: &[u8],
    ) -> Result<Option<String>, Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT customer_id FROM customer_identities \
             WHERE organization_id = $1 AND type = 'PHONE' AND status = 'ACTIVE' AND lookup_hash = $2 \
             LIMIT 1",
        )
        .bind(org_id)
        .bind(lookup_hash)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row.map(|(c,)| c))
    }

    pub async fn revoke_active_phone_identities(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        org_id: Uuid,
        customer_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        sqlx::query(
            "UPDATE customer_identities \
             SET status = 'REVOKED', updated_at = $1 \
             WHERE organization_id = $2 AND customer_id = $3 AND type = 'PHONE' AND status = 'ACTIVE'",
        )
        .bind(now)
        .bind(org_id)
        .bind(customer_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn insert_customer_identity(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        p: InsertIdentityParams<'_>,
    ) -> Result<(), Error> {
        sqlx::query(
            "INSERT INTO customer_identities ( \
                 id, organization_id, customer_id, type, encrypted_value, \
                 key_id, lookup_hash, verified_at, status, created_at \
             ) VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, $7, 'ACTIVE', $8)",
        )
        .bind(p.id)
        .bind(p.organization_id)
        .bind(p.customer_id)
        .bind(p.encrypted_value)
        .bind(p.key_id)
        .bind(p.lookup_hash)
        .bind(p.now)
        .bind(p.now)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn verify_challenge(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge_id: Uuid,
        identity_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<u64, Error> {
        let result = sqlx::query(
            "UPDATE phone_verifications \
             SET status = 'VERIFIED', identity_id = $1, decided_at = $2 \
             WHERE id = $3 AND status = 'PENDING'",
        )
        .bind(identity_id)
        .bind(now)
        .bind(challenge_id)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected())
    }
}
