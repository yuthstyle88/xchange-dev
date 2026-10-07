//! Ticket #169 — what the RPC dispatcher answers when an action finds, under the branch's lock,
//! that its caller's session is no longer valid. Written before the fix. Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Decided by the lane controller (#169, 2026-10-07T00:00Z): an action that answers
//! `AppError::Unauthenticated` goes out exactly as the route answers a request that has no valid
//! session — status, body and `Cache-Control` — so the page reacts once, the way it already does
//! (back to sign-in). Today the dispatcher maps `AppError::Domain` only
//! (`api/src/application/compat/rpc.rs`, `impl From<AppError> for RpcFailure`) and every other
//! error is the server-error sentence.
//!
//! The expected answer is not typed here: it is read from the route itself, by sending the same
//! request with no cookie.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::sync::Arc;

use pos_error::AppError;
use rpc_common::{Answer, App, Call, Cookies, Desk, STAFF_RPC, one_of_each_role, sign_in};
use serde_json::json;
use xchange::application::compat::actions::{ActionFuture, RpcAction, RpcActions, RpcCall};
use xchange::application::compat::envelope::RpcRoute;

const ORIGIN: &str = "http://desk.test";

/// A family that finds its caller's session gone, for `opsClock`.
///
/// Only the staff route is asked. A custody command (`CustodyService::command`) is reached only
/// through the `ops*` family, and `/api/rpc` accepts no `ops*` name for anybody: `Policy`
/// refuses it for the owner with `INVALID_REQUEST` before any family runs
/// (`xc-auth` `policy.rs`, `allow_rpc`; pinned by `rpc_action_seam_test.rs`). The owner's
/// `saveTransaction` goes through `check_save_in`, not through a custody command. So no request
/// on `/api/rpc` can find a closed account under the lock, and there is nothing to pin there.
struct SessionGone;

impl RpcAction for SessionGone {
    fn handles(&self, _route: RpcRoute, action: &str) -> bool {
        action == "opsClock"
    }

    fn run<'a>(&'a self, _call: RpcCall<'a>) -> ActionFuture<'a> {
        Box::pin(async move { Err(AppError::Unauthenticated.into()) })
    }
}

macro_rules! app_with {
    ($desk:expr, $actions:expr) => {{
        let state = $desk.state();
        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = $desk.clock.clone();
        let actions: RpcActions = $actions;
        App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(actions)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

fn same_answer(got: &Answer, door: &Answer, label: &str) {
    assert_eq!(
        got.status, door.status,
        "{label}: status ({:.300})",
        got.text
    );
    assert_eq!(got.json(), door.json(), "{label}: body");
    assert_eq!(
        got.headers.get("cache-control"),
        door.headers.get("cache-control"),
        "{label}: Cache-Control"
    );
}

#[actix_web::test]
async fn an_action_that_finds_the_session_gone_is_answered_as_the_door_answers_no_session() {
    let desk = Desk::open().await;
    let app = app_with!(desk, vec![Arc::new(SessionGone) as Arc<dyn RpcAction>]);
    let people = one_of_each_role();
    desk.employ(&people).await;
    let cookies: Cookies = sign_in(&app, ORIGIN, &people).await;

    for (path, who) in [(STAFF_RPC, "FRONT")] {
        let call = || Call::action(ORIGIN, path, "opsClock", json!({ "direction": "IN" }));
        let door = app.send(call()).await;
        assert!(
            door.status == 401 || door.status == 400,
            "{path}: the request with no cookie is refused ({} {:.200})",
            door.status,
            door.text
        );
        let got = app.send(call().cookie(&cookies[who])).await;
        same_answer(&got, &door, path);
    }
    desk.finish().await;
}
