//! Written before the implementation (ticket T11p, with T11a part one). Do not edit, delete or weaken.
//!
//! `ServiceSnapshotSources`: the real `SnapshotSources`, against rows written
//! by the real services — exchanges saved through `SaveTransactionWorkflow`,
//! accounts made through `AccountService`.
//!
//! Needs `TEST_DATABASE_URL` (see `save_common/mod.rs`); nothing here skips.

mod compat_common;
mod save_common;
mod snapshot_common;

use std::sync::Arc;

use chrono::NaiveDate;
use save_common::{
    JPEG, MAIN, Mode, OWNER, PDF, Shop, Transaction, TransactionService, at, branch, form, millis,
    money, person,
};
use serde_json::json;
use snapshot_common::{NoPorts, RecordedSource};
use xc_auth::application::account::service::{AccountService, EmployeesAnswer};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::{Rate, Side, UnitSize};
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::compat::snapshot::SnapshotBuilder;
use xchange::application::compat::snapshot_sources::{SnapshotSources, SnapshotTransaction};
use xchange::application::compat::v2_json::PersonLabel;
use xchange::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;

const OTHER: &str = "XC-B01";

type Accounts = AccountService<PgAccountRepository>;

fn accounts(shop: &Shop) -> Accounts {
    AccountService::new(
        Arc::new(PgAccountRepository::new(shop.pool.clone())),
        PasswordHasher::with_cost(8, 1, 1),
    )
}

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(pos_infra::transactions::Transactor::new(shop.pool.clone()))
}

/// The sources as the routes build them: over the two services, on the shop's
/// own database.
fn sources(shop: &Shop) -> ServiceSnapshotSources {
    ServiceSnapshotSources::new(
        TransactionService::new(
            Arc::new(PgTransactionRepository::new(shop.pool.clone())),
            branch_tx(shop),
        ),
        accounts(shop),
    )
}

async fn employ(
    accounts: &Accounts,
    username: &str,
    name: &str,
    role: &str,
    branch_id: &str,
) -> String {
    let made = accounts
        .employees(
            "create",
            &json!({ "username": username, "name": name, "role": role,
                     "branchId": branch_id, "password": "first-password-1" }),
            OWNER,
            at(0),
        )
        .await
        .expect("create an employee");
    assert_eq!(made, EmployeesAnswer::Saved);
    let EmployeesAnswer::List(everyone) = accounts
        .employees("list", &json!({}), OWNER, at(0))
        .await
        .expect("list the employees")
    else {
        panic!("a list");
    };
    everyone
        .into_iter()
        .find(|e| e.username == username)
        .expect("the employee just made")
        .id
}

