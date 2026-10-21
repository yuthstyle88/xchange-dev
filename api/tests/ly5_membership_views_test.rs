//! LY-5 (#1408): Customer membership views `my_membership` and `customer_tier`.
//! Written before the implementation by the test-writer (gemini/pool-2, ordered by claude/ad05bf8d).
//! Do not edit, delete or weaken.
//!
//! Locks all 12 specification tests for LY-5 (#1408):
//! 1. `customer_reads_own_membership_with_summary_and_mirror_row`
//! 2. `customer_reads_own_membership_when_fake_errors_or_times_out`
//! 3. `customer_reads_own_membership_with_no_mirror_row`
//! 4. `customer_reads_own_membership_when_fake_answers_unknown_phone_zeros`
//! 5. `customer_reads_own_membership_pending_points_are_kept_separate`
//! 6. `customer_with_no_verified_phone_gets_no_tier_no_points_and_no_summary_call`
//! 7. `customer_reads_only_own_membership_isolation_between_two_customers`
//! 8. `phone_never_appears_in_any_response_or_error_body`
//! 9. `customer_tier_viewed_by_teller_or_marketing`
//! 10. `customer_tier_viewed_by_role_without_permission_is_refused`
//! 11. `customer_tier_for_unknown_customer_answers_not_found`
//! 12. `organisation_isolation_customer_tier_and_mirror_read`

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use quote_common::TestDb;
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiKeys};
use xc_loyalty::domain::summary_source::{
    LoyaltyMemberSummary, LoyaltySummarySource, NextExpiry, SummaryTier, TierProgress,
};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS_HEADER: &str = "x-test-as";

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── PII Hash Helper ─────────────────────────────────────────────────────────

fn phone_hash(e164: &str) -> Vec<u8> {
    let keys = PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
        .expect("valid test key");
    let hasher = LookupHasher::new(&keys);
    hasher.hash(IdentityType::Phone, e164).to_vec()
}

fn at_utc(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, s).single().unwrap()
}

// ── Database Seeding Helpers ────────────────────────────────────────────────

async fn seed_customer(db: &TestDb, org: Uuid, id: &str) {
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2, $1, $3, 'LIVE', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(id)
    .bind(org)
    .bind(format!("Customer {id}"))
    .execute(&db.pool)
    .await
    .expect("insert customer");
}

async fn seed_phone(
    db: &TestDb,
    org: Uuid,
    customer: &str,
    e164: &str,
    verified: bool,
    status: &str,
) {
    let keys = PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
        .expect("valid test key");
    let hasher = LookupHasher::new(&keys);
    let hash = hasher.hash(IdentityType::Phone, e164).to_vec();
    let sealed = xc_kernel::pii::PiiCipher::new(Arc::new(keys))
        .encrypt(format!("{customer}PHONE").as_bytes(), e164.as_bytes());

    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status) \
         VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, CASE WHEN $7 THEN now() END, $8)",
    )
    .bind(Uuid::new_v4())
    .bind(org)
    .bind(customer)
    .bind(sealed.bytes)
    .bind(sealed.key_id)
    .bind(hash)
    .bind(verified)
    .bind(status)
    .execute(&db.pool)
    .await
    .expect("insert phone identity");
}

async fn seed_mirror(
    db: &TestDb,
    org: Uuid,
    e164: &str,
    tier: &str,
    effective_until: Option<DateTime<Utc>>,
) {
    let hash = phone_hash(e164);
    sqlx::query(
        "INSERT INTO customer_tier_mirror (organization_id, phone_lookup_hash, tier_code, tier_rule_id, \
             tier_rule_version, effective_at, effective_until, loyalty_seq, synced_at) \
         VALUES ($1, $2, $3, gen_random_uuid(), 1, now(), $4, 1, now()) \
         ON CONFLICT (organization_id, phone_lookup_hash) DO UPDATE SET \
             tier_code = EXCLUDED.tier_code, \
             effective_until = EXCLUDED.effective_until",
    )
    .bind(org)
    .bind(hash)
    .bind(tier)
    .bind(effective_until)
    .execute(&db.pool)
    .await
    .expect("seed customer_tier_mirror");
}

