//! FN-1 #1068: `reconciliation_certifications` (EX-2's table, append-only by trigger). A certification is only ever
//! appended; a later change to the figures makes it stale, never deletes it.

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::ids::BranchId;

/// One stored certification.
#[derive(Debug, Clone, PartialEq)]
pub struct Certification {
    pub id: Uuid,
    pub branch: BranchId,
    pub business_date: NaiveDate,
    pub figures_hash: String,
    pub certified_by: Uuid,
    pub certified_at: DateTime<Utc>,
    pub note: Option<String>,
}

/// What `append_in` writes: the certification and the figures it certified.
#[derive(Debug, Clone)]
pub struct NewCertification<'a> {
    pub branch: &'a BranchId,
    pub business_date: NaiveDate,
    pub figures_hash: &'a str,
    pub figures: Value,
    pub certified_by: Uuid,
    pub certified_at: DateTime<Utc>,
    pub note: Option<&'a str>,
}

type Row = (
    Uuid,
    String,
    NaiveDate,
    String,
    Uuid,
    DateTime<Utc>,
    Option<String>,
);

fn certification(
    (id, branch, business_date, figures_hash, certified_by, certified_at, note): Row,
) -> Certification {
    Certification {
        id,
        branch: BranchId::new(branch),
        business_date,
        figures_hash,
        certified_by,
        certified_at,
        note,
    }
}

pub struct PgCertificationRepository {
    pool: PgPool,
}

impl PgCertificationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Appends one certification in the caller's transaction; the organisation is the branch's.
    pub async fn append_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        new: NewCertification<'_>,
    ) -> AppResult<Certification> {
        let row: Row = sqlx::query_as(
            "INSERT INTO reconciliation_certifications \
               (id, organization_id, branch_id, business_date, figures_hash, figures, certified_by, certified_at, note) \
             SELECT $1, b.organization_id, b.id, $3, $4, $5, $6, $7, $8 FROM branches b WHERE b.id = $2 \
             RETURNING id, branch_id, business_date, figures_hash, certified_by, certified_at, note",
        )
        .bind(Uuid::new_v4())
        .bind(new.branch.as_str())
        .bind(new.business_date)
        .bind(new.figures_hash)
        .bind(new.figures)
        .bind(new.certified_by)
        .bind(new.certified_at)
        .bind(new.note)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(certification(row))
    }

    /// The certifications of `branch` on `business_date`, latest first.
    pub async fn for_day(
        &self,
        branch: &BranchId,
        business_date: NaiveDate,
    ) -> AppResult<Vec<Certification>> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, branch_id, business_date, figures_hash, certified_by, certified_at, note \
               FROM reconciliation_certifications \
              WHERE branch_id = $1 AND business_date = $2 \
              ORDER BY certified_at DESC, id DESC",
        )
        .bind(branch.as_str())
        .bind(business_date)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(rows.into_iter().map(certification).collect())
    }
}
