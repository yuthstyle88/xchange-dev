//! Ticket #2394: deciding a hand-over (`accept_in` / `reject_in`) checks `ShiftOperate` on the
//! hand-over's own branch (`h.branch_id`), so a receiver whose role is only at another branch is
//! refused with 403 `outOfScope`, the hand-over stays `PENDING`, and no shift opens for the receiver.

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
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OUT_OF_SCOPE: &str = "outOfScope";

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
        let schema = unique("ho2394");
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
            sqlx::query(
                "INSERT INTO ops_state (branch_id, phase, safe) \
                 VALUES ($1, 'OPEN', jsonb_build_object('USD', 50000::bigint))",
            )
            .bind(branch)
            .execute(&pool)
            .await
            .expect("the branch's safe");
            let safe = || xc_cash::domain::posting::CashPlace::Safe(BranchId::new(branch));
            lg4_fund(
                &pool,
                xc_cash::domain::posting::CashPlace::Bank,
                safe(),
                "THB",
                "1000000.00",
            )
            .await;
        }
        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique(role.as_str()))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             VALUES ($1::uuid, $2, $3::uuid, $4)",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("give the user a role");
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
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
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

async fn opened_shift(room: &Room) -> (Who, String, String) {
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let count = id_of(
        &post(
            room,
            &teller,
            "/api/cash-counts",
            open_count_body(room).await,
        )
        .await,
        "the teller submits the OPEN count",
    );
    ok(
        &post(
            room,
            &manager,
            &format!("/api/cash-counts/{count}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "the branch manager verifies the OPEN count",
    );
    let shift = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await
        .expect("the verified OPEN count opened the teller's shift");
    (teller, count, shift)
}

async fn started_handover_for(room: &Room, receiver_branch: &str) -> (Who, Who, String, String) {
    let (teller, _, shift) = opened_shift(room).await;
    let receiver = room.person(Role::Teller, Some(receiver_branch)).await;
    let thb1000 = room.denomination("THB", "1000").await;
    let a = post(
        room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
        }),
    )
    .await;
    let id = id_of(&a, "handoverStart");
    (teller, receiver, shift, id)
}

async fn seeded_handover_for(room: &Room, receiver_branch: &str) -> (Who, Who, String, String) {
    let (teller, _, shift) = opened_shift(room).await;
    let receiver = room.person(Role::Teller, Some(receiver_branch)).await;
    let thb1000 = room.denomination("THB", "1000").await;
    let count_id: String = sqlx::query_scalar(
        "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES ($1::uuid, $2, $3, 'HANDOVER_OUT', $4::uuid, now(), $5) RETURNING id::text",
    )
    .bind(ORG)
    .bind(B01)
    .bind(&shift)
    .bind(&teller.id)
    .bind(unique("seed_count"))
    .fetch_one(&room.pool)
    .await
    .expect("seed HANDOVER_OUT count");
    sqlx::query(
        "INSERT INTO cash_count_lines (count_id, organization_id, currency, denomination_id, qty) \
         VALUES ($1::uuid, $2::uuid, 'THB', $3, 2)",
    )
    .bind(&count_id)
    .bind(ORG)
    .bind(thb1000)
    .execute(&room.pool)
    .await
    .expect("seed HANDOVER_OUT count line");
    sqlx::query(
        "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected) \
         VALUES ($1::uuid, $2::uuid, 'THB', 2000, 2300)",
    )
    .bind(&count_id)
    .bind(ORG)
    .execute(&room.pool)
    .await
    .expect("seed HANDOVER_OUT count total");
    let id: String = sqlx::query_scalar(
        "INSERT INTO handovers (organization_id, branch_id, from_shift_id, count_id, receiver_id, status) \
         VALUES ($1::uuid, $2, $3, $4::uuid, $5::uuid, 'PENDING') RETURNING id::text",
    )
    .bind(ORG)
    .bind(B01)
    .bind(&shift)
    .bind(&count_id)
    .bind(&receiver.id)
    .fetch_one(&room.pool)
    .await
    .expect("seed PENDING handover");
    (teller, receiver, shift, id)
}

async fn handover_status(room: &Room, id: &str) -> Option<String> {
    room.text(&format!(
        "SELECT status FROM handovers WHERE id = '{id}'::uuid"
    ))
    .await
}

