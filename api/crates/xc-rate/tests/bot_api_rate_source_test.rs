//! Specification tests for #1066: `BotApiRateSource` (the official BoT API), `FallbackRateSource`
//! and the shared `bot_rows` (`docs/design/rate-feed-connector.md` §8.3, §8.6, §8.7).
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! One test per row of #1066's behaviour table (§9). What they pin:
//! - the API source emits **exactly the rows `BotRateSource` emits** for the same bank values
//!   (`code`, `date` = `period`, `unit` with JPY 100, the three rates as JSON numbers,
//!   `original` with the strings as received); the six rows are those of the latest `period`
//!   that has all six currencies;
//! - the request: `GET {base}/DAILY_AVG_EXG_RATE/?start_period=<Bangkok today − 14 d>&end_period=
//!   <Bangkok today>`, no `currency`, header `Authorization` = the token as is (no `Bearer`);
//! - one retry, only on timeout / 5xx; never on 4xx; 401/403 log one `error` carrying
//!   `BOT_API_AUTH_REJECTED` and the status, never the token;
//! - one upstream answer (success or error) is kept for 15 minutes, measured on the injected
//!   `Clock` (so the tests can move it);
//! - `FallbackRateSource` answers the primary's `Ok`, else the fallback's result.
//!
//! The fixture `tests/fixtures/bot_api_daily_spec_example.json` is built from the OpenAPI spec's
//! example shape with made-up rates (not fetched data): 2026-10-09 (Fri) all six + SGD,
//! 2026-10-08 five (no CNY) + SGD, 2026-10-07 all six + SGD, newest first like the spec example.
//!
//! No test here reaches the network (except the `#[ignore]` live test): the API and the website
//! are sockets on 127.0.0.1 that this file opens. No real token appears here.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use pos_error::AppError;
use serde_json::{Value, json};
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::infrastructure::rate::bot_rate_source::{
    BOT_SOURCE_PAGE, BOT_UNAVAILABLE, BotRateSource, Clock,
};
use xc_rate::infrastructure::reference_source::bot_api_rate_source::{
    BotApiConfig, BotApiRateSource,
};
use xc_rate::infrastructure::reference_source::bot_rows::bot_rows;
use xc_rate::infrastructure::reference_source::fallback_rate_source::FallbackRateSource;

/// A made-up token: digits with a leading zero, the shape the config trap is about.
const TOKEN: &str = "0012345-test-token-not-real";
const API_PATH: &str = "/Stat-ExchangeRate/v2";
const DAILY: &str = "/DAILY_AVG_EXG_RATE/";
const WEB_PATH: &str =
    "/content/bot/th/statistics/exchange-rate/jcr:content/root/container/statisticstable2";
const FIXTURE: &str = include_str!("fixtures/bot_api_daily_spec_example.json");
const SIX: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];
/// Long enough that a busy runner never hits it by accident.
const PATIENT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------- the bank

/// What the stand-in answers to one request.
#[derive(Clone)]
enum Answer {
    Reply {
        status: u16,
        body: String,
        delay: Duration,
    },
}

fn reply(status: u16, body: impl Into<String>) -> Answer {
    Answer::Reply {
        status,
        body: body.into(),
        delay: Duration::ZERO,
    }
}

fn slow(status: u16, body: impl Into<String>, delay: Duration) -> Answer {
    Answer::Reply {
        status,
        body: body.into(),
        delay,
    }
}

/// One request as received: `METHOD target` and the headers (names in lower case).
#[derive(Clone, Debug)]
struct Seen {
    line: String,
    target: String,
    headers: BTreeMap<String, String>,
}

/// Path prefix → the answers in order; the last one repeats.
type Script = Vec<(String, VecDeque<Answer>)>;

