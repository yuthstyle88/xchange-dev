//! CI-6 (#653): consent texts, consent history and retention (design
//! `docs/design/blueprint/customer-identity-consent.md` K5, K6, U1, U2, §4.1, §4.3, §4.4, §5, §9 row CI-6, §9.1 row CI-6).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests:
//! - `xc_auth::domain::access::Permission::ConsentPolicyManage`, held by EXECUTIVE only (§3, §5.1).
//! - `Policy::need(EntryPath::Rpc, "consentPolicy") == Need::Permission(ConsentPolicyManage)`, and the v2 bridge
//!   (`Policy::allow`) lets the owner through on `(Rpc, "consentPolicy")` and refuses every staff role, like
//!   `ratePolicySave` before #602. Both commands and the view inherit `(EntryPath::Rpc, "consentPolicy")`.
//! - `xc_customer::domain::consent::{Purpose, Channel, ConsentError}`:
//!   `Purpose::{Terms, Marketing}`, `Channel::{Sms, Email, Push}`, `ConsentError::{TextMissing, InvalidPeriod}`
//!   deriving `Debug` + `PartialEq` + `Display`, answered to a caller as `AppError::Domain(error.to_string())` (the
//!   convention of `ShiftError` and `CashError`). The HTTP code of `TextMissing` is `consentTextMissing` (§4.3).
//! - `xc_customer::domain::consent_text::ConsentText { purpose: Purpose, lang: String, version: i32, body: String,
//!   published_by: Option<String>, published_at: DateTime<Utc> }`.
//! - `xc_customer::domain::retention::DataType::{IdentityValue, Contact, ClosedCustomer}`.
//! - `xc_customer::application::consent_service::{ConsentService, Registration, ConsentChoice}`:
//!   - `ConsentService::pg(PgPool)`;
//!   - `publish(&self, org: &str, published_by: Option<&str>, purpose: Purpose, lang: &str, body: &str, now)
//!     -> AppResult<ConsentText>`;
//!   - `register(&self, org: &str, Registration, now) -> AppResult<String>` (the new customer's id);
//!   - `record(&self, customer_id: &str, ConsentChoice, now) -> AppResult<()>`;
//!   - `marketing_allowed(&self, customer_id: &str, Channel) -> AppResult<bool>`.
//!
//!   `Registration { user_id: String, display_name: String, lang: String, marketing: Vec<Channel>, evidence_ref:
//!   String }`; `ConsentChoice { purpose: Purpose, channel: Option<Channel>, granted: bool, lang: String,
//!   evidence_ref: String, recorded_by: Option<String> }`.
//! - `xc_customer::application::retention_job::RetentionJob`: `pg(PgPool)`;
//!   `set_period(&self, org: &str, set_by: Option<&str>, DataType, period_days: Option<i32>, now) -> AppResult<()>`;
//!   `purge(&self, now) -> AppResult<u64>` (rows deleted, every organisation).
//! - Routes (§4.4): `POST /api/consent-texts` body `{"purpose": "TERMS"|"MARKETING", "lang": "th"|"en"|"zh",
//!   "body": "<text>"}` answers `{"purpose", "lang", "version"}`; `PUT /api/retention-policies/{dataType}` body
//!   `{"periodDays": n|null}` answers `{"dataType", "periodDays"}`; view `consent_texts` answers
//!   `data = {"texts": [{"purpose", "lang", "version", "body", "publishedAt"}]}`: the newest version per (purpose,
//!   lang), ordered by purpose then lang.
//!
//! Choices made here where the design leaves the form open (the ticket records them):
//! - A text's version is numbered per (organisation, purpose, language): the first `th` TERMS is 1, the next 2; the
//!   first `en` TERMS is 1 whatever `th` holds. The current text of a language is its newest version.
//! - Authorisation of the commands and the view is the route's (`INHERITS`); the services take the organisation and
//!   the acting user id, not an actor.
//! - The view is not public in this ticket: today's view seam refuses every call without a session
//!   (`api/src/presentation/http/v3/view.rs:353`), and that file is OR-4b #767's. The public read for sign-up is
//!   CI-7's, after #767.
//! - A retention age counts from the row's `updated_at` (when it was revoked), never from `created_at`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::AppError;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Permission, Role as BpRole, RolePermissions};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_customer::application::consent_service::{ConsentChoice, ConsentService, Registration};
use xc_customer::application::retention_job::RetentionJob;
use xc_customer::domain::consent::{Channel, ConsentError, Purpose};
use xc_customer::domain::retention::DataType;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const CONSENT_POLICY: &str = "consentPolicy";
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 3, 0, 0).unwrap() + Duration::seconds(seconds)
}

