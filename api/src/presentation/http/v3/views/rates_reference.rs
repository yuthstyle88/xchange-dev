//! The `rates_reference` view (§7, §12.16).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::{BotRateSource, Clock};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::rates_reference::{
    RatesReferenceData, RatesReferenceFilter, RatesReferenceRow,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

fn format_decimal_4(dec: Decimal) -> String {
    let rounded =
        dec.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    format!("{rounded:.4}")
}

fn parse_and_format_rate(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Number(n) => {
            if let Ok(d) = Decimal::from_str(&n.to_string()) {
                format_decimal_4(d)
            } else {
                String::new()
            }
        }
        serde_json::Value::String(s) => {
            if let Ok(d) = Decimal::from_str(s) {
                format_decimal_4(d)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

fn parse_rows(rows: Vec<serde_json::Value>) -> Vec<RatesReferenceRow> {
    rows.into_iter()
        .map(|r| {
            let code = r
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let date = r
                .get("date")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let unit = r.get("unit").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            let buying_sight = r
                .get("buyingSight")
                .map(parse_and_format_rate)
                .unwrap_or_default();
            let buying_transfer = r
                .get("buyingTransfer")
                .map(parse_and_format_rate)
                .unwrap_or_default();
            let selling = r
                .get("selling")
                .map(parse_and_format_rate)
                .unwrap_or_default();
            RatesReferenceRow {
                code,
                date,
                unit,
                buying_sight,
                buying_transfer,
                selling,
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct CacheState {
    version: i64,
    rows: Vec<RatesReferenceRow>,
    fetched_at: String,
    error: Option<String>,
    checked_at: Option<DateTime<Utc>>,
}

pub struct ReferenceCache {
    source: Arc<dyn RateSource>,
    clock: Clock,
    state: tokio::sync::Mutex<CacheState>,
}

impl ReferenceCache {
    pub fn new(source: Arc<dyn RateSource>, clock: Clock) -> Self {
        Self {
            source,
            clock,
            state: tokio::sync::Mutex::new(CacheState {
                version: 1,
                rows: Vec::new(),
                fetched_at: String::new(),
                error: None,
                checked_at: None,
            }),
        }
    }

    pub async fn ensure_fresh(&self, force_refresh: bool) {
        let mut state = self.state.lock().await;
        let now = (self.clock)();

        if !force_refresh {
            if let Some(checked) = state.checked_at
                && now - checked < chrono::Duration::seconds(60)
            {
                return;
            }
        } else if state.checked_at == Some(now) {
            return;
        }

        match self.source.fetch().await {
            Ok(ref_rates) => {
                let new_rows = parse_rows(ref_rates.rows);
                let new_fetched_at = ref_rates.fetched_at;
                if state.checked_at.is_some() {
                    let err_cleared = state.error.is_some();
                    let rows_changed = state.rows != new_rows;
                    if err_cleared || rows_changed {
                        state.version += 1;
                    }
                }
                state.rows = new_rows;
                state.fetched_at = new_fetched_at;
                state.error = None;
                state.checked_at = Some(now);
            }
            Err(err) => {
                let err_msg = match err {
                    AppError::Domain(s) => s,
                    _ => "เซิร์ฟเวอร์ขัดข้อง".to_string(),
                };
                if state.checked_at.is_some() {
                    let err_changed = state.error.as_deref() != Some(&err_msg);
                    if err_changed {
                        state.version += 1;
                    }
                }
                state.error = Some(err_msg);
                state.checked_at = Some(now);
            }
        }
    }

    pub async fn version(&self) -> i64 {
        self.state.lock().await.version
    }

    pub async fn data(&self) -> RatesReferenceData {
        let state = self.state.lock().await;
        RatesReferenceData {
            rows: state.rows.clone(),
            fetched_at: state.fetched_at.clone(),
            error: state.error.clone(),
        }
    }
}

pub struct RatesReferenceView {
    cache: Arc<ReferenceCache>,
}

impl RatesReferenceView {
    pub fn new(cache: Arc<ReferenceCache>) -> Self {
        Self { cache }
    }
}

impl BranchView for RatesReferenceView {
    const NAME: &'static str = "rates_reference";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "/api/reference-rates");
    const VERSIONED: bool = true;
    type Data = RatesReferenceData;
    type Filter = RatesReferenceFilter;

    fn own_version(
        &self,
        _branch: &BranchId,
        _who: &Actor,
    ) -> impl Future<Output = AppResult<Option<i64>>> + Send {
        let cache = self.cache.clone();
        async move {
            cache.ensure_fresh(false).await;
            Ok(Some(cache.version().await))
        }
    }

    fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let cache = self.cache.clone();
        let fresh = filter.is_fresh();
        async move {
            if fresh {
                cache.ensure_fresh(true).await;
            }
            Ok(cache.data().await)
        }
    }
}

static CACHE: std::sync::LazyLock<Arc<ReferenceCache>> = std::sync::LazyLock::new(|| {
    let source = Arc::new(BotRateSource::bot());
    let clock: Clock = Arc::new(Utc::now);
    Arc::new(ReferenceCache::new(source, clock))
});

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    let view = Arc::new(RatesReferenceView::new(CACHE.clone()));
    ViewEndpoint::register(cfg, view);
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RatesReferenceRow::export_all(&cfg)?;
    RatesReferenceData::export_all(&cfg)?;
    RatesReferenceFilter::export_all(&cfg)?;
    Ok(())
}
