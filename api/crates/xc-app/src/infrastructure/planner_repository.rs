//! `PgPlannerRepository`: `planner_drafts` (migration `20261106100000_customer_app.sql`).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

use crate::domain::app_error::CustomerAppError;
use crate::domain::app_session::SessionHash;
use crate::domain::owner::DraftOwner;
use crate::domain::planner::planner_draft::{DraftAmount, DraftPlan, PlannerDraft};
use crate::domain::planner::repository::PlannerRepository;

const COLUMNS: &str = "id, organization_id, customer_id, session_hash, branch_id, \
     currency_code::text AS currency_code, customer_side, quantity, budget_thb, version, \
     expires_at, created_at, updated_at";

pub struct PgPlannerRepository {
    pool: PgPool,
}

impl PgPlannerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn corrupt(what: &str) -> CustomerAppError {
    CustomerAppError::Server(format!("planner_drafts: {what}"))
}

fn draft_of(row: &PgRow) -> Result<PlannerDraft, CustomerAppError> {
    let customer_id: Option<String> = row.try_get("customer_id")?;
    let session_hash: Option<String> = row.try_get("session_hash")?;
    let owner = match (customer_id, session_hash) {
        (Some(customer), None) => DraftOwner::Customer(customer),
        (None, Some(hash)) => DraftOwner::Session(SessionHash::from_stored(hash)),
        _ => return Err(corrupt("not exactly one owner")),
    };
    let currency: String = row.try_get("currency_code")?;
    let side: String = row.try_get("customer_side")?;
    let quantity: Option<Decimal> = row.try_get("quantity")?;
    let budget: Option<Decimal> = row.try_get("budget_thb")?;
    let amount = match (quantity, budget) {
        (Some(q), None) => DraftAmount::Quantity(ForeignAmount::from_decimal(q)),
        (None, Some(b)) => DraftAmount::BudgetThb(
            Money::parse(&b.round_dp(2).to_string()).map_err(|_| corrupt("budget_thb"))?,
        ),
        _ => return Err(corrupt("not exactly one amount")),
    };
    let branch: Option<String> = row.try_get("branch_id")?;
    Ok(PlannerDraft {
        id: row.try_get("id")?,
        organization_id: row.try_get("organization_id")?,
        owner,
        plan: DraftPlan {
            branch_id: branch.map(BranchId::new),
            currency: CurrencyCode::parse(currency.trim()).map_err(|_| corrupt("currency"))?,
            customer_side: CustomerSide::parse(&side).ok_or_else(|| corrupt("customer_side"))?,
            amount,
        },
        version: row.try_get("version")?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// `(quantity, budget_thb)` as stored: exactly one is set.
fn amount_columns(amount: &DraftAmount) -> (Option<Decimal>, Option<Decimal>) {
    match amount {
        DraftAmount::Quantity(q) => (Some(q.value()), None),
        DraftAmount::BudgetThb(m) => (None, Some(Decimal::new(m.minor(), 2))),
    }
}

impl PlannerRepository for PgPlannerRepository {
    fn get_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<PlannerDraft>, CustomerAppError>> {
        Box::pin(async move {
            let sql = format!("SELECT {COLUMNS} FROM planner_drafts WHERE id = $1");
            let row = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(id)
                .fetch_optional(&mut **tx)
                .await?;
            row.as_ref().map(draft_of).transpose()
        })
    }

    fn list<'a>(
        &'a self,
        owner: &'a DraftOwner,
    ) -> BoxFuture<'a, Result<Vec<PlannerDraft>, CustomerAppError>> {
        Box::pin(async move {
            let (column, value) = match owner {
                DraftOwner::Customer(id) => ("customer_id", id.as_str()),
                DraftOwner::Session(hash) => ("session_hash", hash.as_str()),
            };
            let sql = format!(
                "SELECT {COLUMNS} FROM planner_drafts WHERE {column} = $1 \
                 ORDER BY created_at DESC, id DESC"
            );
            let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(value)
                .fetch_all(&self.pool)
                .await?;
            rows.iter().map(draft_of).collect()
        })
    }

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        draft: &'a PlannerDraft,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>> {
        Box::pin(async move {
            let (customer_id, session_hash) = match &draft.owner {
                DraftOwner::Customer(id) => (Some(id.as_str()), None),
                DraftOwner::Session(hash) => (None, Some(hash.as_str())),
            };
            let (quantity, budget) = amount_columns(&draft.plan.amount);
            sqlx::query(
                "INSERT INTO planner_drafts (id, organization_id, customer_id, session_hash, \
                     branch_id, currency_code, customer_side, quantity, budget_thb, version, \
                     expires_at, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(draft.id)
            .bind(draft.organization_id)
            .bind(customer_id)
            .bind(session_hash)
            .bind(draft.plan.branch_id.as_ref().map(|b| b.as_str()))
            .bind(draft.plan.currency.as_str())
            .bind(draft.plan.customer_side.as_str())
            .bind(quantity)
            .bind(budget)
            .bind(draft.version)
            .bind(draft.expires_at)
            .bind(draft.created_at)
            .bind(draft.updated_at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    }

    fn update_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        draft: &'a PlannerDraft,
        expected: i32,
    ) -> BoxFuture<'a, Result<bool, CustomerAppError>> {
        Box::pin(async move {
            let (quantity, budget) = amount_columns(&draft.plan.amount);
            let done = sqlx::query(
                "UPDATE planner_drafts SET branch_id = $3, currency_code = $4, \
                     customer_side = $5, quantity = $6, budget_thb = $7, \
                     version = version + 1, updated_at = $8 \
                 WHERE id = $1 AND version = $2",
            )
            .bind(draft.id)
            .bind(expected)
            .bind(draft.plan.branch_id.as_ref().map(|b| b.as_str()))
            .bind(draft.plan.currency.as_str())
            .bind(draft.plan.customer_side.as_str())
            .bind(quantity)
            .bind(budget)
            .bind(draft.updated_at)
            .execute(&mut **tx)
            .await?;
            Ok(done.rows_affected() == 1)
        })
    }

    fn delete_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> BoxFuture<'a, Result<(), CustomerAppError>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM planner_drafts WHERE id = $1")
                .bind(id)
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    }

    fn claim_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        session: &'a SessionHash,
        customer_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<u64, CustomerAppError>> {
        Box::pin(async move {
            let moved = sqlx::query(
                "UPDATE planner_drafts SET customer_id = $2, session_hash = NULL, updated_at = $3 \
                 WHERE session_hash = $1",
            )
            .bind(session.as_str())
            .bind(customer_id)
            .bind(now)
            .execute(&mut **tx)
            .await?;
            Ok(moved.rows_affected())
        })
    }
}
