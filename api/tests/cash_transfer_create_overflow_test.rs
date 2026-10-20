//! #2088 follow-up: `transferCreate` stores a DRAFT without any arithmetic, so a transfer whose line total
//! (face value × quantity, in `i64` hundredths) or currency sum does not fit is accepted and stored, and only fails
//! later (submit / dispatch / receive). Create must run the same checked arithmetic #2088 introduces
//! (`Money::try_minor`, `checked_mul`, `checked_add`: `TransferLine::sent_minor() -> Result`, `add_minor`) and refuse
//! early: 400 `validationFailed`, field `lines`, nothing written.
//!
//! Written before the implementation by the ticket author claude/d7aa64. Do not edit, delete or weaken these tests;
//! add your own in another file. Based on #2088's test branch: the refusals here need #2088's checked helpers.
//!
//! Rows:
//! 1. one line of `BIG_FACE` × 922,337,204 (first quantity past `i64` hundredths) → 400 `validationFailed`
//!    (`lines`), no `cash_transfers` / `cash_transfer_lines` row.
//! 2. two lines that each fit (922,337,203 notes of two `BIG_FACE` denominations) but whose USD sum does not →
//!    400 `validationFailed` (`lines`), nothing stored.
//! 3. keep: an ordinary create (10 × USD 50) → 200, `DRAFT`, one transfer and one line stored.
//! 4. keep: the largest line that fits (`BIG_FACE` × 922,337,203) → 200, `DRAFT`, stored.
//!
//! Today rows 1 and 2 answer 200 and store a DRAFT.
//!
//! The harness is `cash_transfer_overflow_test.rs`'s (itself `v3_cash_commands_test.rs`'s), trimmed. Needs
//! `TEST_DATABASE_URL`; nothing skips. Each test works in a schema of its own.

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
use xc_cash::domain::posting::CashPlace;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// The largest face value the transfer repository reads (`Money::parse` caps at 100,000,000 units = 10^10
/// hundredths).
const BIG_FACE: &str = "100000000";
/// `i64::MAX` / 10^10 rounded down: the largest quantity of `BIG_FACE` notes whose total fits in `i64` hundredths
/// (922,337,203 × 10^10 = 9,223,372,030,000,000,000 ≤ 9,223,372,036,854,775,807).
const LARGEST_FITTING_QTY: &str = "922337203";
/// One more note: 9,223,372,040,000,000,000 hundredths, past `i64::MAX`.
const FIRST_OVERFLOWING_QTY: &str = "922337204";

