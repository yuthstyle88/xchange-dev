//! Shared by the T6b-2 tests: the server's own route table on a PostgreSQL schema and an upload
//! directory of the test's own, a fake of the pages' session reader, and the replay that holds
//! the four file routes to what v2 answered.
//! Provided with ticket T6b-2 — do not edit.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, header};
use actix_web::{Error, HttpRequest, test};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::AppState;
use xchange::presentation::pages::{SessionReader, Sessions};

use crate::common::settings;

pub const HOST: &str = "desk.example";
pub const ORIGIN: &str = "https://desk.example";
/// The header the fake reader looks at: names of the fixture's `people`, comma-separated.
pub const AS: &str = "x-test-signed-in-as";
pub const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";
pub const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── who is signed in, faked ──────────────────────────────────────────────

/// The pages' session port (ticket T1c), answered from a request header instead of cookies:
/// `owner` is the owner (actor id `owner`); any other name is looked up in `people`. A person
/// still on the first password has no session, as the real reader has none for them.
pub struct FakeSessions {
    pub people: HashMap<String, (String, Role, String, bool)>,
}

impl FakeSessions {
    pub fn from_fixture(fixture: &Value) -> Self {
        let people = fixture["people"]
            .as_object()
            .expect("people")
            .iter()
            .map(|(name, p)| {
                let role = Role::parse(p["role"].as_str().unwrap()).expect("a role");
                (
                    name.clone(),
                    (
                        p["id"].as_str().unwrap().to_owned(),
                        role,
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
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            let names = req
                .headers()
                .get(AS)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let mut sessions = Sessions {
                owner: None,
                staff: None,
            };
            for name in names.split(',').filter(|n| !n.is_empty()) {
                if name == "owner" {
                    sessions.owner = Some(Actor::owner("owner"));
                } else if let Some((id, role, branch, must_change)) = self.people.get(name) {
                    if !must_change {
                        sessions.staff =
                            Actor::staff(id.clone(), *role, BranchId::new(branch.clone()));
                    }
                } else {
                    panic!("the test signs in as {name:?}, who is not in the fixture's people");
                }
            }
            sessions
        })
    }
}

// ── the database, the directory and the app ──────────────────────────────

pub struct Desk {
    pub pool: PgPool,
    pub schema: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileRow {
    pub key: String,
    pub branch_id: String,
    pub owner_id: String,
    pub mime: String,
    pub sha256: String,
}

impl Desk {
    /// A fresh schema with `api/migrations/` applied, the branches `MAIN` (from the migrations)
    /// and `BR-2`, and an upload directory that does not exist yet.
    ///
    /// `TEST_DATABASE_URL` must name an existing database whose name ends in `_test`. When it
    /// is absent the test FAILS; nothing here skips.
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The file route tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("t6b2");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");
        sqlx::query(
            "INSERT INTO branches (id, name) VALUES ('BR-2', 'BR-2') ON CONFLICT (id) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("insert the branch BR-2");
        let dir = std::env::temp_dir().join(unique("xchange-t6b2-uploads"));
        Self { pool, schema, dir }
    }

    /// The server's state: this schema, and `files.dir` pointing at this test's directory.
    pub fn state(&self) -> AppState {
        let dir = self.dir.to_str().expect("a UTF-8 temp path");
        AppState::new(
            Arc::new(settings(&[("APP_FILES__DIR", dir)])),
            self.pool.clone(),
        )
    }

    pub async fn rows(&self) -> BTreeSet<FileRow> {
        let rows: Vec<(String, String, String, String, String)> =
            sqlx::query_as("SELECT key, branch_id, owner_id, mime, sha256 FROM files")
                .fetch_all(&self.pool)
                .await
                .expect("read files");
        rows.into_iter()
            .map(|(key, branch_id, owner_id, mime, sha256)| FileRow {
                key,
                branch_id,
                owner_id,
                mime,
                sha256,
            })
            .collect()
    }

    /// Names in the upload directory; empty when the directory does not exist.
    pub fn names_on_disk(&self) -> BTreeSet<String> {
        match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .map(|e| {
                    e.expect("a directory entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            Err(_) => BTreeSet::new(),
        }
    }

    /// Puts a file there directly — a row and its bytes — as the save of an exchange stores a
    /// receipt.
    pub async fn seed(&self, key: &str, branch: &str, owner_id: &str, mime: &str, bytes: &[u8]) {
        std::fs::create_dir_all(&self.dir).expect("create the upload directory");
        std::fs::write(self.dir.join(key), bytes).expect("write the seeded file");
        sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2, $3, $4, $5, now())",
        )
        .bind(key)
        .bind(branch)
        .bind(owner_id)
        .bind(mime)
        .bind(sha256_hex(bytes))
        .execute(&self.pool)
        .await
        .expect("insert the seeded row");
    }

    pub async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop the test schema");
        self.pool.close().await;
    }
}

type CallFn = dyn Fn(Call) -> Pin<Box<dyn Future<Output = Answer>>>;

/// The app under test with its request type hidden, so that helpers need not name it.
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
            call: Box::new(move |call| {
                let service = service.clone();
                Box::pin(async move { exchange(&*service, to_request, call).await })
            }),
        }
    }
}

