//! CA-7 (#623), stage 2: an exchange saved by a Blueprint-session teller posts to that teller's own OPEN shift
//! (design `docs/design/blueprint/cash-shift-transfer.md` C1, §4.3 rows "an exchange posts to the saver's own OPEN
//! shift; no open shift → `noOpenShift`" and "after cut-off, a save needs the saver's own OPEN draft"), with no
//! branch custodian. Lane controller's ruling on #623 (issuecomment-6100843512), option A:
//! - a Blueprint-session saver (`users.id`) saves on their own OPEN shift, judged by `BranchDay::check_save`;
//! - a v2 password saver (`employees.id`) keeps today's `apply_delta_in(branch)` path until CA-10 #626, so the
//!   replayed v2 save fixtures keep their expectations and are not repeated here.
//!
//! Written before the implementation by the ticket author (claude/709ec749). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! [required] by these tests, in `xchange::application::workflows::save_transaction::SaveTransactionWorkflow`:
//! - `save_as_teller(branch: &BranchId, cmd: SaveTransaction, extras: &SaveExtras, actor: &SaveActor,
//!   teller: &xc_auth::domain::access::Actor, now: DateTime<Utc>) -> AppResult<SavedTransaction>`: the save of
//!   `save_as`, except that
//!   - the exchange's stock moves on the OPEN shift whose `teller_id` is `teller.user_id`
//!     (`ShiftService::apply_delta_for_teller_in`), never on the branch's v2 shift, and the stored exchange names
//!     that shift (`transactions.shift_id`);
//!   - no such shift → refused with `ShiftError::NoOpenShift`'s sentence, and nothing is written;
//!   - with custody on for the branch, the registered `CustodySaveStep` judges the save by
//!     `BranchDay::check_save(phase, teller, own_shift, drafts, attempt, window_needs_draft)`: no custodian, so two
//!     tellers each on their own shift both save; after cut-off the teller needs an OPEN draft of their own
//!     (`TxDraft.employee_id` = `teller.user_id`) on their own shift, matching the save; none → `SaveClosed`.
//!
//! `SaveActor` and `SaveExtras` gain no field: both are built by struct literal in `kiosk_ports.rs` and
//! `booking_save_step.rs`, which are outside #623's scope (ruling, item 2). The teller travels as its own argument.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`). Without it they
//! FAIL; none skips.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c; cash-shift-transfer.md §6, rulings Q2–Q4, Q7):
//! - the custody state has no `enabled` and no `custodian` any more (M3 drops both), so `custody_in` writes neither;
//! - **changes on purpose (Q2: the Enable switch goes, the Blueprint's model always applies):** the save checks no
//!   longer stand aside while a branch has never been "enabled". A teller whose day CA-7e moved to READY is refused
//!   `SaveClosed` until service starts (`a_tellers_save_before_service_starts_is_closed_without_any_custody_set_up`);
//!   so the tests that save put the day OPEN first;
//! - the v2-actor save path goes (Q4), so the rules `save_steps_test.rs` pinned on it are restated here on the
//!   teller's path: a customer on hold, the hours of service, and a draft that fails to complete undoing the save.

#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use save_common::*;
use xc_auth::domain::access::{Actor, Role as BpRole, ScopedRole};
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xc_shift::application::custody::service::{CustodyFactsPort, CustodyService};
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_error::CustodyError;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::workflows::custody_save_step::CustodySaveStep;
use xchange::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};

// ── the people ─────────────────────────────────────────────────────────────

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

// ── the shifts ─────────────────────────────────────────────────────────────

