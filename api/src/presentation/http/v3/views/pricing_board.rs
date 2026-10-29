//! View `pricing_board` (pricing-tiers.md §6): one branch's pricing board. A page load or a click, not polled.
//! Permission `ReferenceRatesRead`, through the reference-rates route as `rates_reference` does.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, RatePercent};
use xc_rate::application::draft::reference_base::ReferenceBase;
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::pricing::branch_policy::BranchPolicyStore;
use xc_rate::domain::pricing::display_settings::DisplaySettingsStore;
use xc_rate::domain::pricing::follow_plan::FollowResultStore;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::market_snapshot::MarketStaleness;
use xc_rate::domain::pricing::pricing_mode::{PricingMode, PricingModeStore};
use xc_rate::domain::pricing::pricing_suggestion::{
    PricingSuggestion, Suggestion, SuggestionConfig, SuggestionInput, TierPrice,
};
use xc_rate::domain::pricing::recommendation::{
    CounterRate, Recommendation, RecommendationInput, RecommendationSide,
};
use xc_rate::domain::pricing::row_status::{RowFacts, RowStatus};
use xc_rate::domain::pricing::suggestion_settings::SuggestionSettings;
use xc_rate::domain::pricing::tier_set::{PricingSource, TierSetSource, TierSetStore};
use xc_rate::infrastructure::market::market_feed_status_store::MarketFeedStatusStore;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use xc_rate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;
use xc_rate::infrastructure::pricing::display_settings_store::PgDisplaySettingsStore;
use xc_rate::infrastructure::pricing::follow_result_store::PgFollowResultStore;
use xc_rate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;
use xc_rate::infrastructure::pricing::suggestion_settings_store::PricingSuggestionSettingsStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::pricing_board::{
    PricingBandDto, PricingBoardData, PricingBpDto, PricingBranchPolicyDto, PricingDisplayDto,
    PricingFollowResultDto, PricingQuoteDto, PricingRecommendationDto, PricingRowDto,
    PricingSourceDto, PricingTierMarginDto, PricingTierRateDto, PricingTierSetDto,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct PricingBoardView {
    pool: PgPool,
}

impl PricingBoardView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for PricingBoardView {
    const NAME: &'static str = "pricing_board";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/reference-rates");
    const VERSIONED: bool = true;
    type Data = PricingBoardData;
    type Filter = NoFilter;

    fn own_version(
        &self,
        branch: &BranchId,
        _who: &Actor,
    ) -> impl Future<Output = AppResult<Option<i64>>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        async move { pricing_board_version(&pool, &branch_id).await.map(Some) }
    }

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        async move { load_pricing_board(&pool, &branch_id).await }
    }
}