/// The server's whole route table (`bootstrap::configure`) with the fake session reader. The
/// reader is registered **after** `configure`, so that it is the one found even once the real
/// reader (ticket T4c) is registered inside the route table.
#[macro_export]
macro_rules! desk_app {
    ($desk:expr, $fixture:expr) => {{
        let reader: std::sync::Arc<dyn xchange::presentation::pages::SessionReader> =
            std::sync::Arc::new($crate::file_routes_common::FakeSessions::from_fixture(
                $fixture,
            ));
        let state = $desk.state();
        $crate::file_routes_common::TestApp::new(
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

// ── one request ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Call {
    pub method: Option<Method>,
    pub path: String,
    /// Names for the fake reader.
    pub signed_in: Vec<String>,
    pub origin: Option<String>,
    pub marker: Option<String>,
    pub mime: Option<String>,
    pub body: Vec<u8>,
    /// True = the request did not come through the HTTPS proxy: no X-Forwarded-Proto header.
    pub plain_http: bool,
}

impl Call {
    pub fn get(path: &str, signed_in: &[&str]) -> Self {
        Self {
            method: Some(Method::GET),
            path: path.to_owned(),
            signed_in: signed_in.iter().map(|s| (*s).to_owned()).collect(),
            ..Self::default()
        }
    }

    /// An upload as the kept pages send it: same origin, the marker header, the file as the body.
    pub fn upload(path: &str, signed_in: &[&str], mime: &str, body: &[u8]) -> Self {
        Self {
            method: Some(Method::POST),
            path: path.to_owned(),
            signed_in: signed_in.iter().map(|s| (*s).to_owned()).collect(),
            origin: Some(ORIGIN.to_owned()),
            marker: Some("1".to_owned()),
            mime: Some(mime.to_owned()),
            body: body.to_vec(),
            plain_http: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    /// Lower-case header name → value, for the headers the fixture records.
    pub headers: BTreeMap<String, String>,
    pub bytes: Vec<u8>,
}

impl Answer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes)
            .unwrap_or_else(|e| panic!("the body is JSON ({e}): {:?}", self.text()))
    }

    pub fn error(&self) -> String {
        self.json()["error"]
            .as_str()
            .unwrap_or_else(|| panic!("an error answer: {}", self.text()))
            .to_owned()
    }

    pub fn upload_key(&self) -> String {
        assert_eq!(self.status, 200, "an accepted upload: {}", self.text());
        self.json()["uploadKey"]
            .as_str()
            .unwrap_or_else(|| panic!("an upload answer: {}", self.text()))
            .to_owned()
    }
}

pub const RECORDED_HEADERS: [&str; 6] = [
    "content-type",
    "cache-control",
    "x-content-type-options",
    "content-security-policy",
    "content-disposition",
    "location",
];

pub async fn send(app: &TestApp, call: Call) -> Answer {
    (app.call)(call).await
}

async fn exchange<S, R, B>(app: &S, to_request: fn(test::TestRequest) -> R, call: Call) -> Answer
where
    S: Service<R, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    let method = call.method.clone().unwrap_or(Method::GET);
    let mut request = test::TestRequest::default()
        .method(method.clone())
        .uri(&call.path)
        .insert_header((header::HOST, HOST));
    if !call.plain_http {
        request = request.insert_header(("x-forwarded-proto", "https"));
    }
    if !call.signed_in.is_empty() {
        request = request.insert_header((AS, call.signed_in.join(",")));
    }
    if let Some(origin) = &call.origin {
        request = request.insert_header((header::ORIGIN, origin.as_str()));
    }
    if let Some(marker) = &call.marker {
        request = request.insert_header(("x-xchange-request", marker.as_str()));
    }
    if method != Method::GET {
        if let Some(mime) = &call.mime {
            request = request.insert_header((header::CONTENT_TYPE, mime.as_str()));
        }
        request = request.set_payload(call.body.clone());
    }
    let response = test::call_service(app, to_request(request)).await;
    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for name in RECORDED_HEADERS {
        let values: Vec<&str> = response
            .headers()
            .get_all(name)
            .map(|v| v.to_str().expect("an ASCII header"))
            .collect();
        assert!(values.len() <= 1, "{name} is sent once: {values:?}");
        if let Some(value) = values.first() {
            headers.insert(name.to_owned(), (*value).to_owned());
        }
    }
    let bytes = test::read_body(response).await.to_vec();
    Answer {
        status,
        headers,
        bytes,
    }
}

// ── the v2 fixture ───────────────────────────────────────────────────────

pub fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/file_routes/v2_file_route_cases.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the fixture is JSON")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
        .collect()
}

/// `$kN` → the key v3 drew for that upload, anywhere in the text.
fn with_real_keys(text: &str, keys: &HashMap<String, String>) -> String {
    let mut out = text.to_owned();
    // longest symbols first, so that $k1 does not eat the start of $k12
    let mut symbols: Vec<&String> = keys.keys().collect();
    symbols.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for symbol in symbols {
        out = out.replace(symbol.as_str(), &keys[symbol]);
    }
    out
}

