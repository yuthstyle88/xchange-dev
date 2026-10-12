//! Specification tests for T1b: the one route table and `bootstrap::connect`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - Every handler file under `presentation/http/` has a `configure`, and
//!   `presentation::http::configure` is the one function that registers them
//!   all; `bootstrap::configure` adds the state in front, then the page routes
//!   (`presentation::pages::configure`), and `pages::fallback` as the default
//!   service behind them — today a plain 404.
//! - `bootstrap::connect` opens the pool and applies the migrations without
//!   binding a port, so a sub-command can use the database; `Application::build`
//!   goes on doing both.
//!
//! The database tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database
//! whose name ends in `_test`, a role that may create databases). Without it
//! they FAIL; none skips.

mod common;
mod routes_common;

use std::time::{Duration, Instant};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, HttpResponse, test, web};
use common::{FakeDatabase, closed_port, settings, state_with};
use routes_common::{ScratchDatabase, http_get, migration_files, settings_without_a_database};
use sqlx::PgPool;
use xchange::bootstrap::{self, AppState, Application};
use xchange::presentation::{http, pages};

/// Longest a refused connection may take to become an error: the pool's
/// acquire timeout in these settings is 1 s; the rest is slack for a busy runner.
const FAILS_WITHIN: Duration = Duration::from_secs(20);

// ------------------------------------------------------------ the route table

/// What a handler module registers its routes with.
type Configure = fn(&mut web::ServiceConfig, &AppState);

/// The shape every feature ticket fills in. A missing module, a missing
/// function or another signature does not compile.
#[actix_web::test]
async fn every_handler_module_has_the_same_configure() {
    let modules: [(&str, Configure); 9] = [
        ("auth", http::auth::configure),
        ("bookings", http::bookings::configure),
        ("customers", http::customers::configure),
        ("devices", http::devices::configure),
        ("employees", http::employees::configure),
        ("files", http::files::configure),
        ("kiosk", http::kiosk::configure),
        ("reference_rates", http::reference_rates::configure),
        ("rpc", http::rpc::configure),
    ];
    assert_eq!(modules.len(), 9);
    // the table itself, and the two that keep the shape T1a gave them
    let _table: Configure = http::configure;
    let _health: fn(&mut web::ServiceConfig) = http::health::configure;
    let _whole: fn(&mut web::ServiceConfig, AppState) = bootstrap::configure;
    // the page routes have the same shape; ticket T1c fills them in
    let _pages: Configure = pages::configure;
}

