//! Ticket #757 (RE-4c) — `adminSnapshot.transactions[].rate` keeps v2's per-1 rate while the
//! row stores the rate per `unit_size`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.9 R1 (a row stores the line's rate per
//! `unit_size`) and R3 (the v2-compat wire stays per 1, through `CompatRate::per_one`); the
//! ticket author's rulings of 2026-10-10: `adminSnapshot.transactions[].rate` and
//! `referenceRate` are v2's shape, so per 1; each row is divided by its own stored `unit_size`
//! (R9), never the register's.
//!
//! An exchange is saved through `SaveTransactionWorkflow` (the harness of `save_common`) and
//! read back through the snapshot builder over the sources the routes build
//! (`ServiceSnapshotSources`), as `snapshot_members_test.rs` builds them.
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `adminSnapshot.transactions[].rate` and `referenceRate` = the stored value ÷
//!   the row's stored `unit_size` (`transactions.unit_size`), whatever
//!   `currencies.unit_size` says now and whatever the row's `published_rate_line_id`;
//! - the merged signatures of `SnapshotBuilder::{new, admin_snapshot}`,
//!   `ServiceSnapshotSources::new`, `AccountService::new`, `PasswordHasher::with_cost`,
//!   `CustodyService::new` and `RateService::new`, as on the merge base `d6ce058`.

mod compat_common;
mod save_common;
mod snapshot_common;

use std::sync::Arc;

use rust_decimal::Decimal;
use save_common::rates_re4c_common::{d, publish};
use save_common::{
    JPEG, MAIN, OWNER, SaveTransaction, Shop, TransactionService, at, branch, form, person,
};
use serde_json::{Value, json};
use snapshot_common::{NoPorts, RecordedSource};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::compat::snapshot::SnapshotBuilder;
use xchange::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(pos_infra::transactions::Transactor::new(shop.pool.clone()))
}

/// The snapshot builder over the sources the routes build (`presentation/http/rpc.rs`).
fn builder(shop: &Shop) -> SnapshotBuilder {
    let sources = ServiceSnapshotSources::new(
        TransactionService::new(
            Arc::new(PgTransactionRepository::new(shop.pool.clone())),
            branch_tx(shop),
        ),
        AccountService::new(
            Arc::new(PgAccountRepository::new(shop.pool.clone())),
            PasswordHasher::with_cost(8, 1, 1),
        ),
    );
    let custody = CustodyService::new(
        Arc::new(PgCustodyRepository::new(shop.pool.clone())),
        shop.shifts.clone(),
        Arc::new(NoPorts),
        branch_tx(shop),
    );
    SnapshotBuilder::new(
        BranchService::new(branch_tx(shop)),
        RateService::new(branch_tx(shop), Arc::new(RecordedSource::default())),
        shop.customers.clone(),
        shop.shifts.clone(),
        custody,
        Arc::new(sources),
    )
}

fn exchange(
    submission: &str,
    customer: &str,
    passport: &str,
    currency: &str,
    amount: &str,
    rate: &str,
) -> SaveTransaction {
    let mut cmd = form(submission, customer, passport);
    cmd.side = "SELL".to_owned();
    cmd.currency = currency.to_owned();
    cmd.amount = amount.to_owned();
    cmd.rate = rate.to_owned();
    cmd
}

/// Ruling of 2026-10-10 (R3): a JPY exchange sold at 23.5 per 100 is stored at 23.5 with unit
/// 100, and `adminSnapshot` lists it at v2's 0.235 per 1. A USD exchange (unit 1) lists its rate
/// unchanged.
#[tokio::test]
async fn a_jpy_row_shows_its_rate_per_1() {
    let shop = Shop::start().await;
    shop.open_shift(MAIN, &[("THB", "100000"), ("JPY", "5000"), ("USD", "1000")])
        .await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    publish(
        &shop.pool,
        MAIN,
        &[("JPY", "23.1", "23.5"), ("USD", "32.1", "33")],
    )
    .await;

    let jpy = shop
        .save(
            MAIN,
            exchange(
                "re4c-row-jpy-0000001",
                &customer,
                &passport,
                "JPY",
                "1000",
                "0.235",
            ),
            at(0),
        )
        .await
        .expect("the JPY save is accepted");
    shop.save(
        MAIN,
        exchange(
            "re4c-row-usd-0000001",
            &customer,
            &passport,
            "USD",
            "10",
            "33",
        ),
        at(1),
    )
    .await
    .expect("the USD save is accepted");
    let stored: (Decimal, Decimal) =
        sqlx::query_as("SELECT rate, unit_size FROM transactions WHERE document_ref = $1")
            .bind(&jpy.document_ref)
            .fetch_one(&shop.pool)
            .await
            .expect("read the JPY exchange");
    assert_eq!(stored, (d("23.5"), d("100")), "stored per 100");

    let snapshot = builder(&shop)
        .admin_snapshot(&branch(MAIN), "owner", at(10))
        .await
        .expect("the snapshot")
        .to_value();
    let rows = snapshot["transactions"]
        .as_array()
        .expect("transactions is a list");
    let rate_of = |document_ref: &str| -> Value {
        rows.iter()
            .find(|r| r["id"] == json!(document_ref))
            .unwrap_or_else(|| panic!("no row {document_ref} in {snapshot}"))["rate"]
            .clone()
    };
    assert_eq!(rate_of(&jpy.document_ref), json!(0.235), "per 1, not 23.5");
    assert_eq!(rate_of("XC-20261006-000002"), json!(33));
    shop.finish().await;
}