/// The BoT on 127.0.0.1: the API under [`API_PATH`] and the website under [`WEB_PATH`].
struct Bank {
    port: u16,
    script: Arc<Mutex<Script>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Bank {
    fn open() -> Bank {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port for the bank");
        let port = listener.local_addr().expect("local address").port();
        let script: Arc<Mutex<Script>> = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (for_script, for_seen) = (script.clone(), seen.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (script, seen) = (for_script.clone(), for_seen.clone());
                std::thread::spawn(move || serve(stream, &script, &seen));
            }
        });
        Bank { port, script, seen }
    }

    fn api_base(&self) -> String {
        format!("http://127.0.0.1:{}{API_PATH}", self.port)
    }

    fn web_base(&self) -> String {
        format!("http://127.0.0.1:{}{WEB_PATH}", self.port)
    }

    /// The API answers `answers` in order (the last repeats).
    fn api_answers(&self, answers: Vec<Answer>) {
        self.set(&format!("{API_PATH}{DAILY}"), answers);
    }

    /// The website answers its table with `rows` (labels without windows ⇒ level 3).
    fn website_answers(&self, rows: Vec<Value>) {
        self.set(&format!("{WEB_PATH}.labels.json"), vec![reply(200, "{}")]);
        let table = json!({ "responseContent": rows }).to_string();
        self.set(
            &format!("{WEB_PATH}.results.level3cache.json"),
            vec![reply(200, table)],
        );
    }

    fn set(&self, prefix: &str, answers: Vec<Answer>) {
        let mut script = self.script.lock().expect("script lock");
        script.retain(|(p, _)| p != prefix);
        script.push((prefix.to_string(), answers.into()));
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen lock").clone()
    }

    /// Requests that reached the API.
    fn api_calls(&self) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|s| s.target.starts_with(API_PATH))
            .collect()
    }

    fn api_call_count(&self) -> usize {
        self.api_calls().len()
    }

    fn website_call_count(&self) -> usize {
        self.seen()
            .into_iter()
            .filter(|s| s.target.starts_with(WEB_PATH))
            .count()
    }
}

fn serve(mut stream: TcpStream, script: &Mutex<Script>, seen: &Mutex<Vec<Seen>>) {
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
    let line = lines.next().unwrap_or("").to_string();
    let target = line.split(' ').nth(1).unwrap_or("").to_string();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    seen.lock().expect("seen lock").push(Seen {
        line,
        target: target.clone(),
        headers,
    });
    let answer = {
        let mut script = script.lock().expect("script lock");
        let path = target.split('?').next().unwrap_or("");
        match script.iter_mut().find(|(p, _)| path == p.as_str()) {
            Some((_, queue)) if queue.len() > 1 => queue.pop_front(),
            Some((_, queue)) => queue.front().cloned(),
            None => None,
        }
        .unwrap_or_else(|| reply(404, "Not Found"))
    };
    let Answer::Reply {
        status,
        body,
        delay,
    } = answer;
    std::thread::sleep(delay);
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 {status} Answer\r\nContent-Type: application/json;charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    );
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

// ---------------------------------------------------------------- the data

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("the fixture is JSON")
}

fn detail() -> Vec<Value> {
    fixture()["result"]["data"]["data_detail"]
        .as_array()
        .expect("data_detail")
        .clone()
}

/// The fixture with `data_detail` replaced.
fn envelope(detail: Vec<Value>) -> String {
    let mut doc = fixture();
    doc["result"]["data"]["data_detail"] = Value::Array(detail);
    doc.to_string()
}

fn of_period(period: &str) -> Vec<Value> {
    detail()
        .into_iter()
        .filter(|r| r["period"] == json!(period))
        .collect()
}

fn without_period(period: &str) -> Vec<Value> {
    detail()
        .into_iter()
        .filter(|r| r["period"] != json!(period))
        .collect()
}

// ---------------------------------------------------------------- the clock

/// A clock the test moves.
#[derive(Clone)]
struct Hands(Arc<AtomicI64>);

