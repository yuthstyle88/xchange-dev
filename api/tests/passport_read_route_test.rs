//! RR-1 up-front tests: `POST /api/v3/compute/passport_read` (design
//! `docs/design/recognize-passport-read.md` §4, behaviour table §8 rows P1–P13).
//!
//! Written before the implementation by the ticket author (claude/xchange-desk). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The route is mounted alone, with a fake `PassportReader` (`AppState::with_passport_reader`) and a
//! fake session reader that answers from the `x-test-as` header. `AppState` runs on a pool that
//! never connects. No database and no network.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use async_trait::async_trait;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::application::passport_read::{
    AiLocation, AiProvider, ImageMime, PassportImage, PassportRead, PassportReader, PassportResult,
    ReadError, VisualZone,
};
use xchange::bootstrap::AppState;
use xchange::presentation::http::v3::passport_read::{MAX_IMAGE_BYTES, PATH, register};
use xchange::presentation::pages::{SessionReader, Sessions};

const HOST: &str = "desk.example";
const ORIGIN: &str = "http://desk.example";
const AS: &str = "x-test-as";
const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01,
];

// ── fakes ────────────────────────────────────────────────────────────────

/// `owner` is the owner; `teller` is a Front at MAIN; `rates` is a Rates at MAIN; `finance` is a
/// Finance at MAIN; any `Role::STAFF` name is that role at MAIN; anything else is nobody.
struct FakeSessions;

impl SessionReader for FakeSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            let who = req
                .headers()
                .get(AS)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let main = BranchId::new("MAIN");
            match who {
                "owner" => Sessions {
                    owner: Some(Actor::owner("owner")),
                    staff: None,
                },
                "teller" => Sessions {
                    owner: None,
                    staff: Actor::staff("u-teller", Role::Front, main),
                },
                "rates" => Sessions {
                    owner: None,
                    staff: Actor::staff("u-rates", Role::Rates, main),
                },
                "finance" => Sessions {
                    owner: None,
                    staff: Actor::staff("u-finance", Role::Finance, main),
                },
                other => Sessions {
                    owner: None,
                    staff: Role::parse(other)
                        .filter(|r| Role::STAFF.contains(r))
                        .and_then(|r| Actor::staff(format!("u-{other}"), r, main)),
                },
            }
        })
    }
}

/// Answers a fixed outcome and records every image it was given.
struct FakeReader {
    answer: Result<PassportRead, ReadError>,
    calls: AtomicUsize,
    seen: Mutex<Vec<(ImageMime, Vec<u8>)>>,
}

impl FakeReader {
    fn new(answer: Result<PassportRead, ReadError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PassportReader for FakeReader {
    async fn read(&self, image: &PassportImage) -> Result<PassportRead, ReadError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen
            .lock()
            .unwrap()
            .push((image.mime(), image.bytes().to_vec()));
        self.answer.clone()
    }
}

fn full_read() -> PassportRead {
    PassportRead {
        provider: AiProvider::Gemini,
        model: "gemini-3.1-flash-lite".to_owned(),
        location: AiLocation::Cloud,
        result: Some(PassportResult {
            name: Some("ANNA MARIA ERIKSSON".to_owned()),
            passport_no: Some("L898902C3".to_owned()),
            nationality: Some("UTO".to_owned()),
            birth_date: Some("1974-08-12".to_owned()),
            expiry_date: Some("2012-04-15".to_owned()),
            score: 5,
            visual_zone: VisualZone::Same,
        }),
        auto_acceptable: true,
    }
}

// ── harness ──────────────────────────────────────────────────────────────

fn state(reader: Option<Arc<dyn PassportReader>>, env: &[(&str, &str)]) -> AppState {
    let s = AppState::new(Arc::new(common::settings(env)), common::unreachable_pool());
    match reader {
        Some(r) => s.with_passport_reader(r),
        None => s,
    }
}

/// Status, `Retry-After`, `Cache-Control` and the JSON body of one request.
struct Answer {
    status: StatusCode,
    retry_after: Option<String>,
    cache_control: Option<String>,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

struct Req<'a> {
    who: &'a str,
    content_type: Option<&'a str>,
    origin: Option<&'a str>,
    marker: bool,
    body: Vec<u8>,
    query: &'a str,
}

impl<'a> Req<'a> {
    fn teller_jpeg() -> Self {
        Self {
            who: "teller",
            content_type: Some("image/jpeg"),
            origin: Some(ORIGIN),
            marker: true,
            body: JPEG.to_vec(),
            query: "",
        }
    }
}

async fn send(state: AppState, req: Req<'_>) -> Answer {
    let sessions: Arc<dyn SessionReader> = Arc::new(FakeSessions);
    let for_register = state.clone();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::from(sessions))
            .app_data(web::Data::new(state))
            .configure(move |cfg| register(cfg, &for_register)),
    )
    .await;
    let mut r = test::TestRequest::post()
        .uri(&format!("{PATH}{}", req.query))
        .insert_header((header::HOST, HOST))
        .insert_header((AS, req.who));
    if let Some(ct) = req.content_type {
        r = r.insert_header((header::CONTENT_TYPE, ct));
    }
    if let Some(o) = req.origin {
        r = r.insert_header((header::ORIGIN, o));
    }
    if req.marker {
        r = r.insert_header(("x-xchange-request", "1"));
    }
    let res = test::call_service(&app, r.set_payload(req.body).to_request()).await;
    let status = res.status();
    let get = |name: header::HeaderName| {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let retry_after = get(header::RETRY_AFTER);
    let cache_control = get(header::CACHE_CONTROL);
    let bytes = test::read_body(res).await;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer {
        status,
        retry_after,
        cache_control,
        body,
    }
}

