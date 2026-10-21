//! View `pricing_board` (pricing-tiers.md §6): one branch's pricing board. A page load or a click, not polled.
//! Permission `ReferenceRatesRead`, through the reference-rates route as `rates_reference` does.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Rate;
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::pricing::market_quote::{MarketQuote, MarketSource};
use xc_rate::domain::pricing::pricing_suggestion::Suggestion;
use xc_rate::domain::pricing::row_status::RowStatus;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use xc_rate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;

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

async fn load_pricing_board(pool: &PgPool, branch_id: &str) -> AppResult<PricingBoardData> {
    let mut tx = pool.begin().await.map_err(AppError::Database)?;

    let policy_row = sqlx::query(
        "SELECT id, value_tiers_on, anchor_source \
         FROM rate_policy_versions \
         WHERE branch_id = $1 AND valid_to IS NULL \
         ORDER BY version DESC LIMIT 1",
    )
    .bind(branch_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    let (policy_version_id, value_tiers_on, branch_anchor, policy_tiers, overrides) =
        match policy_row {
            Some(r) => {
                let id: Uuid = r.get("id");
                let tiers_on: bool = r.get("value_tiers_on");
                let anchor_str: String = r.get("anchor_source");
                let anchor = AnchorSource::parse(anchor_str.trim()).unwrap_or(AnchorSource::Bot);
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
                (Some(id.to_string()), tiers_on, anchor, pt, ovrs)
            }
            None => (None, false, AnchorSource::Bot, None, Vec::new()),
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

    let sources = [("TT", "MARKET_TT"), ("SR", "MARKET_SR"), ("BOT", "BOT")]
        .into_iter()
        .map(|(source, ref_type)| {
            let fetched_at = source_rows
                .iter()
                .find(|(rt, _)| rt == ref_type)
                .map(|(_, ts)| ts.to_rfc3339());
            PricingSourceDto {
                source: source.to_string(),
                fetched_at,
                // P-2: stale and error wait for V.9's thresholds and fetchers (PT-11/12).
                stale: false,
                error: None,
            }
        })
        .collect();

    let pub_rows = sqlx::query(
        "SELECT trim(l.currency_code) AS currency_code, l.value_tier, l.buy_rate, l.sell_rate \
         FROM published_rate_lines l \
         JOIN branches b ON b.current_publication_id = l.publication_id \
         WHERE b.id = $1",
    )
    .bind(branch_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    let mut published_by_currency: HashMap<String, Vec<PricingTierRateDto>> = HashMap::new();
    for r in pub_rows {
        let code: String = r.get("currency_code");
        let value_tier: String = r.get("value_tier");
        let buy: Option<Decimal> = r.get("buy_rate");
        let sell: Option<Decimal> = r.get("sell_rate");
        let tier_no = match value_tier.as_str() {
            "T1" => 1,
            "T2" => 2,
            "T3" => 3,
            "T4" => 4,
            _ => 0,
        };
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
        fetched_at: DateTime<Utc>,
    }

    let mut quotes_by_currency: HashMap<String, Vec<RawMarketLine>> = HashMap::new();
    for r in market_rows {
        let code: String = r.get("currency_code");
        let ref_type: String = r.get("reference_type");
        let unit_size: Decimal = r.get("unit_size");
        let buy: Option<Decimal> = r.get("buy");
        let sell: Option<Decimal> = r.get("sell");
        let fetched_at: DateTime<Utc> = r.get("fetched_at");
        let (source_label, market_source) = match ref_type.as_str() {
            "MARKET_TT" => ("TT", MarketSource::Tt),
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
                fetched_at,
            });
    }

    tx.commit().await.map_err(AppError::Database)?;

    let mut ordered_codes: Vec<(String, bool)> = all_codes
        .into_iter()
        .map(|c| {
            let enabled = published_by_currency.contains_key(&c);
            (c, enabled)
        })
        .collect();
    ordered_codes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut rows = Vec::with_capacity(ordered_codes.len());
    for (code, enabled) in ordered_codes {
        let unit_size = unit_by_currency
            .get(&code)
            .copied()
            .unwrap_or(Decimal::ONE)
            .normalize()
            .to_string();
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
        for q in raw_quotes {
            quotes.push(PricingQuoteDto {
                source: q.source_label.to_string(),
                buy: q.buy.map(|d| d.to_string()),
                sell: q.sell.map(|d| d.to_string()),
                fetched_at: q.fetched_at.to_rfc3339(),
                // P-2: stale waits for V.9's thresholds (PT-11/12).
                stale: false,
            });
            domain_quotes.push(MarketQuote {
                source: q.market_source,
                buy: q.buy.and_then(|d| Rate::parse(&d.to_string()).ok()),
                sell: q.sell.and_then(|d| Rate::parse(&d.to_string()).ok()),
                unit_size: q.unit_size,
                fetched_at: q.fetched_at,
                stale: false,
            });
        }

        let (_, domain_flags) = MarketQuote::anchor_for(&domain_quotes);
        let suggestion_obj = Suggestion {
            tiers: None,
            margins: None,
            flags: domain_flags,
        };
        let selectable = !suggestion_obj.blocked();
        let status = format!("{:?}", RowStatus::of(&suggestion_obj, enabled, enabled));
        let flags = suggestion_obj
            .flags
            .iter()
            .map(|f| format!("{f:?}"))
            .collect();

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
            // PX-9 (XC-PRICING): the effective `better` and the suggestion. The skeleton only keeps this compiling.
            better_pct: Vec::new(),
            better_pct_from: "default".to_string(),
            suggestion: None,
            flags,
            selectable,
            status,
        });
    }

    Ok(PricingBoardData {
        branch: branch_id.to_string(),
        policy_version_id,
        value_tiers_on,
        anchor_source: branch_anchor.as_str().to_string(),
        bands,
        sources,
        rows,
        recommendations: Vec::new(),
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