async fn lg4_fund(
    pool: &sqlx::PgPool,
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

/// H1: receiver holds TELLER only at XC-B09; accepts a XC-B01 hand-over with `{"branch":"XC-B09"}`
/// -> 403 `outOfScope`; hand-over stays `PENDING`; outgoing shift stays `OPEN`; receiver has no `OPEN` shift.
#[actix_web::test]
async fn h1_receiver_of_another_branch_cannot_accept_handover() {
    let room = Room::start().await;
    let (_, receiver, shift, id) = seeded_handover_for(&room, B09).await;

    let a = post(
        &room,
        &receiver,
        &format!("/api/handovers/{id}/accept"),
        json!({ "branch": B09 }),
    )
    .await;

    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::FORBIDDEN, OUT_OF_SCOPE),
        "H1 accept by a {B09}-only receiver on a {B01} hand-over: {}",
        a.text
    );
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("PENDING"),
        "hand-over stays PENDING"
    );
    assert_eq!(
        room.text(&format!("SELECT status FROM shifts WHERE id = '{shift}'"))
            .await
            .as_deref(),
        Some("OPEN"),
        "outgoing shift stays OPEN"
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            receiver.id
        ))
        .await,
        0,
        "no shift opens for the receiver"
    );
    room.finish().await;
}

/// H2: receiver holds TELLER only at XC-B09; rejects a XC-B01 hand-over with `{"branch":"XC-B09"}`
/// -> 403 `outOfScope`; hand-over stays `PENDING`.
#[actix_web::test]
async fn h2_receiver_of_another_branch_cannot_reject_handover() {
    let room = Room::start().await;
    let (_, receiver, _, id) = seeded_handover_for(&room, B09).await;

    let a = post(
        &room,
        &receiver,
        &format!("/api/handovers/{id}/reject"),
        json!({ "branch": B09 }),
    )
    .await;

    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::FORBIDDEN, OUT_OF_SCOPE),
        "H2 reject by a {B09}-only receiver on a {B01} hand-over: {}",
        a.text
    );
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("PENDING"),
        "hand-over stays PENDING"
    );
    room.finish().await;
}

/// H3: receiver holds TELLER at XC-B01; accepts with `{"branch":"XC-B01"}` -> 200 `ACCEPTED`.
#[actix_web::test]
async fn h3_receiver_at_handover_branch_accepts() {
    let room = Room::start().await;
    let (_, receiver, _, id) = started_handover_for(&room, B01).await;

    let a = post(
        &room,
        &receiver,
        &format!("/api/handovers/{id}/accept"),
        json!({ "branch": B01 }),
    )
    .await;

    assert_eq!(id_of(&a, "handoverAccept"), id);
    assert_eq!(a.body["status"], "ACCEPTED", "{}", a.text);
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("ACCEPTED")
    );
    room.finish().await;
}

/// H4: receiver holds TELLER at XC-B01; rejects -> 200 `REJECTED`.
#[actix_web::test]
async fn h4_receiver_at_handover_branch_rejects() {
    let room = Room::start().await;
    let (_, receiver, _, id) = started_handover_for(&room, B01).await;

    let a = post(
        &room,
        &receiver,
        &format!("/api/handovers/{id}/reject"),
        json!({ "branch": B01 }),
    )
    .await;

    assert_eq!(id_of(&a, "handoverReject"), id);
    assert_eq!(a.body["status"], "REJECTED", "{}", a.text);
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("REJECTED")
    );
    room.finish().await;
}

/// H5: not the receiver (the outgoing teller) accepts or rejects -> 403 `outOfScope`; `PENDING`.
#[actix_web::test]
async fn h5_non_receiver_cannot_accept_or_reject() {
    let room = Room::start().await;
    let (teller, _, _, id) = started_handover_for(&room, B01).await;

    for step in ["accept", "reject"] {
        let a = post(
            &room,
            &teller,
            &format!("/api/handovers/{id}/{step}"),
            json!({ "branch": B01 }),
        )
        .await;
        assert_eq!(
            (a.status, code(&a).as_str()),
            (StatusCode::FORBIDDEN, OUT_OF_SCOPE),
            "H5 {step} by outgoing teller: {}",
            a.text
        );
    }
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("PENDING")
    );
    room.finish().await;
}
