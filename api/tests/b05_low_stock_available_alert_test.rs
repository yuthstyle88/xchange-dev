//! Tests for B05 SR-5 (#3497): A01 low-stock alert compares `branch_stock_floors.min_quantity`
//! with `available = on_hand - reserved` (`docs/design/blueprint/stock-reservation.md` D-13, S17).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_kernel::ids::BranchId;
use xchange::application::executive::ports::{Alert, AlertFacts, AlertKind};
use xchange::infrastructure::executive::alert_facts::PgAlertFacts;
use xchange::infrastructure::executive::ledger_cash_facts::LedgerCashFacts;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

struct Db {
    pool: PgPool,
    admin: PgPool,
    schema: String,
    branch: BranchId,
    user: String,
}

impl Db {
    async fn fresh(floor: &str, on_hand: &str) -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set; point it at a database ending in `_test`")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("b05_sr5");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect admin");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("apply migrations");

        let branch_str = unique("br");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch_str)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("insert branch");

        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, $3, 'ACTIVE', 'test')",
        )
        .bind(user_id)
        .bind(ORG)
        .bind(unique("user"))
        .execute(&pool)
        .await
        .expect("insert user");

        sqlx::query(
            "INSERT INTO branch_stock_floors (organization_id, branch_id, currency, min_quantity, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 'USD', $3::numeric, $4, now())",
        )
        .bind(ORG)
        .bind(&branch_str)
        .bind(floor)
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("insert floor");

        sqlx::query(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ($1::uuid, $2, 'USD', $3::numeric, $3::numeric * 35)",
        )
        .bind(ORG)
        .bind(&branch_str)
        .bind(on_hand)
        .execute(&pool)
        .await
        .expect("insert position");

        Self {
            pool,
            admin,
            schema,
            branch: BranchId::new(branch_str),
            user: user_id.to_string(),
        }
    }

    async fn reserve_usd(&self, quantity: &str, expires_at: DateTime<Utc>) {
        let (quote_id, pub_id, req_id, rate_line_id, line_id) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
             VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4, 'test', '1', '{}'::jsonb, $5, now(), \
             'APPROVED', $6)",
        )
        .bind(req_id)
        .bind(ORG)
        .bind(self.branch.as_str())
        .bind(pub_id)
        .bind(&self.user)
        .bind(format!("k-{req_id}"))
        .execute(&self.pool)
        .await
        .expect("insert approval request");

        sqlx::query(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, \
             draft_id, published_by) VALUES ($1, $2, 1, now(), $3, NULL, $4)",
        )
        .bind(pub_id)
        .bind(self.branch.as_str())
        .bind(req_id)
        .bind(&self.user)
        .execute(&self.pool)
        .await
        .expect("insert publication");

        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
             unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES ($1, $2, 'USD', 'ALL', 1.0, 34.0, 35.0, '{}'::jsonb)",
        )
        .bind(rate_line_id)
        .bind(pub_id)
        .execute(&self.pool)
        .await
        .expect("insert rate line");

        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, 'INDICATIVE', 'DESK', 'ACTIVE', $4, $5, 'test', now(), now())",
        )
        .bind(quote_id)
        .bind(ORG)
        .bind(self.branch.as_str())
        .bind(pub_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .expect("insert quote");

        sqlx::query(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, shop_side, \
             currency, denomination_group, quantity, unit_size, base_rate, final_rate, thb_amount, \
             fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 'SELL', 'USD', 'ALL', $5::numeric, 1.0, 35.0, 35.0, 0, 0, 0, 1, now(), now())",
        )
        .bind(line_id)
        .bind(ORG)
        .bind(quote_id)
        .bind(rate_line_id)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("insert quote line");

        let mut tx = self.pool.begin().await.expect("begin tx");
        PgStockReservations::new()
            .reserve_in(
                &mut tx,
                &ReserveLine {
                    quote_line_id: line_id,
                    denomination_id: None,
                },
                expires_at - Duration::seconds(300),
            )
            .await
            .expect("reserve_in");
        tx.commit().await.expect("commit reserve");
    }

    async fn low_stock_alerts(&self) -> Vec<Alert> {
        let cash = Arc::new(LedgerCashFacts::new(self.pool.clone()));
        let facts = PgAlertFacts::new(self.pool.clone(), cash);
        facts
            .alerts(std::slice::from_ref(&self.branch))
            .await
            .expect("facts.alerts")
            .into_iter()
            .filter(|a| a.kind == AlertKind::LowStock)
            .collect()
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
}

/// §9 Row 1 (S17): floor 50, on hand 80, active hold 40 -> alert (available 40 < 50).
#[tokio::test]
async fn row_1_s17_floor_50_on_hand_80_active_hold_40_alerts() {
    let db = Db::fresh("50.0000", "80.0000").await;
    db.reserve_usd("40.0000", Utc::now() + Duration::seconds(300))
        .await;

    let alerts = db.low_stock_alerts().await;
    assert_eq!(alerts.len(), 1, "expected 1 lowStock alert: {alerts:?}");
    assert_eq!(alerts[0].branch, db.branch);
    assert_eq!(
        alerts[0].detail,
        "USD: available 40.0000 (on hand 80.0000, reserved 40.0000), floor 50.0000"
    );
    db.close().await;
}

/// §9 Row 2: floor 50, on hand 80, no hold -> no alert.
#[tokio::test]
async fn row_2_floor_50_on_hand_80_no_hold_does_not_alert() {
    let db = Db::fresh("50.0000", "80.0000").await;

    let alerts = db.low_stock_alerts().await;
    assert!(alerts.is_empty(), "expected no lowStock alert: {alerts:?}");
    db.close().await;
}

/// §9 Row 3: floor 50, on hand 80, hold 40 expired -> no alert (expired holds nothing).
#[tokio::test]
async fn row_3_floor_50_on_hand_80_expired_hold_40_does_not_alert() {
    let db = Db::fresh("50.0000", "80.0000").await;
    db.reserve_usd("40.0000", Utc::now() - Duration::seconds(60))
        .await;

    let alerts = db.low_stock_alerts().await;
    assert!(
        alerts.is_empty(),
        "expired hold must not reduce available below floor: {alerts:?}"
    );
    db.close().await;
}

/// §9 Row 4: floor 50, on hand 40, no hold -> alert.
#[tokio::test]
async fn row_4_floor_50_on_hand_40_no_hold_alerts() {
    let db = Db::fresh("50.0000", "40.0000").await;

    let alerts = db.low_stock_alerts().await;
    assert_eq!(alerts.len(), 1, "expected 1 lowStock alert: {alerts:?}");
    assert_eq!(alerts[0].branch, db.branch);
    assert!(
        alerts[0]
            .detail
            .starts_with("USD: available 40.0000 (on hand 40.0000, reserved 0"),
        "unexpected detail: {}",
        alerts[0].detail
    );
    assert!(
        alerts[0].detail.ends_with("), floor 50.0000"),
        "unexpected detail: {}",
        alerts[0].detail
    );
    db.close().await;
}
