//! Ticket #1947 — `PublishService::publish` with multiple draft lines sharing a policy version
//! and distinct reference lines (or a missing reference line).

mod rate_engine_common;

use chrono::{Duration, Utc};
use rate_engine_common::*;
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use xc_auth::domain::access;
use xc_kernel::money::Side;
use xc_rate::application::draft::rate_draft_service::SubmitOutcome;
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::{PublicationRef, PublishedRates};

async fn settle<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, RateEngineError>,
) -> Result<T, &'static str> {
    match result {
        Ok(value) => {
            tx.commit().await.expect("commit");
            Ok(value)
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

async fn create_and_approve(
    db: &TestDb,
    e: &Engine,
    who: &access::Actor,
    lines: Vec<DraftLine>,
) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e.drafts.create(&mut tx, &main_branch(), who, lines).await;
    let draft = settle(tx, got).await.expect("draft created");

    let mut tx = db.begin().await;
    let got: Result<SubmitOutcome, RateEngineError> = e
        .drafts
        .submit(&mut tx, &main_branch(), who, &draft.id, &key())
        .await;
    let outcome = settle(tx, got).await.expect("draft submitted");
    assert_eq!(outcome.status, DraftStatus::Approved);
    draft
}

async fn publish_draft(
    db: &TestDb,
    e: &Engine,
    draft: &RateDraft,
    expected_version: Option<i32>,
) -> PublicationRef {
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(
            &mut tx,
            &main_branch(),
            &owner_user(),
            &draft.id,
            expected_version,
        )
        .await;
    settle(tx, got).await.expect("draft published")
}

fn guard(min: &str, max: &str) -> Value {
    json!({ "min_pct": min, "max_pct": max })
}

#[tokio::test]
async fn three_lines_sharing_one_policy_and_three_distinct_references_publish_each_at_its_own_reference()
 {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(30);
    db.seed_batch(
        fetched_at,
        &[
            ("USD", "1", Some("32.00"), Some("33.00")),
            ("EUR", "1", Some("36.00"), Some("37.00")),
            ("GBP", "1", Some("42.00"), Some("43.00")),
        ],
    )
    .await;

    // Shared policy: -1% on buy, +1% on sell
    let pol_id = db
        .seed_policy(
            MAIN,
            "-1.0000",
            "1.0000",
            json!({
                "USD": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
                "EUR": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
                "GBP": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            }),
            Some(24),
            None,
            None,
        )
        .await;

    let draft = create_and_approve(
        &db,
        &e,
        &owner_user(),
        vec![auto_line("USD"), auto_line("EUR"), auto_line("GBP")],
    )
    .await;

    let pub_ref = publish_draft(&db, &e, &draft, None).await;
    assert_eq!(pub_ref.version, 1);

    let mut tx = db.begin().await;
    let usd_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), Side::Buy)
        .await
        .expect("usd buy query")
        .expect("usd buy published");
    let usd_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), Side::Sell)
        .await
        .expect("usd sell query")
        .expect("usd sell published");

    let eur_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("EUR"), Side::Buy)
        .await
        .expect("eur buy query")
        .expect("eur buy published");
    let eur_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("EUR"), Side::Sell)
        .await
        .expect("eur sell query")
        .expect("eur sell published");

    let gbp_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("GBP"), Side::Buy)
        .await
        .expect("gbp buy query")
        .expect("gbp buy published");
    let gbp_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("GBP"), Side::Sell)
        .await
        .expect("gbp sell query")
        .expect("gbp sell published");
    tx.commit().await.expect("commit read");

    // USD: 32.00 * 0.99 = 31.68, 33.00 * 1.01 = 33.33
    assert_eq!(usd_buy.rate, rate("31.68"));
    assert_eq!(usd_sell.rate, rate("33.33"));

    // EUR: 36.00 * 0.99 = 35.64, 37.00 * 1.01 = 37.37
    assert_eq!(eur_buy.rate, rate("35.64"));
    assert_eq!(eur_sell.rate, rate("37.37"));

    // GBP: 42.00 * 0.99 = 41.58, 43.00 * 1.01 = 43.43
    assert_eq!(gbp_buy.rate, rate("41.58"));
    assert_eq!(gbp_sell.rate, rate("43.43"));

    // Verify source_snapshot on all 3 lines carries the shared policy_version_id and distinct reference_line_ids
    let rows = sqlx::query(
        "SELECT currency_code, source_snapshot FROM published_rate_lines \
         WHERE publication_id = $1 ORDER BY currency_code",
    )
    .bind(pub_ref.id.as_uuid())
    .fetch_all(&db.pool)
    .await
    .expect("read published_rate_lines");
    assert_eq!(rows.len(), 3);

    let mut ref_ids = std::collections::HashSet::new();
    for r in rows {
        let snap: Value = r.get("source_snapshot");
        assert_eq!(snap["policy_version_id"].as_str(), Some(pol_id.as_str()));
        let ref_id = snap["reference_line_id"]
            .as_str()
            .expect("reference_line_id present")
            .to_owned();
        ref_ids.insert(ref_id);
    }
    assert_eq!(
        ref_ids.len(),
        3,
        "each line must record its own distinct reference_line_id"
    );

    db.finish().await;
}

