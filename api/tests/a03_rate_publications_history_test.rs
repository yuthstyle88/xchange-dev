//! Up-front specification tests for the A03 rate publication history ticket (FE gap survey section B,
//! #564 issuecomment-6262359159; design `docs/design/blueprint/rates.md:451`: "`rate_publications` |
//! paged view (A03) | versions, who, when, before/after per line").
//!
//! Written before the implementation by the ticket author claude/dell-claude-6, on the xchange BE
//! controller claude/a90ab16c's order. Do not edit, delete or weaken.
//!
//! Needs TEST_DATABASE_URL naming a database whose name ends in `_test`.

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
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
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

fn dec_str(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("invalid decimal string '{s}': {e}"))
}

fn assert_published_at_eq(actual_val: &Value, expected: DateTime<Utc>, msg: &str) {
    let actual_str = actual_val
        .as_str()
        .unwrap_or_else(|| panic!("{msg}: expected string timestamp, got {actual_val}"));
    let actual_dt = chrono::DateTime::parse_from_rfc3339(actual_str)
        .unwrap_or_else(|e| panic!("{msg}: invalid RFC 3339 timestamp '{actual_str}': {e}"))
        .with_timezone(&Utc);
    assert_eq!(actual_dt.timestamp(), expected.timestamp(), "{msg}");
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
        let schema = unique("a03");
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

        Self { pool, schema }
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn create_branch(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id) \
             VALUES ($1, $1, $2::uuid) ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("create branch");
    }

    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("u"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    /// One publication of `branch` with `lines` = (currency, group, value_tier, unit, buy, sell),
    /// published at `at`; becomes the branch's current publication. Answers its id.
    async fn publish(&self, branch: &str, at: DateTime<Utc>, lines: &[Line]) -> String {
        let approval: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
             proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, $2, 'h', 'p', '{}', 'test', now(), 'APPROVED', $2) \
             RETURNING id",
        )
        .bind(branch)
        .bind(unique("pub"))
        .fetch_one(&self.pool)
        .await
        .expect("approval request");
        let version: i32 = sqlx::query_scalar(
            "SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .expect("version");
        let publication: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, 'rates-1') RETURNING id",
        )
        .bind(branch)
        .bind(version)
        .bind(at)
        .bind(approval)
        .fetch_one(&self.pool)
        .await
        .expect("the publication");
        for l in lines {
            sqlx::query(
                "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, value_tier, \
                 unit_size, buy_rate, sell_rate, source_snapshot) \
                 VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(publication)
            .bind(l.0)
            .bind(l.1)
            .bind(l.2)
            .bind(dec_str(l.3))
            .bind(l.4.map(dec_str))
            .bind(l.5.map(dec_str))
            .bind(json!({ "buy_mode": "AUTO", "sell_mode": "AUTO" }))
            .execute(&self.pool)
            .await
            .expect("a rate line");
        }
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch)
            .execute(&self.pool)
            .await
            .expect("make it current");
        publication.to_string()
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

// ------------------------------------------------------------ authentication

struct Who(String);

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, customer) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        (!customer.is_empty()).then_some(customer),
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
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

impl Answer {
    fn ok(&self, what: &str) -> Value {
        assert_eq!(self.status, StatusCode::OK, "{what}: {}", self.text);
        self.body.clone()
    }
}

async fn call(room: &Room, who: &Who, method: Method, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.0.clone()));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// (currency, denomination group, value tier (`ALL` = no tier, the column's default), unit size, buy, sell).
type Line = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
);

fn at(min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 9, min, 0).unwrap()
}

fn rates_manager() -> Who {
    Who(format!("{}|RATE_MANAGER|", uuid::Uuid::new_v4()))
}

/// The page of publications the view answers: `data` is the rows array (a paged view).
async fn page(room: &Room, branch: &str, extra: &str) -> Vec<Value> {
    let uri = format!("/api/v3/views/rate_publications?branch={branch}{extra}");
    let a = call(room, &rates_manager(), Method::GET, &uri).await;
    let body = a.ok("rate_publications");
    body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a paged view answers `data` as the rows array: {}", a.text))
        .clone()
}