impl Hands {
    fn at(text: &str) -> Hands {
        let t = DateTime::parse_from_rfc3339(text).expect("an instant");
        Hands(Arc::new(AtomicI64::new(t.timestamp_millis())))
    }

    fn clock(&self) -> Clock {
        let millis = self.0.clone();
        Arc::new(move || {
            Utc.timestamp_millis_opt(millis.load(Ordering::SeqCst))
                .single()
                .expect("a valid instant")
        })
    }

    fn advance(&self, by: chrono::Duration) {
        self.0.fetch_add(by.num_milliseconds(), Ordering::SeqCst);
    }

    fn now(&self) -> DateTime<Utc> {
        (self.clock())()
    }
}

/// Sunday 2026-10-11 10:00 Bangkok: the latest complete period is Friday 2026-10-09.
const SUNDAY: &str = "2026-10-11T03:00:00Z";

// ---------------------------------------------------------------- the sources

fn api(bank: &Bank, hands: &Hands, timeout: Duration) -> BotApiRateSource {
    BotApiRateSource::new(
        BotApiConfig {
            base: bank.api_base(),
            token: TOKEN.to_string(),
        },
        timeout,
        hands.clock(),
    )
}

fn website(bank: &Bank, hands: &Hands) -> BotRateSource {
    BotRateSource::new(bank.web_base(), PATIENT, hands.clock())
}

fn fallback(bank: &Bank, hands: &Hands, timeout: Duration) -> FallbackRateSource {
    FallbackRateSource::new(
        Arc::new(api(bank, hands, timeout)),
        Arc::new(website(bank, hands)),
    )
}

/// What the scraper gives for `rows` as its table (the reference the API must equal).
async fn scraper_rows(rows: Vec<Value>) -> Vec<Value> {
    let bank = Bank::open();
    bank.website_answers(rows);
    let hands = Hands::at(SUNDAY);
    website(&bank, &hands)
        .fetch()
        .await
        .expect("the scraper reads the same values")
        .rows
}

fn domain_sentence(err: &AppError) -> &str {
    match err {
        AppError::Domain(s) => s,
        other => panic!("a domain refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------- the log

/// Every event, as `(level, "field=value ...")`, while it is the thread's subscriber.
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

impl Log {
    fn events(&self) -> Vec<(tracing::Level, String)> {
        self.0.lock().expect("log lock").clone()
    }

    fn containing(&self, needle: &str) -> Vec<(tracing::Level, String)> {
        self.events()
            .into_iter()
            .filter(|(_, t)| t.contains(needle))
            .collect()
    }
}

// =============================================================== the rows

/// Table row 1: token present, 200, a period with all six ⇒ exactly the scraper's rows for the
/// same values (Friday's six, SGD dropped, JPY per 100).
#[tokio::test]
async fn a_200_answer_gives_exactly_the_rows_the_scraper_gives_for_the_same_values() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, FIXTURE)]);
    let hands = Hands::at(SUNDAY);

    let got = api(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect("the API's rows");

    assert_eq!(bank.api_call_count(), 1, "one call");
    assert_eq!(got.rows, scraper_rows(of_period("2026-10-09")).await);
    assert_eq!(got.rows.len(), 6);
    let codes: Vec<&str> = got
        .rows
        .iter()
        .map(|r| r["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, SIX, "the six, in the answer's order, SGD dropped");
    assert_eq!(
        got.source,
        bank.api_base(),
        "source = the API base, never the token"
    );
    assert_eq!(got.kind, "DAILY_COMMERCIAL_BANK_AVERAGE");
    assert_eq!(
        got.fetched_at,
        hands.now().to_rfc3339_opts(SecondsFormat::Millis, true)
    );
}

/// The row shape spelled out: seven-place decimal strings become exact JSON numbers, `original`
/// keeps the strings, JPY (`"JAPAN : YEN (100 YEN) (JPY)"`) has unit 100 and its per-100 values.
#[tokio::test]
async fn rows_carry_the_decimal_strings_exactly_and_the_yen_per_100() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, FIXTURE)]);
    let hands = Hands::at(SUNDAY);

    let got = api(&bank, &hands, PATIENT).fetch().await.expect("rows");
    let row = |code: &str| -> Value {
        got.rows
            .iter()
            .find(|r| r["code"] == json!(code))
            .unwrap_or_else(|| panic!("a {code} row in {:?}", got.rows))
            .clone()
    };

    let usd = row("USD");
    assert_eq!(usd["date"], json!("2026-10-09"));
    assert_eq!(usd["unit"], json!(1));
    assert_eq!(usd["buyingSight"].to_string(), "32.1012345");
    assert_eq!(usd["buyingTransfer"].to_string(), "32.2023456");
    assert_eq!(usd["selling"].to_string(), "32.5034567");
    assert_eq!(
        usd["original"],
        json!({ "buyingSight": "32.1012345", "buyingTransfer": "32.2023456", "selling": "32.5034567" })
    );

    let eur = row("EUR");
    assert_eq!(
        eur["buyingSight"].to_string(),
        "37.21",
        "trailing zeros dropped"
    );
    assert_eq!(eur["original"]["buyingSight"], json!("37.2100000"));

    let jpy = row("JPY");
    assert_eq!(jpy["unit"], json!(100), "the API quotes JPY per 100 yen");
    assert_eq!(
        jpy["buyingTransfer"].to_string(),
        "20.95123",
        "per 100, never divided"
    );
    assert_eq!(jpy["selling"].to_string(), "21.60123");
    assert_eq!(jpy["original"]["selling"], json!("21.6012300"));
}

