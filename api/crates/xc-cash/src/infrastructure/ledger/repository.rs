use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::ledger::account::{AccountKind, CashAccountId};
use crate::domain::ledger::posting::{Posting, PostingId};
use crate::domain::ledger::repository::LedgerRepository;

/// The PostgreSQL implementation of `LedgerRepository`.
pub struct PgLedgerRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgLedgerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl LedgerRepository for PgLedgerRepository {
    fn account_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        branch: Option<&'a BranchId>,
        currency: CurrencyCode,
        kind: AccountKind,
        shift: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<CashAccountId>> {
        Box::pin(async move {
            let new_id = Uuid::new_v4();
            let branch_str = branch.map(|b| b.as_str());
            let row = sqlx::query(
                "INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, created_at) \
                 VALUES ($1, $2::uuid, $3, $4, $5, $6, now()) \
                 ON CONFLICT (organization_id, branch_id, currency, kind, shift_id) DO NOTHING \
                 RETURNING id",
            )
            .bind(new_id)
            .bind(organization_id)
            .bind(branch_str)
            .bind(currency.as_str())
            .bind(kind.as_str())
            .bind(shift)
            .fetch_optional(&mut **tx)
            .await?;

            if let Some(r) = row {
                let id: Uuid = r.try_get("id")?;
                return Ok(CashAccountId(id));
            }

            let existing_id: Uuid = sqlx::query_scalar(
                "SELECT id FROM cash_accounts \
                 WHERE organization_id = $1::uuid \
                   AND branch_id IS NOT DISTINCT FROM $2 \
                   AND currency = $3 \
                   AND kind = $4 \
                   AND shift_id IS NOT DISTINCT FROM $5",
            )
            .bind(organization_id)
            .bind(branch_str)
            .bind(currency.as_str())
            .bind(kind.as_str())
            .bind(shift)
            .fetch_one(&mut **tx)
            .await?;

            Ok(CashAccountId(existing_id))
        })
    }

    fn insert_posting_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        posting: &'a Posting,
    ) -> BoxFuture<'a, AppResult<PostingId>> {
        Box::pin(async move {
            let new_id = Uuid::new_v4();
            let row = sqlx::query(
                "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
                 VALUES ($1, $2::uuid, $3, $4, $5, $6, 'POSTED', $7, $8) \
                 ON CONFLICT (organization_id, source_type, source_id, source_key) DO NOTHING \
                 RETURNING id",
            )
            .bind(new_id)
            .bind(&posting.organization_id)
            .bind(posting.branch.as_str())
            .bind(posting.source.source_type.as_str())
            .bind(&posting.source.source_id)
            .bind(&posting.source.source_key)
            .bind(posting.posted_at)
            .bind(&posting.posted_by)
            .fetch_optional(&mut **tx)
            .await?;

            let posting_id = if let Some(r) = row {
                let id: Uuid = r.try_get("id")?;
                for entry in &posting.entries {
                    sqlx::query(
                        "INSERT INTO ledger_entries (posting_id, account_id, currency, quantity_signed, valuation_thb, created_at) \
                         VALUES ($1, $2, $3, $4, $5, $6)",
                    )
                    .bind(id)
                    .bind(entry.account.0)
                    .bind(entry.currency.as_str())
                    .bind(entry.quantity_signed)
                    .bind(entry.valuation_thb)
                    .bind(posting.posted_at)
                    .execute(&mut **tx)
                    .await?;
                }
                id
            } else {
                let existing_id: Uuid = sqlx::query_scalar(
                    "SELECT id FROM ledger_postings \
                     WHERE organization_id = $1::uuid \
                       AND source_type = $2 \
                       AND source_id = $3 \
                       AND source_key = $4",
                )
                .bind(&posting.organization_id)
                .bind(posting.source.source_type.as_str())
                .bind(&posting.source.source_id)
                .bind(&posting.source.source_key)
                .fetch_one(&mut **tx)
                .await?;
                existing_id
            };

            Ok(PostingId(posting_id))
        })
    }

    fn balance_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        account: CashAccountId,
    ) -> BoxFuture<'a, AppResult<Decimal>> {
        Box::pin(async move {
            let balance: Decimal = sqlx::query_scalar(
                "SELECT COALESCE(SUM(quantity_signed), 0) FROM ledger_entries WHERE account_id = $1",
            )
            .bind(account.0)
            .fetch_one(&mut **tx)
            .await?;

            Ok(balance)
        })
    }
}
