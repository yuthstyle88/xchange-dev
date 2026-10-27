//! K8 (feat/be-gaps-b; design `docs/design/blueprint/customer-identity-consent.md` §4.3 row `reveal`, §4.4 row
//! `identityReveal`): `POST /api/customers/{id}/identities/{identityId}/reveal` with a required `reason`, needing
//! `IdentityDocumentRead`; one audit row with the actor, customer id, identity id, identity type and reason.
//!
//! Written before the implementation by the sub-controller (claude/2ef7d5a6). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests:
//! - `CustomerService::reveal_in(&self, tx, actor: &Actor, branch: &BranchId, customer_id: &str, identity_id: Uuid,
//!   reason: &str, now) -> AppResult<RevealedIdentity>`; `RevealedIdentity { identity_id, identity_type, value }`.
//!   - any ACTIVE identity type: PASSPORT and NATIONAL_ID decrypt to a JSON payload whose `number` is the value;
//!     PHONE and EMAIL decrypt to the plain value. The AAD is `<customer id><TYPE>`, as written today.
//!   - the identity must belong to `customer_id` in the branch's organisation and be ACTIVE, else
//!     `AppError::NotFound { entity: "customer_identity", .. }`, and no audit row.
//!   - no `IdentityDocumentRead` at the branch ⇒ `AppError::Forbidden`, no audit row.
//!   - a `reason` that is empty after trimming ⇒ `AppError::Validation(_)`, no audit row.
//!   - the audit row is written in `tx`: `action = 'identityReveal'`, `object_ref` = the customer id, `scope =
//!     'BRANCH'`, `branch_id`, `actor_id`, and `after_redacted = {"customerId", "identityId", "identityType",
//!     "reason"}` exactly; never the value, its last 4 or its `lookup_hash`.
//! - Route (`identity_reveal.rs`): body `{"branch", "reason"}`; answers `{"identityId", "type", "value"}` exactly;
//!   empty or > 500-character `reason` ⇒ 400 `validationFailed`, field `reason`; no permission ⇒ the policy's 400 `roleNotAllowedHere`; unknown
//!   identity ⇒ 404 `notFound`. A replay of the same `Idempotency-Key` answers `{"identityId", "type"}` only, and no
//!   `idempotency_records` row holds the value (`stored_output`).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{PiiCipher, PiiKeys};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const TEST_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const PASSPORT: &str = "AA1234567";
const PHONE: &str = "+66812345678";
const TELLER_ID: &str = "00000000-0000-4000-8000-0000000008a1";
const MARKETING_ID: &str = "00000000-0000-4000-8000-0000000008a2";
const HOST: &str = "localhost:8080";
const SAME_ORIGIN: &str = "http://localhost:8080";
const AS: &str = "x-test-as";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn keys() -> Arc<PiiKeys> {
    Arc::new(PiiKeys::parse(TEST_KEYS).expect("the test PII key parses"))
}