// ── P1–P3: the answer ────────────────────────────────────────────────────

#[actix_web::test]
async fn p1_a_teller_read_answers_200_with_the_desks_field_names() {
    let reader = FakeReader::new(Ok(full_read()));
    let a = send(state(Some(reader.clone()), &[]), Req::teller_jpeg()).await;
    assert_eq!(a.status, StatusCode::OK, "body: {}", a.body);
    assert_eq!(
        a.body,
        json!({
            "fields": {
                "name": "ANNA MARIA ERIKSSON",
                "passportNo": "L898902C3",
                "nationality": "UTO",
                "birthDate": "1974-08-12",
                "passportExpiry": "2012-04-15"
            },
            "score": 5,
            "visualZone": "same",
            "autoAcceptable": true,
            "ai": { "provider": "gemini", "model": "gemini-3.1-flash-lite", "location": "cloud" }
        })
    );
    assert_eq!(a.cache_control.as_deref(), Some("no-store"));
    let seen = reader.seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![(ImageMime::Jpeg, JPEG.to_vec())],
        "the exact bytes and type reach the reader"
    );
}

#[actix_web::test]
async fn p2_a_refused_read_is_200_with_fields_null_not_an_error() {
    let mut read = full_read();
    read.result = None;
    read.auto_acceptable = false;
    read.provider = AiProvider::Ollama;
    read.model = "qwen2.5vl:7b".to_owned();
    read.location = AiLocation::Local;
    let a = send(
        state(Some(FakeReader::new(Ok(read))), &[]),
        Req::teller_jpeg(),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "body: {}", a.body);
    assert_eq!(
        a.body,
        json!({
            "fields": null,
            "score": 0,
            "visualZone": null,
            "autoAcceptable": false,
            "ai": { "provider": "ollama", "model": "qwen2.5vl:7b", "location": "local" }
        })
    );
}

#[actix_web::test]
async fn p3_a_partial_read_keeps_null_for_what_was_not_read() {
    let mut read = full_read();
    let result = read.result.as_mut().unwrap();
    result.passport_no = None;
    result.score = 4;
    result.visual_zone = VisualZone::Differs;
    read.auto_acceptable = false;
    let a = send(
        state(Some(FakeReader::new(Ok(read))), &[]),
        Req::teller_jpeg(),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "body: {}", a.body);
    assert_eq!(a.body["fields"]["passportNo"], Value::Null);
    assert_eq!(a.body["fields"]["name"], "ANNA MARIA ERIKSSON");
    assert_eq!(a.body["fields"]["passportExpiry"], "2012-04-15");
    assert_eq!(a.body["score"], 4);
    assert_eq!(a.body["visualZone"], "differs");
    assert_eq!(a.body["autoAcceptable"], false);
}

// ── P4–P7: who, and from where ──────────────────────────────────────────

#[actix_web::test]
async fn p4_nobody_signed_in_is_401_sign_in_and_the_reader_is_never_called() {
    let reader = FakeReader::new(Ok(full_read()));
    let mut req = Req::teller_jpeg();
    req.who = "";
    let a = send(state(Some(reader.clone()), &[]), req).await;
    assert_eq!(a.status, StatusCode::UNAUTHORIZED, "body: {}", a.body);
    assert_eq!(a.code(), "signIn");
    assert_eq!(reader.calls(), 0);
}

