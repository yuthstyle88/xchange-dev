//! Bootstrap and server initialization.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::repositories::account_repository::Purged;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;

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
    configure_with_screens(
        cfg,
        state,
        crate::presentation::screens::ScreenTable::compiled(),
    );
}

pub fn configure_with_screens(
    cfg: &mut web::ServiceConfig,
    state: AppState,
    screens: crate::presentation::screens::ScreenTable,
) {
    cfg.app_data(web::Data::new(state.clone()));
    crate::presentation::http::configure(cfg, &state);
    screens.configure(cfg, &state);
    crate::presentation::pages::configure(cfg, &state);
    cfg.default_service(web::to(crate::presentation::pages::fallback));
}

/// Opens the pool the settings describe, checks the database answers, applies the migrations. Binds no port.
pub async fn connect(settings: &Settings) -> Result<PgPool, anyhow::Error> {
    let db_options = settings.database.with_db();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(settings.database.max_connections)
        .min_connections(settings.database.min_connections)
        .acquire_timeout(std::time::Duration::from_secs(
            settings.database.acquire_timeout_secs,
        ))
        .connect_with(db_options)
        .await?;

    // Eager SELECT 1 to ensure database is reachable
    sqlx::query("SELECT 1").execute(&pool).await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Runs `purge` once at start and then once every `every`, for ever, giving it
/// the time `clock` shows at each round.
///
/// A round that fails is logged and waited out: the next one runs at its own
/// time, and neither the timer nor the server ends because the database
/// answered badly once. A round that takes longer than `every` delays the
/// following ones instead of piling them up.
pub async fn purge_on_a_timer<C, F, Fut>(every: Duration, clock: C, mut purge: F)
where
    C: Fn() -> DateTime<Utc>,
    F: FnMut(DateTime<Utc>) -> Fut,
    Fut: Future<Output = AppResult<Purged>>,
{
    let mut rounds = tokio::time::interval(every);
    rounds.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        rounds.tick().await; // the first tick is at once
        match purge(clock()).await {
            Ok(purged) => tracing::info!(
                owner_sessions = purged.owner_sessions,
                employee_sessions = purged.employee_sessions,
                attempt_windows = purged.attempt_windows,
                "purged expired sessions and past sign-in windows"
            ),
            Err(error) => tracing::error!(
                error = %error,
                "the purge of expired sessions failed; the next round will try again"
            ),
        }
    }
}

pub struct Application {
    port: u16,
    server: actix_web::dev::Server,
    purge: tokio::task::JoinHandle<()>,
}

impl Drop for Application {
    /// The purge timer lives as long as the server it belongs to.
    fn drop(&mut self) {
        self.purge.abort();
    }
}

impl Application {
    pub async fn build(settings: Settings) -> Result<Self, anyhow::Error> {
        let pool = connect(&settings).await?;

        let address = format!(
            "{}:{}",
            settings.application.host, settings.application.port
        );
        let listener = std::net::TcpListener::bind(&address)?;
        let port = listener.local_addr()?.port();
        let workers = settings.application.workers;
        let shutdown_timeout = settings.application.shutdown_timeout_secs;

        let accounts = AccountService::new(
            Arc::new(PgAccountRepository::new(pool.clone())),
            PasswordHasher::new(),
        );
        let purge = tokio::spawn(purge_on_a_timer(
            Duration::from_secs(settings.auth.purge_interval_secs),
            || SystemTime::now().into(),
            move |now| {
                let accounts = accounts.clone();
                async move { accounts.purge_expired(now).await }
            },
        ));

        let state = AppState::new(Arc::new(settings), pool);
        let server = actix_web::HttpServer::new(move || {
            actix_web::App::new().configure(|cfg| configure(cfg, state.clone()))
        })
        .workers(workers)
        .shutdown_timeout(shutdown_timeout)
        .listen(listener)?
        .run();

        Ok(Self {
            port,
            server,
            purge,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn run_until_stopped(mut self) -> Result<(), std::io::Error> {
        (&mut self.server).await
    }
}
