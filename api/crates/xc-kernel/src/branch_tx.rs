//! `BranchTx` — per-branch transaction wrapper.
//!
//! Wraps `pos_infra::Transactor` to take `SELECT … FOR UPDATE` on that
//! branch's `branch_versions` row first, increments the version inside the
//! transaction, and commits.

use futures::future::BoxFuture;
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use sqlx::PgPool;

use crate::ids::BranchId;

/// Per-branch transaction wrapper.
///
/// Ensures sequential execution and concurrency version tracking per branch
/// by taking a row lock (`SELECT ... FOR UPDATE`) on the branch's
/// `branch_versions` record within a single database transaction.
#[derive(Clone)]
pub struct BranchTx {
    transactor: Transactor,
}

impl BranchTx {
    /// Creates a new `BranchTx` wrapping the given `Transactor`.
    pub fn new(transactor: Transactor) -> Self {
        Self { transactor }
    }

    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &PgPool {
        self.transactor.pool()
    }

    /// Runs a closure inside a database transaction scoped to `branch`.
    ///
    /// The transaction ensures the branch's row in `branch_versions` is created
    /// if absent, locks it `FOR UPDATE`, runs `f`, increments the version by 1,
    /// and commits.
    ///
    /// If `f` returns an error, the transaction is rolled back and the original
    /// error is returned.
    pub async fn run<T, F>(&self, branch: &BranchId, f: F) -> AppResult<T>
    where
        T: Send,
        F: for<'a> FnOnce(
                &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
            ) -> BoxFuture<'a, AppResult<T>>
            + Send
            + 'static,
    {
        let branch_id = branch.as_str().to_owned();
        self.transactor
            .run(move |tx| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
                    )
                    .bind(&branch_id)
                    .execute(&mut **tx)
                    .await?;

                    sqlx::query(
                        "SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE",
                    )
                    .bind(&branch_id)
                    .execute(&mut **tx)
                    .await?;

                    let result = f(tx).await?;

                    sqlx::query(
                        "UPDATE branch_versions SET version = version + 1 WHERE branch_id = $1",
                    )
                    .bind(&branch_id)
                    .execute(&mut **tx)
                    .await?;

                    Ok(result)
                })
            })
            .await
    }

    /// Takes the lock of a second branch inside a transaction that
    /// [`BranchTx::run`] started for another one, for a write that depends on
    /// what both branches hold.
    ///
    /// The same as `run` does for its own branch, at once: the branch's row
    /// in `branch_versions` is created if absent, locked `FOR UPDATE` until
    /// the transaction ends, and its version is incremented by 1 — the
    /// increment is part of the transaction, so it is undone with it.
    ///
    /// **Lock order: `MAIN` first, then the other branch — never the reverse.**
    /// A transaction that needs two branch locks is run on `MAIN` and calls
    /// this for the other branch; a transaction run on another branch must
    /// not call this for `MAIN`. With one order there is no cycle to wait in.
    /// Calling it for the branch `run` already locked would count that
    /// version twice: the caller leaves that case out.
    pub async fn lock_also(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await?;

        sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
            .bind(branch.as_str())
            .execute(&mut **tx)
            .await?;

        sqlx::query("UPDATE branch_versions SET version = version + 1 WHERE branch_id = $1")
            .bind(branch.as_str())
            .execute(&mut **tx)
            .await?;

        Ok(())
    }

    /// Returns the current committed version of `branch`.
    ///
    /// Returns `0` if the branch has never been written. Creates no row
    /// and does not wait behind running transactions.
    pub async fn version(&self, branch: &BranchId) -> AppResult<i64> {
        let version: Option<i64> =
            sqlx::query_scalar("SELECT version FROM branch_versions WHERE branch_id = $1")
                .bind(branch.as_str())
                .fetch_optional(self.transactor.pool())
                .await?;

        Ok(version.unwrap_or(0))
    }
}
