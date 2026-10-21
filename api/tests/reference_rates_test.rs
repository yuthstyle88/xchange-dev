//! Specification tests for T5b: `POST /api/reference-rates`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! The route, as v2 answers it (`tests/fixtures/v2_reference_rates_cases.json`,
//! recorded by sending every case through v2's own request callback —
//! `api/tools/v2-fixtures/gen_reference_rates.mjs route`):
//!
//! 1. Only `POST` on exactly `/api/reference-rates` is this route. Any other
//!    method or path is not: the application's fallback answers (404 `Not found`).
//! 2. Nobody signed in → 401 `กรุณาเข้าสู่ระบบ`.
//! 3. Signed in but not the owner → 403 `เฉพาะเจ้าของระบบเท่านั้น`.
//! 4. `Origin` must equal the request's own origin and `X-Xchange-Request` must
//!    be `1` → otherwise 400 `ORIGIN_REJECTED`.
//! 5. The reference-rate source is asked, once, on every request; the body of
//!    the request is never read. Its answer is the 200 body; its refusal is a
//!    400 with the refusal's sentence.
//!
//! The steps are in that order: a caller who may not call is refused before the
//! headers are looked at, and the source is not asked unless 2–4 passed.
//!
//! Who is calling comes through the port ticket T1c declared for the pages:
//! `pages::SessionReader`, found as `web::Data<dyn SessionReader>`. When none
//! is registered nobody is signed in — the state of `main` until ticket T4c
//! registers the real one. An owner session makes the caller the owner, with
//! or without a staff session beside it; a staff session alone makes the
//! caller staff, whatever the role. The source of the rates is the
//! `web::Data<Arc<dyn RateSource>>` in the application data;
//! `reference_rates::configure` registers the real one and these tests register
//! theirs after it, which replaces it.
//!
//! These tests do not compile before ticket T1c (`SessionReader`) is on `main`.
//!
//! No test here reaches the network or needs a database.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use actix_web::body::MessageBody;
use actix_web::dev::ServiceResponse;
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use common::{FakeDatabase, settings, state_with};
use pos_error::{AppError, AppResult};
use serde::Deserialize;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Permission, RoleGrants};
use xc_kernel::ids::BranchId;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xchange::bootstrap;
use xchange::presentation::http as routes;
use xchange::presentation::http::reference_rates;
use xchange::presentation::pages::{self, Callers, SessionReader};
use xchange::presentation::test_callers;

const PATH: &str = "/api/reference-rates";
const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const BANK_DOWN: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";
/// Text no answer may carry: what an internal failure said.
const SECRET: &str = "pool timed out at 10.0.0.7:5432";
const STAFF_ROLES: [&str; 9] = [
    "FRONT",
    "BACKEND",
    "RATES",
    "FINANCE",
    "MANAGER",
    "SYSTEM",
    "MONITOR",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];

/// RBAC §11.2: whether v2 staff role `role` holds `ReferenceRatesRead` once mapped to its Blueprint
/// roles (OR-7.m merge of feat/rbac-groups: the staff roles here are v2 names, as `staff` maps them).
fn holds_reference_rates(role: &str) -> bool {
    test_callers::v2_staff_roles(role)
        .expect("a v2 staff role")
        .iter()
        .any(|&br| RoleGrants::seed(br).contains(&Permission::ReferenceRatesRead))
}

// ----------------------------------------------------------------- the fixture

