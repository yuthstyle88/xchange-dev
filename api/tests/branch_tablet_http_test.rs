//! BT-4 (design `docs/design/blueprint/kiosk-shared-phone.md` §7.3 T-D3, T-D5, T-D6; §7.7;
//! §7.8 rows T1–T8, T11): the branch tablet's three routes through the server's whole route
//! table — `POST /api/branch-tablets` (staff), `POST /api/branch-tablet/pair` and
//! `GET /api/branch-tablet/me` — and the cookie they write.
//!
//! Written before the implementation by the BE lane's skeleton (BT-0). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! `[required]` wire, beyond §7.7's table:
//! - `list` ⇒ `200 {tablets: [{tabletId, branchId, name, state, lastSeenAt, …}]}`, never a code
//!   or a hash;
//! - refusals of `pair` are `{"error": "PAIRING_CODE_INVALID" | "RATE_LIMITED" | "ORIGIN_REJECTED"}`,
//!   as the drawer's `pair` and `device-setup` answer them;
//! - the staff route refuses a branch the caller is not scoped to with `403`, as
//!   `device_setup_branch_scope_test.rs` pins for agents.
//!
//! Callers are given by the `x-test-as` header, as in `device_setup_branch_scope_test.rs`. Needs
//! `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test
//! works in a schema of its own.

mod common;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const STAFF: &str = "/api/branch-tablets";
const PAIR: &str = "/api/branch-tablet/pair";
const ME: &str = "/api/branch-tablet/me";
const CLAIM: &str = "/api/device-agent/claim";
const MAIN: &str = "MAIN";
const B9: &str = "B9";
const COOKIE: &str = "__Host-xc_tablet";
const ATTRIBUTES: &str = "Path=/; HttpOnly; Secure; SameSite=Strict";
const MEMBER: &str = "b7000000-0000-4000-8000-000000000001";
const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// An ADMIN for the whole organisation (holds `SystemSettings`).
const ORG_ADMIN: &str = "org-admin;ADMIN,*";
/// An ADMIN whose only role is tied to branch B9.
const B9_ADMIN: &str = "b9-admin;ADMIN,B9";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn code_shape(code: &str) -> bool {
    code.len() == 11
        && code.as_bytes()[5] == b'-'
        && code
            .chars()
            .enumerate()
            .all(|(i, c)| i == 5 || CROCKFORD.contains(c))
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = xc_testdb::unique_schema_name("bthttp");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        for b in [MAIN, B9] {
            sqlx::query(
                "INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(b)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("branch {b}: {e}"));
        }
        Self { pool, schema }
    }

    async fn finish(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop the test schema");
        self.pool.close().await;
    }

    async fn sql(&self, statement: &str) {
        sqlx::query(AssertSqlSafe(statement.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }

    async fn count(&self, statement: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(statement.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"))
    }

    /// One column of one tablet, as text.
    async fn tablet(&self, id: &str, column: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT {column}::text FROM branch_tablets WHERE id = $1::uuid"
        )))
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read {column} of {id}: {e}"))
    }

    /// Every column of one tablet, as text, for "nothing changed".
    async fn whole_row(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT row_to_json(t)::text FROM branch_tablets t WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("read tablet {id}: {e}"))
    }
}

// ------------------------------------------------------------ callers

/// `<id>;<ROLE>,<branch or *>;…` → a Blueprint user of `ORG`.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.split(';');
    let id = parts.next().filter(|s| !s.is_empty())?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = parts
        .map(|p| {
            let mut f = p.split(',');
            let role = Role::parse(f.next().expect("a role")).expect("a known role");
            let branch = f.next().filter(|b| *b != "*").map(BranchId::new);
            ScopedRole::new(role, branch, from, None)
        })
        .collect();
    Some(User::new(id, ORG, roles, None))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Call {
    method: Method,
    path: String,
    body: Option<Value>,
    headers: Vec<(String, String)>,
    same_origin: bool,
}