#[tokio::test]
async fn three_lines_with_one_line_missing_reference_behaves_as_today() {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(30);
    // Seed references only for USD and EUR; GBP has no reference row.
    db.seed_batch(
        fetched_at,
        &[
            ("USD", "1", Some("32.00"), Some("33.00")),
            ("EUR", "1", Some("36.00"), Some("37.00")),
        ],
    )
    .await;

    let pol_id = db
        .seed_policy(
            MAIN,
            "-1.0000",
            "1.0000",
            json!({
                "USD": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
                "EUR": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
                "GBP": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
            }),
            Some(24),
            None,
            None,
        )
        .await;

    let draft = create_and_approve(
        &db,
        &e,
        &owner_user(),
        vec![auto_line("USD"), auto_line("EUR"), auto_line("GBP")],
    )
    .await;

    let pub_ref = publish_draft(&db, &e, &draft, None).await;
    assert_eq!(pub_ref.version, 1);

    let mut tx = db.begin().await;
    let usd_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), Side::Buy)
        .await
        .expect("usd buy query")
        .expect("usd buy published");
    let eur_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("EUR"), Side::Buy)
        .await
        .expect("eur buy query")
        .expect("eur buy published");
    let gbp_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("GBP"), Side::Buy)
        .await
        .expect("gbp buy query");
    let gbp_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("GBP"), Side::Sell)
        .await
        .expect("gbp sell query");
    tx.commit().await.expect("commit read");

    assert_eq!(usd_buy.rate, rate("31.68"));
    assert_eq!(eur_buy.rate, rate("35.64"));
    assert!(
        gbp_buy.is_none(),
        "AUTO line with missing reference must have no published buy rate"
    );
    assert!(
        gbp_sell.is_none(),
        "AUTO line with missing reference must have no published sell rate"
    );

    let gbp_snap: Value = sqlx::query_scalar(
        "SELECT source_snapshot FROM published_rate_lines \
         WHERE publication_id = $1 AND currency_code = 'GBP'",
    )
    .bind(pub_ref.id.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("gbp row in published_rate_lines");

    assert_eq!(
        gbp_snap["policy_version_id"].as_str(),
        Some(pol_id.as_str())
    );
    assert!(gbp_snap["reference_line_id"].is_null());
    assert!(gbp_snap["fetched_at"].is_null());
    assert!(gbp_snap["source_date"].is_null());

    db.finish().await;
}