#[derive(Deserialize)]
struct Fixture {
    path: String,
    sentences: Sentences,
    answer: Value,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sentences {
    sign_in: String,
    owner_only: String,
    origin: String,
    bank_down: String,
    server_error: String,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    caller: String,
    role: Option<String>,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    upstream: String,
    v2: V2,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct V2 {
    status: u16,
    content_type: Option<String>,
    cache_control: Option<String>,
    body: String,
    upstream_requests: usize,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/v2_reference_rates_cases.json"))
        .expect("tests/fixtures/v2_reference_rates_cases.json parses")
}

/// The fixture's `answer` holds each rate as decimal text; on the wire and in
/// `ReferenceRates` it is the JSON number that prints as that text.
fn with_numbers(answer: &Value) -> Value {
    let mut answer = answer.clone();
    for row in answer["rows"].as_array_mut().expect("rows") {
        for key in ["buyingSight", "buyingTransfer", "selling"] {
            let text = row[key].as_str().expect("decimal text").to_owned();
            row[key] = serde_json::from_str(&text).expect("a JSON number");
        }
    }
    answer
}

fn rates_of(document: &Value) -> ReferenceRates {
    ReferenceRates {
        source: document["source"].as_str().expect("source").to_owned(),
        fetched_at: document["fetchedAt"]
            .as_str()
            .expect("fetchedAt")
            .to_owned(),
        kind: document["kind"].as_str().expect("kind").to_owned(),
        rows: document["rows"].as_array().expect("rows").clone(),
    }
}

/// The reference rates the bank's standard table makes: v2's 200 body.
fn standard() -> Value {
    with_numbers(&fixture().answer)
}

// ------------------------------------------------------------------- the fakes

/// What the fake reference-rate source does when asked.
enum Rates {
    Answer(Value),
    /// Refuses the way a source refuses: `AppError::Domain` with this sentence.
    Refuse(&'static str),
    /// Fails in a way that is nobody's sentence.
    Break,
    /// Answers with a document that names how many times it has been asked.
    Count,
}

struct FakeRates {
    does: Rates,
    calls: AtomicUsize,
}

impl FakeRates {
    fn new(does: Rates) -> Arc<FakeRates> {
        Arc::new(FakeRates {
            does,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl RateSource for FakeRates {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Box::pin(async move {
            match &self.does {
                Rates::Answer(document) => Ok(rates_of(document)),
                Rates::Refuse(sentence) => Err(AppError::Domain((*sentence).to_owned())),
                Rates::Break => Err(AppError::Internal(SECRET.to_owned())),
                Rates::Count => Ok(ReferenceRates {
                    source: "counted".to_owned(),
                    fetched_at: format!("call {n}"),
                    kind: "COUNT".to_owned(),
                    rows: vec![json!({ "n": n })],
                }),
            }
        })
    }
}

/// Which sessions the fake says the request carries.
#[derive(Clone, Copy)]
enum Who {
    Nobody,
    Owner,
    /// A staff session with this role.
    Staff(&'static str),
    /// An owner session and a staff session in the same browser.
    OwnerAndStaff(&'static str),
    /// Whoever the request's `Cookie: who=…` names; nobody without it.
    FromCookie,
}

fn owner() -> Option<Actor> {
    Some(test_callers::owner("owner"))
}

fn staff(role: &str) -> Option<Actor> {
    let roles =
        test_callers::v2_staff_roles(role).unwrap_or_else(|| panic!("unknown staff role {role:?}"));
    Some(test_callers::staff(
        &format!("E-{role}"),
        roles,
        BranchId::new("B2"),
    ))
}

struct FakeCallers {
    who: Who,
    calls: AtomicUsize,
}

impl FakeCallers {
    fn new(who: Who) -> Arc<FakeCallers> {
        Arc::new(FakeCallers {
            who,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SessionReader for FakeCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let (owner, staff) = match self.who {
                Who::Nobody => (None, None),
                Who::Owner => (owner(), None),
                Who::Staff(role) => (None, staff(role)),
                Who::OwnerAndStaff(role) => (owner(), staff(role)),
                Who::FromCookie => {
                    let cookie = req
                        .headers()
                        .get(header::COOKIE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    match cookie {
                        "who=owner" => (owner(), None),
                        // RBAC §11.2: RATES holds ReferenceRatesRead now; FRONT is still refused
                        "who=staff" => (None, staff("FRONT")),
                        _ => (None, None),
                    }
                }
            };
            test_callers::callers(owner, staff)
        })
    }
}

fn as_rates(fake: &Arc<FakeRates>) -> web::Data<Arc<dyn RateSource>> {
    let port: Arc<dyn RateSource> = fake.clone();
    web::Data::new(port)
}

fn as_sessions(fake: &Arc<FakeCallers>) -> web::Data<dyn SessionReader> {
    let reader: Arc<dyn SessionReader> = fake.clone();
    web::Data::from(reader)
}

// --------------------------------------------------------------------- the app

/// The whole application as `bootstrap::configure` builds it, with the two
/// fakes registered after it.
macro_rules! application {
    ($rates:expr, $sessions:expr) => {
        test::init_service(
            App::new()
                .configure(|cfg| {
                    bootstrap::configure(cfg, state_with(settings(&[]), FakeDatabase::Up))
                })
                .app_data(as_rates($rates))
                .app_data(as_sessions($sessions)),
        )
        .await
    };
}

/// A `POST` the owner's browser would send: same origin, the marker header.
fn post() -> test::TestRequest {
    test::TestRequest::post()
        .uri(PATH)
        .insert_header((header::HOST, "desk.test"))
        .insert_header((header::ORIGIN, "http://desk.test"))
        .insert_header(("X-Xchange-Request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .set_payload("{}")
}

/// What came back: status, the two headers v2 sets, the body as text.
struct Answer {
    status: StatusCode,
    content_type: String,
    cache_control: Option<String>,
    body: String,
}

async fn answer_of<B>(resp: ServiceResponse<B>) -> Answer
where
    B: MessageBody,
    B::Error: std::fmt::Debug,
{
    let text = |name: header::HeaderName| {
        resp.headers()
            .get(name)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
    };
    let (status, content_type, cache_control) = (
        resp.status(),
        text(header::CONTENT_TYPE).unwrap_or_default(),
        text(header::CACHE_CONTROL),
    );
    let body = String::from_utf8(test::read_body(resp).await.to_vec()).expect("a UTF-8 body");
    Answer {
        status,
        content_type,
        cache_control,
        body,
    }
}

/// Send one request to an application built by `test::init_service`.
macro_rules! send {
    ($app:expr, $request:expr) => {
        answer_of(test::call_service($app, $request.to_request()).await).await
    };
}

fn error_body(sentence: &str) -> String {
    format!(r#"{{"error":"{sentence}"}}"#)
}

fn is_json(answer: &Answer) -> bool {
    answer
        .content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
}

// ------------------------------------------------------------------ the wiring

#[actix_web::test]
async fn the_fixture_is_the_recording_this_file_expects() {
    let fixture = fixture();
    assert_eq!(fixture.path, PATH);
    assert_eq!(fixture.cases.len(), 72, "cases in the fixture");
    let count = |status: u16| {
        fixture
            .cases
            .iter()
            .filter(|c| c.v2.status == status)
            .count()
    };
    assert_eq!(
        [
            count(200),
            count(400),
            count(401),
            count(403),
            count(404),
            count(500)
        ],
        [18, 22, 5, 12, 14, 1]
    );
    assert_eq!(fixture.sentences.sign_in, SIGN_IN);
    assert_eq!(fixture.sentences.owner_only, OWNER_ONLY);
    assert_eq!(fixture.sentences.origin, ORIGIN_REJECTED);
    assert_eq!(fixture.sentences.bank_down, BANK_DOWN);
    assert_eq!(fixture.sentences.server_error, SERVER_ERROR);
    // all nine staff roles are in it
    let mut roles: Vec<&str> = fixture
        .cases
        .iter()
        .filter_map(|c| c.role.as_deref())
        .collect();
    roles.sort_unstable();
    roles.dedup();
    assert_eq!(roles.len(), 9, "{roles:?}");
    // v2's 200 body is the answer, number for number
    let ok = fixture
        .cases
        .iter()
        .find(|c| c.v2.status == 200)
        .expect("a 200");
    let v2: Value = serde_json::from_str(&ok.v2.body).expect("v2's 200 body");
    assert_eq!(v2, with_numbers(&fixture.answer));
}

/// The first route a feature ticket registers: it must be reachable through
/// the one route table and through the whole application, with nothing injected.
/// With no `SessionReader` registered nobody is signed in, so the answer is the
/// 401 — and never the fallback's 404, which is what an unregistered route gives.
#[actix_web::test]
async fn the_route_is_registered_through_every_configure_and_closed_by_default() {
    let whole =
        test::init_service(App::new().configure(|cfg| {
            bootstrap::configure(cfg, state_with(settings(&[]), FakeDatabase::Up))
        }))
        .await;
    let state = state_with(settings(&[]), FakeDatabase::Up);
    let table = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(|cfg| routes::configure(cfg, &state)),
    )
    .await;
    let state = state_with(settings(&[]), FakeDatabase::Up);
    let module = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(|cfg| reference_rates::configure(cfg, &state)),
    )
    .await;

    for answer in [
        send!(&whole, post()),
        send!(&table, post()),
        send!(&module, post()),
    ] {
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
        assert!(is_json(&answer), "{}", answer.content_type);
        assert_eq!(answer.body, error_body(SIGN_IN));
    }
}

#[actix_web::test]
async fn without_the_module_the_path_is_the_fallbacks() {
    // the control for the test above: this is what "not registered" looks like
    let app = test::init_service(App::new().default_service(web::to(pages::fallback))).await;
    let answer = send!(&app, post());
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.body, "Not found");
}

#[actix_web::test]
async fn with_a_source_but_no_session_reader_nobody_is_signed_in() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let app = test::init_service(
        App::new()
            .configure(|cfg| bootstrap::configure(cfg, state_with(settings(&[]), FakeDatabase::Up)))
            .app_data(as_rates(&rates)),
    )
    .await;
    let answer = send!(&app, post());
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.body, error_body(SIGN_IN));
    assert_eq!(rates.calls(), 0, "the source was asked for nobody");
}

#[actix_web::test]
async fn another_method_is_the_plain_404_even_without_the_applications_fallback() {
    // The route table alone has no default service. The resource itself must
    // hand every other method to `pages::fallback` — v2's 404 `Not found` —
    // instead of actix's own 405 (or its empty 404).
    let state = state_with(settings(&[]), FakeDatabase::Up);
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::Owner);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .configure(|cfg| routes::configure(cfg, &state))
            .app_data(as_rates(&rates))
            .app_data(as_sessions(&sessions)),
    )
    .await;
    for method in [
        Method::GET,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
        Method::HEAD,
    ] {
        let answer = send!(&app, post().method(method.clone()));
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{method}");
        assert!(
            answer
                .content_type
                .to_ascii_lowercase()
                .starts_with("text/plain"),
            "{method}: {}",
            answer.content_type
        );
        if method != Method::HEAD {
            assert_eq!(answer.body, "Not found", "{method}");
        }
    }
    assert_eq!(rates.calls(), 0);
    assert_eq!(
        sessions.calls(),
        0,
        "nobody is looked up for a path that is no route"
    );
    // and the POST beside them is the route
    assert_eq!(send!(&app, post()).status, StatusCode::OK);
    assert_eq!(rates.calls(), 1);
}

// ---------------------------------------------------------------- every case

/// Every recorded case through the whole application.
#[actix_web::test]
async fn every_recorded_case_is_answered_as_v2_answered_it() {
    let fixture = fixture();
    let standard = with_numbers(&fixture.answer);
    let mut wrong = Vec::new();
    let mut skipped = 0;

    for case in &fixture.cases {
        let rates = FakeRates::new(match case.upstream.as_str() {
            "answers" => Rates::Answer(standard.clone()),
            "fails" => Rates::Refuse(BANK_DOWN),
            other => panic!("{}: unknown upstream {other}", case.name),
        });
        let staff_role: Option<&'static str> = if case.caller == "staff" {
            Some(
                STAFF_ROLES
                    .iter()
                    .copied()
                    .find(|&r| Some(r) == case.role.as_deref())
                    .expect("one of the nine staff roles"),
            )
        } else {
            None
        };
        let sessions = FakeCallers::new(match case.caller.as_str() {
            "anonymous" => Who::Nobody,
            "owner" => Who::Owner,
            "staff" => Who::Staff(staff_role.expect("staff role")),
            // v2 answers 500 when it cannot read the session. The port has no way to
            // say that (`sessions` returns `Sessions`, not a `Result`), so what a
            // failed look-up answers belongs to the reader's ticket (T4c), not to this route.
            "lookup fails" => {
                skipped += 1;
                continue;
            }
            other => panic!("{}: unknown caller {other}", case.name),
        });
        let app = application!(&rates, &sessions);

        let method = Method::from_bytes(case.method.as_bytes()).expect("a method");
        let mut request = test::TestRequest::default()
            .method(method.clone())
            .uri(&case.path);
        for (name, value) in &case.headers {
            request = request.insert_header((name.as_str(), value.as_str()));
        }
        if let Some(body) = &case.body {
            request = request.set_payload(body.clone());
        }
        let answer = send!(&app, request);
        let mut fail = |what: String| wrong.push(format!("{}: {what}", case.name));

        // Under §11.2 (rule S11), staff roles holding ReferenceRatesRead in RoleGrants::seed
        // are admitted where v2 refused all staff with 403.
        let s11_admitted = staff_role.is_some_and(holds_reference_rates) && case.v2.status == 403;
        let expected_status = if s11_admitted {
            if case.upstream == "answers" { 200 } else { 400 }
        } else {
            case.v2.status
        };
        let expected_body = if s11_admitted && case.upstream == "fails" {
            error_body(BANK_DOWN)
        } else {
            case.v2.body.clone()
        };
        let expected_cache_control = if s11_admitted {
            Some("no-store")
        } else {
            case.v2.cache_control.as_deref()
        };
        let expected_asked = if s11_admitted {
            1
        } else {
            usize::from(case.v2.upstream_requests > 0)
        };

        if answer.status.as_u16() != expected_status {
            fail(format!(
                "status {}, expected {} — body {:?}",
                answer.status.as_u16(),
                expected_status,
                answer.body.chars().take(80).collect::<String>()
            ));
            continue;
        }
        match expected_status {
            200 => {
                match serde_json::from_str::<Value>(&answer.body) {
                    Ok(body) if body == standard => {}
                    _ => fail(format!("200 body {:?}", answer.body)),
                }
                if !is_json(&answer) {
                    fail(format!("Content-Type {:?}", answer.content_type));
                }
            }
            404 => {
                // not this route: the application's fallback answered
                if method != Method::HEAD && answer.body != expected_body {
                    fail(format!("404 body {:?}", answer.body));
                }
                if !answer
                    .content_type
                    .to_ascii_lowercase()
                    .starts_with("text/plain")
                {
                    fail(format!("404 Content-Type {:?}", answer.content_type));
                }
            }
            _ => {
                if answer.body != expected_body {
                    fail(format!(
                        "body {:?}, expected {:?}",
                        answer.body, expected_body
                    ));
                }
                if !is_json(&answer) {
                    fail(format!("Content-Type {:?}", answer.content_type));
                }
            }
        }
        // where v2 forbids storing the answer, so does v3
        if expected_cache_control.is_some()
            && answer.cache_control.as_deref() != expected_cache_control
        {
            fail(format!(
                "Cache-Control {:?}, expected {:?}",
                answer.cache_control, expected_cache_control
            ));
        }
        // v2 asks the bank (two requests per fetch) exactly when v3 asks its source once
        if rates.calls() != expected_asked {
            fail(format!(
                "the source was asked {} times, expected {expected_asked}",
                rates.calls()
            ));
        }
        assert!(
            case.v2.content_type.is_some(),
            "{}: the recording has no Content-Type",
            case.name
        );
    }
    assert!(
        wrong.is_empty(),
        "{} differences from v2:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert_eq!(skipped, 1, "only the failed look-up is left to ticket T4c");
}

// ------------------------------------------------------------ rule by rule

#[actix_web::test]
async fn the_owner_gets_the_sources_answer_as_json_numbers() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    let answer = send!(&app, post());
    assert_eq!(answer.status, StatusCode::OK);
    assert!(is_json(&answer), "{}", answer.content_type);
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));

    let body: Value = serde_json::from_str(&answer.body).expect("a JSON body");
    let object = body.as_object().expect("an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["fetchedAt", "kind", "rows", "source"]);
    assert_eq!(
        body["source"],
        "https://www.bot.or.th/th/statistics/exchange-rate.html"
    );
    assert_eq!(body["fetchedAt"], "2026-10-06T11:20:00.000Z");
    assert_eq!(body["kind"], "DAILY_COMMERCIAL_BANK_AVERAGE");
    let rows = body["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 6);
    for row in rows {
        let mut keys: Vec<&str> = row
            .as_object()
            .expect("a row")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "buyingSight",
                "buyingTransfer",
                "code",
                "date",
                "original",
                "selling",
                "unit"
            ]
        );
        for key in ["buyingSight", "buyingTransfer", "selling", "unit"] {
            assert!(row[key].is_number(), "{key} of {row} is not a JSON number");
        }
        for key in ["buyingSight", "buyingTransfer", "selling"] {
            assert!(row["original"][key].is_string(), "original.{key} of {row}");
        }
    }
    // rates are numbers on the wire, printed as the decimals they are
    for piece in [
        r#""buyingSight":33.4218"#,
        r#""selling":44.909"#,
        r#""unit":100"#,
        r#""buyingSight":0.2095"#,
        r#""selling":"44.9090""#,
        r#""fetchedAt":"2026-10-06T11:20:00.000Z""#,
    ] {
        assert!(
            answer.body.contains(piece),
            "{piece} not in {}",
            answer.body
        );
    }
    assert_eq!(rates.calls(), 1);
    assert_eq!(sessions.calls(), 1);
}

#[actix_web::test]
async fn the_answer_is_the_sources_whatever_it_holds() {
    // The handler adds nothing, drops nothing and computes nothing: a source
    // with other rows (a second one is planned) is passed on as it is.
    let odd = json!({
        "source": "https://example.test/kbank",
        "fetchedAt": "2001-02-03T04:05:06.007Z",
        "kind": "SOMETHING_ELSE",
        "rows": [
            { "code": "XAU", "unit": 1, "selling": 99999999.9999, "note": "ทองคำ" },
            { "code": "JPY", "unit": 100, "buyingSight": 0.209382, "original": { "buyingSight": "20.9382" } },
        ],
    });
    let rates = FakeRates::new(Rates::Answer(odd.clone()));
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    let answer = send!(&app, post());
    assert_eq!(answer.status, StatusCode::OK);
    let body: Value = serde_json::from_str(&answer.body).expect("a JSON body");
    assert_eq!(body, odd);
    assert!(
        answer.body.contains(r#""buyingSight":0.209382"#),
        "{}",
        answer.body
    );
    assert!(answer.body.contains("ทองคำ"), "{}", answer.body);
}

#[actix_web::test]
async fn every_request_asks_the_source_again() {
    let rates = FakeRates::new(Rates::Count);
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    for n in 1..=3 {
        let answer = send!(&app, post());
        assert_eq!(answer.status, StatusCode::OK);
        let body: Value = serde_json::from_str(&answer.body).expect("a JSON body");
        assert_eq!(body["fetchedAt"], format!("call {n}"));
        assert_eq!(body["rows"], json!([{ "n": n }]));
        assert_eq!(rates.calls(), n);
    }
    assert_eq!(
        sessions.calls(),
        3,
        "the caller is looked up on every request"
    );
}

#[actix_web::test]
async fn a_refusal_of_the_source_is_a_400_with_its_sentence() {
    for sentence in [BANK_DOWN, "ธนาคารอื่นไม่ตอบ"] {
        let rates = FakeRates::new(Rates::Refuse(sentence));
        let sessions = FakeCallers::new(Who::Owner);
        let app = application!(&rates, &sessions);

        let answer = send!(&app, post());
        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        assert!(is_json(&answer), "{}", answer.content_type);
        assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
        assert_eq!(answer.body, error_body(sentence));
        assert_eq!(rates.calls(), 1, "asked once, not retried");
    }
}

#[actix_web::test]
async fn a_source_that_breaks_is_a_500_that_says_nothing() {
    let rates = FakeRates::new(Rates::Break);
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    let answer = send!(&app, post());
    assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(is_json(&answer), "{}", answer.content_type);
    assert_eq!(answer.body, error_body(SERVER_ERROR));
    assert!(!answer.body.contains(SECRET));
    assert_eq!(rates.calls(), 1);
}

#[actix_web::test]
async fn nobody_signed_in_is_a_401_before_anything_else() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::Nobody);
    let app = application!(&rates, &sessions);

    for request in [
        post(),
        // no Origin, no marker: still the 401, not the 400
        test::TestRequest::post()
            .uri(PATH)
            .insert_header((header::HOST, "desk.test")),
    ] {
        let answer = send!(&app, request);
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
        assert!(is_json(&answer), "{}", answer.content_type);
        assert_eq!(answer.body, error_body(SIGN_IN));
    }
    assert_eq!(rates.calls(), 0);
    assert_eq!(sessions.calls(), 2);
}

#[actix_web::test]
async fn staff_are_a_403_before_the_headers_are_looked_at() {
    assert!(
        STAFF_ROLES.iter().any(|&role| !holds_reference_rates(role)),
        "§11.2 must leave some staff role without ReferenceRatesRead, or this test refuses nobody"
    );
    for role in STAFF_ROLES
        .into_iter()
        .filter(|&role| !holds_reference_rates(role))
    {
        let rates = FakeRates::new(Rates::Answer(standard()));
        let sessions = FakeCallers::new(Who::Staff(role));
        let app = application!(&rates, &sessions);

        for request in [
            post(),
            test::TestRequest::post()
                .uri(PATH)
                .insert_header((header::HOST, "desk.test")),
        ] {
            let answer = send!(&app, request);
            assert_eq!(answer.status, StatusCode::FORBIDDEN, "{role:?}");
            assert!(is_json(&answer), "{role:?}: {}", answer.content_type);
            assert_eq!(answer.body, error_body(OWNER_ONLY), "{role:?}");
        }
        assert_eq!(rates.calls(), 0, "{role:?}");
    }
}

#[actix_web::test]
async fn an_owner_session_beside_a_staff_session_is_the_owner() {
    // v2 looks for the owner's session first and stops there
    for role in ["FRONT", "RATES", "MANAGER"] {
        let rates = FakeRates::new(Rates::Answer(standard()));
        let sessions = FakeCallers::new(Who::OwnerAndStaff(role));
        let app = application!(&rates, &sessions);
        let answer = send!(&app, post());
        assert_eq!(answer.status, StatusCode::OK, "{role:?}: {}", answer.body);
        assert_eq!(rates.calls(), 1);
    }
}

#[actix_web::test]
async fn the_owner_needs_the_same_origin_and_the_marker() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);
    let bare = || {
        test::TestRequest::post()
            .uri(PATH)
            .insert_header((header::HOST, "desk.test"))
    };

    for (what, request) in [
        (
            "no Origin",
            bare().insert_header(("X-Xchange-Request", "1")),
        ),
        (
            "no marker",
            bare().insert_header((header::ORIGIN, "http://desk.test")),
        ),
        (
            "another site",
            post().insert_header((header::ORIGIN, "http://desk.test.evil.example")),
        ),
        (
            "the marker with another value",
            post().insert_header(("X-Xchange-Request", "yes")),
        ),
    ] {
        let answer = send!(&app, request);
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{what}");
        assert!(is_json(&answer), "{what}: {}", answer.content_type);
        assert_eq!(answer.cache_control.as_deref(), Some("no-store"), "{what}");
        assert_eq!(answer.body, error_body(ORIGIN_REJECTED), "{what}");
    }
    assert_eq!(rates.calls(), 0, "a refused request reached the source");

    // the request's own origin is its scheme and its Host header, nothing else
    for (what, request) in [
        ("plain http", post()),
        (
            "https behind the proxy",
            post()
                .insert_header(("X-Forwarded-Proto", "https"))
                .insert_header((header::ORIGIN, "https://desk.test")),
        ),
        (
            "a port",
            post()
                .insert_header((header::HOST, "127.0.0.1:8093"))
                .insert_header((header::ORIGIN, "http://127.0.0.1:8093")),
        ),
    ] {
        let answer = send!(&app, request);
        assert_eq!(answer.status, StatusCode::OK, "{what}: {}", answer.body);
    }
    assert_eq!(rates.calls(), 3);
}