fn person(passport: &str) -> ReserveCustomer {
    ReserveCustomer {
        name: "Somchai Jaidee".to_owned(),
        passport_no: passport.to_owned(),
        nationality: "THA".to_owned(),
        birth_date: "1990-01-02".to_owned(),
        passport_expiry: "2031-06-11".to_owned(),
        mode: "LIVE".to_owned(),
    }
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("k8rev");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .as_path(),
        )
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("apply api/migrations");
        let _ = xc_customer::install_pii_keys(keys());
        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("insert MAIN");
        let db = Db { pool };
        let org = db.org().await;
        for (id, name) in [(TELLER_ID, "Teller"), (MARKETING_ID, "Marketing")] {
            sqlx::query(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ($1, $2, $3, 'ACTIVE', 'test') ON CONFLICT (id) DO NOTHING",
            )
            .bind(Uuid::parse_str(id).unwrap())
            .bind(org)
            .bind(name)
            .execute(&db.pool)
            .await
            .expect("seed a user");
        }
        db
    }

    fn service(&self) -> CustomerService {
        CustomerService::new(BranchTx::new(Transactor::new(self.pool.clone())))
    }

    async fn org(&self) -> Uuid {
        sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
            .fetch_one(&self.pool)
            .await
            .expect("the organisation")
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn actor(&self, id: &str, role: Role) -> Actor {
        Actor::new(
            id,
            &self.org().await.to_string(),
            vec![ScopedRole::new(
                role,
                Some(BranchId::new("MAIN")),
                at(-3600),
                None,
            )],
            None,
        )
    }

    /// A LIVE customer with an ACTIVE passport `passport`; answers (customer id, passport identity id).
    async fn customer_with_passport(&self, passport: &str) -> (String, Uuid) {
        let reserved = self
            .service()
            .reserve(&BranchId::new("MAIN"), person(passport), at(0))
            .await
            .expect("reserve");
        let identity: Uuid = sqlx::query_scalar(
            "SELECT id FROM customer_identities WHERE customer_id = $1 AND type = 'PASSPORT'",
        )
        .bind(&reserved.customer_id)
        .fetch_one(&self.pool)
        .await
        .expect("the passport identity");
        (reserved.customer_id, identity)
    }

    /// Adds an identity of `kind` holding `plain`, sealed as the application seals it (AAD `<customer><TYPE>`).
    async fn add_identity(&self, customer_id: &str, kind: &str, plain: &str, status: &str) -> Uuid {
        let cipher = PiiCipher::new(keys());
        let aad = format!("{customer_id}{kind}");
        let sealed = cipher.encrypt(aad.as_bytes(), plain.as_bytes());
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO customer_identities \
             (id, organization_id, customer_id, type, encrypted_value, key_id, lookup_hash, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())",
        )
        .bind(id)
        .bind(self.org().await)
        .bind(customer_id)
        .bind(kind)
        .bind(sealed.bytes)
        .bind(sealed.key_id)
        .bind(Uuid::new_v4().as_bytes().to_vec())
        .bind(status)
        .execute(&self.pool)
        .await
        .expect("insert an identity");
        id
    }

    async fn audit_rows(&self) -> Vec<Value> {
        sqlx::query_scalar(
            "SELECT to_jsonb(a) FROM audit_events a WHERE action = 'identityReveal' ORDER BY at",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read audit_events")
    }

    async fn hash_hex(&self, identity: Uuid) -> String {
        sqlx::query_scalar(
            "SELECT encode(lookup_hash, 'hex') FROM customer_identities WHERE id = $1",
        )
        .bind(identity)
        .fetch_one(&self.pool)
        .await
        .expect("the hash")
    }

    async fn reveal(
        &self,
        actor: &Actor,
        customer_id: &str,
        identity: Uuid,
        reason: &str,
    ) -> Result<xc_customer::application::customer::service::RevealedIdentity, AppError> {
        let mut tx = self.pool.begin().await.expect("begin");
        let answer = self
            .service()
            .reveal_in(
                &mut tx,
                actor,
                &BranchId::new("MAIN"),
                customer_id,
                identity,
                reason,
                at(5),
            )
            .await;
        if answer.is_ok() {
            tx.commit().await.expect("commit");
        } else {
            tx.rollback().await.expect("rollback");
        }
        answer
    }
}

fn assert_no_secret(row: &Value, secrets: &[&str]) {
    let text = row.to_string();
    for secret in secrets {
        assert!(
            !text.contains(secret),
            "the audit row holds {secret:?}: {text}"
        );
    }
}

// ── service ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn k8_s1_a_passport_by_id_answers_its_number_and_one_full_audit_row() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let teller = db.actor(TELLER_ID, Role::Teller).await;

    let revealed = db
        .reveal(&teller, &customer, identity, "  the customer asked  ")
        .await
        .expect("TELLER holds IdentityDocumentRead");
    assert_eq!(revealed.identity_id, identity);
    assert_eq!(revealed.identity_type, "PASSPORT");
    assert_eq!(revealed.value, PASSPORT);

    let rows = db.audit_rows().await;
    assert_eq!(rows.len(), 1, "one audit row: {rows:?}");
    let row = &rows[0];
    assert_eq!(row["actor_id"], TELLER_ID);
    assert_eq!(row["scope"], "BRANCH");
    assert_eq!(row["branch_id"], "MAIN");
    assert_eq!(row["object_ref"], customer.as_str());
    assert_eq!(
        row["after_redacted"],
        json!({
            "customerId": customer,
            "identityId": identity.to_string(),
            "identityType": "PASSPORT",
            "reason": "the customer asked",
        }),
        "exactly the actor's who/what/why, the reason trimmed"
    );
    let hash = db.hash_hex(identity).await;
    assert_no_secret(row, &[PASSPORT, "4567", hash.as_str()]);
}

