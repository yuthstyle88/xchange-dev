//! Specification tests for #1066: the BoT API settings and the choice of `AppState.rate_source`
//! (`docs/design/rate-feed-connector.md` §8.4, §8.5).
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! - `APP_BOT__API_TOKEN` is taken raw: `.try_parsing(true)` (`config/mod.rs:181`) must not turn
//!   `0012345` into the number 12345. Empty or blank means absent.
//! - `APP_BOT__API_BASE` defaults to `https://gateway.api.bot.or.th/Stat-ExchangeRate/v2`.
//! - Token present ⇒ the source `AppState::new` takes from `bootstrap::choose_rate_source` calls
//!   the API (base from settings, token as the `Authorization` header). Token absent ⇒ the scraper,
//!   one `warn` naming the variable, and the API is never called.
//! - The token never shows in `Debug` output of the settings.
//!
//! The choice is tested through `choose_rate_source(&settings.bot, scraper)`, the function
//! `AppState::new` calls with `BotRateSource::bot()`, so the scraper here is a local stand-in:
//! no test in this file reaches bot.or.th. The API and the website are one socket on 127.0.0.1.
//! No real token.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{FixedOffset, Utc};
use common::{config_dir, env, settings};
use serde_json::{Value, json};
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::{BOT_SOURCE_PAGE, BotRateSource};
use xchange::bootstrap::choose_rate_source;
use xchange::config::Settings;

const DEFAULT_BASE: &str = "https://gateway.api.bot.or.th/Stat-ExchangeRate/v2";
/// Made-up; digits with leading zeros, the shape the type-parse trap is about.
const DIGITS_TOKEN: &str = "0012345";
const TOKEN: &str = "0099887766-test-token-not-real";

fn load(pairs: &[(&str, &str)]) -> Settings {
    Settings::load_from(&config_dir(), env(pairs)).expect("the configuration loads")
}

// ---------------------------------------------------------------- settings

#[test]
fn an_all_digit_token_with_leading_zeros_stays_the_same_text() {
    let s = load(&[("APP_BOT__API_TOKEN", DIGITS_TOKEN)]);
    assert_eq!(s.bot.api_token.as_deref(), Some(DIGITS_TOKEN));
}

#[test]
fn an_empty_or_blank_token_is_absent() {
    assert_eq!(load(&[]).bot.api_token, None, "not set");
    assert_eq!(
        load(&[("APP_BOT__API_TOKEN", "")]).bot.api_token,
        None,
        "empty"
    );
    assert_eq!(
        load(&[("APP_BOT__API_TOKEN", "   ")]).bot.api_token,
        None,
        "blank"
    );
}

#[test]
fn the_api_base_defaults_to_the_bot_gateway_and_can_be_overridden() {
    assert_eq!(load(&[]).bot.api_base, DEFAULT_BASE);
    assert_eq!(
        load(&[("APP_BOT__API_TOKEN", TOKEN)]).bot.api_base,
        DEFAULT_BASE,
        "a token alone keeps the default base"
    );
    let s = load(&[("APP_BOT__API_BASE", "http://127.0.0.1:9/v2")]);
    assert_eq!(s.bot.api_base, "http://127.0.0.1:9/v2");
}

#[test]
fn the_settings_debug_never_prints_the_token() {
    let s = load(&[("APP_BOT__API_TOKEN", TOKEN)]);
    let text = format!("{s:?}");
    assert!(!text.contains(TOKEN), "the token in Debug output");
}

// ---------------------------------------------------------------- the API stand-in

struct Api {
    port: u16,
    seen: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
}

impl Api {
    /// Answers the API with `body` and the website (under `/web`) with the same six rows as its
    /// table, recording every request's headers (plus `:target`).
    fn open(body: String) -> Api {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
        let port = listener.local_addr().expect("local address").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let for_seen = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (seen, body) = (for_seen.clone(), body.clone());
                std::thread::spawn(move || serve(stream, &seen, &body));
            }
        });
        Api { port, seen }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}/Stat-ExchangeRate/v2", self.port)
    }

    fn web_base(&self) -> String {
        format!("http://127.0.0.1:{}/web/statisticstable2", self.port)
    }

    /// Requests that reached the API.
    fn calls(&self) -> Vec<BTreeMap<String, String>> {
        self.hits("/Stat-ExchangeRate/")
    }

    /// Requests that reached the website stand-in.
    fn web_calls(&self) -> Vec<BTreeMap<String, String>> {
        self.hits("/web/")
    }

    fn hits(&self, prefix: &str) -> Vec<BTreeMap<String, String>> {
        self.seen
            .lock()
            .expect("seen lock")
            .iter()
            .filter(|h| h[":target"].starts_with(prefix))
            .cloned()
            .collect()
    }
}

fn serve(mut stream: TcpStream, seen: &Mutex<Vec<BTreeMap<String, String>>>, body: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.split("\r\n");
    let target = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or("")
        .to_string();
    let mut headers: BTreeMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    headers.insert(":target".to_string(), target.clone());
    seen.lock().expect("seen lock").push(headers);
    let body = if target.ends_with(".labels.json") {
        "{}".to_string()
    } else if target.ends_with(".results.level3cache.json") {
        let api: Value = serde_json::from_str(body).expect("the API body is JSON");
        json!({ "responseContent": api["result"]["data"]["data_detail"] }).to_string()
    } else {
        body.to_string()
    };
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
}

