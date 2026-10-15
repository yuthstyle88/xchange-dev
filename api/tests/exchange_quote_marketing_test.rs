//! MK-5 #1403 up-front tests: quote campaigns and redemption effect
//! (design `docs/design/blueprint/marketing-campaigns.md` §4.1, §4.2, §5, §6, §7, §9.1 row MK-5; ticket #1403 §4, §8, §9).
//!
//! Written before the implementation by the test-writer (gemini/studio-14, ordered by BE lane controller claude/ad05bf8d).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Behaviours locked (§9 + §9.1 MK-5):
//! 1. `quote_with_eligible_rule_updates_final_rate_reserves_and_populates_snapshot`:
//!    A quote with an eligible rule has final_rate ≠ base_rate, a reservation in `benefit_reservations`,
//!    and `benefit_snapshot` populated with `applied: [...]`; `QuoteDto.benefits` lists applied benefits.
//! 2. `quote_with_no_rule_leaves_today_quote_unchanged`:
//!    With no rule, today's quote is unchanged: `final_rate == base_rate`, `benefits: []`,
//!    and `benefit_snapshot` has `engineVersion: 1` with empty `applied`.
//! 3. `anonymous_quote_evaluates_only_rules_with_no_customer_condition`:
//!    Anonymous quote (`customer_id: None`) evaluates only rules with no customer condition (§4.3);
//!    customer-conditioned rules are skipped.
//! 4. `customer_withdrew_marketing_consent_skips_segment_rule`:
//!    Customer who withdrew marketing consent (`marketing_allowed == false`) skips segment rules at quote (§7).
//! 5. `campaign_paused_with_live_reservations_honoured_at_completion_and_skips_new_quotes`:
//!    Campaign paused with live reservations: new quotes skip the paused campaign, but live reservations
//!    quoted before the pause are honoured and redeemed at completion (§7).
//! 6. `quote_expired_budget_used_up_completion_refuses_benefit_limit_reached_and_rolls_back`:
//!    Quote expired and budget since used up: completion refuses `benefitLimitReached` and rolls back (§7).
//! 7. `completion_rollback_leaves_budget_as_before`:
//!    A completion rollback leaves the budget as before (no redemptions committed, consumed budget untouched).
//! 8. `completion_through_exchange_effects_redeems_once`:
//!    Completion through XS-0 `ExchangeEffects` (phase `Redeem`) redeems once.
//! 9. `same_completion_retried_with_same_key_redeems_once`:
//!    Same completion retried with the same key redeems once (idempotent, §7).
//! 10. `posted_reversal_marks_redemption_reversed_returns_budget_and_reissues_active_coupon`:
//!     A posted reversal marks the redemption `REVERSED`, returns budget and use count, and re-issues a
//!     single-use coupon of a still-ACTIVE version (§7, §9.1).
//! 11. `demo_exchange_evaluated_without_touching_live_budget`:
//!     DEMO exchange (`Mode::Demo`): evaluated, but LIVE `campaign_budgets` is not touched (§7, [proposed]).
//! 12. `adapters_fallbacks`:
//!     The three adapters unit tests with their fallbacks (`None`, `false`) (#1403 §10 step 2).
//! 13. `simulator_line_equals_quote_in_line`:
//!     Parity with MK-4 CampaignSimulator: for the same campaign, branch, currency, side and quantity,
//!     the simulator line equals the quote_in line (rate_after and benefit amount compared as strings) (§4 criterion 1).

mod common;
mod quote_common;

use std::sync::Arc;

use chrono::Utc;
use quote_common::*;
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_marketing::application::simulator::{CampaignSimulator, SimulationScenario};
use xc_marketing::domain::engine::consent_gate::ConsentGate;
use xc_marketing::domain::engine::referral_source::ReferralSource;
use xc_marketing::domain::engine::tier_source::TierSource;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::effects::{
    EffectPhase, ExchangeCompletedFacts, PostedReversal,
};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::bootstrap::AppState;
use xchange::bootstrap::exchange_effects;
use xchange::infrastructure::marketing::consent_gate::MarketingConsentGate;
use xchange::infrastructure::marketing::referral_source::MarketingReferralSource;
use xchange::infrastructure::marketing::tier_source::MarketingTierSource;
use xchange::presentation::dto::v3::quote::QuoteDto;

fn teller() -> SaveActor {
    SaveActor {
        id: "teller-mk5".to_owned(),
        username: "user-teller-mk5".to_owned(),
        employee_id: Some("teller-mk5".to_owned()),
        role: Role::Front,
    }
}

fn usd_buy_line(qty: &str) -> QuoteRequestLine {
    QuoteRequestLine {
        currency: CurrencyCode::parse("USD").expect("USD"),
        shop_side: Side::Buy,
        quantity: ForeignAmount::from_decimal(Decimal::from_str_exact(qty).expect("decimal qty")),
        denomination_group: GroupCode::all(),
        fee_thb: None,
    }
}

fn request(branch: &str, customer: Option<&str>, lines: Vec<QuoteRequestLine>) -> QuoteRequest {
    QuoteRequest {
        branch_id: BranchId::new(branch),
        customer_id: customer.map(|c| c.to_owned()),
        channel: QuoteChannel::Desk,
        lines,
    }
}

// TEMPORARY until MK-6's M3 (#1404): replaced by the real migration before these tests are final
async fn ensure_m3_tables(db: &TestDb) {
    db.exec(
        "CREATE TABLE IF NOT EXISTS segment_definitions (
            id uuid PRIMARY KEY,
            organization_id uuid NOT NULL,
            name text NOT NULL,
            version int NOT NULL CHECK (version > 0),
            rule jsonb NOT NULL,
            created_by uuid NOT NULL,
            created_at timestamptz NOT NULL,
            UNIQUE (organization_id, name, version)
        )",
    )
    .await;

    db.exec(
        "CREATE TABLE IF NOT EXISTS segment_snapshots (
            id uuid PRIMARY KEY,
            organization_id uuid NOT NULL,
            definition_id uuid NOT NULL REFERENCES segment_definitions(id),
            as_of timestamptz NOT NULL,
            member_count int NOT NULL,
            taken_by uuid NOT NULL
        )",
    )
    .await;

    db.exec(
        "CREATE TABLE IF NOT EXISTS segment_snapshot_members (
            snapshot_id uuid NOT NULL REFERENCES segment_snapshots(id),
            customer_id text NOT NULL REFERENCES customers(id),
            metrics jsonb NOT NULL,
            PRIMARY KEY (snapshot_id, customer_id)
        )",
    )
    .await;
}