async fn pricing_board_version(pool: &PgPool, branch_id: &str) -> AppResult<i64> {
    let now = Utc::now();
    let mut conn = pool.acquire().await.map_err(AppError::Database)?;

    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    let mut eat = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    };
    let eat_ts = |eat: &mut dyn FnMut(&[u8]), ts: Option<DateTime<Utc>>| match ts {
        Some(t) => eat(&t.timestamp_micros().to_le_bytes()),
        None => eat(&[0]),
    };

    let ref_rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT l.reference_type, MAX(b.fetched_at) \
         FROM rate_reference_lines l \
         JOIN rate_reference_batches b ON b.id = l.batch_id \
         WHERE l.reference_type IN ('BOT', 'MARKET_TT', 'MARKET_SR', 'START') \
         GROUP BY l.reference_type \
         ORDER BY l.reference_type",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(AppError::Database)?;

    let mut tt_ts = None;
    let mut sr_ts = None;
    for (rt, ts) in &ref_rows {
        eat(rt.as_bytes());
        eat_ts(&mut eat, Some(*ts));
        if rt == "MARKET_TT" {
            tt_ts = Some(*ts);
        } else if rt == "MARKET_SR" {
            sr_ts = Some(*ts);
        }
    }
    eat(&[
        u8::from(MarketStaleness::is_stale(tt_ts, now)),
        u8::from(MarketStaleness::is_stale(sr_ts, now)),
    ]);

    let feed_rows: Vec<(String, DateTime<Utc>, Option<String>)> = sqlx::query_as(
        "SELECT source, last_attempt_at, last_error FROM market_feed_status ORDER BY source",
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    for (src, at, err) in feed_rows {
        eat(src.as_bytes());
        eat_ts(&mut eat, Some(at));
        eat(err.as_deref().unwrap_or("").as_bytes());
    }

    let policy_ver: Option<(Uuid, i32)> = sqlx::query_as(
        "SELECT id, version FROM rate_policy_versions \
         WHERE branch_id = $1 AND valid_to IS NULL \
         ORDER BY version DESC LIMIT 1",
    )
    .bind(branch_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    match policy_ver {
        Some((id, ver)) => {
            eat(id.as_bytes());
            eat(&ver.to_le_bytes());
        }
        None => eat(&[0]),
    }

    let (sug_cnt, sug_ts): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT COUNT(*), MAX(updated_at) FROM pricing_suggestion_settings WHERE branch_id = $1",
    )
    .bind(branch_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    eat(&sug_cnt.to_le_bytes());
    eat_ts(&mut eat, sug_ts);

    let pub_id: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(branch_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(AppError::Database)?
            .flatten();
    match pub_id {
        Some(id) => eat(id.as_bytes()),
        None => eat(&[0]),
    }

    let (start_cnt, start_ver): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(version)::bigint, 0) FROM org_starting_prices",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    eat(&start_cnt.to_le_bytes());
    eat(&start_ver.to_le_bytes());

    let (ts_cnt, ts_max): (i64, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT COUNT(*), MAX(approved_at) FROM pricing_tier_sets")
            .fetch_one(&mut *conn)
            .await
            .map_err(AppError::Database)?;
    eat(&ts_cnt.to_le_bytes());
    eat_ts(&mut eat, ts_max);

    let (mode_rev_sum, mode_max): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT COALESCE(SUM(revision)::bigint, 0), MAX(updated_at) FROM pricing_modes",
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(AppError::Database)?;
    eat(&mode_rev_sum.to_le_bytes());
    eat_ts(&mut eat, mode_max);

    let (bp_cnt, bp_max): (i64, Option<DateTime<Utc>>) =
        sqlx::query_as("SELECT COUNT(*), MAX(updated_at) FROM pricing_branch_policy")
            .fetch_one(&mut *conn)
            .await
            .map_err(AppError::Database)?;
    eat(&bp_cnt.to_le_bytes());
    eat_ts(&mut eat, bp_max);

    let follow_max = PgFollowResultStore
        .newest_in(&mut conn)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    eat_ts(&mut eat, follow_max);

    let disp_cnt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rate_display_settings")
        .fetch_one(&mut *conn)
        .await
        .map_err(AppError::Database)?;
    let disp_max = PgDisplaySettingsStore
        .newest_in(&mut conn)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    eat(&disp_cnt.to_le_bytes());
    eat_ts(&mut eat, disp_max);

    eat(now.date_naive().to_string().as_bytes());

    Ok((hash & 0x001f_ffff_ffff_ffff) as i64)
}

fn parse_updated_at(s: &str) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return Some(ndt.and_utc());
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| ndt.and_utc())
}

