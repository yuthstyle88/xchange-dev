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
//! by a teller on their own shift. The drafts are put straight into the custody tables.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c; cash-shift-transfer.md §6, rulings Q4, Q7): the
//! v2-actor save path goes, so every save here is a Blueprint-session teller's (`save_as_teller`) on their own OPEN
//! shift, opened by a verified count (CA-3), and a draft's `employee_id` is a `users.id`. The custodian, the
//! attendance and the Enable switch are gone from the custody state. Each test's subject and assertions are as
//! before: which draft a save completes (#154) does not depend on who may save.

// the shop of the save tests: its helpers prepare a branch and read it back
#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use save_common::*;
use xc_auth::domain::access::{Actor, Role as BpRole, ScopedRole};
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xc_shift::application::custody::service::{CustodyFactsPort, CustodyService};
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::workflows::custody_save_step::CustodySaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};

async fn org(shop: &Shop) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(&shop.pool)
        .await
        .expect("OR-1's organisation")
}

/// A user of the organisation holding `role` on `MAIN`, and the `access::Actor` for them.
async fn person(shop: &Shop, name: &str, role: BpRole) -> Actor {
    let org = org(shop).await;
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(&org)
    .bind(name)
    .fetch_one(&shop.pool)
    .await
    .expect("insert a user");
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5)",
    )
    .bind(&id)
    .bind(role.as_str())
    .bind(&org)
    .bind(MAIN)
    .bind(from)
    .execute(&shop.pool)
    .await
    .expect("give the user a role on MAIN");
    Actor::new(
        &id,
        &org,
        vec![ScopedRole::new(role, Some(branch(MAIN)), from, None)],
        None,
    )
}

/// The v2 actor a Blueprint-session teller's save carries since #767 (`legacy_actor`: id = `users.id`).
fn save_actor(teller: &Actor) -> SaveActor {
    SaveActor {
        id: teller.user_id.clone(),
        username: teller.user_id.clone(),
        employee_id: Some(teller.user_id.clone()),
        role: Role::Front,
    }
}

/// `teller`'s shift on `MAIN`, opened the way CA-3 opens it: the teller counts THB 1000 × 100, a BRANCH_MANAGER
/// verifies. Its id.
async fn opened(shop: &Shop, teller: &Actor, manager: &Actor) -> String {
    let thousand: i64 = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 1000",
    )
    .fetch_one(&shop.pool)
    .await
    .expect("the THB 1000 face value");
    lg4_fund_safe_thb(&shop.pool, "100000.00").await;
    let counts = CashCountService::pg(shop.pool.clone());
    let count = counts
        .submit(
            teller,
            SubmitCount {
                branch: branch(MAIN),
                shift_id: None,
                kind: CountKind::Open,
                lines: vec![CountLine::new("THB", thousand, 100)],
                note: None,
                idempotency_key: unique("count"),
            },
            at(-7200),
        )
        .await
        .expect("the teller submits the OPEN count");
    counts
        .verify(manager, &count.id, at(-7140))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count")
        .shift_id
        .expect("a verified OPEN count names the shift it opened")
}

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// LG-4 (#929, lane controller's 1:1 ruling): the OPEN count's cash now leaves MAIN's ledger SAFE (design §4.4),
/// so the safe holds exactly what the count takes (`OPENING` from BANK). Before the cutover nothing had to be there.
async fn lg4_fund_safe_thb(pool: &sqlx::PgPool, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(branch(MAIN)),
                currency: xc_kernel::money::CurrencyCode::THB,
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{}",
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund the safe through the ledger");
    tx.commit().await.expect("commit");
}

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(Transactor::new(shop.pool.clone()))
}

/// The custody service's questions to the other contexts, answered with nothing: judging and
/// completing a save asks none of them.
struct NothingOutside;

type Asked<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<T>> + Send + 'a>>;

// CA-10 (#626, #2955): the core asks `CustodyFactsPort` (v2's `CustodyPorts` is no longer asked); the same answers.
impl CustodyFactsPort for NothingOutside {
    fn service_window_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
        now: CustodyTime,
    ) -> Asked<'a, (ServiceWindow, i32)> {
        Box::pin(async move {
            let bkk_millis = now.epoch_millis() + 7 * 3600 * 1000;
            let bkk_day_start = bkk_millis.div_euclid(86_400 * 1000) * (86_400 * 1000);
            let utc_day_start = bkk_day_start - 7 * 3600 * 1000;
            let open = CustodyTime::from_epoch_millis(utc_day_start + 11 * 3600 * 1000);
            let close = CustodyTime::from_epoch_millis(utc_day_start + 23 * 3600 * 1000);
            Ok((ServiceWindow::Open { open, close }, 7 * 60))
        })
    }

    fn has_live_rate_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
    ) -> Asked<'a, (bool, String)> {
        Box::pin(async { Ok((false, String::new())) })
    }

    fn transactions_summary_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
    ) -> Asked<'a, (Vec<String>, Option<String>)> {
        Box::pin(async { Ok((Vec::new(), None)) })
    }

    fn customer_exists_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a xc_kernel::ids::BranchId,
        _customer_id: &'a str,
    ) -> Asked<'a, bool> {
        Box::pin(async { Ok(false) })
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

