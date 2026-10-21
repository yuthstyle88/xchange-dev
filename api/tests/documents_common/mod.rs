//! Shared by the documents tests: the exchanges of the v2 fixture saved for real through the
//! shop of ticket T8b (`save_common`, used as it is), the server's own route table on that shop,
//! a fake of the pages' session reader, and the replay.
//! Provided with the documents ticket — do not edit.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, header};
use actix_web::{Error, HttpRequest, test};
use pos_infra::transactions::Transactor;
use serde_json::Value;
use xc_auth::domain::access::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xchange::application::workflows::documents::DocumentsLookup;
use xchange::application::workflows::save_transaction::SavedTransaction;
use xchange::bootstrap::AppState;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

use super::common::settings;
use super::save_common::{self, Shop};

#[allow(clippy::duplicate_mod)]
#[path = "../shift_holder_common/mod.rs"]
mod shift_holder_common;

pub const HOST: &str = "desk.example";
/// The header the fake reader looks at: names of the fixture's `people`, comma-separated.
pub const AS: &str = "x-test-signed-in-as";
pub const NO_DOCUMENTS: &str = "ไม่พบเอกสารรายการ";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";

fn bytes_of(mime: &str) -> &'static [u8] {
    match mime {
        "image/jpeg" => save_common::JPEG,
        "image/png" => PNG,
        "image/webp" => WEBP,
        "application/pdf" => save_common::PDF,
        other => panic!("the fixture names a type this helper has no bytes for: {other}"),
    }
}

pub fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/documents/v2_document_cases.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the fixture is JSON")
}

/// One exchange of the fixture, saved in v3.
#[derive(Debug, Clone)]
pub struct Saved {
    pub name: String,
    pub branch_id: String,
    pub document_ref: String,
    pub passport: String,
    pub slip: Option<String>,
    pub receipt: String,
}

fn key_of(url: &str) -> String {
    url.strip_prefix("/files/")
        .unwrap_or_else(|| panic!("a file url: {url:?}"))
        .to_owned()
}

/// A shop holding the fixture's exchanges, each saved through `SaveTransactionWorkflow`.
pub struct World {
    pub shop: Shop,
    pub saved: Vec<Saved>,
}

impl World {
    pub async fn start(fixture: &Value) -> Self {
        let shop = Shop::start().await;
        let mut saved = Vec::new();
        let mut ready: Vec<String> = Vec::new();
        for (n, exchange) in fixture["exchanges"]
            .as_array()
            .expect("exchanges")
            .iter()
            .enumerate()
        {
            let text = |key: &str| exchange[key].as_str().unwrap().to_owned();
            let (name, branch_id, mode) = (text("name"), text("branchId"), text("mode"));
            if !ready.contains(&branch_id) {
                if branch_id != save_common::MAIN {
                    shop.seed_branch(&branch_id, &branch_id).await;
                }
                shop.set_rate(&branch_id, "USD", "32.1", "33").await;
                shop.open_shift(&branch_id, &[("THB", "1000000"), ("USD", "1000")])
                    .await;
                ready.push(branch_id.clone());
            }
            let saver = shift_holder_common::teller_of_open_shift(&shop.pool, &branch_id).await;
            let mut who = save_common::person();
            who.passport_no = format!("AA{:07}", n + 1);
            who.mode = mode.clone();
            let customer = shop.reserve(&branch_id, who.clone()).await;
            let passport_mime = text("passport");
            let passport = shop
                .upload(&branch_id, &saver, &passport_mime, bytes_of(&passport_mime))
                .await;
            let mut form = save_common::form(
                &format!("submission-for-{name}-000000"),
                &customer,
                &passport,
            );
            form.customer = who;
            if let Some(slip_mime) = exchange["slip"].as_str() {
                let slip = shop
                    .upload(&branch_id, &saver, slip_mime, bytes_of(slip_mime))
                    .await;
                form.method = "Bank Transfer".to_owned();
                form.payment_ref = "TRX-1".to_owned();
                form.slip = Some(slip);
            }
            let done: SavedTransaction = shop
                .workflow
                .save(
                    &save_common::branch(&branch_id),
                    form,
                    &saver,
                    save_common::at(n as i64),
                )
                .await
                .unwrap_or_else(|e| panic!("save the exchange '{name}': {e:?}"));
            saved.push(Saved {
                name,
                branch_id,
                document_ref: done.document_ref,
                passport: key_of(&done.passport_url),
                slip: (!done.slip_url.is_empty()).then(|| key_of(&done.slip_url)),
                receipt: key_of(&done.receipt_url),
            });
        }
        Self { shop, saved }
    }