#[tokio::test]
async fn two_policy_versions_in_one_draft_publishes_each_line_under_its_own_policy() {
    let db = TestDb::start().await;
    let e = db.engine();

    let fetched_at = Utc::now() - Duration::minutes(30);
    db.seed_batch(
        fetched_at,
        &[
            ("USD", "1", Some("32.00"), Some("33.00")),
            ("EUR", "1", Some("36.00"), Some("37.00")),
            ("GBP", "1", Some("42.00"), Some("43.00")),
        ],
    )
    .await;

    let guards = json!({
        "USD": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
        "EUR": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
        "GBP": { "buy": guard("-5.0000", "5.0000"), "sell": guard("-5.0000", "5.0000") },
    });

    // Policy A (version 1): -1% buy, +1% sell
    let pol_a = db
        .seed_policy(
            MAIN,
            "-1.0000",
            "1.0000",
            guards.clone(),
            Some(24),
            None,
            None,
        )
        .await;

    // Policy B (version 2): -2% buy, +2% sell
    let pol_b: String = sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
            approved_by, reason, source_version) \
         VALUES (gen_random_uuid(), $1, 2, '-2.0000'::numeric, '2.0000'::numeric, $2, 24, NULL, \
                 NULL, now() - interval '12 hours', NULL, $3, 'second policy version', 1) \
         RETURNING id::text",
    )
    .bind(MAIN)
    .bind(&guards)
    .bind(OWNER_ID)
    .fetch_one(&db.pool)
    .await
    .expect("insert second policy version");

    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &owner_user(),
            vec![auto_line("USD"), auto_line("EUR"), auto_line("GBP")],
        )
        .await;
    let draft = settle(tx, got).await.expect("draft created");

    // Bind USD and EUR to policy A, GBP to policy B
    sqlx::query(
        "UPDATE rate_draft_lines SET policy_version_id = $1::uuid \
         WHERE draft_id = $2 AND currency_code IN ('USD', 'EUR')",
    )
    .bind(&pol_a)
    .bind(draft.id.as_uuid())
    .execute(&db.pool)
    .await
    .expect("bind USD and EUR to pol_a");

    sqlx::query(
        "UPDATE rate_draft_lines SET policy_version_id = $1::uuid \
         WHERE draft_id = $2 AND currency_code = 'GBP'",
    )
    .bind(&pol_b)
    .bind(draft.id.as_uuid())
    .execute(&db.pool)
    .await
    .expect("bind GBP to pol_b");

    let mut tx = db.begin().await;
    let got: Result<SubmitOutcome, RateEngineError> = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &draft.id, &key())
        .await;
    let outcome = settle(tx, got).await.expect("draft submitted");
    assert_eq!(outcome.status, DraftStatus::Approved);

    let pub_ref = publish_draft(&db, &e, &draft, None).await;
    assert_eq!(pub_ref.version, 1);

    let mut tx = db.begin().await;
    let usd_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), Side::Buy)
        .await
        .expect("usd buy query")
        .expect("usd buy published");
    let usd_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), Side::Sell)
        .await
        .expect("usd sell query")
        .expect("usd sell published");

    let eur_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("EUR"), Side::Buy)
        .await
        .expect("eur buy query")
        .expect("eur buy published");
    let eur_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("EUR"), Side::Sell)
        .await
        .expect("eur sell query")
        .expect("eur sell published");

    let gbp_buy = e
        .published
        .line_in(&mut tx, &main_branch(), code("GBP"), Side::Buy)
        .await
        .expect("gbp buy query")
        .expect("gbp buy published");
    let gbp_sell = e
        .published
        .line_in(&mut tx, &main_branch(), code("GBP"), Side::Sell)
        .await
        .expect("gbp sell query")
        .expect("gbp sell published");
    tx.commit().await.expect("commit read");

    // USD (Policy A: -1% / +1%): 32.00 * 0.99 = 31.68, 33.00 * 1.01 = 33.33
    assert_eq!(usd_buy.rate, rate("31.68"));
    assert_eq!(usd_sell.rate, rate("33.33"));

    // EUR (Policy A: -1% / +1%): 36.00 * 0.99 = 35.64, 37.00 * 1.01 = 37.37
    assert_eq!(eur_buy.rate, rate("35.64"));
    assert_eq!(eur_sell.rate, rate("37.37"));

    // GBP (Policy B: -2% / +2%): 42.00 * 0.98 = 41.16, 43.00 * 1.02 = 43.86
    assert_eq!(gbp_buy.rate, rate("41.16"));
    assert_eq!(gbp_sell.rate, rate("43.86"));

    // Assert each line's source_snapshot.policy_version_id names its own policy version
    let rows = sqlx::query(
        "SELECT trim(currency_code) AS code, source_snapshot FROM published_rate_lines \
         WHERE publication_id = $1 ORDER BY currency_code",
    )
    .bind(pub_ref.id.as_uuid())
    .fetch_all(&db.pool)
    .await
    .expect("read published_rate_lines");
    assert_eq!(rows.len(), 3);

    for r in rows {
        let curr: String = r.get("code");
        let snap: Value = r.get("source_snapshot");
        match curr.as_str() {
            "USD" | "EUR" => assert_eq!(
                snap["policy_version_id"].as_str(),
                Some(pol_a.as_str()),
                "{curr} must record policy A"
            ),
            "GBP" => assert_eq!(
                snap["policy_version_id"].as_str(),
                Some(pol_b.as_str()),
                "GBP must record policy B"
            ),
            other => panic!("unexpected currency {other}"),
        }
    }

    db.finish().await;
}
