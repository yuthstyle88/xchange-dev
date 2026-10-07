//! Written before the implementation (ticket #124, part (d)). Do not edit, delete or weaken.
//!
//! The owner's `POST /api/upload?branch=…` stores into the **first** `branch` of the query, as
//! v2's `new URL(request.url).searchParams.get("branch") || "MAIN"` does
//! (`xchange-online@3ffc5a5` `server.mjs:7156`); an empty first one is `MAIN`.
//!
//! Needs `TEST_DATABASE_URL` like every file route test.

mod common;
mod file_routes_common;

use file_routes_common::*;

async fn stored_branch(app: &TestApp, desk: &Desk, query: &str) -> String {
    let before = desk.rows().await;
    let key = send(
        app,
        Call::upload(
            &format!("/api/upload{query}"),
            &["owner"],
            "image/jpeg",
            JPEG,
        ),
    )
    .await
    .upload_key();
    let after = desk.rows().await;
    let row = after
        .difference(&before)
        .find(|row| row.key == key)
        .unwrap_or_else(|| panic!("{query}: the stored row of {key}"));
    row.branch_id.clone()
}

#[actix_web::test]
async fn of_a_repeated_branch_the_first_counts() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    for (query, branch) in [
        ("?branch=BR-2", "BR-2"),
        ("", "MAIN"),
        ("?branch=", "MAIN"),
        ("?branch=BR-2&branch=MAIN", "BR-2"),
        ("?branch=MAIN&branch=BR-2", "MAIN"),
        ("?branch=&branch=BR-2", "MAIN"),
    ] {
        assert_eq!(stored_branch(&app, &desk, query).await, branch, "{query}");
    }
    drop(app);
    desk.finish().await;
}
