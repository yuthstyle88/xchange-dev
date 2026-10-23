//! #4998 (Q6 of #4961, three-decimal currencies): `draftBegin` (`POST /api/tx-drafts`) takes `KWD 1.125` end to end:
//! the draft keeps the amount the teller typed, and the matching save completes it. Row M1 of #4906.
//!
//! Written before the implementation by the ticket author's lane (claude/be38637e, for claude/7ca80c). Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! Design `docs/design/three-decimal-currencies.md` §4 (custody JSON stores decimal strings, §3):
//! - K2. `draftBegin` `KWD 1.125`, then the matching save → the draft is stored as `"1.125"`; the save completes it
//!   (`xc-shift` `branch_day.rs:137`, `draft_matches_save`).
//! - K3. `draftBegin` `KWD 1.1255` → 400 `validationFailed` on `amount`, no draft stored.
//! - M2–M4 stay pinned by `draft_begin_foreign_amount_test.rs` and must stay green.
//!
//! Today K2 is refused at `draftBegin`: the amount passes `ForeignAmount::parse` but is turned back into 2-place
//! `Money` for `BeginDraft` (`branch_day.rs` "shortcut", around line 495). K2's save also needs Q4 (#4996), so the row
//! is red until Q2, Q3, Q4 and Q6 have all landed. K3 holds today and must still hold.
//!
//! The matching save is the v3 desk save, as `ca10p_desk_draft_begin_save_test.rs` makes it: `POST /api/quotes`,
//! `POST /api/transactions` naming the draft (`staffDraftId`), then `POST /api/transactions/{id}/confirm`, whose
//! custody step completes the draft it names. Everything goes through HTTP, so this file compiles on today's types and
//! on Q2–Q6's.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending in `_test`; nothing skips. Each test works in a schema of its
//! own.

mod common;
mod rates_re4c_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, SubsecRound, Utc};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
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
const PASSPORT: &str = "PASSPORT-B01";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

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
        let schema = unique("draftkwd");
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
        db.exec(&format!(
            "INSERT INTO branches (id, name) VALUES ('{B01}', 'B01')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('{TELLER_ID}', '{ORG}', '{TELLER}', 'ACTIVE', 'test')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('{PASSPORT}', '{B01}', '{TELLER_ID}', 'image/jpeg', 'sha-b01', now())"
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

    /// Every stored draft of B01: (id, `data->'amount'` as JSON text, status, documentRef).
    async fn drafts(&self) -> Vec<(String, String, String, Option<String>)> {
        sqlx::query_as(
            "SELECT id, (data->'amount')::text, data->>'status', data->>'documentRef' \
             FROM tx_drafts WHERE branch_id = $1 ORDER BY position",
        )
        .bind(B01)
        .fetch_all(&self.pool)
        .await
        .expect("read tx_drafts")
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

fn ok(a: &Answer, what: &str) {
    assert!(
        a.status.is_success(),
        "{what}: expected success; got {} {}",
        a.status,
        a.text
    );
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
    ok(&clock_in, "clock IN");
    post(
        db,
        "/api/tx-drafts",
        json!({ "branch": B01, "side": "BUY", "currency": currency, "amount": amount }),
    )
    .await
}

/// The v3 desk save of BUY `quantity` `currency`, naming `staff_draft_id`: quote, create, confirm. The confirm's answer.
async fn save(db: &TestDb, currency: &str, quantity: &str, staff_draft_id: &str) -> Answer {
    let quote = post(
        db,
        "/api/quotes",
        json!({
            "branch": B01,
            "customerId": db.customer,
            "channel": "DESK",
            "lines": [{ "currency": currency, "shopSide": "BUY", "quantity": quantity }],
        }),
    )
    .await;
    ok(&quote, &format!("quote {currency} {quantity} BUY"));
    let quote_id = quote.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("quote id: {}", quote.text));

    let created = post(
        db,
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
            "valueDate": "2026-10-23",
            "notes": "",
            "countConfirmed": true,
        }),
    )
    .await;
    ok(
        &created,
        &format!("POST /api/transactions {currency} {quantity}"),
    );
    let tx_id = created.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("transaction id: {}", created.text));

    post(
        db,
        &format!("/api/transactions/{tx_id}/confirm"),
        json!({ "branch": B01 }),
    )
    .await
}

// ------------------------------------------------------------ the rows

/// K2: `draftBegin` KWD 1.125 is stored as the decimal string `"1.125"`; the matching save of BUY KWD 1.125 completes
/// it with the save's document number.
#[actix_web::test]
async fn k2_a_kwd_draft_of_1_125_is_stored_as_typed_and_the_matching_save_completes_it() {
    let db = TestDb::start().await;
    rates_re4c_common::set_counter_rate(&db.pool, B01, "KWD", "100", "101").await;

    let draft = begin(&db, "KWD", "1.125").await;
    ok(&draft, "K2: draftBegin KWD 1.125 (KWD has 3 decimals)");
    let draft_id = draft.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("K2: the draft's id: {}", draft.text))
        .to_owned();

    let stored = db.drafts().await;
    assert_eq!(
        stored
            .iter()
            .map(|(id, amount, status, _)| (id.as_str(), amount.as_str(), status.as_str()))
            .collect::<Vec<_>>(),
        vec![(draft_id.as_str(), "\"1.125\"", "OPEN")],
        "K2: the draft is stored OPEN with the amount as the decimal string \"1.125\""
    );

    let saved = save(&db, "KWD", "1.125", &draft_id).await;
    ok(&saved, "K2: the matching save of BUY KWD 1.125");
    assert_eq!(saved.body["status"], "CONFIRMED", "K2: {}", saved.text);

    let after = db.drafts().await;
    assert_eq!(after.len(), 1, "K2: one draft: {after:?}");
    let (_, amount, status, document_ref) = &after[0];
    assert_eq!(
        (amount.as_str(), status.as_str()),
        ("\"1.125\"", "COMPLETED"),
        "K2: the matching save completes the draft and keeps its amount"
    );
    assert!(
        document_ref.as_deref().is_some_and(|r| !r.is_empty()),
        "K2: the completed draft carries the save's document number: {after:?}"
    );
    db.finish().await;
}

/// K3: `draftBegin` KWD 1.1255 has one place more than KWD's minor unit: 400 `validationFailed` on `amount`, and no
/// draft is stored.
#[actix_web::test]
async fn k3_a_kwd_draft_with_four_places_is_refused_and_nothing_is_stored() {
    let db = TestDb::start().await;
    let a = begin(&db, "KWD", "1.1255").await;
    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "K3: draftBegin KWD 1.1255: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"], "validationFailed",
        "K3: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["field"], "amount",
        "K3: the refused field: {}",
        a.text
    );
    assert_eq!(db.drafts().await, vec![], "K3: no draft is stored");
    db.finish().await;
}