// ── Fake Loyalty Summary Source ─────────────────────────────────────────────

type SummaryFn = Arc<dyn Fn(&str) -> Result<LoyaltyMemberSummary, String> + Send + Sync>;

#[derive(Clone)]
enum FakeBehavior {
    Answer(Result<LoyaltyMemberSummary, String>),
    Delay(Duration, Result<LoyaltyMemberSummary, String>),
    Dynamic(SummaryFn),
}

#[derive(Default)]
struct FakeLoyaltySummarySource {
    behavior: Arc<Mutex<FakeBehavior>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Default for FakeBehavior {
    fn default() -> Self {
        FakeBehavior::Answer(Err("unavailable".to_string()))
    }
}

impl FakeLoyaltySummarySource {
    fn new() -> Self {
        Self::default()
    }

    fn answering(summary: LoyaltyMemberSummary) -> Self {
        let fake = Self::new();
        fake.set_behavior(FakeBehavior::Answer(Ok(summary)));
        fake
    }

    fn failing(error_message: &str) -> Self {
        let fake = Self::new();
        fake.set_behavior(FakeBehavior::Answer(Err(error_message.to_string())));
        fake
    }

    fn delaying(duration: Duration, result: Result<LoyaltyMemberSummary, String>) -> Self {
        let fake = Self::new();
        fake.set_behavior(FakeBehavior::Delay(duration, result));
        fake
    }

    fn answering_fn<F>(f: F) -> Self
    where
        F: Fn(&str) -> Result<LoyaltyMemberSummary, String> + Send + Sync + 'static,
    {
        let fake = Self::new();
        fake.set_behavior(FakeBehavior::Dynamic(Arc::new(f)));
        fake
    }

