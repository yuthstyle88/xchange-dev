//! RBAC C3 tests (#3147): `PgCustodyHolds::holds_cash_or_pending_handover`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::user::CustodyHolds;
use xc_cash::infrastructure::custody_holds::PgCustodyHolds;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Db {
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a database ending in `_test`.")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("rbac_c3");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to test db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        Db {
            pool,
            admin,
            schema,
        }
    }

    async fn close(self) {
        self.pool.close().await;
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await;
        self.admin.close().await;
    }

    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("user"))
        .fetch_one(&self.pool)
        .await
        .expect("insert user")
    }

    async fn branch(&self) -> String {
        let id = unique("br");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert branch");
        id
    }

    async fn shift(&self, branch_id: &str, teller_id: &str, status: &str) -> String {
        let id = unique("sh");
        sqlx::query(
            "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2::uuid, $3, $4, now(), $5, $6, $5::uuid)",
        )
        .bind(&id)
        .bind(ORG)
        .bind(branch_id)
        .bind(status)
        .bind(teller_id)
        .bind(unique("req"))
        .execute(&self.pool)
        .await
        .expect("insert shift");
        id
    }

    async fn handover(
        &self,
        branch_id: &str,
        from_shift_id: &str,
        counted_by: &str,
        receiver_id: &str,
        status: &str,
    ) {
        let count_id: String = sqlx::query_scalar(
            "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES ($1::uuid, $2, $3, 'HANDOVER_OUT', $4::uuid, now(), $5) RETURNING id::text",
        )
        .bind(ORG)
        .bind(branch_id)
        .bind(from_shift_id)
        .bind(counted_by)
        .bind(unique("cnt"))
        .fetch_one(&self.pool)
        .await
        .expect("insert cash_count");

        sqlx::query(
            "INSERT INTO handovers (organization_id, branch_id, from_shift_id, count_id, receiver_id, status) \
             VALUES ($1::uuid, $2, $3, $4::uuid, $5::uuid, $6)",
        )
        .bind(ORG)
        .bind(branch_id)
        .bind(from_shift_id)
        .bind(&count_id)
        .bind(receiver_id)
        .bind(status)
        .execute(&self.pool)
        .await
        .expect("insert handover");
    }

    async fn transfer(
        &self,
        branch_id: &str,
        created_by: &str,
        dispatched_by: Option<&str>,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, status, created_by, dispatched_by, idempotency_key) \
             VALUES ($1::uuid, 'EXTERNAL_OUT', $2, $3, $4::uuid, $5::uuid, $6)",
        )
        .bind(ORG)
        .bind(branch_id)
        .bind(status)
        .bind(created_by)
        .bind(dispatched_by)
        .bind(unique("tf"))
        .execute(&self.pool)
        .await
        .expect("insert transfer");
    }

    async fn holds(&self, user_id: &str) -> bool {
        let mut tx = self.pool.begin().await.expect("begin tx");
        let ans = PgCustodyHolds
            .holds_cash_or_pending_handover(&mut tx, user_id)
            .await
            .expect("holds_cash_or_pending_handover");
        tx.commit().await.expect("commit tx");
        ans
    }
}

#[tokio::test]
async fn no_rows_for_user_returns_false() {
    let db = Db::fresh().await;
    let user = db.user().await;
    assert!(!db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn open_shift_with_teller_id_user_returns_true() {
    let db = Db::fresh().await;
    let user = db.user().await;
    let branch = db.branch().await;
    db.shift(&branch, &user, "OPEN").await;
    assert!(db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn only_closed_shifts_returns_false() {
    let db = Db::fresh().await;
    let user = db.user().await;
    let branch = db.branch().await;
    db.shift(&branch, &user, "CLOSED").await;
    assert!(!db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn pending_handover_from_users_shift_returns_true() {
    let db = Db::fresh().await;
    let giver = db.user().await;
    let receiver = db.user().await;
    let branch = db.branch().await;
    let shift = db.shift(&branch, &giver, "CLOSED").await;
    db.handover(&branch, &shift, &giver, &receiver, "PENDING")
        .await;
    assert!(db.holds(&giver).await);
    db.close().await;
}

#[tokio::test]
async fn accepted_or_rejected_handover_from_users_shift_returns_false() {
    let db = Db::fresh().await;
    let giver = db.user().await;
    let receiver = db.user().await;
    let branch = db.branch().await;
    let shift1 = db.shift(&branch, &giver, "CLOSED").await;
    let shift2 = db.shift(&branch, &giver, "CLOSED").await;
    db.handover(&branch, &shift1, &giver, &receiver, "ACCEPTED")
        .await;
    db.handover(&branch, &shift2, &giver, &receiver, "REJECTED")
        .await;
    assert!(!db.holds(&giver).await);
    assert!(!db.holds(&receiver).await);
    db.close().await;
}

#[tokio::test]
async fn pending_handover_with_receiver_id_user_returns_true() {
    let db = Db::fresh().await;
    let giver = db.user().await;
    let receiver = db.user().await;
    let branch = db.branch().await;
    let shift = db.shift(&branch, &giver, "CLOSED").await;
    db.handover(&branch, &shift, &giver, &receiver, "PENDING")
        .await;
    assert!(db.holds(&receiver).await);
    db.close().await;
}

#[tokio::test]
async fn dispatched_transfer_with_dispatched_by_user_returns_true() {
    let db = Db::fresh().await;
    let user = db.user().await;
    let branch = db.branch().await;
    db.transfer(&branch, &user, Some(&user), "DISPATCHED").await;
    assert!(db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn received_or_reconciled_transfer_they_dispatched_returns_false() {
    let db = Db::fresh().await;
    let user = db.user().await;
    let branch = db.branch().await;
    db.transfer(&branch, &user, Some(&user), "RECEIVED").await;
    db.transfer(&branch, &user, Some(&user), "RECONCILED").await;
    assert!(!db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn dispatched_transfer_dispatched_by_someone_else_returns_false() {
    let db = Db::fresh().await;
    let user = db.user().await;
    let other = db.user().await;
    let branch = db.branch().await;
    db.transfer(&branch, &user, Some(&other), "DISPATCHED")
        .await;
    assert!(!db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn another_users_open_shift_returns_false() {
    let db = Db::fresh().await;
    let user = db.user().await;
    let other = db.user().await;
    let branch = db.branch().await;
    db.shift(&branch, &other, "OPEN").await;
    assert!(!db.holds(&user).await);
    db.close().await;
}

#[tokio::test]
async fn malformed_user_id_returns_ok_false() {
    let db = Db::fresh().await;
    let mut tx = db.pool.begin().await.expect("begin tx");
    let ans = PgCustodyHolds
        .holds_cash_or_pending_handover(&mut tx, "not-a-uuid")
        .await
        .expect("malformed user_id must be Ok(false)");
    assert!(!ans);
    let probe: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut *tx)
        .await
        .expect("transaction stays usable");
    assert_eq!(probe, 1);
    tx.commit().await.expect("commit tx");
    db.close().await;
}