/// The LIVE exchanges of the branch asked for, in the order they were saved,
/// each with exactly what the save recorded. A DEMO exchange and an exchange of
/// another branch are not among them.
#[tokio::test]
async fn the_exchanges_are_the_live_ones_of_the_branch_oldest_first() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;

    // the first: a cash purchase
    let mut first = form("first-0001-aaaaaaaaaa", &customer, &passport);
    first.staff = "fr1".to_owned();
    first.notes = "first".to_owned();
    shop.save(MAIN, first, at(10))
        .await
        .expect("the first save");

    // the second: a sale paid by the old transfer name, with a slip
    let mut second = form("second-0002-aaaaaaaaa", &customer, &passport);
    second.side = "SELL".to_owned();
    second.rate = "33".to_owned();
    second.amount = "250.5".to_owned();
    second.fee = "10".to_owned();
    second.bank_fee = "1.5".to_owned();
    second.method = "SCB Transfer".to_owned();
    second.payment_ref = "TRX-1".to_owned();
    second.slip = Some(slip.clone());
    second.staff = "owner".to_owned();
    second.reference_rate = Some("32.95".to_owned());
    second.value_date = "2026-11-01".to_owned();
    shop.save(MAIN, second, at(20))
        .await
        .expect("the second save");

    // a DEMO exchange of the same branch
    let mut demo_person = person();
    demo_person.mode = "DEMO".to_owned();
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let mut demo = form("demo-0003-aaaaaaaaaaa", &demo_customer, &passport);
    demo.customer = demo_person;
    shop.save(MAIN, demo, at(30)).await.expect("the DEMO save");

    // an exchange of another branch
    shop.seed_branch(OTHER, "Second").await;
    shop.set_rate(OTHER, "USD", "32.1", "33").await;
    shop.open_shift(OTHER, &[("THB", "100000"), ("USD", "1000")])
        .await;
    let customer_b = shop.reserve(OTHER, person()).await;
    let passport_b = shop.upload(OTHER, OWNER, "image/jpeg", JPEG).await;
    let elsewhere = form("other-0004-aaaaaaaaaa", &customer_b, &passport_b);
    shop.save(OTHER, elsewhere, at(40))
        .await
        .expect("the save elsewhere");

    let sources = sources(&shop);
    let listed = sources
        .transactions(&branch(MAIN))
        .await
        .expect("the exchanges of MAIN");
    assert_eq!(
        listed,
        vec![
            SnapshotTransaction {
                document_ref: "XC-20261006-000001".to_owned(),
                recorded_at: millis(at(10)),
                value_date: NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
                side: Side::Buy,
                currency: "USD".to_owned(),
                amount: money("100"),
                rate: Rate::parse("32.1").unwrap(),
                // amended for #757 (RE-4c, R9): the row's own unit, as stored
                unit_size: UnitSize::parse("1").unwrap(),
                reference_rate: None,
                fee: money("0"),
                bank_fee: money("0"),
                method: "Cash".to_owned(),
                slip_key: None,
                payment_ref: String::new(),
                customer_id: customer.clone(),
                customer_name: "Somchai Jaidee".to_owned(),
                staff: "fr1".to_owned(),
                notes: "first".to_owned(),
                shift_id: Some(shift.clone()),
            },
            SnapshotTransaction {
                document_ref: "XC-20261006-000002".to_owned(),
                recorded_at: millis(at(20)),
                value_date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
                side: Side::Sell,
                currency: "USD".to_owned(),
                amount: money("250.5"),
                rate: Rate::parse("33").unwrap(),
                // amended for #757 (RE-4c, R9): the row's own unit, as stored
                unit_size: UnitSize::parse("1").unwrap(),
                reference_rate: Some(Rate::parse("32.95").unwrap()),
                fee: money("10"),
                bank_fee: money("1.5"),
                // as stored: printing it as `Bank Transfer` is the builder's
                method: "SCB Transfer".to_owned(),
                slip_key: Some(slip.clone()),
                payment_ref: "TRX-1".to_owned(),
                customer_id: customer.clone(),
                customer_name: "Somchai Jaidee".to_owned(),
                staff: "owner".to_owned(),
                notes: String::new(),
                shift_id: Some(shift.clone()),
            },
        ]
    );

    // nothing is made up on the way: it is what the exchange service holds
    let stored: Transaction = shop
        .transactions
        .find(&branch(MAIN), "XC-20261006-000002")
        .await
        .expect("read the exchange back")
        .expect("the exchange is kept");
    assert_eq!(listed[1].recorded_at, stored.recorded_at);
    assert_eq!(listed[1].staff, stored.staff);
    assert_eq!(listed[1].method, stored.method.as_str());

    let elsewhere = sources
        .transactions(&branch(OTHER))
        .await
        .expect("the exchanges of the other branch");
    assert_eq!(elsewhere.len(), 1);
    assert_eq!(elsewhere[0].document_ref, "XC-20261006-000003");
    assert_eq!(elsewhere[0].customer_id, customer_b);

    // and the DEMO one is there to be left out
    assert_eq!(shop.refs(MAIN, Mode::Demo).await.len(), 1);
    assert!(
        sources
            .transactions(&branch("B-nowhere"))
            .await
            .expect("a branch with nothing")
            .is_empty()
    );
    shop.finish().await;
}