/// `teller`'s shift on `MAIN`, opened the way CA-3 opens it: the teller counts THB 1000 × 10, a BRANCH_MANAGER
/// verifies. Its id.
async fn opened(shop: &Shop, teller: &Actor, manager: &Actor) -> String {
    let thousand: i64 = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 1000",
    )
    .fetch_one(&shop.pool)
    .await
    .expect("the THB 1000 face value");
    lg4_fund_safe_thb(&shop.pool, "10000.00").await;
    let counts = CashCountService::pg(shop.pool.clone());
    let count = counts
        .submit(
            teller,
            SubmitCount {
                branch: branch(MAIN),
                shift_id: None,
                kind: CountKind::Open,
                lines: vec![CountLine::new("THB", thousand, 10)],
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

async fn expected(shop: &Shop, shift: &str, currency: &str) -> Decimal {
    use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
    use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
    PgLedgerShiftBalances::new(shop.pool.clone())
        .shift_balances(&shift.to_owned())
        .await
        .expect("read a balance")
        .into_iter()
        .find(|b| b.currency.as_str() == currency)
        .map(|b| Decimal::from_i128_with_scale(b.expected.minor(), 2))
        .unwrap_or(Decimal::ZERO)
}

fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal")
}

async fn shift_of(shop: &Shop, document_ref: &str) -> Option<String> {
    sqlx::query_scalar("SELECT shift_id FROM transactions WHERE document_ref = $1")
        .bind(document_ref)
        .fetch_one(&shop.pool)
        .await
        .expect("the stored exchange")
}

// ── custody ────────────────────────────────────────────────────────────────

/// The custody service's questions to the other contexts: nothing outside, and today's window open 11:00–23:00
/// Bangkok, so noon (`at(0)`) is inside it and needs no draft.
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

fn branch_tx(shop: &Shop) -> BranchTx {
    BranchTx::new(Transactor::new(shop.pool.clone()))
}

/// The save workflow with custody's step registered.
fn guarded(shop: &Shop) -> SaveTransactionWorkflow {
    let custody = CustodyService::new(
        Arc::new(PgCustodyRepository::new(shop.pool.clone())),
        shop.shifts.clone(),
        Arc::new(NothingOutside),
        branch_tx(shop),
    );
    let step: Arc<dyn SaveStep> = Arc::new(CustodySaveStep::new(custody));
    SaveTransactionWorkflow::new(branch_tx(shop), shop.shifts.clone(), shop.files.clone())
        .with_steps(vec![step])
}

/// `MAIN`'s branch day in `phase`, with `drafts`.
async fn custody_in(shop: &Shop, phase: Phase, drafts: Vec<TxDraft>) {
    let repo = PgCustodyRepository::new(shop.pool.clone());
    let mut tx = shop.pool.begin().await.expect("begin");
    let before = repo
        .load_state_in(&mut tx, &branch(MAIN))
        .await
        .expect("load the custody state");
    let state = CustodyState {
        phase,
        drafts,
        ..before.clone()
    };
    repo.store_state_in(&mut tx, &branch(MAIN), &before, &state)
        .await
        .expect("store the custody state");
    tx.commit().await.expect("commit");
}

fn draft(id: &str, shift: &str, owner: &str) -> TxDraft {
    TxDraft {
        id: id.to_owned(),
        shift_id: shift.to_owned(),
        employee_id: owner.to_owned(),
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
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

fn refused(result: AppResult<SavedTransaction>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected a refusal (AppError::Domain), got {other:?}"),
    }
}

/// The rate, a customer and the teller's passport on `MAIN`. No v2 shift is opened.
async fn counter(shop: &Shop, teller: &Actor) -> (String, String) {
    shop.set_rate(MAIN, "USD", "32.1", "33").await;
    let customer = shop.reserve(MAIN, save_common::person()).await;
    let passport = shop.upload(MAIN, &teller.user_id, "image/jpeg", JPEG).await;
    (customer, passport)
}

// ── the tests ──────────────────────────────────────────────────────────────

/// C1: Nok's exchange moves Nok's drawer only. The shop buys USD 100 at 32.1: USD +100, THB −3,210.
#[tokio::test]
async fn a_tellers_exchange_posts_to_their_own_open_shift_only() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let ploy = person(&shop, "Ploy", BpRole::Teller).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    let ploys = opened(&shop, &ploy, &manager).await;
    let noks = opened(&shop, &nok, &manager).await;
    let (customer, passport) = counter(&shop, &nok).await;
    custody_in(&shop, Phase::Open, Vec::new()).await; // CA-10: the day's phase always applies

    let saved = shop
        .workflow
        .save_as_teller(
            &branch(MAIN),
            form("ca7-0001-aaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &save_actor(&nok),
            &nok,
            at(0),
        )
        .await
        .expect("Nok's exchange is saved");

    assert_eq!(
        shift_of(&shop, &saved.document_ref).await.as_deref(),
        Some(noks.as_str()),
        "the stored exchange names Nok's shift"
    );
    assert_eq!(expected(&shop, &noks, "USD").await, d("100"));
    assert_eq!(expected(&shop, &noks, "THB").await, d("6790"));
    assert_eq!(
        (
            expected(&shop, &ploys, "USD").await,
            expected(&shop, &ploys, "THB").await
        ),
        (d("0"), d("10000")),
        "Ploy's shift is untouched"
    );
    shop.finish().await;
}

/// C1, §4.3: a teller with no OPEN shift of their own is refused `noOpenShift`, and nothing is written, even
/// while another teller's shift is open.
#[tokio::test]
async fn a_teller_with_no_open_shift_of_their_own_is_refused_and_nothing_is_written() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let ploy = person(&shop, "Ploy", BpRole::Teller).await;
    let mai = person(&shop, "Mai", BpRole::Teller).await;
    let ploys = opened(&shop, &ploy, &manager).await;
    let (customer, passport) = counter(&shop, &mai).await;
    custody_in(&shop, Phase::Open, Vec::new()).await; // CA-10: the day's phase always applies

    let answer = shop
        .workflow
        .save_as_teller(
            &branch(MAIN),
            form("ca7-0002-aaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &save_actor(&mai),
            &mai,
            at(0),
        )
        .await;
    assert_eq!(refused(answer), ShiftError::NoOpenShift.to_string());
    assert_eq!(shop.count("transactions").await, 0, "no exchange is stored");
    assert_eq!(
        expected(&shop, &ploys, "THB").await,
        d("10000"),
        "Ploy's shift is untouched"
    );
    shop.finish().await;
}

/// The branch's v2 shift (opened by `save_common`'s v2 opener) is not the teller's: the teller is still refused
/// `noOpenShift`, and the v2 shift does not move.
#[tokio::test]
async fn the_branchs_v2_shift_is_not_the_tellers_shift() {
    let shop = Shop::start().await;
    let mai = person(&shop, "Mai", BpRole::Teller).await;
    let (customer, passport) = counter(&shop, &mai).await;
    shop.open_shift(MAIN, &[("THB", "100000"), ("USD", "1000")])
        .await;
    let v2_before = shop.balances(MAIN).await;
    custody_in(&shop, Phase::Open, Vec::new()).await; // CA-10: the day's phase always applies

    let answer = shop
        .workflow
        .save_as_teller(
            &branch(MAIN),
            form("ca7-0003-aaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &save_actor(&mai),
            &mai,
            at(0),
        )
        .await;
    assert_eq!(refused(answer), ShiftError::NoOpenShift.to_string());
    assert_eq!(
        shop.balances(MAIN).await,
        v2_before,
        "the v2 shift does not move"
    );
    assert_eq!(shop.count("transactions").await, 0);
    shop.finish().await;
}

/// C1: with custody on and service OPEN, there is no branch custodian. Two tellers, each on their own shift, both
/// save; the v2 custodian named in the state is neither of them.
#[tokio::test]
async fn with_service_open_two_tellers_on_their_own_shifts_both_save() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let ploy = person(&shop, "Ploy", BpRole::Teller).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    let ploys = opened(&shop, &ploy, &manager).await;
    let noks = opened(&shop, &nok, &manager).await;
    custody_in(&shop, Phase::Open, Vec::new()).await;
    let flow = guarded(&shop);
    shop.set_rate(MAIN, "USD", "32.1", "33").await;

    for (who, shift, submission) in [
        (&ploy, &ploys, "ca7-0004-aaaaaaaaaaaa"),
        (&nok, &noks, "ca7-0005-aaaaaaaaaaaa"),
    ] {
        let customer = shop.reserve(MAIN, save_common::person()).await;
        let passport = shop.upload(MAIN, &who.user_id, "image/jpeg", JPEG).await;
        let saved = flow
            .save_as_teller(
                &branch(MAIN),
                form(submission, &customer, &passport),
                &SaveExtras::default(),
                &save_actor(who),
                who,
                at(0),
            )
            .await
            .unwrap_or_else(|e| panic!("a teller on their own shift saves while OPEN: {e:?}"));
        assert_eq!(
            shift_of(&shop, &saved.document_ref).await.as_deref(),
            Some(shift.as_str())
        );
    }
    shop.finish().await;
}

/// §4.3 (kept rule of `custody_machine.rs:1194-1214`): after cut-off, a teller's save needs an OPEN draft of
/// their own, on their own shift, matching the save. Without it: `SaveClosed`, nothing written. Another teller's
/// draft, or the teller's draft on another shift, does not count. With it: saved, and the draft is completed.
#[tokio::test]
async fn after_cut_off_a_tellers_save_needs_their_own_open_draft_on_their_own_shift() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let ploy = person(&shop, "Ploy", BpRole::Teller).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    let ploys = opened(&shop, &ploy, &manager).await;
    let noks = opened(&shop, &nok, &manager).await;
    let flow = guarded(&shop);
    let (customer, passport) = counter(&shop, &nok).await;

    custody_in(
        &shop,
        Phase::CutOff,
        vec![
            draft("ploys-own", &ploys, &ploy.user_id),
            draft("noks-on-ploys-shift", &ploys, &nok.user_id),
            draft("noks-own", &noks, &nok.user_id),
        ],
    )
    .await;

    for (named, why) in [
        (None, "no draft named"),
        (Some("ploys-own"), "another teller's draft"),
        (Some("noks-on-ploys-shift"), "Nok's draft on another shift"),
    ] {
        let extras = named.map(naming).unwrap_or_default();
        let answer = flow
            .save_as_teller(
                &branch(MAIN),
                form("ca7-0006-aaaaaaaaaaaa", &customer, &passport),
                &extras,
                &save_actor(&nok),
                &nok,
                at(0),
            )
            .await;
        assert_eq!(
            refused(answer),
            CustodyError::SaveClosed.to_string(),
            "{why}"
        );
        assert_eq!(
            shop.count("transactions").await,
            0,
            "{why}: nothing written"
        );
    }

    let saved = flow
        .save_as_teller(
            &branch(MAIN),
            form("ca7-0006-aaaaaaaaaaaa", &customer, &passport),
            &naming("noks-own"),
            &save_actor(&nok),
            &nok,
            at(0),
        )
        .await
        .expect("Nok's own open draft on Nok's shift lets the save through");
    assert_eq!(
        shift_of(&shop, &saved.document_ref).await.as_deref(),
        Some(noks.as_str())
    );
    let state = PgCustodyRepository::new(shop.pool.clone())
        .load_state(&branch(MAIN))
        .await
        .expect("load the custody state");
    let own = state
        .drafts
        .iter()
        .find(|d| d.id == "noks-own")
        .expect("the draft");
    assert_eq!(
        (own.status, own.document_ref.as_deref()),
        (DraftStatus::Completed, Some(saved.document_ref.as_str()))
    );
    shop.finish().await;
}

/// §4.3: in a phase other than OPEN or CUT_OFF (here READY, before service starts), a teller's save is refused
/// `SaveClosed` even on their own shift.
#[tokio::test]
async fn before_service_starts_a_tellers_save_is_closed() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    opened(&shop, &nok, &manager).await;
    custody_in(&shop, Phase::Ready, Vec::new()).await;
    let flow = guarded(&shop);
    let (customer, passport) = counter(&shop, &nok).await;

    let answer = flow
        .save_as_teller(
            &branch(MAIN),
            form("ca7-0007-aaaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &save_actor(&nok),
            &nok,
            at(0),
        )
        .await;
    assert_eq!(refused(answer), CustodyError::SaveClosed.to_string());
    assert_eq!(shop.count("transactions").await, 0);
    shop.finish().await;
}

/// CA-10 #626, **changes on purpose** (Q2): with no custody state written by anybody, a teller whose OPEN count
/// was verified (CA-7e: the day is READY) is refused `SaveClosed` until service starts. Before CA-10 the checks
/// stood aside for a branch that was never "enabled", and this save went through.
#[tokio::test]
async fn a_tellers_save_before_service_starts_is_closed_without_any_custody_set_up() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    opened(&shop, &nok, &manager).await;
    let flow = guarded(&shop);
    let (customer, passport) = counter(&shop, &nok).await;

    let answer = flow
        .save_as_teller(
            &branch(MAIN),
            form("ca10-0001-aaaaaaaaaaa", &customer, &passport),
            &SaveExtras::default(),
            &save_actor(&nok),
            &nok,
            at(0),
        )
        .await;
    assert_eq!(refused(answer), CustodyError::SaveClosed.to_string());
    assert_eq!(shop.count("transactions").await, 0);
    shop.finish().await;
}

