//! #1349 V3-documents (design `docs/design/xchange-v3-wave3-report-documents.md` §A, file **V** of §A.8): the two
//! views that replace the server-written `/documents` and `/staff/documents` pages,
//! `GET /api/v3/views/documents` (`(UserRoute, "documents")`, gate `DocumentsList`) and
//! `GET /api/v3/views/staff_documents` (`(StaffRpc, "documents")`, gate `IdentityDocumentRead`).
//! Written before the implementation by the test-writer claude/5d45b017, on the FE lane controller's order
//! (claude/5edf7bb5; rulings on #1349). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - **All 39 steps** of `fixtures/documents/v2_document_cases.json` (kept unchanged, D-A6a) are replayed against the
//!   views: a step's page path `/documents?Q` asks `/api/v3/views/documents?Q`, and `/staff/documents?Q` asks
//!   `/api/v3/views/staff_documents?Q`, with the step's sessions. Each step's expectation is §A.1's v3 column
//!   (D-A3/D-A4). Steps 2 and 27 (`POST`) ask the page path itself: the screen table's 404.
//! - The answer is `{version, changed: true, data: {ref, links: [{kind, href, download}]}}`. `kind` is
//!   `passport | paymentProof | receipt` in v2's order. The owner's view (`documents`) links `/files/<key>` and names the
//!   download `<ref>_<PASSPORT|PAYMENT|RECEIPT_FULL>.<ext>`. The staff view links `/api/staff/file/<key>` and has
//!   `download: null` (or absent). The links stay as they are today: there is no short-lived link (ruling on #1349;
//!   #557 V15), and the receipt is today's `receipt_key`.
//! - Not found is **404 `documentsNotFound`, field `ref`, sentence `ไม่พบเอกสารรายการ`**, never an empty list. It comes
//!   through the default-`None` `BranchView::refusal` hook (`view.rs`, ruling A on #1349). The last test here pins
//!   that hook's default: a view that does not override it still answers 500 `serverError` for a load error.
//! - Measured on the shared view core (ruled on #1349: each is a known difference from v2, listed in
//!   `docs/design/xchange-v3-known-differences.md` by the ticket's PR). Where it differs from §A.1's text, the core's
//!   answer is pinned:
//!   - an employee who names another branch (steps 18, 19, 33) gets **403 `outOfScope`**, from the core's scope rule
//!     in `Policy::authorize`, where v2 ignored the name. The staff screen never sends a branch, so `/staff/documents`
//!     keeps v2's redirect (**S**).
//!   - RATES and SYSTEM on `staff_documents` (steps 37, 38) get **400 `documentsNotReadable`** (`PolicyError`'s
//!     status).
//! - Mutants these tests must catch (§A.6):
//!   - `StaffDocuments::href` returning the owner's URL;
//!   - the 404 returned as an empty list;
//!   - `INHERITS` of `staff_documents` set to `UserRoute` (steps 25, 37 and 38 turn 200).
//!
//! The exchanges are saved for real through T8b's workflow (`documents_common`, used as it is). Needs
//! `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing skips.

mod common;
mod documents_common;
mod save_common;
#[allow(clippy::duplicate_mod)]
mod shift_holder_common;

use std::sync::Arc;

use actix_web::http::Method;
use actix_web::{App, test, web};
use documents_common::*;
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::v3::common::NoFilter;
use xchange::presentation::dto::v3::transaction_documents::TransactionDocumentsData;
use xchange::presentation::http::v3::view::{BranchView, ViewEndpoint};

const BAD_BRANCH_QUERY: &str = "คำขอไม่ถูกต้อง";
const NOBODY_HAS: &str = "XC-NOPE-1";

// ── what a step expects ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    Owner,
    Staff,
}