/// `bot_rows` is the scraper's own mapping, shared (not copied): the same rows for the same input.
#[tokio::test]
async fn bot_rows_maps_the_table_rows_exactly_as_the_scraper_does() {
    let friday = of_period("2026-10-09");
    let rows = bot_rows(&friday).expect("seven rows with the six inside");
    assert_eq!(rows, scraper_rows(friday).await);
}

// =============================================================== the request

/// Table row 2: path, window in Bangkok dates, no `currency`, `Authorization` = token as is.
#[tokio::test]
async fn the_request_asks_for_fourteen_bangkok_days_with_the_token_as_the_authorization_header() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, FIXTURE)]);
    // 2026-10-07 18:30 UTC = 2026-10-08 01:30 in Bangkok: the Bangkok date, not the UTC one.
    let hands = Hands::at("2026-10-07T18:30:00Z");

    let _ = api(&bank, &hands, PATIENT).fetch().await;

    let calls = bank.api_calls();
    assert_eq!(calls.len(), 1, "one call: {calls:?}");
    let call = &calls[0];
    assert!(call.line.starts_with("GET "), "{}", call.line);
    let (path, query) = call.target.split_once('?').expect("a query");
    assert_eq!(path, format!("{API_PATH}{DAILY}"));
    let params: BTreeMap<&str, &str> = query
        .split('&')
        .map(|kv| kv.split_once('=').expect("key=value"))
        .collect();
    assert_eq!(
        params,
        BTreeMap::from([("end_period", "2026-10-08"), ("start_period", "2026-09-24")]),
        "Bangkok today and 14 days before; no currency"
    );
    assert_eq!(
        call.headers.get("authorization").map(String::as_str),
        Some(TOKEN),
        "the token exactly, no Bearer"
    );
}

// =============================================================== which day

/// Table row 3: 17:59 Bangkok on a business day, today not released ⇒ yesterday's rows.
#[tokio::test]
async fn before_the_evening_release_the_previous_business_day_is_given() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, envelope(of_period("2026-10-07")))]);
    let hands = Hands::at("2026-10-08T10:59:00Z"); // Thursday 17:59 in Bangkok

    let got = api(&bank, &hands, PATIENT).fetch().await.expect("rows");
    assert!(got.rows.iter().all(|r| r["date"] == json!("2026-10-07")));
    assert_eq!(got.rows, scraper_rows(of_period("2026-10-07")).await);
}

