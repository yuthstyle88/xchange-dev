//! The documents of one exchange: the use case (`DocumentsLookup`) and the two pages that show
//! it, `GET /documents` and `GET /staff/documents`.
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//!
//! The exchanges are saved for real through ticket T8b's workflow. These tests need a real
//! PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`); without it they
//! fail, none skips.

mod common;
mod documents_common;
mod save_common;

use documents_common::*;
use pos_error::AppError;
use serde_json::json;
use xc_kernel::ids::BranchId;
use xchange::application::workflows::documents::{DocumentLink, Documents};

fn refusal<T: std::fmt::Debug>(result: Result<T, AppError>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected a refusal (AppError::Domain), got {other:?}"),
    }
}

fn link(label: &str, key: &str, mime: &str) -> DocumentLink {
    DocumentLink {
        label: label.to_owned(),
        url: format!("/files/{key}"),
        mime: mime.to_owned(),
    }
}

// ── the use case ─────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_fixture_is_the_one_recorded_from_v2() {
    let fixture = fixture();
    assert_eq!(
        fixture["counts"],
        json!({ "requests": 39, "byStatus": { "200": 17, "302": 9, "303": 2, "404": 11 }, "devicesOff": 4 })
    );
    assert_eq!(
        fixture["steps"].as_array().unwrap().len(),
        39,
        "no step was removed from the fixture"
    );
}

#[actix_web::test]
async fn the_documents_of_an_exchange_are_its_passport_its_payment_proof_and_its_receipt() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let lookup = world.lookup();

    let cash = world.exchange("cash");
    let found = lookup
        .find(&BranchId::new("MAIN"), &cash.document_ref)
        .await
        .expect("found");
    assert_eq!(
        found,
        Documents {
            document_ref: cash.document_ref.clone(),
            links: vec![
                link("พาสปอร์ต", &cash.passport, "image/jpeg"),
                link("ใบเสร็จ", &cash.receipt, "text/html"),
            ],
        },
        "a cash exchange has no payment proof: two links, in v2's order"
    );
    // the links are the files the saved exchange names
    let stored: save_common::Transaction = world
        .shop
        .transactions
        .find(&BranchId::new("MAIN"), &cash.document_ref)
        .await
        .expect("read the exchange")
        .expect("the exchange is stored");
    assert_eq!(
        found.links[0].url,
        format!("/files/{}", stored.passport_key)
    );
    assert_eq!(found.links[1].url, format!("/files/{}", stored.receipt_key));
    assert_eq!(stored.slip_key, None);
    // what the RPC action `documents` answers (ticket T11a calls this)
    assert_eq!(
        found.to_json(),
        json!({ "ref": cash.document_ref, "links": [
            ["พาสปอร์ต", format!("/files/{}", cash.passport)],
            ["ใบเสร็จ", format!("/files/{}", cash.receipt)],
        ] })
    );

    let transfer = world.exchange("transfer");
    let found = lookup
        .find(&BranchId::new("MAIN"), &transfer.document_ref)
        .await
        .expect("found");
    assert_eq!(
        found.links,
        vec![
            link("พาสปอร์ต", &transfer.passport, "image/png"),
            link(
                "หลักฐานชำระเงิน",
                transfer.slip.as_deref().unwrap(),
                "application/pdf"
            ),
            link("ใบเสร็จ", &transfer.receipt, "text/html"),
        ]
    );
    assert_eq!(found.to_json()["links"].as_array().unwrap().len(), 3);

    let other = world.exchange("other");
    let found = lookup
        .find(&BranchId::new("BR-2"), &other.document_ref)
        .await
        .expect("found");
    assert_eq!(
        found.links[0],
        link("พาสปอร์ต", &other.passport, "image/webp")
    );
    assert_eq!(
        found.links[1],
        link(
            "หลักฐานชำระเงิน",
            other.slip.as_deref().unwrap(),
            "image/jpeg"
        )
    );
    world.shop.finish().await;
}

#[actix_web::test]
async fn an_exchange_is_found_only_in_its_own_branch_and_only_when_it_is_live() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let lookup = world.lookup();
    let (cash, other, demo) = (
        world.exchange("cash"),
        world.exchange("other"),
        world.exchange("demo"),
    );
    let main = BranchId::new("MAIN");
    let br2 = BranchId::new("BR-2");

    assert_eq!(
        refusal(lookup.find(&br2, &cash.document_ref).await),
        NO_DOCUMENTS
    );
    assert_eq!(
        refusal(lookup.find(&main, &other.document_ref).await),
        NO_DOCUMENTS
    );
    assert_eq!(
        refusal(lookup.find(&main, &demo.document_ref).await),
        NO_DOCUMENTS,
        "a demo exchange has none"
    );
    for unknown in [
        "",
        " ",
        "XC-NOPE",
        &cash.document_ref.to_lowercase(),
        &format!("{} ", cash.document_ref),
        &format!(" {}", cash.document_ref),
    ] {
        if unknown == cash.document_ref {
            continue; // a number without letters is its own lower case
        }
        assert_eq!(
            refusal(lookup.find(&main, unknown).await),
            NO_DOCUMENTS,
            "ref {unknown:?}"
        );
    }
    // the branch is judged first, as v2's engine opens the branch's book before it looks
    for branch in ["NOWHERE", "MAIN_1", "main", ""] {
        assert_eq!(
            refusal(
                lookup
                    .find(&BranchId::new(branch), &cash.document_ref)
                    .await
            ),
            NO_SUCH_BRANCH,
            "branch {branch:?}"
        );
        assert_eq!(
            refusal(lookup.find(&BranchId::new(branch), "XC-NOPE").await),
            NO_SUCH_BRANCH
        );
    }
    world.shop.finish().await;
}

// ── the two pages ────────────────────────────────────────────────────────
// Moved to the views and the screen by #1349 (design §A.8, D-A6a): `v3_documents_views_test.rs` (V) and
// `v3_documents_screens_test.rs` (T).