fn days(n: i64) -> Duration {
    Duration::days(n)
}

fn refused_with<T: std::fmt::Debug>(result: Result<T, AppError>, expected: ConsentError) {
    match result {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, expected.to_string()),
        other => panic!("expected AppError::Domain({expected:?}), got {other:?}"),
    }
}

// ── database ────────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
}

impl Db {
    async fn start() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ci6consent");
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
            .max_connections(8)
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
        common::install_test_pii_keys();
        for b in ["MAIN", "XC-B01"] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(b)
            .execute(&pool)
            .await
            .expect("insert a branch");
        }
        Db { pool }
    }

    fn consents(&self) -> ConsentService {
        ConsentService::pg(self.pool.clone())
    }

    fn retention(&self) -> RetentionJob {
        RetentionJob::pg(self.pool.clone())
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn org(&self) -> String {
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
            .fetch_one(&self.pool)
            .await
            .expect("the organisation")
    }

    /// A user of the organisation (an Identity user who has not registered yet, or an EXECUTIVE).
    async fn user(&self) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'Test User', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' \
             RETURNING id::text",
        )
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    /// A walk-in customer of the organisation, written directly.
    async fn customer(&self) -> String {
        let id = format!("TEST-CX-{:08}", NEXT.fetch_add(1, Ordering::Relaxed));
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, organization_id, $1, 'Somchai Jaidee', 'LIVE', $2 FROM branches WHERE id = 'MAIN'",
        )
        .bind(&id)
        .bind(at(0) - days(400))
        .execute(&self.pool)
        .await
        .expect("insert a customer");
        id
    }

    /// An identity row of `kind` and `status`, created long ago and last changed `changed_days_ago` before `at(0)`.
    async fn identity(
        &self,
        customer: &str,
        kind: &str,
        status: &str,
        changed_days_ago: i64,
    ) -> String {
        sqlx::query_scalar(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, status, created_at, updated_at) \
             SELECT gen_random_uuid(), organization_id, $1, $2, '\\x00'::bytea, 'test-1', gen_random_uuid()::text::bytea, \
             $3, $4, $5 FROM customers WHERE id = $1 RETURNING id::text",
        )
        .bind(customer)
        .bind(kind)
        .bind(status)
        .bind(at(0) - days(400))
        .bind(at(0) - days(changed_days_ago))
        .fetch_one(&self.pool)
        .await
        .expect("insert an identity")
    }

    async fn identity_exists(&self, id: &str) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM customer_identities WHERE id = $1::uuid)")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read an identity")
    }

    async fn publish(&self, purpose: Purpose, lang: &str, body: &str) -> i32 {
        self.consents()
            .publish(&self.org().await, None, purpose, lang, body, at(0))
            .await
            .unwrap_or_else(|e| panic!("publish {purpose:?} {lang}: {e:?}"))
            .version
    }

    async fn choose(
        &self,
        customer: &str,
        purpose: Purpose,
        channel: Option<Channel>,
        granted: bool,
        n: i64,
    ) {
        self.consents()
            .record(
                customer,
                ConsentChoice {
                    purpose,
                    channel,
                    granted,
                    lang: "th".to_owned(),
                    evidence_ref: format!("req-{n}"),
                    recorded_by: None,
                },
                at(n),
            )
            .await
            .unwrap_or_else(|e| panic!("record {purpose:?} {channel:?} {granted}: {e:?}"));
    }

    async fn sms_email(&self, customer: &str) -> (bool, bool) {
        let s = self.consents();
        (
            s.marketing_allowed(customer, Channel::Sms)
                .await
                .expect("SMS eligibility"),
            s.marketing_allowed(customer, Channel::Email)
                .await
                .expect("EMAIL eligibility"),
        )
    }
}

