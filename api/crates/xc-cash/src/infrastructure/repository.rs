//! PostgreSQL implementation of CashRepository.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::count::{CashCount, CountKind, CountLine};
use crate::domain::repository::{CashRepository, CountTotal};

#[derive(Clone)]
pub struct PgCashRepository {
    pool: PgPool,
}

impl PgCashRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[derive(FromRow)]
struct CashCountDbRow {
    id: uuid::Uuid,
    organization_id: uuid::Uuid,
    branch_id: String,
    shift_id: Option<String>,
    kind: String,
    counted_by: uuid::Uuid,
    counted_at: DateTime<Utc>,
    verified_by: Option<uuid::Uuid>,
    verified_at: Option<DateTime<Utc>>,
    note: Option<String>,
    idempotency_key: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl CashCountDbRow {
    fn into_domain(self) -> CashCount {
        CashCount {
            id: self.id.to_string(),
            organization_id: self.organization_id.to_string(),
            branch_id: BranchId::new(self.branch_id),
            shift_id: self.shift_id,
            kind: CountKind::parse(&self.kind).unwrap_or(CountKind::Open),
            counted_by: self.counted_by.to_string(),
            counted_at: self.counted_at,
            verified_by: self.verified_by.map(|u| u.to_string()),
            verified_at: self.verified_at,
            note: self.note,
            idempotency_key: self.idempotency_key,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

impl CashRepository for PgCashRepository {
    fn find_count_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CashCount>>> {
        Box::pin(async move {
            let row: Option<CashCountDbRow> = sqlx::query_as(
                "SELECT id, organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
                        verified_by, verified_at, note, idempotency_key, created_at, updated_at \
                 FROM cash_counts WHERE id = $1::uuid",
            )
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;

            Ok(row.map(CashCountDbRow::into_domain))
        })
    }

    fn find_count_by_idempotency_key<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CashCount>>> {
        Box::pin(async move {
            let row: Option<CashCountDbRow> = sqlx::query_as(
                "SELECT id, organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
                        verified_by, verified_at, note, idempotency_key, created_at, updated_at \
                 FROM cash_counts WHERE branch_id = $1 AND idempotency_key = $2",
            )
            .bind(branch.as_str())
            .bind(key)
            .fetch_optional(&mut **tx)
            .await?;

            Ok(row.map(CashCountDbRow::into_domain))
        })
    }

    fn insert_count<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count: &'a CashCount,
        lines: &'a [CountLine],
        totals: &'a [CountTotal],
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO cash_counts (id, organization_id, branch_id, shift_id, kind, \
                        counted_by, counted_at, note, idempotency_key, created_at, updated_at) \
                 VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6::uuid, $7, $8, $9, $10, $10)",
            )
            .bind(&count.id)
            .bind(&count.organization_id)
            .bind(count.branch_id.as_str())
            .bind(&count.shift_id)
            .bind(count.kind.as_str())
            .bind(&count.counted_by)
            .bind(count.counted_at)
            .bind(&count.note)
            .bind(&count.idempotency_key)
            .bind(count.created_at)
            .execute(&mut **tx)
            .await?;

            for line in lines {
                sqlx::query(
                    "INSERT INTO cash_count_lines (count_id, organization_id, currency, denomination_id, qty, created_at) \
                     VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
                )
                .bind(&count.id)
                .bind(&count.organization_id)
                .bind(&line.currency)
                .bind(line.denomination_id)
                .bind(line.qty)
                .bind(count.created_at)
                .execute(&mut **tx)
                .await?;
            }

            for total in totals {
                sqlx::query(
                    "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference, created_at) \
                     VALUES ($1::uuid, $2::uuid, $3, $4::numeric, $5::numeric, $6::numeric, $7)",
                )
                .bind(&count.id)
                .bind(&count.organization_id)
                .bind(&total.currency)
                .bind(total.counted)
                .bind(total.expected)
                .bind(total.difference)
                .bind(count.created_at)
                .execute(&mut **tx)
                .await?;
            }

            Ok(())
        })
    }

    fn is_newest_count<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count: &'a CashCount,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let newer_count: i64 = if let Some(ref s_id) = count.shift_id {
                sqlx::query_scalar(
                    "SELECT count(*) FROM cash_counts \
                     WHERE shift_id = $1 AND kind = $2 AND id <> $3::uuid \
                       AND (created_at > $4 OR counted_at > $5)",
                )
                .bind(s_id)
                .bind(count.kind.as_str())
                .bind(&count.id)
                .bind(count.created_at)
                .bind(count.counted_at)
                .fetch_one(&mut **tx)
                .await?
            } else {
                sqlx::query_scalar(
                    "SELECT count(*) FROM cash_counts \
                     WHERE branch_id = $1 AND counted_by = $2::uuid AND kind = $3 \
                       AND shift_id IS NULL AND id <> $4::uuid \
                       AND (created_at > $5 OR counted_at > $6)",
                )
                .bind(count.branch_id.as_str())
                .bind(&count.counted_by)
                .bind(count.kind.as_str())
                .bind(&count.id)
                .bind(count.created_at)
                .bind(count.counted_at)
                .fetch_one(&mut **tx)
                .await?
            };

            Ok(newer_count == 0)
        })
    }

    fn get_count_totals<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Vec<CountTotal>>> {
        Box::pin(async move {
            let rows: Vec<(String, Decimal, Option<Decimal>, Option<Decimal>)> = sqlx::query_as(
                "SELECT currency, counted, expected, difference \
                 FROM cash_count_totals WHERE count_id = $1::uuid",
            )
            .bind(count_id)
            .fetch_all(&mut **tx)
            .await?;

            let totals = rows
                .into_iter()
                .map(|(currency, counted, expected, difference)| CountTotal {
                    currency,
                    counted,
                    expected,
                    difference,
                })
                .collect();

            Ok(totals)
        })
    }

    fn update_count_verified<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        count_id: &'a str,
        verified_by: &'a str,
        verified_at: DateTime<Utc>,
        shift_id: Option<&'a str>,
        totals: &'a [CountTotal],
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if let Some(s_id) = shift_id {
                sqlx::query(
                    "UPDATE cash_counts \
                     SET verified_by = $1::uuid, verified_at = $2, shift_id = $3, updated_at = $2 \
                     WHERE id = $4::uuid",
                )
                .bind(verified_by)
                .bind(verified_at)
                .bind(s_id)
                .bind(count_id)
                .execute(&mut **tx)
                .await?;
            } else {
                sqlx::query(
                    "UPDATE cash_counts \
                     SET verified_by = $1::uuid, verified_at = $2, updated_at = $2 \
                     WHERE id = $3::uuid",
                )
                .bind(verified_by)
                .bind(verified_at)
                .bind(count_id)
                .execute(&mut **tx)
                .await?;
            }

            for total in totals {
                sqlx::query(
                    "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference, created_at) \
                     VALUES ($1::uuid, (SELECT organization_id FROM cash_counts WHERE id = $1::uuid), $2, $3::numeric, $4::numeric, $5::numeric, $6) \
                     ON CONFLICT (count_id, currency) DO UPDATE SET expected = $4::numeric, difference = $5::numeric, counted = $3::numeric",
                )
                .bind(count_id)
                .bind(&total.currency)
                .bind(total.counted)
                .bind(total.expected)
                .bind(total.difference)
                .bind(verified_at)
                .execute(&mut **tx)
                .await?;
            }

            Ok(())
        })
    }
}
