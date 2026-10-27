//! Published rates view (§4.8).

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_rate::domain::pricing::display_settings::DisplaySettingsStore;
use xc_rate::domain::pricing::follow_plan::{FollowResultStore, SourceFreshness};
use xc_rate::domain::pricing::follow_visibility::{FollowVisibility, VisibilityInput};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::tier_set::{PricingSource, TierSetSource, TierSetStore};
use xc_rate::domain::publication::freshness::Freshness;
use xc_rate::infrastructure::market::market_feed_status_store::MarketFeedStatusStore;
use xc_rate::infrastructure::pricing::display_settings_store::PgDisplaySettingsStore;
use xc_rate::infrastructure::pricing::follow_result_store::PgFollowResultStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::published_rates::{
    PublishedBandDto, PublishedBandHeadingDto, PublishedDisplayDto, PublishedLineDto,
    PublishedPublicationDto, PublishedRatesData,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct PublishedRatesBranchView {
    pool: PgPool,
}

impl PublishedRatesBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for PublishedRatesBranchView {
    const NAME: &'static str = "published_rates";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    const VERSIONED: bool = true;
    type Data = PublishedRatesData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move { published_rates_data(&pool, &branch).await }
    }
}

/// The branch's current publication in the `published_rates` shape, read once for this view and for
/// the public rates route (#995, `rates.md:454`). `load` moves here unchanged; the view and the
/// route both call it, so there is one reader.
pub async fn published_rates_data(
    pool: &PgPool,
    branch: &BranchId,
) -> AppResult<PublishedRatesData> {
    let branch_id = branch.as_str();

    let row = sqlx::query(
        "SELECT b.current_publication_id \
         FROM branches b WHERE b.id = $1",
    )
    .bind(branch_id)
    .fetch_optional(pool)
    .await
    .map_err(pos_error::AppError::Database)?;

    let current_pub_id: Option<Uuid> = match row {
        Some(r) => r.get(0),
        None => None,
    };

    let pub_id = match current_pub_id {
        Some(id) => id,
        None => {
            let display = load_display(pool, branch_id, &[]).await?;
            return Ok(PublishedRatesData {
                publication: None,
                source_date: None,
                stale: false,
                lines: vec![],
                band_headings: vec![],
                display,
            });
        }
    };

    let pub_row =
        sqlx::query("SELECT id, version, published_at FROM rate_publications WHERE id = $1")
            .bind(pub_id)
            .fetch_one(pool)
            .await
            .map_err(pos_error::AppError::Database)?;

    let pid: Uuid = pub_row.get(0);
    let pver: i32 = pub_row.get(1);
    let pat: DateTime<Utc> = pub_row.get(2);

    let lines_db = sqlx::query(
        "SELECT id, currency_code, denomination_group, unit_size, buy_rate, sell_rate, source_snapshot \
         FROM published_rate_lines WHERE publication_id = $1 AND tier_lower_exclusive_thb IS NULL ORDER BY currency_code ASC",
    )
    .bind(pub_id)
    .fetch_all(pool)
    .await
    .map_err(pos_error::AppError::Database)?;

    let tier_defs_db = sqlx::query(
        "SELECT t.tier_no, t.upper_inclusive_thb::text \
         FROM rate_policy_value_tiers t \
         JOIN rate_policy_versions pv ON pv.id = t.policy_version_id \
         WHERE pv.value_tiers_on = true \
           AND pv.id::text = ( \
               SELECT source_snapshot->>'policy_version_id' \
               FROM published_rate_lines \
               WHERE publication_id = $1 \
                 AND source_snapshot->>'policy_version_id' IS NOT NULL \
               ORDER BY currency_code ASC, denomination_group ASC, value_tier ASC \
               LIMIT 1 \
           ) \
         ORDER BY t.tier_no ASC",
    )
    .bind(pub_id)
    .fetch_all(pool)
    .await
    .map_err(pos_error::AppError::Database)?;

    let tier_defs: Vec<(i16, Option<String>)> = tier_defs_db
        .into_iter()
        .map(|r| (r.get::<i16, _>(0), r.get::<Option<String>, _>(1)))
        .collect();
    let headings = band_headings(&tier_defs);

    let tiered_rows_db = sqlx::query(
        "SELECT currency_code, denomination_group, value_tier, buy_rate, sell_rate \
         FROM published_rate_lines \
         WHERE publication_id = $1 AND value_tier <> 'ALL'",
    )
    .bind(pub_id)
    .fetch_all(pool)
    .await
    .map_err(pos_error::AppError::Database)?;

    let mut band_map = HashMap::new();
    for r in tiered_rows_db {
        let c: String = r.get(0);
        let g: String = r.get(1);
        let t: String = r.get(2);
        let buy_dec: Option<Decimal> = r.get(3);
        let sell_dec: Option<Decimal> = r.get(4);
        band_map.insert(
            (c.trim().to_string(), g, t),
            (
                buy_dec.map(|d| format!("{:.8}", d)),
                sell_dec.map(|d| format!("{:.8}", d)),
            ),
        );
    }

    let mut lines = Vec::new();
    let mut freshnesses = Vec::new();
    let now = Utc::now();

    for r in lines_db {
        let line_id: Uuid = r.get(0);
        let currency: String = r.get(1);
        let group: String = r.get(2);
        let unit_size_dec: Decimal = r.get(3);
        let buy_dec: Option<Decimal> = r.get(4);
        let sell_dec: Option<Decimal> = r.get(5);
        let snapshot: Value = r.get(6);

        let currency = currency.trim().to_string();
        let unit_size_str = unit_size_dec.normalize().to_string();

        let buy_rate = buy_dec.map(|d| format!("{:.8}", d));
        let sell_rate = sell_dec.map(|d| format!("{:.8}", d));

        freshnesses.push(Freshness::of_side(&snapshot, Side::Buy, now));
        freshnesses.push(Freshness::of_side(&snapshot, Side::Sell, now));

        let bands = headings
            .iter()
            .filter_map(|h| {
                let (b, s) = band_map.get(&(currency.clone(), group.clone(), h.tier.clone()))?;
                Some(PublishedBandDto {
                    tier: h.tier.clone(),
                    buy_rate: b.clone(),
                    sell_rate: s.clone(),
                })
            })
            .collect();

        let line_source = snapshot
            .get("pricing")
            .and_then(|p| p.get("source"))
            .and_then(|v| v.as_str())
            .and_then(TierSetSource::parse)
            .map(TierSetSource::pricing_source)
            .unwrap_or(PricingSource::Auto);

        let hidden_reason = if line_source == PricingSource::Follow {
            follow_hidden_reason(pool, branch_id, &currency, &snapshot, now).await?
        } else {
            None
        };

        lines.push(PublishedLineDto {
            currency,
            group,
            unit_size: unit_size_str,
            buy_rate,
            sell_rate,
            line_id: line_id.to_string(),
            bands,
            pricing_source: line_source.as_str().to_string(),
            hidden: hidden_reason.is_some(),
            hidden_reason: hidden_reason.map(|r| r.to_string()),
        });
    }

    let pub_freshness = Freshness::of_publication(freshnesses);
    let display = load_display(pool, branch_id, &lines).await?;

    Ok(PublishedRatesData {
        publication: Some(PublishedPublicationDto {
            id: pid.to_string(),
            version: pver,
            published_at: pat.to_rfc3339(),
        }),
        source_date: pub_freshness
            .source_date
            .map(|d| d.format("%Y-%m-%d").to_string()),
        stale: pub_freshness.stale,
        display,
        lines,
        band_headings: headings,
    })
}