/// Ported from `save_steps_test.rs` (v2 path, retired by Q4): a customer with an OPEN hold is refused on a teller's
/// first save; once the hold is released the same save goes through.
#[tokio::test]
async fn a_customer_on_hold_refuses_a_tellers_save_until_released() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    opened(&shop, &nok, &manager).await;
    custody_in(&shop, Phase::Open, Vec::new()).await;
    let flow = guarded(&shop);
    let (customer, passport) = counter(&shop, &nok).await;
    let hold = |status: HoldStatus| CustomerHold {
        id: "hold-1".to_owned(),
        customer_id: customer.clone(),
        reason: "check".to_owned(),
        by: manager.user_id.clone(),
        at: CustodyTime::from_epoch_millis(at(-60).timestamp_millis()),
        status,
        reviewed_by: None,
        reviewed_at: None,
        note: None,
    };
    with_holds(&shop, vec![hold(HoldStatus::Open)]).await;
    let extras = SaveExtras::default();
    let first = flow
        .save_as_teller(
            &branch(MAIN),
            form("ca10-0002-aaaaaaaaaaa", &customer, &passport),
            &extras,
            &save_actor(&nok),
            &nok,
            at(0),
        )
        .await;
    assert_eq!(refused(first), CustodyError::CustomerOnHold.to_string());
    assert_eq!(shop.count("transactions").await, 0);

    with_holds(&shop, vec![hold(HoldStatus::Released)]).await;
    flow.save_as_teller(
        &branch(MAIN),
        form("ca10-0002-aaaaaaaaaaa", &customer, &passport),
        &extras,
        &save_actor(&nok),
        &nok,
        at(0),
    )
    .await
    .expect("the hold is released");
    assert_eq!(shop.count("transactions").await, 1);
    shop.finish().await;
}

