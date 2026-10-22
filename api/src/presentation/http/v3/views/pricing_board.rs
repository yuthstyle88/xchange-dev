//! View `pricing_board` (pricing-tiers.md §6): one branch's pricing board. A page load or a click, not polled.
//! Permission `ReferenceRatesRead`, through the reference-rates route as `rates_reference` does.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
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
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::market_snapshot::MarketStaleness;
use xc_rate::domain::pricing::pricing_suggestion::{
    PricingSuggestion, Suggestion, SuggestionConfig, SuggestionInput,
};
use xc_rate::domain::pricing::recommendation::{
    CounterRate, Recommendation, RecommendationInput, RecommendationSide,
};
use xc_rate::domain::pricing::row_status::RowStatus;
use xc_rate::infrastructure::market::market_feed_status_store::MarketFeedStatusStore;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use xc_rate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;
use xc_rate::infrastructure::pricing::suggestion_settings_store::PricingSuggestionSettingsStore;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::pricing_board::{
    PricingBandDto, PricingBoardData, PricingQuoteDto, PricingRecommendationDto, PricingRowDto,
    PricingSourceDto, PricingTierMarginDto, PricingTierRateDto,
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
    const VERSIONED: bool = false;
    type Data = PricingBoardData;
    type Filter = NoFilter;

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
            let buy_adjust_pct =
                RatePercent::parse(&buy_adj.to_string()).unwrap_or(RatePercent::ZERO);
            let sell_adjust_pct =
                RatePercent::parse(&sell_adj.to_string()).unwrap_or(RatePercent::ZERO);
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
                        .and_then(|s| Decimal::from_str(s.trim()).ok()),
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

    let mut bases_by_currency: HashMap<String, (Option<Rate>, Option<Rate>)> = HashMap::new();
    let b_id = BranchId::new(branch_id);
    for code in &all_codes {
        let Ok(cur_code) = CurrencyCode::parse(code) else {
            continue;
        };
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
        let cur_unit = unit_by_currency
            .get(code)
            .copied()
            .unwrap_or(Decimal::ONE);
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
                    && qs.iter().any(|q| q.market_source == MarketSource::SuperRich)
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
        let cur_unit = unit_by_currency
            .get(&code)
            .copied()
            .unwrap_or(Decimal::ONE);
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
        if let Some(ms) = ovr.and_then(|o| o.min_spread_pct) {
            config.min_spread_pct = ms.value();
        }

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
        let status = format!("{:?}", RowStatus::of(&suggestion_obj, enabled, enabled));
        let flags = suggestion_obj
            .flags
            .iter()
            .map(|f| format!("{f:?}"))
            .collect();

        if enabled {
            let (c_buy, c_sell) = match t1 {
                Some(t) if cur_unit > Decimal::ZERO => (
                    t.buy.map(|d| d / cur_unit).unwrap_or(Decimal::ZERO),
                    t.sell.map(|d| d / cur_unit).unwrap_or(Decimal::ZERO),
                ),
                _ => (Decimal::ZERO, Decimal::ZERO),
            };
            counters.push(CounterRate {
                currency: code.clone(),
                buy: c_buy,
                sell: c_sell,
                partner_cost: t1.and_then(|t| t.partner_cost),
                published_at: t1.and_then(|t| t.published_at),
                anchor: MarketQuote::anchor_for(&domain_quotes).0.cloned(),
            });
        }

        let published = published_by_currency.remove(&code).unwrap_or_default();

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

    Ok(PricingBoardData {
        branch: branch_id.to_string(),
        policy_version_id,
        value_tiers_on,
        anchor_source: branch_anchor.as_str().to_string(),
        bands,
        sources,
        rows,
        recommendations,
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
    PricingBoardData::export_all(&cfg)?;
    Ok(())
}
