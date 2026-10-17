//! RR-1 up-front tests: `RecognizeHttpReader`, `RecognizeSettings` and the wiring in `Settings` and
//! `AppState` (design `docs/design/recognize-passport-read.md` §4.4, §5, §6; behaviour table §8 rows
//! C1–C9, S1–S3, plus W1).
//!
//! Written before the implementation by the ticket author (claude/xchange-desk). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Recognize is faked by an actix server on `127.0.0.1:0` (the pattern of `upload_body_test.rs`) that
//! records each request and answers what the test set. Nothing leaves the machine. No database.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::http::StatusCode;
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use xchange::application::passport_read::{
    AiLocation, AiProvider, ImageMime, PassportImage, PassportReader, PassportResult, ReadError,
    VisualZone,
};
use xchange::bootstrap::AppState;
use xchange::config::{RecognizeConfigError, RecognizeSettings};
use xchange::infrastructure::recognize::http_reader::RecognizeHttpReader;

const TOKEN: &str = "rk_live_7f3a9c1e5b2d";
const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0xfe,
];

// ── the fake Recognize ───────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Value,
}

#[derive(Clone)]
struct Reply {
    status: u16,
    retry_after: Option<&'static str>,
    body: String,
    delay: Duration,
}

impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            retry_after: None,
            body: body.to_string(),
            delay: Duration::ZERO,
        }
    }

    fn error(status: u16, code: &str) -> Self {
        Self::json(status, json!({ "error": code }))
    }
}

