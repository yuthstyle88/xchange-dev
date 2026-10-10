//! PL-0a (#910) second batch — the outbox-handler seam at boot (design `broker-seam.md` §5.2,
//! §11.1): `collect`, the generated `OUTBOX_HANDLERS`, and the worker that `Application::build`
//! starts and its `Drop` stops.
//! Written before the implementation by the ticket author (claude/89bd61d9), after OB-1 #899
//! merged. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! How the worker is seen without any handler file: `PgOutboxStore::claim_due` claims every due
//! `PENDING` row, handled or not, and raises its `attempts`
//! (`api/crates/xc-outbox/src/infrastructure/pg_store.rs:42-67`); a row with no handler then stays
//! `PENDING` (`application/worker.rs`, `tick`). So a due row of an event type nobody handles
//! shows `attempts >= 1` once a worker has ticked, and stays at 0 when none runs.
//!
//! The boot tests need `TEST_DATABASE_URL` (a role that may create databases); the others do not.

mod common;
mod routes_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pos_error::{AppError, AppResult};
use routes_common::ScratchDatabase;
use sqlx::{PgPool, Postgres, Transaction};
use xc_outbox::domain::event::OutboxEvent;
use xc_outbox::domain::event_kind::EventKind;
use xc_outbox::domain::handler::{DeliveryResult, FinalOutcome, OutboxHandler};
use xchange::bootstrap::outbox_handlers::{self, HandlerFn, OUTBOX_HANDLERS};
use xchange::bootstrap::{self, AppState, Application};

static FIRST: EventKind = EventKind {
    event_type: "test.seam.first",
    destination: "first",
    aggregate_type: "thing",
};
static SECOND: EventKind = EventKind {
    event_type: "test.seam.second",
    destination: "second",
    aggregate_type: "thing",
};
/// `FIRST`'s event type, another destination.
static FIRST_AGAIN: EventKind = EventKind {
    event_type: "test.seam.first",
    destination: "elsewhere",
    aggregate_type: "thing",
};

/// `futures::future::BoxFuture`, which the root package does not depend on.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct Fake(&'static EventKind);

impl OutboxHandler for Fake {
    fn kind(&self) -> &'static EventKind {
        self.0
    }

    fn deliver<'a>(&'a self, _event: &'a OutboxEvent) -> BoxFuture<'a, DeliveryResult> {
        Box::pin(async { DeliveryResult::Final(FinalOutcome::Delivered { http_status: 200 }) })
    }

    fn on_final<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _event: &'a OutboxEvent,
        _outcome: &'a FinalOutcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

fn fake(kind: &'static EventKind) -> Option<Arc<dyn OutboxHandler>> {
    Some(Arc::new(Fake(kind)))
}

/// The shape every destination file's `handler` has, as a `HandlerFn` (#910 §7).
fn unconfigured(_state: &AppState) -> Option<Arc<dyn OutboxHandler>> {
    None
}

// ---------------------------------------------------------------- collect

#[test]
fn collect_skips_a_handler_that_is_not_configured() {
    let registry =
        outbox_handlers::collect([None, fake(&FIRST), None, fake(&SECOND)]).expect("no duplicates");
    assert_eq!(
        registry.handler("test.seam.first").expect("first").kind(),
        &FIRST
    );
    assert_eq!(
        registry.handler("test.seam.second").expect("second").kind(),
        &SECOND
    );
    assert!(registry.handler("test.seam.none").is_none());
}

#[test]
fn collect_of_nothing_configured_is_an_empty_registry() {
    let registry = outbox_handlers::collect([None, None]).expect("nothing to clash");
    assert!(registry.handler("test.seam.first").is_none());
    let registry = outbox_handlers::collect(Vec::new()).expect("no handler at all");
    assert!(registry.handler("test.seam.first").is_none());
}

#[test]
fn collect_refuses_two_handlers_of_one_event_type_and_names_it() {
    match outbox_handlers::collect([fake(&FIRST), None, fake(&FIRST_AGAIN)]) {
        Err(AppError::Internal(message)) => assert!(
            message.contains("test.seam.first"),
            "names the event type: {message}"
        ),
        Err(other) => panic!("expected Internal, got {other:?}"),
        Ok(_) => panic!("two handlers of one event type must fail the boot"),
    }
}

// ---------------------------------------------------------------- the generated table

