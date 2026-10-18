//! Ticket T8c-2b — `DeviceSaveStep`, the devices' part in the save of an exchange
//! (`docs/design/xchange-v3.md` §5.4). Written before the implementation, against a stub that
//! lets every save through and writes nothing. Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! - `before_in`: a live cash exchange whose cash was not counted (`SaveExtras.count_confirmed`
//!   false) is refused with `DeviceService::require_count_confirmed`'s sentence, and nothing of
//!   the save is written. Any other method passes. A `DEMO` exchange and a repeat of the same
//!   submission are not checked.
//! - `after_in`: `DeviceService::after_save_in` on the save's own transaction, with the actor's
//!   **id**: the count line when cash, the link line, one `PRINT_RECEIPT` job `QUEUED` and its
//!   line. A repeat adds nothing; a `DEMO` exchange writes nothing; a device write that fails
//!   undoes the whole save.
//!
//! The save is `save_common`'s; the device service is T13a's, built on the same database with
//! `devices_common`'s fakes.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.

mod common;
mod devices_common;
mod routes_common;
#[allow(unused_imports)]
mod save_common;
mod shift_holder_common;

use std::sync::Arc;

use pos_infra::transactions::Transactor;
use save_common::*;
use serde_json::Value;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xchange::application::workflows::device_save_step::DeviceSaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};
use xchange::infrastructure::devices::device_service::DeviceService;

const FRONT: &str = "employee-7";

/// The saver: its id is not its username, so the lines show which one they were given.
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