fn band_headings(tiers: &[(i16, Option<String>)]) -> Vec<PublishedBandHeadingDto> {
    let mut prev_upper: Option<String> = None;
    let mut out = Vec::with_capacity(tiers.len());
    for (tier_no, upper) in tiers {
        let upper_inclusive_thb = if *tier_no == 4 { None } else { upper.clone() };
        let lower_exclusive_thb = if *tier_no == 1 {
            None
        } else {
            prev_upper.clone()
        };
        out.push(PublishedBandHeadingDto {
            tier: format!("T{tier_no}"),
            lower_exclusive_thb,
            upper_inclusive_thb: upper_inclusive_thb.clone(),
        });
        prev_upper = upper_inclusive_thb;
    }
    out
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(PublishedRatesBranchView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PublishedPublicationDto::export_all(&cfg)?;
    PublishedBandDto::export_all(&cfg)?;
    PublishedBandHeadingDto::export_all(&cfg)?;
    PublishedLineDto::export_all(&cfg)?;
    PublishedDisplayDto::export_all(&cfg)?;
    PublishedRatesData::export_all(&cfg)?;
    Ok(())
}

async fn follow_hidden_reason(
    pool: &PgPool,
    branch_id: &str,
    currency: &str,
    snapshot: &Value,
    now: DateTime<Utc>,
) -> AppResult<Option<&'static str>> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(pos_error::AppError::Database)?;
    let line_tier_set_id = snapshot
        .get("pricing")
        .and_then(|p| p.get("tier_set_id"))
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let line_tier_set = match line_tier_set_id {
        Some(id) => PgTierSetStore
            .get_in(&mut conn, id)
            .await
            .map_err(|e| pos_error::AppError::Internal(e.to_string()))?,
        None => None,
    };
    let line_base_tier_set_id = line_tier_set.as_ref().and_then(|ts| ts.base_tier_set_id);

    let anchor_source = match line_tier_set
        .as_ref()
        .and_then(|ts| ts.anchor.as_ref().map(|a| a.source))
    {
        Some(s) => Some(s),
        None => match line_base_tier_set_id {
            Some(base_id) => PgTierSetStore
                .get_in(&mut conn, base_id)
                .await
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?
                .and_then(|ts| ts.anchor.map(|a| a.source)),
            None => None,
        },
    };

    let source_current = match anchor_source {
        Some(src) => {
            let ref_type = match src {
                MarketSource::Tt => "MARKET_TT",
                MarketSource::SuperRich => "MARKET_SR",
                MarketSource::Bot => "BOT",
            };
            let fetched_at: Option<DateTime<Utc>> = sqlx::query_scalar(
                "SELECT MAX(b.fetched_at) \
                 FROM rate_reference_lines l \
                 JOIN rate_reference_batches b ON b.id = l.batch_id \
                 WHERE l.reference_type = $1",
            )
            .bind(ref_type)
            .fetch_one(&mut *conn)
            .await
            .map_err(pos_error::AppError::Database)?;
            let feed_status = MarketFeedStatusStore::all(&mut conn)
                .await
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
            let attempt = feed_status.get(&src);
            SourceFreshness {
                fetched_at,
                last_attempt_at: attempt.map(|a| a.at),
                error: attempt.and_then(|a| a.error.clone()),
            }
            .is_current(now)
        }
        None => false,
    };

    let cur_code = CurrencyCode::parse(currency).ok();
    let branch_latest_follow = match cur_code {
        Some(c) => PgTierSetStore
            .latest_follow_in(&mut conn, branch_id, c)
            .await
            .map_err(|e| pos_error::AppError::Internal(e.to_string()))?
            .map(|ts| ts.id),
        None => None,
    };
    let main_latest_follow = if branch_id == "MAIN" {
        branch_latest_follow
    } else {
        match cur_code {
            Some(c) => PgTierSetStore
                .latest_follow_in(&mut conn, "MAIN", c)
                .await
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?
                .map(|ts| ts.id),
            None => None,
        }
    };
    let main_result = match cur_code {
        Some(c) => PgFollowResultStore
            .latest_in(&mut conn, c)
            .await
            .map_err(|e| pos_error::AppError::Internal(e.to_string()))?
            .map(|fr| (fr.status, fr.tier_set_id)),
        None => None,
    };

    Ok(FollowVisibility::of(&VisibilityInput {
        line_source: PricingSource::Follow,
        line_tier_set_id,
        line_base_tier_set_id,
        at_main: branch_id == "MAIN",
        source_current,
        branch_latest_follow,
        main_latest_follow,
        main_result,
    })
    .map(|r| r.as_str()))
}

