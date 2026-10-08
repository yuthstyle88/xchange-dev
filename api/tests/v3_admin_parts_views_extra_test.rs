//! Ticket #476 rework: extra tests for transaction_documents error propagation.
//!
//! Must-fix 1:
//! - Map only the not-found case to `files: []`. That is `AppError::Domain(NO_DOCUMENTS)`.
//! - Every other error propagates as the view's error.
//! - A forced database or lookup failure is not answered as `files: []`.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use actix_web::http::Method;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::json;

const ORIGIN: &str = "http://desk.test";
const DOCUMENTS: &str = "transaction_documents";
const MAIN: &str = "MAIN";

fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn view_path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
    }
}

struct TestRoom {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl TestRoom {
    async fn open() -> Self {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        TestRoom { desk, app, cookies }
    }

    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path).cookie(&self.cookies[who]);
        self.app.send(call).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

#[actix_web::test]
async fn not_found_exchange_answers_files_empty_list() {
    let room = TestRoom::open().await;
    // An existing branch, but nonexistent ref -> maps NO_DOCUMENTS to files: []
    let path = view_path(
        DOCUMENTS,
        &format!("branch={MAIN}&ref=NONEXISTENT_REF_12345"),
    );
    let answer = room.get("OWNER", &path).await;
    assert_eq!(
        answer.status, 200,
        "status: {}, body: {}",
        answer.status, answer.text
    );
    let body = answer.json();
    assert_eq!(body["changed"], json!(true));
    assert_eq!(body["data"]["ref"], json!("NONEXISTENT_REF_12345"));
    assert_eq!(body["data"]["files"], json!([]));
    room.finish().await;
}

#[actix_web::test]
async fn lookup_failure_on_unknown_branch_propagates_error_and_is_not_files_empty() {
    let room = TestRoom::open().await;
    // An unknown branch -> lookup.find fails with NO_SUCH_BRANCH, which is not NO_DOCUMENTS.
    let path = view_path(DOCUMENTS, "branch=NONEXISTENT-BRANCH&ref=TX-123");
    let answer = room.get("OWNER", &path).await;
    assert_ne!(
        answer.status, 200,
        "lookup failure must not answer 200 OK: {}",
        answer.text
    );
    assert_eq!(
        answer.status, 500,
        "lookup failure must return 500 serverError: {}",
        answer.text
    );
    assert!(
        !answer.text.contains("\"files\":[]"),
        "must not answer files: []"
    );
    room.finish().await;
}

#[actix_web::test]
async fn forced_database_failure_propagates_error_and_is_not_files_empty() {
    let room = TestRoom::open().await;
    // Force database failure by renaming the transactions table
    sqlx::query("ALTER TABLE transactions RENAME TO transactions_temp_failure")
        .execute(&room.desk.db.pool)
        .await
        .expect("rename transactions table to force db failure");

    let path = view_path(DOCUMENTS, &format!("branch={MAIN}&ref=TX_123"));
    let answer = room.get("OWNER", &path).await;

    // Restore table before asserting to guarantee table is restored even if assert fails
    let restore = sqlx::query("ALTER TABLE transactions_temp_failure RENAME TO transactions")
        .execute(&room.desk.db.pool)
        .await;

    assert_ne!(
        answer.status, 200,
        "db failure must not answer 200 OK: {}",
        answer.text
    );
    assert_eq!(
        answer.status, 500,
        "db failure must return 500 serverError: {}",
        answer.text
    );
    assert!(
        !answer.text.contains("\"files\":[]"),
        "must not answer files: [] on db failure"
    );
    assert!(
        !answer.text.contains("transactions_temp_failure"),
        "error message must not leak sql details"
    );

    restore.expect("restore transactions table");
    room.finish().await;
}
