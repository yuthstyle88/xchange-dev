//! Up-front tests for C05 `GET /api/v3/views/my_benefits` (design `marketing-campaigns.md` §6.3; O-3 decided on
//! #564 issuecomment-6262478258). Written before the implementation by claude/studio-claude-1 on the BE lane
//! controller claude/a90ab16c's order. Do not edit, delete or weaken.
//!
//! Behaviours locked:
//! 1. a customer sees their own usable personal coupon: label, summary, expiry, and never its code;
//! 2. used, void, expired, anonymous, another customer's, and inactive-version coupons are not listed;
//! 3. open rules of ACTIVE versions on APP (or no channel) are listed; DESK-only, coupon-only, not-started, ended
//!    and paused ones are not;
//! 4. a tier rule is listed only for the customer's mirrored tier;
//! 5. a segment rule needs the version's snapshot AND granted MARKETING consent;
//! 6. line conditions come back as values, the branches as code and name;
//! 7. another organisation's rules and coupons never show;
//! 8. a staff caller, a customer asking for someone else, and another organisation's session are refused;
//! 9. the view writes nothing and reserves nothing;
//! 10. nothing to show answers two empty lists;
//! 11. the generated `my_benefits.ts` is committed, carries the fields of §6.3, and no code field.

mod common;
mod quote_common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration, Utc};
use quote_common::{ORG_1, TestDb};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiKeys};
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const AS_HEADER: &str = "x-test-as";
const ROUTE: &str = "/api/v3/views/my_benefits";
const ORG_2: &str = "00000000-0000-0000-0000-000000000002";
const BRANCH: &str = "XC-C05-B1";
const TEST_KEY: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

// ── Sessions ────────────────────────────────────────────────────────────────

fn customer_actor(org: &str, customer: &str) -> xc_auth::domain::access::Actor {
    xc_auth::domain::access::Actor::new(
        &format!("user-{customer}"),
        org,
        vec![xc_auth::domain::access::ScopedRole::new(
            xc_auth::domain::access::Role::Customer,
            None,
            Utc::now() - Duration::hours(1),
            None,
        )],
        Some(customer),
    )
}

fn callers_of(spec: &str) -> Callers {
    if let Some(customer) = spec.strip_prefix("customer:") {
        return Callers {
            user: Some(customer_actor(ORG_1, customer)),
            staff_user: None,
        };
    }
    if let Some(customer) = spec.strip_prefix("other_org_customer:") {
        return Callers {
            user: Some(customer_actor(ORG_2, customer)),
            staff_user: None,
        };
    }
    if spec == "teller" {
        let branch = BranchId::new(BRANCH);
        let user = xc_auth::domain::access::Actor::new(
            "emp-teller-c05",
            ORG_1,
            vec![xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Teller,
                Some(branch.clone()),
                Utc::now() - Duration::hours(1),
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
    panic!("unknown test caller spec: {spec}");
}

struct TestSessions;

impl SessionReader for TestSessions {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        Box::pin(async move { callers_of(&spec) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    common::install_test_pii_keys();
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(TestSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS_HEADER, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn mine(db: &TestDb, customer: &str) -> Answer {
    let ans = get(db, &format!("customer:{customer}"), ROUTE).await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "my_benefits expected 200: {}",
        ans.text
    );
    ans
}

// ── Seeding ─────────────────────────────────────────────────────────────────

struct World {
    db: TestDb,
    user: Uuid,
}

async fn world() -> World {
    let db = TestDb::start().await;
    db.exec(&format!(
        "INSERT INTO branches (id, name, code) VALUES ('{BRANCH}', 'Silom', 'SLM')"
    ))
    .await;
    let user = seed_user(&db, ORG_1).await;
    World { db, user }
}

async fn seed_user(db: &TestDb, org: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'tester-c05', 'ACTIVE', 'system')",
    )
    .bind(id)
    .bind(org)
    .execute(&db.pool)
    .await
    .expect("seed user");
    id
}

async fn seed_customer(db: &TestDb, org: &str, id: &str) {
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $1, $3, 'LIVE', now())",
    )
    .bind(id)
    .bind(org)
    .bind(format!("Customer {id}"))
    .execute(&db.pool)
    .await
    .expect("insert customer");
}

/// A verified phone plus a mirrored tier for it (`MembershipMirror::tier_of` reads these two rows).
async fn seed_tier(db: &TestDb, customer: &str, e164: &str, tier: &str) {
    let keys = PiiKeys::parse(TEST_KEY).expect("valid test key");
    let hash = LookupHasher::new(&keys)
        .hash(IdentityType::Phone, e164)
        .to_vec();
    let sealed = xc_kernel::pii::PiiCipher::new(Arc::new(keys))
        .encrypt(format!("{customer}PHONE").as_bytes(), e164.as_bytes());
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status) \
         VALUES ($1, $2::uuid, $3, 'PHONE', $4, $5, $6, now(), 'ACTIVE')",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(customer)
    .bind(sealed.bytes)
    .bind(sealed.key_id)
    .bind(&hash)
    .execute(&db.pool)
    .await
    .expect("insert phone identity");
    sqlx::query(
        "INSERT INTO customer_tier_mirror (organization_id, phone_lookup_hash, tier_code, tier_rule_id, \
             tier_rule_version, effective_at, effective_until, loyalty_seq, synced_at) \
         VALUES ($1::uuid, $2, $3, gen_random_uuid(), 1, now(), NULL, 1, now())",
    )
    .bind(ORG_1)
    .bind(&hash)
    .bind(tier)
    .execute(&db.pool)
    .await
    .expect("seed customer_tier_mirror");
}

async fn seed_consent(db: &TestDb, customer: &str, granted: bool) {
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, \
             effective_at, evidence_ref) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', $4, now() - interval '1 hour', 'test-c05')",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(customer)
    .bind(granted)
    .execute(&db.pool)
    .await
    .expect("insert consent");
}

