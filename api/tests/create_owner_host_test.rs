//! `create-owner --host` records the install's first host (`docs/design/blueprint/rate-display.md` §10).
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Root cause pinned here: on a fresh install nothing wrote `organization_hosts`, so `GET /api/public/host`
//! answered `404` and every public `/app` screen failed. Each row of §10.4 is a test:
//! - empty database + a host → one row for `XC`, and `/api/public/host` answers `200 {"org":"XC"}`;
//! - a second run (owner `ACTIVE`) refuses the owner and still leaves one row;
//! - an Identity refusal keeps the row; the retry completes the owner with one row;
//! - the host is trimmed and lower-cased; a non-default port is kept;
//! - a malformed host, or one linked to another organisation, writes nothing and never calls Identity.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::{App, test as atest};
use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::*;
use serde_json::json;
use xc_auth::application::session::SessionService;
use xc_auth::domain::identity::identity_client::IdentityClient;
use xc_auth::infrastructure::identity::HttpIdentityClient;
use xchange::bootstrap::{self, AppState};

const HOST_NAME: &str = "xc-v3.spacing.cloud";
const OWNER_NAME: &str = "Root Owner";
const OWNER_PHONE: &str = "+66810000001";
const OWNER_PASSWORD: &str = "root-password-01";
/// The organisation every migrated database is seeded with (`20261011100000_organizations.sql`).
const XC_ID: &str = "00000000-0000-0000-0000-000000000001";
const YY_ID: &str = "00000000-0000-0000-0000-0000000000b2";

/// The service as `api/src/main.rs` builds it, with Identity pointed at the fake.
fn owner_service(desk: &Desk, identity: &FakeIdentity) -> SessionService {
    let settings = state_with(&desk.pool, &identity_env(&identity.url()))
        .settings
        .identity
        .clone()
        .expect("identity settings from identity_env");
    let client: Arc<dyn IdentityClient> =
        Arc::new(HttpIdentityClient::new(settings, "x-forwarded-for"));
    SessionService::new(desk.pool.clone(), Some(client), None, None)
}

async fn create_owner(desk: &Desk, identity: &FakeIdentity, host: &str) -> Result<String, String> {
    owner_service(desk, identity)
        .create_owner(OWNER_NAME, OWNER_PHONE, OWNER_PASSWORD, host, Utc::now())
        .await
}

fn identity_registers(identity: &FakeIdentity) {
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_A }).to_string(),
    );
}

/// Every `organization_hosts` row as `(host, organization_id)`, ordered by host.
async fn host_rows(desk: &Desk) -> Vec<(String, String)> {
    sqlx::query_as("SELECT host, organization_id::text FROM organization_hosts ORDER BY host")
        .fetch_all(&desk.pool)
        .await
        .expect("read organization_hosts")
}

async fn owner_status(desk: &Desk) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM users WHERE name = $1")
        .bind(OWNER_NAME)
        .fetch_optional(&desk.pool)
        .await
        .expect("read the owner user")
}

/// `GET /api/public/host` through the whole route table with this `Host`.
async fn public_host(desk: &Desk, host: &str) -> (StatusCode, String) {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let req = atest::TestRequest::get()
        .uri("/api/public/host")
        .insert_header(("Host", host))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let body = atest::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// §10.4 row 1: from empty, one row for XC, and the public host lookup answers 200.
#[actix_web::test]
async fn from_empty_create_owner_with_a_host_records_one_row_and_public_host_answers_200() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity_registers(&identity);
    assert_eq!(
        host_rows(&desk).await,
        vec![],
        "a migrated database has no host row"
    );

    let res = create_owner(&desk, &identity, HOST_NAME).await;
    assert!(res.is_ok(), "create_owner succeeds: {:?}", res.err());

    assert_eq!(
        host_rows(&desk).await,
        vec![(HOST_NAME.to_string(), XC_ID.to_string())],
        "exactly one row, linking the host to XC"
    );
    assert_eq!(owner_status(&desk).await.as_deref(), Some("ACTIVE"));

    let (status, body) = public_host(&desk, HOST_NAME).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).expect("a JSON body"),
        json!({ "org": "XC" })
    );

    desk.drop_schema().await;
}

/// §10.4 row 2: a second run against the ACTIVE owner refuses it and stays at one row.
#[actix_web::test]
async fn a_second_run_refuses_the_active_owner_and_stays_at_one_host_row() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity_registers(&identity);
    create_owner(&desk, &identity, HOST_NAME)
        .await
        .expect("the first run succeeds");

    let again = create_owner(&desk, &identity, HOST_NAME).await;
    let err = again.expect_err("the second run refuses the ACTIVE owner");
    assert!(err.contains("already ACTIVE"), "today's refusal: {err}");

    assert_eq!(
        host_rows(&desk).await,
        vec![(HOST_NAME.to_string(), XC_ID.to_string())],
        "still one row"
    );
    assert_eq!(
        identity.received("/auth/register").len(),
        1,
        "Identity was asked to register the owner once in total"
    );

    desk.drop_schema().await;
}

