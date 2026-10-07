//! Ticket #154 — which custody draft a save completes. Written before the fix. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Decided by the owner (#154, option B + (d), recorded 2026-10-07T04:19Z): after an accepted save
//! a draft is completed only when it is `Open` and its side, currency and amount are the saved
//! exchange's — whoever opened it. A named draft that does not match is left as it was and the
//! save still succeeds. A `Completed` draft is never changed again, so its document number is
//! never overwritten. Read by the lane controller (#154, 2026-10-07; the owner may reverse it):
//! one exchange completes at most one draft — a save whose document number is already a draft's
//! completes no other.
//!
//! Every save here is a live BUY of 100 USD (`save_common::form`), made at noon in an open service
//! by `FRONT`, who holds the drawer. The drafts are put straight into the custody tables.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.

// the shop of the save tests: its helpers prepare a branch and read it back
#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::workflows::custody_save_step::CustodySaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};

const FRONT: &str = "employee-7";
const OTHER: &str = "employee-8";

fn front() -> SaveActor {
    SaveActor {
        id: FRONT.to_owned(),
        username: format!("user-{FRONT}"),
        employee_id: Some(FRONT.to_owned()),
        role: Role::Front,
    }
}

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(Transactor::new(shop.pool.clone()))
}

/// The custody service's questions to the other contexts, answered with nothing: judging and
/// completing a save asks none of them.
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

async fn draft_now(shop: &Shop, id: &str) -> TxDraft {
    PgCustodyRepository::new(shop.pool.clone())
        .load_state(&branch(MAIN))
        .await
        .expect("load the custody state")
        .drafts
        .into_iter()
        .find(|d| d.id == id)
        .expect("the draft")
}

/// A shop ready for a purchase, `FRONT`'s passport, a workflow with custody's step, and the id of
/// the open shift.
async fn guarded(shop: &Shop) -> (SaveTransactionWorkflow, String, String, String) {
    let (customer, _) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let passport = shop.upload(MAIN, FRONT, "image/jpeg", JPEG).await;
    let step: Arc<dyn SaveStep> = Arc::new(CustodySaveStep::new(custody(shop)));
    let flow =
        SaveTransactionWorkflow::new(branch_tx(shop), shop.shifts.clone(), shop.files.clone())
            .with_steps(vec![step]);
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    (flow, customer, passport, shift)
}

/// An open draft of `employee_id` in `shift`.
fn draft(
    id: &str,
    shift: &str,
    employee_id: &str,
    side: Side,
    currency: &str,
    amount: &str,
) -> TxDraft {
    TxDraft {
        id: id.to_owned(),
        shift_id: shift.to_owned(),
        employee_id: employee_id.to_owned(),
        side,
        currency: code(currency),
        amount: money(amount),
        at: CustodyTime::from_epoch_millis(at(-120).timestamp_millis()),
        status: DraftStatus::Open,
        document_ref: None,
    }
}

fn naming(draft_id: &str) -> SaveExtras {
    SaveExtras {
        staff_draft_id: Some(draft_id.to_owned()),
        ..SaveExtras::default()
    }
}

fn open_untouched(d: &TxDraft, what: &str) {
    assert_eq!(
        (d.status, d.document_ref.as_deref()),
        (DraftStatus::Open, None),
        "{what}"
    );
}

/// Holds before the fix and after it: option B completes a matching open draft whoever opened
/// it — the drafts a previous holder of the drawer left open can be finished by the next one.
#[tokio::test]
async fn another_employees_open_draft_with_the_same_figures_is_completed() {
    let shop = Shop::start().await;
    let (flow, customer, passport, shift) = guarded(&shop).await;
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![draft("left-by-8", &shift, OTHER, Side::Buy, "USD", "100")],
            ..open_service()
        },
    )
    .await;

    let saved = flow
        .save_as(
            &branch(MAIN),
            form("b154-0001-aaaaaaaaaaa", &customer, &passport),
            &naming("left-by-8"),
            &front(),
            at(0),
        )
        .await
        .expect("saved");
    let done = draft_now(&shop, "left-by-8").await;
    assert_eq!(
        (done.status, done.document_ref.as_deref()),
        (DraftStatus::Completed, Some(saved.document_ref.as_str()))
    );
    shop.finish().await;
}