/// A `consents` row: purpose, channel, policy_version, lang, granted, effective_at, evidence_ref.
type ConsentRow = (
    String,
    Option<String>,
    i32,
    String,
    bool,
    DateTime<Utc>,
    String,
);

fn registration(user_id: &str, lang: &str, marketing: Vec<Channel>) -> Registration {
    Registration {
        user_id: user_id.to_owned(),
        display_name: "Somchai Jaidee".to_owned(),
        lang: lang.to_owned(),
        marketing,
        evidence_ref: "sign-up-request-1".to_owned(),
    }
}

// ── who may manage consent and retention (§3, §5.1) ──────────────────────────

const EVERY_ROLE: [BpRole; 9] = [
    BpRole::Executive,
    BpRole::Admin,
    BpRole::RateManager,
    BpRole::BranchManager,
    BpRole::Teller,
    BpRole::Finance,
    BpRole::Marketing,
    BpRole::Customer,
    BpRole::VerificationReviewer,
];

#[test]
fn only_executive_holds_consent_policy_manage() {
    for role in EVERY_ROLE {
        assert_eq!(
            RolePermissions::of(role).contains(&Permission::ConsentPolicyManage),
            role == BpRole::Executive,
            "{}",
            role.as_str()
        );
    }
}

#[test]
fn the_consent_policy_row_needs_consent_policy_manage() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new(CONSENT_POLICY)),
        Some(Need::Permission(Permission::ConsentPolicyManage))
    );
}

// ── consent texts: versioned, immutable (U1, §4.1) ───────────────────────────

#[tokio::test]
async fn publishing_a_text_makes_a_new_version_and_never_changes_an_old_one() {
    let db = Db::start().await;
    assert_eq!(db.publish(Purpose::Terms, "th", "ข้อตกลง ฉบับที่ 1").await, 1);
    assert_eq!(db.publish(Purpose::Terms, "th", "ข้อตกลง ฉบับที่ 2").await, 2);
    assert_eq!(
        db.publish(Purpose::Terms, "en", "Terms, first English text")
            .await,
        1,
        "versions are numbered per language"
    );
    assert_eq!(
        db.publish(Purpose::Marketing, "th", "การตลาด").await,
        1,
        "and per purpose"
    );
    let first: String = sqlx::query_scalar(
        "SELECT body FROM consent_texts WHERE purpose = 'TERMS' AND lang = 'th' AND version = 1",
    )
    .fetch_one(&db.pool)
    .await
    .expect("version 1 is still there");
    assert_eq!(first, "ข้อตกลง ฉบับที่ 1");
    assert_eq!(db.count("SELECT count(*) FROM consent_texts").await, 4);
}

#[tokio::test]
async fn a_published_text_carries_who_published_it_and_when() {
    let db = Db::start().await;
    let executive = db.user().await;
    let text = db
        .consents()
        .publish(
            &db.org().await,
            Some(&executive),
            Purpose::Terms,
            "zh",
            "条款",
            at(5),
        )
        .await
        .expect("publish");
    assert_eq!(text.purpose, Purpose::Terms);
    assert_eq!(text.lang, "zh");
    assert_eq!(text.version, 1);
    assert_eq!(text.body, "条款");
    assert_eq!(text.published_by.as_deref(), Some(executive.as_str()));
    assert_eq!(text.published_at, at(5));
}

// ── register: TERMS in the request's language, no fallback (§4.3) ────────────

#[tokio::test]
async fn registering_with_no_terms_text_at_all_is_refused_and_writes_nothing() {
    let db = Db::start().await;
    let user = db.user().await;
    refused_with(
        db.consents()
            .register(&db.org().await, registration(&user, "th", vec![]), at(0))
            .await,
        ConsentError::TextMissing,
    );
    assert_eq!(db.count("SELECT count(*) FROM customers").await, 0);
    assert_eq!(db.count("SELECT count(*) FROM consents").await, 0);
}