    fn set_behavior(&self, behavior: FakeBehavior) {
        *self.behavior.lock().unwrap() = behavior;
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl LoyaltySummarySource for FakeLoyaltySummarySource {
    fn summary<'a>(&'a self, phone: &'a str) -> BoxFuture<'a, AppResult<LoyaltyMemberSummary>> {
        self.calls.lock().unwrap().push(phone.to_string());
        let behavior = self.behavior.lock().unwrap().clone();
        Box::pin(async move {
            match behavior {
                FakeBehavior::Answer(res) => res.map_err(AppError::Internal),
                FakeBehavior::Delay(d, res) => {
                    tokio::time::sleep(d).await;
                    res.map_err(AppError::Internal)
                }
                FakeBehavior::Dynamic(f) => f(phone).map_err(AppError::Internal),
            }
        })
    }
}

// ── Test Session Reader ─────────────────────────────────────────────────────

struct TestCallers {
    org_id: String,
}

impl TestCallers {
    fn callers_of(&self, spec: &str) -> Callers {
        if spec.is_empty() {
            return Callers {
                user: None,
                staff_user: None,
            };
        }
        if let Some(cust_id) = spec.strip_prefix("customer:") {
            let user = xc_auth::domain::access::Actor::new(
                &format!("user-{cust_id}"),
                &self.org_id,
                vec![xc_auth::domain::access::ScopedRole::new(
                    xc_auth::domain::access::Role::Customer,
                    None,
                    Utc::now() - chrono::Duration::hours(1),
                    None,
                )],
                Some(cust_id),
            );
            return Callers {
                user: Some(user),
                staff_user: None,
            };
        }
        if let Some(cust_id) = spec.strip_prefix("other_org_customer:") {
            let other_org = "00000000-0000-0000-0000-000000000002".to_string();
            let user = xc_auth::domain::access::Actor::new(
                &format!("user-{cust_id}"),
                &other_org,
                vec![xc_auth::domain::access::ScopedRole::new(
                    xc_auth::domain::access::Role::Customer,
                    None,
                    Utc::now() - chrono::Duration::hours(1),
                    None,
                )],
                Some(cust_id),
            );
            return Callers {
                user: Some(user),
                staff_user: None,
            };
        }
        if spec == "teller" {
            let branch = BranchId::new("XC-B01");
            let user = xc_auth::domain::access::Actor::new(
                "emp-teller",
                &self.org_id,
                vec![xc_auth::domain::access::ScopedRole::new(
                    xc_auth::domain::access::Role::Teller,
                    Some(branch.clone()),
                    Utc::now() - chrono::Duration::hours(1),
                    None,
                )],
                None,
            )
            .with_branch(Some(branch));
            return Callers {
                user: Some(user.clone()),
                staff_user: Some(user),
            };
        }
        if spec == "marketing" {
            let user = xc_auth::domain::access::Actor::new(
                "emp-marketing",
                &self.org_id,
                vec![xc_auth::domain::access::ScopedRole::new(
                    xc_auth::domain::access::Role::Marketing,
                    None,
                    Utc::now() - chrono::Duration::hours(1),
                    None,
                )],
                None,
            );
            return Callers {
                user: Some(user),
                staff_user: None,
            };
        }
        if spec == "finance" {
            let user = xc_auth::domain::access::Actor::new(
                "emp-finance",
                &self.org_id,
                vec![xc_auth::domain::access::ScopedRole::new(
                    xc_auth::domain::access::Role::Finance,
                    None,
                    Utc::now() - chrono::Duration::hours(1),
                    None,
                )],
                None,
            );
            return Callers {
                user: Some(user),
                staff_user: None,
            };
        }
        panic!("unknown test caller spec: {spec}");
    }

    fn get_callers_from_req(&self, req: &HttpRequest) -> Callers {
        let spec = req
            .headers()
            .get(AS_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        self.callers_of(spec)
    }
}

impl SessionReader for TestCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let callers = self.get_callers_from_req(req);
        Box::pin(async move { callers })
    }
}

// ── Actix App and Request Helpers ───────────────────────────────────────────

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn get(db: &TestDb, fake: Arc<FakeLoyaltySummarySource>, who: &str, uri: &str) -> Answer {
    common::install_test_pii_keys();
    sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01') ON CONFLICT DO NOTHING")
        .execute(&db.pool)
        .await
        .expect("seed branch XC-B01");

    let state =
        AppState::new(Arc::new(common::settings(&[])), db.pool.clone()).with_loyalty_summary(fake);
    let reader: Arc<dyn SessionReader> = Arc::new(TestCallers {
        org_id: quote_common::ORG_1.to_string(),
    });
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"));
    if !who.is_empty() {
        req = req.insert_header((AS_HEADER, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

// ── The 11 Specification Tests (#1408 §9) ───────────────────────────────────

/// Row 1: a CUSTOMER reads their own membership, the fake answers a summary, the mirror has a row
#[tokio::test]
async fn customer_reads_own_membership_with_summary_and_mirror_row() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-01";
    let phone = "+66812345678";
    seed_customer(&db, org, cust).await;
    seed_phone(&db, org, cust, phone, true, "ACTIVE").await;
    let until = at_utc(2026, 11, 15, 0, 0, 0);
    seed_mirror(&db, org, phone, "GOLD", Some(until)).await;

    let summary = LoyaltyMemberSummary {
        phone_key: "812345678".to_string(),
        available: 100,
        pending: 25,
        next_expiry: Some(NextExpiry {
            points: 50,
            at: at_utc(2026, 12, 31, 23, 59, 59),
        }),
        tier: Some(SummaryTier {
            code: "GOLD".to_string(),
            effective_until: Some(until),
            progress: Some(TierProgress {
                basis: "VOLUME_THB".to_string(),
                current: Decimal::from_str("12500.00").unwrap(),
                next: Some(Decimal::from_str("20000.00").unwrap()),
            }),
        }),
    };
    let fake = Arc::new(FakeLoyaltySummarySource::answering(summary));

    let ans = get(
        &db,
        fake.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200 OK: {}", ans.text);
    assert_eq!(ans.body["tier"], "GOLD");
    assert_eq!(ans.body["effectiveUntil"], "2026-11-15T00:00:00Z");
    assert_eq!(ans.body["points"]["available"], 100);
    assert_eq!(ans.body["points"]["pending"], 25);
    assert_eq!(ans.body["pointsUnavailable"], false);
    assert_eq!(ans.body["nextExpiry"]["points"], 50);
    assert_eq!(ans.body["nextExpiry"]["at"], "2026-12-31T23:59:59Z");
    assert_eq!(ans.body["progress"]["basis"], "VOLUME_THB");
    assert_eq!(ans.body["progress"]["current"], "12500.00");
    assert_eq!(ans.body["progress"]["next"], "20000.00");

    assert_eq!(fake.calls(), vec![phone]);
    db.finish().await;
}

/// Row 2: the fake errors or exceeds 2 s
#[tokio::test]
async fn customer_reads_own_membership_when_fake_errors_or_times_out() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-02";
    let phone = "+66812345678";
    seed_customer(&db, org, cust).await;
    seed_phone(&db, org, cust, phone, true, "ACTIVE").await;
    let until = at_utc(2026, 11, 15, 0, 0, 0);
    seed_mirror(&db, org, phone, "PLATINUM", Some(until)).await;

    // Part A: fake errors
    let fake_err = Arc::new(FakeLoyaltySummarySource::failing("loyalty unavailable"));
    let ans_err = get(
        &db,
        fake_err.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans_err.status,
        StatusCode::OK,
        "expected 200 on error: {}",
        ans_err.text
    );
    assert_eq!(ans_err.body["tier"], "PLATINUM");
    assert_eq!(ans_err.body["effectiveUntil"], "2026-11-15T00:00:00Z");
    assert!(
        ans_err.body["points"].is_null(),
        "points must be null on failure: {}",
        ans_err.body
    );
    assert_eq!(ans_err.body["pointsUnavailable"], true);
    assert!(ans_err.body["nextExpiry"].is_null());
    assert!(ans_err.body["progress"].is_null());

    // Part B: fake exceeds 2s timeout
    let fake_delay = Arc::new(FakeLoyaltySummarySource::delaying(
        Duration::from_millis(2100),
        Ok(LoyaltyMemberSummary {
            phone_key: "812345678".to_string(),
            available: 999,
            pending: 0,
            next_expiry: None,
            tier: None,
        }),
    ));
    let ans_delay = get(
        &db,
        fake_delay.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans_delay.status,
        StatusCode::OK,
        "expected 200 on timeout: {}",
        ans_delay.text
    );
    assert_eq!(ans_delay.body["tier"], "PLATINUM");
    assert_eq!(ans_delay.body["effectiveUntil"], "2026-11-15T00:00:00Z");
    assert!(
        ans_delay.body["points"].is_null(),
        "points must be null on timeout: {}",
        ans_delay.body
    );
    assert_eq!(ans_delay.body["pointsUnavailable"], true);

    db.finish().await;
}

/// Row 3: no mirror row for the customer's phone
#[tokio::test]
async fn customer_reads_own_membership_with_no_mirror_row() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-03";
    let phone = "+66812345678";
    seed_customer(&db, org, cust).await;
    seed_phone(&db, org, cust, phone, true, "ACTIVE").await;
    // No mirror row seeded

    let summary = LoyaltyMemberSummary {
        phone_key: "812345678".to_string(),
        available: 80,
        pending: 10,
        next_expiry: None,
        tier: None,
    };
    let fake = Arc::new(FakeLoyaltySummarySource::answering(summary));

    let ans = get(
        &db,
        fake.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert!(
        ans.body["tier"].is_null(),
        "tier must be null when no mirror row"
    );
    assert!(ans.body["effectiveUntil"].is_null());
    assert_eq!(ans.body["points"]["available"], 80);
    assert_eq!(ans.body["points"]["pending"], 10);
    assert_eq!(ans.body["pointsUnavailable"], false);

    db.finish().await;
}

/// Row 4: the fake answers the unknown-phone summary (zeros, tier: null)
#[tokio::test]
async fn customer_reads_own_membership_when_fake_answers_unknown_phone_zeros() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-04";
    let phone = "+66812345678";
    seed_customer(&db, org, cust).await;
    seed_phone(&db, org, cust, phone, true, "ACTIVE").await;
    seed_mirror(&db, org, phone, "SILVER", None).await;

    // Unknown phone summary from Loyalty: zeros, tier: null
    let summary = LoyaltyMemberSummary {
        phone_key: "812345678".to_string(),
        available: 0,
        pending: 0,
        next_expiry: None,
        tier: None,
    };
    let fake = Arc::new(FakeLoyaltySummarySource::answering(summary));

    let ans = get(
        &db,
        fake.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert_eq!(
        ans.body["tier"], "SILVER",
        "tier must remain the mirror's tier"
    );
    assert_eq!(ans.body["points"]["available"], 0);
    assert_eq!(ans.body["points"]["pending"], 0);
    assert_eq!(ans.body["pointsUnavailable"], false);

    db.finish().await;
}

/// Row 5: pending points (shown apart from available; never added to them D-7)
#[tokio::test]
async fn customer_reads_own_membership_pending_points_are_kept_separate() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-05";
    let phone = "+66812345678";
    seed_customer(&db, org, cust).await;
    seed_phone(&db, org, cust, phone, true, "ACTIVE").await;

    let summary = LoyaltyMemberSummary {
        phone_key: "812345678".to_string(),
        available: 500,
        pending: 120,
        next_expiry: None,
        tier: None,
    };
    let fake = Arc::new(FakeLoyaltySummarySource::answering(summary));

    let ans = get(
        &db,
        fake.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(ans.status, StatusCode::OK, "expected 200: {}", ans.text);
    assert_eq!(ans.body["points"]["available"], 500);
    assert_eq!(ans.body["points"]["pending"], 120);
    assert_ne!(
        ans.body["points"]["available"], 620,
        "pending must never be added to available (D-7)"
    );

    db.finish().await;
}

/// Row 6: the customer has no verified PHONE
#[tokio::test]
async fn customer_with_no_verified_phone_gets_no_tier_no_points_and_no_summary_call() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();

    let fake = Arc::new(FakeLoyaltySummarySource::answering(LoyaltyMemberSummary {
        phone_key: "812345678".to_string(),
        available: 100,
        pending: 0,
        next_expiry: None,
        tier: None,
    }));

    // Case 1: customer has no phone identity at all
    let cust1 = "cust-ly5-06a";
    seed_customer(&db, org, cust1).await;

    let ans1 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust1}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "case 1 expected 200: {}",
        ans1.text
    );
    assert!(
        ans1.body["tier"].is_null(),
        "tier must be null for customer without phone"
    );
    assert!(
        ans1.body["points"].is_null(),
        "points must be null for customer without phone"
    );

    // Case 2: customer has an ACTIVE phone with verified_at NULL
    let cust2 = "cust-ly5-06b";
    let phone2 = "+66810000001";
    seed_customer(&db, org, cust2).await;
    seed_phone(&db, org, cust2, phone2, false, "ACTIVE").await;
    seed_mirror(&db, org, phone2, "GOLD", None).await;

    let ans2 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust2}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "case 2 expected 200: {}",
        ans2.text
    );
    assert!(
        ans2.body["tier"].is_null(),
        "tier must be null for customer with unverified phone"
    );
    assert!(
        ans2.body["points"].is_null(),
        "points must be null for customer with unverified phone"
    );