#[actix_web::test]
async fn the_session_reader_is_given_the_request_itself() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::FromCookie);
    let app = application!(&rates, &sessions);

    for (cookie, status) in [
        (Some("who=owner"), StatusCode::OK),
        (Some("who=staff"), StatusCode::FORBIDDEN),
        (Some("who=stranger"), StatusCode::UNAUTHORIZED),
        (None, StatusCode::UNAUTHORIZED),
    ] {
        let mut request = post();
        if let Some(cookie) = cookie {
            request = request.insert_header((header::COOKIE, cookie));
        }
        assert_eq!(send!(&app, request).status, status, "{cookie:?}");
    }
    assert_eq!(sessions.calls(), 4);
    assert_eq!(rates.calls(), 1);
}

#[actix_web::test]
async fn the_body_is_never_read() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    // what is sent: a name, the bytes, the Content-Type
    type Sent = (&'static str, Option<Vec<u8>>, Option<&'static str>);
    let bodies: [Sent; 6] = [
        ("no body", None, None),
        ("not JSON", Some(b"{".to_vec()), Some("application/json")),
        (
            "not UTF-8",
            Some(vec![0xff, 0xfe, 0x00]),
            Some("application/json"),
        ),
        (
            "a form",
            Some(b"a=1".to_vec()),
            Some("application/x-www-form-urlencoded"),
        ),
        (
            "a megabyte",
            Some(vec![b'x'; 1_048_576]),
            Some("text/plain"),
        ),
        ("no Content-Type", Some(b"{}".to_vec()), None),
    ];
    for (what, body, content_type) in bodies {
        let mut request = test::TestRequest::post()
            .uri(PATH)
            .insert_header((header::HOST, "desk.test"))
            .insert_header((header::ORIGIN, "http://desk.test"))
            .insert_header(("X-Xchange-Request", "1"));
        if let Some(content_type) = content_type {
            request = request.insert_header((header::CONTENT_TYPE, content_type));
        }
        if let Some(body) = body {
            request = request.set_payload(body);
        }
        let answer = send!(&app, request);
        assert_eq!(answer.status, StatusCode::OK, "{what}: {}", answer.body);
        let body: Value = serde_json::from_str(&answer.body).expect("a JSON body");
        assert_eq!(body, standard(), "{what}");
    }
    assert_eq!(rates.calls(), 6);
}

