//! Row AG-3 (`docs/design/xchange-v3.md` §5.4c *One agent, one drawer, one per branch*,
//! *Server-made drawer jobs: one entry and one id scheme*, *Who writes `device_jobs.status`*):
//! the device service's side of the counter drawer agent. Written before the implementation.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! An agent is written straight into `device_agents` (row AG-1's table), so nothing here
//! depends on rows AG-2 or AG-4. The names `DrawerJobKey`, `DrawerJobOutcome`,
//! `PassportSource`, `DeviceService::queue_drawer[_in]` are this ticket's; this file does not
//! compile until they exist.
//!
//! Needs `TEST_DATABASE_URL` (see `devices_common/mod.rs`); nothing here skips.

mod common;
mod devices_common;
mod routes_common;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use devices_common::*;
use rust_decimal::Decimal;
use serde_json::Value;
use xchange::infrastructure::devices::drawer_job::{
    DrawerJobKey, DrawerJobOutcome, PassportSource,
};
use xchange::infrastructure::devices::printer_models::{DrawerPulse, PrinterModel, PrinterModels};
use xchange::infrastructure::devices::printer_settings::PrinterSettings;
use xchange::infrastructure::devices::simulated_driver::{DeviceDriver, SimulatedDriver};

const AGENT_MAIN: &str = "a0000000-0000-4000-8000-000000000001";
const AGENT_B9: &str = "a0000000-0000-4000-8000-000000000009";

/// An active agent of the branch, as the owner's `add` leaves it (row AG-1's table).
async fn agent(db: &Devices, branch_id: &str, id: &str) {
    sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
         VALUES ($1::uuid, $2, 'Mac เคาน์เตอร์ 1', now(), 'owner')",
    )
    .bind(id)
    .bind(branch_id)
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("insert an agent of {branch_id}: {e}"));
}

async fn revoke(db: &Devices, id: &str) {
    sqlx::query(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .expect("revoke the agent");
}

async fn setting(db: &Devices, branch_id: &str, key: &str, value: &str) {
    sqlx::query(
        "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, $2, $3) \
         ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(branch_id)
    .bind(key)
    .bind(value)
    .execute(&db.pool)
    .await
    .expect("set a device setting");
}

fn saved(reference: &str, cash: bool) -> SavedExchange {
    SavedExchange {
        cash,
        submission_id: format!("sub-{reference}"),
        document_ref: reference.to_owned(),
        currency: "USD".to_owned(),
        amount: Decimal::from_str("100.50").expect("a decimal"),
        rate: Decimal::from_str("32.1234").expect("a decimal"),
        passport_key: None,
        slip_key: None,
    }
}

/// The `transactions` row the save writes before its device step (amended for #759, RE-4e:
/// rates.md §4.9 R9 and the ticket author's ruling, the device service reads the stored row's
/// `unit_size` by document number, and a missing row is an error). The rate is per 1, as the
/// saved exchange carries it, so `unit_size` is 1; the principal is the baht HALF_UP to 2
/// places. A customer and an open shift of the branch come with it. A row already there (the
/// same document saved again) is kept.
async fn the_saved_row(
    pool: &sqlx::PgPool,
    branch_id: &str,
    document_ref: &str,
    submission_id: &str,
    currency: &str,
    amount: &str,
    rate: &str,
) {
    sqlx::query(
        // CI-4 (#651): a customer belongs to the organisation
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'dev-customer-' || $1, organization_id, 'dev-customer-' || $1, 'Somchai', 'LIVE', now() \
         FROM branches WHERE id = $1 ON CONFLICT DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's customer");
    // The shift's teller (`shifts.teller_id` is NOT NULL since #753).
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, 'dev-teller', 'INVITED', 'test', 'dev-teller' FROM organizations \
         WHERE code = 'XC' ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("the saved row's teller");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ('dev-shift-' || $1, $1, 'OPEN', now(), 'owner', 'dev-open-' || $1, \
           (SELECT id FROM users WHERE legacy_employee_id = 'dev-teller')) \
         ON CONFLICT DO NOTHING",
    )
    .bind(branch_id)
    .execute(pool)
    .await
    .expect("the saved row's shift");
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, \
            payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, \
            unit_size, principal, fee, bank_fee, received, paid, method, payment_ref, account, \
            staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by) \
         VALUES ($1, $2, 'LIVE', 'dev-shift-' || $2, $3, 'hash', 'dev-customer-' || $2, \
            'Somchai', 'THA', 'BUY', $4, $5::numeric, $6::numeric, 1, \
            round($5::numeric * $6::numeric, 2), 0, 0, 0, round($5::numeric * $6::numeric, 2), \
            'Cash', '', 'Cash drawer', 'owner', 'WALK-IN', '', '2026-10-06', now(), now(), \
            'owner') \
         ON CONFLICT (document_ref) DO NOTHING",
    )
    .bind(document_ref)
    .bind(branch_id)
    .bind(submission_id)
    .bind(currency)
    .bind(amount)
    .bind(rate)
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("the saved row {document_ref}: {e}"));
}

