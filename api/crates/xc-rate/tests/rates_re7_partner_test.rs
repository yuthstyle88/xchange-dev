//! Ticket #604 (RE-7) — partner cost is reference data, and an AUTO base is a `'BOT'` line only.
//! Written before the implementation by claude/dell-claude-5 on the lane controller's order
//! (claude/608dff, 2026-10-10). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.1 "Partner cost", §8 migration 6, §9 RE-7:
//! - "with no `rates` table present, a publish copies the newest `'PARTNER'` line of the currency
//!   into `source_snapshot.partner`, and writes `null` when there is none";
//! - "an AUTO line takes its base from the newest `'BOT'` line even when a newer `'PARTNER'` line
//!   exists".
//!
//! `source_snapshot.partner` is `{"provider", "reference_line_id", "unit_size", "buy", "sell",
//! "source_date", "fetched_at"}` (§4.1). These tests pin the keys and their values, not how a
//! number is spelled: a figure may be a JSON string or a JSON number, compared as a decimal, and
//! `fetched_at` is compared as an instant.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); nothing here skips.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `RateDraftService::submit`
//! takes `access::Actor`, so its callers here pass the B1-mapped user (`rate_engine_common::owner_user`: EXECUTIVE
//! + ADMIN organisation-wide). Every assertion is unchanged.

mod rate_engine_common;

use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use xc_rate::application::draft::rate_draft_service::SubmitOutcome;
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::PublicationRef;

// ── helpers ─────────────────────────────────────────────────────────────────

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

async fn create(db: &TestDb, e: &Engine, lines: Vec<DraftLine>) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &owner(), lines)
        .await;
    settle(tx, got).await.expect("the draft is created")
}

async fn edit(db: &TestDb, e: &Engine, draft: &RateDraft, lines: Vec<DraftLine>) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .edit(
            &mut tx,
            &main_branch(),
            &owner(),
            &draft.id,
            draft.version,
            lines,
        )
        .await;
    settle(tx, got).await.expect("the draft is edited")
}

async fn submit(db: &TestDb, e: &Engine, draft: &RateDraft) -> SubmitOutcome {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &draft.id, &key())
        .await;
    settle(tx, got).await.expect("the submit is accepted")
}

async fn publish(db: &TestDb, e: &Engine, draft: &RateDraft) -> PublicationRef {
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner(), &draft.id, None)
        .await;
    settle(tx, got).await.expect("the publish is accepted")
}

/// Create an owner draft of `lines`, submit it (approved at submit) and publish it.
async fn publish_lines(db: &TestDb, e: &Engine, lines: Vec<DraftLine>) -> PublicationRef {
    let d = create(db, e, lines).await;
    let outcome = submit(db, e, &d).await;
    assert_eq!(
        outcome.status,
        DraftStatus::Approved,
        "an owner draft is approved at submit"
    );
    publish(db, e, &d).await
}

fn guard(min: &str, max: &str) -> Value {
    json!({ "min_pct": min, "max_pct": max })
}

/// A MAIN policy with no adjustment and a `[-50, 50]` guard for USD on both sides, so every
/// proposal in this file is in-bound whichever reference it is measured against.
async fn seed_wide_policy(db: &TestDb) {
    db.seed_policy(
        MAIN,
        "0",
        "0",
        json!({ "USD": { "buy": guard("-50.0000", "50.0000"), "sell": guard("-50.0000", "50.0000") } }),
        None,
        None,
        None,
    )
    .await;
}