#[tokio::test]
async fn registering_in_thai_when_terms_exist_only_in_english_is_refused() {
    let db = Db::start().await;
    db.publish(Purpose::Terms, "en", "Terms").await;
    let user = db.user().await;
    refused_with(
        db.consents()
            .register(&db.org().await, registration(&user, "th", vec![]), at(0))
            .await,
        ConsentError::TextMissing,
    );
    assert_eq!(
        db.count("SELECT count(*) FROM customers").await,
        0,
        "no fallback to another language"
    );
    assert_eq!(db.count("SELECT count(*) FROM consents").await, 0);
}

#[tokio::test]
async fn registering_creates_a_customer_linked_to_the_user_with_the_current_terms_consent() {
    let db = Db::start().await;
    db.publish(Purpose::Terms, "th", "ข้อตกลง ฉบับที่ 1").await;
    db.publish(Purpose::Terms, "th", "ข้อตกลง ฉบับที่ 2").await;
    db.publish(Purpose::Terms, "en", "Terms").await;
    let user = db.user().await;
    let customer = db
        .consents()
        .register(&db.org().await, registration(&user, "th", vec![]), at(7))
        .await
        .expect("register");
    let (linked, name, language): (Option<String>, String, String) = sqlx::query_as(
        "SELECT user_id::text, display_name, preferred_language FROM customers WHERE id = $1",
    )
    .bind(&customer)
    .fetch_one(&db.pool)
    .await
    .expect("the new customer");
    assert_eq!(
        linked.as_deref(),
        Some(user.as_str()),
        "K7: customers.user_id"
    );
    assert_eq!(name, "Somchai Jaidee");
    assert_eq!(language, "th");
    let rows: Vec<ConsentRow> = sqlx::query_as(
        "SELECT purpose, channel, policy_version, lang, granted, effective_at, evidence_ref \
         FROM consents WHERE customer_id = $1",
    )
    .bind(&customer)
    .fetch_all(&db.pool)
    .await
    .expect("the consents");
    assert_eq!(
        rows,
        vec![(
            "TERMS".to_owned(),
            None,
            2,
            "th".to_owned(),
            true,
            at(7),
            "sign-up-request-1".to_owned()
        )],
        "one TERMS row against the newest Thai version"
    );
}

#[tokio::test]
async fn registering_with_marketing_records_each_channel_and_needs_a_marketing_text() {
    let db = Db::start().await;
    db.publish(Purpose::Terms, "th", "ข้อตกลง").await;
    let org = db.org().await;
    let refused_user = db.user().await;
    refused_with(
        db.consents()
            .register(
                &org,
                registration(&refused_user, "th", vec![Channel::Sms]),
                at(0),
            )
            .await,
        ConsentError::TextMissing,
    );
    assert_eq!(
        db.count("SELECT count(*) FROM customers").await,
        0,
        "a refused marketing choice registers nothing"
    );

    db.publish(Purpose::Marketing, "th", "การตลาด").await;
    let user = db.user().await;
    let customer = db
        .consents()
        .register(
            &org,
            registration(&user, "th", vec![Channel::Sms, Channel::Email]),
            at(1),
        )
        .await
        .expect("register with marketing");
    assert_eq!(db.sms_email(&customer).await, (true, true));
    assert!(
        !db.consents()
            .marketing_allowed(&customer, Channel::Push)
            .await
            .expect("PUSH eligibility"),
        "a channel not chosen is not allowed"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM consents WHERE customer_id = '{customer}' AND purpose = 'MARKETING'"
        ))
        .await,
        2
    );
}

// ── record: append-only history, newest row wins (K5) ────────────────────────

#[tokio::test]
async fn a_customer_with_no_marketing_row_is_not_eligible() {
    let db = Db::start().await;
    let customer = db.customer().await;
    assert_eq!(db.sms_email(&customer).await, (false, false));
}