/// `after_save_in` on a branch transaction of its own, as the save runs it.
async fn after_save(db: &Devices, service: &DeviceService, branch_id: &str, s: SavedExchange) {
    // #759 (RE-4e, R9): the save wrote its row before its device step.
    the_saved_row(
        &db.pool,
        branch_id,
        &s.document_ref,
        &s.submission_id,
        &s.currency,
        &s.amount.to_string(),
        &s.rate.to_string(),
    )
    .await;
    let b = branch(branch_id);
    let inner = b.clone();
    let service = service.clone();
    within(
        MUST_FINISH,
        "after_save_in",
        db.branch_tx().run(&b, move |tx| {
            Box::pin(async move { service.after_save_in(tx, &inner, &s, ACTOR, at(0)).await })
        }),
    )
    .await
    .expect("the save's device lines are written");
}

fn cash_save(reference: &str) -> DrawerJobKey {
    DrawerJobKey::CashSave {
        reference: reference.to_owned(),
    }
}

fn passport(source: PassportSource, order: &str) -> DrawerJobKey {
    DrawerJobKey::Passport {
        source,
        order_id: order.to_owned(),
    }
}

async fn jobs_of(db: &Devices, kind: &str) -> Vec<Value> {
    db.jobs()
        .await
        .into_values()
        .filter(|j| j["kind"] == kind)
        .collect()
}

async fn events_of(db: &Devices, kind: &str) -> Vec<Value> {
    db.events()
        .await
        .into_iter()
        .filter(|e| e["kind"] == kind)
        .collect()
}

/// A driver that counts its calls and answers as the simulated one.
#[derive(Default)]
struct CountingDriver {
    calls: AtomicU64,
}

#[async_trait]
impl DeviceDriver for CountingDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    async fn run(&self, kind: JobKind, printer: &PrinterSettings) -> JobStatus {
        self.calls.fetch_add(1, Ordering::SeqCst);
        SimulatedDriver.run(kind, printer).await
    }
}

// ── the cash save (B2 once the shop has an agent) ───────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_cash_save_in_a_branch_with_an_agent_queues_its_drawer_job() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    db.ids.give(&[job_id(1), job_id(2)]);
    after_save(&db, &db.service, MAIN, saved(REF1, true)).await;

    let receipts = jobs_of(&db, "PRINT_RECEIPT").await;
    assert_eq!(receipts.len(), 1, "the receipt job as today");
    assert_eq!(receipts[0]["request_id"], format!("auto:{MAIN}:{REF1}"));

    let drawers = jobs_of(&db, "DRAWER_REQUEST").await;
    assert_eq!(drawers.len(), 1, "one drawer job: {drawers:?}");
    let d = &drawers[0];
    assert_eq!(
        d["request_id"],
        format!("auto:{MAIN}:drawer:{REF1}"),
        "the one id scheme"
    );
    assert_eq!(
        (
            d["ref"].as_str(),
            d["status"].as_str(),
            d["actor"].as_str(),
            d["branch_id"].as_str()
        ),
        (Some(REF1), Some("QUEUED"), Some(ACTOR), Some(MAIN))
    );
    assert_eq!(
        d["created"],
        cut(at(0)),
        "queued at the save's instant, cut to ms"
    );

    let lines = events_of(&db, "DRAWER_REQUEST").await;
    assert_eq!(lines.len(), 1, "one queue line: {lines:?}");
    let line = &lines[0];
    assert_eq!(line["ref"], REF1);
    assert_eq!(line["actor"], ACTOR);
    assert_eq!(line["detail"]["job"], d["id"]);
    assert_eq!(
        line["detail"]["reason"], "CASH_EXCHANGE",
        "the fixed reason"
    );
    assert_eq!(line["detail"]["physicalAction"], false);
    db.finish().await;
}

