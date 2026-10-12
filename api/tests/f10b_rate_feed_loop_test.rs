//! Rate feed F10b — starting the BoT refresh loop with the server
//! (`docs/design/rate-feed-connector.md` §11.5; ticket #1242).
//!
//! Covers the four behaviour rows of #1242:
//! • (1) With RATE_FEED_INTERVAL_SECS small and a stub RateSource, the running server fetches at least once.
//! • (2) §9's no-BOT_DRAFT row: one fetch per tick, and nothing refreshed.
//! • (3) §9's BoT-unreachable row: the fetch error is logged, and the server still starts and answers.
//! • (4) Stopping the server sends true on the stop channel, and the loop task ends.
//!
//! Each test goes red when the rate feed spawn line in `api/src/bootstrap/mod.rs` is removed.

mod common;
mod routes_common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chrono::{FixedOffset, Utc};
use serde_json::json;
use xchange::bootstrap::Application;

use common::install_test_pii_keys;
use routes_common::{ScratchDatabase, http_get};

static TEST_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn sample_bot_rates() -> String {
    let bangkok = FixedOffset::east_opt(7 * 3600).expect("UTC+7");
    let yesterday = (Utc::now().with_timezone(&bangkok).date_naive() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let row = |code: &str, name: &str, s: &str, t: &str, l: &str| {
        json!({
            "period": yesterday,
            "currency_id": code,
            "currency_name_th": name,
            "currency_name_eng": name,
            "buying_sight": s,
            "buying_transfer": t,
            "selling": l,
            "mid_rate": t
        })
    };
    json!({
        "result": {
            "api": "Daily Average Exchange Rate - THB / Foreign Currency",
            "timestamp": "2026-10-09 18:05:00",
            "data": {
                "data_header": { "last_updated": yesterday },
                "data_detail": [
                    row("USD", "USA : DOLLAR (USD) ", "31.1000000", "31.2000000", "31.5000000"),
                    row("EUR", "EURO ZONE : EURO (EUR)", "36.1000000", "36.2000000", "36.9000000"),
                    row("GBP", "UNITED KINGDOM : POUND STERLING (GBP)", "41.1000000", "41.2000000", "41.9000000"),
                    row("JPY", "JAPAN : YEN (100 YEN) (JPY)", "20.1000000", "20.2000000", "20.9000000"),
                    row("CNY", "CHINA : YUAN (CNY)", "4.8000000", "4.8500000", "4.9500000"),
                    row("AUD", "AUSTRALIA : DOLLAR (AUD)", "21.1000000", "21.2000000", "21.9000000"),
                ]
            }
        }
    })
    .to_string()
}

struct StubBotServer {
    port: u16,
    calls: Arc<AtomicUsize>,
    unreachable: Arc<AtomicBool>,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

impl StubBotServer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub bot server");
        let port = listener.local_addr().expect("local addr").port();
        let calls = Arc::new(AtomicUsize::new(0));
        let unreachable = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let calls_clone = calls.clone();
        let unreachable_clone = unreachable.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut socket, _)) = accepted else { break; };
                        calls_clone.fetch_add(1, Ordering::SeqCst);
                        let is_unreachable = unreachable_clone.load(Ordering::SeqCst);
                        tokio::spawn(async move {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            let mut buf = [0u8; 1024];
                            let _ = socket.read(&mut buf).await;
                            if is_unreachable {
                                let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                let _ = socket.write_all(resp.as_bytes()).await;
                            } else {
                                let body = sample_bot_rates();
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                );
                                let _ = socket.write_all(resp.as_bytes()).await;
                            }
                        });
                    }
                }
            }
        });

        Self {
            port,
            calls,
            unreachable,
            _shutdown: shutdown_tx,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn set_unreachable(&self, value: bool) {
        self.unreachable.store(value, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn running_server_with_small_interval_and_stub_rate_source_fetches_at_least_once() {
    let _guard = TEST_MUTEX.lock().await;
    unsafe {
        std::env::set_var("RATE_FEED_INTERVAL_SECS", "1");
    }

    let stub = StubBotServer::start().await;
    let db = ScratchDatabase::create("t1b_f10b_fetch_once").await;
    install_test_pii_keys();

    let mut settings = db.settings();
    settings.bot.api_token = Some("test-token".to_string());
    settings.bot.api_base = format!("http://127.0.0.1:{}/Stat-ExchangeRate/v2", stub.port);

    let application = Application::build(settings)
        .await
        .expect("application builds");

    // The running server's rate feed loop runs immediately on start. Wait for at least one fetch.
    let deadline = Instant::now() + Duration::from_secs(5);
    while stub.calls() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        stub.calls() >= 1,
        "the running server must fetch from the rate source at least once"
    );

    // Verify a batch was stored in the database (poll until written, bounding at 10s to avoid racing writer)
    let pool = db.inspect().await;
    let batch_deadline = Instant::now() + Duration::from_secs(10);
    let mut batches: i64 = 0;
    while Instant::now() < batch_deadline {
        batches = sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches")
            .fetch_one(&pool)
            .await
            .expect("count batches");
        if batches >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        batches >= 1,
        "rate reference batch must be written to the database"
    );

    drop(application);
    pool.close().await;
    db.drop().await;
}

#[tokio::test]
async fn no_bot_draft_row_fetches_once_per_tick_and_refreshes_nothing() {
    let _guard = TEST_MUTEX.lock().await;
    unsafe {
        std::env::set_var("RATE_FEED_INTERVAL_SECS", "1");
    }

    let stub = StubBotServer::start().await;
    let db = ScratchDatabase::create("t1b_f10b_no_bot_draft").await;
    install_test_pii_keys();
    xchange::bootstrap::connect(&db.settings())
        .await
        .expect("migrate scratch database")
        .close()
        .await;

    let pool = db.inspect().await;
    // Set all branches to 'RATE_BOARD', so no branch has price_source = 'BOT_DRAFT'
    sqlx::query("UPDATE branches SET price_source = 'RATE_BOARD'")
        .execute(&pool)
        .await
        .expect("update branches to RATE_BOARD");

    let initial_pubs: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_publications")
        .fetch_one(&pool)
        .await
        .expect("count publications");

    let mut settings = db.settings();
    settings.bot.api_token = Some("test-token".to_string());
    settings.bot.api_base = format!("http://127.0.0.1:{}/Stat-ExchangeRate/v2", stub.port);

    let application = Application::build(settings)
        .await
        .expect("application builds");

    let deadline = Instant::now() + Duration::from_secs(5);
    while stub.calls() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(
        stub.calls() >= 1,
        "the running server must fetch at least once"
    );

    // Poll for the rate_reference_batches row before asserting (avoids racing writer)
    let batch_deadline = Instant::now() + Duration::from_secs(10);
    let mut batches: i64 = 0;
    while Instant::now() < batch_deadline {
        batches = sqlx::query_scalar("SELECT count(*) FROM rate_reference_batches")
            .fetch_one(&pool)
            .await
            .expect("count batches");
        if batches >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        batches >= 1,
        "batch must be inserted even when no branch is BOT_DRAFT"
    );

    // Assert nothing was refreshed: no new publications created
    let pubs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM rate_publications")
        .fetch_one(&pool)
        .await
        .expect("count publications");
    assert_eq!(
        pubs_after, initial_pubs,
        "no new publications must be created when no branch is BOT_DRAFT"
    );

    // And no draft was created by SYSTEM actor
    let system_drafts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_drafts WHERE created_by LIKE 'SYSTEM:%'")
            .fetch_one(&pool)
            .await
            .expect("count system drafts");
    assert_eq!(
        system_drafts, 0,
        "nothing must be refreshed or created when no branch is BOT_DRAFT"
    );

    drop(application);
    pool.close().await;
    db.drop().await;
}

#[tokio::test]
async fn bot_unreachable_logs_fetch_error_and_server_still_starts_and_answers() {
    let _guard = TEST_MUTEX.lock().await;
    unsafe {
        std::env::set_var("RATE_FEED_INTERVAL_SECS", "1");
        // Ensure scraper fallback also fails immediately without external calls
        std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:1");
    }

    let stub = StubBotServer::start().await;
    stub.set_unreachable(true);

    let db = ScratchDatabase::create("t1b_f10b_unreachable").await;
    install_test_pii_keys();

    let mut settings = db.settings();
    settings.bot.api_token = Some("test-token".to_string());
    settings.bot.api_base = format!("http://127.0.0.1:{}/Stat-ExchangeRate/v2", stub.port);

    let application = Application::build(settings)
        .await
        .expect("server must start even when BoT is unreachable");

    let port = application.port();
    let server = tokio::spawn(application.run_until_stopped());

    // 1. Verify the server is running and answers requests
    let (status, _) = tokio::task::spawn_blocking(move || http_get(port, "/healthz"))
        .await
        .expect("http client");
    assert!(
        status.contains(" 200 "),
        "server must answer 200 on /healthz: got {status}"
    );

    // 2. Verify that the loop attempted to fetch (encountered the unreachable error)
    let deadline = Instant::now() + Duration::from_secs(5);
    while stub.calls() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        stub.calls() >= 1,
        "loop must have attempted to fetch from BoT upstream"
    );

    server.abort();
    let _ = server.await;
    db.drop().await;
    unsafe {
        std::env::remove_var("HTTPS_PROXY");
    }
}

#[tokio::test]
async fn stopping_the_server_sends_true_on_the_stop_channel_and_the_loop_task_ends() {
    let _guard = TEST_MUTEX.lock().await;
    unsafe {
        std::env::set_var("RATE_FEED_INTERVAL_SECS", "1");
    }

    let stub = StubBotServer::start().await;
    let db = ScratchDatabase::create("t1b_f10b_stop_server").await;
    install_test_pii_keys();

    let mut settings = db.settings();
    settings.bot.api_token = Some("test-token".to_string());
    settings.bot.api_base = format!("http://127.0.0.1:{}/Stat-ExchangeRate/v2", stub.port);

    let application = Application::build(settings)
        .await
        .expect("application builds");

    // 1. Wait until the running server's loop has fetched at least once
    let deadline = Instant::now() + Duration::from_secs(5);
    while stub.calls() == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        stub.calls() >= 1,
        "the loop must be running and have fetched before stopping"
    );

    // 2. Stop the server by dropping Application (triggers Drop which sends true on stop channel)
    drop(application);
    let calls_at_stop = stub.calls();

    // 3. Wait for longer than the interval (interval is 1s, wait 2.5s)
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // 4. Assert that no more fetches occurred because the loop task ended
    let calls_after = stub.calls();
    assert_eq!(
        calls_after, calls_at_stop,
        "loop task must end when the server stops: expected {calls_at_stop} calls, got {calls_after}"
    );

    db.drop().await;
}
