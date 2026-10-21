//! XC-PRICING PX-4 (pricing-tiers.md §4): the shared HTTP core `MarketRateSource` and the shared `MarketProvider`,
//! after V.9 `src/pricing/market.mjs:77-89` and `test/pricing.mjs:60-66`. The feed here is a fake `MarketFeed` that
//! reads one `code,buy,sell` line per body, so these tests need neither TT nor Super Rich parsing; they need PX-1's
//! `headline` (the core returns headline rows only). Every server is a socket on 127.0.0.1 that this file opens.
//! Written before the implementation by the ticket author (claude/mbp-xc-pricing, PX-0). Do not edit, delete or
//! weaken; add tests in a file of your own.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use xc_rate::domain::pricing::market_feed::{
    MarketAccept, MarketBody, MarketFeed, MarketFeedError, MarketRequest,
};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::market_snapshot::{MarketRow, MarketSnapshot};
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::domain::reference::reference_provider::ReferenceProvider;
use xc_rate::infrastructure::market::market_provider::MarketProvider;
use xc_rate::infrastructure::market::market_rate_source::{MARKET_USER_AGENT, MarketRateSource};

/// One answer per path: status, body, and a delay before answering.
#[derive(Clone)]
struct Answer {
    status: u16,
    body: String,
    delay: Duration,
}

fn ok(body: &str) -> Answer {
    Answer { status: 200, body: body.to_string(), delay: Duration::ZERO }
}

struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

/// `answers`: path → answer. Records every request's head (request line + headers, lower-cased).
fn serve(answers: Vec<(&'static str, Answer)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let answers = answers.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line.to_lowercase());
                }
                log.lock().unwrap().push(head.clone());
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let a = answers
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, a)| a.clone())
                    .unwrap_or(Answer { status: 404, body: String::new(), delay: Duration::ZERO });
                std::thread::sleep(a.delay);
                let mut s = stream;
                let _ = write!(
                    s,
                    "HTTP/1.1 {} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    a.status,
                    a.body.len(),
                    a.body
                );
            });
        }
    });
    Server { port, seen }
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

/// Reads `CODE,buy,sell` lines from the first body; a second, optional body is kept as `source_updated_at`.
struct LineFeed {
    urls: Vec<(String, bool)>,
}

impl MarketFeed for LineFeed {
    fn source(&self) -> MarketSource {
        MarketSource::Tt
    }

    fn requests(&self) -> Vec<MarketRequest> {
        self.urls
            .iter()
            .map(|(url, optional)| MarketRequest { url: url.clone(), accept: MarketAccept::Json, optional: *optional })
            .collect()
    }

    fn parse(&self, bodies: &[MarketBody]) -> Result<MarketSnapshot, MarketFeedError> {
        let text = bodies[0].body.clone().ok_or_else(|| MarketFeedError("TT: no body".into()))?;
        let rows: Vec<MarketRow> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let p: Vec<&str> = l.split(',').collect();
                MarketRow {
                    code: p[0].to_string(),
                    band: String::new(),
                    denoms: String::new(),
                    buy: p[1].parse().ok(),
                    sell: p[2].parse().ok(),
                    decimals: Some(2),
                }
            })
            .collect();
        if rows.is_empty() {
            return Err(MarketFeedError("TT: no rates".into()));
        }
        Ok(MarketSnapshot {
            source: MarketSource::Tt,
            source_updated_at: bodies.get(1).and_then(|b| b.body.clone()),
            rows,
        })
    }
}

fn source(feed: LineFeed, timeout: Duration) -> MarketRateSource {
    let clock = Arc::new(|| Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap());
    MarketRateSource::with(Arc::new(feed), timeout, clock)
}

#[tokio::test]
async fn a_good_answer_gives_one_row_per_currency_with_decimal_strings() {
    let srv = serve(vec![("/rates", ok("USD,33.2,33.9\nJPY,0.2085,0.2175\n")), ("/updated", ok("2026-10-17T18:17:40+07:00"))]);
    let src = source(LineFeed { urls: vec![(srv.url("/rates"), false), (srv.url("/updated"), true)] }, Duration::from_secs(5));
    let rates = src.fetch().await.expect("rates");
    assert_eq!((rates.source.as_str(), rates.kind.as_str()), ("TT", "MARKET"));
    assert_eq!(rates.fetched_at, "2026-10-18T03:00:00+00:00");
    assert_eq!(rates.rows.len(), 2);
    let usd = rates.rows.iter().find(|r| r["code"] == "USD").unwrap();
    assert_eq!(usd["buy"], json!("33.2"));
    assert_eq!(usd["sell"], json!("33.9"));
    assert_eq!(usd["source_updated_at"], json!("2026-10-17T18:17:40+07:00"));
    let head = srv.seen.lock().unwrap()[0].clone();
    assert!(head.contains(&format!("user-agent: {}", MARKET_USER_AGENT.to_lowercase())), "{head}");
    assert!(head.contains("accept: application/json"), "{head}");
}

