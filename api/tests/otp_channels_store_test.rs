//! #6286 — the owner sets which OTP channels a customer may choose (`customer_app_settings.otp_channels`).
//! Pins the ticket's behaviour table T1–T6 over HTTP and, for T5, the column's CHECK by direct SQL. T7 is
//! `web/src/screens/admin/customer-app/CustomerApp.test.tsx`.
//!
//! Written before the implementation by claude/studio-claude-17 on the lane controller's order. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! What these tests assume (the implementer follows them):
//! - `PUT /api/app-settings` takes `otpChannels`: a list saves it in the order given, `null` resets it to not
//!   set, and leaving the key out keeps what is stored. `plannerDraftTtlSeconds` may be left out the same way,
//!   so each of the two saves never wipes the other (T2).
//! - A refused list answers 400 `validationFailed` with `error.field` = `otpChannels` (T5).
//! - `GET /api/auth/otp/channels` and `POST /api/auth/otp/request` read the row of the customer's organisation,
//!   `signup_organization` (`code = 'XC'`), the one the seeded `ORG` below is.
//!
//! Who may save: T4 pins today's refusal, 400 `roleNotAllowedHere` from the policy gate, which
//! `app4_planner_routes_test` also pins for TELLER, CUSTOMER and ADMIN (#6004). The ticket's table says 403.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test runs
//! in its own schema.

mod common;
mod identity_signin_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::Utc;
use identity_signin_common::{BROWSER_ADDRESS, FakeIdentity, ORIGIN, identity_env, state_with};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::extractors::app_session::AppSessionGuard;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MAIN: &str = "MAIN";
const CHALLENGE: &str = "c0ffee00-0000-4000-8000-000000006286";
const EMAIL: (&str, &str) = ("email", "customer@example.com");
const SMS: (&str, &str) = ("sms", "+66812345678");
/// The draft lifetime the customer-app migration seeds for every organisation.
const SEEDED_TTL: i32 = 604_800;

// ------------------------------------------------------------ the room