#[test]
fn the_table_has_one_entry_per_handler_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/outbox_handlers");
    let files = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs") && n != "mod.rs")
        .count();
    assert_eq!(
        OUTBOX_HANDLERS.len(),
        files,
        "every bootstrap/outbox_handlers/<destination>.rs is in OUTBOX_HANDLERS, through build.rs"
    );
    let _shape: HandlerFn = unconfigured;
}

#[tokio::test]
async fn the_registry_is_built_from_the_table() {
    let state = AppState::new(Arc::new(common::settings(&[])), common::unreachable_pool());
    let registry = outbox_handlers::registry(&state).expect("the registered handlers do not clash");
    // every configured entry of the table is in the registry under its own event type
    for handler in OUTBOX_HANDLERS.iter().filter_map(|f| f(&state)) {
        let kind = handler.kind();
        assert_eq!(
            registry
                .handler(kind.event_type)
                .unwrap_or_else(|| panic!("{} is registered", kind.event_type))
                .kind(),
            kind
        );
    }
}

// ---------------------------------------------------------------- the boot

/// A due `PENDING` row of an event type no handler delivers; answers its id.
async fn unhandled_row(pool: &PgPool, n: u32) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO outbox_events (id, organization_id, event_type, destination, aggregate_type, \
             aggregate_id, idempotency_key, payload_hash, payload_redacted, status, attempts, \
             next_attempt_at, created_at) \
         VALUES ($1, '00000000-0000-0000-0000-000000000001', 'test.seam.nobody', 'nowhere', \
             'thing', $2, $3, 'h', '{}'::jsonb, 'PENDING', 0, now() - interval '1 minute', now())",
    )
    .bind(id)
    .bind(format!("aggregate-{n}"))
    .bind(format!("key-{n}"))
    .execute(pool)
    .await
    .expect("insert an outbox row");
    id
}

async fn row(pool: &PgPool, id: uuid::Uuid) -> (String, i32) {
    sqlx::query_as("SELECT status, attempts FROM outbox_events WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the row")
}

/// The migrated scratch database, and a pool to look into it.
async fn migrated(label: &str) -> (ScratchDatabase, PgPool) {
    let scratch = ScratchDatabase::create(label).await;
    bootstrap::connect(&scratch.settings())
        .await
        .expect("migrate the scratch database")
        .close()
        .await;
    let pool = scratch.inspect().await;
    (scratch, pool)
}

fn settings(scratch: &ScratchDatabase) -> xchange::config::Settings {
    let mut settings = scratch.settings();
    settings.application.port = 0;
    settings
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_built_server_runs_the_outbox_worker_and_leaves_unhandled_rows_pending() {
    let (scratch, pool) = migrated("t1b_outbox_boot").await;
    let id = unhandled_row(&pool, 1).await;

    common::install_test_pii_keys();
    let application = Application::build(settings(&scratch))
        .await
        .expect("no handler file: the server still builds and boots");

    // the worker's first tick runs at once; give it well over one TICK (5 s)
    let deadline = Instant::now() + Duration::from_secs(15);
    let (status, attempts) = loop {
        let now = row(&pool, id).await;
        if now.1 >= 1 || Instant::now() > deadline {
            break now;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(
        attempts >= 1,
        "Application::build spawns OutboxWorker::run_forever, which claims the due row"
    );
    assert_eq!(status, "PENDING", "no handler: the row is not delivered");

    drop(application);
    pool.close().await;
    scratch.drop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_server_stops_its_outbox_worker() {
    let (scratch, pool) = migrated("t1b_outbox_drop").await;
    let first = unhandled_row(&pool, 1).await;

    common::install_test_pii_keys();
    let application = Application::build(settings(&scratch))
        .await
        .expect("the server builds");
    let deadline = Instant::now() + Duration::from_secs(15);
    while row(&pool, first).await.1 < 1 {
        assert!(
            Instant::now() < deadline,
            "the worker never claimed the first row"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    drop(application);
    let second = unhandled_row(&pool, 2).await;
    // longer than one TICK (5 s): a worker still running would claim the new row
    tokio::time::sleep(Duration::from_secs(8)).await;
    assert_eq!(
        row(&pool, second).await,
        ("PENDING".to_owned(), 0),
        "Application's Drop aborts the worker, as it does the purge timer"
    );

    pool.close().await;
    scratch.drop().await;
}
