//! #4906: `draftBegin` (`POST /api/tx-drafts`) parses the foreign amount with the currency's own minor unit, as the
//! quote does (`ForeignAmount::parse`), instead of `Money::parse`'s fixed 2 decimals.
//!
//! Written before the implementation by the ticket author's lane (claude/be38637e, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Rows (#4906 §3, as ruled on the ticket 2026-10-23: M1 moved to its own ticket):
//! - M2. KWD (minor unit 3), `1.1255` → 400 `validationFailed`, field `amount`.
//! - M3. JPY (minor unit 0), `1.5` → 400 `validationFailed`, field `amount`.
//! - M4. USD (minor unit 2), `10.50` → accepted (as today).
//!
//! Today M3 is accepted. M2 is already refused (`Money::parse` allows 2 decimals), and M4 is accepted.
//!
//! The harness is `ca10p_desk_draft_begin_save_test.rs`'s, trimmed to what `draftBegin` needs: the branch OPEN
//! within its service hours, the teller's OPEN shift, and the teller clocked in. Needs `TEST_DATABASE_URL` naming a
//! database ending in `_test`; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER: &str = "TELLER@XC-B01";
const TELLER_ID: &str = "00000000-0000-4000-8000-0000000000b2";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
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
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        let schema = unique("draftfx");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(8)
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
        let db = Self { pool, schema };
        db.exec(&format!(
            "INSERT INTO branches (id, name) VALUES ('{B01}', 'B01')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{TELLER_ID}', '{ORG}', '{TELLER}', 'ACTIVE', 'test')"
        ))
        .await;
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// The branch OPEN, inside its service hours, with the teller's OPEN shift: what `begin_draft` checks.
    async fn ready_for_drafts(&self) {
        self.exec(&format!(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('{B01}', 'OPEN') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase"
        ))
        .await;
        self.exec(&format!(
            "UPDATE branches b SET opening_hours = (SELECT jsonb_object_agg(d, jsonb_build_array(jsonb_build_object( \
               'open', to_char((now() AT TIME ZONE b.timezone) - interval '1 hour', 'HH24:MI'), \
               'close', to_char((now() AT TIME ZONE b.timezone) + interval '2 hours', 'HH24:MI')))) \
             FROM unnest(ARRAY['mon','tue','wed','thu','fri','sat','sun']) d) WHERE b.id = '{B01}'"
        ))
        .await;
        let id = unique("SH");
        self.exec(&format!(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{id}', '{B01}', 'OPEN', now(), 'test', '{id}-open', '{TELLER_ID}')"
        ))
        .await;
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ session

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    Some(User::new(
        TELLER_ID,
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn post(db: &TestDb, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
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
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("k")))
        .insert_header((AS, TELLER))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A teller at B01, clocked in, begins a BUY draft of `amount` `currency`.
async fn begin(db: &TestDb, currency: &str, amount: &str) -> Answer {
    db.ready_for_drafts().await;
    let clock_in = post(
        db,
        "/api/attendance",
        json!({ "branch": B01, "direction": "IN" }),
    )
    .await;
    assert!(
        clock_in.status.is_success(),
        "clock IN: {} {}",
        clock_in.status,
        clock_in.text
    );
    post(
        db,
        "/api/tx-drafts",
        json!({ "branch": B01, "side": "BUY", "currency": currency, "amount": amount }),
    )
    .await
}

/// Refused as #4906 requires: 400 `validationFailed` on the field `amount`.
fn assert_refused_on_amount(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["code"], "validationFailed",
        "{what}: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"], "amount",
        "{what}: the refused field: {}",
        a.text
    );
}

// ------------------------------------------------------------ the rows

#[actix_web::test]
async fn m2_a_kwd_amount_with_four_decimals_is_refused() {
    let db = TestDb::start().await;
    let a = begin(&db, "KWD", "1.1255").await;
    assert_refused_on_amount(&a, "draftBegin KWD 1.1255 (KWD has 3 decimals)");
    db.finish().await;
}

#[actix_web::test]
async fn m3_a_jpy_amount_with_a_decimal_is_refused() {
    let db = TestDb::start().await;
    let a = begin(&db, "JPY", "1.5").await;
    assert_refused_on_amount(&a, "draftBegin JPY 1.5 (JPY has no decimals)");
    db.finish().await;
}

#[actix_web::test]
async fn m4_a_usd_amount_with_two_decimals_is_accepted_as_today() {
    let db = TestDb::start().await;
    let a = begin(&db, "USD", "10.50").await;
    assert!(
        a.status.is_success(),
        "draftBegin USD 10.50: expected success; got {} {}",
        a.status,
        a.text
    );
    assert!(
        a.body["id"].as_str().is_some_and(|id| !id.is_empty()),
        "draftBegin answers the draft's id: {}",
        a.text
    );
    db.finish().await;
}