/// Table row 4: on a Sunday the latest rows are Friday's.
#[tokio::test]
async fn on_a_sunday_fridays_rows_are_given() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, FIXTURE)]);
    let hands = Hands::at(SUNDAY);

    let got = api(&bank, &hands, PATIENT).fetch().await.expect("rows");
    assert!(
        got.rows.iter().all(|r| r["date"] == json!("2026-10-09")),
        "{:?}",
        got.rows
    );
}

/// Table row 5: the latest period has 5 of 6 ⇒ the older complete one.
#[tokio::test]
async fn a_latest_period_missing_a_currency_gives_the_older_complete_period() {
    let bank = Bank::open();
    // Latest is 2026-10-08 (no CNY); 2026-10-07 is complete.
    bank.api_answers(vec![reply(200, envelope(without_period("2026-10-09")))]);
    let hands = Hands::at(SUNDAY);

    let got = api(&bank, &hands, PATIENT).fetch().await.expect("rows");
    assert_eq!(got.rows, scraper_rows(of_period("2026-10-07")).await);
}

/// Table row 6: `data_detail` empty ⇒ `Err` from the API; through the fallback, the scraper's rows.
#[tokio::test]
async fn an_empty_detail_is_an_error_and_the_fallback_answers_with_the_scraper() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, envelope(Vec::new()))]);
    bank.website_answers(of_period("2026-10-07"));
    let hands = Hands::at(SUNDAY);

    let err = api(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect_err("no rows");
    assert_eq!(domain_sentence(&err), BOT_UNAVAILABLE);
    assert_eq!(bank.api_call_count(), 1, "read the answer once");

    let got = fallback(&bank, &Hands::at(SUNDAY), PATIENT)
        .fetch()
        .await
        .expect("the scraper's rows");
    assert_eq!(got.source, BOT_SOURCE_PAGE, "the scraper answered");
    assert_eq!(got.rows, scraper_rows(of_period("2026-10-07")).await);
}

// =============================================================== statuses

async fn auth_rejected(status: u16) {
    let bank = Bank::open();
    bank.api_answers(vec![reply(status, r#"{"message":"rejected"}"#)]);
    bank.website_answers(of_period("2026-10-07"));
    let hands = Hands::at(SUNDAY);
    let log = Log::default();
    let _guard = tracing::dispatcher::set_default(&tracing::Dispatch::new(log.clone()));

    let err = api(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect_err("rejected");
    assert_eq!(domain_sentence(&err), BOT_UNAVAILABLE);
    assert!(!err.to_string().contains(TOKEN), "no token in the error");
    assert_eq!(bank.api_call_count(), 1, "{status}: no retry");

    let rejected = log.containing("BOT_API_AUTH_REJECTED");
    assert_eq!(
        rejected.len(),
        1,
        "{status}: one log line: {:?}",
        log.events()
    );
    assert_eq!(rejected[0].0, tracing::Level::ERROR, "an error, the alert");
    assert!(
        rejected[0].1.contains(&status.to_string()),
        "with the status"
    );
    assert!(
        log.events().iter().all(|(_, t)| !t.contains(TOKEN)),
        "never the token in the log"
    );

    let got = fallback(&bank, &Hands::at(SUNDAY), PATIENT)
        .fetch()
        .await
        .expect("the scraper answers");
    assert_eq!(got.source, BOT_SOURCE_PAGE);
    assert_eq!(
        bank.api_call_count(),
        2,
        "{status}: the fallback's API call, no retry"
    );
}

/// Table row 7: HTTP 401.
#[tokio::test]
async fn a_401_is_one_call_one_error_log_and_the_scraper_answers() {
    auth_rejected(401).await;
}

/// Table row 7: HTTP 403.
#[tokio::test]
async fn a_403_is_one_call_one_error_log_and_the_scraper_answers() {
    auth_rejected(403).await;
}

/// Table row 8: 500 then 200 ⇒ one retry, then rows; 2 calls in total.
#[tokio::test]
async fn a_500_then_200_retries_once_and_gives_the_rows() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(500, "oops"), reply(200, FIXTURE)]);
    let hands = Hands::at(SUNDAY);

    let got = api(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect("rows after one retry");
    assert_eq!(bank.api_call_count(), 2);
    assert_eq!(got.rows, scraper_rows(of_period("2026-10-09")).await);
}

/// Table row 9: 500 twice ⇒ `Err` after exactly 2 calls; the fallback answers.
#[tokio::test]
async fn a_500_twice_fails_after_exactly_two_calls_and_the_scraper_answers() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(500, "oops")]);
    bank.website_answers(of_period("2026-10-07"));
    let hands = Hands::at(SUNDAY);

    let err = api(&bank, &hands, PATIENT).fetch().await.expect_err("down");
    assert_eq!(domain_sentence(&err), BOT_UNAVAILABLE);
    assert_eq!(bank.api_call_count(), 2, "one call and exactly one retry");

    let got = fallback(&bank, &Hands::at(SUNDAY), PATIENT)
        .fetch()
        .await
        .expect("the scraper answers");
    assert_eq!(got.source, BOT_SOURCE_PAGE);
    assert_eq!(bank.api_call_count(), 4, "again one call and one retry");
}