#[derive(Debug, Clone)]
enum Want {
    /// The listed exchange's files, in this order, written for this audience.
    Links(&'static str, &'static [&'static str], Audience),
    /// A refusal: status, code, field.
    Refused(u16, &'static str, Option<&'static str>),
    /// The page path's 404 (a wrong method).
    PageNotFound,
}

const CASH: &[&str] = &["passport", "receipt"];
const THREE: &[&str] = &["passport", "paymentProof", "receipt"];

/// §A.1's v3 column, one row per fixture step.
fn want(step: usize) -> Want {
    use Audience::*;
    use Want::*;
    let not_found = Refused(404, "documentsNotFound", Some("ref"));
    match step {
        0 | 1 => Refused(401, "signIn", None),
        2 | 27 => PageNotFound,
        3..=5 => Links("cash", CASH, Owner),
        6 => Links("transfer", THREE, Owner),
        7 => Links("other", THREE, Owner),
        8 | 9 | 12..=15 => not_found,
        10 => Refused(400, "noSuchBranch", Some("branch")),
        11 => Refused(400, "invalidRequest", None),
        16 => Links("other", THREE, Owner),
        17 => Links("transfer", THREE, Owner),
        18 => Refused(403, "outOfScope", None),
        19 => Refused(403, "outOfScope", None),
        20 => Links("other", THREE, Owner),
        21..=23 => Links("cash", CASH, Owner),
        24..=26 => Refused(401, "employeeAuthRequired", None),
        28 | 30 | 37 => Links("cash", CASH, Staff),
        29 | 31 => Links("transfer", THREE, Staff),
        32 => Links("other", THREE, Staff),
        33 => Refused(403, "outOfScope", None),
        34..=36 => not_found,
        // Under RBAC §11.2, Rates holds IdentityDocumentRead (Customers: Full); System (Admin) does not.
        38 => Refused(400, "documentsNotReadable", None),
        other => panic!("the fixture has no step {other}"),
    }
}

// ── one request ──────────────────────────────────────────────────────────

/// `/documents?Q` → `/api/v3/views/documents?Q`; `/staff/documents?Q` → `/api/v3/views/staff_documents?Q`.
fn view_path(page: &str) -> String {
    let (path, query) = match page.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (page, None),
    };
    let name = match path {
        "/documents" => "documents",
        "/staff/documents" => "staff_documents",
        other => panic!("not a documents page: {other}"),
    };
    match query {
        Some(q) => format!("/api/v3/views/{name}?{q}"),
        None => format!("/api/v3/views/{name}"),
    }
}

fn json_of(answer: &Answer, what: &str) -> Value {
    serde_json::from_str(&answer.text).unwrap_or_else(|e| {
        panic!(
            "{what}: a JSON answer ({e}), got {} {:?}",
            answer.status, answer.text
        )
    })
}

/// The expected link of `kind` for `saved`, written for `audience`.
fn expected_link(saved: &Saved, kind: &str, audience: Audience, mime_end: &str) -> Value {
    let key = match kind {
        "passport" => saved.passport.clone(),
        "paymentProof" => saved.slip.clone().expect("a payment proof"),
        "receipt" => saved.receipt.clone(),
        other => panic!("kind {other}"),
    };
    let tag = match kind {
        "passport" => "PASSPORT",
        "paymentProof" => "PAYMENT",
        _ => "RECEIPT_FULL",
    };
    match audience {
        Audience::Owner => json!({
            "kind": kind,
            "href": format!("/files/{key}"),
            "download": format!("{}_{tag}.{mime_end}", saved.document_ref),
        }),
        Audience::Staff => json!({
            "kind": kind,
            "href": format!("/api/staff/file/{key}"),
            "download": Value::Null,
        }),
    }
}

/// The file ending of each of the fixture's exchanges' files, as their recorded type gives it.
fn ending(fixture: &Value, name: &str, kind: &str) -> &'static str {
    if kind == "receipt" {
        return "html";
    }
    let exchange = fixture["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .unwrap();
    let mime = match kind {
        "passport" => exchange["passport"].as_str().unwrap(),
        _ => exchange["slip"].as_str().unwrap(),
    };
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        other => panic!("{other}"),
    }
}