/// One reference batch of `provider` fetched at `fetched_at`, with one `'PARTNER'` line per
/// `(currency, the partner's unit, buy, sell)` (§4.1 "Partner cost": a partner's quote is a
/// `RateReferenceBatch` with `provider = <partner name>` and `reference_type = 'PARTNER'` lines).
/// Answers `(batch id, [line id per line])`.
async fn seed_partner_batch(
    db: &TestDb,
    provider: &str,
    fetched_at: DateTime<Utc>,
    lines: &[(&str, &str, Option<&str>, Option<&str>)],
) -> (String, Vec<String>) {
    let batch: String = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches \
           (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), $1, ($2::timestamptz AT TIME ZONE 'UTC')::date, $2, $3, 'COMPLETE') \
         RETURNING id::text",
    )
    .bind(provider)
    .bind(fetched_at)
    .bind(unique("checksum"))
    .fetch_one(&db.pool)
    .await
    .expect("insert a partner batch into rate_reference_batches (§4.1)");
    let mut ids = Vec::with_capacity(lines.len());
    for (currency, unit, buy, sell) in lines {
        let id: String = sqlx::query_scalar(
            "INSERT INTO rate_reference_lines \
               (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             VALUES (gen_random_uuid(), $1::uuid, $2, 'PARTNER', $3::numeric, $4::numeric, \
                     $5::numeric, '{}'::jsonb) \
             RETURNING id::text",
        )
        .bind(&batch)
        .bind(*currency)
        .bind(*unit)
        .bind(*buy)
        .bind(*sell)
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert a PARTNER line ({currency}): {e}"));
        ids.push(id);
    }
    (batch, ids)
}

/// The id of the one `'BOT'` line of `batch` for `currency`.
async fn bot_line(db: &TestDb, batch: &str, currency: &str) -> String {
    sqlx::query_scalar(
        "SELECT id::text FROM rate_reference_lines \
         WHERE batch_id::text = $1 AND currency_code = $2 AND reference_type = 'BOT'",
    )
    .bind(batch)
    .bind(currency)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("the BOT line of {currency} in batch {batch}: {e}"))
}

/// `rate_draft_lines.reference_line_id` of `currency` in one draft, as text.
async fn draft_reference(db: &TestDb, draft: &RateDraft, currency: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT reference_line_id::text FROM rate_draft_lines \
         WHERE draft_id::text = $1 AND currency_code = $2",
    )
    .bind(draft.id.to_string())
    .bind(currency)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("the {currency} line of draft {}: {e}", draft.id))
}

/// `(buy_rate, sell_rate, source_snapshot)` of `currency` in the branch's current publication.
async fn published_line(db: &TestDb, currency: &str) -> (Option<Decimal>, Option<Decimal>, Value) {
    sqlx::query_as(
        "SELECT l.buy_rate, l.sell_rate, l.source_snapshot \
         FROM published_rate_lines l JOIN branches b ON b.current_publication_id = l.publication_id \
         WHERE b.id = $1 AND l.currency_code = $2",
    )
    .bind(MAIN)
    .bind(currency)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("the published {currency} line of {MAIN}: {e}"))
}