/// Timeouts count like 5xx: one retry, then the fallback.
#[tokio::test]
async fn a_timeout_twice_fails_after_exactly_two_calls_and_the_scraper_answers() {
    let bank = Bank::open();
    bank.api_answers(vec![slow(200, FIXTURE, Duration::from_millis(2500))]);
    bank.website_answers(of_period("2026-10-07"));
    let hands = Hands::at(SUNDAY);

    let got = fallback(&bank, &hands, Duration::from_millis(300))
        .fetch()
        .await
        .expect("the scraper answers");
    assert_eq!(got.source, BOT_SOURCE_PAGE);
    assert_eq!(bank.api_call_count(), 2, "one call and exactly one retry");
}

/// Table row 10: 429 ⇒ `Err`, no retry.
#[tokio::test]
async fn a_429_is_an_error_without_a_retry() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(429, "slow down")]);
    let hands = Hands::at(SUNDAY);

    let err = api(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect_err("limited");
    assert_eq!(domain_sentence(&err), BOT_UNAVAILABLE);
    assert_eq!(bank.api_call_count(), 1, "never retried on 4xx");
}

// =============================================================== the budget

/// Table row 11: two fetches 1 minute apart ⇒ 1 upstream call; at 16 minutes ⇒ 2.
#[tokio::test]
async fn an_answer_is_kept_fifteen_minutes() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, FIXTURE)]);
    let hands = Hands::at(SUNDAY);
    let source = api(&bank, &hands, PATIENT);

    let first = source.fetch().await.expect("rows");
    hands.advance(chrono::Duration::minutes(1));
    let second = source.fetch().await.expect("the kept rows");
    assert_eq!(
        bank.api_call_count(),
        1,
        "one upstream call inside 15 minutes"
    );
    assert_eq!(first.rows, second.rows);

    hands.advance(chrono::Duration::minutes(15)); // 16 minutes after the first
    source.fetch().await.expect("fresh rows");
    assert_eq!(bank.api_call_count(), 2, "a new call after 15 minutes");
}

/// An error answer is kept too (design §8.6: "success or error"), so a down API is not hammered.
#[tokio::test]
async fn an_error_answer_is_kept_fifteen_minutes_too() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(500, "oops")]);
    let hands = Hands::at(SUNDAY);
    let source = api(&bank, &hands, PATIENT);

    source.fetch().await.expect_err("down");
    assert_eq!(bank.api_call_count(), 2, "one call and one retry");
    hands.advance(chrono::Duration::minutes(1));
    source.fetch().await.expect_err("the kept error");
    assert_eq!(bank.api_call_count(), 2, "no call inside 15 minutes");
}