/// Ported from `save_steps_test.rs`: outside the hours of service (11:00–23:00 Bangkok here) a teller's first save
/// is refused `SaveClosed` even with the day OPEN and the teller on their own shift.
#[tokio::test]
async fn outside_the_hours_of_service_a_tellers_save_is_closed() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    opened(&shop, &nok, &manager).await;
    custody_in(&shop, Phase::Open, Vec::new()).await;
    let flow = guarded(&shop);
    let (customer, passport) = counter(&shop, &nok).await;
    // at(0) is noon in Bangkok: 10:59 and 23:00
    for when in [at(-3_660), at(11 * 3_600)] {
        let answer = flow
            .save_as_teller(
                &branch(MAIN),
                form("ca10-0003-aaaaaaaaaaa", &customer, &passport),
                &SaveExtras::default(),
                &save_actor(&nok),
                &nok,
                when,
            )
            .await;
        assert_eq!(
            refused(answer),
            CustodyError::SaveClosed.to_string(),
            "{when}"
        );
    }
    assert_eq!(shop.count("transactions").await, 0);
    shop.finish().await;
}

/// Ported from `save_steps_test.rs`: custody's writes are in the save's transaction. When completing the teller's
/// draft fails, there is no exchange either; once mended, the same save goes through.
#[tokio::test]
async fn a_draft_that_fails_to_complete_undoes_a_tellers_save() {
    let shop = Shop::start().await;
    let manager = person(&shop, "Krit", BpRole::BranchManager).await;
    let nok = person(&shop, "Nok", BpRole::Teller).await;
    let noks = opened(&shop, &nok, &manager).await;
    custody_in(
        &shop,
        Phase::Open,
        vec![draft("noks-own", &noks, &nok.user_id)],
    )
    .await;
    let flow = guarded(&shop);
    let (customer, passport) = counter(&shop, &nok).await;
    let before = shop.everything().await;

    shop.break_writes("tx_drafts").await;
    let got = flow
        .save_as_teller(
            &branch(MAIN),
            form("ca10-0004-aaaaaaaaaaa", &customer, &passport),
            &naming("noks-own"),
            &save_actor(&nok),
            &nok,
            at(0),
        )
        .await;
    assert!(
        matches!(&got, Err(e) if !matches!(e, AppError::Domain(_))),
        "{got:?}"
    );
    shop.mend("tx_drafts").await;
    assert_eq!(
        shop.everything().await,
        before,
        "nothing of the save is kept"
    );

    flow.save_as_teller(
        &branch(MAIN),
        form("ca10-0004-aaaaaaaaaaa", &customer, &passport),
        &naming("noks-own"),
        &save_actor(&nok),
        &nok,
        at(1),
    )
    .await
    .expect("after mending");
    assert_eq!(shop.count("transactions").await, 1);
    shop.finish().await;
}

/// `MAIN`'s custody state with `holds`, everything else as stored.
async fn with_holds(shop: &Shop, holds: Vec<CustomerHold>) {
    let repo = PgCustodyRepository::new(shop.pool.clone());
    let mut tx = shop.pool.begin().await.expect("begin");
    let before = repo
        .load_state_in(&mut tx, &branch(MAIN))
        .await
        .expect("load the custody state");
    let state = CustodyState {
        holds,
        ..before.clone()
    };
    repo.store_state_in(&mut tx, &branch(MAIN), &before, &state)
        .await
        .expect("store the custody state");
    tx.commit().await.expect("commit");
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