async fn seed_snapshot(w: &World, members: &[&str]) -> Uuid {
    let definition = Uuid::new_v4();
    let snapshot = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, '{}'::jsonb, $4, now())",
    )
    .bind(definition)
    .bind(ORG_1)
    .bind(format!("seg-{definition}"))
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert segment definition");
    sqlx::query(
        "INSERT INTO segment_snapshots (id, organization_id, definition_id, as_of, member_count, taken_by) \
         VALUES ($1, $2::uuid, $3, now(), $4, $5)",
    )
    .bind(snapshot)
    .bind(ORG_1)
    .bind(definition)
    .bind(members.len() as i32)
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert segment snapshot");
    for m in members {
        sqlx::query(
            "INSERT INTO segment_snapshot_members (snapshot_id, customer_id, metrics) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(snapshot)
        .bind(*m)
        .execute(&w.db.pool)
        .await
        .expect("insert snapshot member");
    }
    snapshot
}

struct VersionSpec<'a> {
    org: &'a str,
    branch: &'a str,
    name: &'a str,
    status: &'a str,
    starts_in_hours: i64,
    ends_in_hours: i64,
    segment_snapshot: Option<Uuid>,
}

impl<'a> VersionSpec<'a> {
    fn active(name: &'a str) -> Self {
        Self {
            org: ORG_1,
            branch: BRANCH,
            name,
            status: "APPROVED",
            starts_in_hours: -24,
            ends_in_hours: 24 * 30,
            segment_snapshot: None,
        }
    }
}

/// A campaign with one version, its branch link and its budget row; answers the version id.
async fn seed_version(db: &TestDb, user: Uuid, spec: VersionSpec<'_>) -> Uuid {
    let campaign = Uuid::new_v4();
    let version = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign)
    .bind(spec.org)
    .bind(spec.name)
    .bind(user)
    .execute(&db.pool)
    .await
    .expect("insert campaign");
    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, segment_snapshot_id, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, $4, now() + make_interval(hours => $5), now() + make_interval(hours => $6), \
             10000, 'default', 1, $7, 'hash-c05', now(), $8, now())",
    )
    .bind(version)
    .bind(spec.org)
    .bind(campaign)
    .bind(spec.status)
    .bind(spec.starts_in_hours as i32)
    .bind(spec.ends_in_hours as i32)
    .bind(spec.segment_snapshot)
    .bind(user)
    .execute(&db.pool)
    .await
    .expect("insert campaign version");
    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version)
        .bind(campaign)
        .execute(&db.pool)
        .await
        .expect("set current version");
    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version)
        .bind(spec.branch)
        .execute(&db.pool)
        .await
        .expect("insert campaign branch");
    sqlx::query("INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)")
        .bind(version)
        .execute(&db.pool)
        .await
        .expect("insert campaign budget");
    version
}