impl Call {
    fn post(path: &str, body: Value) -> Self {
        Self {
            method: Method::POST,
            path: path.to_owned(),
            body: Some(body),
            headers: vec![("x-real-ip".to_owned(), "203.0.113.30".to_owned())],
            same_origin: true,
        }
    }

    fn get(path: &str) -> Self {
        Self {
            method: Method::GET,
            path: path.to_owned(),
            body: None,
            headers: vec![
                ("x-real-ip".to_owned(), "203.0.113.30".to_owned()),
                ("user-agent".to_owned(), "Mozilla/5.0 (iPad)".to_owned()),
            ],
            same_origin: false,
        }
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.retain(|(n, _)| n != name);
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    fn cross_site(mut self) -> Self {
        self.same_origin = false;
        self
    }

    fn tablet(self, token: &str) -> Self {
        self.header("cookie", &format!("{COOKIE}={token}"))
    }
}

struct Answer {
    status: u16,
    text: String,
    body: Value,
    headers: BTreeMap<String, String>,
    set_cookies: Vec<String>,
}

impl Answer {
    fn error(&self) -> &str {
        self.body["error"].as_str().unwrap_or("")
    }

    /// The `Set-Cookie` lines for the tablet cookie.
    fn tablet_cookies(&self) -> Vec<&str> {
        self.set_cookies
            .iter()
            .map(String::as_str)
            .filter(|c| c.starts_with(&format!("{COOKIE}=")))
            .collect()
    }

    /// The token of the one tablet cookie this answer sets, checking its attributes (T-D3).
    fn token(&self) -> String {
        let lines = self.tablet_cookies();
        assert_eq!(lines.len(), 1, "one {COOKIE} line: {:?}", self.set_cookies);
        let line = lines[0];
        let rest = line.strip_prefix(&format!("{COOKIE}=")).expect("the name");
        let (token, attributes) = rest.split_once("; ").expect("attributes");
        assert_eq!(
            attributes,
            format!("{ATTRIBUTES}; Max-Age=34560000"),
            "the cookie's attributes: {line}"
        );
        let hex = token
            .strip_prefix("xcbt_")
            .unwrap_or_else(|| panic!("an xcbt_ token: {line}"));
        assert!(
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{token}"
        );
        token.to_owned()
    }

    fn clears_the_cookie(&self) -> bool {
        self.tablet_cookies() == vec![format!("{COOKIE}=; {ATTRIBUTES}; Max-Age=0").as_str()]
    }
}

async fn send(room: &Room, who: Option<&str>, call: Call) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(call.method)
        .uri(&call.path)
        .insert_header((header::HOST, HOST));
    if call.same_origin {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN));
    }
    if let Some(who) = who {
        req = req.insert_header((AS, who.to_owned()));
    }
    for (name, value) in &call.headers {
        req = req.insert_header((name.as_str(), value.as_str()));
    }
    if let Some(body) = &call.body {
        req = req
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status().as_u16();
    let mut headers = BTreeMap::new();
    for (name, value) in resp.headers() {
        if name != header::SET_COOKIE {
            headers.insert(
                name.as_str().to_owned(),
                value.to_str().unwrap_or("").to_owned(),
            );
        }
    }
    let set_cookies = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .collect();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer {
        status,
        text,
        body,
        headers,
        set_cookies,
    }
}

async fn staff(room: &Room, who: &str, action: &str, payload: Value) -> Answer {
    send(
        room,
        Some(who),
        Call::post(STAFF, json!({ "action": action, "payload": payload })),
    )
    .await
}

/// A tablet of `branch` added by the organisation-wide ADMIN: (tabletId, code).
async fn added(room: &Room, branch: &str, name: &str) -> (String, String) {
    let a = staff(
        room,
        ORG_ADMIN,
        "add",
        json!({ "branchId": branch, "name": name }),
    )
    .await;
    assert_eq!(a.status, 200, "add to {branch}: {}", a.text);
    (
        a.body["tabletId"]
            .as_str()
            .unwrap_or_else(|| panic!("tabletId: {}", a.text))
            .to_owned(),
        a.body["code"]
            .as_str()
            .unwrap_or_else(|| panic!("code: {}", a.text))
            .to_owned(),
    )
}