    // Case 3: customer has a verified phone with status REVOKED
    let cust3 = "cust-ly5-06c";
    let phone3 = "+66810000002";
    seed_customer(&db, org, cust3).await;
    seed_phone(&db, org, cust3, phone3, true, "REVOKED").await;
    seed_mirror(&db, org, phone3, "PLATINUM", None).await;

    let ans3 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust3}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans3.status,
        StatusCode::OK,
        "case 3 expected 200: {}",
        ans3.text
    );
    assert!(
        ans3.body["tier"].is_null(),
        "tier must be null for customer with revoked phone"
    );
    assert!(
        ans3.body["points"].is_null(),
        "points must be null for customer with revoked phone"
    );

    assert!(
        fake.calls().is_empty(),
        "fake must record NO call when customer has no verified phone: {:?}",
        fake.calls()
    );

    db.finish().await;
}

/// Row 7: customer reads only own membership (isolation between two customers, Ruling b)
#[tokio::test]
async fn customer_reads_only_own_membership_isolation_between_two_customers() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust1 = "cust-ly5-07a";
    let cust2 = "cust-ly5-07b";
    let phone1 = "+66811111111";
    let phone2 = "+66822222222";

    seed_customer(&db, org, cust1).await;
    seed_phone(&db, org, cust1, phone1, true, "ACTIVE").await;
    seed_mirror(&db, org, phone1, "GOLD", None).await;

    seed_customer(&db, org, cust2).await;
    seed_phone(&db, org, cust2, phone2, true, "ACTIVE").await;
    seed_mirror(&db, org, phone2, "SILVER", None).await;

    let fake = Arc::new(FakeLoyaltySummarySource::answering_fn(|phone| {
        let (points, tier_code) = match phone {
            "+66811111111" | "811111111" => (100, "GOLD"),
            "+66822222222" | "822222222" => (50, "SILVER"),
            _ => (0, "NONE"),
        };
        Ok(LoyaltyMemberSummary {
            phone_key: phone.trim_start_matches("+66").to_string(),
            available: points,
            pending: 0,
            next_expiry: None,
            tier: Some(SummaryTier {
                code: tier_code.to_string(),
                effective_until: None,
                progress: None,
            }),
        })
    }));

    // Customer 1 reads own membership
    let ans1 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust1}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "cust1 expected 200: {}",
        ans1.text
    );
    assert_eq!(ans1.body["tier"], "GOLD");
    assert_eq!(ans1.body["points"]["available"], 100);

    // Customer 2 reads own membership
    let ans2 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust2}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "cust2 expected 200: {}",
        ans2.text
    );
    assert_eq!(ans2.body["tier"], "SILVER");
    assert_eq!(ans2.body["points"]["available"], 50);

    // Isolation: each customer sees only their own tier and points
    assert_ne!(ans1.body["tier"], ans2.body["tier"]);
    assert_ne!(
        ans1.body["points"]["available"],
        ans2.body["points"]["available"]
    );

    db.finish().await;
}