fn versions(rows: &[Value]) -> Vec<i64> {
    rows.iter()
        .map(|r| r["version"].as_i64().unwrap_or(-1))
        .collect()
}

/// The row's line for (currency, group, tier).
fn line<'a>(row: &'a Value, currency: &str, group: &str, tier: &str) -> &'a Value {
    row["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("each row carries `lines`: {row}"))
        .iter()
        .find(|l| l["currency"] == currency && l["group"] == group && l["valueTier"] == tier)
        .unwrap_or_else(|| panic!("no line {currency}/{group}/{tier} in {row}"))
}

/// 1. Newest first, `limit` pages, `before` continues after the named publication.
#[actix_web::test]
async fn a03_pages_newest_first_with_limit_and_before() {
    let room = Room::start().await;
    room.create_branch("A03-B1").await;
    let l: &[Line] = &[("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00"))];
    let _v1 = room.publish("A03-B1", at(1), l).await;
    let v2 = room.publish("A03-B1", at(2), l).await;
    let _v3 = room.publish("A03-B1", at(3), l).await;

    let first = page(&room, "A03-B1", "&limit=2").await;
    assert_eq!(
        versions(&first),
        vec![3, 2],
        "newest first, two rows: {first:?}"
    );
    assert_eq!(first[1]["id"], json!(v2));
    let next = page(&room, "A03-B1", &format!("&limit=2&before={v2}")).await;
    assert_eq!(
        versions(&next),
        vec![1],
        "the page after v2 holds v1 only: {next:?}"
    );
    let all = page(&room, "A03-B1", "").await;
    assert_eq!(
        versions(&all),
        vec![3, 2, 1],
        "no limit: the default page holds all three"
    );
    room.finish().await;
}

/// 2. A changed line carries the previous publication's rates as `before` and its own as `after`.
#[actix_web::test]
async fn a03_a_changed_line_has_before_and_after() {
    let room = Room::start().await;
    room.create_branch("A03-B2").await;
    room.publish(
        "A03-B2",
        at(1),
        &[("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00"))],
    )
    .await;
    room.publish(
        "A03-B2",
        at(2),
        &[("USD", "ALL", "ALL", "1", Some("35.10"), Some("36.00"))],
    )
    .await;

    let rows = page(&room, "A03-B2", "").await;
    let usd = line(&rows[0], "USD", "ALL", "ALL");
    assert_eq!(usd["before"]["buy"], json!("35.00000000"), "{usd}");
    assert_eq!(usd["before"]["sell"], json!("36.00000000"), "{usd}");
    assert_eq!(usd["after"]["buy"], json!("35.10000000"), "{usd}");
    assert_eq!(usd["after"]["sell"], json!("36.00000000"), "{usd}");
    assert_eq!(usd["unitSize"], json!("1"), "{usd}");
    room.finish().await;
}

/// 3. The first publication has nothing before it: every line's `before` is null.
#[actix_web::test]
async fn a03_the_first_publication_has_no_before() {
    let room = Room::start().await;
    room.create_branch("A03-B3").await;
    room.publish(
        "A03-B3",
        at(1),
        &[("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00"))],
    )
    .await;

    let rows = page(&room, "A03-B3", "").await;
    let usd = line(&rows[0], "USD", "ALL", "ALL");
    assert_eq!(usd["before"], Value::Null, "{usd}");
    assert_eq!(usd["after"]["buy"], json!("35.00000000"), "{usd}");
    room.finish().await;
}