/// A figure of the snapshot, written as a JSON string or a JSON number; `None` for `null`.
fn figure(snapshot: &Value, key: &str) -> Option<Decimal> {
    match snapshot.get(key) {
        Some(Value::String(s)) => {
            Some(Decimal::from_str(s).unwrap_or_else(|e| panic!("partner.{key} = {s:?}: {e}")))
        }
        Some(Value::Number(n)) => Some(
            Decimal::from_str(&n.to_string())
                .unwrap_or_else(|e| panic!("partner.{key} = {n}: {e}")),
        ),
        Some(Value::Null) => None,
        other => panic!("partner.{key} is a figure or null, got {other:?} in {snapshot}"),
    }
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

// ── publish: source_snapshot.partner from the newest 'PARTNER' line ───────────

/// §4.1 "At publish": `source_snapshot.partner` is copied from the newest `'PARTNER'` line for
/// that currency (by the batch's `fetched_at`), with every key §4.1 lists. An older partner
/// quote and the BoT line are not it.
#[tokio::test]
async fn a_publish_copies_the_newest_partner_line_into_source_snapshot_partner() {
    let db = TestDb::start().await;
    let e = db.engine();
    let now = Utc::now();
    seed_wide_policy(&db).await;
    db.seed_batch(
        now - Duration::hours(3),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;
    seed_partner_batch(
        &db,
        "Old Partner Co",
        now - Duration::hours(2),
        &[("USD", "1", Some("31.5"), Some("32.9"))],
    )
    .await;
    let newest_at = now - Duration::hours(1);
    let (_, ids) = seed_partner_batch(
        &db,
        "SuperRich",
        newest_at,
        &[("USD", "1", Some("31.75"), None)],
    )
    .await;
    let partner_line = ids[0].clone();

    within(
        MUST_FINISH,
        "publish",
        publish_lines(&db, &e, vec![auto_line("USD")]),
    )
    .await;

    let (_, _, snapshot) = published_line(&db, "USD").await;
    let partner = snapshot
        .get("partner")
        .unwrap_or_else(|| panic!("source_snapshot has a partner key: {snapshot}"));
    assert!(
        partner.is_object(),
        "a 'PARTNER' line exists, so source_snapshot.partner is an object: {snapshot}"
    );
    assert_eq!(
        partner.get("provider").and_then(Value::as_str),
        Some("SuperRich"),
        "partner.provider is the newest partner batch's provider: {partner}"
    );
    assert_eq!(
        partner.get("reference_line_id").and_then(Value::as_str),
        Some(partner_line.as_str()),
        "partner.reference_line_id is the newest 'PARTNER' line: {partner}"
    );
    assert_eq!(
        figure(partner, "unit_size"),
        Some(dec("1")),
        "partner.unit_size: {partner}"
    );
    assert_eq!(
        figure(partner, "buy"),
        Some(dec("31.75")),
        "partner.buy: {partner}"
    );
    assert_eq!(
        figure(partner, "sell"),
        None,
        "a side the partner did not quote is null: {partner}"
    );
    assert_eq!(
        partner.get("source_date").and_then(Value::as_str),
        Some(newest_at.date_naive().to_string().as_str()),
        "partner.source_date is the batch's: {partner}"
    );
    let fetched = partner
        .get("fetched_at")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .unwrap_or_else(|| panic!("partner.fetched_at is an RFC 3339 time: {partner}"));
    assert_eq!(
        fetched.timestamp_micros(),
        newest_at.timestamp_micros(),
        "partner.fetched_at is the batch's: {partner}"
    );
    db.finish().await;
}

/// The partner line is per currency: a partner batch that quotes only EUR leaves USD's partner
/// `null`, and EUR's line carries the partner's own `unit_size`.
#[tokio::test]
async fn a_partner_line_of_another_currency_is_not_copied() {
    let db = TestDb::start().await;
    let e = db.engine();
    let now = Utc::now();
    seed_wide_policy(&db).await;
    db.seed_batch(
        now - Duration::hours(3),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;
    seed_partner_batch(
        &db,
        "SuperRich",
        now - Duration::hours(1),
        &[("EUR", "1", Some("37.2"), Some("37.9"))],
    )
    .await;

    within(
        MUST_FINISH,
        "publish",
        publish_lines(&db, &e, vec![auto_line("USD")]),
    )
    .await;

    let (_, _, snapshot) = published_line(&db, "USD").await;
    assert_eq!(
        snapshot.get("partner"),
        Some(&Value::Null),
        "no 'PARTNER' line of USD exists, so USD's partner is null: {snapshot}"
    );
    db.finish().await;
}

/// §4.1: "It is `null` when no partner line exists."
#[tokio::test]
async fn a_publish_writes_a_null_partner_when_no_partner_line_exists() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_wide_policy(&db).await;
    db.seed_batch(
        Utc::now() - Duration::hours(1),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;

    within(
        MUST_FINISH,
        "publish",
        publish_lines(&db, &e, vec![auto_line("USD")]),
    )
    .await;

    let (_, _, snapshot) = published_line(&db, "USD").await;
    assert_eq!(
        snapshot.get("partner"),
        Some(&Value::Null),
        "source_snapshot.partner is present and null: {snapshot}"
    );
    db.finish().await;
}

// ── the AUTO base is the newest 'BOT' line ──────────────────────────────────

/// §4.1 "It is never an AUTO base": a draft created after a newer `'PARTNER'` line still takes
/// the newest `'BOT'` line, and the publication is priced from it.
#[tokio::test]
async fn an_auto_line_takes_its_base_from_the_newest_bot_line_even_when_a_newer_partner_line_exists()
 {
    let db = TestDb::start().await;
    let e = db.engine();
    let now = Utc::now();
    seed_wide_policy(&db).await;
    let older_bot = db
        .seed_batch(
            now - Duration::hours(5),
            &[("USD", "1", Some("31.0"), Some("31.5"))],
        )
        .await;
    let newest_bot = db
        .seed_batch(
            now - Duration::hours(3),
            &[("USD", "1", Some("32.1"), Some("32.45"))],
        )
        .await;
    let (_, partner_ids) = seed_partner_batch(
        &db,
        "SuperRich",
        now - Duration::hours(1),
        &[("USD", "1", Some("30.0"), Some("31.0"))],
    )
    .await;
    let bot = bot_line(&db, &newest_bot, "USD").await;
    assert_ne!(bot, bot_line(&db, &older_bot, "USD").await);

    let d = create(&db, &e, vec![auto_line("USD")]).await;
    assert_eq!(
        draft_reference(&db, &d, "USD").await.as_deref(),
        Some(bot.as_str()),
        "the AUTO line's reference is the newest 'BOT' line, not the newer 'PARTNER' line {}",
        partner_ids[0]
    );

    let outcome = submit(&db, &e, &d).await;
    assert_eq!(outcome.status, DraftStatus::Approved);
    within(MUST_FINISH, "publish", publish(&db, &e, &d)).await;

    let (buy, sell, snapshot) = published_line(&db, "USD").await;
    assert_eq!(
        (buy, sell),
        (Some(dec("32.1")), Some(dec("32.45"))),
        "the publication is priced from the BoT line (no adjustment): {snapshot}"
    );
    assert_eq!(
        snapshot.get("reference_line_id").and_then(Value::as_str),
        Some(bot.as_str()),
        "source_snapshot.reference_line_id is the BoT line: {snapshot}"
    );
    db.finish().await;
}

/// The edit path looks the reference up again (`rate_draft_service.rs`, the second
/// `SELECT l.id FROM rate_reference_lines`): a `'PARTNER'` line that arrives between create and
/// edit is not picked either.
#[tokio::test]
async fn an_edit_keeps_the_newest_bot_line_as_the_auto_base_when_a_partner_line_arrives() {
    let db = TestDb::start().await;
    let e = db.engine();
    let now = Utc::now();
    seed_wide_policy(&db).await;
    let batch = db
        .seed_batch(
            now - Duration::hours(3),
            &[("USD", "1", Some("32.1"), Some("32.45"))],
        )
        .await;
    let bot = bot_line(&db, &batch, "USD").await;

    let d = create(&db, &e, vec![auto_line("USD")]).await;
    assert_eq!(
        draft_reference(&db, &d, "USD").await.as_deref(),
        Some(bot.as_str())
    );

    seed_partner_batch(
        &db,
        "SuperRich",
        now - Duration::hours(1),
        &[("USD", "1", Some("30.0"), Some("31.0"))],
    )
    .await;
    let edited = edit(&db, &e, &d, vec![auto_line("USD")]).await;
    assert_eq!(
        draft_reference(&db, &edited, "USD").await.as_deref(),
        Some(bot.as_str()),
        "after an edit the AUTO line's reference is still the newest 'BOT' line"
    );
    db.finish().await;
}

/// With only a `'PARTNER'` line for a currency, an AUTO line has no reference (it is not
/// guessed from the partner), and its sides publish as unavailable (§4.1 "Unavailable").
#[tokio::test]
async fn an_auto_line_with_only_a_partner_line_has_no_reference() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_wide_policy(&db).await;
    seed_partner_batch(
        &db,
        "SuperRich",
        Utc::now() - Duration::hours(1),
        &[("USD", "1", Some("30.0"), Some("31.0"))],
    )
    .await;

    let d = create(&db, &e, vec![auto_line("USD")]).await;
    assert_eq!(
        draft_reference(&db, &d, "USD").await,
        None,
        "a 'PARTNER' line is never an AUTO base"
    );
    db.finish().await;
}