#[actix_web::test]
async fn p5_a_role_without_exchange_transact_is_refused_and_the_reader_is_never_called() {
    use xc_auth::domain::access::v3_roles::blueprint_roles;
    use xc_auth::domain::access::{Permission, RoleGrants};

    assert!(
        Role::STAFF.iter().any(|&r| {
            !blueprint_roles(r)
                .iter()
                .any(|&br| RoleGrants::seed(br).contains(&Permission::ExchangeTransact))
        }),
        "§11.2 must leave some staff role without ExchangeTransact, or this test refuses nobody"
    );
    for role in Role::STAFF.into_iter().filter(|&r| {
        !blueprint_roles(r)
            .iter()
            .any(|&br| RoleGrants::seed(br).contains(&Permission::ExchangeTransact))
    }) {
        let reader = FakeReader::new(Ok(full_read()));
        let mut req = Req::teller_jpeg();
        req.who = role.as_str();
        let a = send(state(Some(reader.clone()), &[]), req).await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "{role:?}: body: {}",
            a.body
        );
        assert_eq!(a.code(), "roleNotAllowedHere", "{role:?}");
        assert_eq!(reader.calls(), 0, "{role:?}");
    }
}

/// The owner signs in as Executive + Admin (`v3_roles.rs:13`); under §11.2 (`RoleGrants::seed`),
/// `Executive` holds `ExchangeTransact`, so the read follows `/api/quotes` (same row).
#[actix_web::test]
async fn p6_the_owner_gets_the_same_refusal_as_the_desks_quote_route() {
    use xc_auth::domain::access::{Permission, Role as BpRole, RoleGrants};

    let reader = FakeReader::new(Ok(full_read()));
    let mut req = Req::teller_jpeg();
    req.who = "owner";
    req.query = "?branch=MAIN";
    let a = send(state(Some(reader.clone()), &[]), req).await;
    if [BpRole::Executive, BpRole::Admin]
        .iter()
        .any(|&r| RoleGrants::seed(r).contains(&Permission::ExchangeTransact))
    {
        assert_eq!(a.status, StatusCode::OK, "body: {}", a.body);
        assert_eq!(reader.calls(), 1);
    } else {
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "body: {}", a.body);
        assert_eq!(a.code(), "roleNotAllowedHere");
        assert_eq!(reader.calls(), 0);
    }
}

#[actix_web::test]
async fn p7_a_foreign_origin_or_no_marker_is_origin_rejected_and_the_reader_is_never_called() {
    for (origin, marker) in [
        (Some("http://evil.example"), true),
        (None, true),
        (Some(ORIGIN), false),
    ] {
        let reader = FakeReader::new(Ok(full_read()));
        let mut req = Req::teller_jpeg();
        req.origin = origin;
        req.marker = marker;
        let a = send(state(Some(reader.clone()), &[]), req).await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "{origin:?}/{marker}: {}",
            a.body
        );
        assert_eq!(a.code(), "originRejected", "{origin:?}/{marker}");
        assert_eq!(reader.calls(), 0, "{origin:?}/{marker}");
    }
}

// ── P8–P10: the image ───────────────────────────────────────────────────

#[actix_web::test]
async fn p8_only_jpeg_png_and_webp_are_read_and_a_pdf_has_its_own_code() {
    for (ct, status, code) in [
        (Some("application/pdf"), 415, "pdfNotAccepted"),
        (Some("image/gif"), 415, "imageTypeNotAccepted"),
        (Some("text/plain"), 415, "imageTypeNotAccepted"),
        (None, 415, "imageTypeNotAccepted"),
    ] {
        let reader = FakeReader::new(Ok(full_read()));
        let mut req = Req::teller_jpeg();
        req.content_type = ct;
        let a = send(state(Some(reader.clone()), &[]), req).await;
        assert_eq!(a.status.as_u16(), status, "{ct:?}: {}", a.body);
        assert_eq!(a.code(), code, "{ct:?}");
        assert_eq!(reader.calls(), 0, "{ct:?}");
    }
    for (ct, mime) in [
        ("image/png", ImageMime::Png),
        ("image/webp", ImageMime::Webp),
        ("image/PNG; charset=x", ImageMime::Png),
        (" image/jpeg ", ImageMime::Jpeg),
    ] {
        let reader = FakeReader::new(Ok(full_read()));
        let mut req = Req::teller_jpeg();
        req.content_type = Some(ct);
        let a = send(state(Some(reader.clone()), &[]), req).await;
        assert_eq!(a.status, StatusCode::OK, "{ct}: {}", a.body);
        assert_eq!(reader.seen.lock().unwrap()[0].0, mime, "{ct}");
    }
}