/// `download` absent counts as null: the staff view has none.
fn normalised_links(data: &Value) -> Vec<Value> {
    data["links"]
        .as_array()
        .expect("data.links is a list")
        .iter()
        .map(|l| {
            json!({
                "kind": l["kind"],
                "href": l["href"],
                "download": l.get("download").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

fn assert_links(
    answer: &Answer,
    world: &World,
    fixture: &Value,
    name: &str,
    kinds: &[&str],
    audience: Audience,
    what: &str,
) {
    assert_eq!(answer.status, 200, "{what}: {}", answer.text);
    let body = json_of(answer, what);
    assert_eq!(body["changed"], json!(true), "{what}: changed");
    let saved = world.exchange(name);
    let data = &body["data"];
    assert_eq!(data["ref"], json!(saved.document_ref), "{what}: ref");
    let want: Vec<Value> = kinds
        .iter()
        .map(|k| expected_link(saved, k, audience, ending(fixture, name, k)))
        .collect();
    assert_eq!(
        normalised_links(data),
        want,
        "{what}: links (kind, href, download, order)"
    );
}

fn assert_refused(answer: &Answer, status: u16, code: &str, field: Option<&str>, what: &str) {
    assert_eq!(
        answer.status, status,
        "{what}: status, body {:?}",
        answer.text
    );
    let body = json_of(answer, what);
    assert_eq!(
        body["error"]["code"],
        json!(code),
        "{what}: code, body {body}"
    );
    assert_eq!(
        body["error"].get("field").and_then(Value::as_str),
        field,
        "{what}: field, body {body}"
    );
    assert!(
        body.get("data").is_none(),
        "{what}: a refusal carries no data"
    );
}

// ── the 39 steps ─────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_39_steps_answer_as_section_a1_says() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let steps = fixture["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 39, "the fixture is kept whole (D-A6a)");
    for (index, step) in steps.iter().enumerate() {
        let what = format!("step {index}: {}", step["note"].as_str().unwrap());
        let request = &step["request"];
        let who: Vec<&str> = request["as"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        let method = Method::from_bytes(request["method"].as_str().unwrap().as_bytes()).unwrap();
        let page = world.with_real_values(request["path"].as_str().unwrap());
        match want(index) {
            Want::PageNotFound => {
                assert_eq!(
                    method,
                    Method::POST,
                    "{what}: only the POST steps ask the page"
                );
                let answer = send(&app, method, &page, &who).await;
                assert_eq!(
                    (answer.status, answer.text.as_str()),
                    (404, "Not found"),
                    "{what}"
                );
            }
            Want::Links(name, kinds, audience) => {
                let answer = send(&app, method, &view_path(&page), &who).await;
                assert_links(&answer, &world, &fixture, name, kinds, audience, &what);
            }
            Want::Refused(status, code, field) => {
                let answer = send(&app, method, &view_path(&page), &who).await;
                assert_refused(&answer, status, code, field, &what);
            }
        }
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn not_found_is_404_documents_not_found_with_v2s_sentence_never_an_empty_list() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    for (view, who) in [
        ("documents", "owner"),
        ("documents", "front"),
        ("staff_documents", "front"),
    ] {
        let path = format!("/api/v3/views/{view}?ref={NOBODY_HAS}");
        let answer = get(&app, &path, &[who]).await;
        assert_eq!(answer.status, 404, "{path} as {who}: {}", answer.text);
        assert_eq!(
            json_of(&answer, &path),
            json!({ "error": { "code": "documentsNotFound", "field": "ref", "message": NO_DOCUMENTS } }),
            "{path} as {who}: the whole body"
        );
    }
    drop(app);
    world.shop.finish().await;
}

// ── what an answer may hold ──────────────────────────────────────────────

#[actix_web::test]
async fn an_answer_names_only_the_exchange_asked_for() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let (cash, transfer, other) = (
        world.exchange("cash"),
        world.exchange("transfer"),
        world.exchange("other"),
    );
    for (view, who) in [
        ("documents", "owner"),
        ("documents", "front"),
        ("staff_documents", "front"),
    ] {
        let path = format!("/api/v3/views/{view}?ref={}", cash.document_ref);
        let answer = get(&app, &path, &[who]).await;
        assert_eq!(answer.status, 200, "{path} as {who}: {}", answer.text);
        assert!(
            answer.text.contains(&cash.passport) && answer.text.contains(&cash.receipt),
            "{path}: its own files"
        );
        for foreign in [
            &transfer.document_ref,
            &transfer.passport,
            &transfer.receipt,
            &other.document_ref,
            &other.passport,
        ] {
            assert!(
                !answer.text.contains(foreign.as_str()),
                "{path} as {who}: {foreign} does not belong in this answer"
            );
        }
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn the_staff_view_links_only_the_staff_file_door() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let transfer = world.exchange("transfer");
    let path = format!(
        "/api/v3/views/staff_documents?ref={}",
        transfer.document_ref
    );
    let answer = get(&app, &path, &["front"]).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let body = json_of(&answer, &path);
    let hrefs: Vec<&str> = body["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["href"].as_str().unwrap())
        .collect();
    assert_eq!(hrefs.len(), 3, "a transfer has three documents");
    for href in &hrefs {
        assert!(href.starts_with("/api/staff/file/"), "{href}");
        assert!(!href.starts_with("/files/"), "{href}");
    }
    assert!(!answer.text.contains("\"/files/"), "{}", answer.text);
    drop(app);
    world.shop.finish().await;
}

// ── the branch rows (documents_query_test.rs:48, :68, :147) ──────────────

#[actix_web::test]
async fn the_six_measured_branch_rows() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    for (query, status, code, field, sentence) in [
        (
            "branch=MAIN_1",
            400,
            "invalidRequest",
            None,
            BAD_BRANCH_QUERY,
        ),
        (
            "branch=B-nowhere",
            400,
            "noSuchBranch",
            Some("branch"),
            NO_SUCH_BRANCH,
        ),
        (
            "branch=MAIN",
            404,
            "documentsNotFound",
            Some("ref"),
            NO_DOCUMENTS,
        ),
        (
            "branch=B-nowhere&branch=MAIN",
            400,
            "noSuchBranch",
            Some("branch"),
            NO_SUCH_BRANCH,
        ),
        (
            "branch=MAIN&branch=B-nowhere",
            404,
            "documentsNotFound",
            Some("ref"),
            NO_DOCUMENTS,
        ),
        (
            "branch=MAIN_1&branch=MAIN",
            400,
            "invalidRequest",
            None,
            BAD_BRANCH_QUERY,
        ),
    ] {
        let path = format!("/api/v3/views/documents?{query}&ref={NOBODY_HAS}");
        let answer = get(&app, &path, &["owner"]).await;
        assert_refused(&answer, status, code, field, &path);
        assert_eq!(
            json_of(&answer, &path)["error"]["message"],
            json!(sentence),
            "{path}"
        );
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn the_eleven_branch_names_of_issue_139() {
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
        let path = format!(
            "/api/v3/views/documents?branch={branch}&ref={}",
            cash.document_ref
        );
        let answer = get(&app, &path, &["owner"]).await;
        assert_eq!(
            (answer.status, json_of(&answer, &path)),
            (
                400,
                json!({ "error": { "code": "invalidRequest", "message": BAD_BRANCH_QUERY } })
            ),
            "{path}: the view core's refusal, byte-identical (§A.7)"
        );
    }
    for branch in [seventy.as_str(), "-", "b-1"] {
        let path = format!(
            "/api/v3/views/documents?branch={branch}&ref={}",
            cash.document_ref
        );
        let answer = get(&app, &path, &["owner"]).await;
        assert_refused(&answer, 400, "noSuchBranch", Some("branch"), &path);
    }
    let path = format!("/api/v3/views/documents?branch=&ref={}", cash.document_ref);
    let answer = get(&app, &path, &["owner"]).await;
    assert_links(
        &answer,
        &world,
        &fixture,
        "cash",
        CASH,
        Audience::Owner,
        &path,
    );
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
        "/api/v3/views/documents?branch=BR-2&branch=MAIN&ref={}",
        other.document_ref
    );
    let answer = get(&app, &path, &["owner"]).await;
    assert_links(
        &answer,
        &world,
        &fixture,
        "other",
        THREE,
        Audience::Owner,
        &path,
    );
    let path = format!(
        "/api/v3/views/documents?branch=MAIN&branch=BR-2&ref={}",
        other.document_ref
    );
    let answer = get(&app, &path, &["owner"]).await;
    assert_refused(&answer, 404, "documentsNotFound", Some("ref"), &path);
    drop(app);
    world.shop.finish().await;
}

// ── the files (documents_query_test.rs:169, :199) ────────────────────────

fn download_of(body: &Value, kind: &str) -> String {
    body["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["kind"] == kind)
        .unwrap_or_else(|| panic!("a {kind} link in {body}"))["download"]
        .as_str()
        .unwrap_or_else(|| panic!("the owner's {kind} link has a download name: {body}"))
        .to_owned()
}

#[actix_web::test]
async fn a_file_whose_bytes_are_gone_keeps_its_type() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = world_app!(world, &fixture);
    let transfer = world.exchange("transfer");
    let path = format!("/api/v3/views/documents?ref={}", transfer.document_ref);
    let before = get(&app, &path, &["owner"]).await;
    assert_eq!(before.status, 200, "{}", before.text);
    let before = json_of(&before, &path);
    assert_eq!(
        download_of(&before, "passport"),
        format!("{}_PASSPORT.png", transfer.document_ref)
    );
    assert_eq!(
        download_of(&before, "paymentProof"),
        format!("{}_PAYMENT.pdf", transfer.document_ref)
    );

    for key in [&transfer.passport, transfer.slip.as_ref().expect("a slip")] {
        std::fs::remove_file(world.shop.dir.join(key)).expect("remove the stored bytes");
    }
    let after = get(&app, &path, &["owner"]).await;
    assert_eq!(after.status, 200, "{}", after.text);
    let after = json_of(&after, &path);
    assert_eq!(
        download_of(&after, "passport"),
        format!("{}_PASSPORT.png", transfer.document_ref),
        "the type is the record's, not the disk's"
    );
    assert_eq!(
        download_of(&after, "paymentProof"),
        format!("{}_PAYMENT.pdf", transfer.document_ref)
    );
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn the_files_an_employee_saved_are_in_both_views() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let shop = &world.shop;
    let employee = "employee-front";
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
    let mut form = save_common::form("submission-by-the-front-v3docs", &customer, &passport);
    form.method = "Bank Transfer".to_owned();
    form.payment_ref = "TRX-9".to_owned();
    form.slip = Some(slip.clone());
    shift_holder_common::open_shift_for(
        &shop.pool,
        save_common::MAIN,
        employee,
        &[("THB", "1000000"), ("USD", "1000")],
    )
    .await;
    let employee_id = shift_holder_common::user_of(&shop.pool, employee).await;
    let saved = shop
        .workflow
        .save(
            &save_common::branch(save_common::MAIN),
            form,
            // OR-7.m G1b: rule 1 (#3797)
            &employee_id,
            save_common::at(100),
        )
        .await
        .expect("an employee's save");

    let app = world_app!(world, &fixture);
    for (view, who) in [("documents", "owner"), ("staff_documents", "front")] {
        let path = format!("/api/v3/views/{view}?ref={}", saved.document_ref);
        let answer = get(&app, &path, &[who]).await;
        assert_eq!(answer.status, 200, "{path} as {who}: {}", answer.text);
        assert!(
            answer.text.contains(&passport) && answer.text.contains(&slip),
            "{path} as {who}"
        );
    }
    let path = format!("/api/v3/views/documents?ref={}", saved.document_ref);
    let body = json_of(&get(&app, &path, &["owner"]).await, &path);
    assert!(download_of(&body, "passport").ends_with(".jpg"));
    assert!(download_of(&body, "paymentProof").ends_with(".pdf"));
    drop(app);
    world.shop.finish().await;
}

// ── no reader, no database (documents_test.rs:237, :315) ─────────────────

#[actix_web::test]
async fn without_a_session_reader_the_staff_view_refuses_with_401() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let state = world.state();
    let app = TestApp::new(
        test::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state))).await,
        |t| t.to_request(),
    );
    let reference = &world.exchange("cash").document_ref;
    let answer = get(
        &app,
        &format!("/api/v3/views/staff_documents?ref={reference}"),
        &[],
    )
    .await;
    assert_eq!(answer.status, 401, "{}", answer.text);
    let answer = get(
        &app,
        &format!("/api/v3/views/documents?ref={reference}"),
        &[],
    )
    .await;
    assert_eq!(answer.status, 401, "{}", answer.text);
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn when_the_database_is_down_the_view_answers_500() {
    let fixture = fixture();
    let dir = std::env::temp_dir().join(save_common::unique("xchange-v3-documents-down"));
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().unwrap(),
        )])),
        common::unreachable_pool(),
    );
    let reader: Arc<dyn xchange::presentation::pages::SessionReader> =
        Arc::new(FakeSessions::from_fixture(&fixture));
    let app = TestApp::new(
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await,
        |t| t.to_request(),
    );
    for (view, who) in [
        ("documents", "owner"),
        ("documents", "front"),
        ("staff_documents", "front"),
    ] {
        let path = format!("/api/v3/views/{view}?ref=XC-1");
        let answer = get(&app, &path, &[who]).await;
        assert_eq!(
            (answer.status, json_of(&answer, &path)),
            (
                500,
                json!({ "error": { "code": "serverError", "message": SERVER_ERROR } })
            ),
            "{path} as {who}: nothing of the cause is shown"
        );
    }
}

