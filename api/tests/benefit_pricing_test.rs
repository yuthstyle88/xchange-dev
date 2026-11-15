// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE under the xchange BE lane controller claude/bc8fd29a). Do not edit, delete or weaken.

//! Up-front tests for BE-1, `BenefitPricing::evaluate_in` (design `marketing-campaigns.md` §6.4, decision E-3,
//! class diagram §6.4.4, behaviour table §6.4.5). Owner decision OD-1 B (2026-10-27): the estimate keeps only
//! rules whose `channels` also admit DESK, through `BenefitPricingInput.also_admitted_on = Some(Desk)`.
//!
//! Rows of §6.4.5 covered here:
//! - 16: `also_admitted_on = Some(Desk)` and an APP-only rule ⇒ the rule is not in `evaluation.snapshot.applied`;
//!   a lower-priority rule admitting DESK in the same stack group applies instead. Control: with
//!   `also_admitted_on = None` the higher-priority APP-only rule applies.
//!
//! - 15: `price_in` before and after the extraction, every channel, with and without a coupon ⇒ identical
//!   `quote_lines`, `benefit_snapshot` and reservations. The expected values were recorded from `price_in` on
//!   `feat/booking-estimate` at `2601c6fe` (main `e28b581e` merged in), before BE-1, on the Mac Studio
//!   (2026-10-28). m1–m6 (`booking_quote_benefit_test.rs`), `v3_quotes_route_test.rs` and
//!   `app_planner_quote_parity_test.rs` must also stay green unedited.
//!
//! Each evaluation runs in a transaction the test rolls back. Needs `TEST_DATABASE_URL` naming a database whose
//! name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;
use xchange::application::workflows::exchange::benefit_pricing::{
    BenefitPricing, BenefitPricingInput, PricingLine,
};
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;

/// An APPROVED running campaign on `branch` in stack group `default` at `priority`, with one PRICE
/// `RATE_IMPROVEMENT` rule of `eligibility` and `delta`; its rule id.
async fn campaign(
    db: &TestDb,
    branch: &str,
    user: Uuid,
    now: DateTime<Utc>,
    priority: i32,
    eligibility: Value,
    delta: &str,
) -> Uuid {
    let (campaign, version, rule) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign)
    .bind(ORG_1)
    .bind(unique("be1-campaign"))
    .bind(user)
    .execute(&db.pool)
    .await
    .expect("insert campaign");
    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', $4, $5, 5000.00, 'default', $6, 'hash-be1', now(), $7, now())",
    )
    .bind(version)
    .bind(ORG_1)
    .bind(campaign)
    .bind(now - Duration::days(1))
    .bind(now + Duration::days(30))
    .bind(priority)
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
        .bind(branch)
        .execute(&db.pool)
        .await
        .expect("insert campaign branch");
    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
    )
    .bind(version)
    .execute(&db.pool)
    .await
    .expect("insert campaign budget");
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, \
             caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, false, 1)",
    )
    .bind(rule)
    .bind(version)
    .bind(eligibility)
    .bind(json!({ "delta": delta }))
    .execute(&db.pool)
    .await
    .expect("insert benefit rule");
    rule
}