struct TestCampaign {
    version_id: Uuid,
    rule_id: Uuid,
}

async fn seed_test_user(db: &TestDb) -> Uuid {
    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'tester-mk5', 'ACTIVE', 'system') ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .bind(ORG_1)
    .execute(&db.pool)
    .await
    .expect("seed user");
    user_id
}

async fn seed_campaign(
    db: &TestDb,
    user_id: Uuid,
    branch: &str,
    budget_thb: &str,
    rule_type: &str,
    eligibility: serde_json::Value,
    calculation: serde_json::Value,
) -> TestCampaign {
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();
    let budget = Decimal::from_str_exact(budget_thb).expect("budget decimal");

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign_id)
    .bind(ORG_1)
    .bind(format!("campaign-{}", Uuid::new_v4()))
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert campaign");

    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', now() - interval '1 day', now() + interval '30 days', \
         $4, 'default', 1, 'hash_mk5', now(), $5, now())",
    )
    .bind(version_id)
    .bind(ORG_1)
    .bind(campaign_id)
    .bind(budget)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(&db.pool)
        .await
        .expect("set current_version_id");

    sqlx::query(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(version_id)
    .bind(branch)
    .execute(&db.pool)
    .await
    .expect("insert branch link");

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, $3, 'PRICE', $4, $5, '{}'::jsonb, false, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(rule_type)
    .bind(eligibility)
    .bind(calculation)
    .execute(&db.pool)
    .await
    .expect("insert rule");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0) \
         ON CONFLICT (campaign_version_id) DO NOTHING",
    )
    .bind(version_id)
    .execute(&db.pool)
    .await
    .expect("insert budget row");

    TestCampaign {
        version_id,
        rule_id,
    }
}

