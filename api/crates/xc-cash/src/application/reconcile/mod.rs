//! The ledger reconcile core (LG-7.0 #1054; design `docs/design/blueprint/ledger-inventory.md` §6 and
//! §6.1). Checks live one per file in `src/application/reconcile/checks/`, `build.rs` lists them,
//! and adding a check means adding one file; the core runs them in one read-only snapshot.

use std::sync::Arc;

use chrono::NaiveDate;
use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::ids::BranchId;

/// What a reconcile run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileScope {
    pub organization_id: String,
    pub branch: Option<BranchId>,
    pub period: Option<(NaiveDate, NaiveDate)>,
}

/// One row a check found wrong: `key` names it (a posting id, `"<branch>:<currency>"`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offender {
    pub key: String,
    pub detail: String,
}

/// One check's outcome: `ok` exactly when it found no offender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub id: u8,
    pub ok: bool,
    pub offenders: Vec<Offender>,
}

/// Every registered check's outcome, in id order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub checks: Vec<CheckResult>,
}

/// One of the five checks of design §6. Each is one SQL statement (`pub const QUERY`) in its own file.
pub trait ReconcileCheck: Send + Sync {
    /// 1..=5, the numbers of design §6.
    fn id(&self) -> u8;

    /// The offender rows, read inside the run's transaction. Empty = passed.
    fn run_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>>;
}

include!(concat!(env!("OUT_DIR"), "/reconcile_checks.rs"));

/// Runs registered checks.
pub struct Reconcile {
    checks: Vec<Arc<dyn ReconcileCheck>>,
}

impl Reconcile {
    pub fn new(checks: Vec<Arc<dyn ReconcileCheck>>) -> Self {
        Self { checks }
    }

    /// The checks of design §6, discovered at build time from `checks/`.
    pub fn standard() -> Self {
        Self::new(standard_checks())
    }

    /// Runs every check in one `REPEATABLE READ READ ONLY` transaction, ended with a rollback.
    pub async fn run(&self, pool: &PgPool, scope: &ReconcileScope) -> AppResult<Report> {
        let mut tx = pool.begin().await.map_err(pos_error::AppError::Database)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(pos_error::AppError::Database)?;

        let mut checks = self.checks.clone();
        checks.sort_by_key(|c| c.id());

        let mut results = Vec::with_capacity(checks.len());
        for check in checks {
            let offenders = check.run_in(&mut tx, scope).await?;
            let ok = offenders.is_empty();
            results.push(CheckResult {
                id: check.id(),
                ok,
                offenders,
            });
        }

        let _ = tx.rollback().await;
        Ok(Report { checks: results })
    }
}