#[tokio::test]
async fn a_503_is_an_error_naming_the_status() {
    let srv = serve(vec![("/rates", Answer { status: 503, body: "busy".into(), delay: Duration::ZERO })]);
    let src = source(LineFeed { urls: vec![(srv.url("/rates"), false)] }, Duration::from_secs(5));
    let err = src.fetch().await.unwrap_err().to_string();
    assert!(err.contains("HTTP 503"), "{err}");
}

#[tokio::test]
async fn a_source_slower_than_the_timeout_is_an_error_not_a_hang() {
    let srv = serve(vec![("/rates", Answer { status: 200, body: "USD,1,2".into(), delay: Duration::from_secs(3) })]);
    let src = source(LineFeed { urls: vec![(srv.url("/rates"), false)] }, Duration::from_millis(300));
    let started = std::time::Instant::now();
    assert!(src.fetch().await.is_err());
    assert!(started.elapsed() < Duration::from_secs(2), "gave up after {:?}", started.elapsed());
}

#[tokio::test]
async fn a_failed_optional_request_does_not_fail_the_source() {
    let srv = serve(vec![("/rates", ok("USD,33.2,33.9\n")), ("/updated", Answer { status: 500, body: String::new(), delay: Duration::ZERO })]);
    let src = source(LineFeed { urls: vec![(srv.url("/rates"), false), (srv.url("/updated"), true)] }, Duration::from_secs(5));
    let rates = src.fetch().await.expect("rates");
    assert_eq!(rates.rows[0]["source_updated_at"], json!(null));
}

#[tokio::test]
async fn a_parse_error_is_the_feeds_text() {
    let srv = serve(vec![("/rates", ok(""))]);
    let src = source(LineFeed { urls: vec![(srv.url("/rates"), false)] }, Duration::from_secs(5));
    let err = src.fetch().await.unwrap_err().to_string();
    assert!(err.contains("TT: no rates"), "{err}");
}

fn rows() -> ReferenceRates {
    ReferenceRates {
        source: "TT".into(),
        fetched_at: "2026-10-18T03:00:00+00:00".into(),
        kind: "MARKET".into(),
        rows: vec![
            json!({"code": "USD", "band": "L", "denoms": "100-50", "buy": "33.2", "sell": "33.9", "decimals": 2, "source_updated_at": "2026-10-17T18:17:40+07:00"}),
            json!({"code": "XAU", "band": "", "denoms": "", "buy": "1", "sell": "2", "decimals": 2, "source_updated_at": "2026-10-17T18:17:40+07:00"}),
            json!({"code": "SCOT", "band": "0", "denoms": "", "buy": "40", "sell": "41", "decimals": null, "source_updated_at": "2026-10-17T18:17:40+07:00"}),
        ],
    }
}

#[test]
fn the_provider_names_the_source_and_keeps_only_known_currencies() {
    let known = HashSet::from(["USD".to_string(), "EUR".to_string()]);
    let tt = MarketProvider::new(MarketSource::Tt, known.clone());
    assert_eq!((tt.provider(), tt.reference_type()), ("TT", "MARKET_TT"));
    let sr = MarketProvider::new(MarketSource::SuperRich, known);
    assert_eq!((sr.provider(), sr.reference_type()), ("SR", "MARKET_SR"));
    assert_eq!(tt.row_key(&rows().rows[0]), "USD");

    let batch = tt.parse(&rows()).expect("batch");
    assert_eq!(batch.lines.len(), 1, "XAU and SCOT are not in `currencies`");
    let usd = &batch.lines[0];
    assert_eq!(usd.currency, "USD");
    assert_eq!(usd.unit_size, Decimal::ONE);
    assert_eq!(usd.buy, Some("33.2".parse().unwrap()));
    assert_eq!(usd.sell, Some("33.9".parse().unwrap()));
    assert_eq!(usd.raw["band"], json!("L"));
    assert_eq!(batch.source_date.to_string(), "2026-10-17");
    assert!(batch.complete);
}

#[test]
fn no_known_currency_is_an_error() {
    let p = MarketProvider::new(MarketSource::Tt, HashSet::from(["EUR".to_string()]));
    let err = p.parse(&rows()).unwrap_err().to_string();
    assert!(err.contains("TT: no known currency"), "{err}");
}