// ── the hook's default (ruling A on #1349) ───────────────────────────────

/// A view that fails to load and does not override `BranchView::refusal`.
struct FailingView;

impl BranchView for FailingView {
    const NAME: &'static str = "test_failing_documents";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::UserRoute, "documents");
    type Data = TransactionDocumentsData;
    type Filter = NoFilter;

    async fn load(
        &self,
        _branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        // the very error the documents views turn into a 404: here nothing turns it
        Err(AppError::Domain(NO_DOCUMENTS.to_owned()))
    }
}

#[actix_web::test]
async fn a_view_that_does_not_override_refusal_still_answers_500_for_a_load_error() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let state = world.state();
    let reader: Arc<dyn xchange::presentation::pages::SessionReader> =
        Arc::new(FakeSessions::from_fixture(&fixture));
    let app = TestApp::new(
        test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(|cfg| ViewEndpoint::register(cfg, Arc::new(FailingView)))
                .app_data(web::Data::from(reader)),
        )
        .await,
        |t| t.to_request(),
    );
    let answer = get(&app, "/api/v3/views/test_failing_documents", &["owner"]).await;
    assert_eq!(
        (answer.status, json_of(&answer, "the failing view")),
        (
            500,
            json!({ "error": { "code": "serverError", "message": SERVER_ERROR } })
        ),
        "the default hook keeps the core's 500"
    );
    drop(app);
    world.shop.finish().await;
}