/// No active agent (none, or a revoked one): a cash save writes exactly what it writes today.
#[tokio::test(flavor = "multi_thread")]
async fn a_cash_save_without_an_active_agent_writes_what_it_writes_today() {
    let db = Devices::start().await;
    agent(&db, B9, AGENT_B9).await;
    db.ids.give(&[job_id(1), job_id(2)]);
    after_save(&db, &db.service, MAIN, saved(REF1, true)).await;
    let kinds: Vec<Value> = db
        .events()
        .await
        .iter()
        .map(|e| e["kind"].clone())
        .collect();
    assert_eq!(
        kinds,
        ["COUNT_CONFIRMED", "EXCHANGE_LINKED", "PRINT_RECEIPT"],
        "another branch's agent is not this branch's"
    );
    assert_eq!(db.jobs().await.len(), 1);

    agent(&db, MAIN, AGENT_MAIN).await;
    revoke(&db, AGENT_MAIN).await;
    db.ids.give(&[job_id(3), job_id(4)]);
    after_save(&db, &db.service, MAIN, saved(REF2, true)).await;
    assert!(
        jobs_of(&db, "DRAWER_REQUEST").await.is_empty(),
        "a revoked agent is not active"
    );
    assert!(events_of(&db, "DRAWER_REQUEST").await.is_empty());
    assert!(events_of(&db, "DRAWER_SKIPPED").await.is_empty());
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_that_is_not_cash_or_is_repeated_queues_no_drawer_job() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    db.ids.give(&[job_id(1), job_id(2)]);
    after_save(&db, &db.service, MAIN, saved(REF1, false)).await;
    assert!(
        jobs_of(&db, "DRAWER_REQUEST").await.is_empty(),
        "not cash: no drawer"
    );

    db.ids.give(&[job_id(3), job_id(4)]);
    after_save(&db, &db.service, MAIN, saved(REF2, true)).await;
    db.ids.give(&[job_id(5), job_id(6)]);
    after_save(&db, &db.service, MAIN, saved(REF2, true)).await;
    assert_eq!(
        jobs_of(&db, "DRAWER_REQUEST").await.len(),
        1,
        "the repeat of a save opens no second drawer"
    );
    assert_eq!(events_of(&db, "DRAWER_REQUEST").await.len(), 1);
    db.finish().await;
}

// ── nothing to drive: hasDrawer = 0, or a model with no pulse ───────────────

/// `hasDrawer = 0` in a branch with an agent: `NoDrawer`, one `DRAWER_SKIPPED` line, nothing
/// queued — and never an error, so the save is not undone.
#[tokio::test(flavor = "multi_thread")]
async fn has_drawer_0_skips_with_one_line_and_the_save_stands() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    setting(&db, MAIN, "hasDrawer", "0").await;
    db.ids.give(&[job_id(1), job_id(2)]);
    after_save(&db, &db.service, MAIN, saved(REF1, true)).await;
    assert!(jobs_of(&db, "DRAWER_REQUEST").await.is_empty());
    let skipped = events_of(&db, "DRAWER_SKIPPED").await;
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(
        (skipped[0]["ref"].as_str(), skipped[0]["actor"].as_str()),
        (Some(REF1), Some(ACTOR))
    );
    assert_eq!(
        jobs_of(&db, "PRINT_RECEIPT").await.len(),
        1,
        "the save stands"
    );

    let outcome = db
        .service
        .queue_drawer(&branch(MAIN), cash_save(REF2), ACTOR, at(1))
        .await
        .expect("never an error");
    assert_eq!(outcome, DrawerJobOutcome::NoDrawer);
    db.finish().await;
}