async fn load_pricing_board(pool: &PgPool, branch_id: &str) -> AppResult<PricingBoardData> {
    let now = Utc::now();
    let mut tx = pool.begin().await.map_err(AppError::Database)?;

    let policy_row = sqlx::query(
        "SELECT id, value_tiers_on, anchor_source, buy_adjust_pct, sell_adjust_pct \
         FROM rate_policy_versions \
         WHERE branch_id = $1 AND valid_to IS NULL \
         ORDER BY version DESC LIMIT 1",
    )
    .bind(branch_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    let (
        policy_version_id,
        value_tiers_on,
        branch_anchor,
        policy_tiers,
        overrides,
        buy_adjust_pct,
        sell_adjust_pct,
    ) = match policy_row {
        Some(r) => {
            let id: Uuid = r.get("id");
            let tiers_on: bool = r.get("value_tiers_on");
            let anchor_str: String = r.get("anchor_source");
            let anchor = AnchorSource::parse(anchor_str.trim()).unwrap_or(AnchorSource::Bot);
            let buy_adj: Decimal = r.get("buy_adjust_pct");
            let sell_adj: Decimal = r.get("sell_adjust_pct");
            let buy_adjust_pct = RatePercent::parse(&buy_adj.to_string()).map_err(|_| {
                AppError::Internal(format!("invalid adjust_pct on policy version {id}"))
            })?;
            let sell_adjust_pct = RatePercent::parse(&sell_adj.to_string()).map_err(|_| {
                AppError::Internal(format!("invalid adjust_pct on policy version {id}"))
            })?;
            let pt = if tiers_on {
                PolicyTiersStore::for_version(&mut tx, &id)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?
            } else {
                None
            };
            let ovrs = CurrencyTierOverrideStore::for_version(&mut tx, &id)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
            (
                Some(id.to_string()),
                tiers_on,
                anchor,
                pt,
                ovrs,
                buy_adjust_pct,
                sell_adjust_pct,
            )
        }
        None => (
            None,
            false,
            AnchorSource::Bot,
            None,
            Vec::new(),
            RatePercent::ZERO,
            RatePercent::ZERO,
        ),
    };

    let bands = match &policy_tiers {
        Some(pt) => pt
            .bands()
            .into_iter()
            .enumerate()
            .map(|(idx, b)| PricingBandDto {
                tier_no: (idx + 1) as u8,
                lower_exclusive_thb: b.lower_exclusive_thb.map(|m| m.to_string()),
                upper_inclusive_thb: b.upper_inclusive_thb.map(|m| m.to_string()),
            })
            .collect(),
        None => Vec::new(),
    };

    let feed_status = MarketFeedStatusStore::all(&mut tx)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let source_rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT l.reference_type, MAX(b.fetched_at) \
         FROM rate_reference_lines l \
         JOIN rate_reference_batches b ON b.id = l.batch_id \
         WHERE l.reference_type IN ('MARKET_TT', 'MARKET_SR', 'BOT') \
         GROUP BY l.reference_type",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    let mut tt_stale = true;
    let mut sr_stale = true;
    let sources = [("TT", "MARKET_TT"), ("SR", "MARKET_SR"), ("BOT", "BOT")]
        .into_iter()
        .map(|(source, ref_type)| {
            let ts = source_rows
                .iter()
                .find(|(rt, _)| rt == ref_type)
                .map(|(_, ts)| *ts);
            let stale = match source {
                "TT" => {
                    tt_stale = MarketStaleness::is_stale(ts, now);
                    tt_stale
                }
                "SR" => {
                    sr_stale = MarketStaleness::is_stale(ts, now);
                    sr_stale
                }
                _ => false,
            };
            let error = match source {
                "TT" => feed_status
                    .get(&MarketSource::Tt)
                    .and_then(|a| a.error.clone()),
                "SR" => feed_status
                    .get(&MarketSource::SuperRich)
                    .and_then(|a| a.error.clone()),
                _ => None,
            };
            PricingSourceDto {
                source: source.to_string(),
                fetched_at: ts.map(|t| t.to_rfc3339()),
                stale,
                error,
            }
        })
        .collect();

    let pub_rows = sqlx::query(
        "SELECT trim(l.currency_code) AS currency_code, \
                l.value_tier, \
                l.buy_rate, \
                l.sell_rate, \
                l.source_snapshot->'partner'->>'cost' AS partner_cost, \
                p.published_at \
         FROM published_rate_lines l \
         JOIN branches b ON b.current_publication_id = l.publication_id \
         LEFT JOIN rate_publications p ON p.id = l.publication_id \
         WHERE b.id = $1",
    )
    .bind(branch_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    struct Tier1Published {
        buy: Option<Decimal>,
        sell: Option<Decimal>,
        partner_cost: Option<Decimal>,
        published_at: Option<DateTime<Utc>>,
    }

    let mut published_by_currency: HashMap<String, Vec<PricingTierRateDto>> = HashMap::new();
    let mut tier1_by_currency: HashMap<String, Tier1Published> = HashMap::new();
    for r in pub_rows {
        let code: String = r.get("currency_code");
        let value_tier: String = r.get("value_tier");
        let buy: Option<Decimal> = r.get("buy_rate");
        let sell: Option<Decimal> = r.get("sell_rate");
        let partner_cost_str: Option<String> = r.get("partner_cost");
        let published_at: Option<DateTime<Utc>> = r.get("published_at");
        let tier_no = match value_tier.as_str() {
            "T1" => 1,
            "T2" => 2,
            "T3" => 3,
            "T4" => 4,
            _ => 0,
        };
        if value_tier == "T1" || value_tier == "ALL" {
            tier1_by_currency.insert(
                code.clone(),
                Tier1Published {
                    buy,
                    sell,
                    partner_cost: partner_cost_str
                        .and_then(|s| Rate::parse(s.trim()).ok().map(Rate::value)),
                    published_at,
                },
            );
        }
        published_by_currency
            .entry(code)
            .or_default()
            .push(PricingTierRateDto {
                tier_no,
                buy: buy.map(|d| d.to_string()),
                sell: sell.map(|d| d.to_string()),
            });
    }
    for list in published_by_currency.values_mut() {
        list.sort_by_key(|item| item.tier_no);
    }

    let ref_currencies: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT trim(currency_code) \
         FROM rate_reference_lines \
         WHERE reference_type IN ('BOT', 'MARKET_TT', 'MARKET_SR')",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    let mut all_codes: BTreeSet<String> = published_by_currency.keys().cloned().collect();
    for (c,) in ref_currencies {
        all_codes.insert(c);
    }

    let unit_rows: Vec<(String, Decimal)> =
        sqlx::query_as("SELECT trim(code), unit_size FROM currencies")
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::Database)?;
    let unit_by_currency: HashMap<String, Decimal> = unit_rows.into_iter().collect();

    let market_rows = sqlx::query(
        "SELECT DISTINCT ON (l.currency_code, l.reference_type) \
             trim(l.currency_code) AS currency_code, \
             l.reference_type, \
             l.unit_size, \
             l.buy, \
             l.sell, \
             l.raw->>'decimals' AS decimals_str, \
             l.raw->>'source_updated_at' AS source_updated_at, \
             b.fetched_at \
         FROM rate_reference_lines l \
         JOIN rate_reference_batches b ON b.id = l.batch_id \
         WHERE l.reference_type IN ('MARKET_TT', 'MARKET_SR') \
         ORDER BY l.currency_code, l.reference_type, b.fetched_at DESC, b.id DESC",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    struct RawMarketLine {
        source_label: &'static str,
        market_source: MarketSource,
        unit_size: Decimal,
        buy: Option<Decimal>,
        sell: Option<Decimal>,
        decimals: Option<u32>,
        fetched_at: DateTime<Utc>,
    }

    let mut quotes_by_currency: HashMap<String, Vec<RawMarketLine>> = HashMap::new();
    let mut newest_tt: Option<(DateTime<Utc>, Option<String>)> = None;
    for r in market_rows {
        let code: String = r.get("currency_code");
        let ref_type: String = r.get("reference_type");
        let unit_size: Decimal = r.get("unit_size");
        let buy: Option<Decimal> = r.get("buy");
        let sell: Option<Decimal> = r.get("sell");
        let decimals_str: Option<String> = r.get("decimals_str");
        let source_updated_at: Option<String> = r.get("source_updated_at");
        let fetched_at: DateTime<Utc> = r.get("fetched_at");
        let (source_label, market_source) = match ref_type.as_str() {
            "MARKET_TT" => {
                if newest_tt
                    .as_ref()
                    .is_none_or(|(prev_ts, _)| fetched_at > *prev_ts)
                {
                    newest_tt = Some((fetched_at, source_updated_at));
                }
                ("TT", MarketSource::Tt)
            }
            "MARKET_SR" => ("SR", MarketSource::SuperRich),
            _ => continue,
        };
        quotes_by_currency
            .entry(code)
            .or_default()
            .push(RawMarketLine {
                source_label,
                market_source,
                unit_size,
                buy,
                sell,
                decimals: decimals_str.and_then(|s| s.trim().parse::<u32>().ok()),
                fetched_at,
            });
    }
    let tt_source_updated_at = newest_tt
        .and_then(|(_, s)| s)
        .as_deref()
        .and_then(parse_updated_at);

    let suggestion_settings = PricingSuggestionSettingsStore::for_branch(&mut tx, branch_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let branch_min_spread = PricingSuggestionSettingsStore::branch_min_spread(&mut tx, branch_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let branch_policies = PgBranchPolicyStore
        .for_branch_in(&mut tx, branch_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let (display_settings, display_from) = PgDisplaySettingsStore
        .effective_in(&mut tx, branch_id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut bases_by_currency: HashMap<String, (Option<Rate>, Option<Rate>)> = HashMap::new();
    let mut branch_tier_sets = HashMap::new();
    let mut main_tier_sets = HashMap::new();
    let mut main_modes = HashMap::new();
    let mut main_follow_results = HashMap::new();
    let b_id = BranchId::new(branch_id);
    for code in &all_codes {
        let Ok(cur_code) = CurrencyCode::parse(code) else {
            continue;
        };
        if let Some(ts) = PgTierSetStore
            .latest_in(&mut tx, branch_id, cur_code)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
        {
            if branch_id == "MAIN" {
                main_tier_sets.insert(code.clone(), ts.clone());
            }
            branch_tier_sets.insert(code.clone(), ts);
        }
        if branch_id != "MAIN"
            && let Some(ts) = PgTierSetStore
                .latest_in(&mut tx, "MAIN", cur_code)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?
        {
            main_tier_sets.insert(code.clone(), ts);
        }
        if let Some(m) = PgPricingModeStore
            .get_in(&mut tx, cur_code)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
        {
            main_modes.insert(code.clone(), m.mode);
        }
        if let Some(fr) = PgFollowResultStore
            .latest_in(&mut tx, cur_code)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
        {
            main_follow_results.insert(code.clone(), fr);
        }
        let Some(ref_id) = ReferenceBase::newest_in(&mut tx, &b_id, &cur_code)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
        else {
            continue;
        };
        let Some(ref_row) =
            sqlx::query("SELECT unit_size, buy, sell FROM rate_reference_lines WHERE id = $1")
                .bind(ref_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?
        else {
            continue;
        };
        let ref_unit: Decimal = ref_row.get("unit_size");
        let ref_buy: Option<Decimal> = ref_row.get("buy");
        let ref_sell: Option<Decimal> = ref_row.get("sell");
        let cur_unit = unit_by_currency.get(code).copied().unwrap_or(Decimal::ONE);
        if ref_unit > Decimal::ZERO && cur_unit > Decimal::ZERO {
            let b_buy = ref_buy
                .map(|b| b * cur_unit / ref_unit)
                .and_then(|b| Rate::parse(&b.to_string()).ok());
            let b_sell = ref_sell
                .map(|s| s * cur_unit / ref_unit)
                .and_then(|s| Rate::parse(&s.to_string()).ok());
            bases_by_currency.insert(code.clone(), (b_buy, b_sell));
        }
    }

    // TX-9 (20261216100000_drop_wide_transaction_columns.sql) moved transactions(code, principal) to transaction_lines(currency, thb_amount).
    let volume_rows: Vec<(String, Decimal)> = sqlx::query_as(
        "SELECT trim(l.currency), COALESCE(SUM(l.thb_amount), 0) \
         FROM transactions t \
         JOIN transaction_lines l ON l.transaction_id = t.id \
         WHERE t.branch_id = $1 \
           AND t.mode = 'LIVE' \
           AND t.status <> 'CANCELLED' \
           AND t.recorded_at >= now() - interval '7 days' \
         GROUP BY l.currency",
    )
    .bind(branch_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;
    let volume_thb: HashMap<String, Decimal> = volume_rows.into_iter().collect();

    tx.commit().await.map_err(AppError::Database)?;

    let listed_by_both: Vec<String> = all_codes
        .iter()
        .filter(|c| {
            quotes_by_currency.get(*c).is_some_and(|qs| {
                qs.iter().any(|q| q.market_source == MarketSource::Tt)
                    && qs
                        .iter()
                        .any(|q| q.market_source == MarketSource::SuperRich)
            })
        })
        .cloned()
        .collect();
    let enabled_set: HashSet<String> = published_by_currency.keys().cloned().collect();

    let mut ordered_codes: Vec<(String, bool)> = all_codes
        .into_iter()
        .map(|c| {
            let enabled = published_by_currency.contains_key(&c);
            (c, enabled)
        })
        .collect();
    ordered_codes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut rows = Vec::with_capacity(ordered_codes.len());
    let mut counters = Vec::new();
    for (code, enabled) in ordered_codes {
        let cur_unit = unit_by_currency.get(&code).copied().unwrap_or(Decimal::ONE);
        let unit_size = cur_unit.normalize().to_string();
        let ovr = overrides.iter().find(|o| o.currency.as_str() == code);
        let overridden = ovr.and_then(|o| o.tiers.as_ref()).is_some();
        let min_spread_pct = ovr.and_then(|o| o.min_spread_pct).map(|p| p.to_string());
        let row_anchor = ovr
            .and_then(|o| o.anchor_source)
            .unwrap_or(branch_anchor)
            .as_str()
            .to_string();

        let margins = match &policy_tiers {
            Some(pt) => pt
                .for_currency(ovr)
                .tiers()
                .iter()
                .map(|t| PricingTierMarginDto {
                    tier_no: t.tier_no,
                    buy_margin_pct: t.buy_margin_pct.to_string(),
                    sell_margin_pct: t.sell_margin_pct.to_string(),
                })
                .collect(),
            None => Vec::new(),
        };

        let raw_quotes = quotes_by_currency.remove(&code).unwrap_or_default();
        let mut quotes = Vec::with_capacity(raw_quotes.len());
        let mut domain_quotes = Vec::with_capacity(raw_quotes.len());
        let mut tt_decimals = None;
        for q in raw_quotes {
            if q.market_source == MarketSource::Tt {
                tt_decimals = q.decimals;
            }
            let q_stale = MarketStaleness::is_stale(Some(q.fetched_at), now);
            quotes.push(PricingQuoteDto {
                source: q.source_label.to_string(),
                buy: q.buy.map(|d| d.to_string()),
                sell: q.sell.map(|d| d.to_string()),
                fetched_at: q.fetched_at.to_rfc3339(),
                stale: q_stale,
            });
            domain_quotes.push(MarketQuote {
                source: q.market_source,
                buy: q.buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                sell: q.sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                unit_size: q.unit_size,
                fetched_at: q.fetched_at,
                stale: q_stale,
            });
        }

        let (better, better_from) = suggestion_settings.better_for(&code);
        let better_pct = better.iter().map(|d| d.normalize().to_string()).collect();
        let better_pct_from = better_from.as_str().to_string();
        let mut config = SuggestionConfig::v9_default();
        config.better = better;
        config.min_spread_pct = SuggestionSettings::min_spread_for(
            ovr.and_then(|o| o.min_spread_pct).map(|p| p.value()),
            branch_min_spread,
        );

        let t1 = tier1_by_currency.get(&code);
        let (base_buy, base_sell) = bases_by_currency
            .get(&code)
            .copied()
            .unwrap_or((None, None));

        let suggestion_obj = match CurrencyCode::parse(&code) {
            Ok(cur_code) => PricingSuggestion::of(&SuggestionInput {
                currency: cur_code,
                quotes: domain_quotes.clone(),
                tt_decimals,
                unit_size: cur_unit,
                base_buy,
                base_sell,
                buy_adjust_pct,
                sell_adjust_pct,
                partner_cost: t1.and_then(|t| t.partner_cost),
                published_tier1_buy: t1
                    .and_then(|t| t.buy)
                    .and_then(|d| Rate::parse(&d.to_string()).ok()),
                published_tier1_sell: t1
                    .and_then(|t| t.sell)
                    .and_then(|d| Rate::parse(&d.to_string()).ok()),
                config,
            }),
            Err(_) => {
                let (_, domain_flags) = MarketQuote::anchor_for(&domain_quotes);
                Suggestion {
                    tiers: None,
                    margins: None,
                    flags: domain_flags,
                }
            }
        };

        let suggestion = suggestion_obj.tiers.map(|tiers| {
            tiers
                .into_iter()
                .map(|t| PricingTierRateDto {
                    tier_no: t.tier_no,
                    buy: t.buy.map(|r| r.to_string()),
                    sell: t.sell.map(|r| r.to_string()),
                })
                .collect()
        });
        let selectable = !suggestion_obj.blocked();
        let published = published_by_currency.remove(&code).unwrap_or_default();
        let published_tiers = (!published.is_empty()).then(|| {
            published
                .iter()
                .map(|p| TierPrice {
                    tier_no: p.tier_no,
                    buy: p.buy.as_deref().and_then(|s| Rate::parse(s).ok()),
                    sell: p.sell.as_deref().and_then(|s| Rate::parse(s).ok()),
                })
                .collect::<Vec<_>>()
        });
        let status = format!(
            "{:?}",
            RowStatus::of_row(&RowFacts {
                suggestion: &suggestion_obj,
                priceable: CurrencyCode::parse(&code).is_ok(),
                published: published_tiers.as_deref(),
            })
        );
        let flags = suggestion_obj
            .flags
            .iter()
            .map(|f| format!("{f:?}"))
            .collect();

        if enabled
            && cur_unit > Decimal::ZERO
            && let Some(t) = t1
            && let (Some(buy), Some(sell)) = (t.buy, t.sell)
        {
            counters.push(CounterRate {
                currency: code.clone(),
                buy: buy / cur_unit,
                sell: sell / cur_unit,
                partner_cost: t.partner_cost,
                published_at: t.published_at,
                anchor: MarketQuote::anchor_for(&domain_quotes).0.cloned(),
            });
        }

        let resolved_policy = branch_policies.resolve(&code);
        let branch_ts = branch_tier_sets.remove(&code);
        let main_mode = if main_modes.get(&code) == Some(&PricingMode::Follow) {
            PricingSource::Follow
        } else if main_tier_sets.contains_key(&code) {
            PricingSource::Manual
        } else {
            PricingSource::Auto
        };
        let eff_mode = if branch_id == "MAIN" {
            main_mode
        } else if resolved_policy.own
            || branch_ts
                .as_ref()
                .is_some_and(|ts| ts.source == TierSetSource::Manual)
        {
            PricingSource::Manual
        } else if main_mode == PricingSource::Follow && resolved_policy.follow {
            // §15.5: only a branch whose currency row enrols it follows MAIN's FOLLOW (`H:` §7 last bullet).
            PricingSource::Follow
        } else {
            // Otherwise the branch shows what it publishes: a MAIN push reads as MANUAL, a FOLLOW row as FOLLOW,
            // nothing as AUTO (reviewer fix, claude/451f0116).
            branch_ts
                .as_ref()
                .map(|ts| ts.source.pricing_source())
                .unwrap_or(PricingSource::Auto)
        };
        let tier_set = if eff_mode == PricingSource::Auto {
            None
        } else {
            branch_ts.map(|ts| PricingTierSetDto {
                id: ts.id.to_string(),
                source: ts.source.as_str().to_string(),
                tiers: ts
                    .tiers
                    .iter()
                    .enumerate()
                    .map(|(i, t)| PricingTierRateDto {
                        tier_no: (i + 1) as u8,
                        buy: Some(t.buy.to_string()),
                        sell: Some(t.sell.to_string()),
                    })
                    .collect(),
                approved_by: ts.approved_by,
                approved_at: ts.approved_at.to_rfc3339(),
            })
        };
        let follow_result = if eff_mode == PricingSource::Follow {
            main_follow_results
                .remove(&code)
                .map(|fr| PricingFollowResultDto {
                    currency: fr.currency.as_str().to_string(),
                    status: fr.status.as_str().to_string(),
                    reasons: fr.reasons.iter().map(|r| r.as_str().to_string()).collect(),
                    planned_at: fr.planned_at.to_rfc3339(),
                })
        } else {
            None
        };

        rows.push(PricingRowDto {
            currency: code,
            unit_size,
            enabled,
            anchor_source: row_anchor,
            quotes,
            published,
            margins,
            overridden,
            min_spread_pct,
            better_pct,
            better_pct_from,
            suggestion,
            flags,
            selectable,
            status,
            mode: eff_mode.as_str().to_string(),
            tier_set,
            follow_result,
            branch_policy: PricingBranchPolicyDto {
                buy_bp: resolved_policy.buy_bp,
                sell_bp: resolved_policy.sell_bp,
                own: resolved_policy.own,
                follow: resolved_policy.follow,
                from: resolved_policy.from.as_str().to_string(),
            },
        });
    }

    let recommendations = Recommendation::list(&RecommendationInput {
        tt_stale,
        sr_stale,
        tt_source_updated_at,
        counters,
        listed_by_both,
        enabled: enabled_set,
        volume_thb,
    })
    .into_iter()
    .map(|r| PricingRecommendationDto {
        severity: format!("{:?}", r.severity),
        code: format!("{:?}", r.code),
        currency: r.currency,
        side: r.side.map(|s| match s {
            RecommendationSide::Buy => "buy".to_string(),
            RecommendationSide::Sell => "sell".to_string(),
        }),
    })
    .collect();

    let star_policy = branch_policies.resolve("*");

    Ok(PricingBoardData {
        branch: branch_id.to_string(),
        policy_version_id,
        value_tiers_on,
        anchor_source: branch_anchor.as_str().to_string(),
        bands,
        sources,
        rows,
        recommendations,
        branch_default: PricingBpDto {
            buy_bp: star_policy.buy_bp,
            sell_bp: star_policy.sell_bp,
        },
        display: PricingDisplayDto {
            tiers: display_settings.tiers,
            featured: display_settings.featured,
            highlight: display_settings.highlight,
            all_market: display_settings.all_market,
            from: display_from.as_str().to_string(),
        },
        is_main: branch_id == "MAIN",
        formula_source: None,
    })
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(PricingBoardView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingBandDto::export_all(&cfg)?;
    PricingSourceDto::export_all(&cfg)?;
    PricingQuoteDto::export_all(&cfg)?;
    PricingTierRateDto::export_all(&cfg)?;
    PricingTierMarginDto::export_all(&cfg)?;
    PricingRowDto::export_all(&cfg)?;
    PricingRecommendationDto::export_all(&cfg)?;
    PricingBpDto::export_all(&cfg)?;
    PricingDisplayDto::export_all(&cfg)?;
    PricingTierSetDto::export_all(&cfg)?;
    PricingFollowResultDto::export_all(&cfg)?;
    PricingBranchPolicyDto::export_all(&cfg)?;
    PricingBoardData::export_all(&cfg)?;
    Ok(())
}
