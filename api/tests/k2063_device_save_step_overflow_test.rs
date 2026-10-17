//! Ticket #2859 (#2063 tail): `DeviceSaveStep` uses checked `Money::try_minor`.

mod common;
mod devices_common;
mod routes_common;
#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::mode::Mode;
use xc_kernel::money::{Money, Rate, Side};
use xchange::application::workflows::device_save_step::DeviceSaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveContext, SaveExtras, SaveStep};
use xchange::infrastructure::devices::device_service::DeviceService;

const FRONT: &str = "employee-7";

fn front() -> SaveActor {
    SaveActor {
        id: FRONT.to_owned(),
        username: format!("user-{FRONT}"),
        employee_id: Some(FRONT.to_owned()),
        role: Role::Front,
    }
}

const COUNTED: SaveExtras = SaveExtras {
    staff_draft_id: None,
    count_confirmed: true,
    kiosk_request_id: None,
};

fn save_step_context<'a>(
    branch: &'a xc_kernel::ids::BranchId,
    form: &'a TransactionForm,
    extras: &'a SaveExtras,
    actor: &'a SaveActor,
    now: chrono::DateTime<Utc>,
) -> SaveContext<'a> {
    SaveContext {
        branch,
        form,
        extras,
        booking: None,
        actor,
        teller: None,
        now,
        prior: None,
    }
}

fn sample_transaction(amount: Money, cash: bool) -> Transaction {
    Transaction {
        document_ref: "TX-20261017-0001".to_string(),
        branch_id: branch(MAIN),
        mode: Mode::Live,
        shift_id: Some("shift-1".to_string()),
        submission_id: "sub-1234567890123456".to_string(),
        customer_id: "cust-1".to_string(),
        customer_name: "Customer One".to_string(),
        nationality: "THA".to_string(),
        side: Side::Buy,
        currency: "USD".to_string(),
        amount,
        rate: Rate::parse("32.5").unwrap(),
        reference_rate: None,
        principal: Money::ZERO,
        fee: Money::ZERO,
        bank_fee: Money::ZERO,
        received: Money::ZERO,
        paid: Money::ZERO,
        method: if cash {
            PaymentMethod::Cash
        } else {
            PaymentMethod::BankTransfer
        },
        payment_ref: String::new(),
        account: "Bank Transfer".to_string(),
        staff: "user-front".to_string(),
        booking: String::new(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
        paid_at: Utc::now(),
        passport_key: "passport-key-1".to_string(),
        slip_key: None,
        receipt_key: "receipt-key-1".to_string(),
        recorded_at: Utc::now(),
        recorded_by: "user-front".to_string(),
    }
}

#[tokio::test]
async fn amount_at_i64_max_boundary_fits_and_succeeds() {
    let shop = Shop::start().await;
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;

    let ids = Arc::new(devices_common::FakeIds::default());
    ids.give_one(&devices_common::job_id(1));
    let devices = DeviceService::new(
        BranchTx::new(Transactor::new(shop.pool.clone())),
        Arc::new(devices_common::FakeExchanges::default()),
        ids.clone(),
    );
    let step = DeviceSaveStep::new(devices);

    let form_cmd = form("dse-0002-boundarytest", &customer, &passport);
    let parsed_form = TransactionForm::parse(&form_cmd).expect("valid transaction form");

    let branch_id = branch(MAIN);
    let actor = front();
    let ctx = save_step_context(&branch_id, &parsed_form, &COUNTED, &actor, at(0));

    // Hundredths: exactly i64::MAX fits
    let boundary_amount =
        Money::try_from_minor_i128(i64::MAX as i128).expect("valid money in decimal");

    let saved = sample_transaction(boundary_amount, false);

    let mut tx = shop.pool.begin().await.expect("begin transaction");
    let result = step.after_in(&mut tx, &ctx, &saved).await;

    assert!(
        result.is_ok(),
        "amount at boundary i64::MAX must succeed, got {result:?}"
    );

    drop(tx);
    shop.finish().await;
}

#[tokio::test]
async fn ordinary_amount_succeeds_unchanged() {
    let shop = Shop::start().await;
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;

    let ids = Arc::new(devices_common::FakeIds::default());
    ids.give_one(&devices_common::job_id(2));
    let devices = DeviceService::new(
        BranchTx::new(Transactor::new(shop.pool.clone())),
        Arc::new(devices_common::FakeExchanges::default()),
        ids.clone(),
    );
    let step = DeviceSaveStep::new(devices);

    let form_cmd = form("dse-0003-ordinarytest", &customer, &passport);
    let parsed_form = TransactionForm::parse(&form_cmd).expect("valid transaction form");

    let branch_id = branch(MAIN);
    let actor = front();
    let ctx = save_step_context(&branch_id, &parsed_form, &COUNTED, &actor, at(0));

    let ordinary_amount = Money::parse("100").expect("valid money");
    let saved = sample_transaction(ordinary_amount, false);

    let mut tx = shop.pool.begin().await.expect("begin transaction");
    let result = step.after_in(&mut tx, &ctx, &saved).await;

    assert!(
        result.is_ok(),
        "ordinary amount must succeed, got {result:?}"
    );

    drop(tx);
    shop.finish().await;
}