/// Row 8: any response or error body holds no phone
#[tokio::test]
async fn phone_never_appears_in_any_response_or_error_body() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-08";
    let phone_e164 = "+66899991234";
    let phone_national = "899991234";
    seed_customer(&db, org, cust).await;
    seed_phone(&db, org, cust, phone_e164, true, "ACTIVE").await;
    seed_mirror(&db, org, phone_e164, "SILVER", None).await;

    let fake = Arc::new(FakeLoyaltySummarySource::answering(LoyaltyMemberSummary {
        phone_key: phone_national.to_string(),
        available: 50,
        pending: 0,
        next_expiry: None,
        tier: None,
    }));

    // 1. my_membership response (200 OK)
    let ans1 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::OK,
        "my_membership expected 200: {}",
        ans1.text
    );
    assert!(
        !ans1.text.contains(phone_e164),
        "my_membership body must not contain E.164 phone: {}",
        ans1.text
    );
    assert!(
        !ans1.text.contains(phone_national),
        "my_membership body must not contain national phone: {}",
        ans1.text
    );

    // 2. customer_tier response (200 OK)
    let ans2 = get(
        &db,
        fake.clone(),
        "teller",
        &format!("/api/v3/views/customer_tier?customer={cust}"),
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "customer_tier expected 200: {}",
        ans2.text
    );
    assert!(
        !ans2.text.contains(phone_e164),
        "customer_tier body must not contain E.164 phone: {}",
        ans2.text
    );
    assert!(
        !ans2.text.contains(phone_national),
        "customer_tier body must not contain national phone: {}",
        ans2.text
    );

    // 3. error response (403 outOfScope)
    let ans3 = get(
        &db,
        fake.clone(),
        &format!("other_org_customer:{cust}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans3.status,
        StatusCode::FORBIDDEN,
        "error response expected 403: {}",
        ans3.text
    );
    assert_eq!(ans3.body["error"]["code"], "outOfScope");
    assert!(
        !ans3.text.contains(phone_e164),
        "error body must not contain E.164 phone: {}",
        ans3.text
    );
    assert!(
        !ans3.text.contains(phone_national),
        "error body must not contain national phone: {}",
        ans3.text
    );

    db.finish().await;
}