// ── the generated type (web/src/shared/api/generated/documents.ts) ──────

fn exported_documents_ts() -> String {
    let view = xchange::presentation::http::v3::VIEWS
        .iter()
        .find(|v| v.name == "documents")
        .unwrap_or_else(|| {
            panic!(
                "documents is in the generated view table: {:?}",
                xchange::presentation::http::v3::VIEWS
                    .iter()
                    .map(|v| v.name)
                    .collect::<Vec<_>>()
            )
        });
    let dir = std::env::temp_dir().join(save_common::unique("xchange-documents-ts"));
    std::fs::create_dir_all(&dir).unwrap();
    (view.export_types)(&dir).unwrap_or_else(|e| panic!("export documents: {e}"));
    let ts = std::fs::read_to_string(dir.join("documents.ts"))
        .unwrap_or_else(|e| panic!("documents writes documents.ts: {e}"));
    let _ = std::fs::remove_dir_all(&dir);
    ts
}

#[actix_web::test]
async fn the_generated_type_carries_ref_and_links_of_kind_href_and_download() {
    let ts = exported_documents_ts();
    for needle in [
        "ref: string",
        "links: Array<DocumentsLink>",
        "href: string",
        "download: string | null",
        "\"passport\"",
        "\"paymentProof\"",
        "\"receipt\"",
    ] {
        assert!(ts.contains(needle), "{needle}\n{ts}");
    }
    assert!(!ts.contains("bigint"), "no bigint\n{ts}");
}

#[actix_web::test]
async fn the_generated_file_is_committed() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/documents.ts");
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/documents.ts: {e}"));
    assert_eq!(
        committed,
        exported_documents_ts(),
        "documents.ts is what the DTOs generate"
    );
}