#[actix_web::test]
async fn the_route_table_alone_serves_health_and_owns_no_404() {
    // `presentation::http::configure` without `bootstrap::configure` around it:
    // the health routes are part of the table; the plain-text 404 is not.
    let state = state_with(settings(&[]), FakeDatabase::Up);
    let for_table = state.clone();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(move |cfg| http::configure(cfg, &for_table)),
    )
    .await;

    let resp =
        test::call_service(&app, test::TestRequest::get().uri("/healthz").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(&test::read_body(resp).await[..], br#"{"ok":true}"#);

    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/no/such/route/ever")
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_ne!(
        &test::read_body(resp).await[..],
        b"Not found",
        "the plain-text 404 belongs to bootstrap::configure, not to the route table"
    );
}

#[actix_web::test]
async fn the_whole_application_still_answers_as_before() {
    let app =
        test::init_service(App::new().configure(|cfg| {
            bootstrap::configure(cfg, state_with(settings(&[]), FakeDatabase::Up))
        }))
        .await;

    let resp =
        test::call_service(&app, test::TestRequest::post().uri("/healthz").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(&test::read_body(resp).await[..], br#"{"ok":true}"#);

    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/health/ready").to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "ok");

    for request in [
        test::TestRequest::get().uri("/no/such/route/ever"),
        test::TestRequest::post().uri("/no/such/route/ever"),
        test::TestRequest::get().uri("/healthz/extra"),
    ] {
        let resp = test::call_service(&app, request.to_request()).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("a Content-Type header")
            .to_str()
            .unwrap()
            .to_ascii_lowercase();
        assert!(content_type.starts_with("text/plain"), "{content_type}");
        assert_eq!(&test::read_body(resp).await[..], b"Not found");
    }
}

// ------------------------------------------------------------- pages::fallback

async fn is_the_plain_404(resp: HttpResponse) {
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .expect("a Content-Type header")
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(content_type.starts_with("text/plain"), "{content_type}");
    let body = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("read the body");
    assert_eq!(&body[..], b"Not found");
}

#[actix_web::test]
async fn the_fallback_answers_the_plain_404_for_anything() {
    // The handler takes the request (T1c will look a static file up by its path)
    // and gives the response; today that is always v2's last answer, 404 "Not found".
    for request in [
        test::TestRequest::get().uri("/"),
        test::TestRequest::get().uri("/kiosk.css"),
        test::TestRequest::post().uri("/api/nothing"),
    ] {
        let req: HttpRequest = request.to_http_request();
        let resp: HttpResponse = pages::fallback(req).await;
        is_the_plain_404(resp).await;
    }
}

#[actix_web::test]
async fn the_fallback_works_as_a_default_service() {
    // exactly how `bootstrap::configure` must be able to register it
    let app = test::init_service(App::new().default_service(web::to(pages::fallback))).await;
    let resp =
        test::call_service(&app, test::TestRequest::get().uri("/anything").to_request()).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(&test::read_body(resp).await[..], b"Not found");
}

// ---------------------------------------------------------- bootstrap::connect

async fn tables(pool: &PgPool) -> (bool, i64) {
    let branch_versions: bool =
        sqlx::query_scalar("SELECT to_regclass('public.branch_versions') IS NOT NULL")
            .fetch_one(pool)
            .await
            .expect("ask for the table");
    let applied: i64 = sqlx::query_scalar(
        "SELECT CASE WHEN to_regclass('public._sqlx_migrations') IS NULL THEN 0 \
         ELSE (SELECT count(*) FROM _sqlx_migrations WHERE success) END",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    (branch_versions, applied)
}

#[actix_web::test]
async fn connect_gives_a_pool_on_a_migrated_database_and_binds_nothing() {
    let db = ScratchDatabase::create("t1b_connect").await;
    let inspect = db.inspect().await;
    assert_eq!(
        tables(&inspect).await,
        (false, 0),
        "the scratch database starts empty"
    );

    let settings = db.settings();
    let pool: PgPool = bootstrap::connect(&settings)
        .await
        .expect("connect to an empty, reachable database");

    // the migrations were applied, all of them, to the database the settings name
    assert_eq!(tables(&inspect).await, (true, migration_files()));
    // the pool it returns is a working pool into that same database
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("use the pool");
    assert_eq!(name, db.name);
    assert_eq!(
        pool.options().get_max_connections(),
        settings.database.max_connections
    );
    assert_eq!(
        pool.options().get_acquire_timeout(),
        Duration::from_secs(settings.database.acquire_timeout_secs)
    );

    // a second caller (the server after a sub-command, or a restart) finds nothing to do
    let again = bootstrap::connect(&settings)
        .await
        .expect("connect a second time");
    assert_eq!(tables(&inspect).await, (true, migration_files()));

    pool.close().await;
    again.close().await;
    inspect.close().await;
    db.drop().await;
}

#[actix_web::test]
async fn connect_fails_soon_when_nothing_listens() {
    let settings = settings_without_a_database(closed_port());
    let started = Instant::now();
    let result = bootstrap::connect(&settings).await;
    assert!(result.is_err(), "a database nobody runs was connected to");
    assert!(
        started.elapsed() < FAILS_WITHIN,
        "took {:?} to give up",
        started.elapsed()
    );
}

// --------------------------------------------------------- Application::build

#[actix_web::test]
async fn the_server_still_refuses_to_start_without_a_database() {
    let settings = settings_without_a_database(closed_port());
    let started = Instant::now();
    common::install_test_pii_keys();
    let result = Application::build(settings).await;
    assert!(result.is_err(), "the server started without a database");
    assert!(
        started.elapsed() < FAILS_WITHIN,
        "took {:?} to give up",
        started.elapsed()
    );
}

#[actix_web::test]
async fn the_server_migrates_binds_and_serves_the_route_table() {
    let db = ScratchDatabase::create("t1b_server").await;
    let inspect = db.inspect().await;

    common::install_test_pii_keys();
    let application = Application::build(db.settings())
        .await
        .expect("start on an empty, reachable database");
    let port = application.port();
    assert_ne!(
        port, 0,
        "port 0 in the settings means: report the port really bound"
    );
    assert_eq!(tables(&inspect).await, (true, migration_files()));

    let server = actix_web::rt::spawn(application.run_until_stopped());
    // blocking sockets, so off the runtime thread the server runs on
    let (health, missing) = tokio::task::spawn_blocking(move || {
        (
            http_get(port, "/healthz"),
            http_get(port, "/no/such/route/ever"),
        )
    })
    .await
    .expect("the client thread");

    assert!(health.0.contains(" 200 "), "{}", health.0);
    assert!(health.1.ends_with(r#"{"ok":true}"#), "{}", health.1);
    assert!(missing.0.contains(" 404 "), "{}", missing.0);
    assert!(missing.1.ends_with("Not found"), "{}", missing.1);

    server.abort();
    inspect.close().await;
    db.drop().await;
}
