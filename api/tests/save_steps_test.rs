//! Specification tests for T8c-1: the steps other contexts add around a save,
//! and custody's step.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `SaveTransactionWorkflow::save_as` is the save as a signed-in person makes
//!   it: the form's `staff` becomes the saver's username, and every registered
//!   `SaveStep` runs before and after the save **inside its transaction**.
//! - `CustodySaveStep` is the first such step: it asks `CustodyService` whether
//!   a live exchange may be saved now by this person, and completes the draft
//!   the exchange was begun under.
//! - `save` (ticket T8b) stays the save with nothing around it.
//!
//! The custody rule itself is `xc-shift`'s and has its own tests; here it is
//! reached through a save, with the state put straight into the custody tables.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

// the shop of the save tests: its helpers prepare a branch and read it back
#[allow(unused_imports)]
mod save_common;

use std::sync::{Arc, Mutex};

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::workflows::custody_save_step::CustodySaveStep;
use xchange::application::workflows::save_step::{
    SaveActor, SaveContext, SaveExtras, SaveStep, StepFuture,
};

// The sentences custody answers a save with (`xc-shift`'s, from v2).
const NOT_CUSTODIAN: &str = "ต้องเป็น Front ผู้รับผิดชอบลิ้นชักที่ลงเวลาเข้าแล้ว";
const ON_HOLD: &str = "ลูกค้านี้อยู่ระหว่างพักตรวจ ให้ Monitor ยืนยันผลก่อนทำรายการ";
const SAVE_CLOSED: &str = "ปิดรับรายการใหม่แล้ว หรือกำลังส่งกะ/ตรวจนับเงิน";
const NOT_OWN_RETRY: &str = "คำขอเดิมไม่ใช่ของพนักงานคนนี้";

fn owner() -> SaveActor {
    SaveActor {
        id: OWNER.to_owned(),
        username: "boss".to_owned(),
        employee_id: None,
        role: Role::Owner,
    }
}

/// A staff member: the id is the employee's id, the username something else.
fn staff(id: &str, role: Role) -> SaveActor {
    SaveActor {
        id: id.to_owned(),
        username: format!("user-{id}"),
        employee_id: Some(id.to_owned()),
        role,
    }
}

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(Transactor::new(shop.pool.clone()))
}

fn workflow(shop: &Shop, steps: Vec<Arc<dyn SaveStep>>) -> SaveTransactionWorkflow {
    SaveTransactionWorkflow::new(branch_tx(shop), shop.shifts.clone(), shop.files.clone())
        .with_steps(steps)
}

// ------------------------------------------------------------ a step that tells

/// What a step saw, in order: `before <name> …` / `after <name> …`.
type Seen = Arc<Mutex<Vec<String>>>;

/// A step that writes down each call, leaves a row behind through the
/// transaction it is handed, and can be told to refuse or to fail.
struct Telling {
    name: &'static str,
    seen: Seen,
    refuse_before: Option<&'static str>,
    fail_after: bool,
}

impl Telling {
    fn new(name: &'static str, seen: &Seen) -> Telling {
        Telling {
            name,
            seen: seen.clone(),
            refuse_before: None,
            fail_after: false,
        }
    }

    /// A row only this step writes, in a table nothing in a save touches.
    async fn mark(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &SaveContext<'_>,
        when: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(ctx.branch.as_str())
        .bind(format!("{when}-{}-{}", self.name, ctx.form.submission_id))
        .bind(ctx.now)
        .bind(&ctx.actor.id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}

impl SaveStep for Telling {
    fn before_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(format!(
                "before {} staff={} actor={} draft={:?} counted={} kiosk={:?} prior={:?} amount={} at={}",
                self.name,
                ctx.form.staff,
                ctx.actor.id,
                ctx.extras.staff_draft_id,
                ctx.extras.count_confirmed,
                ctx.extras.kiosk_request_id,
                ctx.prior.map(|t| t.document_ref.clone()),
                ctx.form.amount,
                ctx.now.timestamp_subsec_micros(),
            ));
            self.mark(tx, ctx, "before").await?;
            match self.refuse_before {
                Some(sentence) => Err(AppError::Domain(sentence.to_owned())),
                None => Ok(()),
            }
        })
    }

    fn after_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        ctx: &'a SaveContext<'a>,
        saved: &'a Transaction,
    ) -> StepFuture<'a> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(format!(
                "after {} saved={} staff={} prior={:?}",
                self.name,
                saved.document_ref,
                saved.staff,
                ctx.prior.map(|t| t.document_ref.clone()),
            ));
            self.mark(tx, ctx, "after").await?;
            if self.fail_after {
                return Err(AppError::Internal("the step failed".to_owned()));
            }
            Ok(())
        })
    }
}

