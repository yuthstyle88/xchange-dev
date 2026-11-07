//! OL-6 wave B docs-reader-fix (owner finding 1B on #564, issuecomment-6705475792; findings
//! `/tmp/ol6-wB/docs-download-name.FINDINGS.md`): the documents of an exchange completed through the v3 commands.
//! Written before the fix by the ticket author claude/studio-claude-17 for the sub-controller claude/1cc51615.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! On xc-v3 every exchange's documents page showed `…_PASSPORT.bin` and `…_RECEIPT_FULL.bin` with `href="/files/"`:
//! the v3 completion (`complete_numbered_in`) writes the receipt's file key to `receipts.document_ref`, never to
//! `transaction_files`, and the documents read the key from `transaction_files` only; a backdated exchange keyed
//! without a passport has no PASSPORT key at all, yet still got a passport link.
//! - DR-1 a backdated exchange with a passport: the passport and the receipt, each with its real key, type and name.
//! - DR-2 a backdated exchange without a passport: the receipt only; no link is ever `/files/`.
//! - DR-3 the owner's view answers the same names and hrefs.
//! - DR-4 the download name `<ref>-<passport|slip|receipt>.<ext>` (owner 1B), the ending from the recorded type
//!   (`bin` only for an unknown one), characters a file name cannot hold replaced by `_`.
//!
//! The exchanges are keyed for real through `POST /api/transactions/backdated` (the harness of
//! `backdate_http_test.rs`). Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use pos_infra::transactions::Transactor;
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::lg4_fund;
use xc_auth::domain::access::Role;
use xc_cash::domain::posting::CashPlace;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xchange::application::workflows::documents::{
    DocumentLink, DocumentsLookup, PASSPORT, PAYMENT_PROOF, RECEIPT,
};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::files::file_service::FileService;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const OWNER: &str = "owner-bk";
const MANAGER: &str = "manager-bk";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
/// 2026-11-04 13:23 ICT: the first Telegram order of the owner's replay.
const PAID_AT: &str = "2026-11-04T13:23:00+07:00";
const VALUE_DATE: &str = "2026-11-04";

static PII: Once = Once::new();

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        let callers = match spec.as_str() {
            "owner" => test_callers::callers(Some(test_callers::owner(OWNER)), None),
            "manager" => test_callers::callers(
                None,
                Some(test_callers::staff(
                    MANAGER,
                    &[Role::BranchManager],
                    BranchId::new("MAIN"),
                )),
            ),
            "teller" => test_callers::callers(
                None,
                Some(test_callers::staff(
                    TELLER,
                    &[Role::Teller],
                    BranchId::new("MAIN"),
                )),
            ),
            other => panic!("unexpected caller {other:?}"),
        };
        Box::pin(async move { callers })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// The fixture: MAIN with no shift, a reserved LIVE customer, a passport scan owned by each
/// caller, and the SAFE holding THB `safe_thb`.
struct Bk {
    desk: Desk,
    scratch: std::path::PathBuf,
    live: String,
}

/// A customer reserved as the desk reserves one (PII sealed), in `mode` (`LIVE` | `DEMO`).
async fn reserve(desk: &Desk, mode: &str) -> String {
    CustomerService::new(BranchTx::new(Transactor::new(desk.db.pool.clone())))
        .reserve(
            &BranchId::new("MAIN"),
            ReserveCustomer {
                name: "Somchai Jaidee".to_owned(),
                passport_no: format!("P{}", &uuid::Uuid::new_v4().simple().to_string()[..19])
                    .to_uppercase(),
                nationality: "THA".to_owned(),
                birth_date: String::new(),
                passport_expiry: String::new(),
                mode: mode.to_owned(),
            },
            whole_second_now(),
        )
        .await
        .unwrap_or_else(|e| panic!("reserve a {mode} customer: {e}"))
        .customer_id
}

async fn start(safe_thb: &str) -> Bk {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
    let desk = Desk::start().await;
    let live = reserve(&desk, "LIVE").await;
    for who in [OWNER, MANAGER, TELLER] {
        desk.db
            .exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('PASSPORT-BK-{who}', 'MAIN', '{who}', 'image/jpeg', 'sha-bk-{who}', now())"
            ))
            .await;
    }
    lg4_fund(
        &desk.db.pool,
        CashPlace::Bank,
        CashPlace::Safe(BranchId::new("MAIN")),
        "THB",
        safe_thb,
    )
    .await;
    let scratch = std::env::temp_dir().join(unique("xc-bk"));
    std::fs::create_dir_all(&scratch).unwrap();
    Bk {
        desk,
        scratch,
        live,
    }
}

/// B1's body: BUY 500 USD cash at 32.15, paid 2026-11-04 13:23 ICT.
fn body(who: &str, customer: &str) -> Value {
    json!({
        "customerId": customer,
        "side": "BUY",
        "currency": "USD",
        "quantity": "500",
        "rate": "32.15",
        "payment": {
            "method": "Cash",
            "reference": "",
            "account": "",
            "bankFeeThb": "0.00",
            "paidAt": PAID_AT,
        },
        "valueDate": VALUE_DATE,
        "passport": { "uploadKey": format!("PASSPORT-BK-{who}") },
        "notes": "Telegram order 1",
    })
}