#[tokio::test]
async fn withdrawing_sms_marketing_stops_sms_and_leaves_email_unchanged() {
    let db = Db::start().await;
    db.publish(Purpose::Marketing, "th", "การตลาด").await;
    let customer = db.customer().await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), true, 1)
        .await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Email), true, 2)
        .await;
    assert_eq!(db.sms_email(&customer).await, (true, true));
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), false, 3)
        .await;
    assert_eq!(db.sms_email(&customer).await, (false, true));
    assert_eq!(
        db.count("SELECT count(*) FROM consents").await,
        3,
        "a withdrawal is a new row, not an update"
    );
}

#[tokio::test]
async fn granting_again_after_a_withdrawal_makes_the_channel_eligible_again() {
    let db = Db::start().await;
    db.publish(Purpose::Marketing, "th", "การตลาด").await;
    let customer = db.customer().await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), true, 1)
        .await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), false, 2)
        .await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), true, 3)
        .await;
    assert_eq!(db.sms_email(&customer).await, (true, false));
}

#[tokio::test]
async fn the_newest_row_wins_by_effective_time_not_by_insert_order() {
    let db = Db::start().await;
    db.publish(Purpose::Marketing, "th", "การตลาด").await;
    let customer = db.customer().await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), false, 20)
        .await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Sms), true, 10)
        .await;
    assert_eq!(
        db.sms_email(&customer).await,
        (false, false),
        "the withdrawal effective at +20s is newer than the grant at +10s"
    );
}

#[tokio::test]
async fn recording_a_consent_in_a_language_with_no_text_is_refused_and_writes_nothing() {
    let db = Db::start().await;
    db.publish(Purpose::Marketing, "en", "Marketing").await;
    let customer = db.customer().await;
    refused_with(
        db.consents()
            .record(
                &customer,
                ConsentChoice {
                    purpose: Purpose::Marketing,
                    channel: Some(Channel::Sms),
                    granted: true,
                    lang: "th".to_owned(),
                    evidence_ref: "req-1".to_owned(),
                    recorded_by: None,
                },
                at(0),
            )
            .await,
        ConsentError::TextMissing,
    );
    assert_eq!(db.count("SELECT count(*) FROM consents").await, 0);
}

#[tokio::test]
async fn a_recorded_consent_names_the_newest_text_version_of_its_language() {
    let db = Db::start().await;
    db.publish(Purpose::Marketing, "th", "การตลาด 1").await;
    db.publish(Purpose::Marketing, "th", "การตลาด 2").await;
    db.publish(Purpose::Marketing, "th", "การตลาด 3").await;
    let customer = db.customer().await;
    db.choose(&customer, Purpose::Marketing, Some(Channel::Email), true, 1)
        .await;
    let version: i32 =
        sqlx::query_scalar("SELECT policy_version FROM consents WHERE customer_id = $1")
            .bind(&customer)
            .fetch_one(&db.pool)
            .await
            .expect("the consent row");
    assert_eq!(version, 3);
}

// ── retention: data, nothing purged until set (K6, U2) ───────────────────────

#[tokio::test]
async fn the_purge_deletes_nothing_for_a_data_type_with_no_period() {
    let db = Db::start().await;
    let customer = db.customer().await;
    let passport = db.identity(&customer, "PASSPORT", "REVOKED", 3650).await;
    let phone = db.identity(&customer, "PHONE", "REVOKED", 3650).await;
    db.retention()
        .set_period(&db.org().await, None, DataType::IdentityValue, None, at(0))
        .await
        .expect("an explicit NULL period");
    assert_eq!(db.retention().purge(at(0)).await.expect("purge"), 0);
    assert!(
        db.identity_exists(&passport).await,
        "IDENTITY_VALUE has no period"
    );
    assert!(
        db.identity_exists(&phone).await,
        "CONTACT has no row at all"
    );
}