/// §10.4 row 2, an install that has an owner and no row: the re-run links the host.
#[actix_web::test]
async fn a_run_against_an_active_owner_with_no_row_links_the_host() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity_registers(&identity);
    create_owner(&desk, &identity, HOST_NAME)
        .await
        .expect("the first run succeeds");
    sqlx::query("DELETE FROM organization_hosts")
        .execute(&desk.pool)
        .await
        .expect("an install from before §10 has no row");

    let err = create_owner(&desk, &identity, HOST_NAME)
        .await
        .expect_err("the owner is ACTIVE");
    assert!(err.contains("already ACTIVE"), "{err}");

    assert_eq!(
        host_rows(&desk).await,
        vec![(HOST_NAME.to_string(), XC_ID.to_string())],
        "the host is linked even though the owner part refuses"
    );
    let (status, body) = public_host(&desk, HOST_NAME).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    desk.drop_schema().await;
}

/// §10.4 row 3: Identity refuses the first run; the row stays, the retry completes the owner.
#[actix_web::test]
async fn an_identity_refusal_keeps_the_row_and_the_retry_completes_the_owner_with_one_row() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity.answer("/auth/register", 503, "{}");

    let first = create_owner(&desk, &identity, HOST_NAME).await;
    assert!(first.is_err(), "Identity refused: {first:?}");
    assert_eq!(owner_status(&desk).await.as_deref(), Some("PENDING"));
    assert_eq!(
        host_rows(&desk).await,
        vec![(HOST_NAME.to_string(), XC_ID.to_string())],
        "the host is recorded before the Identity call"
    );

    identity_registers(&identity);
    let second = create_owner(&desk, &identity, HOST_NAME).await;
    assert!(
        second.is_ok(),
        "the retry completes the owner: {:?}",
        second.err()
    );
    assert_eq!(owner_status(&desk).await.as_deref(), Some("ACTIVE"));
    assert_eq!(
        host_rows(&desk).await,
        vec![(HOST_NAME.to_string(), XC_ID.to_string())],
        "still one row"
    );

    desk.drop_schema().await;
}

/// §10.4 rows 4 and 5: trimmed and lower-cased as `request_host()` reads it; a non-default port is kept.
#[actix_web::test]
async fn the_host_is_trimmed_and_lower_cased_and_a_non_default_port_is_kept() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity_registers(&identity);

    create_owner(&desk, &identity, "  XC-V3.Spacing.Cloud ")
        .await
        .expect("create_owner succeeds");
    // The owner is ACTIVE now; the second run only links the second host.
    let _ = create_owner(&desk, &identity, "xc-v3.spacing.cloud:8443").await;

    assert_eq!(
        host_rows(&desk).await,
        vec![
            (HOST_NAME.to_string(), XC_ID.to_string()),
            ("xc-v3.spacing.cloud:8443".to_string(), XC_ID.to_string()),
        ]
    );
    let (status, body) = public_host(&desk, HOST_NAME).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    desk.drop_schema().await;
}

/// §10.4 row 6: a malformed host writes nothing, creates no owner and never calls Identity.
#[actix_web::test]
async fn a_malformed_host_writes_nothing_and_never_calls_identity() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity_registers(&identity);

    for bad in [
        "",
        "  ",
        "https://xc-v3.spacing.cloud",
        "xc-v3.spacing.cloud/app",
        "xc v3",
        "a@b",
        "xc-v3.spacing.cloud:443",
        "xc-v3.spacing.cloud:80",
    ] {
        let err = create_owner(&desk, &identity, bad)
            .await
            .expect_err(&format!("{bad:?} is refused"));
        assert!(
            err.starts_with("host:"),
            "{bad:?}: the error names the host: {err}"
        );
    }

    assert_eq!(host_rows(&desk).await, vec![], "no row written");
    assert_eq!(owner_status(&desk).await, None, "no owner user created");
    assert_eq!(
        identity.received("/auth/register").len(),
        0,
        "Identity is never called"
    );

    desk.drop_schema().await;
}

/// §10.4 row 7: a host already linked to another organisation is refused and left as it is.
#[actix_web::test]
async fn a_host_linked_to_another_organisation_is_refused_and_left_as_it_is() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    identity_registers(&identity);
    sqlx::query(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ($1::uuid, 'YY', NULL, 'Asia/Bangkok', now())",
    )
    .bind(YY_ID)
    .execute(&desk.pool)
    .await
    .expect("a second organisation");
    sqlx::query("INSERT INTO organization_hosts (host, organization_id) VALUES ($1, $2::uuid)")
        .bind(HOST_NAME)
        .bind(YY_ID)
        .execute(&desk.pool)
        .await
        .expect("the host belongs to YY");

    let err = create_owner(&desk, &identity, HOST_NAME)
        .await
        .expect_err("the host belongs to another organisation");
    assert!(err.starts_with("host:"), "{err}");
    assert!(err.contains("another organisation"), "{err}");

    assert_eq!(
        host_rows(&desk).await,
        vec![(HOST_NAME.to_string(), YY_ID.to_string())],
        "the row still points at YY"
    );
    assert_eq!(owner_status(&desk).await, None, "no owner user created");
    assert_eq!(identity.received("/auth/register").len(), 0);

    desk.drop_schema().await;
}
