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

use documents_common::*;

const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
const NOBODY_HAS: &str = "XC-NOPE-1";

fn refused(answer: &Answer, sentence: &str, what: &str) {
    assert_eq!(answer.status, 404, "{what}: {}", answer.text);
    assert_eq!(answer.text, sentence, "{what}");
}

/// The download name of the first link of an owner's page whose kind is `kind`.
fn download_name(page: &str, kind: &str) -> String {
    let marker = format!("_{kind}.");
    let at = page
        .find(&marker)
        .unwrap_or_else(|| panic!("the page has no {kind} download: {page}"));
    let start = page[..at]
        .rfind("download=\"")
        .expect("a download attribute")
        + "download=\"".len();
    let end = at + page[at..].find('"').expect("the end of the attribute");
    page[start..end].to_owned()
}

#[actix_web::test]
async fn the_six_measured_branch_rows_are_answered_as_v2_answered_them() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    for (query, sentence) in [
        ("branch=MAIN_1", BAD_BRANCH),
        ("branch=B-nowhere", NO_SUCH_BRANCH),
        ("branch=MAIN", NO_DOCUMENTS),
        ("branch=B-nowhere&branch=MAIN", NO_SUCH_BRANCH),
        ("branch=MAIN&branch=B-nowhere", NO_DOCUMENTS),
        ("branch=MAIN_1&branch=MAIN", BAD_BRANCH),
    ] {
        let path = format!("/documents?{query}&ref={NOBODY_HAS}");
        refused(&get(&app, &path, &["owner"]).await, sentence, &path);
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn a_name_that_cannot_be_a_branch_id_is_refused_before_the_branch_is_looked_for() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let cash = world.exchange("cash");
    let seventy = "A".repeat(70);
    let seventy_one = "A".repeat(71);
    for branch in [
        "MAIN_1",
        seventy_one.as_str(),
        "a%20b",
        "a.b",
        "%00",
        "%E0%B8%81",
        "MAIN%0A",
    ] {
        let path = format!("/documents?branch={branch}&ref={}", cash.document_ref);
        refused(&get(&app, &path, &["owner"]).await, BAD_BRANCH, &path);
    }
    // well formed, and no such branch
    for branch in [seventy.as_str(), "-", "b-1"] {
        let path = format!("/documents?branch={branch}&ref={}", cash.document_ref);
        refused(&get(&app, &path, &["owner"]).await, NO_SUCH_BRANCH, &path);
    }
    // an empty name is MAIN, as v2's `|| "MAIN"`
    let path = format!("/documents?branch=&ref={}", cash.document_ref);
    assert_eq!(get(&app, &path, &["owner"]).await.status, 200, "{path}");
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn of_a_repeated_ref_the_first_counts_on_both_pages() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let (cash, transfer) = (world.exchange("cash"), world.exchange("transfer"));
    for (path, who) in [
        (
            format!(
                "/documents?ref={}&ref={}",
                cash.document_ref, transfer.document_ref
            ),
            "owner",
        ),
        (
            format!(
                "/staff/documents?ref={}&ref={}",
                cash.document_ref, transfer.document_ref
            ),
            "front",
        ),
    ] {
        let answer = get(&app, &path, &[who]).await;
        assert_eq!(answer.status, 200, "{path} as {who}: {}", answer.text);
        assert!(
            answer.text.contains(&cash.passport),
            "{path}: the first ref's page"
        );
        assert!(
            !answer.text.contains(&transfer.passport),
            "{path}: not the second ref's page"
        );
    }
    // the first is a number nobody has: not found, though the second exists
    let path = format!("/documents?ref={NOBODY_HAS}&ref={}", cash.document_ref);
    refused(&get(&app, &path, &["owner"]).await, NO_DOCUMENTS, &path);
    let path = format!(
        "/staff/documents?ref={NOBODY_HAS}&ref={}",
        cash.document_ref
    );
    // v2 sends the employee back to the staff page when nothing is found (fixture:
    // "staff documents, Front: a number nobody was given" → 302)
    assert_eq!(get(&app, &path, &["front"]).await.status, 302, "{path}");
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn of_a_repeated_branch_the_first_counts() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let other = world.exchange("other");
    let path = format!(
        "/documents?branch=BR-2&branch=MAIN&ref={}",
        other.document_ref
    );
    let answer = get(&app, &path, &["owner"]).await;
    assert_eq!(answer.status, 200, "{path}: {}", answer.text);
    assert!(answer.text.contains(&other.passport), "{path}");
    let path = format!(
        "/documents?branch=MAIN&branch=BR-2&ref={}",
        other.document_ref
    );
    refused(&get(&app, &path, &["owner"]).await, NO_DOCUMENTS, &path);
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn a_file_whose_bytes_are_gone_keeps_its_type() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let transfer = world.exchange("transfer");
    let path = format!("/documents?ref={}", transfer.document_ref);
    let before = get(&app, &path, &["owner"]).await;
    assert_eq!(before.status, 200, "{}", before.text);
    assert!(download_name(&before.text, "PASSPORT").ends_with(".png"));
    assert!(download_name(&before.text, "PAYMENT").ends_with(".pdf"));

    for key in [&transfer.passport, transfer.slip.as_ref().expect("a slip")] {
        std::fs::remove_file(world.shop.dir.join(key)).expect("remove the stored bytes");
    }
    let after = get(&app, &path, &["owner"]).await;
    assert_eq!(after.status, 200, "{}", after.text);
    assert_eq!(
        download_name(&after.text, "PASSPORT"),
        format!("{}_PASSPORT.png", transfer.document_ref),
        "the type is the record's, not the disk's"
    );
    assert_eq!(
        download_name(&after.text, "PAYMENT"),
        format!("{}_PAYMENT.pdf", transfer.document_ref)
    );
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn the_files_of_an_exchange_an_employee_saved_are_found_by_branch() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let shop = &world.shop;
    let employee = "employee-front";
    // MAIN is ready (rate, open shift) from the fixture's exchanges
    let customer = shop.reserve(save_common::MAIN, save_common::person()).await;
    let passport = shop
        .upload(save_common::MAIN, employee, "image/jpeg", save_common::JPEG)
        .await;
    let slip = shop
        .upload(
            save_common::MAIN,
            employee,
            "application/pdf",
            save_common::PDF,
        )
        .await;
    let mut form = save_common::form("submission-by-the-front-000000", &customer, &passport);
    form.method = "Bank Transfer".to_owned();
    form.payment_ref = "TRX-9".to_owned();
    form.slip = Some(slip.clone());
    let saved = shop
        .workflow
        .save(
            &save_common::branch(save_common::MAIN),
            form,
            employee,
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

    let app = world_app!(world, &fixture);
    for (path, who) in [
        (format!("/documents?ref={}", saved.document_ref), "owner"),
        (
            format!("/staff/documents?ref={}", saved.document_ref),
            "front",
        ),
    ] {
        let answer = get(&app, &path, &[who]).await;
        assert_eq!(answer.status, 200, "{path} as {who}: {}", answer.text);
        assert!(
            answer.text.contains(&passport) && answer.text.contains(&slip),
            "{path} as {who}"
        );
    }
    let page = get(
        &app,
        &format!("/documents?ref={}", saved.document_ref),
        &["owner"],
    )
    .await;
    assert!(download_name(&page.text, "PASSPORT").ends_with(".jpg"));
    assert!(download_name(&page.text, "PAYMENT").ends_with(".pdf"));
    drop(app);
    world.shop.finish().await;
}
