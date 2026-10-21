//! #3008 §9 row 9: a teller who begins a draft with `POST /api/tx-drafts` (`draftBegin`) and then
//! saves the matching transaction with that `staffDraftId` through `/api/transactions`, while a
//! draft is required (`Phase::CutOff`), is accepted; the same save without a draft is refused
//! `SaveClosed`.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending in `_test`; nothing skips.

mod common;
mod rates_re4c_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, SubsecRound, Utc};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use std::sync::Once;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::value_objects::custody_error::CustodyError;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const TELLER: &str = "TELLER@XC-B01";
const PASSPORT: &str = "PASSPORT-B01";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn user_id(spec: &str) -> &'static str {
    match spec {
        TELLER => "00000000-0000-4000-8000-0000000000b2",
        _ => "00000000-0000-4000-8000-0000000000b7",
    }
}

struct TestDb {
    pool: PgPool,
    schema: String,
    customer: String,
}

impl TestDb {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            let _ = xc_customer::install_pii_keys(keys);
        });
        let candidates = [
            std::env::var("TEST_DATABASE_URL").ok(),
            Some("postgres://yuth@127.0.0.1:5433/xc_767_test".to_owned()),
            Some("postgres://postgres:pg@127.0.0.1:55626/xchange_test".to_owned()),
        ];
        let mut chosen = None;
        for url in candidates.into_iter().flatten() {
            let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
            let database = options.get_database().unwrap_or("").to_owned();
            assert!(
                database.ends_with("_test"),
                "'{database}' does not end in `_test`"
            );
            if let Ok(admin) = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(3))
                .connect_with(options.clone())
                .await
            {
                chosen = Some((options, admin));
                break;
            }
        }
        let (options, admin) = chosen.unwrap_or_else(|| {
            panic!("TEST_DATABASE_URL is not set or reachable; these tests need a real PostgreSQL and do not skip")
        });
        let schema = unique("ca10p3008");
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
        let mut db = Self {
            pool,
            schema,
            customer: String::new(),
        };
        db.exec("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{}', '{ORG}', '{TELLER}', 'ACTIVE', 'test')",
            user_id(TELLER)
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('{PASSPORT}', '{B01}', '{}', 'image/jpeg', 'sha-b01', now())",
            user_id(TELLER)
        ))
        .await;
        let customers = CustomerService::new(BranchTx::new(Transactor::new(db.pool.clone())));
        db.customer = customers
            .reserve(
                &BranchId::new(B01),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: "P1234567".to_owned(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "LIVE".to_owned(),
                },
                Utc::now().trunc_subsecs(0),
            )
            .await
            .expect("reserve LIVE customer")
            .customer_id;
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn phase_is(&self, branch: &str, phase: &str) {
        // CA-10 (#626) drops ops_state.enabled and custodian_id; the phase is all this seed sets.
        self.exec(&format!(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('{branch}', '{phase}') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase"
        ))
        .await;
    }

    async fn open_around_now(&self, branch: &str) {
        self.exec(&format!(
            "UPDATE branches b SET opening_hours = (SELECT jsonb_object_agg(d, jsonb_build_array(jsonb_build_object( \
               'open', to_char((now() AT TIME ZONE b.timezone) - interval '1 hour', 'HH24:MI'), \
               'close', to_char((now() AT TIME ZONE b.timezone) + interval '2 hours', 'HH24:MI')))) \
             FROM unnest(ARRAY['mon','tue','wed','thu','fri','sat','sun']) d) WHERE b.id = '{branch}'"
        ))
        .await;
    }

    async fn open_shift(&self, branch: &str, spec: &str) {
        let id = unique("SH");
        self.exec(&format!(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{id}', '{branch}', 'OPEN', now(), 'test', '{id}-open', '{}')",
            user_id(spec)
        ))
        .await;
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    Some(User::new(
        user_id(spec),
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

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn fresh(db: &TestDb, who: &str, uri: &str, body: Value) -> Answer {
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
        .insert_header((AS, who))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn ok(a: &Answer, what: &str) {
    assert!(
        a.status.is_success(),
        "{what}: expected success; got {} {}",
        a.status,
        a.text
    );
}

/// Quotes USD 150 BUY at `B01`, creates the transaction with `staff_draft_id`, and confirms it.
async fn save_at_cutoff(db: &TestDb, staff_draft_id: Option<&str>) -> Answer {
    let quote = fresh(
        db,
        TELLER,
        "/api/quotes",
        json!({
            "branch": B01,
            "customerId": db.customer,
            "channel": "DESK",
            "lines": [{ "currency": "USD", "shopSide": "BUY", "quantity": "150.00" }],
        }),
    )
    .await;
    ok(&quote, "quote USD 150 BUY");
    let quote_id = quote.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("quote id: {}", quote.text));

    let created = fresh(
        db,
        TELLER,
        "/api/transactions",
        json!({
            "branch": B01,
            "quoteId": quote_id,
            "customerId": db.customer,
            "staffDraftId": staff_draft_id,
            "passport": { "uploadKey": PASSPORT },
            "payment": {
                "method": "Cash",
                "reference": "R-1",
                "account": "Cash Drawer",
                "bankFeeThb": "0.00",
                "paidAt": Utc::now().to_rfc3339(),
            },
            "valueDate": "2026-10-18",
            "notes": "",
            "countConfirmed": true,
        }),
    )
    .await;
    ok(&created, "POST /api/transactions");
    let tx_id = created.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("transaction id: {}", created.text));

    fresh(
        db,
        TELLER,
        &format!("/api/transactions/{tx_id}/confirm"),
        json!({ "branch": B01 }),
    )
    .await
}

#[actix_web::test]
async fn draft_begun_with_draft_begin_passes_the_save_gate_at_cutoff_and_no_draft_is_refused() {
    let db = TestDb::start().await;
    db.phase_is(B01, "OPEN").await;
    db.open_around_now(B01).await;
    db.open_shift(B01, TELLER).await;
    rates_re4c_common::set_counter_rate(&db.pool, B01, "USD", "35.0", "36.0").await;

    let clock_in = fresh(
        &db,
        TELLER,
        "/api/attendance",
        json!({ "branch": B01, "direction": "IN" }),
    )
    .await;
    ok(&clock_in, "clock IN");

    let draft = fresh(
        &db,
        TELLER,
        "/api/tx-drafts",
        json!({ "branch": B01, "side": "BUY", "currency": "USD", "amount": "150.00" }),
    )
    .await;
    ok(&draft, "POST /api/tx-drafts");
    let draft_id = draft.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("draft id: {}", draft.text))
        .to_owned();

    // Move the branch day to CUT_OFF so a save requires a matching OPEN draft.
    db.phase_is(B01, "CUT_OFF").await;

    let without_draft = save_at_cutoff(&db, None).await;
    assert!(
        without_draft.status.is_client_error()
            && without_draft
                .text
                .contains(&CustodyError::SaveClosed.to_string()),
        "save at CUT_OFF without a draft must be refused SaveClosed; got {} {}",
        without_draft.status,
        without_draft.text
    );

    let with_draft = save_at_cutoff(&db, Some(&draft_id)).await;
    ok(
        &with_draft,
        "save at CUT_OFF with the draft begun via POST /api/tx-drafts",
    );
    assert_eq!(with_draft.body["status"], "CONFIRMED");

    db.finish().await;
}
