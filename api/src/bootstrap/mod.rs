//! Bootstrap and server initialization.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::{HttpResponse, web};
use sqlx::PgPool;

use crate::config::Settings;

#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub version: String,
    pub build: String,
    pub built_at: String,
}

impl BuildInfo {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            build: option_env!("XCHANGE_BUILD")
                .unwrap_or("unknown")
                .to_string(),
            built_at: option_env!("XCHANGE_BUILT_AT").unwrap_or("").to_string(),
        }
    }
}

pub trait DatabaseProbe: Send + Sync {
    fn ping(&self) -> Pin<Box<dyn Future<Output = bool> + '_>>;
}

impl DatabaseProbe for PgPool {
    fn ping(&self) -> Pin<Box<dyn Future<Output = bool> + '_>> {
        Box::pin(async move { sqlx::query("SELECT 1").execute(self).await.is_ok() })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: PgPool,
    pub build: BuildInfo,
    pub database_probe: Arc<dyn DatabaseProbe>,
}

impl AppState {
    pub fn new(settings: Arc<Settings>, db: PgPool) -> Self {
        let probe = Arc::new(db.clone());
        Self {
            settings,
            db,
            build: BuildInfo::current(),
            database_probe: probe,
        }
    }

    pub fn with_build(mut self, build: BuildInfo) -> Self {
        self.build = build;
        self
    }

    pub fn with_database_probe(mut self, database_probe: Arc<dyn DatabaseProbe>) -> Self {
        self.database_probe = database_probe;
        self
    }
}

pub fn configure(cfg: &mut web::ServiceConfig, state: AppState) {
    cfg.app_data(web::Data::new(state));
    crate::presentation::http::health::configure(cfg);
    cfg.default_service(web::to(|| async {
        HttpResponse::NotFound()
            .content_type("text/plain")
            .body("Not found")
    }));
}

pub struct Application {
    port: u16,
    server: actix_web::dev::Server,
}

impl Application {
    pub async fn build(settings: Settings) -> Result<Self, anyhow::Error> {
        let db_options = settings.database.with_db();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(settings.database.max_connections)
            .acquire_timeout(std::time::Duration::from_secs(
                settings.database.acquire_timeout_secs,
            ))
            .connect_with(db_options)
            .await?;

        // Eager SELECT 1 to ensure database is reachable
        sqlx::query("SELECT 1").execute(&pool).await?;

        // Run migrations
        sqlx::migrate!("./migrations").run(&pool).await?;

        let address = format!(
            "{}:{}",
            settings.application.host, settings.application.port
        );
        let listener = std::net::TcpListener::bind(&address)?;
        let port = listener.local_addr()?.port();
        let workers = settings.application.workers;

        let state = AppState::new(Arc::new(settings), pool);
        let server = actix_web::HttpServer::new(move || {
            actix_web::App::new().configure(|cfg| configure(cfg, state.clone()))
        })
        .workers(workers)
        .listen(listener)?
        .run();

        Ok(Self { port, server })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn run_until_stopped(self) -> Result<(), std::io::Error> {
        self.server.await
    }
}