async fn seed_rule(db: &TestDb, version: Uuid, eligibility: Value, requires_coupon: bool) -> Uuid {
    let rule = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, \
             caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, '{\"delta\": \"0.05\"}'::jsonb, '{}'::jsonb, $4, \
             (SELECT coalesce(max(rule_no), 0) + 1 FROM benefit_rules WHERE campaign_version_id = $2))",
    )
    .bind(rule)
    .bind(version)
    .bind(eligibility)
    .bind(requires_coupon)
    .execute(&db.pool)
    .await
    .expect("insert benefit rule");
    rule
}

async fn seed_coupon(
    w: &World,
    org: &str,
    rule: Uuid,
    code: &str,
    status: &str,
    customer: Option<&str>,
    expires_in_hours: i64,
) -> Uuid {
    common::install_test_pii_keys();
    let keys = xc_customer::get_pii_keys().expect("test PII keys installed");
    let code_hash = CouponCodeHasher::new(&keys).hash(code);
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, issued_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, now() + make_interval(hours => $7), $8, now())",
    )
    .bind(id)
    .bind(org)
    .bind(rule)
    .bind(&code_hash[..])
    .bind(status)
    .bind(customer)
    .bind(expires_in_hours as i32)
    .bind(w.user)
    .execute(&w.db.pool)
    .await
    .expect("insert coupon");
    id
}

fn ids(list: &Value, key: &str) -> Vec<String> {
    list.as_array()
        .unwrap_or_else(|| panic!("expected an array, got {list}"))
        .iter()
        .map(|item| item[key].as_str().unwrap_or_default().to_string())
        .collect()
}

// ── 1 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_customer_sees_their_usable_personal_coupon_without_its_code() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-a").await;
    let version = seed_version(&w.db, w.user, VersionSpec::active("Songkran bonus")).await;
    let rule = seed_rule(&w.db, version, json!({ "requiresCoupon": true }), true).await;
    let coupon = seed_coupon(
        &w,
        ORG_1,
        rule,
        "C05SECRETCODE1",
        "ISSUED",
        Some("cust-c05-a"),
        48,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-a").await;
    assert_eq!(
        ids(&ans.body["coupons"], "couponId"),
        vec![coupon.to_string()],
        "{}",
        ans.text
    );
    let item = &ans.body["coupons"][0];
    assert_eq!(item["label"], "Songkran bonus");
    assert_eq!(item["summary"]["benefitType"], "RATE_IMPROVEMENT");
    assert_eq!(item["summary"]["benefitClass"], "PRICE");
    assert_eq!(item["summary"]["delta"], "0.05", "decimals are strings");
    assert!(item["expiresAt"].is_string(), "{}", ans.text);
    assert!(
        !ans.text.contains("C05SECRETCODE1"),
        "the code is shown once, at issue (D-9): {}",
        ans.text
    );
    assert!(
        !ans.text.contains("codeHash") && !ans.text.contains("code_hash"),
        "{}",
        ans.text
    );
    assert!(
        item.get("code").is_none(),
        "no code field at all: {}",
        ans.text
    );
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        Vec::<String>::new(),
        "a coupon-only rule is not an open benefit: {}",
        ans.text
    );
    w.db.finish().await;
}