#[tokio::test]
async fn k8_s2_a_phone_identity_answers_the_plain_phone_and_its_type() {
    let db = Db::start().await;
    let (customer, _) = db.customer_with_passport(PASSPORT).await;
    let phone = db.add_identity(&customer, "PHONE", PHONE, "ACTIVE").await;
    let teller = db.actor(TELLER_ID, Role::Teller).await;

    let revealed = db
        .reveal(&teller, &customer, phone, "callback")
        .await
        .expect("reveal the phone");
    assert_eq!(revealed.identity_type, "PHONE");
    assert_eq!(revealed.value, PHONE);

    let rows = db.audit_rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["after_redacted"]["identityType"], "PHONE");
    assert_eq!(rows[0]["after_redacted"]["identityId"], phone.to_string());
    assert_no_secret(&rows[0], &[PHONE, "5678"]);
}

#[tokio::test]
async fn k8_s3_another_customers_identity_is_not_found_and_writes_no_audit() {
    let db = Db::start().await;
    let (first, _) = db.customer_with_passport(PASSPORT).await;
    let (_, other_identity) = db.customer_with_passport("BB7654321").await;
    let teller = db.actor(TELLER_ID, Role::Teller).await;

    let answer = db.reveal(&teller, &first, other_identity, "checking").await;
    assert!(
        matches!(
            answer,
            Err(AppError::NotFound {
                entity: "customer_identity",
                ..
            })
        ),
        "an identity of another customer: {answer:?}"
    );
    assert!(db.audit_rows().await.is_empty(), "no audit row");
}

#[tokio::test]
async fn k8_s4_a_revoked_identity_is_not_found_and_writes_no_audit() {
    let db = Db::start().await;
    let (customer, _) = db.customer_with_passport(PASSPORT).await;
    let revoked = db
        .add_identity(&customer, "EMAIL", "old@example.com", "REVOKED")
        .await;
    let teller = db.actor(TELLER_ID, Role::Teller).await;

    let answer = db.reveal(&teller, &customer, revoked, "checking").await;
    assert!(
        matches!(
            answer,
            Err(AppError::NotFound {
                entity: "customer_identity",
                ..
            })
        ),
        "a REVOKED identity: {answer:?}"
    );
    assert!(db.audit_rows().await.is_empty(), "no audit row");
}

#[tokio::test]
async fn k8_s5_without_identity_document_read_it_is_forbidden_with_no_audit() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let marketing = db.actor(MARKETING_ID, Role::Marketing).await;

    let answer = db.reveal(&marketing, &customer, identity, "checking").await;
    assert!(matches!(answer, Err(AppError::Forbidden)), "{answer:?}");
    assert!(db.audit_rows().await.is_empty(), "no audit row");
}

#[tokio::test]
async fn k8_s6_a_blank_reason_is_refused_with_no_audit() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let teller = db.actor(TELLER_ID, Role::Teller).await;

    for reason in ["", "   "] {
        let answer = db.reveal(&teller, &customer, identity, reason).await;
        assert!(
            matches!(answer, Err(AppError::Validation(_))),
            "reason {reason:?}: {answer:?}"
        );
    }
    assert!(db.audit_rows().await.is_empty(), "no audit row");
}

#[tokio::test]
async fn k8_s7_the_audit_row_is_written_in_the_callers_transaction() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let teller = db.actor(TELLER_ID, Role::Teller).await;

    let mut tx = db.pool.begin().await.expect("begin");
    db.service()
        .reveal_in(
            &mut tx,
            &teller,
            &BranchId::new("MAIN"),
            &customer,
            identity,
            "checking",
            at(5),
        )
        .await
        .expect("reveal");
    tx.rollback().await.expect("rollback");
    assert!(
        db.audit_rows().await.is_empty(),
        "a rolled-back caller leaves no audit row"
    );
}

// ── route ────────────────────────────────────────────────────────────────────

