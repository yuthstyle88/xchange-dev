//! #5683 (F-4 of #5558, `docs/design/org-wide-actor-branch.md` §15.10): once `pricingModeSave` and
//! `marketRatesRefresh` are organisation-level rows (`xc-auth` `areas/rates.rs`), the policy decides who acts on the
//! organisation's object. The command no longer looks at the session branch's name, so an org-wide holder whose
//! session branch is not MAIN saves the mode and gets the FOLLOW plan, the same as at MAIN.
//! Written before the implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! `v9_follow_common` pins the session branch to MAIN (its `user_of`) and says not to edit it, so this file reads
//! callers through its own `SessionReader`: `x-test-as: id|ROLE@BRANCH`.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod v9_follow_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use actix_web::http::{Method, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use v9_follow_common::*;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const BR_B: &str = "BR-B";

/// The owner's roles (EXECUTIVE + ADMIN organisation-wide), signed in at `session`.
fn owner_signed_in_at(session: &str) -> User {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = vec![
        ScopedRole::new(Role::Executive, None::<BranchId>, from, None),
        ScopedRole::new(Role::Admin, None::<BranchId>, from, None),
    ];
    User::new("owner-1", ORG, roles, None).with_branch(Some(BranchId::new(session)))
}

/// Reads `x-test-as: owner|<session branch>`.
struct SessionAt;

impl SessionReader for SessionAt {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let session = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("owner|"))
            .map(str::to_owned);
        Box::pin(async move {
            let user = session.as_deref().map(owner_signed_in_at);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

/// `POST <route>` as the owner signed in at `session`, naming `session` in the body too.
async fn post_at(state: AppState, session: &str, route: &str, mut body: Value) -> Answer {
    body["branch"] = json!(session);
    let reader: Arc<dyn SessionReader> = Arc::new(SessionAt);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(route)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, format!("owner|{session}")))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    Answer {
        status,
        etag: None,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

/// Row 1: an org-wide EXECUTIVE signed in at branch B saves the organisation's mode (today: `400 validationFailed`,
/// field `branch`, from `pricing_mode_save.rs:62`).
#[tokio::test]
async fn pricing_mode_save_by_an_org_wide_holder_signed_in_away_from_main_saves_the_mode() {
    let room = Room::start().await;
    room.create_branch(BR_B).await;
    let a = post_at(
        state(&room),
        BR_B,
        MODE,
        json!({ "currency": "USD", "mode": "FOLLOW" }),
    )
    .await;
    assert_eq!(
        a.ok("pricingModeSave signed in at B"),
        json!({ "saved": true }),
        "{}",
        a.text
    );
    let mode: String =
        sqlx::query_scalar("SELECT mode FROM pricing_modes WHERE trim(currency_code) = 'USD'")
            .fetch_one(&room.pool)
            .await
            .expect("the organisation's USD mode row");
    assert_eq!(mode, "FOLLOW");
    room.finish().await;
}

/// Row 2: an org-wide holder signed in at branch B refreshes the market with FOLLOW on for USD and gets the USD
/// plan, as at MAIN (today: `followResults` is empty, `market_rates_refresh.rs:106`).
#[tokio::test]
async fn market_rates_refresh_by_an_org_wide_holder_signed_in_away_from_main_plans_follow() {
    let room = Room::start().await;
    room.create_branch(BR_B).await;
    room.policy(MAIN).await;
    room.policy(BR_B).await;
    room.bot(&[("USD", USD_BOT.0, USD_BOT.1)]).await;
    follow_on(&room, "USD").await;

    let tt = Arc::new(FakeMarket::new("TT", &[("USD", PV1_TT.0, PV1_TT.1)]));
    let sr = Arc::new(FakeMarket::new("SR", &[("USD", "33.33", "33.40")]));
    let sources: MarketSources = vec![
        (MarketSource::Tt, tt.clone() as Arc<dyn RateSource>),
        (MarketSource::SuperRich, sr as Arc<dyn RateSource>),
    ];
    let a = post_at(
        state(&room).with_market_sources(sources),
        BR_B,
        MARKET_REFRESH,
        json!({}),
    )
    .await;
    let out = a.ok("marketRatesRefresh signed in at B");
    assert_eq!(tt.calls.load(Ordering::SeqCst), 1, "TT fetched once");
    let results = out["followResults"]
        .as_array()
        .unwrap_or_else(|| panic!("followResults: {}", a.text));
    assert_eq!(results.len(), 1, "one per FOLLOW currency: {}", a.text);
    assert_eq!(results[0]["currency"], json!("USD"), "{}", a.text);
    assert_eq!(results[0]["status"], json!("PUBLISHED"), "{}", a.text);
    assert_eq!(
        room.follow_result("USD").await.map(|r| r.status),
        Some("PUBLISHED".to_string())
    );
    room.finish().await;
}