// =============================================================== bad values

async fn a_bad_rate(text: &str) {
    let mut rows = detail();
    let usd = rows
        .iter_mut()
        .find(|r| r["period"] == json!("2026-10-09") && r["currency_id"] == json!("USD"))
        .expect("Friday's USD");
    usd["selling"] = json!(text);
    let bank = Bank::open();
    bank.api_answers(vec![reply(200, envelope(rows))]);
    let hands = Hands::at(SUNDAY);

    let err = api(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect_err("a bad rate fails the fetch");
    assert_eq!(domain_sentence(&err), BOT_UNAVAILABLE);
    assert_eq!(bank.api_call_count(), 1, "the answer was read, not retried");
}

/// Table row 12: a rate of zero fails, as the scraper does.
#[tokio::test]
async fn a_zero_rate_fails_the_fetch() {
    a_bad_rate("0.0000000").await;
}

/// Table row 12: a rate that is not a number fails, as the scraper does.
#[tokio::test]
async fn a_rate_that_is_not_a_number_fails_the_fetch() {
    a_bad_rate("abc").await;
}

// =============================================================== the token

/// Table row 15: `Debug` never prints the token.
#[test]
fn the_config_debug_never_prints_the_token() {
    let config = BotApiConfig {
        base: "https://gateway.api.bot.or.th/Stat-ExchangeRate/v2".to_string(),
        token: TOKEN.to_string(),
    };
    let text = format!("{config:?}");
    assert!(!text.contains(TOKEN), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
    assert!(
        text.contains("gateway.api.bot.or.th"),
        "the base is not secret: {text}"
    );
}

// =============================================================== the fallback

struct Fixed {
    answer: Result<ReferenceRates, String>,
    asked: AtomicI64,
}

impl Fixed {
    fn ok(source: &str) -> Arc<Fixed> {
        Arc::new(Fixed {
            answer: Ok(ReferenceRates {
                source: source.to_string(),
                fetched_at: "2026-10-11T03:00:00.000Z".to_string(),
                kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
                rows: vec![json!({ "code": "USD" })],
            }),
            asked: AtomicI64::new(0),
        })
    }

    fn err(sentence: &str) -> Arc<Fixed> {
        Arc::new(Fixed {
            answer: Err(sentence.to_string()),
            asked: AtomicI64::new(0),
        })
    }

    fn asked(&self) -> i64 {
        self.asked.load(Ordering::SeqCst)
    }
}

impl RateSource for Fixed {
    fn fetch(&self) -> futures::future::BoxFuture<'_, pos_error::AppResult<ReferenceRates>> {
        Box::pin(async move {
            self.asked.fetch_add(1, Ordering::SeqCst);
            match &self.answer {
                Ok(r) => Ok(r.clone()),
                Err(s) => Err(AppError::Domain(s.clone())),
            }
        })
    }
}

/// The primary's `Ok` is the answer; the fallback is not asked.
#[tokio::test]
async fn the_fallback_source_answers_the_primary_when_it_succeeds() {
    let (primary, spare) = (Fixed::ok("primary"), Fixed::ok("spare"));
    let got = FallbackRateSource::new(primary.clone(), spare.clone())
        .fetch()
        .await
        .expect("the primary's rows");
    assert_eq!(got.source, "primary");
    assert_eq!((primary.asked(), spare.asked()), (1, 0));
}

/// On the primary's `Err`, the fallback's result, whatever it is.
#[tokio::test]
async fn the_fallback_source_answers_the_fallback_when_the_primary_fails() {
    let (primary, spare) = (Fixed::err("api down"), Fixed::ok("spare"));
    let got = FallbackRateSource::new(primary.clone(), spare.clone())
        .fetch()
        .await
        .expect("the fallback's rows");
    assert_eq!(got.source, "spare");
    assert_eq!((primary.asked(), spare.asked()), (1, 1));

    let (primary, spare) = (Fixed::err("api down"), Fixed::err("spare down"));
    let err = FallbackRateSource::new(primary, spare)
        .fetch()
        .await
        .expect_err("both down");
    assert_eq!(domain_sentence(&err), "spare down", "the fallback's result");
}

/// Table row 16: the API and the scraper both fail ⇒ `Err(BOT_UNAVAILABLE)`.
#[tokio::test]
async fn the_api_and_the_scraper_both_failing_is_bot_unavailable() {
    let bank = Bank::open();
    bank.api_answers(vec![reply(500, "oops")]);
    // The website is not scripted: 404 for everything.
    let hands = Hands::at(SUNDAY);

    let err = fallback(&bank, &hands, PATIENT)
        .fetch()
        .await
        .expect_err("nobody answers");
    assert_eq!(domain_sentence(&err), BOT_UNAVAILABLE);
    assert_eq!(
        bank.api_call_count(),
        2,
        "the API was tried, with its one retry"
    );
    assert!(bank.website_call_count() >= 1, "then the scraper");
}

// =============================================================== live

/// The "proven" check (design §8.7): the live API and the live scraper agree for the same period,
/// and the API still names the yen "(100 YEN)". Runs only by hand with `APP_BOT__API_TOKEN` set:
/// `APP_BOT__API_TOKEN=… cargo test -p xc-rate --test bot_api_rate_source_test -- --ignored`.
#[tokio::test]
#[ignore = "live: needs APP_BOT__API_TOKEN and the network"]
async fn bot_api_live_matches_scraper() {
    let Ok(token) = std::env::var("APP_BOT__API_TOKEN") else {
        eprintln!("APP_BOT__API_TOKEN not set; nothing to compare");
        return;
    };
    let base = "https://gateway.api.bot.or.th/Stat-ExchangeRate/v2".to_string();
    let config = BotApiConfig {
        base: base.clone(),
        token: token.clone(),
    };
    let from_api = BotApiRateSource::new(config, PATIENT, Arc::new(Utc::now))
        .fetch()
        .await
        .expect("the API answers");
    let from_web = BotRateSource::bot()
        .fetch()
        .await
        .expect("the website answers");

    let api_date = from_api.rows[0]["date"].clone();
    if from_web.rows.iter().all(|r| r["date"] == api_date) {
        for code in SIX {
            let pick = |rows: &[Value]| -> Value {
                let r = rows.iter().find(|r| r["code"] == json!(code)).expect(code);
                json!([
                    r["unit"],
                    r["buyingSight"],
                    r["buyingTransfer"],
                    r["selling"]
                ])
            };
            assert_eq!(
                pick(&from_api.rows),
                pick(&from_web.rows),
                "{code} on {api_date}"
            );
        }
    } else {
        eprintln!("the two sources are on different periods now; compare again later");
    }

    let today = Utc::now().date_naive();
    let url = format!(
        "{base}/DAILY_AVG_EXG_RATE/?start_period={}&end_period={today}&currency=JPY",
        today - chrono::Duration::days(14)
    );
    let bytes = reqwest::Client::new()
        .get(url)
        .header("Authorization", token)
        .send()
        .await
        .expect("the API answers")
        .bytes()
        .await
        .expect("a body");
    let body: Value = serde_json::from_slice(&bytes).expect("JSON");
    let names: Vec<&str> = body["result"]["data"]["data_detail"]
        .as_array()
        .expect("data_detail")
        .iter()
        .filter_map(|r| r["currency_name_eng"].as_str())
        .collect();
    assert!(!names.is_empty(), "JPY rows in the last 14 days");
    assert!(
        names.iter().all(|n| n.contains("(100 YEN)")),
        "BoT still quotes the yen per 100: {names:?}"
    );
}