// ------------------------------------------------------- added with ticket #97

/// Sentences a source could refuse with that are not safe to paste between
/// two quotes: the body must still be JSON and say exactly the sentence.
#[actix_web::test]
async fn a_refusal_sentence_that_needs_escaping_is_still_json() {
    for sentence in [
        r#"ธนาคารตอบว่า "ปิดปรับปรุง""#,
        r"C:\rates\today",
        "บรรทัดแรก\nบรรทัดสอง",
        "tab\there, return\r, bell\u{7}",
        r#"","rows":[],"x":""#,
        r"ends with a backslash \",
    ] {
        let rates = FakeRates::new(Rates::Refuse(sentence));
        let sessions = FakeCallers::new(Who::Owner);
        let app = application!(&rates, &sessions);

        let answer = send!(&app, post());
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{sentence:?}");
        assert!(is_json(&answer), "{}", answer.content_type);
        assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
        let body: Value = serde_json::from_slice(answer.body.as_bytes()).unwrap_or_else(|e| {
            panic!("{sentence:?}: the body is not JSON ({e}): {}", answer.body)
        });
        assert_eq!(body, json!({ "error": sentence }), "{sentence:?}");
    }
}

/// The request's own origin drops a port only when it is the default port of
/// the request's own scheme: `:443` on plain http and `:80` on https are
/// ports like any other.
#[actix_web::test]
async fn the_default_port_of_the_other_scheme_is_not_dropped() {
    let rates = FakeRates::new(Rates::Answer(standard()));
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    let request = |host: &'static str, origin: &'static str, https: bool| {
        let request = post()
            .insert_header((header::HOST, host))
            .insert_header((header::ORIGIN, origin));
        if https {
            request.insert_header(("X-Forwarded-Proto", "https"))
        } else {
            request
        }
    };
    for (host, origin, https, status) in [
        ("x:443", "http://x", false, StatusCode::BAD_REQUEST),
        ("x:443", "http://x:443", false, StatusCode::OK),
        ("x:443", "https://x", false, StatusCode::BAD_REQUEST),
        ("x:80", "https://x", true, StatusCode::BAD_REQUEST),
        ("x:80", "https://x:80", true, StatusCode::OK),
        ("x:80", "http://x", true, StatusCode::BAD_REQUEST),
        // and its own default port still is, as v2 recorded
        ("x:80", "http://x", false, StatusCode::OK),
        ("x:443", "https://x", true, StatusCode::OK),
    ] {
        let what = format!("Host {host}, Origin {origin}, https {https}");
        let answer = send!(&app, request(host, origin, https));
        assert_eq!(answer.status, status, "{what}: {}", answer.body);
        if status == StatusCode::BAD_REQUEST {
            assert_eq!(answer.body, error_body(ORIGIN_REJECTED), "{what}");
        }
    }
    assert_eq!(
        rates.calls(),
        4,
        "only the four accepted requests reach the source"
    );
}