/// Follow-up 1 of the review of PR 324: a branch whose model has no pulse — a model listed
/// with `drawerPulse: null`, or an id the registry does not know — gets no server-made drawer
/// job, exactly as `hasDrawer = 0` (§5.4c *The bytes*). A job that reaches the agent anyway (a
/// client `queue`, or one queued before the model changed) is failed `NO_PULSE` at claim:
/// row AG-2's.
#[tokio::test(flavor = "multi_thread")]
async fn a_model_without_a_pulse_skips_like_has_drawer_0() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    let models = PrinterModels::new(vec![
        PrinterModel {
            id: "NEOCAL_NTP_8220BT".to_owned(),
            name: "NEOCAL NTP-8220BT".to_owned(),
            paper_mm: 80,
            drawer_pulse: Some(DrawerPulse {
                pin: 0,
                on: 25,
                off: 250,
            }),
        },
        PrinterModel {
            id: "PLAIN_58".to_owned(),
            name: "No drawer port".to_owned(),
            paper_mm: 58,
            drawer_pulse: None,
        },
    ])
    .expect("a valid registry");
    let service = db.service.clone().with_printer_models(Arc::new(models));

    for model in ["PLAIN_58", "NOT_IN_THE_REGISTRY"] {
        setting(&db, MAIN, "printerModel", model).await;
        let outcome = service
            .queue_drawer(&branch(MAIN), cash_save(REF1), ACTOR, at(0))
            .await
            .expect("never an error");
        assert_eq!(outcome, DrawerJobOutcome::NoDrawer, "model {model}");
    }
    assert!(jobs_of(&db, "DRAWER_REQUEST").await.is_empty());
    assert_eq!(events_of(&db, "DRAWER_SKIPPED").await.len(), 2);

    setting(&db, MAIN, "printerModel", "NEOCAL_NTP_8220BT").await;
    db.ids.give_one(&job_id(7));
    let outcome = service
        .queue_drawer(&branch(MAIN), cash_save(REF1), ACTOR, at(1))
        .await
        .expect("queue");
    assert!(
        matches!(&outcome, DrawerJobOutcome::Queued(job) if job.id == job_id(7)),
        "a model with a pulse queues: {outcome:?}"
    );
    db.finish().await;
}

// ── queue_drawer: the id scheme, Existing, NoAgent, the branch ──────────────

#[tokio::test(flavor = "multi_thread")]
async fn queue_drawer_follows_the_one_id_scheme_and_answers_existing() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    let main = branch(MAIN);

    db.ids.give_one(&job_id(1));
    let kiosk = match db
        .service
        .queue_drawer(
            &main,
            passport(PassportSource::Kiosk, "O-1"),
            "kiosk-user",
            at(0),
        )
        .await
        .expect("queue")
    {
        DrawerJobOutcome::Queued(job) => job,
        other => panic!("queued: {other:?}"),
    };
    assert_eq!(kiosk.request_id, format!("auto:{MAIN}:passport:KIOSK:O-1"));
    assert_eq!(
        (
            kiosk.kind,
            kiosk.status,
            kiosk.reference.as_str(),
            kiosk.actor.as_str()
        ),
        (JobKind::DrawerRequest, JobStatus::Queued, "", "kiosk-user")
    );

    db.ids.give_one(&job_id(2));
    let desk = db
        .service
        .queue_drawer(
            &main,
            passport(PassportSource::Desk, "O-1"),
            "front2",
            at(1),
        )
        .await
        .expect("queue");
    assert!(
        matches!(&desk, DrawerJobOutcome::Queued(job)
            if job.request_id == format!("auto:{MAIN}:passport:DESK:O-1")),
        "the desk's is another job: {desk:?}"
    );

    let lines_before = db.events().await.len();
    db.ids.give_one(&job_id(3));
    let again = db
        .service
        .queue_drawer(
            &main,
            passport(PassportSource::Kiosk, "O-1"),
            "front2",
            at(2),
        )
        .await
        .expect("queue");
    assert_eq!(
        again,
        DrawerJobOutcome::Existing(kiosk.clone()),
        "the same key again answers the first job, with its own actor"
    );
    assert_eq!(db.events().await.len(), lines_before, "and writes nothing");
    assert_eq!(jobs_of(&db, "DRAWER_REQUEST").await.len(), 2);

    let lines = events_of(&db, "DRAWER_REQUEST").await;
    assert_eq!(lines[0]["detail"]["reason"], "PASSPORT_STEP");
    db.finish().await;
}