#[actix_web::test]
async fn p9_a_body_over_8_mib_is_413_and_exactly_8_mib_is_read() {
    assert_eq!(MAX_IMAGE_BYTES, 8 * 1024 * 1024);

    let reader = FakeReader::new(Ok(full_read()));
    let mut req = Req::teller_jpeg();
    req.body = vec![0x5a; MAX_IMAGE_BYTES + 1];
    let a = send(state(Some(reader.clone()), &[]), req).await;
    assert_eq!(a.status, StatusCode::PAYLOAD_TOO_LARGE, "body: {}", a.body);
    assert_eq!(a.code(), "imageTooLarge");
    assert_eq!(reader.calls(), 0);

    let reader = FakeReader::new(Ok(full_read()));
    let mut req = Req::teller_jpeg();
    req.body = vec![0x5a; MAX_IMAGE_BYTES];
    let a = send(state(Some(reader.clone()), &[]), req).await;
    assert_eq!(a.status, StatusCode::OK, "body: {}", a.body);
    assert_eq!(reader.seen.lock().unwrap()[0].1.len(), MAX_IMAGE_BYTES);
}

#[actix_web::test]
async fn p10_an_empty_body_is_image_invalid_and_the_reader_is_never_called() {
    let reader = FakeReader::new(Ok(full_read()));
    let mut req = Req::teller_jpeg();
    req.body = Vec::new();
    let a = send(state(Some(reader.clone()), &[]), req).await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "body: {}", a.body);
    assert_eq!(a.code(), "imageInvalid");
    assert_eq!(a.body["error"]["field"], "image");
    assert_eq!(reader.calls(), 0);
}

// ── P11–P12: Recognize's failures ───────────────────────────────────────

#[actix_web::test]
async fn p11_every_read_error_has_its_status_and_code() {
    for (err, status, code) in [
        (ReadError::NotConfigured, 503, "passportReaderNotConfigured"),
        (
            ReadError::Busy {
                retry_after_secs: 7,
            },
            429,
            "passportReaderBusy",
        ),
        (ReadError::Failed, 502, "passportReaderFailed"),
        (ReadError::Timeout, 504, "passportReaderTimeout"),
        (ReadError::ImageInvalid, 400, "imageInvalid"),
        (ReadError::ImageTooLarge, 413, "imageTooLarge"),
        (ReadError::PdfNotAccepted, 415, "pdfNotAccepted"),
        (ReadError::Internal, 500, "serverError"),
    ] {
        let a = send(
            state(Some(FakeReader::new(Err(err.clone()))), &[]),
            Req::teller_jpeg(),
        )
        .await;
        assert_eq!(a.status.as_u16(), status, "{err:?}: {}", a.body);
        assert_eq!(a.code(), code, "{err:?}");
        assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{err:?}");
        if let ReadError::Busy { .. } = err {
            assert_eq!(a.retry_after.as_deref(), Some("7"));
        }
        if let ReadError::ImageInvalid = err {
            assert_eq!(a.body["error"]["field"], "image");
        }
    }
}

#[actix_web::test]
async fn p12_a_failed_read_is_not_retried() {
    for err in [
        ReadError::Failed,
        ReadError::Timeout,
        ReadError::Busy {
            retry_after_secs: 1,
        },
    ] {
        let reader = FakeReader::new(Err(err.clone()));
        let _ = send(state(Some(reader.clone()), &[]), Req::teller_jpeg()).await;
        assert_eq!(
            reader.calls(),
            1,
            "{err:?}: one desk read, one Recognize call"
        );
    }
}

// ── P13: no configuration ───────────────────────────────────────────────

#[actix_web::test]
async fn p13_without_a_usable_configuration_the_read_is_503_not_configured() {
    let unset: &[(&str, &str)] = &[];
    let claude: &[(&str, &str)] = &[
        ("XC_RECOGNIZE_BASE_URL", "http://127.0.0.1:9"),
        ("XC_RECOGNIZE_TOKEN", "t0k3n"),
        ("XC_RECOGNIZE_PROVIDER", "claude"),
    ];
    let half: &[(&str, &str)] = &[("XC_RECOGNIZE_BASE_URL", "http://127.0.0.1:9")];
    for env in [unset, claude, half] {
        let a = send(state(None, env), Req::teller_jpeg()).await;
        assert_eq!(
            a.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{env:?}: {}",
            a.body
        );
        assert_eq!(a.code(), "passportReaderNotConfigured", "{env:?}");
    }
}