/// The rule ids `evaluate_in` applied, judged on APP for an anonymous caller buying-side 100 USD, fee 0.
async fn applied_on_app(
    db: &TestDb,
    branch: &str,
    now: DateTime<Utc>,
    also_admitted_on: Option<QuoteChannel>,
) -> Vec<Uuid> {
    let branch_id = BranchId::new(branch);
    let currency = CurrencyCode::parse("USD").expect("USD");
    let quantity = ForeignAmount::from_decimal(Decimal::from(100));
    let mut tx = db.pool.begin().await.expect("begin");
    let published = PgPublishedRates::new(db.pool.clone())
        .line_for(
            &mut tx,
            &branch_id,
            LineSelector {
                code: currency,
                side: Side::Buy,
                group: GroupCode::all(),
                quantity: Some(quantity),
            },
        )
        .await
        .expect("read the published line")
        .expect("USD BUY is published");
    let priced = BenefitPricing::new(Arc::new(PgCampaignRepository::new()))
        .evaluate_in(
            &mut tx,
            BenefitPricingInput {
                organization_id: Uuid::from_str(ORG_1).expect("ORG_1 is a uuid"),
                branch: &branch_id,
                eligibility_channel: QuoteChannel::App,
                also_admitted_on,
                customer_id: None,
                coupon: None,
                coupon_code: None,
                line: PricingLine {
                    currency,
                    shop_side: Side::Buy,
                    quantity,
                    fee_thb: Money::ZERO,
                    published,
                },
                now,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("evaluate_in with also_admitted_on {also_admitted_on:?}: {e}"));
    tx.rollback().await.expect("rollback");
    priced
        .evaluation
        .snapshot
        .applied
        .iter()
        .map(|a| a.rule_id)
        .collect()
}

#[tokio::test]
async fn be1_row16_also_admitted_on_desk_drops_an_app_only_rule() {
    let db = TestDb::start().await;
    let branch = "B-BE1-R16";
    db.branch(branch).await;
    xc_testdb::seed_branch_currencies(&db.pool, branch, &["USD"])
        .await
        .expect("seed branch currencies");
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{user}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
        unique("be1-user")
    ))
    .await;
    let now = whole_second_now();
    // Same stack group: the APP-only rule wins on priority unless it is filtered out first.
    let app_only = campaign(
        &db,
        branch,
        user,
        now,
        2,
        json!({ "currencies": ["USD"], "channels": ["APP"] }),
        "0.50",
    )
    .await;
    let app_and_desk = campaign(
        &db,
        branch,
        user,
        now,
        1,
        json!({ "currencies": ["USD"], "channels": ["APP", "DESK"] }),
        "0.20",
    )
    .await;

    let filtered = applied_on_app(&db, branch, now, Some(QuoteChannel::Desk)).await;
    assert!(
        !filtered.contains(&app_only),
        "also_admitted_on = Some(Desk) drops the APP-only rule {app_only}; applied {filtered:?}"
    );
    assert_eq!(
        filtered,
        vec![app_and_desk],
        "the lower-priority rule admitting DESK applies instead; applied {filtered:?}"
    );

    let unfiltered = applied_on_app(&db, branch, now, None).await;
    assert_eq!(
        unfiltered,
        vec![app_only],
        "control: with also_admitted_on = None the higher-priority APP-only rule applies; applied {unfiltered:?}"
    );
    db.finish().await;
}

// ── row 15: price_in is unchanged by the extraction ─────────────────────────────────────────────────────────────

const ALL_CHANNELS: [QuoteChannel; 4] = [
    QuoteChannel::Desk,
    QuoteChannel::Kiosk,
    QuoteChannel::Booking,
    QuoteChannel::App,
];

/// One rule of the row-15 world, each in a stack group of its own and at its own priority, so the one PRICE
/// benefit a line gets (`Stacking::resolve`) never depends on a random id: `coupon` (9) wins when its code is
/// given; `spent` (8) would win everywhere else if the exhausted-budget filter let it through; the channel rules
/// (5) win on the channel they admit; `open` (1) applies only where nothing else does.
struct WorldRule {
    name: &'static str,
    group: &'static str,
    priority: i32,
    eligibility: Value,
    delta: &'static str,
    requires_coupon: bool,
    /// `consumed_thb` of a 5000.00 budget.
    consumed: &'static str,
}

