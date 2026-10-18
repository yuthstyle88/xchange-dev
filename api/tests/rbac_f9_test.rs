//! RBAC F9 tests (#3155): `POST /api/users` (`userCreate`) generates a 16-character temporary
//! password through `TempPassword::generate()`, ignores any `password` field sent by the caller,
//! sets `must_change_password: true` on Identity's register request, returns `temporaryPassword`
//! once on success, never returns it on refusal, and never writes it to the server log.

mod auth_http_common;
mod common;
mod identity_signin_common;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, OnceLock};

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;

const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz";

fn is_valid_temp_password(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| ALPHABET.contains(&b))
}

async fn setup() -> (Desk, FakeIdentity, App) {
    let _ = log();
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

async fn owner_session(desk: &Desk, identity: &FakeIdentity, app: &App) -> String {
    let _ = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("ADMIN", None), ("EXECUTIVE", None)],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    app.send(Req::post(
        "/api/auth/login",
        json!({ "identifier": "+66811111111", "password": "pw-owner-01" }),
    ))
    .await
    .session()
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

struct Text(String);

impl tracing::field::Visit for Text {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let _ = write!(self.0, "{}={} ", field.name(), value);
    }
}

impl Log {
    fn push(&self, text: String) {
        self.0.lock().expect("log lock").push(text);
    }
    fn all(&self) -> String {
        self.0.lock().expect("log lock").join("\n")
    }
}

impl tracing::Subscriber for Log {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut text = Text(String::new());
        attrs.record(&mut text);
        self.push(format!("span {} {}", attrs.metadata().name(), text.0));
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut text = Text(String::new());
        values.record(&mut text);
        self.push(text.0);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut text = Text(String::new());
        event.record(&mut text);
        self.push(format!("{} {}", event.metadata().target(), text.0));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

fn log() -> &'static Log {
    static LOG: OnceLock<Log> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Log::default();
        let _ = tracing::subscriber::set_global_default(log.clone());
        log
    })
}

/// Row 1: create with no `password` field -> 200; `temporaryPassword` is 16 characters from `TempPassword`'s alphabet.
#[actix_web::test]
async fn create_with_no_password_field_returns_16_char_temp_password() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "NoPasswordField",
                    "phone": "+66812345601",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = reply.json();
    let temp = body["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword in 200 body");
    assert!(
        is_valid_temp_password(temp),
        "expected 16 chars from TempPassword alphabet, got {temp:?}"
    );

    let sent = identity.received("/auth/register");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].json()["password"], temp);

    desk.drop_schema().await;
}

/// Row 2: create with a `password` field -> 200; Identity receives the generated password, not the typed one.
#[actix_web::test]
async fn create_with_password_field_ignores_typed_password_and_sends_generated_one() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let typed = "typed-by-admin-999";
    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "TypedPasswordField",
                    "phone": "+66812345602",
                    "password": typed,
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let body = reply.json();
    let temp = body["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword in 200 body");
    assert_ne!(temp, typed);
    assert!(is_valid_temp_password(temp));

    let sent = identity.received("/auth/register");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].json()["password"], temp);
    assert_ne!(sent[0].json()["password"], typed);

    desk.drop_schema().await;
}

/// Row 3: Identity's register request has `must_change_password: true`.
#[actix_web::test]
async fn identity_register_request_sets_must_change_password_true() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "MustChangeStaff",
                    "phone": "+66812345603",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let sent = identity.received("/auth/register");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].json()["mustChangePassword"], true);

    desk.drop_schema().await;
}

/// Row 4: two creates produce two different temporary passwords (including on retry_user_id).
#[actix_web::test]
async fn two_creates_generate_two_different_temporary_passwords() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": "11111111-aaaa-4aaa-8aaa-000000000101" }).to_string(),
    );
    let reply1 = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "StaffOne",
                    "phone": "+66812345604",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;
    assert_eq!(reply1.status, 200, "{}", reply1.text);
    let temp1 = reply1.json()["temporaryPassword"]
        .as_str()
        .expect("temp1")
        .to_owned();

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": "11111111-aaaa-4aaa-8aaa-000000000102" }).to_string(),
    );
    let reply2 = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "StaffTwo",
                    "phone": "+66812345605",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;
    assert_eq!(reply2.status, 200, "{}", reply2.text);
    let temp2 = reply2.json()["temporaryPassword"]
        .as_str()
        .expect("temp2")
        .to_owned();

    assert_ne!(temp1, temp2);
    assert!(is_valid_temp_password(&temp1));
    assert!(is_valid_temp_password(&temp2));

    desk.drop_schema().await;
}

/// Row 5: the answer of a refused create (e.g. invalid phone) has no `temporaryPassword`.
#[actix_web::test]
async fn refused_create_has_no_temporary_password() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "InvalidPhoneStaff",
                    "phone": "not-a-phone",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 400, "{}", reply.text);
    let body = reply.json();
    assert!(
        body.get("temporaryPassword").is_none(),
        "refused create must not include temporaryPassword: {body}"
    );
    assert!(!reply.text.contains("temporaryPassword"));

    desk.drop_schema().await;
}

/// Row 6: the server log of a create doesn't contain the temporary password.
#[actix_web::test]
async fn server_log_of_create_does_not_contain_the_password() {
    let log = log();
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    tracing::info!(probe = "rbac-f9-log-check", "capture active");

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": "11111111-aaaa-4aaa-8aaa-000000000106" }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "UnloggedStaff",
                    "phone": "+66812345606",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    let temp = reply.json()["temporaryPassword"]
        .as_str()
        .expect("temporaryPassword")
        .to_owned();

    let captured = log.all();
    assert!(
        captured.contains("rbac-f9-log-check"),
        "tracing capture must be active: {captured}"
    );
    assert!(
        !captured.contains(&temp),
        "server log must never contain the temporary password"
    );

    desk.drop_schema().await;
}
