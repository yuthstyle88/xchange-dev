//! Ticket #245, cut from #228 item 17 — the device-write-error test must catch a step that
//! swallows the error. `device_save_step_test::a_device_write_that_fails_undoes_the_save` proves
//! the save fails, but the save would fail either way: once the trigger's error has broken the
//! save's transaction, the save's next statement fails too (SQLSTATE 25P02, "current transaction
//! is aborted"), swallowed or not. This file pins the error's identity instead: the save fails
//! with the device write's own database error — SQLSTATE `P0001` and the trigger's message —
//! and nothing of the save is kept.
//!
//! The set-up is `device_save_step_test`'s: `save_common`'s shop, `devices_common`'s fakes, a
//! `DeviceService` on the shop's pool, and a workflow with the device step only.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.

mod common;
mod devices_common;
mod routes_common;
#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use pos_error::AppError;
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xchange::application::workflows::device_save_step::DeviceSaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};
use xchange::infrastructure::devices::device_service::DeviceService;

const FRONT: &str = "employee-7";

/// The saver, as in `device_save_step_test`: its id is not its username.
fn front() -> SaveActor {
    SaveActor {
        id: FRONT.to_owned(),
        username: format!("user-{FRONT}"),
        employee_id: Some(FRONT.to_owned()),
        role: Role::Front,
    }
}

/// A live cash save whose cash was counted: past the step's check, into its writes.
const COUNTED: SaveExtras = SaveExtras {
    staff_draft_id: None,
    count_confirmed: true,
    kiosk_request_id: None,
};

/// A shop ready for a purchase, `FRONT`'s passport, and a workflow with the device step only.
async fn guarded(
    shop: &Shop,
    ids: &Arc<devices_common::FakeIds>,
) -> (SaveTransactionWorkflow, String, String) {
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
    let devices = DeviceService::new(
        BranchTx::new(Transactor::new(shop.pool.clone())),
        Arc::new(devices_common::FakeExchanges::default()),
        ids.clone(),
    );
    let step: Arc<dyn SaveStep> = Arc::new(DeviceSaveStep::new(devices));
    let flow = SaveTransactionWorkflow::new(
        BranchTx::new(Transactor::new(shop.pool.clone())),
        shop.shifts.clone(),
        shop.files.clone(),
    )
    .with_steps(vec![step]);
    (flow, customer, passport)
}

#[tokio::test]
async fn the_save_fails_with_the_device_write_s_own_error() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    ids.give_one(&devices_common::job_id(5));
    shop.break_writes("device_jobs").await;
    let before = shop.everything().await;

    let got = flow
        .save_as(
            &branch(MAIN),
            form("dse-0001-aaaaaaaaaaaa", &customer, &passport),
            &COUNTED,
            &front(),
            at(0),
        )
        .await;

    // The error is the trigger's own, not the 25P02 "transaction aborted" the save's next
    // statement answers with once the device's error was swallowed.
    let error = got.expect_err("the device's failure is the save's");
    let AppError::Database(sqlx::Error::Database(db)) = &error else {
        panic!("the save fails with the device write's own database error, got {error:?}");
    };
    assert_eq!(db.code().as_deref(), Some("P0001"), "SQLSTATE: {error:?}");
    assert_eq!(
        db.message(),
        "test: this table refuses writes",
        "the trigger's message: {error:?}"
    );

    assert_eq!(shop.count("transactions").await, 0);
    assert_eq!(shop.count("device_events").await, 0);
    assert_eq!(
        shop.everything().await,
        before,
        "nothing of the save is kept"
    );
    shop.mend("device_jobs").await;
    shop.finish().await;
}