/// A shop ready for a purchase, `FRONT`'s passport, and a workflow with the device step only.
async fn guarded(
    shop: &Shop,
    ids: &Arc<devices_common::FakeIds>,
) -> (SaveTransactionWorkflow, String, String) {
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    shop.close_shift(MAIN).await;
    let shift = shift_holder_common::open_shift_for(
        &shop.pool,
        MAIN,
        FRONT,
        &[("THB", "100000"), ("USD", "1000")],
    )
    .await;
    fund_ledger_drawer(&shop.pool, &shift, &[("THB", "100000"), ("USD", "1000")]).await;
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

/// (kind, ref, actor, detail) of every device line, in the order written.
async fn lines(shop: &Shop) -> Vec<(String, String, String, Value)> {
    sqlx::query_as("SELECT kind, ref, actor, detail FROM device_events ORDER BY seq")
        .fetch_all(&shop.pool)
        .await
        .expect("read the device lines")
}

/// (id, kind, ref, status, actor) of every device job.
async fn jobs(shop: &Shop) -> Vec<(String, String, String, String, String)> {
    sqlx::query_as("SELECT id, kind, ref, status, actor FROM device_jobs ORDER BY created_at, id")
        .fetch_all(&shop.pool)
        .await
        .expect("read the device jobs")
}

fn kinds(lines: &[(String, String, String, Value)]) -> Vec<&str> {
    lines.iter().map(|(kind, ..)| kind.as_str()).collect()
}

#[tokio::test]
async fn a_cash_exchange_whose_cash_was_not_counted_is_refused_and_nothing_is_written() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    let before = shop.everything().await;

    let got = flow
        .save_as(
            &branch(MAIN),
            form("dss-0001-aaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &front(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), devices_common::COUNT_FIRST);
    assert_eq!(shop.count("transactions").await, 0);
    assert_eq!(shop.count("device_events").await, 0);
    assert_eq!(shop.count("device_jobs").await, 0);
    assert_eq!(
        shop.everything().await,
        before,
        "nothing of the save is kept"
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_counted_cash_exchange_writes_the_count_the_link_and_one_print_job() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    let job = devices_common::job_id(1);
    ids.give_one(&job);

    let saved = flow
        .save_as(
            &branch(MAIN),
            form("dss-0002-aaaaaaaaaaaa", &customer, &passport),
            &COUNTED,
            &front(),
            at(0),
        )
        .await
        .expect("a counted cash exchange is saved");

    let written = lines(&shop).await;
    assert_eq!(
        kinds(&written),
        ["COUNT_CONFIRMED", "EXCHANGE_LINKED", "PRINT_RECEIPT"],
        "{written:?}"
    );
    for (kind, _, actor, _) in &written {
        assert_eq!(actor, FRONT, "{kind} is written with the actor's id");
    }
    let (_, count_ref, _, count) = &written[0];
    assert_eq!(
        count_ref, "dss-0002-aaaaaaaaaaaa",
        "the count line names the submission"
    );
    assert_eq!(count["currency"], "USD");
    let (foreign, thb): (bool, bool) = sqlx::query_as(
        "SELECT (detail->>'foreignAmount')::numeric = 100, (detail->>'thb')::numeric = 3210 \
         FROM device_events WHERE kind = 'COUNT_CONFIRMED'",
    )
    .fetch_one(&shop.pool)
    .await
    .expect("the count line's amounts");
    assert!(foreign && thb, "100 USD at 32.1 is 3210 THB: {count}");

    let (_, link_ref, _, link) = &written[1];
    assert_eq!(link_ref, &saved.document_ref);
    assert_eq!(link["submissionId"], "dss-0002-aaaaaaaaaaaa");
    assert_eq!(link["passport"], passport.as_str());
    assert_eq!(link["slip"], Value::Null);

    assert_eq!(
        jobs(&shop).await,
        [(
            job.clone(),
            "PRINT_RECEIPT".to_owned(),
            saved.document_ref.clone(),
            "QUEUED".to_owned(),
            FRONT.to_owned(),
        )]
    );
    assert_eq!(written[2].1, saved.document_ref);
    assert_eq!(written[2].3["job"], job.as_str());
    shop.finish().await;
}

#[tokio::test]
async fn another_method_needs_no_count_and_writes_no_count_line() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    ids.give_one(&devices_common::job_id(2));
    let slip = shop.upload(MAIN, FRONT, "application/pdf", PDF).await;
    let mut cmd = form("dss-0003-aaaaaaaaaaaa", &customer, &passport);
    cmd.method = "Bank Transfer".to_owned();
    cmd.payment_ref = "TRX-1".to_owned();
    cmd.slip = Some(slip.clone());

    let saved = flow
        .save_as(&branch(MAIN), cmd, &SaveExtras::default(), &front(), at(0))
        .await
        .expect("a transfer needs no count");
    let written = lines(&shop).await;
    assert_eq!(
        kinds(&written),
        ["EXCHANGE_LINKED", "PRINT_RECEIPT"],
        "{written:?}"
    );
    assert_eq!(written[0].1, saved.document_ref);
    assert_eq!(written[0].3["slip"], slip.as_str());
    assert_eq!(shop.count("device_jobs").await, 1);
    shop.finish().await;
}

#[tokio::test]
async fn a_repeat_is_not_counted_again_and_adds_nothing() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    ids.give_one(&devices_common::job_id(3));
    let send = |extras: SaveExtras, when| {
        let (flow, cmd) = (
            flow.clone(),
            form("dss-0004-aaaaaaaaaaaa", &customer, &passport),
        );
        async move {
            flow.save_as(&branch(MAIN), cmd, &extras, &front(), when)
                .await
        }
    };

    let saved = send(COUNTED, at(0)).await.expect("the first save");
    let after_first = (lines(&shop).await, jobs(&shop).await);
    assert_eq!(
        after_first.0.len(),
        3,
        "count, link, print: {:?}",
        after_first.0
    );
    assert_eq!(after_first.1.len(), 1);

    // the same submission again, this time without the count: answered, not refused
    let again = send(SaveExtras::default(), at(60))
        .await
        .expect("a repeat is not counted again");
    assert_eq!(again, saved);
    assert_eq!(
        (lines(&shop).await, jobs(&shop).await),
        after_first,
        "a repeat adds no line and no job"
    );
    assert_eq!(ids.asked_for_too_many(), 0, "no second job id was drawn");
    shop.finish().await;
}

/// Holds before the implementation and after it: a DEMO exchange is not the devices' business.
#[tokio::test]
async fn a_demo_exchange_is_neither_checked_nor_recorded() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, _, _) = guarded(&shop, &ids).await;
    let demo_person = ReserveCustomer {
        mode: "DEMO".to_owned(),
        ..person()
    };
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
    let mut demo = form("dss-0005-aaaaaaaaaaaa", &demo_customer, &passport);
    demo.customer = demo_person;

    let saved = flow
        .save_as(&branch(MAIN), demo, &SaveExtras::default(), &front(), at(0))
        .await
        .expect("a DEMO cash exchange is saved without a count");
    assert_eq!(saved.mode, Mode::Demo);
    assert_eq!(shop.count("device_events").await, 0);
    assert_eq!(shop.count("device_jobs").await, 0);
    shop.finish().await;
}

#[tokio::test]
async fn a_device_write_that_fails_undoes_the_save() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    ids.give_one(&devices_common::job_id(4));
    shop.break_writes("device_jobs").await;
    let before = shop.everything().await;

    let got = flow
        .save_as(
            &branch(MAIN),
            form("dss-0006-aaaaaaaaaaaa", &customer, &passport),
            &COUNTED,
            &front(),
            at(0),
        )
        .await;
    assert!(got.is_err(), "the device's failure is the save's: {got:?}");
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

/// Holds before the implementation and after it: a save refused after the device's check passed
/// (no open shift) leaves no device line and no job.
#[tokio::test]
async fn a_save_refused_after_the_check_leaves_no_device_line() {
    let shop = Shop::start().await;
    let ids = Arc::new(devices_common::FakeIds::default());
    let (flow, customer, passport) = guarded(&shop, &ids).await;
    shop.close_shift(MAIN).await;

    let got = flow
        .save_as(
            &branch(MAIN),
            form("dss-0007-aaaaaaaaaaaa", &customer, &passport),
            &COUNTED,
            &front(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), NO_OPEN_SHIFT);
    assert_eq!(shop.count("device_events").await, 0);
    assert_eq!(shop.count("device_jobs").await, 0);
    shop.finish().await;
}