/// 4. A line new in this publication has `before: null`; a line it dropped is listed with `after: null`.
#[actix_web::test]
async fn a03_added_and_dropped_lines() {
    let room = Room::start().await;
    room.create_branch("A03-B4").await;
    room.publish(
        "A03-B4",
        at(1),
        &[
            ("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00")),
            ("JPY", "ALL", "ALL", "100", Some("22.00"), Some("23.00")),
        ],
    )
    .await;
    room.publish(
        "A03-B4",
        at(2),
        &[
            ("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00")),
            ("EUR", "ALL", "ALL", "1", Some("38.00"), Some("39.00")),
        ],
    )
    .await;

    let rows = page(&room, "A03-B4", "").await;
    let eur = line(&rows[0], "EUR", "ALL", "ALL");
    assert_eq!(eur["before"], Value::Null, "added: {eur}");
    assert_eq!(eur["after"]["sell"], json!("39.00000000"), "added: {eur}");
    let jpy = line(&rows[0], "JPY", "ALL", "ALL");
    assert_eq!(jpy["after"], Value::Null, "dropped: {jpy}");
    assert_eq!(jpy["before"]["buy"], json!("22.00000000"), "dropped: {jpy}");
    assert_eq!(
        jpy["unitSize"],
        json!("100"),
        "dropped keeps its unit: {jpy}"
    );
    let usd = line(&rows[0], "USD", "ALL", "ALL");
    assert_eq!(
        usd["before"], usd["after"],
        "unchanged lines are listed with equal sides: {usd}"
    );
    room.finish().await;
}

/// 5. Lines are matched on (currency, group, value tier): two tiers of one currency are two lines.
#[actix_web::test]
async fn a03_value_tiers_are_matched_separately() {
    let room = Room::start().await;
    room.create_branch("A03-B5").await;
    room.publish(
        "A03-B5",
        at(1),
        &[
            ("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00")),
            ("USD", "ALL", "T2", "1", Some("35.20"), Some("35.80")),
        ],
    )
    .await;
    room.publish(
        "A03-B5",
        at(2),
        &[
            ("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00")),
            ("USD", "ALL", "T2", "1", Some("35.30"), Some("35.80")),
        ],
    )
    .await;

    let rows = page(&room, "A03-B5", "").await;
    let base = line(&rows[0], "USD", "ALL", "ALL");
    let t2 = line(&rows[0], "USD", "ALL", "T2");
    assert_eq!(base["before"]["buy"], json!("35.00000000"), "{base}");
    assert_eq!(t2["before"]["buy"], json!("35.20000000"), "{t2}");
    assert_eq!(t2["after"]["buy"], json!("35.30000000"), "{t2}");
    room.finish().await;
}

/// 6. Another branch's publication is neither listed nor used as `before`.
#[actix_web::test]
async fn a03_another_branch_is_never_mixed_in() {
    let room = Room::start().await;
    room.create_branch("A03-B6").await;
    room.create_branch("A03-B7").await;
    room.publish(
        "A03-B6",
        at(1),
        &[("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00"))],
    )
    .await;
    room.publish(
        "A03-B7",
        at(2),
        &[("USD", "ALL", "ALL", "1", Some("40.00"), Some("41.00"))],
    )
    .await;
    room.publish(
        "A03-B6",
        at(3),
        &[("USD", "ALL", "ALL", "1", Some("35.50"), Some("36.00"))],
    )
    .await;

    let rows = page(&room, "A03-B6", "").await;
    assert_eq!(
        versions(&rows),
        vec![2, 1],
        "only A03-B6's two versions: {rows:?}"
    );
    let usd = line(&rows[0], "USD", "ALL", "ALL");
    assert_eq!(
        usd["before"]["buy"],
        json!("35.00000000"),
        "before is A03-B6's v1, not A03-B7's: {usd}"
    );
    room.finish().await;
}

/// 7. The row keeps today's facts: who, when, state (the current one PUBLISHED, older SUPERSEDED).
#[actix_web::test]
async fn a03_rows_keep_who_when_and_state() {
    let room = Room::start().await;
    room.create_branch("A03-B8").await;
    let l: &[Line] = &[("USD", "ALL", "ALL", "1", Some("35.00"), Some("36.00"))];
    room.publish("A03-B8", at(1), l).await;
    room.publish("A03-B8", at(2), l).await;

    let rows = page(&room, "A03-B8", "").await;
    assert_eq!(rows[0]["state"], json!("PUBLISHED"), "{}", rows[0]);
    assert_eq!(rows[1]["state"], json!("SUPERSEDED"), "{}", rows[1]);
    assert_eq!(rows[0]["publishedBy"], json!("rates-1"), "{}", rows[0]);
    assert_published_at_eq(&rows[1]["publishedAt"], at(1), "v1's time");
    room.finish().await;
}
