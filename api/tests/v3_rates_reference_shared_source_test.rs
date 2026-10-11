//! Specification test for #1066 (`docs/design/rate-feed-connector.md` §8.3): the reference view's
//! cache reads the one `AppState.rate_source`, instead of building its own `BotRateSource::bot()`
//! (`views/rates_reference.rs:226-233` at `00eb37bd`).
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The view is registered the way the server registers it — through `bootstrap::configure` with a
//! state whose `rate_source` is this file's fake — and never through `ViewEndpoint::register` with
//! a cache of the test's own. One test only in this target: the view's cache is server-wide (one
//! per process), so a second state in the same process would not be a fair test.
//!
//! No request leaves the machine: the view's only source is this file's fake. (Before the
//! implementation the scaffold's view cache holds a placeholder that answers an error without a
//! call, and the test fails on the rows.)
//!
//! Sessions are made at the real sign-in doors (`rpc_common`). Needs `TEST_DATABASE_URL`.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use pos_error::AppResult;
use rpc_common::{App, Call, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xchange::presentation::http::rpc::RpcClock;

const ORIGIN: &str = "http://desk.test";

/// Answers six made-up rows nobody else has (USD 35.4321), counting every fetch.
struct Marked {
    asked: AtomicU64,
}

impl RateSource for Marked {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        Box::pin(async move {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let row = |code: &str, unit: u64, t: &str| {
                let n: Value = serde_json::from_str(t).expect("a JSON number");
                json!({ "code": code, "date": "2026-10-09", "unit": unit,
                        "buyingSight": n, "buyingTransfer": n, "selling": n,
                        "original": { "buyingSight": t, "buyingTransfer": t, "selling": t } })
            };
            Ok(ReferenceRates {
                source: "https://gateway.api.bot.or.th/Stat-ExchangeRate/v2".to_string(),
                fetched_at: "2026-10-09T11:05:00.000Z".to_string(),
                kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
                rows: vec![
                    row("USD", 1, "35.4321"),
                    row("EUR", 1, "38.4321"),
                    row("GBP", 1, "44.4321"),
                    row("JPY", 100, "22.4321"),
                    row("CNY", 1, "5.4321"),
                    row("AUD", 1, "23.4321"),
                ],
            })
        })
    }
}

#[actix_web::test]
async fn the_reference_view_reads_the_rate_source_the_state_was_built_with() {
    let desk = Desk::open().await;
    let marked = Arc::new(Marked {
        asked: AtomicU64::new(0),
    });
    let state = desk
        .state()
        .with_rate_source(marked.clone() as Arc<dyn RateSource>);
    let rpc_clock: Arc<dyn RpcClock> = desk.clock.clone();
    let app = App::new(
        actix_web::test::init_service(
            actix_web::App::new()
                .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                .app_data(actix_web::web::Data::from(rpc_clock)),
        )
        .await,
        |t| t.to_request(),
    );
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies = sign_in(&app, ORIGIN, &people).await;

    let mut call = Call::post(
        ORIGIN,
        "/api/v3/views/rates_reference?branch=MAIN&fresh=1",
        Vec::<u8>::new(),
    )
    .without("x-xchange-request")
    .without("content-type")
    .without("origin")
    .cookie(&cookies["OWNER"]);
    call.method = Method::GET;
    let answer = app.send(call).await;

    assert_eq!(answer.status, 200, "{:.400}", answer.text);
    let body = answer.json();
    let usd = body["data"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("rows in {body}"))
        .iter()
        .find(|r| r["code"] == json!("USD"))
        .unwrap_or_else(|| panic!("a USD row in {body}"))
        .clone();
    assert_eq!(
        usd["buyingTransfer"],
        json!("35.4321"),
        "the state's source: {body}"
    );
    assert_eq!(body["data"]["fetchedAt"], json!("2026-10-09T11:05:00.000Z"));
    assert!(
        marked.asked.load(Ordering::SeqCst) >= 1,
        "the state's source was asked"
    );
    desk.finish().await;
}
