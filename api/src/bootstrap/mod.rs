//! Bootstrap and server initialization.

pub mod approval_subjects;
pub mod exchange_effects;
pub mod outbox_handlers;

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
use xc_outbox::application::worker::OutboxWorker;
use xc_outbox::infrastructure::pg_store::PgOutboxStore;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use xc_rate::infrastructure::reference::pg_reference_batch_store::PgReferenceBatchStore;

use crate::config::Settings;
use crate::infrastructure::rate_feed::branch_refresh::DraftBranchRefresh;
use crate::infrastructure::rate_feed::feed_loop::{RateFeedConfig, RateFeedLoop};
use crate::presentation::http::v3::commands::make_approval_service;

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

use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Settings>,
    pub db: PgPool,
    pub build: BuildInfo,
    pub database_probe: Arc<dyn DatabaseProbe>,
    pub rate_source: Arc<dyn RateSource>,
}

impl AppState {
    pub fn new(settings: Arc<Settings>, db: PgPool) -> Self {
        let probe = Arc::new(db.clone());
        let rate_source = choose_rate_source(&settings.bot, Arc::new(BotRateSource::bot()));
        Self {
            settings,
            db,
            build: BuildInfo::current(),
            database_probe: probe,
            rate_source,
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

    pub fn with_rate_source(mut self, source: Arc<dyn RateSource>) -> Self {
        self.rate_source = source;
        self
    }
}

/// The reference-rate source the server reads (`docs/design/rate-feed-connector.md` §8.5): with
/// `bot.api_token`, the official API with `scraper` behind it; without it, `scraper` alone and one
/// `warn`. `AppState::new` passes `BotRateSource::bot()`.
// scaffold (#1066): placeholder answer, the scraper whatever the settings say; no warn.
pub fn choose_rate_source(
    bot: &crate::config::BotSettings,
    scraper: Arc<dyn RateSource>,
) -> Arc<dyn RateSource> {
    if let Some(token) = &bot.api_token
        && !token.trim().is_empty()
    {
        let api_config =
            xc_rate::infrastructure::reference_source::bot_api_rate_source::BotApiConfig {
                base: bot.api_base.clone(),
                token: token.clone(),
            };
        let api_source = Arc::new(
            xc_rate::infrastructure::reference_source::bot_api_rate_source::BotApiRateSource::new(
                api_config,
                std::time::Duration::from_secs(5),
                Arc::new(chrono::Utc::now),
            ),
        );
        return Arc::new(
            xc_rate::infrastructure::reference_source::fallback_rate_source::FallbackRateSource::new(
                api_source,
                scraper,
            ),
        );
    }
    tracing::warn!(
        "BoT API token not set (APP_BOT__API_TOKEN); reference rates come from the website scraper"
    );
    scraper
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
    outbox: tokio::task::JoinHandle<()>,
    rate_feed_stop: tokio::sync::watch::Sender<bool>,
    rate_feed: tokio::task::JoinHandle<()>,
}

impl Drop for Application {
    /// The purge timer, outbox worker, and rate feed loop live as long as the server they belong to.
    fn drop(&mut self) {
        let _ = self.rate_feed_stop.send(true);
        self.purge.abort();
        self.outbox.abort();
        self.rate_feed.abort();
    }
}

impl Application {
    pub async fn build(settings: Settings) -> Result<Self, anyhow::Error> {
        let pii_key_path = std::path::Path::new("/etc/xc-v3/pii.key");
        if pii_key_path.exists() {
            let keys = xc_kernel::pii::PiiKeys::load(pii_key_path)?;
            let _ = xc_customer::install_pii_keys(std::sync::Arc::new(keys));
        } else if xc_customer::get_pii_keys().is_err() {
            anyhow::bail!(
                "PII key file {} does not exist and no keys are installed",
                pii_key_path.display()
            );
        }

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

        let state = AppState::new(Arc::new(settings), pool.clone());

        let registry = outbox_handlers::registry(&state)?;
        registry.verify_against(&pool).await?;
        let outbox = tokio::spawn(
            OutboxWorker::new(
                Arc::new(PgOutboxStore::new(pool.clone())),
                Arc::new(registry),
            )
            .run_forever(),
        );

        let (rate_feed_stop, stop_rx) = tokio::sync::watch::channel(false);
        let rate_feed_loop = RateFeedLoop::new(
            pool.clone(),
            Arc::new(ReferenceBatchWriter::new(
                state.rate_source.clone(),
                Arc::new(PgReferenceBatchStore::new()),
            )),
            Arc::new(DraftBranchRefresh::new(
                Arc::new(RateDraftService::new(
                    pool.clone(),
                    make_approval_service(pool.clone()),
                )),
                Arc::new(PublishService::new(pool)),
            )),
            RateFeedConfig::from_env(),
        );
        let rate_feed = tokio::spawn(Arc::new(rate_feed_loop).run(stop_rx));

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
            outbox,
            rate_feed_stop,
            rate_feed,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn run_until_stopped(mut self) -> Result<(), std::io::Error> {
        let result = (&mut self.server).await;
        let _ = self.rate_feed_stop.send(true);
        let _ = (&mut self.rate_feed).await;
        result
    }
}