fn told(seen: &Seen) -> Vec<String> {
    std::mem::take(&mut *seen.lock().unwrap())
}

async fn marks(shop: &Shop) -> Vec<String> {
    sqlx::query_scalar("SELECT request_id FROM abandoned_requests ORDER BY request_id")
        .fetch_all(&shop.pool)
        .await
        .expect("read the steps' rows")
}

// ------------------------------------------------------------------ save_as

#[tokio::test]
async fn the_forms_staff_is_the_savers_username_whatever_was_sent() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let flow = workflow(&shop, vec![]);

    // sent with another name
    let mut cmd = form("staff-0001-aaaaaaaaaa", &customer, &passport);
    cmd.staff = "somebody else".to_owned();
    let saved = flow
        .save_as(&branch(MAIN), cmd, &SaveExtras::default(), &owner(), at(0))
        .await
        .expect("a save");
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.staff, "boss", "the saver's username");
    assert_eq!(stored.recorded_by, OWNER, "the saver's id");
    let receipt = shop
        .files
        .open_owned(saved.receipt_url.strip_prefix("/files/").unwrap(), OWNER)
        .await
        .unwrap()
        .expect("the receipt belongs to the saver's id");
    let html = String::from_utf8(receipt.bytes).unwrap();
    assert!(
        html.contains(">boss<") && !html.contains("somebody else"),
        "{html}"
    );

    // sent with no name at all: the form is not refused for it
    let mut cmd = form("staff-0002-aaaaaaaaaa", &customer, &passport);
    cmd.staff = String::new();
    cmd.amount = "1".to_owned();
    let saved = flow
        .save_as(&branch(MAIN), cmd, &SaveExtras::default(), &owner(), at(1))
        .await
        .expect("an empty staff field is filled in");
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.staff, "boss");

    // the same submission again with another name sent is the same exchange
    let mut again = form("staff-0002-aaaaaaaaaa", &customer, &passport);
    again.staff = "a third name".to_owned();
    again.amount = "1".to_owned();
    let answered = flow
        .save_as(
            &branch(MAIN),
            again,
            &SaveExtras::default(),
            &owner(),
            at(2),
        )
        .await
        .expect("the staff sent is not part of what makes it the same");
    assert_eq!(answered, saved);

    // `save` leaves the field as it was sent
    let mut cmd = form("staff-0003-aaaaaaaaaa", &customer, &passport);
    cmd.staff = "EMP-009".to_owned();
    cmd.amount = "2".to_owned();
    let saved = flow.save(&branch(MAIN), cmd, OWNER, at(3)).await.unwrap();
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.staff, "EMP-009");
    shop.finish().await;
}

