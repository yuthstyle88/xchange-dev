//! Ticket #3143 (LG-6.r7): `GET /api/v3/views/my_shift` reads the shift's balances through the
//! `ShiftBalances` port (`state.shift_balances`), never from `shift_balances`.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct Room {
    pool: PgPool,
    schema: String,
}

struct Who {
    id: String,
    spec: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL must be set");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("lg6r7myshift");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(B01)
            .execute(&pool)
            .await
            .expect("insert branch");
        Self { pool, schema }
    }

    async fn teller(&self) -> Who {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("teller"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             VALUES ($1::uuid, $2, $3::uuid, $4)",
        )
        .bind(&id)
        .bind(Role::Teller.as_str())
        .bind(ORG)
        .bind(Some(B01))
        .execute(&self.pool)
        .await
        .expect("give the user a role");
        Who {
            spec: format!("{id}|{}|{B01}", Role::Teller.as_str()),
            id,
        }
    }

    async fn open_shift(&self, teller: &Who) -> String {
        let shift_id = unique("shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), $3, $4, $3::uuid)",
        )
        .bind(&shift_id)
        .bind(B01)
        .bind(&teller.id)
        .bind(unique("open_req"))
        .execute(&self.pool)
        .await
        .expect("insert OPEN shift");
        shift_id
    }

    async fn post_ledger(
        &self,
        from: CashPlace,
        to: CashPlace,
        currency: &str,
        amount: &str,
        source_type: SourceType,
    ) {
        let mut tx = self.pool.begin().await.expect("begin");
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from,
                    to,
                    currency: CurrencyCode::parse(currency).expect("a register currency"),
                    quantity: ForeignAmount::from_decimal(
                        amount.parse::<Decimal>().expect("an amount"),
                    ),
                    source: SourceRef {
                        source_type,
                        source_id: unique("src"),
                        source_key: format!("lg6r7:{currency}"),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("post through the ledger");
        tx.commit().await.expect("commit");
    }

    async fn my_shift(&self, who: &Who) -> Value {
        let state = AppState::new(Arc::new(common::settings(&[])), self.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::default()
            .method(Method::GET)
            .uri(&format!("/api/v3/views/my_shift?branch={B01}"))
            .insert_header((header::HOST, HOST))
            .insert_header((AS, who.spec.clone()))
            .to_request();
        let resp = atest::call_service(&app, req).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(status, StatusCode::OK, "my_shift status: {text}");
        let body: Value = serde_json::from_slice(&bytes).expect("JSON response");
        body["data"].clone()
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move {
            let mut parts = spec.splitn(3, '|');
            let user = (|| {
                let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
                let role = Role::parse(role)?;
                let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
                Some((
                    id.to_owned(),
                    branch.to_owned(),
                    User::new(
                        id,
                        ORG,
                        vec![ScopedRole::new(
                            role,
                            Some(BranchId::new(branch)),
                            from,
                            None,
                        )],
                        None,
                    ),
                ))
            })();
            match user {
                Some((_, _, u)) => Callers {
                    user: Some(u.clone()),
                    staff_user: Some(u),
                },
                None => Callers {
                    user: None,
                    staff_user: None,
                },
            }
        })
    }
}

#[actix_web::test]
async fn row1_open_shift_drawer_funded_opening_reads_balances_from_ledger() {
    let room = Room::start().await;
    let teller = room.teller().await;
    let shift = room.open_shift(&teller).await;
    room.post_ledger(
        CashPlace::Bank,
        CashPlace::Drawer(shift.clone()),
        "THB",
        "1000",
        SourceType::Opening,
    )
    .await;

    let data = room.my_shift(&teller).await;
    assert_eq!(data["id"], json!(shift));
    assert_eq!(
        data["balances"],
        json!([{ "code": "THB", "opening": "1000.00", "expected": "1000.00" }])
    );

    room.finish().await;
}

#[actix_web::test]
async fn row2_posting_out_of_drawer_keeps_opening_and_reduces_expected() {
    let room = Room::start().await;
    let teller = room.teller().await;
    let shift = room.open_shift(&teller).await;
    room.post_ledger(
        CashPlace::Bank,
        CashPlace::Drawer(shift.clone()),
        "THB",
        "1000",
        SourceType::Opening,
    )
    .await;
    room.post_ledger(
        CashPlace::Drawer(shift.clone()),
        CashPlace::Safe(BranchId::new(B01)),
        "THB",
        "300",
        SourceType::Transfer,
    )
    .await;

    let data = room.my_shift(&teller).await;
    assert_eq!(
        data["balances"],
        json!([{ "code": "THB", "opening": "1000.00", "expected": "700.00" }])
    );

    room.finish().await;
}

#[actix_web::test]
async fn row3_caller_with_no_open_shift_in_branch_gets_null() {
    let room = Room::start().await;
    let teller = room.teller().await;

    let data = room.my_shift(&teller).await;
    assert_eq!(data, Value::Null);

    room.finish().await;
}

#[actix_web::test]
async fn row4_balances_ordered_thb_first_then_a_to_z() {
    let room = Room::start().await;
    let teller = room.teller().await;
    let shift = room.open_shift(&teller).await;
    for (code, amount) in [
        ("USD", "200"),
        ("EUR", "150"),
        ("THB", "1000"),
        ("AUD", "50"),
    ] {
        room.post_ledger(
            CashPlace::Bank,
            CashPlace::Drawer(shift.clone()),
            code,
            amount,
            SourceType::Opening,
        )
        .await;
    }

    let data = room.my_shift(&teller).await;
    let codes: Vec<&str> = data["balances"]
        .as_array()
        .expect("balances array")
        .iter()
        .map(|b| b["code"].as_str().expect("currency code"))
        .collect();
    assert_eq!(codes, vec!["THB", "AUD", "EUR", "USD"]);

    room.finish().await;
}