    pub fn exchange(&self, name: &str) -> &Saved {
        self.saved
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("the fixture has no exchange '{name}'"))
    }

    /// The use case on this shop's services.
    pub fn lookup(&self) -> DocumentsLookup {
        DocumentsLookup::new(
            self.shop.transactions.clone(),
            BranchService::new(BranchTx::new(Transactor::new(self.shop.pool.clone()))),
            self.shop.files.clone(),
        )
    }

    pub fn state(&self) -> AppState {
        let dir = self.shop.dir.to_str().expect("a UTF-8 temp path");
        AppState::new(
            Arc::new(settings(&[("APP_FILES__DIR", dir)])),
            self.shop.pool.clone(),
        )
    }

    /// `$name.ref`, `$name.passport`, `$name.slip`, `$name.receipt` → what v3 gave.
    pub fn with_real_values(&self, text: &str) -> String {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for s in &self.saved {
            pairs.push((format!("${}.ref", s.name), s.document_ref.clone()));
            pairs.push((format!("${}.passport", s.name), s.passport.clone()));
            pairs.push((format!("${}.receipt", s.name), s.receipt.clone()));
            if let Some(slip) = &s.slip {
                pairs.push((format!("${}.slip", s.name), slip.clone()));
            }
        }
        let mut out = text.to_owned();
        for (symbol, real) in pairs {
            out = out.replace(&symbol.replace('$', "%24"), &real);
            out = out.replace(&symbol, &real);
        }
        assert!(
            !out.contains("$") || !out.contains(".ref"),
            "a symbol was left in {out:?}"
        );
        out
    }
}

// ── who is signed in, faked ──────────────────────────────────────────────

/// The pages' session port (ticket T1c) answered from a request header: `owner` is the owner
/// (actor id `owner`), any other name is one of the fixture's `people`. A person still on the
/// first password has no session, as the real reader has none for them.
fn roles_of(text: &str) -> &'static [Role] {
    match text.trim().to_ascii_uppercase().as_str() {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown v2 role {other}"),
    }
}

pub struct FakeSessions {
    people: HashMap<String, (String, &'static [Role], String, bool)>,
}

impl FakeSessions {
    pub fn from_fixture(fixture: &Value) -> Self {
        let people = fixture["people"]
            .as_object()
            .expect("people")
            .iter()
            .map(|(name, p)| {
                (
                    name.clone(),
                    (
                        p["id"].as_str().unwrap().to_owned(),
                        roles_of(p["role"].as_str().unwrap()),
                        p["branchId"].as_str().unwrap().to_owned(),
                        p["mustChange"].as_bool().unwrap(),
                    ),
                )
            })
            .collect();
        Self { people }
    }
}

impl SessionReader for FakeSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let names = req
                .headers()
                .get(AS)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let mut owner = None;
            let mut staff = None;
            for name in names.split(',').filter(|n| !n.is_empty()) {
                if name == "owner" {
                    owner = Some(test_callers::owner("owner"));
                } else if let Some((id, roles, branch, must_change)) = self.people.get(name) {
                    if !must_change && !branch.is_empty() {
                        staff = Some(test_callers::staff(
                            id,
                            roles,
                            BranchId::new(branch.clone()),
                        ));
                    }
                } else {
                    panic!("the test signs in as {name:?}, who is not in the fixture's people");
                }
            }
            test_callers::callers(owner, staff)
        })
    }
}

// ── the app and one request ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub text: String,
}

pub const RECORDED_HEADERS: [&str; 5] = [
    "content-type",
    "cache-control",
    "x-content-type-options",
    "referrer-policy",
    "location",
];

type CallFn = dyn Fn(Method, String, Vec<String>) -> Pin<Box<dyn Future<Output = Answer>>>;