#[tokio::test]
async fn the_steps_run_in_order_around_the_save_and_are_told_about_it() {
    let shop = Shop::start().await;
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let me = staff("employee-7", Role::Front);
    let passport = shop.upload(MAIN, &me.id, "image/jpeg", JPEG).await;
    let seen: Seen = Seen::default();
    let flow = workflow(
        &shop,
        vec![
            Arc::new(Telling::new("A", &seen)),
            Arc::new(Telling::new("B", &seen)),
        ],
    );
    let extras = SaveExtras {
        staff_draft_id: Some("draft-1".to_owned()),
        count_confirmed: true,
        kiosk_request_id: Some("kiosk-9".to_owned()),
    };

    let saved = flow
        .save_as(
            &branch(MAIN),
            form("order-0001-aaaaaaaaaa", &customer, &passport),
            &extras,
            &me,
            at(0),
        )
        .await
        .expect("a save");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    let before = |name: &str, prior: &str| {
        format!(
            "before {name} staff=user-employee-7 actor=employee-7 draft=Some(\"draft-1\") counted=true \
             kiosk=Some(\"kiosk-9\") prior={prior} amount=100.00 at=123000"
        )
    };
    assert_eq!(
        told(&seen),
        vec![
            before("A", "None"),
            before("B", "None"),
            "after A saved=XC-20261006-000001 staff=user-employee-7 prior=None".to_owned(),
            "after B saved=XC-20261006-000001 staff=user-employee-7 prior=None".to_owned(),
        ],
        "before in order, the save, after in order; the time is the save's, cut to milliseconds"
    );
    // what the steps wrote through the transaction is there with the exchange
    assert_eq!(
        marks(&shop).await,
        vec![
            "after-A-order-0001-aaaaaaaaaa",
            "after-B-order-0001-aaaaaaaaaa",
            "before-A-order-0001-aaaaaaaaaa",
            "before-B-order-0001-aaaaaaaaaa",
        ]
    );

    // the same submission again: both are told it is a repeat, and `after`
    // runs again with the stored exchange
    let again = flow
        .save_as(
            &branch(MAIN),
            form("order-0001-aaaaaaaaaa", &customer, &passport),
            &extras,
            &me,
            at(5),
        )
        .await
        .expect("the same exchange again");
    assert_eq!(again, saved);
    let prior = "Some(\"XC-20261006-000001\")";
    assert_eq!(
        told(&seen),
        vec![
            before("A", prior),
            before("B", prior),
            format!("after A saved=XC-20261006-000001 staff=user-employee-7 prior={prior}"),
            format!("after B saved=XC-20261006-000001 staff=user-employee-7 prior={prior}"),
        ]
    );
    assert_eq!(shop.count("transactions").await, 1);

    // again with something changed: `before` runs, the save is refused, no `after`
    let mut changed = form("order-0001-aaaaaaaaaa", &customer, &passport);
    changed.amount = "5".to_owned();
    assert_eq!(
        refusal(
            flow.save_as(&branch(MAIN), changed, &extras, &me, at(6))
                .await
        ),
        CHANGED
    );
    let calls = told(&seen);
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls.iter().all(|c| c.starts_with("before ")), "{calls:?}");

    // `save` runs no step at all
    let mut plain = form("order-0002-aaaaaaaaaa", &customer, &passport);
    plain.amount = "1".to_owned();
    flow.save(&branch(MAIN), plain, &me.id, at(7))
        .await
        .expect("the save with nothing around it");
    assert_eq!(told(&seen), Vec::<String>::new());
    shop.finish().await;
}