/// Row 9: customer_tier?customer= as TELLER (ExchangeTransact) or MARKETING (MarketingMembers)
#[tokio::test]
async fn customer_tier_viewed_by_teller_or_marketing() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust1 = "cust-ly5-09a";
    let cust2 = "cust-ly5-09b";
    let phone = "+66812345678";
    seed_customer(&db, org, cust1).await;
    seed_phone(&db, org, cust1, phone, true, "ACTIVE").await;
    seed_mirror(&db, org, phone, "GOLD", None).await;

    seed_customer(&db, org, cust2).await;
    // cust2 has no mirror row

    let fake = Arc::new(FakeLoyaltySummarySource::new());

    // As TELLER (ExchangeTransact) -> 200 with mirror tier
    let ans_teller = get(
        &db,
        fake.clone(),
        "teller",
        &format!("/api/v3/views/customer_tier?customer={cust1}"),
    )
    .await;
    assert_eq!(
        ans_teller.status,
        StatusCode::OK,
        "teller view cust1: {}",
        ans_teller.text
    );
    assert_eq!(ans_teller.body["tier"], "GOLD");

    // As MARKETING (MarketingMembers) -> 200 with mirror tier
    let ans_mkt = get(
        &db,
        fake.clone(),
        "marketing",
        &format!("/api/v3/views/customer_tier?customer={cust1}"),
    )
    .await;
    assert_eq!(
        ans_mkt.status,
        StatusCode::OK,
        "marketing view cust1: {}",
        ans_mkt.text
    );
    assert_eq!(ans_mkt.body["tier"], "GOLD");

    // As TELLER for customer without mirror row -> 200 with tier: null
    let ans_nomirror = get(
        &db,
        fake.clone(),
        "teller",
        &format!("/api/v3/views/customer_tier?customer={cust2}"),
    )
    .await;
    assert_eq!(
        ans_nomirror.status,
        StatusCode::OK,
        "teller view cust2: {}",
        ans_nomirror.text
    );
    assert!(
        ans_nomirror.body["tier"].is_null(),
        "tier must be null when no mirror row"
    );

    db.finish().await;
}