async fn call(
    desk: &Desk,
    scratch: &std::path::Path,
    who: &str,
    req: atest::TestRequest,
) -> Answer {
    let dir = scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = req
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn finish(desk: Desk, scratch: std::path::PathBuf) {
    let _ = std::fs::remove_dir_all(&scratch);
    desk.db.finish().await;
}

/// Keys one backdated exchange as the owner, with or without a passport, and returns its document ref.
async fn keyed(bk: &Bk, with_passport: bool) -> String {
    let mut b = body(OWNER, &bk.live);
    if !with_passport {
        b.as_object_mut().unwrap().remove("passport");
    }
    let req = atest::TestRequest::post()
        .uri("/api/transactions/backdated")
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("dr")))
        .set_payload(b.to_string());
    let a = call(&bk.desk, &bk.scratch, "owner", req).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["status"], "COMPLETED", "{}", a.text);
    a.body["documentRef"]
        .as_str()
        .unwrap_or_else(|| panic!("a document ref in {}", a.text))
        .to_owned()
}

fn lookup(bk: &Bk) -> DocumentsLookup {
    let pool = bk.desk.db.pool.clone();
    let tx = || BranchTx::new(Transactor::new(pool.clone()));
    DocumentsLookup::new(
        TransactionService::new(Arc::new(PgTransactionRepository::new(pool.clone())), tx()),
        BranchService::new(tx()),
        FileService::new(pool.clone(), bk.scratch.join("uploads")),
    )
}

/// The receipt's file key, as the v3 completion recorded it.
async fn receipt_key(bk: &Bk, document_ref: &str) -> String {
    bk.desk
        .db
        .text(&format!(
            "SELECT r.document_ref FROM receipts r JOIN transactions t ON t.id = r.transaction_id \
             WHERE t.document_ref = '{document_ref}' AND r.revision = 1"
        ))
        .await
        .expect("the v3 completion wrote a receipts row")
}

fn link(label: &str, url: &str, mime: &str) -> DocumentLink {
    DocumentLink {
        label: label.to_owned(),
        url: url.to_owned(),
        mime: mime.to_owned(),
    }
}

#[actix_web::test]
async fn dr1_a_backdated_exchange_with_a_passport_links_both_files_with_their_types() {
    let bk = start("100000.00").await;
    let r = keyed(&bk, true).await;
    let receipt = receipt_key(&bk, &r).await;
    let found = lookup(&bk)
        .find(&BranchId::new("MAIN"), &r)
        .await
        .expect("found");
    assert_eq!(
        found.links,
        vec![
            link(
                PASSPORT,
                &format!("/files/PASSPORT-BK-{OWNER}"),
                "image/jpeg"
            ),
            link(RECEIPT, &format!("/files/{receipt}"), "text/html"),
        ],
        "the passport and the receipt the v3 completion recorded"
    );
    assert_eq!(found.links[0].file_name(&r), format!("{r}-passport.jpg"));
    assert_eq!(found.links[1].file_name(&r), format!("{r}-receipt.html"));
    finish(bk.desk, bk.scratch).await;
}

#[actix_web::test]
async fn dr2_a_backdated_exchange_without_a_passport_links_the_receipt_only() {
    let bk = start("100000.00").await;
    let r = keyed(&bk, false).await;
    let receipt = receipt_key(&bk, &r).await;
    let found = lookup(&bk)
        .find(&BranchId::new("MAIN"), &r)
        .await
        .expect("found");
    assert_eq!(
        found.links,
        vec![link(RECEIPT, &format!("/files/{receipt}"), "text/html")],
        "no passport link without a passport"
    );
    assert!(
        found.links.iter().all(|l| l.url != "/files/"),
        "{:?}",
        found.links
    );
    finish(bk.desk, bk.scratch).await;
}

#[actix_web::test]
async fn dr3_the_owners_documents_view_answers_the_real_hrefs_and_names() {
    let bk = start("100000.00").await;
    let r = keyed(&bk, true).await;
    let receipt = receipt_key(&bk, &r).await;
    let req =
        atest::TestRequest::get().uri(&format!("/api/v3/views/documents?ref={r}&branch=MAIN"));
    let a = call(&bk.desk, &bk.scratch, "owner", req).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(
        a.body["data"]["links"],
        json!([
            { "kind": "passport", "href": format!("/files/PASSPORT-BK-{OWNER}"), "download": format!("{r}-passport.jpg") },
            { "kind": "receipt", "href": format!("/files/{receipt}"), "download": format!("{r}-receipt.html") },
        ]),
        "{}",
        a.text
    );
    finish(bk.desk, bk.scratch).await;
}

#[test]
fn dr4_the_download_name_is_ref_dash_kind_dot_ending() {
    let r = "XC-20261105-000023";
    assert_eq!(
        link(PASSPORT, "/files/a", "image/jpeg").file_name(r),
        format!("{r}-passport.jpg")
    );
    assert_eq!(
        link(PAYMENT_PROOF, "/files/b", "image/png").file_name(r),
        format!("{r}-slip.png")
    );
    assert_eq!(
        link(RECEIPT, "/files/c", "application/pdf").file_name(r),
        format!("{r}-receipt.pdf")
    );
    assert_eq!(
        link(RECEIPT, "/files/c", "text/html").file_name(r),
        format!("{r}-receipt.html")
    );
    assert_eq!(
        link(PASSPORT, "/files/a", "").file_name(r),
        format!("{r}-passport.bin")
    );
    assert_eq!(
        link(PASSPORT, "/files/a", "application/x-unknown").file_name(r),
        format!("{r}-passport.bin")
    );
    assert_eq!(
        link(RECEIPT, "/files/c", "text/html").file_name("XC/1\\2:3*4?5\"6<7>8|9\u{7}0"),
        "XC_1_2_3_4_5_6_7_8_9_0-receipt.html",
        "/ \\ : * ? \" < > | and control characters become _"
    );
}