// ── 2 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn used_void_expired_anonymous_others_and_inactive_version_coupons_are_not_listed() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-b").await;
    seed_customer(&w.db, ORG_1, "cust-c05-other").await;
    let live = seed_version(&w.db, w.user, VersionSpec::active("Live")).await;
    let live_rule = seed_rule(&w.db, live, json!({ "requiresCoupon": true }), true).await;
    let paused = seed_version(
        &w.db,
        w.user,
        VersionSpec {
            status: "PAUSED",
            ..VersionSpec::active("Paused")
        },
    )
    .await;
    let paused_rule = seed_rule(&w.db, paused, json!({ "requiresCoupon": true }), true).await;
    let ended = seed_version(
        &w.db,
        w.user,
        VersionSpec {
            starts_in_hours: -48,
            ends_in_hours: -1,
            ..VersionSpec::active("Ended")
        },
    )
    .await;
    let ended_rule = seed_rule(&w.db, ended, json!({ "requiresCoupon": true }), true).await;

    let keep = seed_coupon(
        &w,
        ORG_1,
        live_rule,
        "KEEP-1",
        "ISSUED",
        Some("cust-c05-b"),
        48,
    )
    .await;
    seed_coupon(
        &w,
        ORG_1,
        live_rule,
        "USED-1",
        "REDEEMED",
        Some("cust-c05-b"),
        48,
    )
    .await;
    seed_coupon(
        &w,
        ORG_1,
        live_rule,
        "VOID-1",
        "VOID",
        Some("cust-c05-b"),
        48,
    )
    .await;
    seed_coupon(
        &w,
        ORG_1,
        live_rule,
        "OLD-1",
        "ISSUED",
        Some("cust-c05-b"),
        -1,
    )
    .await;
    seed_coupon(&w, ORG_1, live_rule, "ANON-1", "ISSUED", None, 48).await;
    seed_coupon(
        &w,
        ORG_1,
        live_rule,
        "THEIRS-1",
        "ISSUED",
        Some("cust-c05-other"),
        48,
    )
    .await;
    seed_coupon(
        &w,
        ORG_1,
        paused_rule,
        "PAUSED-1",
        "ISSUED",
        Some("cust-c05-b"),
        48,
    )
    .await;
    seed_coupon(
        &w,
        ORG_1,
        ended_rule,
        "ENDED-1",
        "ISSUED",
        Some("cust-c05-b"),
        48,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-b").await;
    assert_eq!(
        ids(&ans.body["coupons"], "couponId"),
        vec![keep.to_string()],
        "only an ISSUED, unexpired coupon of this customer, on an ACTIVE version: {}",
        ans.text
    );
    w.db.finish().await;
}

// ── 3 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn open_app_rules_are_listed_and_desk_only_coupon_only_and_inactive_rules_are_not() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-c").await;
    let any_channel = seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("Any channel")).await,
        json!({}),
        false,
    )
    .await;
    let app = seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("App only")).await,
        json!({ "channels": ["APP"] }),
        false,
    )
    .await;
    seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("Desk only")).await,
        json!({ "channels": ["DESK"] }),
        false,
    )
    .await;
    seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("Coupon only")).await,
        json!({}),
        true,
    )
    .await;
    for (name, status, starts, ends) in [
        ("Not started", "APPROVED", 24, 48),
        ("Ended", "APPROVED", -48, -1),
        ("Paused", "PAUSED", -24, 48),
        ("Superseded", "SUPERSEDED", -24, 48),
    ] {
        seed_rule(
            &w.db,
            seed_version(
                &w.db,
                w.user,
                VersionSpec {
                    status,
                    starts_in_hours: starts,
                    ends_in_hours: ends,
                    ..VersionSpec::active(name)
                },
            )
            .await,
            json!({}),
            false,
        )
        .await;
    }

    let ans = mine(&w.db, "cust-c05-c").await;
    let mut got = ids(&ans.body["benefits"], "ruleId");
    got.sort();
    let mut want = vec![any_channel.to_string(), app.to_string()];
    want.sort();
    assert_eq!(got, want, "{}", ans.text);
    let labels: Vec<&str> = ans.body["benefits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"Any channel") && labels.contains(&"App only"),
        "{}",
        ans.text
    );
    assert!(
        ans.body["benefits"][0]["endsAt"].is_string(),
        "{}",
        ans.text
    );
    w.db.finish().await;
}

// ── 4 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_tier_rule_is_listed_only_for_the_mirrored_tier() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-gold").await;
    seed_customer(&w.db, ORG_1, "cust-c05-silver").await;
    seed_customer(&w.db, ORG_1, "cust-c05-none").await;
    seed_tier(&w.db, "cust-c05-gold", "+66810000501", "GOLD").await;
    seed_tier(&w.db, "cust-c05-silver", "+66810000502", "SILVER").await;
    let gold = seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("Gold members")).await,
        json!({ "tiers": ["GOLD"] }),
        false,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-gold").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        vec![gold.to_string()],
        "{}",
        ans.text
    );
    let ans = mine(&w.db, "cust-c05-silver").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        Vec::<String>::new(),
        "{}",
        ans.text
    );
    let ans = mine(&w.db, "cust-c05-none").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        Vec::<String>::new(),
        "no mirrored tier never matches a tier rule: {}",
        ans.text
    );
    w.db.finish().await;
}

