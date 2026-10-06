//! Specification tests for T5b: `BotRateSource`, the Bank of Thailand
//! reference-rate fetch behind the `RateSource` port.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! What the source does, in v2's words turned into tests:
//! - two `GET`s per call, never fewer and never kept: `<base>.labels.json`, then
//!   exactly one of `<base>.results.level{1,2,3}cache.json`, chosen by the two
//!   time windows the labels carry and the clock;
//! - of the bank's table it keeps six currencies (USD, EUR, GBP, JPY, CNY, AUD)
//!   in the bank's order, the yen divided by 100, each row with the bank's own
//!   text under `original`;
//! - anything else — no answer, a status that is not 2xx, a body that is not the
//!   table, a currency missing or twice, a rate that is not a positive number, a
//!   date that is not `YYYY-MM-DD` — is one refusal, `AppError::Domain` with one
//!   sentence.
//!
//! `tests/fixtures/v2_bot_source_cases.json` was recorded by running v2's own
//! fetch against scripted answers of the bank (`api/tools/v2-fixtures/
//! gen_reference_rates.mjs source`). v3 differs from v2 in one place, on purpose:
//! v2 divides the yen as a binary float (`20.9382 / 100` is
//! `0.20938199999999998` there); v3 computes in decimal and answers `0.209382`.
//! The fixture's `answer` holds every rate as the exact decimal text the JSON
//! number must print as.
//!
//! No test here reaches the network or needs a database: the bank is a socket
//! on 127.0.0.1 that this file opens.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use pos_error::AppError;
use serde::Deserialize;
use serde_json::Value;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_rate::infrastructure::rate::bot_rate_source::{
    BOT_BASE, BOT_SOURCE_PAGE, BOT_TIMEOUT, BOT_UNAVAILABLE, BotRateSource, Clock,
};

/// The one sentence v2 has for "the bank gave nothing usable".
const SENTENCE: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";
/// The path of the bank's table, as v2 asks for it (the `:` is part of it).
const BASE_PATH: &str =
    "/content/bot/th/statistics/exchange-rate/jcr:content/root/container/statisticstable2";
const LABELS: &str = ".labels.json";
const LEVEL_1: &str = ".results.level1cache.json";
const LEVEL_2: &str = ".results.level2cache.json";
const LEVEL_3: &str = ".results.level3cache.json";
/// Long enough that a busy runner never hits it by accident.
const PATIENT: Duration = Duration::from_secs(20);
/// Upper bound for a call that must give up.
const MUST_FINISH: Duration = Duration::from_secs(15);

// ------------------------------------------------------------------ the bank

/// What the stand-in bank does with one path.
#[derive(Clone)]
enum Answer {
    /// Status and body, after waiting `delay`.
    Reply {
        status: u16,
        body: String,
        delay: Duration,
    },
    /// Read the request, then close the connection without a byte.
    Hangup,
    /// Promise more bytes than are sent, then close.
    CutShort,
}

fn reply(status: u16, body: impl Into<String>) -> Answer {
    Answer::Reply {
        status,
        body: body.into(),
        delay: Duration::ZERO,
    }
}

fn slow(body: impl Into<String>, delay: Duration) -> Answer {
    Answer::Reply {
        status: 200,
        body: body.into(),
        delay,
    }
}

type Script = BTreeMap<String, Answer>;

/// A bank on 127.0.0.1. It answers by path suffix, 404 for a path it was not
/// told about, and writes down every request line it received.
struct Bank {
    port: u16,
    script: Arc<Mutex<Script>>,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Bank {
    fn open(script: Script) -> Bank {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port for the bank");
        let port = listener.local_addr().expect("local address").port();
        let script = Arc::new(Mutex::new(script));
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

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}{BASE_PATH}", self.port)
    }

    /// Replace what the bank answers from now on.
    fn now_answers(&self, script: Script) {
        *self.script.lock().expect("script lock") = script;
    }

    /// Every request so far, as `METHOD path`.
    fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("seen lock").clone()
    }
}

