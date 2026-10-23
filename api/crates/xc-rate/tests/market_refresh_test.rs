//! XC-PRICING PX-5 (pricing-tiers.md §4, §12.1): `MarketRefresh::tick` and `MarketFeedStatusStore`, after V.9
//! `refreshMarket`/`latestMarket` (`src/pricing/market.mjs:101-137`, `test/pricing.mjs:60-70`). Sources are fakes
//! (no network); the database is a fresh test database. Needs PX-4's `MarketProvider` on the branch. Written before
//! the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit, delete or weaken; add tests
//! in a file of your own.

mod rate_engine_common;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rate_engine_common::TestDb;
use serde_json::json;
use xc_rate::application::market::market_refresh::{MarketRefresh, MarketRefreshConfig};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::infrastructure::market::market_feed_status_store::MarketFeedStatusStore;

struct Fixed(AppResult<ReferenceRates>);

impl RateSource for Fixed {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        let out = match &self.0 {
            Ok(r) => Ok(r.clone()),
            Err(e) => Err(AppError::Domain(e.to_string())),
        };
        Box::pin(async move { out })
    }
}

fn answer(source: &str, buy: &str) -> ReferenceRates {
    ReferenceRates {
        source: source.into(),
        fetched_at: "2026-10-18T03:00:00+00:00".into(),
        kind: "MARKET".into(),
        rows: vec![
            json!({"code": "USD", "band": "L", "denoms": "100-50", "buy": buy, "sell": "33.9", "decimals": 2, "source_updated_at": "2026-10-18"}),
            json!({"code": "SCOT", "band": "0", "denoms": "", "buy": "40", "sell": "41", "decimals": null, "source_updated_at": "2026-10-18"}),
        ],
    }
}

async fn status(
    db: &TestDb,
) -> HashMap<MarketSource, xc_rate::infrastructure::market::market_feed_status_store::MarketAttempt>
{
    let mut conn = db.pool.acquire().await.unwrap();
    MarketFeedStatusStore::all(&mut conn).await.expect("status")
}

async fn lines(db: &TestDb, reference_type: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM rate_reference_lines WHERE reference_type = $1")
        .bind(reference_type)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn one_tick_writes_each_source_and_records_its_attempt() {
    let db = TestDb::start().await;
    let refresh = MarketRefresh::new(
        db.pool.clone(),
        vec![
            (
                MarketSource::Tt,
                Arc::new(Fixed(Ok(answer("TT", "33.2")))) as Arc<dyn RateSource>,
            ),
            (
                MarketSource::SuperRich,
                Arc::new(Fixed(Ok(answer("SR", "33.33")))) as Arc<dyn RateSource>,
            ),
        ],
    );
    let now = Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap();
    let report = refresh.tick(now).await;
    assert_eq!(report.results.len(), 2);
    assert!(report.results.iter().all(|(_, r)| r.is_ok()), "{report:?}");
    assert_eq!(
        lines(&db, "MARKET_TT").await,
        1,
        "SCOT is not in `currencies`"
    );
    assert_eq!(lines(&db, "MARKET_SR").await, 1);
    let tt = status(&db)
        .await
        .remove(&MarketSource::Tt)
        .expect("attempt");
    assert_eq!((tt.at, tt.error), (now, None));

    refresh.tick(now).await;
    assert_eq!(
        lines(&db, "MARKET_TT").await,
        1,
        "an unchanged answer writes no new batch"
    );
}

#[tokio::test]
async fn a_failing_source_records_its_error_and_the_other_is_still_written() {
    let db = TestDb::start().await;
    let refresh = MarketRefresh::new(
        db.pool.clone(),
        vec![
            (
                MarketSource::Tt,
                Arc::new(Fixed(Err(AppError::Domain("TT: HTTP 503".into()))))
                    as Arc<dyn RateSource>,
            ),
            (
                MarketSource::SuperRich,
                Arc::new(Fixed(Ok(answer("SR", "33.33")))) as Arc<dyn RateSource>,
            ),
        ],
    );
    let now = Utc.with_ymd_and_hms(2026, 10, 18, 3, 6, 0).unwrap();
    let report = refresh.tick(now).await;
    assert!(report.results[0].1.as_ref().unwrap_err().contains("503"));
    assert!(report.results[1].1.is_ok());
    assert_eq!(lines(&db, "MARKET_SR").await, 1);
    let all = status(&db).await;
    let tt = all.get(&MarketSource::Tt).expect("attempt");
    assert_eq!(tt.at, now);
    assert!(tt.error.as_deref().unwrap_or("").contains("HTTP 503"));
    assert_eq!(all.get(&MarketSource::SuperRich).unwrap().error, None);
}

#[tokio::test]
async fn status_keeps_the_last_attempt_and_cuts_errors_to_200_characters() {
    let db = TestDb::start().await;
    let t0 = Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap();
    assert!(status(&db).await.is_empty());
    MarketFeedStatusStore::record(&db.pool, MarketSource::Tt, t0, Some(&"x".repeat(500)))
        .await
        .unwrap();
    assert_eq!(
        status(&db).await[&MarketSource::Tt]
            .error
            .clone()
            .unwrap()
            .chars()
            .count(),
        200
    );
    let t1 = t0 + chrono::Duration::minutes(5);
    MarketFeedStatusStore::record(&db.pool, MarketSource::Tt, t1, None)
        .await
        .unwrap();
    let last = status(&db).await.remove(&MarketSource::Tt).unwrap();
    assert_eq!(
        (last.at, last.error),
        (t1, None),
        "one row per source, replaced"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM market_feed_status")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn the_loop_is_off_unless_enabled() {
    // Env is process-wide: read the default only when the variable is unset in this run.
    if std::env::var("MARKET_FEED_ENABLED").is_err() {
        let c = MarketRefreshConfig::from_env();
        assert!(!c.enabled);
        assert_eq!(c.interval.as_secs(), 300);
    }
}