/// The app under test with its request type hidden.
pub struct TestApp {
    call: Box<CallFn>,
}

impl TestApp {
    /// `to_request` is always `|t| t.to_request()`.
    pub fn new<S, R, B>(service: S, to_request: fn(test::TestRequest) -> R) -> Self
    where
        S: Service<R, Response = ServiceResponse<B>, Error = Error> + 'static,
        R: 'static,
        B: MessageBody + 'static,
    {
        let service = std::rc::Rc::new(service);
        Self {
            call: Box::new(move |method, path, signed_in| {
                let service = service.clone();
                Box::pin(async move {
                    let mut request = test::TestRequest::default()
                        .method(method)
                        .uri(&path)
                        .insert_header((header::HOST, HOST))
                        .insert_header(("x-forwarded-proto", "https"));
                    if !signed_in.is_empty() {
                        request = request.insert_header((AS, signed_in.join(",")));
                    }
                    let response = test::call_service(&*service, to_request(request)).await;
                    let status = response.status().as_u16();
                    let mut headers = BTreeMap::new();
                    for name in RECORDED_HEADERS {
                        if let Some(value) = response.headers().get(name) {
                            headers
                                .insert(name.to_owned(), value.to_str().expect("ASCII").to_owned());
                        }
                    }
                    let bytes = test::read_body(response).await;
                    Answer {
                        status,
                        headers,
                        text: String::from_utf8_lossy(&bytes).into_owned(),
                    }
                })
            }),
        }
    }
}

pub async fn get(app: &TestApp, path: &str, signed_in: &[&str]) -> Answer {
    (app.call)(
        Method::GET,
        path.to_owned(),
        signed_in.iter().map(|s| (*s).to_owned()).collect(),
    )
    .await
}

pub async fn send(app: &TestApp, method: Method, path: &str, signed_in: &[&str]) -> Answer {
    (app.call)(
        method,
        path.to_owned(),
        signed_in.iter().map(|s| (*s).to_owned()).collect(),
    )
    .await
}

/// The server's whole route table (`bootstrap::configure`) with the fake session reader,
/// registered **after** `configure` so that it is the one found even when a real reader is
/// registered inside the route table.
#[allow(unused_macros)]
macro_rules! world_app {
    ($world:expr, $fixture:expr) => {{
        let reader: std::sync::Arc<dyn xchange::presentation::pages::SessionReader> =
            std::sync::Arc::new(documents_common::FakeSessions::from_fixture($fixture));
        let state = $world.state();
        documents_common::TestApp::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(reader)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

/// Replays the fixture's `steps` (recorded with devices on) and compares every answer.
pub async fn replay(app: &TestApp, world: &World, fixture: &Value) {
    for (index, step) in fixture["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .enumerate()
    {
        let label = format!("step {index}: {}", step["note"].as_str().unwrap());
        let request = &step["request"];
        let who: Vec<&str> = request["as"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        let method = Method::from_bytes(request["method"].as_str().unwrap().as_bytes()).unwrap();
        let path = world.with_real_values(request["path"].as_str().unwrap());
        let answer = send(app, method, &path, &who).await;
        let want = &step["response"];
        assert_eq!(
            answer.status, want["status"],
            "{label}: status — v3 answered {:?}",
            answer.text
        );

        let want_headers: BTreeMap<String, String> = want["headers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        match answer.status {
            // a redirect: where to
            302 | 303 => assert_eq!(
                answer.headers.get("location"),
                want_headers.get("location"),
                "{label}: Location"
            ),
            // a refusal in plain text: the sentence; the pages' own 404 is theirs
            404 => {
                assert_eq!(answer.text, want["text"].as_str().unwrap(), "{label}: body");
                assert!(
                    answer
                        .headers
                        .get("content-type")
                        .is_some_and(|t| t.starts_with("text/plain")),
                    "{label}: a plain-text answer, got {:?}",
                    answer.headers.get("content-type")
                );
            }
            _ => {
                assert_eq!(answer.headers, want_headers, "{label}: headers");
                assert_eq!(
                    answer.text,
                    world.with_real_values(want["text"].as_str().unwrap()),
                    "{label}: the page, character for character"
                );
            }
        }
    }
}