async fn pair(room: &Room, code: &str) -> Answer {
    send(room, None, Call::post(PAIR, json!({ "code": code }))).await
}

/// A paired tablet of `branch`: (tabletId, token).
async fn paired(room: &Room, branch: &str, name: &str) -> (String, String) {
    let (id, code) = added(room, branch, name).await;
    let a = pair(room, &code).await;
    assert_eq!(a.status, 200, "pair: {}", a.text);
    (id, a.token())
}

async fn me(room: &Room, token: Option<&str>) -> Answer {
    let call = Call::get(ME);
    let call = match token {
        Some(t) => call.tablet(t),
        None => call,
    };
    send(room, None, call).await
}

fn listed(a: &Answer) -> Vec<(String, String, String)> {
    let mut rows: Vec<(String, String, String)> = a.body["tablets"]
        .as_array()
        .unwrap_or_else(|| panic!("tablets is a list: {}", a.text))
        .iter()
        .map(|t| {
            (
                t["branchId"].as_str().expect("branchId").to_owned(),
                t["tabletId"].as_str().expect("tabletId").to_owned(),
                t["state"].as_str().expect("state").to_owned(),
            )
        })
        .collect();
    rows.sort();
    rows
}

// ------------------------------------------------------------ T1, T2

/// T1: `add` answers a code `XXXXX-XXXXX` valid 10 minutes, once; only its hash is stored; the
/// tablet is listed `PENDING` with no code.
#[actix_web::test]
async fn t1_add_answers_the_code_once_and_stores_only_its_hash() {
    let room = Room::start().await;
    let a = staff(
        &room,
        ORG_ADMIN,
        "add",
        json!({ "branchId": MAIN, "name": "front tablet" }),
    )
    .await;
    assert_eq!(a.status, 200, "{}", a.text);
    let keys: Vec<&String> = a.body.as_object().expect("an object").keys().collect();
    assert_eq!(
        keys,
        vec!["code", "codeExpiresAt", "tabletId"],
        "{}",
        a.text
    );
    let id = a.body["tabletId"].as_str().unwrap().to_owned();
    let code = a.body["code"].as_str().unwrap().to_owned();
    assert!(code_shape(&code), "{code}");
    assert_eq!(
        room.tablet(&id, "pair_code_hash").await,
        Some(sha256_hex(&code.replace('-', ""))),
        "only the code's hash"
    );
    assert_eq!(
        room.count(&format!(
            "SELECT count(*) FROM branch_tablets WHERE id = '{id}' \
             AND pair_code_expires_at BETWEEN now() + interval '9 minutes' AND now() + interval '10 minutes 1 second'"
        ))
        .await,
        1,
        "valid 10 minutes"
    );

    let list = staff(&room, ORG_ADMIN, "list", json!({})).await;
    assert_eq!(list.status, 200, "{}", list.text);
    assert_eq!(
        listed(&list),
        vec![(MAIN.to_owned(), id.clone(), "PENDING".to_owned())]
    );
    assert_eq!(list.body["tablets"][0]["name"], json!("front tablet"));
    assert!(
        !list.text.contains(&code) && !list.text.contains("Hash"),
        "{}",
        list.text
    );
    room.finish().await;
}