#[tokio::test]
async fn the_purge_deletes_a_revoked_identity_older_than_its_period_and_nothing_else() {
    let db = Db::start().await;
    let customer = db.customer().await;
    let old = db.identity(&customer, "PASSPORT", "REVOKED", 31).await;
    let recent = db.identity(&customer, "NATIONAL_ID", "REVOKED", 29).await;
    let active = db.identity(&customer, "PASSPORT", "ACTIVE", 3650).await;
    let contact = db.identity(&customer, "EMAIL", "REVOKED", 3650).await;
    db.retention()
        .set_period(
            &db.org().await,
            None,
            DataType::IdentityValue,
            Some(30),
            at(0),
        )
        .await
        .expect("set IDENTITY_VALUE to 30 days");
    assert_eq!(db.retention().purge(at(0)).await.expect("purge"), 1);
    assert!(
        !db.identity_exists(&old).await,
        "revoked 31 days ago: past the period"
    );
    assert!(
        db.identity_exists(&recent).await,
        "revoked 29 days ago, created 400 days ago: the age counts from the revocation"
    );
    assert!(
        db.identity_exists(&active).await,
        "an ACTIVE identity is never purged"
    );
    assert!(
        db.identity_exists(&contact).await,
        "an e-mail is CONTACT, which has no period"
    );
}

#[tokio::test]
async fn the_contact_period_purges_a_revoked_phone_or_email_but_no_document() {
    let db = Db::start().await;
    let customer = db.customer().await;
    let phone = db.identity(&customer, "PHONE", "REVOKED", 100).await;
    let email = db.identity(&customer, "EMAIL", "REVOKED", 100).await;
    let passport = db.identity(&customer, "PASSPORT", "REVOKED", 100).await;
    db.retention()
        .set_period(&db.org().await, None, DataType::Contact, Some(90), at(0))
        .await
        .expect("set CONTACT to 90 days");
    assert_eq!(db.retention().purge(at(0)).await.expect("purge"), 2);
    assert!(!db.identity_exists(&phone).await);
    assert!(!db.identity_exists(&email).await);
    assert!(
        db.identity_exists(&passport).await,
        "a passport is IDENTITY_VALUE"
    );
}