/// Replays every step of the fixture through the app, in order, and compares every answer and
/// what is stored after it.
pub async fn replay(app: &TestApp, desk: &Desk, fixture: &Value) {
    let mut keys: HashMap<String, String> = HashMap::new();
    for (index, step) in fixture["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .enumerate()
    {
        let label = format!("step {index}: {}", step["note"].as_str().unwrap());
        if let Some(seed) = step.get("seed") {
            desk.seed(
                seed["key"].as_str().unwrap(),
                seed["branchId"].as_str().unwrap(),
                seed["ownerId"].as_str().unwrap(),
                seed["mime"].as_str().unwrap(),
                seed["text"].as_str().unwrap().as_bytes(),
            )
            .await;
            continue;
        }

        let request = &step["request"];
        let method = Method::from_str(request["method"].as_str().unwrap()).unwrap();
        let mut body = Vec::new();
        if method == Method::POST {
            body = hex_bytes(request["head"].as_str().unwrap_or(""));
            body.resize(
                body.len() + request["pad"].as_u64().unwrap_or(0) as usize,
                0,
            );
        }
        let call = Call {
            method: Some(method),
            path: with_real_keys(request["path"].as_str().unwrap(), &keys),
            signed_in: request["as"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_owned())
                .collect(),
            origin: request["origin"].as_str().map(str::to_owned),
            marker: request["marker"].as_str().map(str::to_owned),
            mime: request["mime"].as_str().map(str::to_owned),
            body: body.clone(),
            plain_http: false,
        };

        let rows_before = desk.rows().await;
        let disk_before = desk.names_on_disk();
        let answer = send(app, call).await;

        // Where v3 differs from v2 on purpose, the fixture says what v3 answers.
        let want = match step.get("v3") {
            Some(v3) => v3,
            None => &step["response"],
        };
        assert_eq!(
            answer.status,
            want["status"],
            "{label}: status — v3 answered {:?}",
            answer.text()
        );

        let is_fallback_404 = want["status"] == 404 && want.get("text").is_some();
        if step.get("v3").is_none() && !is_fallback_404 {
            let mut expected = BTreeMap::new();
            for (name, value) in want["headers"].as_object().unwrap() {
                expected.insert(name.clone(), with_real_keys(value.as_str().unwrap(), &keys));
            }
            // the key of this very upload is not known before the answer
            let mut got = answer.headers.clone();
            if want["status"] == 303 {
                // only the destination of a redirect is v2's to pin
                got.retain(|name, _| name == "location");
                expected.retain(|name, _| name == "location");
            }
            assert_eq!(got, expected, "{label}: headers");
        }

        if let Some(want_json) = want.get("json") {
            let mut got = answer.json();
            if let Some(symbol) = want_json.get("uploadKey").and_then(Value::as_str) {
                let key = answer.upload_key();
                assert!(
                    key.len() == 36 && !keys.values().any(|k| k == &key),
                    "{label}: a new key of 36 characters, got {key:?}"
                );
                keys.insert(symbol.to_owned(), key);
                got["uploadKey"] = Value::String(symbol.to_owned());
            }
            assert_eq!(got, *want_json, "{label}: body");
        } else if let Some(text) = want.get("text") {
            if want["status"] != 303 {
                assert_eq!(answer.text(), text.as_str().unwrap(), "{label}: body");
            }
        } else if let Some(file) = want.get("body") {
            assert_eq!(
                answer.bytes.len() as u64,
                file["length"].as_u64().unwrap(),
                "{label}: body length"
            );
            assert_eq!(
                sha256_hex(&answer.bytes),
                file["sha256"].as_str().unwrap(),
                "{label}: the bytes handed out"
            );
        }

        // what the request left behind
        let rows_after = desk.rows().await;
        let disk_after = desk.names_on_disk();
        match step.get("stored") {
            Some(stored) => {
                let key = keys[stored["key"].as_str().unwrap()].clone();
                let new_rows: Vec<&FileRow> = rows_after.difference(&rows_before).collect();
                assert_eq!(
                    new_rows,
                    vec![&FileRow {
                        key: key.clone(),
                        branch_id: stored["branchId"].as_str().unwrap().to_owned(),
                        owner_id: stored["ownerId"].as_str().unwrap().to_owned(),
                        mime: stored["mime"].as_str().unwrap().to_owned(),
                        sha256: stored["sha256"].as_str().unwrap().to_owned(),
                    }],
                    "{label}: the one row the upload stored"
                );
                let new_files: Vec<&String> = disk_after.difference(&disk_before).collect();
                assert_eq!(
                    new_files,
                    vec![&key],
                    "{label}: the one file the upload stored"
                );
                assert_eq!(
                    std::fs::read(desk.dir.join(&key)).expect("read the stored file"),
                    body,
                    "{label}: the stored bytes"
                );
            }
            None => {
                assert_eq!(
                    rows_after, rows_before,
                    "{label}: no row is added or changed"
                );
                assert_eq!(
                    disk_after, disk_before,
                    "{label}: nothing is written to the directory"
                );
            }
        }
    }
}