struct HeaderUsers {
    org: Uuid,
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let org = self.org;
        Box::pin(async move {
            let (id, role) = match spec.as_str() {
                "TELLER" => (TELLER_ID, Role::Teller),
                "MARKETING" => (MARKETING_ID, Role::Marketing),
                _ => {
                    return Callers {
                        user: None,
                        staff_user: None,
                    };
                }
            };
            let user = Actor::new(
                id,
                &org.to_string(),
                vec![ScopedRole::new(
                    role,
                    Some(BranchId::new("MAIN")),
                    DateTime::<Utc>::UNIX_EPOCH,
                    None,
                )],
                None,
            );
            Callers {
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

macro_rules! post {
    ($app:expr, $who:expr, $customer:expr, $identity:expr, $key:expr, $reason:expr) => {{
        let req = atest::TestRequest::post()
            .uri(&format!("/api/customers/{}/identities/{}/reveal", $customer, $identity))
            .insert_header((header::HOST, HOST))
            .insert_header((header::ORIGIN, SAME_ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("idempotency-key", $key))
            .insert_header((AS, $who))
            .set_json(json!({ "branch": "MAIN", "reason": $reason }));
        let resp = atest::call_service(&$app, req.to_request()).await;
        let status: StatusCode = resp.status();
        let body: Value = atest::read_body_json(resp).await;
        (status, body)
    }};
}

fn keys_of(value: &Value) -> Vec<String> {
    let mut k: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

macro_rules! app {
    ($db:expr) => {{
        let state = AppState::new(Arc::new(common::settings(&[])), $db.pool.clone());
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers {
            org: $db.org().await,
        });
        atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

#[actix_web::test]
async fn k8_r1_the_route_answers_identity_id_type_and_value_exactly() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let app = app!(db);

    let (status, body) = post!(
        app,
        "TELLER",
        &customer,
        &identity.to_string(),
        unique("k"),
        "asked"
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        keys_of(&body),
        vec!["identityId", "type", "value"],
        "{body}"
    );
    assert_eq!(body["identityId"], identity.to_string());
    assert_eq!(body["type"], "PASSPORT");
    assert_eq!(body["value"], PASSPORT);
    assert_eq!(db.audit_rows().await.len(), 1);
}

#[actix_web::test]
async fn k8_r2_a_role_without_identity_document_read_is_refused_by_the_policy_with_no_audit() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let app = app!(db);

    let (status, body) = post!(
        app,
        "MARKETING",
        &customer,
        &identity.to_string(),
        unique("k"),
        "asked"
    );
    // The route's policy row refuses a staff role without the permission as every v3 command does (measured on the
    // skeleton: 400 `roleNotAllowedHere`).
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "roleNotAllowedHere", "{body}");
    assert!(db.audit_rows().await.is_empty(), "no audit row");
}

#[actix_web::test]
async fn k8_r3_an_empty_or_too_long_reason_is_400_validation_failed_on_reason() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let app = app!(db);

    let too_long = "ก".repeat(501);
    for reason in ["", "  ", too_long.as_str()] {
        let (status, body) = post!(
            app,
            "TELLER",
            &customer,
            &identity.to_string(),
            unique("k"),
            reason
        );
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "reason of {} chars: {body}",
            reason.chars().count()
        );
        assert_eq!(body["error"]["code"], "validationFailed", "{body}");
        assert_eq!(body["error"]["field"], "reason", "{body}");
    }
    let (status, body) = post!(
        app,
        "TELLER",
        &customer,
        &identity.to_string(),
        unique("k"),
        &"ก".repeat(500)
    );
    assert_eq!(status, StatusCode::OK, "500 characters is accepted: {body}");
    assert_eq!(
        db.audit_rows().await.len(),
        1,
        "only the accepted call is audited"
    );
}

#[actix_web::test]
async fn k8_r4_an_unknown_identity_is_404_not_found() {
    let db = Db::start().await;
    let (customer, _) = db.customer_with_passport(PASSPORT).await;
    let app = app!(db);

    let (status, body) = post!(
        app,
        "TELLER",
        &customer,
        &Uuid::new_v4().to_string(),
        unique("k"),
        "asked"
    );
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "notFound", "{body}");
}

#[actix_web::test]
async fn k8_r5_a_replay_answers_no_value_and_no_idempotency_record_holds_it() {
    let db = Db::start().await;
    let (customer, identity) = db.customer_with_passport(PASSPORT).await;
    let app = app!(db);
    let key = unique("k");

    let (status, first) = post!(
        app,
        "TELLER",
        &customer,
        &identity.to_string(),
        key.as_str(),
        "asked"
    );
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["value"], PASSPORT);

    let (status, replay) = post!(
        app,
        "TELLER",
        &customer,
        &identity.to_string(),
        key.as_str(),
        "asked"
    );
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(
        keys_of(&replay),
        vec!["identityId", "type"],
        "a replay never answers the value: {replay}"
    );
    assert_eq!(replay["identityId"], identity.to_string());
    assert_eq!(replay["type"], "PASSPORT");

    let stored = db
        .count(&format!(
            "SELECT count(*) FROM idempotency_records r WHERE to_jsonb(r)::text LIKE '%{PASSPORT}%'"
        ))
        .await;
    assert_eq!(stored, 0, "no idempotency record holds the plain value");
    assert_eq!(
        db.audit_rows().await.len(),
        1,
        "a replay writes no second audit row"
    );
}