struct Room {
    pool: PgPool,
    schema: String,
    guard: Arc<AppSessionGuard>,
    identity: FakeIdentity,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = xc_testdb::unique_schema_name("otp_channels");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        common::install_test_pii_keys();
        let identity = FakeIdentity::start();
        identity.answer(
            "/auth/otp/request",
            200,
            &json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 }).to_string(),
        );
        Self {
            pool,
            schema,
            guard: Arc::new(AppSessionGuard::new(20)),
            identity,
        }
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }

    /// One request through the server's own route table, with `who` as the signed-in caller.
    async fn send(&self, who: Option<&str>, req: atest::TestRequest) -> (StatusCode, Value) {
        let state = state_with(&self.pool, &identity_env(&self.identity.url()));
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
        let guard = self.guard.clone();
        let app = atest::init_service(
            App::new()
                .app_data(web::Data::from(guard))
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = match who {
            Some(w) => req.insert_header((AS, w)),
            None => req,
        };
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// `PUT /api/app-settings` as `who`.
    async fn save(&self, who: &str, body: Value, key: &str) -> (StatusCode, Value) {
        let req = same_origin(atest::TestRequest::default().method(Method::PUT))
            .uri("/api/app-settings")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(body.to_string());
        self.send(Some(who), req).await
    }

    /// The owner's save, which must succeed.
    async fn owner_saves(&self, body: Value, key: &str) {
        let (status, answer) = self.save("EXECUTIVE", body.clone(), key).await;
        assert_eq!(status, StatusCode::OK, "the owner saves {body}: {answer}");
    }

    /// `GET /api/auth/otp/channels`, as a customer who has not signed in.
    async fn channels(&self) -> Value {
        let req = same_origin(atest::TestRequest::get()).uri("/api/auth/otp/channels");
        let (status, body) = self.send(None, req).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// `POST /api/auth/otp/request` for `(channel, destination)`; the status, the body and how many requests
    /// Identity has received so far.
    async fn otp_request(
        &self,
        (channel, destination): (&str, &str),
    ) -> (StatusCode, Value, usize) {
        let req = same_origin(atest::TestRequest::post())
            .uri("/api/auth/otp/request")
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .set_payload(json!({ "destination": destination, "channel": channel }).to_string());
        let (status, body) = self.send(None, req).await;
        (
            status,
            body,
            self.identity.received("/auth/otp/request").len(),
        )
    }

    async fn stored(&self) -> Option<Vec<String>> {
        sqlx::query_scalar(
            "SELECT otp_channels FROM customer_app_settings WHERE organization_id = $1::uuid",
        )
        .bind(ORG)
        .fetch_one(&self.pool)
        .await
        .expect("read otp_channels")
    }

    async fn ttl(&self) -> Option<i32> {
        sqlx::query_scalar(
            "SELECT planner_draft_ttl_seconds FROM customer_app_settings \
             WHERE organization_id = $1::uuid",
        )
        .bind(ORG)
        .fetch_one(&self.pool)
        .await
        .expect("read the draft lifetime")
    }

    /// Writes the column by SQL, past the service.
    async fn store(&self, channels: Option<&[&str]>) {
        let channels = channels.map(|c| c.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        sqlx::query(
            "UPDATE customer_app_settings SET otp_channels = $2 WHERE organization_id = $1::uuid",
        )
        .bind(ORG)
        .bind(channels)
        .execute(&self.pool)
        .await
        .expect("store otp_channels");
    }
}

fn same_origin(req: atest::TestRequest) -> atest::TestRequest {
    req.insert_header((header::HOST, identity_signin_common::HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", BROWSER_ADDRESS))
        .insert_header((header::ORIGIN, ORIGIN))
        .insert_header(("x-xchange-request", "1"))
}

fn user_of(spec: &str) -> Option<Actor> {
    let from = chrono::DateTime::<Utc>::UNIX_EPOCH;
    let main = || Some(BranchId::new(MAIN));
    let (role, branch, customer) = match spec {
        "EXECUTIVE" => (Role::Executive, None, None),
        "ADMIN" => (Role::Admin, None, None),
        "TELLER" => (Role::Teller, main(), None),
        "CUSTOMER" => (Role::Customer, None, Some("C-01")),
        _ => return None,
    };
    Some(
        Actor::new(
            &format!("user-{spec}"),
            ORG,
            vec![ScopedRole::new(role, branch, from, None)],
            customer,
        )
        .with_branch(main()),
    )
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = user_of(spec);
        let staff_user = user
            .as_ref()
            .filter(|u| u.roles.iter().any(|r| r.role != Role::Customer))
            .cloned();
        Box::pin(async move { test_callers::callers(user, staff_user) })
    }
}

fn the_default() -> Value {
    json!({ "channels": ["email", "sms"], "source": "default" })
}

// ------------------------------------------------------------ T1–T6

/// T1: NULL or no row answers email then SMS, source `default`, and both channels sign in. The stored `[sms]`
/// first proves the read comes from the column and not from a constant that happens to match.
#[actix_web::test]
async fn t1_not_set_or_no_row_answers_the_default() {
    let room = Room::start().await;
    room.store(Some(&["sms"])).await;
    assert_eq!(
        room.channels().await,
        json!({ "channels": ["sms"], "source": "set" }),
        "a stored value is what the route answers"
    );

    room.store(None).await;
    assert_eq!(room.channels().await, the_default(), "NULL");
    let (email, _, _) = room.otp_request(EMAIL).await;
    let (sms, _, reached) = room.otp_request(SMS).await;
    assert_eq!(
        (email, sms, reached),
        (StatusCode::OK, StatusCode::OK, 2),
        "both channels sign in while not set"
    );

    sqlx::query("DELETE FROM customer_app_settings WHERE organization_id = $1::uuid")
        .bind(ORG)
        .execute(&room.pool)
        .await
        .expect("delete the row");
    assert_eq!(room.channels().await, the_default(), "no row");
    room.finish().await;
}

/// T2: the owner saves `[sms]`. The route answers it, email sign-in is refused before Identity is called, and
/// neither save wipes the other setting.
#[actix_web::test]
async fn t2_saved_sms_only_refuses_email_before_identity() {
    let room = Room::start().await;
    room.owner_saves(json!({ "otpChannels": ["sms"] }), "t2-save")
        .await;
    assert_eq!(
        room.channels().await,
        json!({ "channels": ["sms"], "source": "set" })
    );

    let (status, body, reached) = room.otp_request(EMAIL).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "invalidRequest", "{body}");
    assert_eq!(reached, 0, "Identity is not called for a refused channel");

    let (status, body, reached) = room.otp_request(SMS).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(reached, 1, "the allowed channel reaches Identity");

    assert_eq!(
        room.ttl().await,
        Some(SEEDED_TTL),
        "saving the channels keeps the draft lifetime"
    );
    room.owner_saves(json!({ "plannerDraftTtlSeconds": 86400 }), "t2-ttl")
        .await;
    assert_eq!(room.ttl().await, Some(86400));
    assert_eq!(
        room.stored().await,
        Some(vec!["sms".to_owned()]),
        "saving the draft lifetime keeps the channels"
    );
    room.finish().await;
}

/// T3: the order the owner saved is the order stored and answered; the first is the preselected channel.
#[actix_web::test]
async fn t3_saved_order_is_kept() {
    let room = Room::start().await;
    room.owner_saves(json!({ "otpChannels": ["sms", "email"] }), "t3-save")
        .await;
    assert_eq!(
        room.channels().await,
        json!({ "channels": ["sms", "email"], "source": "set" })
    );
    assert_eq!(
        room.stored().await,
        Some(vec!["sms".to_owned(), "email".to_owned()])
    );
    room.finish().await;
}

/// T4: nobody but the owner saves. The owner's save first, so a refusal cannot pass by refusing everyone.
#[actix_web::test]
async fn t4_a_non_owner_cannot_save() {
    let room = Room::start().await;
    room.owner_saves(json!({ "otpChannels": ["sms"] }), "t4-owner")
        .await;
    for who in ["TELLER", "ADMIN", "CUSTOMER"] {
        let (status, body) = room
            .save(
                who,
                json!({ "otpChannels": ["email"] }),
                &format!("t4-{who}"),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{who}: {body}");
        assert_eq!(body["error"]["code"], "roleNotAllowedHere", "{who}: {body}");
    }
    assert_eq!(
        room.stored().await,
        Some(vec!["sms".to_owned()]),
        "a refused save writes nothing"
    );
    room.finish().await;
}

/// T5: `[]`, an unknown channel and a repeat are refused by the save, with the field named, and by the column's
/// CHECK when written past the service.
#[actix_web::test]
async fn t5_a_bad_list_is_refused_by_the_save_and_by_the_check() {
    let room = Room::start().await;
    let bad: [&[&str]; 3] = [&[], &["fax"], &["sms", "sms"]];

    sqlx::query("DELETE FROM customer_app_settings WHERE organization_id = $1::uuid")
        .bind(ORG)
        .execute(&room.pool)
        .await
        .expect("delete the row");
    let insert = |channels: Option<Vec<String>>| {
        sqlx::query(
            "INSERT INTO customer_app_settings (organization_id, otp_channels, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 'test', now())",
        )
        .bind(ORG)
        .bind(channels)
        .execute(&room.pool)
    };
    for list in bad {
        let channels = list.iter().map(|s| (*s).to_owned()).collect();
        let err = insert(Some(channels))
            .await
            .expect_err(&format!("the CHECK refuses {list:?}"));
        let code = err.as_database_error().and_then(|e| e.code());
        assert_eq!(code.as_deref(), Some("23514"), "{list:?}: {err}");
    }
    insert(Some(vec!["sms".to_owned(), "email".to_owned()]))
        .await
        .expect("the CHECK accepts a good list");
    room.store(None).await;

    for (i, list) in bad.iter().enumerate() {
        let (status, body) = room
            .save(
                "EXECUTIVE",
                json!({ "otpChannels": list }),
                &format!("t5-{i}"),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{list:?}: {body}");
        assert_eq!(
            body["error"]["code"], "validationFailed",
            "{list:?}: {body}"
        );
        assert_eq!(body["error"]["field"], "otpChannels", "{list:?}: {body}");
    }
    assert_eq!(room.stored().await, None, "a refused list writes nothing");
    room.finish().await;
}

/// T6: saving `null` goes back to not set; the draft lifetime stays.
#[actix_web::test]
async fn t6_saving_null_goes_back_to_the_default() {
    let room = Room::start().await;
    room.owner_saves(json!({ "otpChannels": ["sms"] }), "t6-set")
        .await;
    assert_eq!(room.stored().await, Some(vec!["sms".to_owned()]));

    room.owner_saves(json!({ "otpChannels": null }), "t6-reset")
        .await;
    assert_eq!(room.stored().await, None);
    assert_eq!(room.channels().await, the_default());
    assert_eq!(room.ttl().await, Some(SEEDED_TTL));
    room.finish().await;
}

/// The CHECK compares `otp_channels[1]` and `[2]`, so an array indexed from 2 must be refused too.
#[actix_web::test]
async fn check_refuses_a_duplicate_list_not_indexed_from_one() {
    let room = Room::start().await;
    sqlx::query("DELETE FROM customer_app_settings WHERE organization_id = $1::uuid")
        .bind(ORG)
        .execute(&room.pool)
        .await
        .expect("delete the row");
    let err = sqlx::query(
        "INSERT INTO customer_app_settings (organization_id, otp_channels, updated_by, updated_at) \
         VALUES ($1::uuid, '[2:3]={sms,sms}'::text[], 'test', now())",
    )
    .bind(ORG)
    .execute(&room.pool)
    .await
    .expect_err("the CHECK refuses '[2:3]={sms,sms}'");
    let code = err.as_database_error().and_then(|e| e.code());
    assert_eq!(code.as_deref(), Some("23514"), "{err}");
    room.finish().await;
}