async fn load_display(
    pool: &PgPool,
    branch_id: &str,
    lines: &[PublishedLineDto],
) -> AppResult<PublishedDisplayDto> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(pos_error::AppError::Database)?;
    let (settings, _) = PgDisplaySettingsStore
        .effective_in(&mut conn, branch_id)
        .await
        .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
    let sr_order: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT trim(l.currency_code) \
         FROM rate_reference_lines l \
         WHERE l.reference_type = 'MARKET_SR' \
           AND l.batch_id = ( \
               SELECT b.id \
               FROM rate_reference_batches b \
               JOIN rate_reference_lines rl ON rl.batch_id = b.id \
               WHERE rl.reference_type = 'MARKET_SR' \
               ORDER BY b.fetched_at DESC, b.id DESC \
               LIMIT 1 \
           ) \
         ORDER BY 1 ASC",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(pos_error::AppError::Database)?;

    let mut priced: Vec<String> = Vec::new();
    for line in lines {
        if !priced.contains(&line.currency) {
            priced.push(line.currency.clone());
        }
    }
    let ordered = settings.order(&priced, &sr_order);
    Ok(PublishedDisplayDto {
        tiers: settings.tiers,
        featured: settings.featured,
        highlight: settings.highlight,
        order: ordered.ordered,
        more: ordered.more,
    })
}
