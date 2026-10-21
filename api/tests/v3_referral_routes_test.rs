//! RA-3 (#981, design `docs/design/blueprint/broker-seam.md` §4.3, the owner's answer to O-2,
//! 2026-10-10: "Teller attaches, OWNER replaces"): `attachReferral`, `supersedeReferral` and the
//! `referral_attributions` view.
//! Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The callers are Blueprint `access::Actor`s with exactly the role under test, handed to the seam
//! through `SessionReader::callers`. A policy refusal is pinned by its code, not its HTTP status.
//! Needs `TEST_DATABASE_URL`; nothing skips.

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
const B09: &str = "XC-B09";
const TELLER: &str = "TELLER@XC-B01";
const TELLER_B09: &str = "TELLER@XC-B09";
const EXECUTIVE: &str = "EXECUTIVE@*";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("v3referralroutes");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches");
        Self { pool, schema }
    }

    /// A LIVE customer and a DEMO exchange of theirs at `branch` (a DEMO exchange needs no shift);
    /// answers the exchange's id.
    async fn exchange(&self, branch: &str, customer: &str) -> String {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $1, 'LIVE', now() FROM branches b WHERE b.id = $2 \
             ON CONFLICT DO NOTHING",
        )
        .bind(customer)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        let document_ref = unique("DOC");
        sqlx::query_scalar(
            "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, customer_id, customer_name, \
                 nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by, \
                 status, completed_at) \
             VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'owner', '', '', \
                 DATE '2026-10-06', now(), now(), 'owner', 'COMPLETED', now()) RETURNING id::text",
        )
        .bind(&document_ref)
        .bind(branch)
        .bind(format!("submission-of-{document_ref}"))
        .bind(customer)
        .fetch_one(&self.pool)
        .await
        .expect("insert an exchange")
    }

    /// `(status, code, captured_by, source)` of a capture.
    async fn capture(&self, id: &str) -> (String, String, String, String) {
        sqlx::query_as(
            "SELECT status, code, captured_by, source FROM referral_attributions WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("the capture row")
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
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

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "MARKETING" => Role::Marketing,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        "XC",
        vec![ScopedRole::new(
            role,
            branch,
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

enum Key<'a> {
    Fresh,
    This(&'a str),
    Missing,
}

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key<'_>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
        match key {
            Key::Fresh => req = req.insert_header(("idempotency-key", unique("key"))),
            Key::This(k) => req = req.insert_header(("idempotency-key", k.to_owned())),
            Key::Missing => {}
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn attach_with(
    db: &TestDb,
    who: &str,
    branch: &str,
    tx: &str,
    code: &str,
    key: Key<'_>,
) -> Answer {
    call(
        db,
        who,
        Method::POST,
        &format!("/api/transactions/{tx}/referral"),
        Some(json!({ "branch": branch, "referralCode": code })),
        key,
    )
    .await
}

async fn attach(db: &TestDb, who: &str, branch: &str, tx: &str, code: &str) -> Answer {
    attach_with(db, who, branch, tx, code, Key::Fresh).await
}

async fn supersede(
    db: &TestDb,
    who: &str,
    branch: &str,
    capture: &str,
    code: &str,
    reason: &str,
) -> Answer {
    call(
        db,
        who,
        Method::POST,
        &format!("/api/referral-attributions/{capture}/supersede"),
        Some(json!({ "branch": branch, "referralCode": code, "reason": reason })),
        Key::Fresh,
    )
    .await
}

async fn rows(db: &TestDb, who: &str, branch: &str, transaction: Option<&str>) -> Answer {
    let uri = match transaction {
        Some(t) => format!("/api/v3/views/referral_attributions?branch={branch}&transaction={t}"),
        None => format!("/api/v3/views/referral_attributions?branch={branch}"),
    };
    call(db, who, Method::GET, &uri, None, Key::Fresh).await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn capture_id(a: &Answer) -> String {
    a.body["captureId"]
        .as_str()
        .unwrap_or_else(|| panic!("a captureId: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ attach (O-2: teller or owner)

#[actix_web::test]
async fn a_teller_attaches_a_code_to_their_branchs_exchange() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let a = attach(&db, TELLER, B01, &tx, "ab12cd").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["status"], "ACTIVE");
    let (status, c, by, source) = db.capture(&capture_id(&a)).await;
    assert_eq!(
        (status.as_str(), c.as_str(), by.as_str(), source.as_str()),
        ("ACTIVE", "AB12CD", user_id(TELLER).as_str(), "DESK")
    );
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_attaches_too() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let a = attach(&db, EXECUTIVE, B01, &tx, "AB12CD").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    db.finish().await;
}

#[actix_web::test]
async fn no_other_role_attaches_and_a_teller_stays_in_their_branch() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    // Under RBAC §11.2, BranchManager holds ReferralAttach (Customers: Full).
    // Roles without ReferralAttach (Admin, Finance, Marketing) are refused roleNotAllowedHere.
    for who in ["ADMIN@*", "FINANCE@XC-B01", "MARKETING@*"] {
        let a = attach(&db, who, B01, &tx, "AB12CD").await;
        assert_ne!(a.status, StatusCode::OK, "{who}: {}", a.text);
        assert_eq!(code(&a), "roleNotAllowedHere", "{who}: {}", a.text);
    }
    let named = attach(&db, TELLER_B09, B01, &tx, "AB12CD").await;
    assert_eq!(
        code(&named),
        "outOfScope",
        "naming another branch: {}",
        named.text
    );
    let own = attach(&db, TELLER_B09, B09, &tx, "AB12CD").await;
    assert_eq!(
        code(&own),
        "outOfScope",
        "an exchange of another branch, though the teller names their own: {}",
        own.text
    );
    assert_eq!(
        db.count("SELECT count(*) FROM referral_attributions").await,
        0
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_second_code_is_already_captured_and_the_same_code_is_a_repeat() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let first = attach(&db, TELLER, B01, &tx, "AB12CD").await;
    let again = attach(&db, TELLER, B01, &tx, "ab12cd").await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text);
    assert_eq!(
        capture_id(&again),
        capture_id(&first),
        "a repeat answers the same capture"
    );
    let other = attach(&db, TELLER, B01, &tx, "ZZ99YY").await;
    assert_eq!(other.status, StatusCode::CONFLICT, "{}", other.text);
    assert_eq!(
        code(&other),
        "referralAlreadyCaptured",
        "supersede instead: {}",
        other.text
    );
    assert_eq!(
        db.count("SELECT count(*) FROM referral_attributions").await,
        1
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_bad_code_is_refused_by_field() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    for bad in ["", "AB12C", "AB-12C", "AB12CDEFGH345"] {
        let a = attach(&db, TELLER, B01, &tx, bad).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{bad:?}: {}", a.text);
        assert_eq!(code(&a), "validationFailed", "{bad:?}: {}", a.text);
        assert_eq!(
            a.body["error"]["field"], "referralCode",
            "{bad:?}: {}",
            a.text
        );
    }
    db.finish().await;
}

#[actix_web::test]
async fn the_idempotency_key_is_required_and_a_repeat_applies_once() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let none = attach_with(&db, TELLER, B01, &tx, "AB12CD", Key::Missing).await;
    assert_eq!(none.status, StatusCode::BAD_REQUEST, "{}", none.text);
    assert_eq!(
        db.count("SELECT count(*) FROM referral_attributions").await,
        0
    );
    let first = attach_with(&db, TELLER, B01, &tx, "AB12CD", Key::This("k-1")).await;
    let again = attach_with(&db, TELLER, B01, &tx, "AB12CD", Key::This("k-1")).await;
    assert_eq!(again.body, first.body, "the stored answer");
    let other = attach_with(&db, TELLER, B01, &tx, "ZZ99YY", Key::This("k-1")).await;
    assert_eq!(code(&other), "idempotencyConflict", "{}", other.text);
    assert_eq!(
        db.count("SELECT count(*) FROM referral_attributions").await,
        1
    );
    db.finish().await;
}

// ------------------------------------------------------------ supersede (O-2: owner only)

#[actix_web::test]
async fn the_owner_replaces_a_code_with_a_reason_and_it_is_audited() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let old = capture_id(&attach(&db, TELLER, B01, &tx, "AB12CD").await);
    let a = supersede(
        &db,
        EXECUTIVE,
        B01,
        &old,
        "zz99yy",
        "the customer named another broker",
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let new = capture_id(&a);
    assert_ne!(new, old);
    assert_eq!(db.capture(&old).await.0, "SUPERSEDED");
    let (status, c, by, _) = db.capture(&new).await;
    assert_eq!((status.as_str(), c.as_str()), ("ACTIVE", "ZZ99YY"));
    assert_eq!(by, user_id(EXECUTIVE));
    let audited = db
        .count(&format!(
            "SELECT count(*) FROM audit_events WHERE action = 'supersedeReferral' \
             AND actor_id = '{}' AND after_redacted->>'reason' = 'the customer named another broker'",
            user_id(EXECUTIVE)
        ))
        .await;
    assert_eq!(audited, 1, "one audit row naming the actor and the reason");
    db.finish().await;
}

#[actix_web::test]
async fn only_the_owner_replaces_and_only_with_a_reason_and_only_an_active_code() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let old = capture_id(&attach(&db, TELLER, B01, &tx, "AB12CD").await);
    // Under RBAC §11.2, BranchManager holds ReferralSupersede (Customers: Full).
    // Roles without ReferralSupersede (Teller, Finance, Admin) are refused roleNotAllowedHere.
    for who in [TELLER, "FINANCE@XC-B01", "ADMIN@*"] {
        let a = supersede(&db, who, B01, &old, "ZZ99YY", "reason").await;
        assert_eq!(code(&a), "roleNotAllowedHere", "{who}: {}", a.text);
    }
    let empty = supersede(&db, EXECUTIVE, B01, &old, "ZZ99YY", "  ").await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST, "{}", empty.text);
    assert_eq!(code(&empty), "validationFailed");
    assert_eq!(empty.body["error"]["field"], "reason");
    assert_eq!(db.capture(&old).await.0, "ACTIVE", "nothing changed yet");

    supersede(&db, EXECUTIVE, B01, &old, "ZZ99YY", "first").await;
    let stale = supersede(&db, EXECUTIVE, B01, &old, "QQ88PP", "second").await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.text);
    assert_eq!(code(&stale), "staleVersion", "{}", stale.text);
    db.finish().await;
}

// ------------------------------------------------------------ the view

#[actix_web::test]
async fn a_teller_sees_their_branchs_codes_newest_first_with_the_sub_masked() {
    let db = TestDb::start().await;
    let first_tx = db.exchange(B01, "C-1").await;
    let second_tx = db.exchange(B01, "C-2").await;
    let first = capture_id(&attach(&db, TELLER, B01, &first_tx, "AB12CD").await);
    let second = capture_id(&attach(&db, TELLER, B01, &second_tx, "ZZ99YY").await);
    sqlx::query("UPDATE referral_attributions SET customer_sub = 'sub-0001-12345678', captured_at = now() - interval '1 hour' WHERE id = $1::uuid")
        .bind(&first)
        .execute(&db.pool)
        .await
        .expect("give the first a sub, earlier");
    let other_tx = db.exchange(B09, "C-3").await;
    attach(&db, TELLER_B09, B09, &other_tx, "QQ88PP").await;

    let r = rows(&db, TELLER, B01, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let list = r.body["data"].as_array().expect("rows");
    let ids: Vec<&str> = list.iter().map(|x| x["id"].as_str().expect("id")).collect();
    assert_eq!(
        ids,
        [second.as_str(), first.as_str()],
        "B01 only, newest first"
    );
    assert_eq!(
        list[1]["customerSub"], "***5678",
        "masked as Redactor masks"
    );
    assert_eq!(list[0]["customerSub"], Value::Null);
    assert_eq!(list[1]["transactionId"], first_tx.as_str());
    assert!(
        !r.text.contains("12345678") && !r.text.to_lowercase().contains("phone"),
        "no full sub and no phone: {}",
        r.text
    );

    let one = rows(&db, TELLER, B01, Some(&first_tx)).await;
    let ids: Vec<&str> = one.body["data"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [first.as_str()],
        "?transaction= narrows to one exchange"
    );

    let bad = rows(&db, TELLER, B01, Some("not-a-uuid")).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);

    let owner = rows(&db, EXECUTIVE, B01, None).await;
    assert_eq!(owner.body["data"].as_array().expect("rows").len(), 2);
    db.finish().await;
}

#[actix_web::test]
async fn the_view_is_the_tellers_and_the_owners_only() {
    let db = TestDb::start().await;
    let r = rows(&db, TELLER_B09, B01, None).await;
    assert_eq!(code(&r), "outOfScope", "{}", r.text);
    // Under RBAC §11.2, BranchManager (Customers: Full) and Finance (Customers: View) hold ReferralRead.
    // Roles without Customers (Admin, Marketing) are refused roleNotAllowedHere.
    for who in ["ADMIN@*", "MARKETING@*"] {
        let r = rows(&db, who, B01, None).await;
        assert_eq!(code(&r), "roleNotAllowedHere", "{who}: {}", r.text);
    }
    for who in ["BRANCH_MANAGER@XC-B01", "FINANCE@XC-B01"] {
        let r = rows(&db, who, B01, None).await;
        assert_eq!(r.status, StatusCode::OK, "{who}: {}", r.text);
    }
    db.finish().await;
}

// ------------------------------------------------------------ #4132: the supersede audit names the actor's role
//
// #4132 (owner 2026-10-21, "ทำตามที่แนะนำทั้ง 5 ข้อ", recommendation 4): a referral supersede records the role the
// policy admitted the actor with, not a constant "OWNER" (`capture_service.rs` `supersede_in`).
//
// Written before the implementation by the BE-R6 sub-controller (claude/studio-claude-8). Do not edit, delete or
// weaken; add tests of your own after them.

/// `actor_role` of the one `supersedeReferral` audit row of `actor`.
async fn supersede_audit_role(db: &TestDb, actor: &str) -> String {
    sqlx::query_scalar(
        "SELECT actor_role FROM audit_events WHERE action = 'supersedeReferral' AND actor_id = $1",
    )
    .bind(user_id(actor))
    .fetch_one(&db.pool)
    .await
    .expect("one supersedeReferral audit row of the actor")
}

/// Row 1: an EXECUTIVE supersedes; the audit row's role is EXECUTIVE.
#[actix_web::test]
async fn an_executives_supersede_is_audited_as_executive() {
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let old = capture_id(&attach(&db, TELLER, B01, &tx, "AB12CD").await);
    let a = supersede(&db, EXECUTIVE, B01, &old, "ZZ99YY", "corrected").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(supersede_audit_role(&db, EXECUTIVE).await, "EXECUTIVE");
    db.finish().await;
}

/// Row 2: a BRANCH_MANAGER (the policy admits one, RBAC §11.2) supersedes; the audit row's role is BRANCH_MANAGER.
#[actix_web::test]
async fn a_branch_managers_supersede_is_audited_as_branch_manager() {
    const MANAGER: &str = "BRANCH_MANAGER@XC-B01";
    let db = TestDb::start().await;
    let tx = db.exchange(B01, "C-1").await;
    let old = capture_id(&attach(&db, TELLER, B01, &tx, "AB12CD").await);
    let a = supersede(&db, MANAGER, B01, &old, "ZZ99YY", "corrected").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(supersede_audit_role(&db, MANAGER).await, "BRANCH_MANAGER");
    db.finish().await;
}