/// The saver's own open draft, and another employee's, each for other figures than the exchange:
/// the save succeeds, the draft stays open with no document number.
#[tokio::test]
async fn a_draft_for_other_figures_is_left_open_and_the_save_still_succeeds() {
    let shop = Shop::start().await;
    let (flow, customer, passport, shift) = guarded(&shop).await;
    let cases = [
        // (draft, what differs)
        (
            draft("own-sell", &shift, FRONT, Side::Sell, "USD", "100"),
            "its side",
        ),
        (
            draft("own-eur", &shift, FRONT, Side::Buy, "EUR", "100"),
            "its currency",
        ),
        (
            draft("own-7", &shift, FRONT, Side::Buy, "USD", "7"),
            "its amount",
        ),
        (
            draft("of-8-sell-7-eur", &shift, OTHER, Side::Sell, "EUR", "7"),
            "everything, and another employee's",
        ),
    ];
    put_state(
        &shop,
        &CustodyState {
            drafts: cases.iter().map(|(d, _)| d.clone()).collect(),
            ..open_service()
        },
    )
    .await;

    for (n, (d, differs)) in cases.iter().enumerate() {
        let submission = format!("b154-01{n:02}-aaaaaaaaaa");
        let saved = flow
            .save_as(
                &branch(MAIN),
                form(&submission, &customer, &passport),
                &naming(&d.id),
                &front(),
                at(0),
            )
            .await;
        assert!(
            saved.is_ok(),
            "a save naming a draft that differs in {differs} is still saved: {saved:?}"
        );
        open_untouched(
            &draft_now(&shop, &d.id).await,
            &format!("a draft that differs in {differs}"),
        );
    }
    assert_eq!(shop.count("transactions").await, cases.len() as i64);
    shop.finish().await;
}

/// The issue's own table: the same submission sent again naming a second draft — of another
/// employee, for other figures — does not close that draft; the first draft keeps its number.
#[tokio::test]
async fn a_repeat_naming_a_second_draft_for_other_figures_leaves_it_open() {
    let shop = Shop::start().await;
    let (flow, customer, passport, shift) = guarded(&shop).await;
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![
                draft("mine", &shift, FRONT, Side::Buy, "USD", "100"),
                draft("of-8", &shift, OTHER, Side::Sell, "EUR", "7"),
            ],
            ..open_service()
        },
    )
    .await;
    let send = |draft_id: &'static str, when| {
        let (flow, cmd) = (
            flow.clone(),
            form("b154-0200-aaaaaaaaaaa", &customer, &passport),
        );
        async move {
            flow.save_as(&branch(MAIN), cmd, &naming(draft_id), &front(), when)
                .await
        }
    };

    let saved = send("mine", at(0)).await.expect("saved");
    let again = send("of-8", at(60)).await.expect("the saver's own repeat");
    assert_eq!(again, saved);

    open_untouched(
        &draft_now(&shop, "of-8").await,
        "the second draft named by a repeat",
    );
    let first = draft_now(&shop, "mine").await;
    assert_eq!(
        (first.status, first.document_ref.as_deref()),
        (DraftStatus::Completed, Some(saved.document_ref.as_str()))
    );
    assert_eq!(shop.count("transactions").await, 1);
    shop.finish().await;
}

/// One exchange completes at most one draft (the lane controller's reading of B, #154; the owner
/// may reverse it): a repeat of the same submission naming a second open draft with the same
/// figures leaves that draft open, because the exchange's number is already a draft's.
#[tokio::test]
async fn a_repeat_naming_a_second_draft_with_the_same_figures_leaves_it_open() {
    let shop = Shop::start().await;
    let (flow, customer, passport, shift) = guarded(&shop).await;
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![
                draft("first", &shift, FRONT, Side::Buy, "USD", "100"),
                draft("second", &shift, OTHER, Side::Buy, "USD", "100"),
            ],
            ..open_service()
        },
    )
    .await;
    let send = |draft_id: &'static str, when| {
        let (flow, cmd) = (
            flow.clone(),
            form("b154-0400-aaaaaaaaaaa", &customer, &passport),
        );
        async move {
            flow.save_as(&branch(MAIN), cmd, &naming(draft_id), &front(), when)
                .await
        }
    };

    let saved = send("first", at(0)).await.expect("saved");
    let again = send("second", at(60))
        .await
        .expect("the saver's own repeat");
    assert_eq!(again, saved);

    open_untouched(
        &draft_now(&shop, "second").await,
        "a second draft with the same figures, named by a repeat",
    );
    let first = draft_now(&shop, "first").await;
    assert_eq!(
        (first.status, first.document_ref.as_deref()),
        (DraftStatus::Completed, Some(saved.document_ref.as_str()))
    );
    shop.finish().await;
}

/// (d): a completed draft is never changed again — a later exchange that names it, with the same
/// figures, is saved and the draft keeps the number of the exchange that completed it.
#[tokio::test]
async fn a_completed_draft_keeps_its_first_document_number() {
    let shop = Shop::start().await;
    let (flow, customer, passport, shift) = guarded(&shop).await;
    let mut done = draft("done", &shift, FRONT, Side::Buy, "USD", "100");
    done.status = DraftStatus::Completed;
    done.document_ref = Some("XC-20261005-000042".to_owned());
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![done.clone()],
            ..open_service()
        },
    )
    .await;

    let saved = flow
        .save_as(
            &branch(MAIN),
            form("b154-0300-aaaaaaaaaaa", &customer, &passport),
            &naming("done"),
            &front(),
            at(0),
        )
        .await
        .expect("a save naming a completed draft is still saved");
    assert_ne!(saved.document_ref, "XC-20261005-000042");
    assert_eq!(
        draft_now(&shop, "done").await,
        done,
        "the draft is unchanged"
    );
    shop.finish().await;
}