#[tokio::test]
async fn a_step_that_refuses_ends_the_save_and_nothing_is_kept() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let seen: Seen = Seen::default();
    let mut second = Telling::new("B", &seen);
    second.refuse_before = Some("ขั้นที่สองไม่ให้ผ่าน");
    let flow = workflow(
        &shop,
        vec![
            Arc::new(Telling::new("A", &seen)),
            Arc::new(second),
            Arc::new(Telling::new("C", &seen)),
        ],
    );
    let before = shop.everything().await;

    let got = flow
        .save_as(
            &branch(MAIN),
            form("refuse-0001-aaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), "ขั้นที่สองไม่ให้ผ่าน", "the step's own sentence");
    let calls = told(&seen);
    assert_eq!(
        calls.len(),
        2,
        "A and B before; C never, and no after: {calls:?}"
    );
    assert!(
        calls[0].starts_with("before A ") && calls[1].starts_with("before B "),
        "{calls:?}"
    );
    assert_eq!(shop.everything().await, before);
    assert_eq!(
        marks(&shop).await,
        Vec::<String>::new(),
        "what A wrote before B refused is undone"
    );

    // a step is asked after the files, the branch and the form are judged …
    let mut bad_form = form("refuse-0002-aaaaaaaaa", &customer, &passport);
    bad_form.amount = "0".to_owned();
    let got = flow
        .save_as(
            &branch(MAIN),
            bad_form,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_ne!(refusal(got), "ขั้นที่สองไม่ให้ผ่าน");
    let got = flow
        .save_as(
            &branch("XC-B99"),
            form("refuse-0003-aaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), ATTACHMENT_NOT_FOUND);
    assert_eq!(
        told(&seen),
        Vec::<String>::new(),
        "no step was asked about a save that was not well-formed"
    );
    // … and before the customer, the shift, the rate and the stock
    let mut wrong = form("refuse-0004-aaaaaaaaa", "CX-99999999", &passport);
    wrong.rate = "1".to_owned();
    wrong.amount = "99999999".to_owned();
    let got = flow
        .save_as(
            &branch(MAIN),
            wrong,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), "ขั้นที่สองไม่ให้ผ่าน");
    shop.finish().await;
}

#[tokio::test]
async fn a_step_that_fails_after_the_save_undoes_the_save() {
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let seen: Seen = Seen::default();
    let mut second = Telling::new("B", &seen);
    second.fail_after = true;
    let failing = workflow(
        &shop,
        vec![Arc::new(Telling::new("A", &seen)), Arc::new(second)],
    );
    let before = shop.everything().await;

    let got = failing
        .save_as(
            &branch(MAIN),
            form("undo-0001-aaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert!(
        matches!(&got, Err(e) if !matches!(e, AppError::Domain(_))),
        "a failure, not a refusal: {got:?}"
    );
    assert_eq!(told(&seen).len(), 4, "both before, the save, both after");
    assert_eq!(
        shop.everything().await,
        before,
        "the exchange, its number, the stock and the customer are undone"
    );
    assert_eq!(
        marks(&shop).await,
        Vec::<String>::new(),
        "and so is every row a step wrote"
    );

    // a save refused after the steps were asked undoes what they wrote, and runs no `after`
    let ok = workflow(&shop, vec![Arc::new(Telling::new("A", &seen))]);
    let mut wrong_rate = form("undo-0002-aaaaaaaaaaa", &customer, &passport);
    wrong_rate.rate = "32.2".to_owned();
    let got = ok
        .save_as(
            &branch(MAIN),
            wrong_rate,
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), RATE_MISMATCH);
    let calls = told(&seen);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].starts_with("before A "));
    assert_eq!(marks(&shop).await, Vec::<String>::new());
    assert_eq!(shop.everything().await, before);

    // and with nothing failing the same submission is saved as number 1
    let saved = ok
        .save_as(
            &branch(MAIN),
            form("undo-0001-aaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            at(1),
        )
        .await
        .expect("a save");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    shop.finish().await;
}

// -------------------------------------------------------------- custody's step

const FRONT: &str = "employee-7";

/// 2026-10-06 12:00 in Bangkok plus minutes: `at(0)` is noon.
fn bangkok(hour: i64, minute: i64) -> chrono::DateTime<chrono::Utc> {
    at((hour - 12) * 3600 + minute * 60)
}

/// The custody service's questions to the other contexts, answered with
/// nothing: judging a save asks none of them (the real answers are ticket
/// T10b's `PgCustodyPorts`).
struct NothingOutside;

type Asked<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<T>> + Send + 'a>>;

impl CustodyPorts for NothingOutside {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> Asked<'a, OutsideFacts> {
        Box::pin(async { Ok(OutsideFacts::default()) })
    }

    fn customer_known<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
        _customer_id: &'a str,
    ) -> Asked<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn evidence_owned<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
        _owner_id: &'a str,
        _keys: &'a [String],
    ) -> Asked<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn employee_role<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
        _employee_id: &'a str,
    ) -> Asked<'a, Option<CustodyRole>> {
        Box::pin(async { Ok(None) })
    }
}

fn custody(shop: &Shop) -> CustodyService {
    CustodyService::new(
        Arc::new(PgCustodyRepository::new(shop.pool.clone())),
        shop.shifts.clone(),
        Arc::new(NothingOutside),
        branch_tx(shop),
    )
}

