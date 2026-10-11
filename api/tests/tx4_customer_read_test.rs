//! Ticket #672 (TX-4) Q1 — the non-decrypting customer read create uses instead of
//! `CustomerService::confirm_in`, against a real PostgreSQL.
//! Written before the implementation (claude/14cc24bd, on the owner's order, 2026-10-10), after
//! the lane controller's ruling Q1 = B on #672 (issuecomment-6094343896). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The ruling: create does not decrypt the passport. A new read on `CustomerService` answers the
//! customer's mode, display name and nationality, which are stored in clear
//! (`customers.mode`, `customers.display_name`, `customer_identities.nationality`), and refuses a
//! customer that is not the branch's organisation's, is CLOSED, or has no ACTIVE PASSPORT
//! identity. K8 (`docs/design/blueprint/customer-identity-consent.md`): a value is shown only
//! through `reveal`.
//!
//! **This binary never installs a PII key**, and every identity is seeded with a ciphertext that no
//! key opens. A read that decrypted would fail here; one that reads the clear columns passes.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips.
//!
//! Signatures this test assumes (the method is [required] by the ruling; its name and shape are
//! "choice", following the research draft on #672):
//! - `CustomerService::exchange_customer_in(tx: &mut sqlx::Transaction<'_, Postgres>, branch:
//!   &BranchId, customer_id: &str) -> AppResult<ExchangeCustomer>` (async, associated like
//!   `confirm_in`), in `api/crates/xc-customer/src/application/customer/service.rs`, reached
//!   through `xc_exchange::application::customer::service` as every `CustomerService` item is.
//! - choice: `ExchangeCustomer { id: String, mode: Mode, display_name: String, nationality:
//!   String }` in the same file, fields public, `Debug` + `PartialEq`.
//! - Refusals as `Err(AppError::Domain(m))`:
//!   - no ACTIVE PASSPORT identity ⇒ today's `NOT_THIS_PASSPORT` sentence (the ruling narrows its
//!     meaning to "this customer has no active passport");
//!   - choice: another organisation's customer, or no such customer ⇒ code `outOfScope`;
//!   - choice: a CLOSED customer ⇒ code `validationFailed`.

mod quote_common;

use pos_error::{AppError, AppResult};
use quote_common::*;
use xc_exchange::application::customer::service::{CustomerService, ExchangeCustomer};
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_kernel::ids::BranchId;

const ORG_2: &str = "00000000-0000-0000-0000-0000000000d2";
/// Today's sentence (`xc-customer` `NOT_THIS_PASSPORT`).
const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";

/// A customer of `org` with one identity of `kind` in `status`, whose ciphertext no key opens.
async fn seed(
    db: &TestDb,
    org: &str,
    id: &str,
    mode: &str,
    status: &str,
    identity: Option<(&str, &str, Option<&str>)>,
) {
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, status, created_at) \
         VALUES ('{id}', '{org}', '{id}', 'Name of {id}', '{mode}', '{status}', now())"
    ))
    .await;
    if let Some((kind, identity_status, nationality)) = identity {
        let nationality = nationality.map_or("NULL".to_owned(), |n| format!("'{n}'"));
        db.exec(&format!(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, \
                 key_id, lookup_hash, nationality, status) \
             VALUES (gen_random_uuid(), '{org}', '{id}', '{kind}', '\\x00ff00ff'::bytea, \
                 'no-such-key', gen_random_uuid()::text::bytea, {nationality}, '{identity_status}')"
        ))
        .await;
    }
}

async fn read(db: &TestDb, branch: &str, id: &str) -> AppResult<ExchangeCustomer> {
    let mut tx = db.pool.begin().await.unwrap();
    let got = CustomerService::exchange_customer_in(&mut tx, &BranchId::new(branch), id).await;
    tx.commit().await.unwrap();
    got
}

fn code<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m
            .split(|c: char| c == ':' || c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_owned(),
        other => panic!("expected Err(AppError::Domain(\"<code>: …\")), got {other:?}"),
    }
}

fn sentence<T: std::fmt::Debug>(got: AppResult<T>) -> String {
    match got {
        Err(AppError::Domain(m)) => m,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

#[tokio::test]
async fn the_read_answers_mode_name_and_nationality_without_any_key() {
    let db = TestDb::start().await;
    seed(
        &db,
        ORG_1,
        "CU-D",
        "DEMO",
        "ACTIVE",
        Some(("PASSPORT", "ACTIVE", Some("JPN"))),
    )
    .await;
    seed(
        &db,
        ORG_1,
        "CU-L",
        "LIVE",
        "ACTIVE",
        Some(("PASSPORT", "ACTIVE", Some("THA"))),
    )
    .await;

    assert_eq!(
        read(&db, "MAIN", "CU-D")
            .await
            .expect("read with no PII key installed"),
        ExchangeCustomer {
            id: "CU-D".to_owned(),
            mode: Mode::Demo,
            display_name: "Name of CU-D".to_owned(),
            nationality: "JPN".to_owned(),
        },
        "Q1 = B: the clear columns only; no key exists in this binary"
    );
    assert_eq!(read(&db, "MAIN", "CU-L").await.unwrap().mode, Mode::Live);
    db.finish().await;
}

#[tokio::test]
async fn a_customer_without_an_active_passport_is_refused() {
    let db = TestDb::start().await;
    seed(&db, ORG_1, "CU-NONE", "DEMO", "ACTIVE", None).await;
    seed(
        &db,
        ORG_1,
        "CU-REVOKED",
        "DEMO",
        "ACTIVE",
        Some(("PASSPORT", "REVOKED", Some("THA"))),
    )
    .await;
    seed(
        &db,
        ORG_1,
        "CU-PHONE",
        "DEMO",
        "ACTIVE",
        Some(("PHONE", "ACTIVE", None)),
    )
    .await;

    for id in ["CU-NONE", "CU-REVOKED", "CU-PHONE"] {
        assert_eq!(
            sentence(read(&db, "MAIN", id).await),
            NOT_THIS_PASSPORT,
            "{id}: no ACTIVE PASSPORT identity"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn another_organisations_customer_or_an_unknown_one_is_out_of_scope() {
    let db = TestDb::start().await;
    db.second_organisation(ORG_2, "ORG2", "X1").await;
    seed(
        &db,
        ORG_2,
        "CU-THEIRS",
        "DEMO",
        "ACTIVE",
        Some(("PASSPORT", "ACTIVE", Some("THA"))),
    )
    .await;

    assert_eq!(
        code(read(&db, "MAIN", "CU-THEIRS").await),
        "outOfScope",
        "ORG_2's customer at MAIN"
    );
    assert_eq!(
        code(read(&db, "MAIN", "CU-NOBODY").await),
        "outOfScope",
        "no such customer"
    );
    assert_eq!(
        read(&db, "X1", "CU-THEIRS")
            .await
            .expect("its own organisation's branch")
            .id,
        "CU-THEIRS"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_closed_customer_is_refused() {
    let db = TestDb::start().await;
    seed(
        &db,
        ORG_1,
        "CU-CLOSED",
        "DEMO",
        "CLOSED",
        Some(("PASSPORT", "ACTIVE", Some("THA"))),
    )
    .await;

    assert_eq!(
        code(read(&db, "MAIN", "CU-CLOSED").await),
        "validationFailed"
    );
    db.finish().await;
}
