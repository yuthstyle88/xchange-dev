//! Ticket CS-7 row C8: an OPEN count inserted without a template is verified -> the shift opens with `template_id` NULL.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::CashCountService;
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn bkk(d: u32, h: u32, m: u32) -> DateTime<Utc> {
    FixedOffset::east_opt(7 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, d, h, m, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

fn nov(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

async fn person(pool: &PgPool, name: &str, roles: &[Role], branch: &BranchId) -> Actor {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert a user");
    for role in roles {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.as_str())
        .execute(pool)
        .await
        .expect("give the user a role");
    }
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        &id,
        ORG,
        roles
            .iter()
            .map(|r| ScopedRole::new(*r, Some(branch.clone()), from, None))
            .collect(),
        None,
    )
}

#[tokio::test]
async fn c8_an_open_count_inserted_without_a_template_opens_the_shift_with_template_id_null() {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| panic!("TEST_DATABASE_URL is not set; these tests do not skip"));
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(options.get_database().unwrap_or("").ends_with("_test"));
    let schema = unique("cs7_c8");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("connect");
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("schema");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    Migrator::new(dir.as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");

    let branch_id = unique("main");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&branch_id)
        .execute(&pool)
        .await
        .expect("insert a branch");
    let main = BranchId::new(branch_id);

    let mut tx = pool.begin().await.unwrap();
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(main.clone()),
                currency: CurrencyCode::THB,
                quantity: ForeignAmount::from_decimal(Decimal::from_str("10000.00").unwrap()),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: unique("fund"),
                    source_key: "thb".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund the safe");
    tx.commit().await.unwrap();

    let teller = person(&pool, "T", &[Role::Teller], &main).await;
    let manager = person(&pool, "M", &[Role::BranchManager], &main).await;
    let at = bkk(4, 10, 0);

    let count_id: String = sqlx::query_scalar(
        "INSERT INTO cash_counts (organization_id, branch_id, kind, counted_by, counted_at, idempotency_key, shift_template_id, business_date) \
         VALUES ($1::uuid, $2, 'OPEN', $3::uuid, $4, $5, NULL, $6) RETURNING id::text",
    )
    .bind(ORG)
    .bind(main.as_str())
    .bind(&teller.user_id)
    .bind(at)
    .bind(unique("c8"))
    .bind(nov(4))
    .fetch_one(&pool)
    .await
    .expect("insert untemplated OPEN count");

    sqlx::query(
        "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted) \
         VALUES ($1::uuid, $2::uuid, 'THB', 2000::numeric)",
    )
    .bind(&count_id)
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("insert count total");

    let counts = CashCountService::pg(pool.clone());
    let mut tx = pool.begin().await.unwrap();
    let got = counts
        .verify_in(&mut tx, &manager, &count_id, bkk(4, 10, 5))
        .await
        .expect("verify untemplated OPEN count");
    tx.commit().await.unwrap();

    let shift_id = match got {
        CountVerified::Verified(c) => c.shift_id.expect("the new shift"),
        other => panic!("expected Verified, got {other:?}"),
    };

    let row: (Option<String>, Option<String>, NaiveDate) = sqlx::query_as(
        "SELECT template_id::text, template_name, business_date FROM shifts WHERE id = $1",
    )
    .bind(&shift_id)
    .fetch_one(&pool)
    .await
    .expect("the shift row");

    assert_eq!(row, (None, None, nov(4)));
}
