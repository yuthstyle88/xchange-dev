//! Written before the implementation (ticket #139). Do not edit, delete or weaken.
//!
//! The documents pages, `GET /documents` and `GET /staff/documents`, around what the recorded
//! fixture holds:
//!
//! - a branch name that cannot be a branch id is `สาขาไม่ถูกต้อง`, before the branch is looked for
//!   (v2's `execute`, `server.mjs:6999`; the rows measured through v2's `handle()` by
//!   claude/be518a0f, ticket #139, comment 6022241441);
//! - of a repeated query name the **first** counts (v2 `searchParams.get`, `server.mjs:7770`,
//!   `:7773`);
//! - a file's type comes from its record in the branch, never from its bytes (v2
//!   `SELECT mime FROM file_records WHERE key=$1 AND branch_id=$2`, `server.mjs:7787`): a file
//!   whose bytes are gone keeps its type, and a file stored by an employee is found as well as
//!   one stored by the owner.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`); without it they fail, none skips.

mod common;
mod documents_common;
mod save_common;
#[allow(clippy::duplicate_mod)]
mod shift_holder_common;

use documents_common::*;

// The page tests of this file (the six branch rows, the eleven names, the repeated `ref` and `branch`, a file
// whose bytes are gone, and the page half of the last test) moved to the views and the screen by #1349
// (design §A.8, D-A6a): `v3_documents_views_test.rs` and `web/src/screens/documents/DocumentsScreen.test.tsx`.

#[actix_web::test]
async fn the_files_of_an_exchange_an_employee_saved_are_found_by_branch() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let shop = &world.shop;
    // OR-7.m G1a: rule 1 (#3796)
    let employee = shift_holder_common::user_of(&shop.pool, "employee-front").await;
    // MAIN is ready (rate, open shift) from the fixture's exchanges
    let customer = shop.reserve(save_common::MAIN, save_common::person()).await;
    let passport = shop
        .upload(
            save_common::MAIN,
            &employee,
            "image/jpeg",
            save_common::JPEG,
        )
        .await;
    let slip = shop
        .upload(
            save_common::MAIN,
            &employee,
            "application/pdf",
            save_common::PDF,
        )
        .await;
    let mut form = save_common::form("submission-by-the-front-000000", &customer, &passport);
    form.method = "Bank Transfer".to_owned();
    form.payment_ref = "TRX-9".to_owned();
    form.slip = Some(slip.clone());
    shift_holder_common::open_shift_for(
        &shop.pool,
        save_common::MAIN,
        &employee,
        &[("THB", "1000000"), ("USD", "1000")],
    )
    .await;
    let saved = shop
        .workflow
        .save(
            &save_common::branch(save_common::MAIN),
            form,
            &employee,
            save_common::at(100),
        )
        .await
        .expect("an employee's save");

    let lookup = world.lookup();
    let found = lookup
        .find(&save_common::branch(save_common::MAIN), &saved.document_ref)
        .await
        .expect("found");
    let types: Vec<&str> = found.links.iter().map(|l| l.mime.as_str()).collect();
    assert_eq!(types, ["image/jpeg", "application/pdf", "text/html"]);

    world.shop.finish().await;
}
