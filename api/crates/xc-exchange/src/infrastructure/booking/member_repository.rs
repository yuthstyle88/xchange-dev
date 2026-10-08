//! Postgres member repository.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use xc_kernel::ids::BranchId;

use crate::domain::booking::entities::member::Member;
use crate::domain::booking::repositories::member_repository::{MemberRepository, TokenTaken};

#[derive(Clone)]
pub struct PgMemberRepository {
    pool: PgPool,
}

impl PgMemberRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const SELECT_BY_BRANCH: &str = "\
    SELECT id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by \
    FROM members \
    WHERE branch_id = $1 \
    ORDER BY verified_at ASC, id ASC";

const SELECT_BY_BRANCH_AND_ID: &str = "\
    SELECT id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by \
    FROM members \
    WHERE branch_id = $1 AND id = $2";

const SELECT_BY_BRANCH_AND_CUSTOMER: &str = "\
    SELECT id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by \
    FROM members \
    WHERE branch_id = $1 AND customer_id = $2";

const UPSERT_MEMBER: &str = "\
    INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by) \
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
    ON CONFLICT (branch_id, customer_id) DO UPDATE SET \
        name = EXCLUDED.name, \
        phone = EXCLUDED.phone, \
        token_hash = EXCLUDED.token_hash, \
        verified_at = EXCLUDED.verified_at, \
        verified_by = EXCLUDED.verified_by \
    RETURNING id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by";

fn map_member(row: &sqlx::postgres::PgRow) -> AppResult<Member> {
    Ok(Member {
        id: row.try_get("id").map_err(AppError::Database)?,
        branch_id: BranchId::new(
            row.try_get::<String, _>("branch_id")
                .map_err(AppError::Database)?,
        ),
        customer_id: row.try_get("customer_id").map_err(AppError::Database)?,
        name: row.try_get("name").map_err(AppError::Database)?,
        phone: row.try_get("phone").map_err(AppError::Database)?,
        token_hash: row.try_get("token_hash").map_err(AppError::Database)?,
        verified_at: row.try_get("verified_at").map_err(AppError::Database)?,
        verified_by: row.try_get("verified_by").map_err(AppError::Database)?,
    })
}

impl MemberRepository for PgMemberRepository {
    fn list<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Vec<Member>>> {
        Box::pin(async move {
            let rows = sqlx::query(SELECT_BY_BRANCH)
                .bind(branch.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(AppError::Database)?;
            rows.iter().map(map_member).collect()
        })
    }

    fn find<'a>(
        &'a self,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Member>>> {
        Box::pin(async move {
            let row = sqlx::query(SELECT_BY_BRANCH_AND_ID)
                .bind(branch.as_str())
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
            row.as_ref().map(map_member).transpose()
        })
    }

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Member>>> {
        Box::pin(async move {
            let row = sqlx::query(SELECT_BY_BRANCH_AND_ID)
                .bind(branch.as_str())
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            row.as_ref().map(map_member).transpose()
        })
    }

    fn by_customer_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Member>>> {
        Box::pin(async move {
            let row = sqlx::query(SELECT_BY_BRANCH_AND_CUSTOMER)
                .bind(branch.as_str())
                .bind(customer_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            row.as_ref().map(map_member).transpose()
        })
    }

    fn upsert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        member: &'a Member,
    ) -> BoxFuture<'a, AppResult<Result<Member, TokenTaken>>> {
        Box::pin(async move {
            let res = sqlx::query(UPSERT_MEMBER)
                .bind(&member.id)
                .bind(member.branch_id.as_str())
                .bind(&member.customer_id)
                .bind(&member.name)
                .bind(&member.phone)
                .bind(&member.token_hash)
                .bind(member.verified_at)
                .bind(&member.verified_by)
                .fetch_one(&mut **tx)
                .await;

            match res {
                Ok(row) => Ok(Ok(map_member(&row)?)),
                Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                    if db_err.constraint() == Some("members_token_hash_key")
                        || db_err.message().contains("token_hash")
                    {
                        Ok(Err(TokenTaken))
                    } else {
                        Err(AppError::Database(sqlx::Error::Database(db_err)))
                    }
                }
                Err(e) => Err(AppError::Database(e)),
            }
        })
    }
}