// ── 5 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_segment_rule_needs_snapshot_membership_and_marketing_consent() {
    let w = world().await;
    for c in ["cust-c05-in", "cust-c05-noconsent", "cust-c05-out"] {
        seed_customer(&w.db, ORG_1, c).await;
    }
    seed_consent(&w.db, "cust-c05-in", true).await;
    seed_consent(&w.db, "cust-c05-noconsent", false).await;
    seed_consent(&w.db, "cust-c05-out", true).await;
    let snapshot = seed_snapshot(&w, &["cust-c05-in", "cust-c05-noconsent"]).await;
    let seg = seed_rule(
        &w.db,
        seed_version(
            &w.db,
            w.user,
            VersionSpec {
                segment_snapshot: Some(snapshot),
                ..VersionSpec::active("Segment")
            },
        )
        .await,
        json!({ "segment": true }),
        false,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-in").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        vec![seg.to_string()],
        "{}",
        ans.text
    );
    let ans = mine(&w.db, "cust-c05-noconsent").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        Vec::<String>::new(),
        "MARKETING consent not granted stops segment eligibility (D-13): {}",
        ans.text
    );
    let ans = mine(&w.db, "cust-c05-out").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        Vec::<String>::new(),
        "not in the version's snapshot: {}",
        ans.text
    );
    w.db.finish().await;
}

// ── 6 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn line_conditions_come_back_as_values_and_branches_as_codes_and_names() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-d").await;
    let rule = seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("Big USD buyers")).await,
        json!({ "minThb": "1000.00", "currencies": ["USD"], "shopSide": "BUY", "channels": ["APP", "DESK"] }),
        false,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-d").await;
    assert_eq!(
        ids(&ans.body["benefits"], "ruleId"),
        vec![rule.to_string()],
        "line conditions are shown, not evaluated: {}",
        ans.text
    );
    let conditions = &ans.body["benefits"][0]["conditions"];
    assert_eq!(conditions["minThb"], "1000.00", "{}", ans.text);
    assert_eq!(conditions["currencies"], json!(["USD"]), "{}", ans.text);
    assert_eq!(conditions["shopSide"], "BUY", "{}", ans.text);
    assert_eq!(
        conditions["branches"],
        json!([{ "branchCode": "SLM", "name": "Silom" }]),
        "{}",
        ans.text
    );

    let plain = seed_rule(
        &w.db,
        seed_version(&w.db, w.user, VersionSpec::active("Plain")).await,
        json!({}),
        false,
    )
    .await;
    let ans = mine(&w.db, "cust-c05-d").await;
    let item = ans.body["benefits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["ruleId"] == plain.to_string())
        .unwrap_or_else(|| panic!("plain rule listed: {}", ans.text))
        .clone();
    assert!(item["conditions"]["minThb"].is_null(), "{}", ans.text);
    assert!(item["conditions"]["currencies"].is_null(), "{}", ans.text);
    assert!(item["conditions"]["shopSide"].is_null(), "{}", ans.text);
    w.db.finish().await;
}

// ── 7 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn another_organisations_rules_and_coupons_never_show() {
    let w = world().await;
    w.db.second_organisation(ORG_2, "X2", "XC-C05-O2").await;
    seed_customer(&w.db, ORG_1, "cust-c05-e").await;
    let user_2 = seed_user(&w.db, ORG_2).await;
    let theirs = seed_version(
        &w.db,
        user_2,
        VersionSpec {
            org: ORG_2,
            branch: "XC-C05-O2",
            ..VersionSpec::active("Other org")
        },
    )
    .await;
    seed_rule(&w.db, theirs, json!({}), false).await;
    let theirs_coupon_rule =
        seed_rule(&w.db, theirs, json!({ "requiresCoupon": true }), true).await;
    seed_coupon(
        &w,
        ORG_2,
        theirs_coupon_rule,
        "ORG2-1",
        "ISSUED",
        Some("cust-c05-e"),
        48,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-e").await;
    assert_eq!(ans.body["benefits"], json!([]), "{}", ans.text);
    assert_eq!(ans.body["coupons"], json!([]), "{}", ans.text);
    w.db.finish().await;
}