static NEXT: AtomicU64 = AtomicU64::new(0);
static FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

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
        let schema = unique("cashcreate");
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
        for branch in [B01, B09] {
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("insert a branch");
            sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("the branch's ops_state");
            // USD 100,000,500.00 in each safe's ledger: enough to dispatch one note of the big face value.
            fund(
                &pool,
                CashPlace::Safe(BranchId::new(branch)),
                "USD",
                "100000500.00",
            )
            .await;
        }
        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let id = seed_person(
            &mut self.pool.acquire().await.unwrap(),
            ORG,
            &unique(role.as_str()),
            &[RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: from,
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
        }
    }

    /// A USD denomination of `BIG_FACE` units in its own group (stored data: only seeds and SQL write face values;
    /// two groups give two denominations of the same face value, so one transfer can hold two such lines).
    async fn big_denomination(&self, group: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('USD', $1::numeric, $2) RETURNING id",
        )
        .bind(BIG_FACE)
        .bind(group)
        .fetch_one(&self.pool)
        .await
        .expect("insert a big USD denomination")
    }

    /// The seeded USD 50 note.
    async fn fifty(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the seeded USD 50 face value")
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// Rows the transfer tables hold: (`cash_transfers`, `cash_transfer_lines`).
    async fn stored(&self) -> (i64, i64) {
        (
            self.scalar("SELECT count(*) FROM cash_transfers").await,
            self.scalar("SELECT count(*) FROM cash_transfer_lines")
                .await,
        )
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

/// Moves `amount` of `currency` from the bank into `to` through the ledger (`OPENING`), as
/// `v3_cash_commands_test.rs::lg4_fund` does.
async fn fund(pool: &PgPool, to: CashPlace, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to,
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "overflow-fund:{currency}:{}",
                        FUND.fetch_add(1, Ordering::Relaxed)
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

struct Who {
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

struct HeaderUsers;

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

fn id_of(a: &Answer, what: &str) -> String {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

fn assert_validation_failed(a: &Answer, what: &str) {
    assert_eq!(code(a), "validationFailed", "{what}: {}", a.text);
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
}

/// The creator of one BRANCH transfer B01 → B09.
async fn creator(room: &Room) -> Who {
    room.person(Role::BranchManager, Some(B01)).await
}

/// `transferCreate` B01 → B09 of the USD `lines` (`(denomination, qty)`).
async fn create(room: &Room, who: &Who, lines: &[(i64, &str)]) -> Answer {
    let lines: Vec<Value> = lines
        .iter()
        .map(|(d, q)| json!({ "currency": "USD", "denominationId": d, "qty": q }))
        .collect();
    post(
        room,
        who,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": lines,
            "note": "to B09"
        }),
    )
    .await
}

/// Refused as the ticket requires: 400 `validationFailed` on the field `lines`.
fn assert_refused_on_lines(a: &Answer, what: &str) {
    assert_validation_failed(a, what);
    assert_eq!(
        a.body["error"]["field"], "lines",
        "{what}: the refused field: {}",
        a.text
    );
}

// ------------------------------------------------------------ the refusals

#[actix_web::test]
async fn a_create_whose_one_line_overflows_is_refused_and_nothing_is_stored() {
    let room = Room::start().await;
    let who = creator(&room).await;
    let big = room.big_denomination("TEST").await;

    let a = create(&room, &who, &[(big, FIRST_OVERFLOWING_QTY)]).await;
    assert_refused_on_lines(&a, "transferCreate of one line past i64 hundredths");
    assert_eq!(
        room.stored().await,
        (0, 0),
        "a refused create writes no transfer and no line"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_create_whose_lines_fit_but_whose_sum_overflows_is_refused_and_nothing_is_stored() {
    let room = Room::start().await;
    let who = creator(&room).await;
    let big_a = room.big_denomination("TEST").await;
    let big_b = room.big_denomination("TEST2").await;

    let a = create(
        &room,
        &who,
        &[(big_a, LARGEST_FITTING_QTY), (big_b, LARGEST_FITTING_QTY)],
    )
    .await;
    assert_refused_on_lines(
        &a,
        "transferCreate of two fitting lines whose USD sum overflows",
    );
    assert_eq!(
        room.stored().await,
        (0, 0),
        "a refused create writes no transfer and no line"
    );
    room.finish().await;
}

// ------------------------------------------------------------ what must keep working

#[actix_web::test]
async fn an_ordinary_create_is_still_stored_as_a_draft() {
    let room = Room::start().await;
    let who = creator(&room).await;
    let fifty = room.fifty().await;

    let a = create(&room, &who, &[(fifty, "10")]).await;
    let id = id_of(&a, "transferCreate of 10 × USD 50");
    assert_eq!(
        a.body["status"], "DRAFT",
        "a new transfer is a DRAFT: {}",
        a.text
    );
    assert_eq!(room.stored().await, (1, 1), "one transfer, one line");
    assert_eq!(
        room.scalar(&format!(
            "SELECT qty_sent::bigint FROM cash_transfer_lines WHERE transfer_id = '{id}'::uuid"
        ))
        .await,
        10,
        "the line's quantity"
    );
    room.finish().await;
}

#[actix_web::test]
async fn the_largest_line_that_fits_is_still_stored_as_a_draft() {
    let room = Room::start().await;
    let who = creator(&room).await;
    let big = room.big_denomination("TEST").await;

    let a = create(&room, &who, &[(big, "99999999")]).await;
    id_of(&a, "transferCreate of the largest line that fits");
    assert_eq!(
        a.body["status"], "DRAFT",
        "a new transfer is a DRAFT: {}",
        a.text
    );
    assert_eq!(room.stored().await, (1, 1), "one transfer, one line");
    room.finish().await;
}