/// MAIN with an open shift, a customer, a passport file, and JPY at 23.1 / 23.5 per 100
/// published. Answers (customer, passport).
async fn jpy_counter(shop: &Shop) -> (String, String) {
    shop.open_shift(MAIN, &[("THB", "100000"), ("JPY", "5000")])
        .await;
    let customer = shop.reserve(MAIN, person()).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    publish(&shop.pool, MAIN, &[("JPY", "23.1", "23.5")]).await;
    (customer, passport)
}

/// The listed row of `document_ref` in MAIN's snapshot.
async fn listed(shop: &Shop, document_ref: &str) -> Value {
    let snapshot = builder(shop)
        .admin_snapshot(&branch(MAIN), "owner", at(10))
        .await
        .expect("the snapshot")
        .to_value();
    snapshot["transactions"]
        .as_array()
        .expect("transactions is a list")
        .iter()
        .find(|r| r["id"] == json!(document_ref))
        .unwrap_or_else(|| panic!("no row {document_ref} in {snapshot}"))
        .clone()
}

/// Ruling of 2026-10-10 (R3): the reference rate is stored per unit and listed per 1. A JPY
/// reference sent as 0.230 is stored 23.0 per 100 and listed 0.23.
#[tokio::test]
async fn a_jpy_reference_is_listed_per_1() {
    let shop = Shop::start().await;
    let (customer, passport) = jpy_counter(&shop).await;
    let mut cmd = exchange(
        "re4c-row-jpy-ref-001",
        &customer,
        &passport,
        "JPY",
        "1000",
        "0.235",
    );
    cmd.reference_rate = Some("0.230".to_owned());
    let saved = shop
        .save(MAIN, cmd, at(0))
        .await
        .expect("the JPY save is accepted");
    let stored: Option<Decimal> =
        sqlx::query_scalar("SELECT reference_rate FROM transactions WHERE document_ref = $1")
            .bind(&saved.document_ref)
            .fetch_one(&shop.pool)
            .await
            .expect("read the exchange");
    assert_eq!(stored, Some(d("23.0")), "stored per 100");

    let row = listed(&shop, &saved.document_ref).await;
    assert_eq!(
        (&row["rate"], &row["referenceRate"]),
        (&json!(0.235), &json!(0.23)),
        "both per 1"
    );
    shop.finish().await;
}

/// R9 (ruling of 2026-10-10): a row is divided by the unit it was stored with, not by the
/// register's unit now. A JPY row stored at 23.5 per 100 still lists 0.235 after the register
/// moves JPY to 1000 (test data, written directly; no writer is under test).
#[tokio::test]
async fn a_row_is_divided_by_its_own_unit_not_the_registers() {
    let shop = Shop::start().await;
    let (customer, passport) = jpy_counter(&shop).await;
    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-row-jpy-unit-01",
                &customer,
                &passport,
                "JPY",
                "1000",
                "0.235",
            ),
            at(0),
        )
        .await
        .expect("the JPY save is accepted");
    sqlx::query("UPDATE currencies SET unit_size = 1000 WHERE code = 'JPY'")
        .execute(&shop.pool)
        .await
        .expect("move the register's JPY unit");

    assert_eq!(
        listed(&shop, &saved.document_ref).await["rate"],
        json!(0.235),
        "23.5 ÷ the row's 100, not ÷ 1000"
    );
    shop.finish().await;
}

/// §8 migration 5 / R9: a row stored before RE-4 has no line. Here a JPY row holds 0.2195 per
/// 1 unit (its own `unit_size` 1, written directly as such a row is), while the register says
/// 100: it lists its stored rate per its own unit, 0.2195.
#[tokio::test]
async fn a_row_without_a_line_lists_its_stored_rate_per_its_own_unit() {
    let shop = Shop::start().await;
    let (customer, passport) = jpy_counter(&shop).await;
    let saved = shop
        .save(
            MAIN,
            exchange(
                "re4c-row-jpy-null-01",
                &customer,
                &passport,
                "JPY",
                "1000",
                "0.235",
            ),
            at(0),
        )
        .await
        .expect("the JPY save is accepted");
    sqlx::query(
        "UPDATE transactions SET published_rate_line_id = NULL, rate = 0.2195, unit_size = 1 \
         WHERE document_ref = $1",
    )
    .bind(&saved.document_ref)
    .execute(&shop.pool)
    .await
    .expect("make it a row from before RE-4");
    sqlx::query(
        "UPDATE transaction_lines l SET rate_snapshot = 0.2195, unit_size = 1 FROM transactions t \
         WHERE l.transaction_id = t.id AND t.document_ref = $1",
    )
    .bind(&saved.document_ref)
    .execute(&shop.pool)
    .await
    .expect("give its line the shape TX-2's backfill gives a row from before RE-4");

    assert_eq!(
        listed(&shop, &saved.document_ref).await["rate"],
        json!(0.2195),
        "the stored 0.2195 ÷ its own unit 1"
    );
    shop.finish().await;
}