/// Row 10: customer_tier as a role without either permission
#[tokio::test]
async fn customer_tier_viewed_by_role_without_permission_is_refused() {
    let db = TestDb::start().await;
    let org = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let cust = "cust-ly5-10";
    seed_customer(&db, org, cust).await;

    let fake = Arc::new(FakeLoyaltySummarySource::new());

    // As FINANCE (no ExchangeTransact, no MarketingMembers) -> 400 roleNotAllowed
    let ans_fin = get(
        &db,
        fake.clone(),
        "finance",
        &format!("/api/v3/views/customer_tier?customer={cust}"),
    )
    .await;
    assert_eq!(
        ans_fin.status,
        StatusCode::BAD_REQUEST,
        "finance should get 400: {}",
        ans_fin.text
    );
    let code = ans_fin.body["error"]["code"].as_str().unwrap_or("");
    assert!(
        code == "roleNotAllowed" || code == "roleNotAllowedHere",
        "unexpected code: {code}"
    );

    // As CUSTOMER -> 400 roleNotAllowed
    let ans_cust = get(
        &db,
        fake.clone(),
        &format!("customer:{cust}"),
        &format!("/api/v3/views/customer_tier?customer={cust}"),
    )
    .await;
    assert_eq!(
        ans_cust.status,
        StatusCode::BAD_REQUEST,
        "customer should get 400: {}",
        ans_cust.text
    );
    let code = ans_cust.body["error"]["code"].as_str().unwrap_or("");
    assert!(
        code == "roleNotAllowed" || code == "roleNotAllowedHere",
        "unexpected code: {code}"
    );

    db.finish().await;
}