struct FakeRecognize {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeRecognize {
    fn start(reply: Reply) -> Self {
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let server = HttpServer::new(move || {
            let record = record.clone();
            let reply = reply.clone();
            App::new()
                .app_data(web::PayloadConfig::new(32 * 1024 * 1024))
                .default_service(web::to(move |req: HttpRequest, body: web::Bytes| {
                    let record = record.clone();
                    let reply = reply.clone();
                    async move {
                        let header = |name: &str| {
                            req.headers()
                                .get(name)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned)
                        };
                        record.lock().unwrap().push(Seen {
                            method: req.method().to_string(),
                            path: req.path().to_owned(),
                            authorization: header("authorization"),
                            content_type: header("content-type"),
                            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
                        });
                        if !reply.delay.is_zero() {
                            actix_web::rt::time::sleep(reply.delay).await;
                        }
                        let mut res =
                            HttpResponse::build(StatusCode::from_u16(reply.status).unwrap());
                        res.content_type("application/json");
                        if let Some(ra) = reply.retry_after {
                            res.insert_header(("retry-after", ra));
                        }
                        res.body(reply.body)
                    }
                }))
        })
        .workers(1)
        .bind(("127.0.0.1", 0))
        .expect("bind a free port");
        let addr = server.addrs()[0];
        actix_web::rt::spawn(server.run());
        Self { addr, seen }
    }

    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn settings_for(
    base: &str,
    provider: Option<&str>,
    model: Option<&str>,
    timeout_ms: Option<&str>,
) -> RecognizeSettings {
    RecognizeSettings::from_raw(Some(base), Some(TOKEN), provider, model, timeout_ms)
        .expect("valid settings")
}

fn image() -> PassportImage {
    PassportImage::new(ImageMime::Jpeg, JPEG.to_vec())
}

fn full_answer(provider: &str, model: &str, location: &str) -> Value {
    json!({
        "document": "passport",
        "ai": { "provider": provider, "model": model, "location": location },
        "result": {
            "name": "ANNA MARIA ERIKSSON",
            "passport_no": "L898902C3",
            "nationality": "UTO",
            "birth_date": "1974-08-12",
            "expiry_date": "2012-04-15",
            "score": 5,
            "visual_zone": "same"
        },
        "auto_acceptable": true,
        "usage": { "input_tokens": 1812, "output_tokens": 96, "cost_baht": 0.019 }
    })
}

// ── C1–C3: the request and the 200 answers ──────────────────────────────

#[actix_web::test]
async fn c1_the_request_is_section_4_of_ai_reader_with_the_configured_provider() {
    let fake = FakeRecognize::start(Reply::json(
        200,
        full_answer("gemini", "gemini-3.1-flash-lite", "cloud"),
    ));
    let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
    reader.read(&image()).await.expect("a 200 read");
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    let s = &seen[0];
    assert_eq!(s.method, "POST");
    assert_eq!(s.path, "/v1/read");
    assert_eq!(
        s.authorization.as_deref(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert!(
        s.content_type
            .as_deref()
            .is_some_and(|c| c.starts_with("application/json")),
        "{:?}",
        s.content_type
    );
    assert_eq!(
        s.body,
        json!({
            "document": "passport",
            "ai": { "provider": "gemini" },
            "image": { "mime": "image/jpeg", "data": STANDARD.encode(JPEG) }
        }),
        "no `model` key when the model is unset"
    );

    // a configured provider and model are sent as they are; a trailing `/` on the base is dropped
    let fake = FakeRecognize::start(Reply::json(
        200,
        full_answer("ollama", "qwen2.5vl:7b", "local"),
    ));
    let base = format!("{}/", fake.base());
    let reader = RecognizeHttpReader::new(&settings_for(
        &base,
        Some("ollama"),
        Some("qwen2.5vl:7b"),
        None,
    ));
    reader
        .read(&PassportImage::new(ImageMime::Webp, JPEG.to_vec()))
        .await
        .expect("a 200 read");
    let s = &fake.seen()[0];
    assert_eq!(s.path, "/v1/read");
    assert_eq!(
        s.body["ai"],
        json!({ "provider": "ollama", "model": "qwen2.5vl:7b" })
    );
    assert_eq!(s.body["image"]["mime"], "image/webp");
}

#[actix_web::test]
async fn c2_a_full_answer_is_read_into_passport_read() {
    let fake = FakeRecognize::start(Reply::json(
        200,
        full_answer("gemini", "gemini-3.1-flash-lite", "cloud"),
    ));
    let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
    let read = reader.read(&image()).await.expect("a 200 read");
    assert_eq!(read.provider, AiProvider::Gemini);
    assert_eq!(read.model, "gemini-3.1-flash-lite");
    assert_eq!(read.location, AiLocation::Cloud);
    assert!(read.auto_acceptable);
    assert_eq!(
        read.result,
        Some(PassportResult {
            name: Some("ANNA MARIA ERIKSSON".to_owned()),
            passport_no: Some("L898902C3".to_owned()),
            nationality: Some("UTO".to_owned()),
            birth_date: Some("1974-08-12".to_owned()),
            expiry_date: Some("2012-04-15".to_owned()),
            score: 5,
            visual_zone: VisualZone::Same,
        })
    );

    // a partial result: null fields stay None, `differs` and `unchecked` are read
    for (zone, want) in [
        ("differs", VisualZone::Differs),
        ("unchecked", VisualZone::Unchecked),
    ] {
        let mut body = full_answer("gemini", "gemini-3.1-flash-lite", "cloud");
        body["result"]["passport_no"] = Value::Null;
        body["result"]["score"] = json!(4);
        body["result"]["visual_zone"] = json!(zone);
        body["auto_acceptable"] = json!(false);
        let fake = FakeRecognize::start(Reply::json(200, body));
        let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
        let read = reader.read(&image()).await.expect("a 200 read");
        let result = read.result.expect("a result");
        assert_eq!(result.passport_no, None, "{zone}");
        assert_eq!(result.score, 4, "{zone}");
        assert_eq!(result.visual_zone, want, "{zone}");
        assert!(!read.auto_acceptable, "{zone}");
    }
}

#[actix_web::test]
async fn c3_a_null_result_is_an_answer_not_an_error() {
    let mut body = full_answer("gemini", "gemini-3.1-flash-lite", "cloud");
    body["result"] = Value::Null;
    body["auto_acceptable"] = json!(false);
    let fake = FakeRecognize::start(Reply::json(200, body));
    let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
    let read = reader
        .read(&image())
        .await
        .expect("a null result is still Ok");
    assert_eq!(read.result, None);
    assert!(!read.auto_acceptable);
}

// ── C4–C9: everything that is not a 200 answer ──────────────────────────

#[actix_web::test]
async fn c4_every_recognize_error_maps_to_its_read_error() {
    let cases: Vec<(Reply, ReadError)> = vec![
        (Reply::error(400, "IMAGE_INVALID"), ReadError::ImageInvalid),
        (
            Reply::error(413, "IMAGE_TOO_LARGE"),
            ReadError::ImageTooLarge,
        ),
        (
            Reply::error(415, "PDF_NOT_ACCEPTED"),
            ReadError::PdfNotAccepted,
        ),
        (Reply::error(401, "UNAUTHORIZED"), ReadError::NotConfigured),
        (Reply::error(400, "AI_UNKNOWN"), ReadError::NotConfigured),
        (
            Reply::error(400, "AI_MODEL_NOT_ALLOWED"),
            ReadError::NotConfigured,
        ),
        (
            Reply::error(503, "AI_NOT_CONFIGURED"),
            ReadError::NotConfigured,
        ),
        (
            Reply::error(501, "DOCUMENT_NOT_SUPPORTED"),
            ReadError::NotConfigured,
        ),
        (Reply::error(504, "AI_TIMEOUT"), ReadError::Timeout),
        (Reply::error(502, "AI_FAILED"), ReadError::Failed),
        (
            Reply::json(
                502,
                json!({ "error": "AI_FAILED", "detail": "quota exceeded" }),
            ),
            ReadError::Failed,
        ),
        (Reply::error(500, "WHATEVER"), ReadError::Failed),
        (
            Reply {
                status: 503,
                retry_after: None,
                body: "<html>no healthy upstream</html>".to_owned(),
                delay: Duration::ZERO,
            },
            ReadError::Failed,
        ),
        (Reply::error(400, "AI_REQUIRED"), ReadError::Internal),
        (Reply::error(400, "DOCUMENT_UNKNOWN"), ReadError::Internal),
        (Reply::error(404, "NOT_FOUND"), ReadError::Internal),
    ];
    for (reply, want) in cases {
        let label = format!("{} {}", reply.status, reply.body);
        let fake = FakeRecognize::start(reply);
        let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
        assert_eq!(reader.read(&image()).await, Err(want), "{label}");
    }
}

#[actix_web::test]
async fn c5_busy_carries_retry_after_or_5() {
    for (header, want) in [
        (Some("7"), 7),
        (None, 5),
        (Some("soon"), 5),
        (Some("0"), 5),
        (Some("3600"), 5),
    ] {
        let mut reply = Reply::error(429, "AI_BUSY");
        reply.retry_after = header;
        let fake = FakeRecognize::start(reply);
        let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
        assert_eq!(
            reader.read(&image()).await,
            Err(ReadError::Busy {
                retry_after_secs: want
            }),
            "Retry-After {header:?}"
        );
    }
}

#[actix_web::test]
async fn c6_a_200_that_is_not_the_answer_shape_is_failed() {
    for body in [
        "not json".to_owned(),
        json!({ "document": "passport" }).to_string(),
        {
            let mut b = full_answer("gemini", "m", "cloud");
            b["result"]["visual_zone"] = json!("maybe");
            b.to_string()
        },
    ] {
        let fake = FakeRecognize::start(Reply {
            status: 200,
            retry_after: None,
            body: body.clone(),
            delay: Duration::ZERO,
        });
        let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
        assert_eq!(
            reader.read(&image()).await,
            Err(ReadError::Failed),
            "{body}"
        );
    }
}

#[actix_web::test]
async fn c7_nothing_listening_is_failed() {
    let base = format!("http://127.0.0.1:{}", common::closed_port());
    let reader = RecognizeHttpReader::new(&settings_for(&base, None, None, None));
    assert_eq!(reader.read(&image()).await, Err(ReadError::Failed));
}

#[actix_web::test]
async fn c8_no_answer_within_the_timeout_is_timeout() {
    let mut reply = Reply::json(200, full_answer("gemini", "m", "cloud"));
    reply.delay = Duration::from_secs(3);
    let fake = FakeRecognize::start(reply);
    let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, Some("300")));
    let started = std::time::Instant::now();
    assert_eq!(reader.read(&image()).await, Err(ReadError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
}

#[actix_web::test]
async fn c9_one_read_is_one_request_even_when_it_fails() {
    for reply in [
        Reply::error(502, "AI_FAILED"),
        Reply::error(429, "AI_BUSY"),
        Reply::error(503, "AI_NOT_CONFIGURED"),
    ] {
        let fake = FakeRecognize::start(reply);
        let reader = RecognizeHttpReader::new(&settings_for(&fake.base(), None, None, None));
        let _ = reader.read(&image()).await;
        assert_eq!(fake.seen().len(), 1, "no retry, no other provider");
    }
}

// ── S1–S3: settings ──────────────────────────────────────────────────────

#[test]
fn s1_from_raw_follows_the_section_6_table() {
    use RecognizeConfigError::{Invalid, Missing, Unset};
    let url = Some("https://recognize.example");
    let tok = Some(TOKEN);

    assert_eq!(
        RecognizeSettings::from_raw(None, None, None, None, None),
        Err(Unset)
    );
    assert_eq!(
        RecognizeSettings::from_raw(Some("  "), Some(""), None, None, None),
        Err(Unset)
    );
    assert_eq!(
        RecognizeSettings::from_raw(url, None, None, None, None),
        Err(Missing {
            variable: "XC_RECOGNIZE_TOKEN"
        })
    );
    assert_eq!(
        RecognizeSettings::from_raw(None, tok, None, None, None),
        Err(Missing {
            variable: "XC_RECOGNIZE_BASE_URL"
        })
    );
    assert_eq!(
        RecognizeSettings::from_raw(Some("recognize.example"), tok, None, None, None),
        Err(Invalid {
            variable: "XC_RECOGNIZE_BASE_URL"
        })
    );

    let s = RecognizeSettings::from_raw(url, tok, None, None, None).expect("defaults");
    assert_eq!(s.base_url, "https://recognize.example");
    assert_eq!(s.token(), TOKEN);
    assert_eq!(
        s.provider,
        AiProvider::Gemini,
        "gemini by default (owner 2026-10-17)"
    );
    assert_eq!(s.model, None);
    assert_eq!(s.timeout, Duration::from_millis(60_000));

    let s = RecognizeSettings::from_raw(url, tok, Some(" "), Some(" "), Some(" ")).expect("blanks");
    assert_eq!(
        (s.provider, s.model.clone(), s.timeout),
        (AiProvider::Gemini, None, Duration::from_millis(60_000))
    );

    let s = RecognizeSettings::from_raw(
        url,
        tok,
        Some("ollama"),
        Some(" qwen2.5vl:7b "),
        Some("1500"),
    )
    .expect("ollama");
    assert_eq!(s.provider, AiProvider::Ollama);
    assert_eq!(s.model.as_deref(), Some("qwen2.5vl:7b"));
    assert_eq!(s.timeout, Duration::from_millis(1500));

    for bad in ["claude", "Gemini", "openai", "gemini,ollama"] {
        assert_eq!(
            RecognizeSettings::from_raw(url, tok, Some(bad), None, None),
            Err(Invalid {
                variable: "XC_RECOGNIZE_PROVIDER"
            }),
            "{bad}"
        );
    }
    for bad in ["0", "-1", "abc", "1.5", "600001"] {
        assert_eq!(
            RecognizeSettings::from_raw(url, tok, None, None, Some(bad)),
            Err(Invalid {
                variable: "XC_RECOGNIZE_TIMEOUT_MS"
            }),
            "{bad}"
        );
    }
    assert!(RecognizeSettings::from_raw(url, tok, None, None, Some("600000")).is_ok());

    for (err, variable) in [
        (
            Missing {
                variable: "XC_RECOGNIZE_TOKEN",
            },
            "XC_RECOGNIZE_TOKEN",
        ),
        (
            Invalid {
                variable: "XC_RECOGNIZE_PROVIDER",
            },
            "XC_RECOGNIZE_PROVIDER",
        ),
    ] {
        assert!(err.to_string().contains(variable), "{err}");
    }
    let shown = RecognizeSettings::from_raw(url, tok, Some("claude"), None, None)
        .unwrap_err()
        .to_string();
    assert!(
        !shown.contains("claude") && !shown.contains(TOKEN),
        "{shown}"
    );
}

#[test]
fn s2_debug_never_shows_the_token_the_image_or_a_value_read() {
    let s = RecognizeSettings::from_raw(
        Some("https://recognize.example"),
        Some(TOKEN),
        None,
        None,
        None,
    )
    .expect("valid");
    let shown = format!("{s:?}");
    assert!(!shown.contains(TOKEN), "{shown}");

    let bytes: Vec<u8> = b"SECRETIMAGEBYTES".to_vec();
    let shown = format!("{:?}", PassportImage::new(ImageMime::Png, bytes.clone()));
    assert!(
        !shown.contains("SECRETIMAGEBYTES") && !shown.contains("83, 69, 67"),
        "{shown}"
    );
    assert!(
        shown.contains(&bytes.len().to_string()),
        "the byte count is shown: {shown}"
    );

    let read = xchange::application::passport_read::PassportRead {
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
    };
    let shown = format!("{read:?}");
    for value in ["ERIKSSON", "L898902C3", "UTO", "1974-08-12", "2012-04-15"] {
        assert!(!shown.contains(value), "{value} in {shown}");
    }
}

#[test]
fn s3_settings_read_the_five_variables() {
    let settings = common::settings(&[
        ("XC_RECOGNIZE_BASE_URL", "https://recognize.example"),
        ("XC_RECOGNIZE_TOKEN", TOKEN),
        ("XC_RECOGNIZE_PROVIDER", "gemini"),
        ("XC_RECOGNIZE_MODEL", "gemini-3.5-flash"),
        ("XC_RECOGNIZE_TIMEOUT_MS", "45000"),
    ]);
    let r = settings.recognize.as_ref().expect("configured");
    assert_eq!(r.base_url, "https://recognize.example");
    assert_eq!(r.token(), TOKEN);
    assert_eq!(r.provider, AiProvider::Gemini);
    assert_eq!(r.model.as_deref(), Some("gemini-3.5-flash"));
    assert_eq!(r.timeout, Duration::from_millis(45_000));

    let settings = common::settings(&[]);
    assert_eq!(
        settings.recognize.as_ref().err(),
        Some(&RecognizeConfigError::Unset)
    );

    let settings = common::settings(&[
        ("XC_RECOGNIZE_BASE_URL", "https://recognize.example"),
        ("XC_RECOGNIZE_TOKEN", TOKEN),
        ("XC_RECOGNIZE_PROVIDER", "claude"),
    ]);
    assert_eq!(
        settings.recognize.as_ref().err(),
        Some(&RecognizeConfigError::Invalid {
            variable: "XC_RECOGNIZE_PROVIDER"
        }),
        "a wrong value never stops the server from loading its settings"
    );
}

// ── W1: AppState wires the real reader ───────────────────────────────────

#[actix_web::test]
async fn w1_app_state_reads_through_recognize_when_configured() {
    let fake = FakeRecognize::start(Reply::json(
        200,
        full_answer("gemini", "gemini-3.1-flash-lite", "cloud"),
    ));
    let base = fake.base();
    let settings = common::settings(&[
        ("XC_RECOGNIZE_BASE_URL", base.as_str()),
        ("XC_RECOGNIZE_TOKEN", TOKEN),
    ]);
    let state = AppState::new(Arc::new(settings), common::unreachable_pool());
    assert!(state.passport_reader.configured());
    let read = state
        .passport_reader
        .read(&image())
        .await
        .expect("read through the fake Recognize");
    assert_eq!(read.provider, AiProvider::Gemini);
    assert_eq!(fake.seen().len(), 1);

    let state = AppState::new(Arc::new(common::settings(&[])), common::unreachable_pool());
    assert!(!state.passport_reader.configured());
    assert_eq!(
        state.passport_reader.read(&image()).await,
        Err(ReadError::NotConfigured)
    );
}