fn world_rules() -> Vec<WorldRule> {
    let rule = |name, group, priority, eligibility, delta, requires_coupon, consumed| WorldRule {
        name,
        group,
        priority,
        eligibility,
        delta,
        requires_coupon,
        consumed,
    };
    vec![
        rule(
            "desk",
            "g-desk",
            5,
            json!({ "currencies": ["USD"], "channels": ["DESK"] }),
            "0.10",
            false,
            "0",
        ),
        rule(
            "kiosk",
            "g-kiosk",
            5,
            json!({ "currencies": ["USD"], "channels": ["KIOSK"] }),
            "0.05",
            false,
            "0",
        ),
        rule(
            "app",
            "g-app",
            5,
            json!({ "currencies": ["USD"], "channels": ["APP"] }),
            "0.20",
            false,
            "0",
        ),
        rule(
            "open",
            "g-open",
            1,
            json!({ "currencies": ["USD"] }),
            "0.01",
            false,
            "0",
        ),
        rule(
            "coupon",
            "g-coupon",
            9,
            json!({ "currencies": ["USD"], "requiresCoupon": true }),
            "0.30",
            true,
            "0",
        ),
        rule(
            "spent",
            "g-spent",
            8,
            json!({ "currencies": ["USD"] }),
            "0.40",
            false,
            "5000.00",
        ),
    ]
}

fn row15_teller() -> SaveActor {
    SaveActor {
        id: "teller-be1-r15".to_owned(),
        username: "user-teller-be1-r15".to_owned(),
        employee_id: Some("teller-be1-r15".to_owned()),
        standing: xc_auth::domain::access::StandingKind::Staff(
            xc_auth::domain::access::Role::Teller,
        ),
    }
}

/// The world on a branch of its own: USD 1 at 32.00 / 33.00 and the [`world_rules`]; every id it made, by name.
async fn row15_world(
    db: &TestDb,
    branch: &str,
    now: DateTime<Utc>,
) -> (Vec<(String, &'static str)>, String) {
    db.branch(branch).await;
    xc_testdb::seed_branch_currencies(&db.pool, branch, &["USD"])
        .await
        .expect("seed branch currencies");
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{user}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
        unique("be1-r15-user")
    ))
    .await;
    let mut names = Vec::new();
    let mut coupon_code = String::new();
    for spec in world_rules() {
        let (campaign, version, rule) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        db.exec(&format!(
            "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
             VALUES ('{campaign}', '{ORG_1}'::uuid, '{}', '{user}', now())",
            unique("be1-r15-campaign")
        ))
        .await;
        sqlx::query(
            "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
                 budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
             VALUES ($1, $2::uuid, $3, 1, 'APPROVED', $4, $5, 5000.00, $6, $7, 'hash-be1-r15', now(), $8, now())",
        )
        .bind(version)
        .bind(ORG_1)
        .bind(campaign)
        .bind(now - Duration::days(1))
        .bind(now + Duration::days(30))
        .bind(spec.group)
        .bind(spec.priority)
        .bind(user)
        .execute(&db.pool)
        .await
        .expect("insert campaign version");
        db.exec(&format!(
            "UPDATE campaigns SET current_version_id = '{version}' WHERE id = '{campaign}'"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ('{version}', '{branch}')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) \
             VALUES ('{version}', {}, 0)",
            spec.consumed
        ))
        .await;
        sqlx::query(
            "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, \
                 calculation, caps, requires_coupon, rule_no) \
             VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, $5, 1)",
        )
        .bind(rule)
        .bind(version)
        .bind(&spec.eligibility)
        .bind(json!({ "type": "FIXED_RATE", "delta": spec.delta }))
        .bind(spec.requires_coupon)
        .execute(&db.pool)
        .await
        .expect("insert benefit rule");
        names.push((campaign.to_string(), spec.name));
        names.push((version.to_string(), spec.name));
        names.push((rule.to_string(), spec.name));
        if spec.requires_coupon {
            common::install_test_pii_keys();
            let keys = xc_customer::get_pii_keys().expect("test PII keys installed");
            let hasher = xc_marketing::domain::coupon::code_hasher::CouponCodeHasher::new(&keys);
            coupon_code = unique("BE1-R15-CPN").to_uppercase();
            let coupon = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, \
                     issued_by, issued_at) VALUES ($1, $2::uuid, $3, $4, 'ISSUED', NULL, $5, $6, now())",
            )
            .bind(coupon)
            .bind(ORG_1)
            .bind(rule)
            .bind(&hasher.hash(&coupon_code)[..])
            .bind(now + Duration::days(1))
            .bind(user)
            .execute(&db.pool)
            .await
            .expect("insert coupon");
            names.push((coupon.to_string(), "coupon-row"));
        }
    }
    (names, coupon_code)
}

