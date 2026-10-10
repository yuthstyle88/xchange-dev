//! OR-4b (#767, D-4, §8, §9 rows 7 and last): the kiosk route authorises a Blueprint-session
//! employee on the branch the request names. Implementer tests, added on the lane controller's
//! order after the FAIL at `5da07e0` (PR 1019, rows P3 and P2c).
//!
//! Every request goes through the server's whole route table (`bootstrap::configure`), and the
//! teller is signed in only through Identity (`__Host-xc_session`); no password cookie is sent.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::{Value, json};

/// `PolicyError::OutOfScope`'s sentence (D-4).
const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";

/// A database with branch `B2`, the app on it, and a TELLER of `branches` signed in through
/// Identity. The fake Identity is returned so it lives as long as the test.
async fn teller_of(branches: &[&str]) -> (Desk, FakeIdentity, App, String) {
    let desk = Desk::start().await;
    sqlx::query("INSERT INTO branches (id, name) VALUES ('B2', 'Branch Two')")
        .execute(&desk.pool)
        .await
        .expect("seed B2");
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let roles: Vec<(&str, Option<&str>)> = branches.iter().map(|b| ("TELLER", Some(*b))).collect();
    bound_user(&desk.pool, SUB_A, "ACTIVE", "Teller", &roles).await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let token = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "teller", "password": "pw-from-the-browser-01" }),
        ))
        .await
        .session();
    (desk, identity, app, token)
}

async fn kiosk(app: &App, token: &str, action: &str, payload: Value) -> Reply {
    app.send(
        Req::post(
            "/api/kiosk",
            json!({ "action": action, "payload": payload }),
        )
        .with_session(token),
    )
    .await
}

/// P3: a TELLER of MAIN only, naming B2, is refused `outOfScope`: never answered with MAIN's
/// queue, and never with B2's.
#[actix_web::test]
async fn a_teller_naming_a_branch_outside_their_scope_is_refused() {
    let (desk, _identity, app, token) = teller_of(&["MAIN"]).await;

    let reply = kiosk(&app, &token, "list", json!({ "branchId": "B2" })).await;
    assert_eq!(reply.status, 403, "{}", reply.text);
    assert_eq!(reply.json()["error"], json!(OUT_OF_SCOPE), "{}", reply.text);

    // the own branch, named, is answered
    let reply = kiosk(&app, &token, "list", json!({ "branchId": "MAIN" })).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["branchId"], json!("MAIN"), "{}", reply.text);
    desk.drop_schema().await;
}

/// P2c: a TELLER of MAIN and B2, naming B2, is answered for B2.
#[actix_web::test]
async fn a_two_branch_teller_naming_their_second_branch_runs_there() {
    let (desk, _identity, app, token) = teller_of(&["MAIN", "B2"]).await;

    let reply = kiosk(&app, &token, "list", json!({ "branchId": "B2" })).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["branchId"], json!("B2"), "{}", reply.text);

    let reply = kiosk(&app, &token, "list", json!({ "branchId": "MAIN" })).await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["branchId"], json!("MAIN"), "{}", reply.text);
    desk.drop_schema().await;
}