/// Everybody of the branch by employee id, with the name and the account's
/// role as v2 spells it — a closed account included, another branch's people
/// left out.
#[tokio::test]
async fn the_people_are_everybody_of_the_branch_closed_accounts_included() {
    let shop = Shop::start().await;
    let accounts = accounts(&shop);
    let front = employ(&accounts, "fr1", "FR1 Front", "FRONT", MAIN).await;
    let review = employ(&accounts, "rv1", "RV1 Review", "DOCUMENT_REVIEW", MAIN).await;
    let gone = employ(&accounts, "be1", "BE1 Backend", "BACKEND", MAIN).await;
    // OR-2: an employee's branch must name a `branches` row (design #550 follow-up, PR 636).
    shop.seed_branch(OTHER, "Second").await;
    let other = employ(&accounts, "fr2", "FR2 Front", "FRONT", OTHER).await;
    accounts
        .employees("disable", &json!({ "id": gone }), OWNER, at(5))
        .await
        .expect("close an account");

    let sources = sources(&shop);
    let people = sources
        .people(&branch(MAIN))
        .await
        .expect("the people of MAIN");
    let label = |name: &str, role: &str| PersonLabel {
        name: name.to_owned(),
        role: role.to_owned(),
    };
    assert_eq!(people.len(), 3, "{people:?}");
    assert_eq!(people.get(&front), Some(&label("FR1 Front", "FRONT")));
    assert_eq!(
        people.get(&review),
        Some(&label("RV1 Review", "DOCUMENT_REVIEW"))
    );
    assert_eq!(people.get(&gone), Some(&label("BE1 Backend", "BACKEND")));

    let elsewhere = sources
        .people(&branch(OTHER))
        .await
        .expect("the other branch");
    assert_eq!(elsewhere.len(), 1);
    assert_eq!(elsewhere.get(&other), Some(&label("FR2 Front", "FRONT")));
    assert!(
        sources
            .people(&branch("B-nowhere"))
            .await
            .expect("a branch with nobody")
            .is_empty()
    );
    shop.finish().await;
}

/// Membership comes from the branch's `members` rows (D5 is decided): nobody
/// is a member without a member row, whoever the customers are.
#[tokio::test]
async fn nobody_is_a_member_without_a_member_row() {
    let shop = Shop::start().await;
    shop.reserve(MAIN, person()).await;
    let members = sources(&shop)
        .members(&branch(MAIN))
        .await
        .expect("the members of MAIN");
    assert!(members.is_empty(), "{members:?}");
    shop.finish().await;
}

/// The snapshot builder over the real sources shows a saved exchange as v2
/// showed one: the reference as `id`, the old transfer name as `Bank Transfer`,
/// the slip as a link — and only the LIVE ones.
#[tokio::test]
async fn a_snapshot_over_the_real_sources_lists_the_saved_exchanges() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let slip = shop.upload(MAIN, OWNER, "application/pdf", PDF).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let mut sale = form("sale-0001-aaaaaaaaaaa", &customer, &passport);
    sale.side = "SELL".to_owned();
    sale.rate = "33".to_owned();
    sale.method = "SCB Transfer".to_owned();
    sale.payment_ref = "TRX-9".to_owned();
    sale.slip = Some(slip.clone());
    sale.staff = "fr1".to_owned();
    shop.save(MAIN, sale, at(10)).await.expect("the sale");
    let mut demo_person = person();
    demo_person.mode = "DEMO".to_owned();
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let mut demo = form("demo-0002-aaaaaaaaaaa", &demo_customer, &passport);
    demo.customer = demo_person;
    shop.save(MAIN, demo, at(20)).await.expect("the DEMO save");

    let custody = CustodyService::new(
        Arc::new(PgCustodyRepository::new(shop.pool.clone())),
        shop.shifts.clone(),
        Arc::new(NoPorts),
        branch_tx(&shop),
    );
    let builder = SnapshotBuilder::new(
        BranchService::new(branch_tx(&shop)),
        RateService::new(branch_tx(&shop), Arc::new(RecordedSource::default())),
        shop.customers.clone(),
        shop.shifts.clone(),
        custody,
        Arc::new(sources(&shop)),
    );
    let snapshot = builder
        .admin_snapshot(&branch(MAIN), "owner", at(30))
        .await
        .expect("a snapshot")
        .to_value();
    assert_eq!(
        snapshot["transactions"],
        json!([{
            "id": "XC-20261006-000001",
            "status": "COMPLETED",
            "created": "2026-10-06T05:00:10.123Z",
            "date": "2026-10-06",
            "type": "SELL",
            "currency": "USD",
            "amount": 100,
            "rate": 33,
            "referenceRate": null,
            "fee": 0,
            "bankFee": 0,
            "method": "Bank Transfer",
            "slipUrl": format!("/files/{slip}"),
            "paymentRef": "TRX-9",
            "customerId": customer,
            "customer": "Somchai Jaidee",
            "staff": "fr1",
            "notes": "",
            "shiftId": shift,
        }])
    );
    shop.finish().await;
}