fn serve(mut stream: TcpStream, script: &Mutex<Script>, seen: &Mutex<Vec<String>>) {
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
    let mut words = head.lines().next().unwrap_or("").split(' ');
    let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    seen.lock()
        .expect("seen lock")
        .push(format!("{method} {target}"));
    let answer = {
        let script = script.lock().expect("script lock");
        target
            .strip_prefix(BASE_PATH)
            .and_then(|suffix| script.get(suffix).cloned())
            .unwrap_or_else(|| reply(404, "Not Found"))
    };
    match answer {
        Answer::Hangup => {}
        Answer::CutShort => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json;charset=utf-8\r\n\
                  Content-Length: 5000\r\nConnection: close\r\n\r\n{\"responseContent\":[",
            );
        }
        Answer::Reply {
            status,
            body,
            delay,
        } => {
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
        }
    }
    let _ = stream.flush();
}

// ----------------------------------------------------------------- the clock

/// A clock the test moves, to the millisecond. `fine` is added to every
/// reading: a wall clock has microseconds, and they must not reach the answer.
#[derive(Clone)]
struct Hands {
    millis: Arc<AtomicI64>,
    fine: chrono::Duration,
}

impl Hands {
    fn at(text: &str) -> Hands {
        Hands {
            millis: Arc::new(AtomicI64::new(millis_of(text))),
            fine: chrono::Duration::zero(),
        }
    }

    fn with_microseconds(mut self, microseconds: i64) -> Hands {
        self.fine = chrono::Duration::microseconds(microseconds);
        self
    }

    fn set(&self, text: &str) {
        self.millis.store(millis_of(text), Ordering::SeqCst);
    }

    fn clock(&self) -> Clock {
        let (millis, fine) = (self.millis.clone(), self.fine);
        Arc::new(move || {
            Utc.timestamp_millis_opt(millis.load(Ordering::SeqCst))
                .single()
                .expect("a valid instant")
                + fine
        })
    }
}

fn millis_of(text: &str) -> i64 {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .timestamp_millis()
}

// --------------------------------------------------------------- the fixture

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    base: String,
    labels_suffix: String,
    level_suffixes: Vec<String>,
    source_page: String,
    kind: String,
    timeout_ms: u64,
    sentence: String,
    bodies: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    now: String,
    upstream: BTreeMap<String, Upstream>,
    requested: Vec<String>,
    v2_error: Option<String>,
    v2_body: Option<String>,
    differs_from_v2: Option<bool>,
    answer: Option<Expected>,
}

