//! `MarketRateSource`: the one HTTP core every market feed shares (pricing-tiers.md §4, V.9 `market.mjs:77-89`).
//!
//! SKELETON (XC-PRICING PX-0): `fetch` is PX-4's. Constants, constructors and signatures stay.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::json;

use crate::domain::pricing::market_feed::{MarketAccept, MarketBody, MarketFeed};
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};

/// V.9 `AbortSignal.timeout(10000)` (`market.mjs:78`).
pub const MARKET_TIMEOUT: Duration = Duration::from_secs(10);
/// V.9 `UA` (`market.mjs:11`).
pub const MARKET_USER_AGENT: &str = "XCHANGE-rate-monitor/1.0 (+https://xc-th.work)";

pub type MarketClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub struct MarketRateSource {
    pub(crate) feed: Arc<dyn MarketFeed>,
    pub(crate) client: reqwest::Client,
    pub(crate) clock: MarketClock,
}

impl MarketRateSource {
    pub fn new(feed: Arc<dyn MarketFeed>) -> Self {
        Self::with(feed, MARKET_TIMEOUT, Arc::new(Utc::now))
    }

    /// Tests pass a short timeout and a fixed clock.
    pub fn with(feed: Arc<dyn MarketFeed>, timeout: Duration, clock: MarketClock) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(MARKET_USER_AGENT)
            .build()
            .unwrap_or_default();
        Self {
            feed,
            client,
            clock,
        }
    }
}

impl RateSource for MarketRateSource {
    /// GET every `feed.requests()` in order (`Accept: application/json` or `text/html` per request); a non-2xx status is `HTTP <status>`; a
    /// failed optional request becomes `body: None`; then `feed.parse`, then `MarketSnapshot::headline`.
    /// Returns `ReferenceRates { source: "TT" | "SR", fetched_at: clock() as RFC 3339, kind: "MARKET", rows }`, one
    /// JSON row per headline: `code`, `band`, `denoms`, `buy`, `sell` (decimal strings or null), `decimals`,
    /// `source_updated_at`. Any failure is an `AppError` whose text names the source (`TT: …`, `SR: …`).
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        Box::pin(async move {
            let src = match self.feed.source() {
                MarketSource::Tt => "TT",
                MarketSource::SuperRich => "SR",
                MarketSource::Bot => "BOT",
            };
            let mut bodies = Vec::new();
            for req in self.feed.requests() {
                let accept = match req.accept {
                    MarketAccept::Json => "application/json",
                    MarketAccept::Html => "text/html",
                };
                let res = self
                    .client
                    .get(&req.url)
                    .header("Accept", accept)
                    .send()
                    .await;
                if req.optional {
                    let body = match res {
                        Ok(r) if r.status().is_success() => r.text().await.ok(),
                        _ => None,
                    };
                    bodies.push(MarketBody { url: req.url, body });
                } else {
                    let r = res.map_err(|e| AppError::Domain(format!("{src}: {e}")))?;
                    let status = r.status();
                    if !status.is_success() {
                        return Err(AppError::Domain(format!("{src}: HTTP {}", status.as_u16())));
                    }
                    let text = r
                        .text()
                        .await
                        .map_err(|e| AppError::Domain(format!("{src}: {e}")))?;
                    bodies.push(MarketBody {
                        url: req.url,
                        body: Some(text),
                    });
                }
            }

            let snapshot = self
                .feed
                .parse(&bodies)
                .map_err(|e| AppError::Domain(e.0))?;
            let source_updated_at = snapshot.source_updated_at.clone();
            let rows = snapshot
                .headline()
                .into_iter()
                .map(|r| {
                    json!({
                        "code": r.code,
                        "band": r.band,
                        "denoms": r.denoms,
                        "buy": r.buy.map(|d| d.normalize().to_string()),
                        "sell": r.sell.map(|d| d.normalize().to_string()),
                        "decimals": r.decimals,
                        "source_updated_at": source_updated_at,
                    })
                })
                .collect();

            Ok(ReferenceRates {
                source: src.to_string(),
                fetched_at: (self.clock)().to_rfc3339(),
                kind: "MARKET".to_string(),
                rows,
            })
        })
    }
}