/// An answer in the spec's shape whose newest complete period is yesterday in Bangkok (so it
/// lies inside any 14-day window the real clock gives). Made-up rates.
fn yesterdays_six() -> String {
    let bangkok = FixedOffset::east_opt(7 * 3600).expect("UTC+7");
    let yesterday = (Utc::now().with_timezone(&bangkok).date_naive() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let row = |code: &str, name: &str, s: &str, t: &str, l: &str| {
        json!({ "period": yesterday, "currency_id": code, "currency_name_th": name,
                "currency_name_eng": name, "buying_sight": s, "buying_transfer": t,
                "selling": l, "mid_rate": t })
    };
    json!({ "result": { "api": "Daily Average Exchange Rate - THB / Foreign Currency",
        "timestamp": "2026-10-09 18:05:00",
        "data": { "data_header": { "last_updated": yesterday }, "data_detail": [
            row("USD", "USA : DOLLAR (USD) ", "31.1000000", "31.2000000", "31.5000000"),
            row("EUR", "EURO ZONE : EURO (EUR)", "36.1000000", "36.2000000", "36.9000000"),
            row("GBP", "UNITED KINGDOM : POUND STERLING (GBP)", "41.1000000", "41.2000000", "41.9000000"),
            row("JPY", "JAPAN : YEN (100 YEN) (JPY)", "20.1000000", "20.2000000", "20.9000000"),
            row("CNY", "CHINA : YUAN RENMINBI (CNY)", "4.1000000", "4.2000000", "4.4000000"),
            row("AUD", "AUSTRALIA : DOLLAR (AUD)", "20.5000000", "20.6000000", "21.3000000"),
        ] } } })
    .to_string()
}

// ---------------------------------------------------------------- the log

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<(tracing::Level, String)>>>);

struct Text(String);

impl tracing::field::Visit for Text {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

impl tracing::Subscriber for Log {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = Text(String::new());
        event.record(&mut text);
        self.0
            .lock()
            .expect("log lock")
            .push((*event.metadata().level(), text.0));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

// ---------------------------------------------------------------- the choice

/// The scraper as `AppState::new` passes it, but pointed at the local stand-in.
fn local_scraper(api: &Api) -> Arc<dyn RateSource> {
    Arc::new(BotRateSource::new(
        api.web_base(),
        Duration::from_secs(5),
        Arc::new(Utc::now),
    ))
}

/// Token present ⇒ the state's source calls the API with that token and gives its rows.
#[tokio::test]
async fn with_a_token_the_state_reads_the_api_with_it() {
    let api = Api::open(yesterdays_six());
    let settings = settings(&[
        ("APP_BOT__API_TOKEN", DIGITS_TOKEN),
        ("APP_BOT__API_BASE", &api.base()),
    ]);
    let source = choose_rate_source(&settings.bot, local_scraper(&api));

    let got = tokio::time::timeout(Duration::from_secs(5), source.fetch()).await;

    let calls = api.calls();
    assert_eq!(calls.len(), 1, "one API call: {calls:?}");
    assert!(
        calls[0][":target"].starts_with("/Stat-ExchangeRate/v2/DAILY_AVG_EXG_RATE/"),
        "{calls:?}"
    );
    assert_eq!(
        calls[0].get("authorization").map(String::as_str),
        Some(DIGITS_TOKEN),
        "the raw token, leading zeros kept"
    );
    let rates = got.expect("answers inside 5 s").expect("the API's rows");
    assert_eq!(
        rates.source,
        api.base(),
        "the API answered, not the scraper"
    );
    let usd: &Value = rates
        .rows
        .iter()
        .find(|r| r["code"] == json!("USD"))
        .expect("a USD row");
    assert_eq!(usd["buyingTransfer"].to_string(), "31.2");
    assert!(
        api.web_calls().is_empty(),
        "the scraper is not asked when the API answers"
    );
}

/// Token absent ⇒ one `warn` at start naming the variable, and the API is never called.
#[tokio::test]
async fn without_a_token_the_state_warns_once_and_never_calls_the_api() {
    let api = Api::open(yesterdays_six());
    let log = Log::default();
    let settings = settings(&[
        ("APP_BOT__API_TOKEN", ""),
        ("APP_BOT__API_BASE", &api.base()),
    ]);
    let source = {
        let _guard = tracing::dispatcher::set_default(&tracing::Dispatch::new(log.clone()));
        choose_rate_source(&settings.bot, local_scraper(&api))
    };

    let warned: Vec<_> = log
        .0
        .lock()
        .expect("log lock")
        .iter()
        .filter(|(level, text)| {
            *level == tracing::Level::WARN && text.contains("APP_BOT__API_TOKEN")
        })
        .cloned()
        .collect();
    assert_eq!(warned.len(), 1, "one warn naming the variable: {warned:?}");

    let got = tokio::time::timeout(Duration::from_secs(5), source.fetch())
        .await
        .expect("answers inside 5 s")
        .expect("the scraper's rows");
    assert!(api.calls().is_empty(), "no API call without a token");
    assert!(!api.web_calls().is_empty(), "the scraper answered");
    assert_eq!(got.source, BOT_SOURCE_PAGE);
}