/// The headers of the 500, as v2 recorded them (fixture case "the session
/// look-up fails"): JSON in UTF-8 and, unlike every 200 and 400 of this route,
/// no `Cache-Control`.
#[actix_web::test]
async fn a_source_that_breaks_answers_with_v2s_500_headers() {
    let recorded = fixture()
        .cases
        .into_iter()
        .find(|case| case.v2.status == 500)
        .expect("the fixture records a 500");
    assert_eq!(
        (
            recorded.v2.content_type.as_deref(),
            recorded.v2.cache_control.as_deref()
        ),
        (Some("application/json; charset=utf-8"), None)
    );

    let rates = FakeRates::new(Rates::Break);
    let sessions = FakeCallers::new(Who::Owner);
    let app = application!(&rates, &sessions);

    let answer = send!(&app, post());
    assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(answer.content_type, "application/json; charset=utf-8");
    assert_eq!(answer.cache_control, None);
    assert_eq!(answer.body, recorded.v2.body);
}

/// The 401 and the 403 carry no `Cache-Control`, as v2 recorded for every one
/// of them; the 400s and the 200 say `no-store`.
#[actix_web::test]
async fn the_401_and_the_403_carry_no_cache_control() {
    let recorded = fixture();
    for status in [401, 403] {
        let of_status: Vec<&Case> = recorded
            .cases
            .iter()
            .filter(|case| case.v2.status == status)
            .collect();
        assert!(!of_status.is_empty(), "the fixture records a {status}");
        assert!(
            of_status.iter().all(|case| case.v2.cache_control.is_none()
                && case.v2.content_type.as_deref() == Some("application/json")),
            "v2's {status} headers"
        );
    }

    assert!(
        !xc_auth::domain::access::RoleGrants::seed(xc_auth::domain::access::Role::Teller)
            .contains(&xc_auth::domain::access::Permission::ReferenceRatesRead),
        "§11.2: TELLER (Role::Front) does not hold ReferenceRatesRead"
    );
    for (who, status) in [
        (Who::Nobody, StatusCode::UNAUTHORIZED),
        (Who::Staff("FRONT"), StatusCode::FORBIDDEN),
    ] {
        let rates = FakeRates::new(Rates::Answer(standard()));
        let sessions = FakeCallers::new(who);
        let app = application!(&rates, &sessions);

        let answer = send!(&app, post());
        assert_eq!(answer.status, status);
        assert_eq!(answer.content_type, "application/json");
        assert_eq!(answer.cache_control, None, "{status}");
    }
}