/// Custody on, service open, `FRONT` clocked in and holding the drawer.
fn open_service() -> CustodyState {
    CustodyState {
        enabled: true,
        phase: Phase::Open,
        custodian: Some(FRONT.to_owned()),
        attendance: vec![Attendance {
            id: "att-1".to_owned(),
            employee_id: FRONT.to_owned(),
            role: CustodyRole::Front,
            clock_in: CustodyTime::from_epoch_millis(at(-3600).timestamp_millis()),
            clock_out: None,
        }],
        ..CustodyState::initial()
    }
}

async fn put_state(shop: &Shop, state: &CustodyState) {
    let repo = PgCustodyRepository::new(shop.pool.clone());
    let mut tx = shop.pool.begin().await.expect("begin");
    let before = repo
        .load_state_in(&mut tx, &branch(MAIN))
        .await
        .expect("load the custody state");
    repo.store_state_in(&mut tx, &branch(MAIN), &before, state)
        .await
        .expect("store the custody state");
    tx.commit().await.expect("commit");
}

async fn state(shop: &Shop) -> CustodyState {
    PgCustodyRepository::new(shop.pool.clone())
        .load_state(&branch(MAIN))
        .await
        .expect("load the custody state")
}

/// MAIN ready for a purchase, a passport of `FRONT`, and a workflow with custody's step.
async fn guarded(shop: &Shop) -> (SaveTransactionWorkflow, String, String) {
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
    let flow = workflow(shop, vec![Arc::new(CustodySaveStep::new(custody(shop)))]);
    (flow, customer, passport)
}