// ── 8 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_staff_caller_another_customer_and_another_organisations_session_are_refused() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-f").await;
    seed_customer(&w.db, ORG_1, "cust-c05-g").await;

    let ans = get(
        &w.db,
        "customer:cust-c05-f",
        &format!("{ROUTE}?customer=cust-c05-g"),
    )
    .await;
    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(ans.body["error"]["code"], "outOfScope", "{}", ans.text);

    let ans = get(
        &w.db,
        "customer:cust-c05-f",
        &format!("{ROUTE}?customer=cust-c05-f"),
    )
    .await;
    assert_eq!(
        ans.status,
        StatusCode::OK,
        "naming yourself is allowed: {}",
        ans.text
    );

    let ans = get(&w.db, "other_org_customer:cust-c05-f", ROUTE).await;
    assert_eq!(ans.status, StatusCode::FORBIDDEN, "{}", ans.text);
    assert_eq!(ans.body["error"]["code"], "outOfScope", "{}", ans.text);

    let ans = get(&w.db, "teller", ROUTE).await;
    assert_eq!(
        ans.status,
        StatusCode::FORBIDDEN,
        "a staff caller has no CustomerSelf: {}",
        ans.text
    );
    w.db.finish().await;
}

// ── 9 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_view_writes_nothing_and_reserves_nothing() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-h").await;
    let version = seed_version(&w.db, w.user, VersionSpec::active("Reserve check")).await;
    seed_rule(&w.db, version, json!({}), false).await;
    let coupon_rule = seed_rule(&w.db, version, json!({ "requiresCoupon": true }), true).await;
    seed_coupon(
        &w,
        ORG_1,
        coupon_rule,
        "NORES-1",
        "ISSUED",
        Some("cust-c05-h"),
        48,
    )
    .await;

    let ans = mine(&w.db, "cust-c05-h").await;
    assert_eq!(
        ans.body["benefits"].as_array().map(Vec::len),
        Some(1),
        "{}",
        ans.text
    );
    assert_eq!(
        ans.body["coupons"].as_array().map(Vec::len),
        Some(1),
        "{}",
        ans.text
    );
    assert_eq!(
        w.db.count("SELECT count(*) FROM benefit_reservations")
            .await,
        0,
        "no reservation (O-3)"
    );
    assert_eq!(
        w.db.count("SELECT count(*) FROM benefit_redemptions").await,
        0
    );
    assert_eq!(
        w.db.count("SELECT count(*) FROM quotes").await,
        0,
        "no quote (O-3)"
    );
    let consumed: Decimal = sqlx::query_scalar(
        "SELECT consumed_thb FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(version)
    .fetch_one(&w.db.pool)
    .await
    .expect("budget row");
    assert_eq!(consumed, Decimal::ZERO, "the budget is untouched");
    assert_eq!(
        w.db.count("SELECT count(*) FROM coupons WHERE status = 'ISSUED'")
            .await,
        1,
        "the coupon is not consumed"
    );
    assert_eq!(ans.text.matches("NORES-1").count(), 0, "{}", ans.text);
    w.db.finish().await;
}

// ── 10 ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn nothing_to_show_answers_two_empty_lists() {
    let w = world().await;
    seed_customer(&w.db, ORG_1, "cust-c05-i").await;
    let ans = mine(&w.db, "cust-c05-i").await;
    assert_eq!(
        ans.body,
        json!({ "coupons": [], "benefits": [] }),
        "{}",
        ans.text
    );
    w.db.finish().await;
}

// ── 11 ──────────────────────────────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("c05-my-benefits-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exported() -> String {
    let view = VIEWS
        .iter()
        .find(|v| v.name == "my_benefits")
        .unwrap_or_else(|| panic!("my_benefits is in the generated view table"));
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export my_benefits: {e}"));
    std::fs::read_to_string(scratch.0.join("my_benefits.ts"))
        .unwrap_or_else(|e| panic!("my_benefits writes my_benefits.ts: {e}"))
}

#[test]
fn the_generated_type_is_committed_and_carries_no_code_field() {
    let ts = exported();
    for field in [
        "coupons: Array<MyCouponDto>",
        "benefits: Array<MyBenefitDto>",
        "couponId: string",
        "ruleId: string",
        "label: string",
        "expiresAt: string",
        "endsAt: string",
        "benefitType: string",
        "minThb: string | null",
        "branchCode: string",
    ] {
        assert!(ts.contains(field), "my_benefits.ts has {field}:\n{ts}");
    }
    assert!(
        !ts.contains("code: string") && !ts.contains("codeHash"),
        "no coupon code field:\n{ts}"
    );
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/my_benefits.ts");
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/my_benefits.ts: {e}"));
    assert_eq!(committed, ts, "my_benefits.ts is what the DTOs generate");
}