/// No active agent: `NoAgent`, nothing queued and no line — even with `hasDrawer = 0` (a
/// branch without an agent writes exactly what it writes today, so `NoAgent` is decided first).
#[tokio::test(flavor = "multi_thread")]
async fn no_active_agent_answers_no_agent_and_writes_nothing() {
    let db = Devices::start().await;
    let main = branch(MAIN);
    let outcome = db
        .service
        .queue_drawer(&main, cash_save(REF1), ACTOR, at(0))
        .await
        .expect("never an error");
    assert_eq!(outcome, DrawerJobOutcome::NoAgent);
    setting(&db, MAIN, "hasDrawer", "0").await;
    let outcome = db
        .service
        .queue_drawer(&main, passport(PassportSource::Desk, "O-9"), ACTOR, at(1))
        .await
        .expect("never an error");
    assert_eq!(outcome, DrawerJobOutcome::NoAgent);
    assert!(db.jobs().await.is_empty());
    assert!(db.events().await.is_empty());
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_badly_formed_branch_is_refused_with_today_s_sentence() {
    let db = Devices::start().await;
    let answer = db
        .service
        .queue_drawer(&branch("no such!"), cash_save(REF1), ACTOR, at(0))
        .await;
    assert_eq!(refusal(answer), BAD_BRANCH);
    db.finish().await;
}

// ── process leaves drawer jobs to the agent ─────────────────────────────────

/// A drawer job of a branch with an active agent is the agent's: `process` answers it as it
/// stands, calls no driver and writes nothing. Receipts are processed as today.
#[tokio::test(flavor = "multi_thread")]
async fn process_leaves_a_drawer_job_of_a_branch_with_an_agent_to_the_agent() {
    let db = Devices::start().await;
    let driver = Arc::new(CountingDriver::default());
    let service = db.service.clone().with_driver(driver.clone());
    let drawer = db.queued(MAIN, "DRAWER_REQUEST", "", "ทอนเงิน", 1).await;
    let receipt = db.queued_print(2).await;
    agent(&db, MAIN, AGENT_MAIN).await;
    let lines_before = db.events().await.len();

    let job = service
        .process(&branch(MAIN), &drawer, ACTOR, at(1))
        .await
        .expect("process answers");
    assert_eq!(
        (job.id.as_str(), job.status),
        (drawer.as_str(), JobStatus::Queued)
    );
    assert_eq!(
        driver.calls.load(Ordering::SeqCst),
        0,
        "no driver is called"
    );
    assert_eq!(db.events().await.len(), lines_before, "nothing is written");
    assert_eq!(db.jobs().await[&drawer]["status"], "QUEUED");

    let job = service
        .process(&branch(MAIN), &receipt, ACTOR, at(2))
        .await
        .expect("process a receipt");
    assert_eq!(job.status, JobStatus::SimulatedPrinted, "receipts as today");
    assert_eq!(driver.calls.load(Ordering::SeqCst), 1);
    db.finish().await;
}

/// Without an active agent (a revoked one does not count), `process` drives a drawer job as
/// today.
#[tokio::test(flavor = "multi_thread")]
async fn process_drives_a_drawer_job_of_a_branch_without_an_active_agent() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    revoke(&db, AGENT_MAIN).await;
    let drawer = db.queued(MAIN, "DRAWER_REQUEST", "", "ทอนเงิน", 1).await;
    let job = db
        .service
        .process(&branch(MAIN), &drawer, ACTOR, at(1))
        .await
        .expect("process");
    assert_eq!(job.status, JobStatus::CommandSimulated);
    db.finish().await;
}

/// `DeviceService::new` keeps its signature and asks the agent table on the transaction it
/// already holds: on a pool of one connection, the save's lines and `queue_drawer` still finish.
#[tokio::test(flavor = "multi_thread")]
async fn the_agent_is_read_on_the_transaction_already_held() {
    let db = Devices::start().await;
    agent(&db, MAIN, AGENT_MAIN).await;
    let (service, pool) = db.on_one_connection().await;
    db.ids.give(&[job_id(1), job_id(2)]);
    let b = branch(MAIN);
    let inner = b.clone();
    let s = saved(REF1, true);
    // #759 (RE-4e, R9): the save wrote its row before its device step.
    the_saved_row(
        &db.pool,
        MAIN,
        &s.document_ref,
        &s.submission_id,
        &s.currency,
        &s.amount.to_string(),
        &s.rate.to_string(),
    )
    .await;
    let svc = service.clone();
    within(
        MUST_FINISH,
        "after_save_in on one connection",
        xc_kernel::branch_tx::BranchTx::new(pos_infra::transactions::Transactor::new(pool.clone()))
            .run(&b, move |tx| {
                Box::pin(async move { svc.after_save_in(tx, &inner, &s, ACTOR, at(0)).await })
            }),
    )
    .await
    .expect("after_save_in");
    db.ids.give_one(&job_id(3));
    within(
        MUST_FINISH,
        "queue_drawer on one connection",
        service.queue_drawer(&b, passport(PassportSource::Kiosk, "O-2"), ACTOR, at(1)),
    )
    .await
    .expect("queue_drawer");
    pool.close().await;
    assert_eq!(jobs_of(&db, "DRAWER_REQUEST").await.len(), 2);
    db.finish().await;
}