#[tokio::test]
async fn with_custody_off_the_step_lets_every_save_through() {
    let shop = Shop::start().await;
    let (flow, customer, _) = guarded(&shop).await;
    let passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    // nobody clocked in, no service open, the owner saving, late at night
    let saved = flow
        .save_as(
            &branch(MAIN),
            form("off-0001-aaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &owner(),
            bangkok(23, 30),
        )
        .await
        .expect("custody is not in use");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    shop.finish().await;
}

#[tokio::test]
async fn with_custody_on_only_the_front_holding_the_drawer_saves() {
    let shop = Shop::start().await;
    let (flow, customer, passport) = guarded(&shop).await;
    put_state(&shop, &open_service()).await;
    let before = shop.everything().await;
    let front = staff(FRONT, Role::Front);

    // the owner, a Front who does not hold the drawer, a Manager who somehow does
    let owners_passport = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    let got = flow
        .save_as(
            &branch(MAIN),
            form("on-0001-aaaaaaaaaaaaa", &customer, &owners_passport),
            &SaveExtras::default(),
            &owner(),
            at(0),
        )
        .await;
    assert_eq!(refusal(got), NOT_CUSTODIAN);
    let other = staff("employee-8", Role::Front);
    let others_passport = shop.upload(MAIN, &other.id, "image/jpeg", JPEG).await;
    let got = flow
        .save_as(
            &branch(MAIN),
            form("on-0001-aaaaaaaaaaaaa", &customer, &others_passport),
            &SaveExtras::default(),
            &other,
            at(0),
        )
        .await;
    assert_eq!(refusal(got), NOT_CUSTODIAN);
    let manager = staff(FRONT, Role::Manager);
    let got = flow
        .save_as(
            &branch(MAIN),
            form("on-0001-aaaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &manager,
            at(0),
        )
        .await;
    assert_eq!(
        refusal(got),
        NOT_CUSTODIAN,
        "the role is the saver's, not the attendance record's"
    );
    assert_eq!(shop.count("transactions").await, 0);

    // the Front who holds it
    let saved = flow
        .save_as(
            &branch(MAIN),
            form("on-0001-aaaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &front,
            at(0),
        )
        .await
        .expect("the custodian saves");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    let stored = shop
        .transactions
        .find(&branch(MAIN), &saved.document_ref)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (stored.recorded_by.as_str(), stored.staff.as_str()),
        (FRONT, "user-employee-7")
    );
    assert_ne!(shop.everything().await, before);

    // a demo exchange is not custody's business: the owner saves one
    let demo_person = ReserveCustomer {
        mode: "DEMO".to_owned(),
        ..person()
    };
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;
    let mut demo = form("on-0002-aaaaaaaaaaaaa", &demo_customer, &owners_passport);
    demo.customer = demo_person;
    let saved = flow
        .save_as(
            &branch(MAIN),
            demo,
            &SaveExtras::default(),
            &owner(),
            bangkok(23, 30),
        )
        .await
        .expect("a demo exchange");
    assert_eq!(saved.document_ref, "TEST-XC-20261006-000001");
    shop.finish().await;
}

#[tokio::test]
async fn a_customer_on_hold_and_a_closed_service_refuse_the_save() {
    let shop = Shop::start().await;
    let (flow, customer, passport) = guarded(&shop).await;
    let front = staff(FRONT, Role::Front);
    let save = |submission: &'static str, when| {
        let (flow, cmd, front) = (
            flow.clone(),
            form(submission, &customer, &passport),
            front.clone(),
        );
        async move {
            flow.save_as(&branch(MAIN), cmd, &SaveExtras::default(), &front, when)
                .await
        }
    };

    let mut held = open_service();
    held.holds.push(CustomerHold {
        id: "hold-1".to_owned(),
        customer_id: customer.clone(),
        reason: "check".to_owned(),
        by: "employee-3".to_owned(),
        at: CustodyTime::from_epoch_millis(at(-60).timestamp_millis()),
        status: HoldStatus::Open,
        reviewed_by: None,
        reviewed_at: None,
        note: None,
    });
    put_state(&shop, &held).await;
    assert_eq!(refusal(save("hold-0001-aaaaaaaaaaa", at(0)).await), ON_HOLD);

    // released: saved
    held.holds[0].status = HoldStatus::Released;
    put_state(&shop, &held).await;
    save("hold-0001-aaaaaaaaaaa", at(0))
        .await
        .expect("the hold is released");

    // outside the hours of service, and in a phase that takes no exchange
    put_state(&shop, &open_service()).await;
    assert_eq!(
        refusal(save("hours-0001-aaaaaaaaaa", bangkok(10, 59)).await),
        SAVE_CLOSED
    );
    assert_eq!(
        refusal(save("hours-0001-aaaaaaaaaa", bangkok(23, 0)).await),
        SAVE_CLOSED
    );
    for phase in [Phase::Closed, Phase::Ready, Phase::Counting] {
        put_state(
            &shop,
            &CustodyState {
                phase,
                ..open_service()
            },
        )
        .await;
        assert_eq!(
            refusal(save("phase-0001-aaaaaaaaaa", at(0)).await),
            SAVE_CLOSED,
            "{phase:?}"
        );
    }
    assert_eq!(shop.count("transactions").await, 1);
    shop.finish().await;
}

#[tokio::test]
async fn after_the_cut_off_a_save_needs_its_draft_and_the_draft_is_completed() {
    let shop = Shop::start().await;
    let (flow, customer, passport) = guarded(&shop).await;
    let front = staff(FRONT, Role::Front);
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let draft = |id: &str, amount: &str| TxDraft {
        id: id.to_owned(),
        shift_id: shift.clone(),
        employee_id: FRONT.to_owned(),
        side: Side::Buy,
        currency: code("USD"),
        amount: money(amount),
        at: CustodyTime::from_epoch_millis(at(-120).timestamp_millis()),
        status: DraftStatus::Open,
        document_ref: None,
    };
    let cut_off = CustodyState {
        phase: Phase::CutOff,
        drafts: vec![draft("draft-100", "100"), draft("draft-7", "7")],
        ..open_service()
    };
    put_state(&shop, &cut_off).await;
    let with = |draft_id: Option<&str>| SaveExtras {
        staff_draft_id: draft_id.map(str::to_owned),
        ..SaveExtras::default()
    };

    // no draft; a draft that is not there; a draft for another amount
    for extras in [with(None), with(Some("draft-0")), with(Some("draft-7"))] {
        let got = flow
            .save_as(
                &branch(MAIN),
                form("cut-0001-aaaaaaaaaaaa", &customer, &passport),
                &extras,
                &front,
                at(0),
            )
            .await;
        assert_eq!(refusal(got), SAVE_CLOSED, "{extras:?}");
    }
    assert_eq!(
        state(&shop).await,
        cut_off,
        "a refused save leaves the drafts as they were"
    );

    // its own draft: saved, and the draft says so
    let saved = flow
        .save_as(
            &branch(MAIN),
            form("cut-0001-aaaaaaaaaaaa", &customer, &passport),
            &with(Some("draft-100")),
            &front,
            at(0),
        )
        .await
        .expect("begun before the cut-off");
    let after = state(&shop).await;
    let done = after
        .drafts
        .iter()
        .find(|d| d.id == "draft-100")
        .expect("the draft");
    assert_eq!(
        (done.status, done.document_ref.as_deref()),
        (DraftStatus::Completed, Some(saved.document_ref.as_str()))
    );
    let untouched = after
        .drafts
        .iter()
        .find(|d| d.id == "draft-7")
        .expect("the other draft");
    assert_eq!(
        (untouched.status, untouched.document_ref.as_deref()),
        (DraftStatus::Open, None)
    );

    // the same submission again, by the same Front: answered, though its draft is used up
    let again = flow
        .save_as(
            &branch(MAIN),
            form("cut-0001-aaaaaaaaaaaa", &customer, &passport),
            &with(Some("draft-100")),
            &front,
            at(60),
        )
        .await
        .expect("the saver's own repeat");
    assert_eq!(again, saved);
    assert_eq!(state(&shop).await, after);

    // by somebody else — another Front, with a copy of the same passport: not theirs to repeat
    let other = staff("employee-8", Role::Front);
    let others_passport = shop.upload(MAIN, &other.id, "image/jpeg", JPEG).await;
    let got = flow
        .save_as(
            &branch(MAIN),
            form("cut-0001-aaaaaaaaaaaa", &customer, &others_passport),
            &with(None),
            &other,
            at(60),
        )
        .await;
    assert_eq!(refusal(got), NOT_OWN_RETRY);
    assert_eq!(shop.count("transactions").await, 1);
    shop.finish().await;
}

#[tokio::test]
async fn a_draft_that_fails_to_complete_undoes_the_save() {
    // custody's writes are in the save's transaction: when its tables refuse
    // the write, there is no exchange either
    let shop = Shop::start().await;
    let (flow, customer, passport) = guarded(&shop).await;
    let front = staff(FRONT, Role::Front);
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let mut on = open_service();
    on.drafts.push(TxDraft {
        id: "draft-100".to_owned(),
        shift_id: shift,
        employee_id: FRONT.to_owned(),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
        at: CustodyTime::from_epoch_millis(at(-120).timestamp_millis()),
        status: DraftStatus::Open,
        document_ref: None,
    });
    put_state(&shop, &on).await;
    let before = shop.everything().await;
    let extras = SaveExtras {
        staff_draft_id: Some("draft-100".to_owned()),
        ..SaveExtras::default()
    };

    shop.break_writes("tx_drafts").await;
    let got = flow
        .save_as(
            &branch(MAIN),
            form("draft-0001-aaaaaaaaaa", &customer, &passport),
            &extras,
            &front,
            at(0),
        )
        .await;
    assert!(
        matches!(&got, Err(e) if !matches!(e, AppError::Domain(_))),
        "{got:?}"
    );
    shop.mend("tx_drafts").await;
    assert_eq!(shop.everything().await, before);
    assert_eq!(state(&shop).await, on);

    let saved = flow
        .save_as(
            &branch(MAIN),
            form("draft-0001-aaaaaaaaaa", &customer, &passport),
            &extras,
            &front,
            at(1),
        )
        .await
        .expect("after mending");
    assert_eq!(saved.document_ref, "XC-20261006-000001");
    shop.finish().await;
}