#[derive(Deserialize)]
struct Upstream {
    #[serde(default)]
    fail: bool,
    status: Option<u16>,
    body: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expected {
    source: String,
    fetched_at: String,
    kind: String,
    rows: Vec<ExpectedRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedRow {
    code: String,
    date: String,
    unit: u64,
    buying_sight: String,
    buying_transfer: String,
    selling: String,
    original: Value,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/v2_bot_source_cases.json"))
        .expect("tests/fixtures/v2_bot_source_cases.json parses")
}

fn script_of(case: &Case, bodies: &[String]) -> Script {
    case.upstream
        .iter()
        .map(|(suffix, u)| {
            let answer = if u.fail {
                Answer::Hangup
            } else {
                reply(
                    u.status.expect("a status"),
                    bodies[u.body.expect("a body")].clone(),
                )
            };
            (suffix.clone(), answer)
        })
        .collect()
}

/// Everything in which `got` is not the answer the fixture wants.
fn differences(got: &ReferenceRates, want: &Expected) -> Vec<String> {
    let mut wrong = Vec::new();
    let mut check = |what: String, got: String, want: String| {
        if got != want {
            wrong.push(format!("{what}: got {got}, want {want}"));
        }
    };
    check("source".into(), got.source.clone(), want.source.clone());
    check(
        "fetched_at".into(),
        got.fetched_at.clone(),
        want.fetched_at.clone(),
    );
    check("kind".into(), got.kind.clone(), want.kind.clone());
    check(
        "number of rows".into(),
        got.rows.len().to_string(),
        want.rows.len().to_string(),
    );
    for (i, (row, w)) in got.rows.iter().zip(&want.rows).enumerate() {
        let Some(object) = row.as_object() else {
            check(format!("row {i}"), row.to_string(), "an object".into());
            continue;
        };
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        check(
            format!("row {i} keys"),
            keys.join(","),
            "buyingSight,buyingTransfer,code,date,original,selling,unit".into(),
        );
        // `Value`'s own text: a string keeps its quotes, a number prints as JSON
        let text = |key: &str| row.get(key).map(Value::to_string).unwrap_or_default();
        check(
            format!("row {i} code"),
            text("code"),
            format!("{:?}", w.code),
        );
        check(
            format!("row {i} date"),
            text("date"),
            format!("{:?}", w.date),
        );
        check(format!("row {i} unit"), text("unit"), w.unit.to_string());
        check(
            format!("row {i} ({}) buyingSight", w.code),
            text("buyingSight"),
            w.buying_sight.clone(),
        );
        check(
            format!("row {i} ({}) buyingTransfer", w.code),
            text("buyingTransfer"),
            w.buying_transfer.clone(),
        );
        check(
            format!("row {i} ({}) selling", w.code),
            text("selling"),
            w.selling.clone(),
        );
        check(
            format!("row {i} original"),
            text("original"),
            w.original.to_string(),
        );
    }
    wrong
}

/// The sentence of a refusal; anything else fails the test.
fn refusal(result: Result<ReferenceRates, AppError>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

// ------------------------------------------------------- the standard answers

const NOW: &str = "2026-10-06T11:20:00.000Z";

/// Labels whose second window holds [`NOW`]: the bank's level 2 is the one to read.
fn labels() -> String {
    serde_json::json!({
        "tab1Label": "x", "currentTime": "06:20 PM",
        "level1StartTime": "2026-10-06T11:01:00Z", "level1EndTime": "2026-10-06T11:15:00Z",
        "level2StartTime": "2026-10-06T11:15:00Z", "level2EndTime": "2026-10-06T11:30:00Z",
    })
    .to_string()
}

fn bank_row(code: &str, sight: &str, transfer: &str, selling: &str) -> Value {
    serde_json::json!({
        "period": "2026-10-06", "currency_id": code, "countryName": code,
        "buying_sight": sight, "buying_transfer": transfer, "selling": selling,
        "flagPath": "/x.png", "sortOrder": 1,
    })
}

/// A full table whose dollar selling rate is `usd_selling`.
fn table(usd_selling: &str) -> String {
    serde_json::json!({
        "lastUpdated": "x", "noDataFoundFlag": false,
        "responseContent": [
            bank_row("USD", "33.4218", "33.5008", usd_selling),
            bank_row("GBP", "44.0195", "44.1299", "44.9090"),
            bank_row("EUR", "37.3552", "37.4413", "38.0844"),
            bank_row("JPY", "20.9382", "21.0000", "21.6300"),
            bank_row("HKD", "4.2610", "4.2729", "4.3508"),
            bank_row("AUD", "23.0133", "23.0000", "23.7839"),
            bank_row("CNY", "4.9541", "4.9767", "5.0667"),
        ],
    })
    .to_string()
}

fn plain_script(usd_selling: &str) -> Script {
    Script::from([
        (LABELS.to_owned(), reply(200, labels())),
        (LEVEL_1.to_owned(), reply(200, table("1.0001"))),
        (LEVEL_2.to_owned(), reply(200, table(usd_selling))),
        (LEVEL_3.to_owned(), reply(200, table("3.0003"))),
    ])
}

fn source(bank: &Bank, timeout: Duration, hands: &Hands) -> BotRateSource {
    BotRateSource::new(bank.base(), timeout, hands.clock())
}

/// The JSON text of one field of the row for `code`.
fn field(rates: &ReferenceRates, code: &str, key: &str) -> String {
    rates
        .rows
        .iter()
        .find(|r| r["code"] == code)
        .unwrap_or_else(|| panic!("no row for {code}"))[key]
        .to_string()
}

// ------------------------------------------------------------------ the tests

#[test]
fn the_fixture_is_the_recording_this_file_expects() {
    let fixture = fixture();
    assert_eq!(fixture.cases.len(), 76, "cases in the fixture");
    let answered = fixture
        .cases
        .iter()
        .filter(|c| c.v2_error.is_none())
        .count();
    assert_eq!(answered, 25, "cases v2 answered");
    assert_eq!(
        fixture
            .cases
            .iter()
            .filter(|c| c.differs_from_v2 == Some(true))
            .count(),
        2,
        "cases where v2's float arithmetic shows"
    );
    for case in &fixture.cases {
        assert_eq!(
            case.answer.is_some(),
            case.v2_error.is_none(),
            "{}",
            case.name
        );
        assert_eq!(
            case.v2_body.is_some(),
            case.v2_error.is_none(),
            "{}",
            case.name
        );
        if let Some(sentence) = &case.v2_error {
            assert_eq!(sentence, &fixture.sentence, "{}", case.name);
        }
    }
    assert_eq!(fixture.sentence, SENTENCE);
    assert_eq!(fixture.labels_suffix, LABELS);
    assert_eq!(fixture.level_suffixes, [LEVEL_1, LEVEL_2, LEVEL_3]);
    assert_eq!(fixture.kind, "DAILY_COMMERCIAL_BANK_AVERAGE");
    assert!(fixture.base.ends_with(BASE_PATH), "{}", fixture.base);
}

#[test]
fn the_constants_are_v2s() {
    let fixture = fixture();
    assert_eq!(BOT_BASE, fixture.base);
    assert_eq!(
        BOT_BASE,
        "https://www.bot.or.th/content/bot/th/statistics/exchange-rate/jcr:content/root/container/statisticstable2"
    );
    assert_eq!(BOT_SOURCE_PAGE, fixture.source_page);
    assert_eq!(
        BOT_SOURCE_PAGE,
        "https://www.bot.or.th/th/statistics/exchange-rate.html"
    );
    assert_eq!(BOT_TIMEOUT, Duration::from_millis(fixture.timeout_ms));
    assert_eq!(BOT_TIMEOUT, Duration::from_secs(10));
    assert_eq!(BOT_UNAVAILABLE, fixture.sentence);
    assert_eq!(BOT_UNAVAILABLE, SENTENCE);
}

#[test]
fn the_real_source_points_at_the_bank_and_waits_ten_seconds() {
    // built, never asked: no test reaches the bank
    let real = BotRateSource::bot();
    assert_eq!(real.base(), BOT_BASE);
    assert_eq!(real.timeout(), BOT_TIMEOUT);

    let hands = Hands::at(NOW);
    let other = BotRateSource::new(
        "http://127.0.0.1:1/table",
        Duration::from_millis(250),
        hands.clock(),
    );
    assert_eq!(other.base(), "http://127.0.0.1:1/table");
    assert_eq!(other.timeout(), Duration::from_millis(250));
}

#[test]
fn it_is_a_rate_source_that_threads_can_share() {
    fn shareable<T: Send + Sync + 'static>(_: &T) {}
    let hands = Hands::at(NOW);
    let source = BotRateSource::new("http://127.0.0.1:1/table", PATIENT, hands.clock());
    shareable(&source);
    let _as_the_port: Arc<dyn RateSource> = Arc::new(source);
}

/// Every recorded case: the same requests v2 made, the same refusal or the
/// same document (with decimals where v2 has floats).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_recorded_case_is_answered_as_v2_answered_it() {
    let fixture = fixture();
    let mut wrong = Vec::new();
    for case in &fixture.cases {
        let bank = Bank::open(script_of(case, &fixture.bodies));
        let hands = Hands::at(&case.now);
        let result = source(&bank, PATIENT, &hands).fetch().await;

        let asked: Vec<String> = case
            .requested
            .iter()
            .map(|suffix| format!("GET {BASE_PATH}{suffix}"))
            .collect();
        if bank.seen() != asked {
            wrong.push(format!(
                "{}: asked the bank {:?}, v2 asked {asked:?}",
                case.name,
                bank.seen()
            ));
        }
        match (&case.answer, result) {
            (Some(want), Ok(got)) => {
                for difference in differences(&got, want) {
                    wrong.push(format!("{}: {difference}", case.name));
                }
            }
            (None, Err(AppError::Domain(sentence))) => {
                if sentence != fixture.sentence {
                    wrong.push(format!("{}: refused with {sentence:?}", case.name));
                }
            }
            (Some(_), Err(e)) => wrong.push(format!("{}: v2 answers, got {e:?}", case.name)),
            (None, Ok(got)) => wrong.push(format!("{}: v2 refuses, got {got:?}", case.name)),
            (None, Err(e)) => wrong.push(format!(
                "{}: a refusal must be AppError::Domain, got {e:?}",
                case.name
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} differences from v2:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_gets_labels_first_then_the_one_level() {
    let bank = Bank::open(plain_script("33.8278"));
    let hands = Hands::at(NOW);
    let rates = source(&bank, PATIENT, &hands)
        .fetch()
        .await
        .expect("the bank answers");
    assert_eq!(
        bank.seen(),
        [
            format!("GET {BASE_PATH}{LABELS}"),
            format!("GET {BASE_PATH}{LEVEL_2}"),
        ]
    );
    assert_eq!(field(&rates, "USD", "selling"), "33.8278");
    assert_eq!(rates.source, BOT_SOURCE_PAGE);
    assert_eq!(rates.kind, "DAILY_COMMERCIAL_BANK_AVERAGE");
    let codes: Vec<&str> = rates
        .rows
        .iter()
        .filter_map(|r| r["code"].as_str())
        .collect();
    assert_eq!(codes, ["USD", "GBP", "EUR", "JPY", "AUD", "CNY"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_level_follows_the_clock() {
    let bank = Bank::open(plain_script("2.0002"));
    let hands = Hands::at("2026-10-06T11:00:59.999Z");
    let source = source(&bank, PATIENT, &hands);
    let mut read = Vec::new();
    for now in [
        "2026-10-06T11:00:59.999Z", // before the first window
        "2026-10-06T11:01:00.000Z", // its first millisecond
        "2026-10-06T11:14:59.999Z", // its last
        "2026-10-06T11:15:00.000Z", // the second window's first
        "2026-10-06T11:29:59.999Z", // its last
        "2026-10-06T11:30:00.000Z", // after both
    ] {
        hands.set(now);
        let rates = source.fetch().await.expect("the bank answers");
        read.push(field(&rates, "USD", "selling"));
    }
    assert_eq!(
        read,
        ["3.0003", "1.0001", "1.0001", "2.0002", "2.0002", "3.0003"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetched_at_is_the_clock_at_each_call_cut_to_milliseconds() {
    let bank = Bank::open(plain_script("33.8278"));
    // 987 ms and 456 µs past the second: the answer says 987
    let hands = Hands::at("2026-10-06T11:20:00.987Z").with_microseconds(456);
    let source = source(&bank, PATIENT, &hands);

    let first = source.fetch().await.expect("the bank answers");
    assert_eq!(first.fetched_at, "2026-10-06T11:20:00.987Z");

    // seven hours ahead of UTC it is already the next day in Bangkok; the text stays UTC
    hands.set("2026-12-31T17:00:00.004Z");
    let second = source.fetch().await.expect("the bank answers");
    assert_eq!(second.fetched_at, "2026-12-31T17:00:00.004Z");
    assert_ne!(
        second.fetched_at,
        (hands.clock())().to_rfc3339_opts(SecondsFormat::Micros, true)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_call_asks_the_bank_again_and_keeps_nothing() {
    let bank = Bank::open(plain_script("33.8278"));
    let hands = Hands::at(NOW);
    let source = source(&bank, PATIENT, &hands);

    let first = source.fetch().await.expect("the bank answers");
    assert_eq!(field(&first, "USD", "selling"), "33.8278");
    assert_eq!(bank.seen().len(), 2);

    // the bank publishes a new rate: the very next call has it
    bank.now_answers(plain_script("34.0001"));
    let second = source.fetch().await.expect("the bank answers");
    assert_eq!(field(&second, "USD", "selling"), "34.0001");
    assert_eq!(bank.seen().len(), 4);

    // the bank goes down: no earlier answer is served in its place
    bank.now_answers(Script::from([(LABELS.to_owned(), reply(503, ""))]));
    assert_eq!(refusal(source.fetch().await), SENTENCE);
    assert_eq!(bank.seen().len(), 5);

    // and comes back: the failure was not kept either
    bank.now_answers(plain_script("34.0002"));
    let fourth = source.fetch().await.expect("the bank answers again");
    assert_eq!(field(&fourth, "USD", "selling"), "34.0002");
    assert_eq!(bank.seen().len(), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rates_are_decimals_never_floats() {
    let bank = Bank::open(plain_script("33.0000"));
    let hands = Hands::at(NOW);
    let rates = source(&bank, PATIENT, &hands)
        .fetch()
        .await
        .expect("the bank answers");

    // a binary float gives 0.20938199999999998 here
    assert_eq!(field(&rates, "JPY", "buyingSight"), "0.209382");
    assert_eq!(field(&rates, "JPY", "buyingTransfer"), "0.21");
    assert_eq!(field(&rates, "JPY", "selling"), "0.2163");
    assert_eq!(field(&rates, "JPY", "unit"), "100");
    // the bank's own text is kept beside it, untouched
    assert_eq!(
        field(&rates, "JPY", "original"),
        r#"{"buyingSight":"20.9382","buyingTransfer":"21.0000","selling":"21.6300"}"#
    );
    // trailing zeros go, a whole number has no point, nothing is a string
    assert_eq!(field(&rates, "USD", "selling"), "33");
    assert_eq!(field(&rates, "GBP", "selling"), "44.909");
    assert_eq!(field(&rates, "AUD", "buyingTransfer"), "23");
    assert_eq!(field(&rates, "USD", "unit"), "1");
    for row in &rates.rows {
        for key in ["buyingSight", "buyingTransfer", "selling", "unit"] {
            assert!(row[key].is_number(), "{key} of {row}");
            let text = row[key].to_string();
            assert!(!text.contains(['e', 'E']), "{key} prints as {text}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nobody_listening_is_the_refusal_and_soon() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    let hands = Hands::at(NOW);
    let source = BotRateSource::new(
        format!("http://127.0.0.1:{port}{BASE_PATH}"),
        PATIENT,
        hands.clock(),
    );
    let started = Instant::now();
    assert_eq!(refusal(source.fetch().await), SENTENCE);
    assert!(started.elapsed() < MUST_FINISH, "{:?}", started.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_cut_short_is_the_refusal() {
    for cut in [LABELS, LEVEL_2] {
        let mut script = plain_script("33.8278");
        script.insert(cut.to_owned(), Answer::CutShort);
        let bank = Bank::open(script);
        let hands = Hands::at(NOW);
        assert_eq!(
            refusal(source(&bank, PATIENT, &hands).fetch().await),
            SENTENCE,
            "{cut} cut short"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bank_slower_than_the_timeout_is_the_refusal() {
    let timeout = Duration::from_millis(400);
    for slow_path in [LABELS, LEVEL_2] {
        let mut script = plain_script("33.8278");
        let body = if slow_path == LABELS {
            labels()
        } else {
            table("33.8278")
        };
        script.insert(slow_path.to_owned(), slow(body, Duration::from_secs(6)));
        let bank = Bank::open(script);
        let hands = Hands::at(NOW);
        let started = Instant::now();
        assert_eq!(
            refusal(source(&bank, timeout, &hands).fetch().await),
            SENTENCE,
            "{slow_path} slower than the timeout"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "gave up on {slow_path} only after {:?}; the timeout is {timeout:?}",
            started.elapsed()
        );
        if slow_path == LABELS {
            assert_eq!(
                bank.seen().len(),
                1,
                "no rates are asked for without labels"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_timeout_is_for_each_request_not_for_the_pair() {
    // each answer takes 1.2 s; together they take longer than the 2 s timeout
    let each = Duration::from_millis(1200);
    let mut script = plain_script("33.8278");
    script.insert(LABELS.to_owned(), slow(labels(), each));
    script.insert(LEVEL_2.to_owned(), slow(table("33.8278"), each));
    let bank = Bank::open(script);
    let hands = Hands::at(NOW);
    let started = Instant::now();
    let rates = source(&bank, Duration::from_secs(2), &hands)
        .fetch()
        .await
        .expect("two requests, each inside the timeout");
    assert!(started.elapsed() >= each * 2, "{:?}", started.elapsed());
    assert_eq!(field(&rates, "USD", "selling"), "33.8278");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calls_at_the_same_time_do_not_share_an_answer() {
    let bank = Bank::open(plain_script("33.8278"));
    let hands = Hands::at(NOW);
    let source = Arc::new(source(&bank, PATIENT, &hands));
    let mut calls = Vec::new();
    for _ in 0..4 {
        let source = source.clone();
        calls.push(tokio::spawn(async move { source.fetch().await }));
    }
    for call in calls {
        let rates = call.await.expect("the task").expect("the bank answers");
        assert_eq!(field(&rates, "USD", "selling"), "33.8278");
    }
    assert_eq!(
        bank.seen().len(),
        8,
        "four calls are eight requests: {:?}",
        bank.seen()
    );
}