/// The branch day OPEN.
fn open_service() -> CustodyState {
    CustodyState {
        phase: Phase::Open,
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

/// What every test starts from: the save workflow with custody's step, a customer, the teller's (`me`) passport,
/// `me`'s OPEN shift, and `other`, a second teller of `MAIN` whose drafts `me` may finish (#154 option B).
struct G {
    flow: SaveTransactionWorkflow,
    customer: String,
    passport: String,
    shift: String,
    me: Actor,
    other: Actor,
}

async fn guarded(shop: &Shop) -> G {
    let manager = person(shop, "Krit", BpRole::BranchManager).await;
    let me = person(shop, "Nok", BpRole::Teller).await;
    let other = person(shop, "Ploy", BpRole::Teller).await;
    let shift = opened(shop, &me, &manager).await;
    shop.set_rate(MAIN, "USD", "32.1", "33").await;
    let customer = shop.reserve(MAIN, save_common::person()).await;
    let passport = shop.upload(MAIN, &me.user_id, "image/jpeg", JPEG).await;
    let step: Arc<dyn SaveStep> = Arc::new(CustodySaveStep::new(custody(shop)));
    let flow =
        SaveTransactionWorkflow::new(branch_tx(shop), shop.shifts.clone(), shop.files.clone())
            .with_steps(vec![step]);
    G {
        flow,
        customer,
        passport,
        shift,
        me,
        other,
    }
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
    let G {
        flow,
        customer,
        passport,
        shift,
        me,
        other,
    } = guarded(&shop).await;
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![draft(
                "left-by-8",
                &shift,
                &other.user_id,
                Side::Buy,
                "USD",
                "100",
            )],
            ..open_service()
        },
    )
    .await;

    let saved = flow
        .save_as_teller(
            &branch(MAIN),
            form("b154-0001-aaaaaaaaaaa", &customer, &passport),
            &naming("left-by-8"),
            &save_actor(&me),
            &me,
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
    let G {
        flow,
        customer,
        passport,
        shift,
        me,
        other,
    } = guarded(&shop).await;
    let cases = [
        // (draft, what differs)
        (
            draft("own-sell", &shift, &me.user_id, Side::Sell, "USD", "100"),
            "its side",
        ),
        (
            draft("own-eur", &shift, &me.user_id, Side::Buy, "EUR", "100"),
            "its currency",
        ),
        (
            draft("own-7", &shift, &me.user_id, Side::Buy, "USD", "7"),
            "its amount",
        ),
        (
            draft(
                "of-8-sell-7-eur",
                &shift,
                &other.user_id,
                Side::Sell,
                "EUR",
                "7",
            ),
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
            .save_as_teller(
                &branch(MAIN),
                form(&submission, &customer, &passport),
                &naming(&d.id),
                &save_actor(&me),
                &me,
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
    let G {
        flow,
        customer,
        passport,
        shift,
        me,
        other,
    } = guarded(&shop).await;
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![
                draft("mine", &shift, &me.user_id, Side::Buy, "USD", "100"),
                draft("of-8", &shift, &other.user_id, Side::Sell, "EUR", "7"),
            ],
            ..open_service()
        },
    )
    .await;
    let send = |draft_id: &'static str, when| {
        let (flow, me, cmd) = (
            flow.clone(),
            me.clone(),
            form("b154-0200-aaaaaaaaaaa", &customer, &passport),
        );
        async move {
            flow.save_as_teller(
                &branch(MAIN),
                cmd,
                &naming(draft_id),
                &save_actor(&me),
                &me,
                when,
            )
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
    let G {
        flow,
        customer,
        passport,
        shift,
        me,
        other,
    } = guarded(&shop).await;
    put_state(
        &shop,
        &CustodyState {
            drafts: vec![
                draft("first", &shift, &me.user_id, Side::Buy, "USD", "100"),
                draft("second", &shift, &other.user_id, Side::Buy, "USD", "100"),
            ],
            ..open_service()
        },
    )
    .await;
    let send = |draft_id: &'static str, when| {
        let (flow, me, cmd) = (
            flow.clone(),
            me.clone(),
            form("b154-0400-aaaaaaaaaaa", &customer, &passport),
        );
        async move {
            flow.save_as_teller(
                &branch(MAIN),
                cmd,
                &naming(draft_id),
                &save_actor(&me),
                &me,
                when,
            )
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
    let G {
        flow,
        customer,
        passport,
        shift,
        me,
        other: _,
    } = guarded(&shop).await;
    let mut done = draft("done", &shift, &me.user_id, Side::Buy, "USD", "100");
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
        .save_as_teller(
            &branch(MAIN),
            form("b154-0300-aaaaaaaaaaa", &customer, &passport),
            &naming("done"),
            &save_actor(&me),
            &me,
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