async fn seed_transaction(db: &TestDb, branch: &str, customer: Option<&str>, mode: Mode) -> Uuid {
    let id = Uuid::new_v4();

    let shift_id = match mode {
        Mode::Live => {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN' LIMIT 1",
            )
            .bind(branch)
            .fetch_optional(&db.pool)
            .await
            .expect("check open shift");

            if let Some(s_id) = existing {
                Some(s_id)
            } else {
                let s_id = format!("shift-{}", Uuid::new_v4());
                let teller_user = seed_test_user(db).await;
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
                     VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
                )
                .bind(&s_id)
                .bind(branch)
                .bind(format!("req-{}", Uuid::new_v4()))
                .bind(teller_user)
                .execute(&db.pool)
                .await
                .expect("insert shift");
                Some(s_id)
            }
        }
        Mode::Demo => None,
    };

    let mode_str = match mode {
        Mode::Live => "LIVE",
        Mode::Demo => "DEMO",
    };

    sqlx::query(
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
         recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, 'hash', $8, 'Somchai', 'THA', 'system', 'WALK-IN', '', \
         current_date, now(), now(), 'system', 'COMPLETED', now())",
    )
    .bind(id)
    .bind(ORG_1)
    .bind(format!("DOC-{}", Uuid::new_v4()))
    .bind(branch)
    .bind(mode_str)
    .bind(shift_id)
    .bind(format!("sub-{}", Uuid::new_v4()))
    .bind(customer)
    .execute(&db.pool)
    .await
    .expect("insert transaction");

    id
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 + §9.1 Row 1: Quote with eligible rule updates final_rate, reserves, and populates snapshot
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn quote_with_eligible_rule_updates_final_rate_reserves_and_populates_snapshot() {
    let db = TestDb::start().await;
    let branch = "B-MK5-1";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-101", branch).await;
    let req = request(branch, Some("cust-101"), vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    assert_ne!(
        quote.lines[0].final_rate, quote.lines[0].base_rate,
        "quote with eligible rule must improve final_rate (base: {}, final: {})",
        quote.lines[0].base_rate, quote.lines[0].final_rate
    );

    let res_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM benefit_reservations WHERE quote_id = $1 AND status = 'RESERVED'",
    )
    .bind(quote.id)
    .fetch_one(&db.pool)
    .await
    .expect("count reservations");
    assert_eq!(
        res_count, 1,
        "eligible quote must record a reservation in benefit_reservations"
    );

    let applied = quote
        .benefit_snapshot
        .get("applied")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty());
    assert!(
        applied.is_some(),
        "quote benefit_snapshot must contain applied benefits"
    );

    let dto = QuoteDto::from_domain(quote, 1);
    assert!(
        !dto.benefits.is_empty(),
        "QuoteDto.benefits must list applied benefits from snapshot"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 + §9.1 Row 2: Quote with no rule leaves today's quote unchanged (benefits: [])
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn quote_with_no_rule_leaves_today_quote_unchanged() {
    let db = TestDb::start().await;
    let branch = "B-MK5-2";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;

    let req = request(branch, None, vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    assert_eq!(
        quote.lines[0].final_rate, quote.lines[0].base_rate,
        "without rules final_rate must equal base_rate"
    );

    // Design §4.1: benefit_snapshot must conform to engineVersion: 1 with empty applied
    assert_eq!(
        quote.benefit_snapshot.get("engineVersion"),
        Some(&serde_json::json!(1)),
        "benefit_snapshot must have engineVersion: 1"
    );

    let dto = QuoteDto::from_domain(quote, 1);
    assert_eq!(
        dto.benefits,
        Vec::<serde_json::Value>::new(),
        "with no rule QuoteDto.benefits must be empty []"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Row 3: Anonymous quote evaluates only rules with no customer condition (§4.3)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn anonymous_quote_evaluates_only_rules_with_no_customer_condition() {
    let db = TestDb::start().await;
    let branch = "B-MK5-3";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    // Rule requiring customer tier / condition
    seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"], "tiers": ["GOLD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.75" }),
    )
    .await;

    // Rule with NO customer condition (branch/currency only)
    seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.25" }),
    )
    .await;

    let req = request(branch, None, vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    assert_ne!(
        quote.lines[0].final_rate, quote.lines[0].base_rate,
        "rule with no customer condition must apply to anonymous quote"
    );

    let skipped = quote
        .benefit_snapshot
        .get("skipped")
        .and_then(|v| v.as_array())
        .expect("snapshot skipped array");
    assert!(
        !skipped.is_empty(),
        "customer-conditioned rule must be skipped for anonymous quote"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Row 4: Customer withdrew marketing consent skips segment rule at quote (§7)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn customer_withdrew_marketing_consent_skips_segment_rule() {
    let db = TestDb::start().await;
    ensure_m3_tables(&db).await;
    let branch = "B-MK5-4";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    // Seed segment definition and snapshot (TEMPORARY M3)
    let def_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO segment_definitions (id, organization_id, name, version, rule, created_by, created_at) \
         VALUES ($1, $2::uuid, 'high-value', 1, '{}'::jsonb, $3, now())",
    )
    .bind(def_id)
    .bind(ORG_1)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed segment definition");

    let snap_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO segment_snapshots (id, organization_id, definition_id, as_of, member_count, taken_by) \
         VALUES ($1, $2::uuid, $3, now(), 2, $4)",
    )
    .bind(snap_id)
    .bind(ORG_1)
    .bind(def_id)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed segment snapshot");

    // Segment rule targeting customers with marketing consent
    let camp = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"], "segment": true }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    sqlx::query("UPDATE campaign_versions SET segment_snapshot_id = $1 WHERE id = $2")
        .bind(snap_id)
        .bind(camp.version_id)
        .execute(&db.pool)
        .await
        .expect("link version to segment snapshot");

    // Control: customer with granted marketing consent
    let cust_consenting = "cust-consenting";
    db.customer(cust_consenting, branch).await;
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref, recorded_by) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', true, now(), 'ref-granted', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(cust_consenting)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed consenting customer");

    // Customer who withdrew marketing consent
    let cust_opted_out = "cust-opted-out";
    db.customer(cust_opted_out, branch).await;
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref, recorded_by) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', true, now() - interval '2 days', 'ref-init', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(cust_opted_out)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed initial consent");
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref, recorded_by) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', false, now() - interval '1 day', 'ref-withdrawn', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(cust_opted_out)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed withdrawn consent");

    // Both customers are snapshot members
    sqlx::query(
        "INSERT INTO segment_snapshot_members (snapshot_id, customer_id, metrics) \
         VALUES ($1, $2, '{\"recencyDays\": 5, \"frequency\": 10, \"valueThb\": \"100000.00\"}'::jsonb), \
                ($1, $3, '{\"recencyDays\": 2, \"frequency\": 20, \"valueThb\": \"200000.00\"}'::jsonb)",
    )
    .bind(snap_id)
    .bind(cust_consenting)
    .bind(cust_opted_out)
    .execute(&db.pool)
    .await
    .expect("seed segment snapshot members");

    let service = QuoteService::new(db.pool.clone());

    // Consenting control quote: gets the rule
    let req_consenting = request(branch, Some(cust_consenting), vec![usd_buy_line("100")]);
    let mut tx_consenting = db.pool.begin().await.expect("begin");
    let quote_consenting = service
        .quote_in(&mut tx_consenting, req_consenting, &teller(), Utc::now())
        .await
        .expect("quote_in consenting");
    tx_consenting.commit().await.expect("commit");

    assert_ne!(
        quote_consenting.lines[0].final_rate, quote_consenting.lines[0].base_rate,
        "consenting customer control must receive segment rule benefit"
    );

    // Withdrawn customer quote: skips segment rule
    let req_withdrawn = request(branch, Some(cust_opted_out), vec![usd_buy_line("100")]);
    let mut tx_withdrawn = db.pool.begin().await.expect("begin");
    let quote_withdrawn = service
        .quote_in(&mut tx_withdrawn, req_withdrawn, &teller(), Utc::now())
        .await
        .expect("quote_in withdrawn");
    tx_withdrawn.commit().await.expect("commit");

    assert_eq!(
        quote_withdrawn.lines[0].final_rate, quote_withdrawn.lines[0].base_rate,
        "customer who withdrew consent must skip segment rule and keep base rate"
    );

    let skipped = quote_withdrawn
        .benefit_snapshot
        .get("skipped")
        .and_then(|v| v.as_array())
        .expect("snapshot skipped array");
    assert!(
        skipped
            .iter()
            .any(|s| s.get("reason") == Some(&serde_json::json!("notInSegment"))),
        "segment rule skip reason must be notInSegment when consent is withdrawn: {skipped:?}"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Row 5: Campaign paused with live reservations honoured at completion, no new ones (§7)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn campaign_paused_with_live_reservations_honoured_at_completion_and_skips_new_quotes() {
    let db = TestDb::start().await;
    let branch = "B-MK5-5";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    let c = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    // 1. Quote while ACTIVE: reserves benefit
    db.customer("cust-1", branch).await;
    let req1 = request(branch, Some("cust-1"), vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx1 = db.pool.begin().await.expect("begin");
    let quote1 = service
        .quote_in(&mut tx1, req1, &teller(), Utc::now())
        .await
        .expect("quote_in active");
    tx1.commit().await.expect("commit");

    // 2. Pause the campaign
    sqlx::query("UPDATE campaign_versions SET status = 'PAUSED' WHERE id = $1")
        .bind(c.version_id)
        .execute(&db.pool)
        .await
        .expect("pause campaign");

    // 3. New quote after pause: skips paused campaign
    db.customer("cust-2", branch).await;
    let req2 = request(branch, Some("cust-2"), vec![usd_buy_line("100")]);
    let mut tx2 = db.pool.begin().await.expect("begin");
    let quote2 = service
        .quote_in(&mut tx2, req2, &teller(), Utc::now())
        .await
        .expect("quote_in paused");
    tx2.commit().await.expect("commit");

    assert_eq!(
        quote2.lines[0].final_rate, quote2.lines[0].base_rate,
        "paused campaign must not offer benefits to new quotes"
    );

    // 4. Complete quote 1 through effects: honoured and redeemed
    let txn_id = seed_transaction(&db, branch, Some("cust-1"), Mode::Live).await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);

    let facts = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_id,
        quote_id: quote1.id,
        customer_id: Some("cust-1".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_tx = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut comp_tx, EffectPhase::Redeem, &facts)
        .await
        .expect("live reservation honoured at completion despite campaign pause");
    comp_tx.commit().await.expect("commit");

    let redemptions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1 AND status = 'REDEEMED'",
    )
    .bind(txn_id)
    .fetch_one(&db.pool)
    .await
    .expect("count redemptions");
    assert_eq!(
        redemptions, 1,
        "honoured reservation must produce one redemption"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Row 6: Quote expired, budget since used up: completion refuses benefitLimitReached, rolls back
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn quote_expired_budget_used_up_completion_refuses_benefit_limit_reached_and_rolls_back() {
    let db = TestDb::start().await;
    let branch = "B-MK5-6";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    let c = seed_campaign(
        &db,
        user_id,
        branch,
        "50.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-exp", branch).await;
    let req = request(branch, Some("cust-exp"), vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    // Move reservation expiry into the past
    sqlx::query(
        "UPDATE benefit_reservations SET expires_at = now() - interval '10 minutes' WHERE quote_id = $1",
    )
    .bind(quote.id)
    .execute(&db.pool)
    .await
    .expect("expire reservation");

    // Budget since used up by another completion
    sqlx::query("UPDATE campaign_budgets SET consumed_thb = 50.00 WHERE campaign_version_id = $1")
        .bind(c.version_id)
        .execute(&db.pool)
        .await
        .expect("exhaust budget");

    let txn_id = seed_transaction(&db, branch, Some("cust-exp"), Mode::Live).await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);

    let facts = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_id,
        quote_id: quote.id,
        customer_id: Some("cust-exp".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_tx = db.pool.begin().await.expect("begin");
    let res = effects
        .run_completed_in(&mut comp_tx, EffectPhase::Redeem, &facts)
        .await;
    match res {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("benefitLimitReached"),
                "completion of expired quote with exhausted budget must refuse benefitLimitReached, got {msg}"
            );
        }
        Ok(()) => panic!("completion of expired quote with exhausted budget must refuse"),
    }
    comp_tx.rollback().await.expect("rollback");

    let redemptions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1")
            .bind(txn_id)
            .fetch_one(&db.pool)
            .await
            .expect("count redemptions");
    assert_eq!(
        redemptions, 0,
        "refused completion must roll back and leave no redemptions"
    );

    // Control (D-8): expired reservation WITH budget left is re-reserved and redeems
    let c_ctrl = seed_campaign(
        &db,
        user_id,
        branch,
        "500.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-ctrl", branch).await;
    let req_ctrl = request(branch, Some("cust-ctrl"), vec![usd_buy_line("100")]);
    let mut tx_ctrl = db.pool.begin().await.expect("begin");
    let quote_ctrl = service
        .quote_in(&mut tx_ctrl, req_ctrl, &teller(), Utc::now())
        .await
        .expect("quote_in ctrl");
    tx_ctrl.commit().await.expect("commit");

    sqlx::query(
        "UPDATE benefit_reservations SET expires_at = now() - interval '10 minutes' WHERE quote_id = $1",
    )
    .bind(quote_ctrl.id)
    .execute(&db.pool)
    .await
    .expect("expire reservation ctrl");

    let txn_ctrl_id = seed_transaction(&db, branch, Some("cust-ctrl"), Mode::Live).await;
    let facts_ctrl = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_ctrl_id,
        quote_id: quote_ctrl.id,
        customer_id: Some("cust-ctrl".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_ctrl_tx = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut comp_ctrl_tx, EffectPhase::Redeem, &facts_ctrl)
        .await
        .expect("expired reservation with budget left is re-reserved and redeems (D-8)");
    comp_ctrl_tx.commit().await.expect("commit");

    let ctrl_redemptions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1 AND status = 'REDEEMED'",
    )
    .bind(txn_ctrl_id)
    .fetch_one(&db.pool)
    .await
    .expect("count ctrl redemptions");
    assert_eq!(
        ctrl_redemptions, 1,
        "expired reservation with budget left must redeem once"
    );

    let ctrl_consumed: Decimal = sqlx::query_scalar(
        "SELECT consumed_thb FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(c_ctrl.version_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch ctrl budget");
    assert!(
        ctrl_consumed > Decimal::ZERO,
        "ctrl re-reserve must consume budget: {ctrl_consumed}"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Design §9.1: Completion rollback leaves budget as before
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn completion_rollback_leaves_budget_as_before() {
    let db = TestDb::start().await;
    let branch = "B-MK5-7";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    let c = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-rb", branch).await;
    let req = request(branch, Some("cust-rb"), vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    let txn_id = seed_transaction(&db, branch, Some("cust-rb"), Mode::Live).await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);

    let facts = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_id,
        quote_id: quote.id,
        customer_id: Some("cust-rb".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_tx = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut comp_tx, EffectPhase::Redeem, &facts)
        .await
        .expect("effect completes inside comp_tx");

    // Must show inside comp_tx that the effect moved the budget and wrote a redemption
    let (tx_consumed, tx_used): (Decimal, i32) = sqlx::query_as(
        "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(c.version_id)
    .fetch_one(&mut *comp_tx)
    .await
    .expect("fetch budget in comp_tx");
    assert!(
        tx_consumed > Decimal::ZERO,
        "inside comp_tx consumed_thb must have increased, got {tx_consumed}"
    );
    assert!(
        tx_used > 0,
        "inside comp_tx used_count must have increased, got {tx_used}"
    );

    let tx_redemptions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1 AND status = 'REDEEMED'",
    )
    .bind(txn_id)
    .fetch_one(&mut *comp_tx)
    .await
    .expect("count redemptions in comp_tx");
    assert_eq!(
        tx_redemptions, 1,
        "inside comp_tx one redemption must be written"
    );

    // Simulate transaction failure downstream (e.g. stock or ledger failure)
    comp_tx.rollback().await.expect("rollback");

    let (consumed, used): (Decimal, i32) = sqlx::query_as(
        "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(c.version_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch budget after rollback");

    assert_eq!(
        consumed,
        Decimal::ZERO,
        "rolled-back completion must leave consumed_thb as 0"
    );
    assert_eq!(used, 0, "rolled-back completion must leave used_count as 0");

    let redemptions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1")
            .bind(txn_id)
            .fetch_one(&db.pool)
            .await
            .expect("count redemptions after rollback");
    assert_eq!(
        redemptions, 0,
        "rolled-back completion must leave no redemptions"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Design §9.1: Completion through XS-0 ExchangeEffects redeems once
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn completion_through_exchange_effects_redeems_once() {
    let db = TestDb::start().await;
    let branch = "B-MK5-8";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-xs0", branch).await;
    let req = request(branch, Some("cust-xs0"), vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    let txn_id = seed_transaction(&db, branch, Some("cust-xs0"), Mode::Live).await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);

    let facts = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_id,
        quote_id: quote.id,
        customer_id: Some("cust-xs0".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_tx = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut comp_tx, EffectPhase::Redeem, &facts)
        .await
        .expect("effects run_completed_in");
    comp_tx.commit().await.expect("commit");

    let redemptions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1 AND status = 'REDEEMED'",
    )
    .bind(txn_id)
    .fetch_one(&db.pool)
    .await
    .expect("count redemptions");
    assert_eq!(
        redemptions, 1,
        "completion through effects must redeem once"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Row 5: Same completion retried with the same key redeems once (idempotent, §7)
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn same_completion_retried_with_same_key_redeems_once() {
    let db = TestDb::start().await;
    let branch = "B-MK5-9";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-idem", branch).await;
    let req = request(branch, Some("cust-idem"), vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    let txn_id = seed_transaction(&db, branch, Some("cust-idem"), Mode::Live).await;
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);

    let facts = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_id,
        quote_id: quote.id,
        customer_id: Some("cust-idem".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    // First completion
    let mut tx1 = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut tx1, EffectPhase::Redeem, &facts)
        .await
        .expect("first completion");
    tx1.commit().await.expect("commit");

    // Retried completion with same key / facts
    let mut tx2 = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut tx2, EffectPhase::Redeem, &facts)
        .await
        .expect("retried completion succeeds idempotently");
    tx2.commit().await.expect("commit");

    let redemptions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM benefit_redemptions WHERE transaction_id = $1")
            .bind(txn_id)
            .fetch_one(&db.pool)
            .await
            .expect("count redemptions");
    assert_eq!(
        redemptions, 1,
        "idempotent retry must not insert a duplicate redemption"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Design §9.1: Posted reversal marks redemption REVERSED, returns budget, re-issues active coupon
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn posted_reversal_marks_redemption_reversed_returns_budget_and_reissues_active_coupon() {
    let db = TestDb::start().await;
    let branch = "B-MK5-10";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    let c = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    db.customer("cust-rev", branch).await;
    let coupon_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, issued_at) \
         VALUES ($1, $2::uuid, $3, $4, 'REDEEMED', 'cust-rev', now() + interval '1 day', $5, now())",
    )
    .bind(coupon_id)
    .bind(ORG_1)
    .bind(c.rule_id)
    .bind(b"hash_rev".to_vec())
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert coupon");

    let txn_id = seed_transaction(&db, branch, Some("cust-rev"), Mode::Live).await;
    let service = QuoteService::new(db.pool.clone());
    let req = request(branch, Some("cust-rev"), vec![usd_buy_line("100")]);
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in");
    tx.commit().await.expect("commit");

    let existing_res: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM benefit_reservations WHERE quote_id = $1 AND rule_id = $2",
    )
    .bind(quote.id)
    .bind(c.rule_id)
    .fetch_optional(&db.pool)
    .await
    .expect("fetch reservation");

    let reservation_id = match existing_res {
        Some(res_id) => {
            sqlx::query(
                "UPDATE benefit_reservations SET status = 'CONSUMED', coupon_id = $1, amount_thb = 50.00 WHERE id = $2",
            )
            .bind(coupon_id)
            .bind(res_id)
            .execute(&db.pool)
            .await
            .expect("update existing reservation");
            res_id
        }
        None => {
            let res_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO benefit_reservations (id, organization_id, rule_id, campaign_version_id, quote_id, coupon_id, customer_id, amount_thb, status, expires_at, created_at) \
                 VALUES ($1, $2::uuid, $3, $4, $5, $6, 'cust-rev', 50.00, 'CONSUMED', now() + interval '1 day', now())",
            )
            .bind(res_id)
            .bind(ORG_1)
            .bind(c.rule_id)
            .bind(c.version_id)
            .bind(quote.id)
            .bind(coupon_id)
            .execute(&db.pool)
            .await
            .expect("insert reservation");
            res_id
        }
    };

    let redemption_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_redemptions (id, organization_id, rule_id, campaign_version_id, reservation_id, coupon_id, transaction_id, customer_id, benefit_class, amount_thb, status, redeemed_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, 'cust-rev', 'PRICE', 50.00, 'REDEEMED', now())",
    )
    .bind(redemption_id)
    .bind(ORG_1)
    .bind(c.rule_id)
    .bind(c.version_id)
    .bind(reservation_id)
    .bind(coupon_id)
    .bind(txn_id)
    .execute(&db.pool)
    .await
    .expect("insert redemption");

    sqlx::query(
        "UPDATE campaign_budgets SET consumed_thb = 50.00, used_count = 1 WHERE campaign_version_id = $1",
    )
    .bind(c.version_id)
    .execute(&db.pool)
    .await
    .expect("update budget");

    let reversal_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
             reason, amount_thb, business_date, requested_by, requested_at) \
         VALUES ($1, $2::uuid, $3, $4, 'REQUESTED', 'wrong currency', $5, current_date, 'teller', now())",
    )
    .bind(reversal_id)
    .bind(ORG_1)
    .bind(branch)
    .bind(txn_id)
    .bind(Decimal::from(3200))
    .execute(&db.pool)
    .await
    .expect("insert reversal");

    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);

    let rev = PostedReversal {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        reversal_id,
        transaction_id: txn_id,
        customer_id: Some("cust-rev".to_string()),
        mode: Mode::Live,
        posted_at: Utc::now(),
        amount_thb: Decimal::from(3200),
    };

    let mut rev_tx = db.pool.begin().await.expect("begin");
    effects
        .run_reversed_in(&mut rev_tx, EffectPhase::Redeem, &rev)
        .await
        .expect("on_reversed_in");
    rev_tx.commit().await.expect("commit");

    let (status, stored_reversal_id): (String, Option<Uuid>) =
        sqlx::query_as("SELECT status, reversal_id FROM benefit_redemptions WHERE id = $1")
            .bind(redemption_id)
            .fetch_one(&db.pool)
            .await
            .expect("redemption status and reversal_id");
    assert_eq!(status, "REVERSED", "redemption must be marked REVERSED");
    assert_eq!(
        stored_reversal_id,
        Some(reversal_id),
        "reversal_id must be set on the REVERSED redemption (matching §4.1 CHECK)"
    );

    let (consumed, used): (Decimal, i32) = sqlx::query_as(
        "SELECT consumed_thb, used_count FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(c.version_id)
    .fetch_one(&db.pool)
    .await
    .expect("budget");
    assert_eq!(
        consumed,
        Decimal::ZERO,
        "budget must return consumed amount on reversal"
    );
    assert_eq!(used, 0, "budget must return used_count on reversal");

    let c_status: String = sqlx::query_scalar("SELECT status FROM coupons WHERE id = $1")
        .bind(coupon_id)
        .fetch_one(&db.pool)
        .await
        .expect("coupon status");
    assert_eq!(
        c_status, "ISSUED",
        "single-use coupon of active version must return to ISSUED"
    );

    // Opposite case (control, M3 A): coupon of a version that is no longer ACTIVE stays used (REDEEMED)
    let c_inactive = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    sqlx::query("UPDATE campaign_versions SET status = 'PAUSED', ends_at = now() - interval '1 hour' WHERE id = $1")
        .bind(c_inactive.version_id)
        .execute(&db.pool)
        .await
        .expect("expire inactive version");

    let coupon_inact_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, issued_at) \
         VALUES ($1, $2::uuid, $3, $4, 'REDEEMED', 'cust-rev', now() + interval '1 day', $5, now())",
    )
    .bind(coupon_inact_id)
    .bind(ORG_1)
    .bind(c_inactive.rule_id)
    .bind(b"hash_inact".to_vec())
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("insert coupon inactive");

    let txn_inact_id = seed_transaction(&db, branch, Some("cust-rev"), Mode::Live).await;
    let req_inact = request(branch, Some("cust-rev"), vec![usd_buy_line("100")]);
    let mut tx_inact = db.pool.begin().await.expect("begin");
    let quote_inact = service
        .quote_in(&mut tx_inact, req_inact, &teller(), Utc::now())
        .await
        .expect("quote_in inact");
    tx_inact.commit().await.expect("commit");

    let existing_res_inact: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM benefit_reservations WHERE quote_id = $1 AND rule_id = $2",
    )
    .bind(quote_inact.id)
    .bind(c_inactive.rule_id)
    .fetch_optional(&db.pool)
    .await
    .expect("fetch reservation inact");

    let res_inact_id = match existing_res_inact {
        Some(res_id) => {
            sqlx::query(
                "UPDATE benefit_reservations SET status = 'CONSUMED', coupon_id = $1, amount_thb = 50.00 WHERE id = $2",
            )
            .bind(coupon_inact_id)
            .bind(res_id)
            .execute(&db.pool)
            .await
            .expect("update existing reservation inact");
            res_id
        }
        None => {
            let res_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO benefit_reservations (id, organization_id, rule_id, campaign_version_id, quote_id, coupon_id, customer_id, amount_thb, status, expires_at, created_at) \
                 VALUES ($1, $2::uuid, $3, $4, $5, $6, 'cust-rev', 50.00, 'CONSUMED', now() + interval '1 day', now())",
            )
            .bind(res_id)
            .bind(ORG_1)
            .bind(c_inactive.rule_id)
            .bind(c_inactive.version_id)
            .bind(quote_inact.id)
            .bind(coupon_inact_id)
            .execute(&db.pool)
            .await
            .expect("insert reservation inact");
            res_id
        }
    };

    let red_inact_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO benefit_redemptions (id, organization_id, rule_id, campaign_version_id, reservation_id, coupon_id, transaction_id, customer_id, benefit_class, amount_thb, status, redeemed_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, 'cust-rev', 'PRICE', 50.00, 'REDEEMED', now())",
    )
    .bind(red_inact_id)
    .bind(ORG_1)
    .bind(c_inactive.rule_id)
    .bind(c_inactive.version_id)
    .bind(res_inact_id)
    .bind(coupon_inact_id)
    .bind(txn_inact_id)
    .execute(&db.pool)
    .await
    .expect("insert redemption inact");

    sqlx::query(
        "UPDATE campaign_budgets SET consumed_thb = 50.00, used_count = 1 WHERE campaign_version_id = $1",
    )
    .bind(c_inactive.version_id)
    .execute(&db.pool)
    .await
    .expect("update budget inact");

    let rev_inact_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
             reason, amount_thb, business_date, requested_by, requested_at) \
         VALUES ($1, $2::uuid, $3, $4, 'REQUESTED', 'wrong currency', $5, current_date, 'teller', now())",
    )
    .bind(rev_inact_id)
    .bind(ORG_1)
    .bind(branch)
    .bind(txn_inact_id)
    .bind(Decimal::from(3200))
    .execute(&db.pool)
    .await
    .expect("insert reversal inact");

    let rev_inact = PostedReversal {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        reversal_id: rev_inact_id,
        transaction_id: txn_inact_id,
        customer_id: Some("cust-rev".to_string()),
        mode: Mode::Live,
        posted_at: Utc::now(),
        amount_thb: Decimal::from(3200),
    };

    let mut rev_inact_tx = db.pool.begin().await.expect("begin");
    effects
        .run_reversed_in(&mut rev_inact_tx, EffectPhase::Redeem, &rev_inact)
        .await
        .expect("on_reversed_in inact");
    rev_inact_tx.commit().await.expect("commit");

    let c_inact_status: String = sqlx::query_scalar("SELECT status FROM coupons WHERE id = $1")
        .bind(coupon_inact_id)
        .fetch_one(&db.pool)
        .await
        .expect("coupon status inact");
    assert_eq!(
        c_inact_status, "REDEEMED",
        "coupon of a version no longer ACTIVE must stay REDEEMED on reversal (M3 A)"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 Row 6: DEMO exchange evaluated without touching LIVE budget (§7, [proposed])
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn demo_exchange_evaluated_without_touching_live_budget() {
    let db = TestDb::start().await;
    let branch = "B-MK5-11";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    let c = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let effects = exchange_effects::effects(&state);
    let service = QuoteService::new(db.pool.clone());

    // 1. LIVE control: quote + completion DOES move consumed_thb
    db.customer("cust-live", branch).await;
    let req_live = request(branch, Some("cust-live"), vec![usd_buy_line("100")]);
    let mut tx_live = db.pool.begin().await.expect("begin");
    let quote_live = service
        .quote_in(&mut tx_live, req_live, &teller(), Utc::now())
        .await
        .expect("quote_in live");
    tx_live.commit().await.expect("commit");

    assert_ne!(
        quote_live.lines[0].final_rate, quote_live.lines[0].base_rate,
        "LIVE quote must receive benefits"
    );

    let txn_live_id = seed_transaction(&db, branch, Some("cust-live"), Mode::Live).await;
    let facts_live = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_live_id,
        quote_id: quote_live.id,
        customer_id: Some("cust-live".to_string()),
        mode: Mode::Live,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_live_tx = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut comp_live_tx, EffectPhase::Redeem, &facts_live)
        .await
        .expect("LIVE completion");
    comp_live_tx.commit().await.expect("commit");

    let consumed_after_live: Decimal = sqlx::query_scalar(
        "SELECT consumed_thb FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(c.version_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch budget after live");
    assert!(
        consumed_after_live > Decimal::ZERO,
        "LIVE completion must move consumed_thb, got {consumed_after_live}"
    );

    // 2. DEMO quote is evaluated with benefits
    db.customer("cust-demo", branch).await;
    let req = request(branch, Some("cust-demo"), vec![usd_buy_line("100")]);
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .expect("quote_in demo");
    tx.commit().await.expect("commit");

    assert_ne!(
        quote.lines[0].final_rate, quote.lines[0].base_rate,
        "DEMO quote must be evaluated with benefits"
    );

    let txn_id = seed_transaction(&db, branch, Some("cust-demo"), Mode::Demo).await;
    let facts = ExchangeCompletedFacts {
        organization_id: Uuid::parse_str(ORG_1).expect("org uuid"),
        branch_id: BranchId::new(branch),
        transaction_id: txn_id,
        quote_id: quote.id,
        customer_id: Some("cust-demo".to_string()),
        mode: Mode::Demo,
        completed_at: Utc::now(),
        business_date: Utc::now().date_naive(),
        reversible_until: Utc::now() + chrono::Duration::hours(24),
    };

    let mut comp_tx = db.pool.begin().await.expect("begin");
    effects
        .run_completed_in(&mut comp_tx, EffectPhase::Redeem, &facts)
        .await
        .expect("DEMO completion");
    comp_tx.commit().await.expect("commit");

    let consumed_after_demo: Decimal = sqlx::query_scalar(
        "SELECT consumed_thb FROM campaign_budgets WHERE campaign_version_id = $1",
    )
    .bind(c.version_id)
    .fetch_one(&db.pool)
    .await
    .expect("fetch budget after demo");
    assert_eq!(
        consumed_after_demo, consumed_after_live,
        "DEMO exchange must not consume LIVE campaign budget (stays {consumed_after_live})"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// #1403 §10 Step 2: The three adapters unit tests with real data and fallbacks
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn adapters_fallbacks() {
    let tier_source = MarketingTierSource::new();
    assert_eq!(
        tier_source.tier_of(Uuid::new_v4(), "cust-fallback"),
        None,
        "MarketingTierSource before LY-4 must answer None"
    );

    let db = TestDb::start().await;
    let branch = "B-MK5-ADP";
    db.branch(branch).await;

    // Seed customer with granted marketing consent
    let cust_granted = "cust-consent-granted";
    db.customer(cust_granted, branch).await;
    let user_id = seed_test_user(&db).await;
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref, recorded_by) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', true, now(), 'ref-granted', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(cust_granted)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed granted consent");

    // Seed customer with withdrawn marketing consent
    let cust_withdrawn = "cust-consent-withdrawn";
    db.customer(cust_withdrawn, branch).await;
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref, recorded_by) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', true, now() - interval '2 days', 'ref-prev', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(cust_withdrawn)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed initial consent");
    sqlx::query(
        "INSERT INTO consents (id, organization_id, customer_id, purpose, channel, policy_version, lang, granted, effective_at, evidence_ref, recorded_by) \
         VALUES ($1, $2::uuid, $3, 'MARKETING', 'SMS', 1, 'th', false, now() - interval '1 day', 'ref-withdrawn', $4)",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(cust_withdrawn)
    .bind(user_id)
    .execute(&db.pool)
    .await
    .expect("seed withdrawn consent");

    // Assert ConsentGate answers from real data
    let gate_granted = MarketingConsentGate::prefetch(&db.pool, cust_granted)
        .await
        .expect("prefetch granted");
    assert!(
        gate_granted.marketing_allowed(cust_granted),
        "customer with granted consent must have marketing_allowed == true"
    );

    let gate_withdrawn = MarketingConsentGate::prefetch(&db.pool, cust_withdrawn)
        .await
        .expect("prefetch withdrawn");
    assert!(
        !gate_withdrawn.marketing_allowed(cust_withdrawn),
        "customer with withdrawn consent must have marketing_allowed == false"
    );

    let gate_none = MarketingConsentGate::prefetch(&db.pool, "cust-no-consent")
        .await
        .expect("prefetch none");
    assert!(
        !gate_none.marketing_allowed("cust-no-consent"),
        "customer with no consent row must have marketing_allowed == false"
    );

    // Seed ACTIVE referral attribution captured before exchange
    let cust_ref = "cust-referred";
    db.customer(cust_ref, branch).await;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO referral_attributions (id, organization_id, branch_id, code, customer_id, source, captured_at, captured_by, status) \
         VALUES ($1, $2::uuid, $3, 'REFCODE123', $4, 'DESK', $5, 'teller', 'ACTIVE')",
    )
    .bind(Uuid::new_v4())
    .bind(ORG_1)
    .bind(branch)
    .bind(cust_ref)
    .bind(now - chrono::Duration::hours(1))
    .execute(&db.pool)
    .await
    .expect("seed referral attribution");

    let ref_source = MarketingReferralSource::prefetch(&db.pool, cust_ref, now)
        .await
        .expect("prefetch referral");
    assert!(
        ref_source.referred(cust_ref, now),
        "customer with ACTIVE referral attribution before exchange must be referred"
    );

    let ref_after =
        MarketingReferralSource::prefetch(&db.pool, cust_ref, now - chrono::Duration::hours(2))
            .await
            .expect("prefetch referral after");
    assert!(
        !ref_after.referred(cust_ref, now - chrono::Duration::hours(2)),
        "referral captured after exchange time must not be considered referred"
    );

    let ref_none = MarketingReferralSource::prefetch(&db.pool, "cust-unreferred", now)
        .await
        .expect("prefetch referral none");
    assert!(
        !ref_none.referred("cust-unreferred", now),
        "customer without referral attribution must have referred == false"
    );

    db.finish().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 + §9.1 Row 13: Parity — MK-4 campaign simulator line equals quote_in line
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn simulator_line_equals_quote_in_line() {
    let db = TestDb::start().await;
    let branch = "B-MK5-PARITY";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user_id = seed_test_user(&db).await;

    let campaign = seed_campaign(
        &db,
        user_id,
        branch,
        "5000.00",
        "RATE_IMPROVEMENT",
        serde_json::json!({ "currencies": ["USD"] }),
        serde_json::json!({ "type": "FIXED_RATE", "delta": "0.50" }),
    )
    .await;

    let simulator = CampaignSimulator::new(db.pool.clone());
    let scenario = SimulationScenario {
        branch_id: branch.to_string(),
        currency: "USD".to_string(),
        amount: Decimal::from_str_exact("100.00").expect("decimal"),
    };
    let now = Utc::now();
    let sim_result = simulator
        .simulate(
            Uuid::parse_str(ORG_1).unwrap(),
            campaign.version_id,
            vec![scenario],
            user_id,
            now,
        )
        .await
        .expect("simulator succeeds");

    let sim_eval = sim_result
        .evaluation
        .expect("simulation evaluation must be present");
    assert!(
        !sim_eval.snapshot.applied.is_empty(),
        "simulator must have applied rules"
    );
    let sim_line = &sim_eval.snapshot.applied[0];

    let req = request(branch, None, vec![usd_buy_line("100")]);
    let service = QuoteService::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let quote = service
        .quote_in(&mut tx, req, &teller(), now)
        .await
        .expect("quote_in succeeds");
    tx.commit().await.expect("commit");

    let quote_applied = quote
        .benefit_snapshot
        .get("applied")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty());
    assert!(
        quote_applied.is_some(),
        "quote_in benefit_snapshot must contain applied benefits matching simulator line"
    );

    let quote_line = &quote_applied.unwrap()[0];
    let quote_rate_after = quote_line
        .get("rateAfter")
        .or_else(|| quote_line.get("rate_after"))
        .and_then(|v| v.as_str())
        .expect("quote rateAfter");
    let quote_amount_thb = quote_line
        .get("amountThb")
        .or_else(|| quote_line.get("amount_thb"))
        .and_then(|v| v.as_str())
        .expect("quote amountThb");

    assert_eq!(
        quote_rate_after, sim_line.rate_after,
        "quote_in rate_after must equal simulator line rate_after as string"
    );
    assert_eq!(
        quote_amount_thb, sim_line.amount_thb,
        "quote_in benefit amount must equal simulator line benefit amount as string"
    );

    db.finish().await;
}