#[tokio::test]
async fn setting_a_period_again_replaces_it_and_null_switches_the_purge_off() {
    let db = Db::start().await;
    let org = db.org().await;
    let executive = db.user().await;
    let job = db.retention();
    job.set_period(
        &org,
        Some(&executive),
        DataType::IdentityValue,
        Some(30),
        at(0),
    )
    .await
    .expect("30 days");
    job.set_period(&org, Some(&executive), DataType::IdentityValue, None, at(1))
        .await
        .expect("back to none");
    let (period, by, when): (Option<i32>, Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT period_days, set_by::text, set_at FROM retention_policies WHERE data_type = 'IDENTITY_VALUE'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("one row per data type");
    assert_eq!(
        (period, by.as_deref(), when),
        (None, Some(executive.as_str()), Some(at(1)))
    );
    let customer = db.customer().await;
    let old = db.identity(&customer, "PASSPORT", "REVOKED", 365).await;
    assert_eq!(job.purge(at(2)).await.expect("purge"), 0);
    assert!(db.identity_exists(&old).await);
}

#[tokio::test]
async fn a_period_of_zero_or_less_is_refused() {
    let db = Db::start().await;
    let org = db.org().await;
    for bad in [0, -1] {
        refused_with(
            db.retention()
                .set_period(&org, None, DataType::Contact, Some(bad), at(0))
                .await,
            ConsentError::InvalidPeriod,
        );
    }
    assert_eq!(db.count("SELECT count(*) FROM retention_policies").await, 0);
}

// ── routes (§4.4) through today's v2 bridge ──────────────────────────────────

/// Who is signed in, from the request header `x-test-as`: `owner`, `staff:<ROLE>:<branch>`, or absent.
struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(
            format!("employee-{}", role.as_str()),
            role,
            BranchId::new(branch),
        ),
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "MANAGER",
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &Db, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn routes() -> Vec<(Method, &'static str, Option<Value>)> {
    vec![
        (
            Method::POST,
            "/api/consent-texts",
            Some(json!({"purpose": "TERMS", "lang": "th", "body": "ข้อตกลง"})),
        ),
        (
            Method::PUT,
            "/api/retention-policies/CONTACT",
            Some(json!({"periodDays": 90})),
        ),
        (Method::GET, "/api/v3/views/consent_texts", None),
    ]
}

#[tokio::test]
async fn nobody_and_every_staff_role_are_refused_by_the_policy_and_write_nothing() {
    let db = Db::start().await;
    let mut callers = vec![String::new()];
    callers.extend(STAFF_ROLES.iter().map(|r| format!("staff:{r}:MAIN")));
    for who in &callers {
        let actor = sessions_of(who).owner.or(sessions_of(who).staff);
        let policy = Policy::allow(actor.as_ref(), EntryPath::Rpc, &Action::new(CONSENT_POLICY));
        let err = policy.expect_err("the bridge refuses everyone but the owner");
        for (method, uri, body) in routes() {
            let a = call(&db, who, method.clone(), uri, body).await;
            assert_eq!(
                a.status.as_u16(),
                err.status(),
                "{method} {uri} as {who:?}: {}",
                a.text
            );
        }
    }
    assert_eq!(db.count("SELECT count(*) FROM consent_texts").await, 0);
    assert_eq!(db.count("SELECT count(*) FROM retention_policies").await, 0);
}

#[tokio::test]
async fn the_owner_publishes_texts_sets_retention_and_reads_the_current_texts() {
    let db = Db::start().await;
    for (n, body) in ["ข้อตกลง 1", "ข้อตกลง 2"].iter().enumerate() {
        let a = call(
            &db,
            "owner",
            Method::POST,
            "/api/consent-texts",
            Some(json!({"purpose": "TERMS", "lang": "th", "body": body})),
        )
        .await;
        assert_eq!(a.status, StatusCode::OK, "publish: {}", a.text);
        assert_eq!(
            a.body,
            json!({"purpose": "TERMS", "lang": "th", "version": n + 1})
        );
    }
    let a = call(
        &db,
        "owner",
        Method::POST,
        "/api/consent-texts",
        Some(json!({"purpose": "MARKETING", "lang": "en", "body": "Marketing"})),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "publish: {}", a.text);

    let a = call(
        &db,
        "owner",
        Method::PUT,
        "/api/retention-policies/IDENTITY_VALUE",
        Some(json!({"periodDays": 30})),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "retention: {}", a.text);
    assert_eq!(
        a.body,
        json!({"dataType": "IDENTITY_VALUE", "periodDays": 30})
    );
    let a = call(
        &db,
        "owner",
        Method::PUT,
        "/api/retention-policies/CONTACT",
        Some(json!({"periodDays": null})),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "retention: {}", a.text);
    assert_eq!(a.body, json!({"dataType": "CONTACT", "periodDays": null}));
    assert_eq!(db.count("SELECT count(*) FROM retention_policies").await, 2);

    let a = call(
        &db,
        "owner",
        Method::GET,
        "/api/v3/views/consent_texts",
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "view: {}", a.text);
    let texts: Vec<Value> = a.body["data"]["texts"]
        .as_array()
        .unwrap_or_else(|| panic!("data.texts: {}", a.text))
        .iter()
        .map(|t| json!({"purpose": t["purpose"], "lang": t["lang"], "version": t["version"], "body": t["body"]}))
        .collect();
    assert_eq!(
        texts,
        vec![
            json!({"purpose": "MARKETING", "lang": "en", "version": 1, "body": "Marketing"}),
            json!({"purpose": "TERMS", "lang": "th", "version": 2, "body": "ข้อตกลง 2"}),
        ],
        "the newest version per (purpose, lang), ordered by purpose then lang"
    );
    assert!(
        a.body["data"]["texts"][0]["publishedAt"].is_string(),
        "publishedAt: {}",
        a.text
    );
}

#[tokio::test]
async fn an_unknown_retention_data_type_or_a_zero_period_is_a_validation_error() {
    let db = Db::start().await;
    for (uri, body) in [
        (
            "/api/retention-policies/EVERYTHING",
            json!({"periodDays": 30}),
        ),
        ("/api/retention-policies/CONTACT", json!({"periodDays": 0})),
    ] {
        let a = call(&db, "owner", Method::PUT, uri, Some(body)).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{uri}: {}", a.text);
        assert_eq!(
            a.body["error"]["code"],
            json!("validationFailed"),
            "{uri}: {}",
            a.text
        );
    }
    assert_eq!(db.count("SELECT count(*) FROM retention_policies").await, 0);
}