/// What `price_in` wrote for one quote, with every id this world made replaced by its rule's name and a
/// reservation id replaced by `"<reservation>"`: the lines (without ids and times), the snapshot, and the
/// reservations by rule.
async fn row15_outcome(db: &TestDb, quote: Uuid, names: &[(String, &'static str)]) -> Value {
    let (lines,): (Value,) = sqlx::query_as(
        "SELECT coalesce(jsonb_agg(to_jsonb(l) - 'id' - 'organization_id' - 'quote_id' - 'published_rate_line_id' \
             - 'created_at' - 'updated_at' ORDER BY l.line_no), '[]'::jsonb) \
         FROM quote_lines l WHERE l.quote_id = $1",
    )
    .bind(quote)
    .fetch_one(&db.pool)
    .await
    .expect("read quote_lines");
    let (snapshot, reservations): (Value, Value) = sqlx::query_as(
        "SELECT q.benefit_snapshot, \
             (SELECT coalesce(jsonb_agg(jsonb_build_object('rule', r.rule_id::text, 'coupon', r.coupon_id::text, \
                  'amountThb', r.amount_thb::text, 'status', r.status) ORDER BY r.rule_id::text), '[]'::jsonb) \
              FROM benefit_reservations r WHERE r.quote_id = q.id) \
         FROM quotes q WHERE q.id = $1",
    )
    .bind(quote)
    .fetch_one(&db.pool)
    .await
    .expect("read the quote's snapshot and reservations");
    let reservation_ids: Vec<String> =
        sqlx::query_scalar("SELECT id::text FROM benefit_reservations WHERE quote_id = $1")
            .bind(quote)
            .fetch_all(&db.pool)
            .await
            .expect("read reservation ids");
    let mut text =
        json!({ "lines": lines, "snapshot": snapshot, "reservations": reservations }).to_string();
    for (id, name) in names {
        text = text.replace(id, name);
    }
    for id in reservation_ids {
        text = text.replace(&id, "<reservation>");
    }
    serde_json::from_str(&text).expect("the outcome is JSON")
}

/// One quote of `quantity` USD on `shop_side` and `channel`, committed: its outcome, or the refusal's text.
async fn row15_quote(
    db: &TestDb,
    branch: &str,
    channel: QuoteChannel,
    (shop_side, quantity): (Side, i64),
    coupon_code: Option<&str>,
    now: DateTime<Utc>,
    names: &[(String, &'static str)],
) -> Value {
    let req = QuoteRequest {
        branch_id: BranchId::new(branch),
        customer_id: None,
        channel,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse("USD").expect("USD"),
            shop_side,
            quantity: ForeignAmount::from_decimal(Decimal::from(quantity)),
            fee_thb: None,
            denomination_group: GroupCode::all(),
        }],
        payment_method: None,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    let quoted = QuoteService::new(db.pool.clone())
        .quote_with_coupon_in(&mut tx, req, &row15_teller(), now, coupon_code)
        .await;
    match quoted {
        Ok(q) => {
            tx.commit().await.expect("commit");
            row15_outcome(db, q.id, names).await
        }
        Err(e) => {
            tx.rollback().await.expect("rollback");
            json!({ "refused": e.to_string() })
        }
    }
}

#[tokio::test]
async fn be1_row15_price_in_writes_what_it_wrote_before_the_extraction() {
    let db = TestDb::start().await;
    let now = whole_second_now();
    let mut got = serde_json::Map::new();
    for (i, channel) in ALL_CHANNELS.into_iter().enumerate() {
        for with_coupon in [false, true] {
            // a world of its own per case: a DESK quote reserves budget and locks the coupon
            let branch = format!("B-BE1-R15-{i}{}", if with_coupon { "C" } else { "N" });
            let (names, code) = row15_world(&db, &branch, now).await;
            let coupon = with_coupon.then_some(code.as_str());
            // BUY first, then SELL in the same world: the SELL quote sees what the BUY quote reserved
            let mut outcome = serde_json::Map::new();
            for (label, side) in [("buy 100", (Side::Buy, 100)), ("sell 50", (Side::Sell, 50))] {
                let one = row15_quote(&db, &branch, channel, side, coupon, now, &names).await;
                outcome.insert(label.to_owned(), one);
            }
            let outcome = Value::Object(outcome);
            let case = format!(
                "{channel:?} {}",
                if with_coupon { "coupon" } else { "no coupon" }
            );
            got.insert(case, outcome);
        }
    }
    let got = Value::Object(got);
    if std::env::var("BE1_ROW15_RECORD").is_ok() {
        println!(
            "BE1_ROW15 {}",
            serde_json::to_string_pretty(&got).expect("json")
        );
    }
    let expected = row15_expected();
    for (case, want) in expected.as_object().expect("cases") {
        assert_eq!(&got[case], want, "row 15, {case}: price_in changed");
    }
    assert_eq!(
        got.as_object().map(|m| m.len()),
        Some(8),
        "every case is pinned"
    );
    db.finish().await;
}

/// Recorded before BE-1 (see the module comment). GA-4 (#6928): each stored line gains `cash_step_thb`, `null` for
/// these quotes, which record no cash step.
fn row15_expected() -> Value {
    json!({
        "App coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.3,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3230.0,
                        "shop_side": "BUY",
                        "thb_amount": 3230.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "30.00",
                        "coupon": "coupon-row",
                        "rule": "coupon",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "30.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "couponId": "coupon-row",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.3000",
                            "rateBefore": "32.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "coupon",
                            "versionId": "coupon"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 33.0,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1650.0,
                        "shop_side": "SELL",
                        "thb_amount": 1650.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        },
                        {
                            "reason": "couponInvalid",
                            "ruleId": "coupon"
                        }
                    ]
                }
            }
        },
        "App no coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.2,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3220.0,
                        "shop_side": "BUY",
                        "thb_amount": 3220.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "20.00",
                        "coupon": null,
                        "rule": "app",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "20.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.2000",
                            "rateBefore": "32.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "app",
                            "versionId": "app"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.8,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1640.0,
                        "shop_side": "SELL",
                        "thb_amount": 1640.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "10.00",
                        "coupon": null,
                        "rule": "app",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "10.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.8000",
                            "rateBefore": "33.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "app",
                            "versionId": "app"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            }
        },
        "Booking coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.3,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3230.0,
                        "shop_side": "BUY",
                        "thb_amount": 3230.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "30.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.3000",
                            "rateBefore": "32.0000",
                            "ruleId": "coupon",
                            "versionId": "coupon"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.7,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1635.0,
                        "shop_side": "SELL",
                        "thb_amount": 1635.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "15.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.7000",
                            "rateBefore": "33.0000",
                            "ruleId": "coupon",
                            "versionId": "coupon"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            }
        },
        "Booking no coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.1,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3210.0,
                        "shop_side": "BUY",
                        "thb_amount": 3210.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "10.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.1000",
                            "rateBefore": "32.0000",
                            "ruleId": "desk",
                            "versionId": "desk"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.9,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1645.0,
                        "shop_side": "SELL",
                        "thb_amount": 1645.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "5.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.9000",
                            "rateBefore": "33.0000",
                            "ruleId": "desk",
                            "versionId": "desk"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            }
        },
        "Desk coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.3,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3230.0,
                        "shop_side": "BUY",
                        "thb_amount": 3230.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "30.00",
                        "coupon": "coupon-row",
                        "rule": "coupon",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "30.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "couponId": "coupon-row",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.3000",
                            "rateBefore": "32.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "coupon",
                            "versionId": "coupon"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 33.0,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1650.0,
                        "shop_side": "SELL",
                        "thb_amount": 1650.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        },
                        {
                            "reason": "couponInvalid",
                            "ruleId": "coupon"
                        }
                    ]
                }
            }
        },
        "Desk no coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.1,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3210.0,
                        "shop_side": "BUY",
                        "thb_amount": 3210.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "10.00",
                        "coupon": null,
                        "rule": "desk",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "10.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.1000",
                            "rateBefore": "32.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "desk",
                            "versionId": "desk"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.9,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1645.0,
                        "shop_side": "SELL",
                        "thb_amount": 1645.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "5.00",
                        "coupon": null,
                        "rule": "desk",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "5.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.9000",
                            "rateBefore": "33.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "desk",
                            "versionId": "desk"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "kiosk"
                        }
                    ]
                }
            }
        },
        "Kiosk coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.3,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3230.0,
                        "shop_side": "BUY",
                        "thb_amount": 3230.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "30.00",
                        "coupon": "coupon-row",
                        "rule": "coupon",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "30.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "couponId": "coupon-row",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.3000",
                            "rateBefore": "32.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "coupon",
                            "versionId": "coupon"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 33.0,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1650.0,
                        "shop_side": "SELL",
                        "thb_amount": 1650.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [],
                "snapshot": {
                    "applied": [],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        },
                        {
                            "reason": "couponInvalid",
                            "ruleId": "coupon"
                        }
                    ]
                }
            }
        },
        "Kiosk no coupon": {
            "buy 100": {
                "lines": [
                    {
                        "base_rate": 32.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.05,
                        "line_no": 1,
                        "quantity": 100.0,
                        "settle_thb": 3205.0,
                        "shop_side": "BUY",
                        "thb_amount": 3205.0,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "5.00",
                        "coupon": null,
                        "rule": "kiosk",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "5.00",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.0500",
                            "rateBefore": "32.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "kiosk",
                            "versionId": "kiosk"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        }
                    ]
                }
            },
            "sell 50": {
                "lines": [
                    {
                        "base_rate": 33.0,
                        "cash_step_thb": null,
                        "currency": "USD",
                        "denomination_group": "ALL",
                        "fee_thb": 0.0,
                        "final_rate": 32.95,
                        "line_no": 1,
                        "quantity": 50.0,
                        "settle_thb": 1647.5,
                        "shop_side": "SELL",
                        "thb_amount": 1647.5,
                        "unit_size": 1.0
                    }
                ],
                "reservations": [
                    {
                        "amountThb": "2.50",
                        "coupon": null,
                        "rule": "kiosk",
                        "status": "RESERVED"
                    }
                ],
                "snapshot": {
                    "applied": [
                        {
                            "amountThb": "2.50",
                            "benefitClass": "PRICE",
                            "benefitType": "RATE_IMPROVEMENT",
                            "feeAfter": "0.00",
                            "feeBefore": "0.00",
                            "floorCapped": false,
                            "rateAfter": "32.9500",
                            "rateBefore": "33.0000",
                            "reservationId": "<reservation>",
                            "ruleId": "kiosk",
                            "versionId": "kiosk"
                        }
                    ],
                    "engineVersion": 1,
                    "segmentSnapshotIds": [],
                    "skipped": [
                        {
                            "reason": "channelMismatch",
                            "ruleId": "app"
                        },
                        {
                            "reason": "channelMismatch",
                            "ruleId": "desk"
                        }
                    ]
                }
            }
        }
    })
}