/// T2: a live code pairs: `200 {tabletId, branchCode, name}`, the token only in an `HttpOnly`
/// cookie, never in the body; the code is used up and the tablet is `PAIRED`; `me` answers it.
#[actix_web::test]
async fn t2_pairing_sets_the_cookie_and_answers_no_token() {
    let room = Room::start().await;
    let (id, code) = added(&room, MAIN, "front tablet").await;
    room.sql(&format!(
        "UPDATE branch_tablets SET pair_code_expires_at = now() + interval '1 minute' WHERE id = '{id}'"
    ))
    .await;
    let a = pair(&room, &code.to_lowercase()).await;
    assert_eq!(a.status, 200, "{}", a.text);
    assert_eq!(
        a.body,
        json!({ "tabletId": id, "branchCode": MAIN, "name": "front tablet" })
    );
    assert!(
        !a.text.contains("xcbt_"),
        "no token in the body: {}",
        a.text
    );
    assert_eq!(
        a.headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    let token = a.token();
    assert_eq!(
        room.tablet(&id, "token_hash").await,
        Some(sha256_hex(&token))
    );
    assert_eq!(
        room.tablet(&id, "pair_code_hash").await,
        None,
        "the code is used up"
    );
    assert_eq!(room.tablet(&id, "pair_code_expires_at").await, None);

    let list = staff(&room, ORG_ADMIN, "list", json!({})).await;
    assert_eq!(
        listed(&list),
        vec![(MAIN.to_owned(), id.clone(), "PAIRED".to_owned())]
    );

    let m = me(&room, Some(&token)).await;
    assert_eq!(m.status, 200, "{}", m.text);
    assert_eq!(
        m.body,
        json!({ "paired": true, "tabletId": id, "branchCode": MAIN, "name": "front tablet" })
    );
    assert_eq!(
        m.headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert!(
        m.tablet_cookies().is_empty(),
        "a live cookie is left alone: {:?}",
        m.set_cookies
    );
    assert_eq!(
        room.tablet(&id, "user_agent").await.as_deref(),
        Some("Mozilla/5.0 (iPad)")
    );
    assert!(room.tablet(&id, "last_seen_at").await.is_some());
    room.finish().await;
}

/// `me` without a cookie is `{paired: false}` and clears nothing; with an unknown cookie it is
/// `{paired: false}` and clears it.
#[actix_web::test]
async fn me_without_a_live_cookie_is_unpaired() {
    let room = Room::start().await;
    let none = me(&room, None).await;
    assert_eq!(
        (none.status, &none.body),
        (200, &json!({ "paired": false })),
        "{}",
        none.text
    );
    assert_eq!(
        none.headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert!(none.tablet_cookies().is_empty());

    let junk = me(&room, Some(&format!("xcbt_{}", "0".repeat(64)))).await;
    assert_eq!(
        (junk.status, &junk.body),
        (200, &json!({ "paired": false })),
        "{}",
        junk.text
    );
    assert!(junk.clears_the_cookie(), "{:?}", junk.set_cookies);
    room.finish().await;
}

// ------------------------------------------------------------ T3, T4, T5, origin

/// T3, T4: an expired code, a used one and an unknown one are the same `400`, with no cookie.
#[actix_web::test]
async fn t3_t4_expired_used_and_unknown_codes_are_one_refusal() {
    let room = Room::start().await;
    let (expired_id, expired) = added(&room, MAIN, "old").await;
    room.sql(&format!(
        "UPDATE branch_tablets SET pair_code_expires_at = now() - interval '1 minute' WHERE id = '{expired_id}'"
    ))
    .await;
    let (_, used) = added(&room, MAIN, "used").await;
    assert_eq!(pair(&room, &used).await.status, 200);

    for (label, code) in [
        ("expired", expired.as_str()),
        ("used", used.as_str()),
        ("unknown", "00000-00000"),
    ] {
        let a = pair(&room, code).await;
        assert_eq!(
            (a.status, &a.body),
            (400, &json!({ "error": "PAIRING_CODE_INVALID" })),
            "{label}: {}",
            a.text
        );
        assert!(a.tablet_cookies().is_empty(), "{label}: no cookie");
    }
    assert_eq!(room.tablet(&expired_id, "paired_at").await, None);
    room.finish().await;
}

/// T5: the 11th pair attempt from one address in 15 minutes is `429`, even with a live code;
/// another address pairs.
#[actix_web::test]
async fn t5_the_eleventh_attempt_from_one_address_is_rate_limited() {
    let room = Room::start().await;
    let (_, code) = added(&room, MAIN, "front tablet").await;
    for i in 0..10 {
        let a = send(
            &room,
            None,
            Call::post(PAIR, json!({ "code": "00000-00000" })).header("x-real-ip", "198.51.100.9"),
        )
        .await;
        assert_eq!(a.status, 400, "attempt {}: {}", i + 1, a.text);
    }
    let limited = send(
        &room,
        None,
        Call::post(PAIR, json!({ "code": code })).header("x-real-ip", "198.51.100.9"),
    )
    .await;
    assert_eq!(
        (limited.status, limited.error()),
        (429, "RATE_LIMITED"),
        "{}",
        limited.text
    );
    assert!(limited.tablet_cookies().is_empty());
    let other = send(
        &room,
        None,
        Call::post(PAIR, json!({ "code": code })).header("x-real-ip", "198.51.100.10"),
    )
    .await;
    assert_eq!(other.status, 200, "another address: {}", other.text);
    room.finish().await;
}

/// A cross-origin pair is `400 ORIGIN_REJECTED`, sets no cookie and does not use the code.
#[actix_web::test]
async fn a_cross_origin_pair_is_refused_and_leaves_the_code() {
    let room = Room::start().await;
    let (id, code) = added(&room, MAIN, "front tablet").await;
    let a = send(
        &room,
        None,
        Call::post(PAIR, json!({ "code": code })).cross_site(),
    )
    .await;
    assert_eq!(
        (a.status, a.error()),
        (400, "ORIGIN_REJECTED"),
        "{}",
        a.text
    );
    assert!(a.tablet_cookies().is_empty());
    assert_eq!(room.tablet(&id, "paired_at").await, None);
    assert_eq!(pair(&room, &code).await.status, 200, "the code still pairs");
    room.finish().await;
}

// ------------------------------------------------------------ T6, T7, T8

/// T6: staff `revoke` ⇒ `{saved: true}`; the old cookie's `me` is `{paired: false}` with a
/// clearing cookie; the row keeps its hashes; the bound customer session is deleted.
#[actix_web::test]
async fn t6_revoke_unpairs_the_tablet_and_ends_its_customer_session() {
    let room = Room::start().await;
    let (id, token) = paired(&room, MAIN, "front tablet").await;
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2::uuid, 'Member', 'ACTIVE', 'test')",
    )
    .bind(MEMBER)
    .bind(ORG)
    .execute(&room.pool)
    .await
    .expect("a member");
    let mut conn = room.pool.acquire().await.expect("a connection");
    let session = xc_testdb::sessions::seed_session(
        &mut conn,
        MEMBER,
        None,
        Utc::now() + chrono::Duration::hours(1),
    )
    .await
    .expect("a session");
    drop(conn);
    let hash = sha256_hex(&session);
    room.sql(&format!(
        "UPDATE branch_tablets SET customer_session_hash = '{hash}', customer_user_id = '{MEMBER}' WHERE id = '{id}'"
    ))
    .await;

    let r = staff(&room, ORG_ADMIN, "revoke", json!({ "tabletId": id })).await;
    assert_eq!(
        (r.status, &r.body),
        (200, &json!({ "saved": true })),
        "{}",
        r.text
    );

    let m = me(&room, Some(&token)).await;
    assert_eq!(
        (m.status, &m.body),
        (200, &json!({ "paired": false })),
        "{}",
        m.text
    );
    assert!(m.clears_the_cookie(), "{:?}", m.set_cookies);
    assert_eq!(
        room.tablet(&id, "token_hash").await,
        Some(sha256_hex(&token)),
        "the hash stays"
    );
    assert!(room.tablet(&id, "revoked_at").await.is_some());
    assert_eq!(
        room.count(&format!(
            "SELECT count(*) FROM user_sessions WHERE hash = '{hash}'"
        ))
        .await,
        0,
        "the bound session is deleted"
    );
    let list = staff(&room, ORG_ADMIN, "list", json!({})).await;
    assert_eq!(
        listed(&list),
        vec![(MAIN.to_owned(), id, "REVOKED".to_owned())]
    );
    room.finish().await;
}

/// T7: staff `newCode` answers a new code; pairing with it sets a new cookie that works, and the
/// old cookie is `{paired: false}`.
#[actix_web::test]
async fn t7_a_new_code_re_pairs_and_the_old_cookie_stops_working() {
    let room = Room::start().await;
    let (id, old) = paired(&room, MAIN, "front tablet").await;
    let n = staff(&room, ORG_ADMIN, "newCode", json!({ "tabletId": id })).await;
    assert_eq!(n.status, 200, "{}", n.text);
    assert_eq!(n.body["tabletId"], json!(id));
    let code = n.body["code"].as_str().expect("a code").to_owned();
    assert!(code_shape(&code));
    assert!(n.body["codeExpiresAt"].is_string(), "{}", n.text);

    let a = pair(&room, &code).await;
    assert_eq!(a.status, 200, "{}", a.text);
    let new = a.token();
    assert_ne!(new, old);
    assert_eq!(me(&room, Some(&new)).await.body["paired"], json!(true));
    let stale = me(&room, Some(&old)).await;
    assert_eq!(stale.body, json!({ "paired": false }));
    assert!(stale.clears_the_cookie());
    room.finish().await;
}

/// T8: an ADMIN scoped to B9 cannot add to MAIN, re-pair or revoke MAIN's tablet, and does not
/// see it in `list`; nothing is changed.
#[actix_web::test]
async fn t8_a_branch_admin_cannot_reach_another_branchs_tablet() {
    let room = Room::start().await;
    let (main_id, _) = paired(&room, MAIN, "main tablet").await;
    let before = room.whole_row(&main_id).await;
    let mine = staff(
        &room,
        B9_ADMIN,
        "add",
        json!({ "branchId": B9, "name": "b9 tablet" }),
    )
    .await;
    assert_eq!(mine.status, 200, "B9's own: {}", mine.text);
    let b9_id = mine.body["tabletId"].as_str().unwrap().to_owned();

    let add = staff(
        &room,
        B9_ADMIN,
        "add",
        json!({ "branchId": MAIN, "name": "rogue" }),
    )
    .await;
    assert_eq!(add.status, 403, "{}", add.text);
    for action in ["newCode", "revoke"] {
        let a = staff(&room, B9_ADMIN, action, json!({ "tabletId": main_id })).await;
        assert_eq!(a.status, 403, "{action}: {}", a.text);
        assert!(a.body.get("code").is_none(), "{action}: {}", a.text);
    }
    assert_eq!(
        room.whole_row(&main_id).await,
        before,
        "MAIN's tablet is unchanged"
    );
    assert_eq!(
        room.count("SELECT count(*) FROM branch_tablets WHERE branch_id = 'MAIN'")
            .await,
        1
    );

    let list = staff(&room, B9_ADMIN, "list", json!({})).await;
    assert_eq!(list.status, 200, "{}", list.text);
    assert_eq!(
        listed(&list),
        vec![(B9.to_owned(), b9_id, "PENDING".to_owned())]
    );
    let no_one = send(
        &room,
        None,
        Call::post(STAFF, json!({ "action": "list", "payload": {} })),
    )
    .await;
    assert!(
        no_one.status == 401 || no_one.status == 403,
        "no session: {}",
        no_one.text
    );
    room.finish().await;
}

// ------------------------------------------------------------ T11

/// T11: a tablet token sent as a bearer token to the drawer's claim is `401`, as any unknown
/// token.
#[actix_web::test]
async fn t11_a_tablet_token_is_not_a_drawer_token() {
    let room = Room::start().await;
    let (_, token) = paired(&room, MAIN, "front tablet").await;
    let claim = json!({ "waitMs": 0, "version": "1.0.0", "build": "abc1234", "os": "macos-14.6-arm64",
                        "host": "counter-1.local", "drawer": { "transport": "TCP", "target": "192.168.1.50:9100" } });
    let a = send(
        &room,
        None,
        Call::post(CLAIM, claim)
            .cross_site()
            .header("authorization", &format!("Bearer {token}")),
    )
    .await;
    assert_eq!(a.status, 401, "{}", a.text);
    room.finish().await;
}