/// Row 11: customer_tier for an unknown customer of the organisation
#[tokio::test]
async fn customer_tier_for_unknown_customer_answers_not_found() {
    let db = TestDb::start().await;
    let fake = Arc::new(FakeLoyaltySummarySource::new());

    // As TELLER for unknown customer of the organisation -> 404
    let ans = get(
        &db,
        fake.clone(),
        "teller",
        "/api/v3/views/customer_tier?customer=non-existent-customer-999",
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::NOT_FOUND,
        "expected 404 Not Found: {}",
        ans.text
    );
    let code = ans.body["error"]["code"].as_str().unwrap_or("");
    assert!(
        code == "notFound" || code == "customerNotFound",
        "unexpected code: {code}"
    );

    db.finish().await;
}

/// Specification 12: organisation isolation for customer_tier and mirror read
#[tokio::test]
async fn organisation_isolation_customer_tier_and_mirror_read() {
    let db = TestDb::start().await;
    let org1 = Uuid::parse_str(quote_common::ORG_1).unwrap();
    let org2_str = "00000000-0000-0000-0000-000000000002";
    let org2 = Uuid::parse_str(org2_str).unwrap();
    db.second_organisation(org2_str, "ORG2", "XC-B02").await;

    let cust_org2 = "cust-ly5-org2";
    seed_customer(&db, org2, cust_org2).await;

    let fake = Arc::new(FakeLoyaltySummarySource::new());

    // 1. customer_tier for a customer belonging to another organization -> 404 (not a customer of this org, §8)
    let ans1 = get(
        &db,
        fake.clone(),
        "teller",
        &format!("/api/v3/views/customer_tier?customer={cust_org2}"),
    )
    .await;
    assert_eq!(
        ans1.status,
        StatusCode::NOT_FOUND,
        "expected 404 for other org's customer: {}",
        ans1.text
    );
    let code1 = ans1.body["error"]["code"].as_str().unwrap_or("");
    assert!(
        code1 == "notFound" || code1 == "customerNotFound",
        "unexpected code: {code1}"
    );

    // 2. mirror row under another organisation must not become this customer's tier
    let cust_org1 = "cust-ly5-org1";
    let phone = "+66812349999";
    seed_customer(&db, org1, cust_org1).await;
    seed_phone(&db, org1, cust_org1, phone, true, "ACTIVE").await;
    // Seed mirror row with this phone in ORG_2 as DIAMOND, but NO mirror row in ORG_1
    seed_mirror(&db, org2, phone, "DIAMOND", None).await;

    // As teller in ORG_1, query customer_tier for cust_org1 -> tier must be null (mirror row from ORG_2 must not leak)
    let ans2 = get(
        &db,
        fake.clone(),
        "teller",
        &format!("/api/v3/views/customer_tier?customer={cust_org1}"),
    )
    .await;
    assert_eq!(
        ans2.status,
        StatusCode::OK,
        "teller view cust_org1: {}",
        ans2.text
    );
    assert!(
        ans2.body["tier"].is_null(),
        "tier must be null; mirror row from ORG_2 must not leak into ORG_1 customer"
    );

    // As cust_org1 calling my_membership -> tier must be null (mirror row from ORG_2 must not leak)
    let ans3 = get(
        &db,
        fake.clone(),
        &format!("customer:{cust_org1}"),
        "/api/v3/views/my_membership",
    )
    .await;
    assert_eq!(
        ans3.status,
        StatusCode::OK,
        "my_membership cust_org1: {}",
        ans3.text
    );
    assert!(
        ans3.body["tier"].is_null(),
        "my_membership tier must be null; mirror row from ORG_2 must not leak"
    );

    db.finish().await;
}
