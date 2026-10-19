//! Ticket #2396: `POST /api/cash-counts/{id}/verify` (`countVerify`) with a non-uuid `{id}`
//! answers 404 `notFound`, matching an unknown well-formed UUID (§9 behaviour table V1–V4).

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const UNKNOWN_UUID: &str = "00000000-0000-0000-0000-00000000c0a8";

static NEXT: AtomicU64 = AtomicU64::new(0);
static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

async fn lg4_fund(
    pool: &PgPool,
    from: xc_cash::domain::posting::CashPlace,
    to: xc_cash::domain::posting::CashPlace,
    currency: &str,
    amount: &str,
) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from,
                to,
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{currency}:{}",
                        LG4_FUND.fetch_add(1, Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund through the ledger");
    tx.commit().await.expect("commit");
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("cntverify2396");
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
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase, safe) \
             VALUES ($1, 'OPEN', jsonb_build_object('USD', 50000::bigint))",
        )
        .bind(B01)
        .execute(&pool)
        .await
        .expect("branch safe");

        let safe = || xc_cash::domain::posting::CashPlace::Safe(BranchId::new(B01));
        lg4_fund(
            &pool,
            xc_cash::domain::posting::CashPlace::Bank,
            safe(),
            "THB",
            "1000000.00",
        )
        .await;

        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let id = xc_testdb::people::seed_person(
            &mut self.pool.acquire().await.expect("acquire"),
            ORG,
            &unique(role.as_str()),
            &[xc_testdb::people::RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user and give them a role");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
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

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

fn legacy_of(spec: &str) -> Sessions {
    let mut parts = spec.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(id), Some(_), Some("*")) => Sessions {
            owner: Some(Actor::owner(id)),
            staff: None,
        },
        (Some(id), Some(_), Some(branch)) => Sessions {
            owner: None,
            staff: Actor::staff(id, OldRole::Finance, BranchId::new(branch)),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let _s = legacy_of(&spec);
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.spec.clone()))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

fn id_of(a: &Answer, what: &str) -> String {
    ok(a, what)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

async fn open_count_body(room: &Room) -> Value {
    json!({
        "branch": B01,
        "kind": "OPEN",
        "lines": [
            { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" },
            { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": "3" }
        ]
    })
}

/// V1: `POST /api/cash-counts/not-a-uuid/verify` answers 404 `notFound` (on unpatched main: 500 `serverError`).
#[actix_web::test]
async fn v1_count_verify_with_non_uuid_id_answers_404_not_found() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;

    let a = post(
        &room,
        &manager,
        "/api/cash-counts/not-a-uuid/verify",
        json!({ "branch": B01 }),
    )
    .await;

    assert_eq!(
        a.status,
        StatusCode::NOT_FOUND,
        "V1 non-uuid id must answer 404: {}",
        a.text
    );
    assert_eq!(
        code(&a),
        "notFound",
        "V1 non-uuid id must answer error.code = notFound: {}",
        a.text
    );
    room.finish().await;
}

/// V2: `POST /api/cash-counts/00000000-0000-0000-0000-00000000c0a8/verify` (well-formed, no such count) answers 404 `notFound`.
#[actix_web::test]
async fn v2_count_verify_with_unknown_well_formed_uuid_answers_404_not_found() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;

    let a = post(
        &room,
        &manager,
        &format!("/api/cash-counts/{UNKNOWN_UUID}/verify"),
        json!({ "branch": B01 }),
    )
    .await;

    assert_eq!(
        a.status,
        StatusCode::NOT_FOUND,
        "V2 unknown well-formed uuid must answer 404: {}",
        a.text
    );
    assert_eq!(
        code(&a),
        "notFound",
        "V2 unknown well-formed uuid must answer error.code = notFound: {}",
        a.text
    );
    room.finish().await;
}

/// V3: the id of a submitted OPEN count, verified by a branch manager who did not count it -> 200, count VERIFIED, shift opened.
#[actix_web::test]
async fn v3_count_verify_with_submitted_open_count_id_verifies_and_opens_shift() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;

    let submitted = post(
        &room,
        &teller,
        "/api/cash-counts",
        open_count_body(&room).await,
    )
    .await;
    let count = id_of(&submitted, "countSubmit");

    let a = post(
        &room,
        &manager,
        &format!("/api/cash-counts/{count}/verify"),
        json!({ "branch": B01 }),
    )
    .await;

    assert_eq!(id_of(&a, "countVerify"), count);
    assert_eq!(a.body["status"], "VERIFIED", "{}", a.text);
    assert_eq!(
        room.text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{count}'::uuid"
        ))
        .await
        .as_deref(),
        Some(manager.id.as_str())
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await,
        1,
        "the verified OPEN count opens the teller's shift"
    );
    room.finish().await;
}

/// V4: a uuid in upper case, of a real count -> 200, same as V3.
#[actix_web::test]
async fn v4_count_verify_with_uppercase_uuid_of_real_count_verifies_and_opens_shift() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;

    let submitted = post(
        &room,
        &teller,
        "/api/cash-counts",
        open_count_body(&room).await,
    )
    .await;
    let count = id_of(&submitted, "countSubmit");
    let upper = count.to_ascii_uppercase();
    assert_ne!(upper, count, "count uuid should have uppercase hex digits");

    let a = post(
        &room,
        &manager,
        &format!("/api/cash-counts/{upper}/verify"),
        json!({ "branch": B01 }),
    )
    .await;

    assert_eq!(id_of(&a, "countVerify"), count);
    assert_eq!(a.body["status"], "VERIFIED", "{}", a.text);
    assert_eq!(
        room.text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{count}'::uuid"
        ))
        .await
        .as_deref(),
        Some(manager.id.as_str())
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await,
        1,
        "the verified OPEN count opens the teller's shift"
    );
    room.finish().await;
}
