//! Written before the implementation (ticket T10). Do not edit, delete or weaken.
//!
//! Specification of `CustodyService`: the layer that takes an `ops*` request as
//! the screens send it, loads the branch's custody state and the facts around it,
//! lets `CustodyMachine` (ticket T9) decide, stores the new state and its events,
//! and carries out the shift operations the decision calls for through
//! `ShiftService` (ticket T7) — all in ONE `BranchTx::run`, so that a shift
//! operation that fails undoes the whole command.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`). Without it every database test here FAILS; none skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use std::collections::BTreeSet;

use custody_common::{CustodyState, PriorSave, SaveAttempt, actor};
use custody_service_common::{Desk, FakeOutside, at_millis, bangkok, custody, iso};
use pos_error::{AppError, AppResult};
use serde_json::json;
use shift_service_common::{TestDb, code, money, refusal};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_shift::domain::custody::entities::tx_draft::DraftStatus;
use xc_shift::domain::custody::value_objects::custody_event::EventKind;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::phase::Phase;

const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const RECEIVER_INVALID: &str = "ผู้รับหรือบทบาทไม่ถูกต้องสำหรับสาขา";
const BRANCH_NOT_FOUND: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";

// ── a branch that was never used, and one that does not exist ───────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_never_used_reads_as_the_initial_state() {
    let db = TestDb::start().await;
    let outside = FakeOutside::new();
    let service = custody(&db, &outside);
    let branch = db.branch("fresh").await;
    assert_eq!(
        service.state(&branch).await.unwrap(),
        CustodyState::initial()
    );
    assert_eq!(service.events(&branch).await.unwrap(), vec![]);
    // reading creates nothing: the branch has no lock row yet
    assert_eq!(db.version(&branch).await, 0);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_that_does_not_exist_is_refused_and_leaves_no_trace() {
    let db = TestDb::start().await;
    let outside = FakeOutside::new();
    let service = custody(&db, &outside);
    let ghost = BranchId::new(shift_service_common::unique("ghost"));
    let got = service
        .command(
            &ghost,
            "opsSetEntity",
            &json!({ "company": "XC", "site": "B", "confirmSeparated": true }),
            &actor("owner"),
            bangkok("12:00"),
        )
        .await;
    assert_eq!(refusal(got), BRANCH_NOT_FOUND);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM branch_versions WHERE branch_id = $1")
        .bind(ghost.as_str())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    db.drop_schema().await;
}

// ── one transaction: the decision, its records and the shift operations ─────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_accepted_command_is_one_version_step_and_a_refused_one_is_none() {
    let db = TestDb::start().await;
    let (mut d, open_id) = Desk::opening_proposed(&db).await;
    let v = db.version(&d.branch).await;

    // the commit opens the shift: custody records and the shift land together
    d.ok("BE1", "opsCommit", json!({ "id": open_id })).await;
    assert_eq!(db.version(&d.branch).await, v + 1);
    let shift = db
        .service()
        .active(&d.branch)
        .await
        .unwrap()
        .expect("the shift is open");
    assert_eq!(shift.opened_by, "BE1");
    assert_eq!(shift.opening.get(code("USD")), money("2000"));
    let state = d.custody.state(&d.branch).await.unwrap();
    assert_eq!(state.phase, Phase::Ready);
    assert_eq!(state.custodian.as_deref(), Some("F1"));

    // refusals of every origin: the machine, the form, a port (including the decided difference
    // from v2 on ticket #68, owner 2026-10-06, where a refused opsPropose leaves nothing in
    // transit and advances no version)
    let got = d.act("F1", "opsCommit", json!({ "id": open_id })).await;
    assert_eq!(refusal(got), "บทบาทนี้ไม่มีสิทธิ์ทำรายการ");
    let got = d
        .act("F1", "opsClock", json!({ "direction": "SIDEWAYS" }))
        .await;
    assert_eq!(refusal(got), BAD_REQUEST);
    d.outside.set_employee("GONE", None);
    let p = d.proposal(
        "SAFE_OUT",
        json!({ "receiverId": "GONE", "currency": "USD", "amount": 1 }),
    );
    assert_eq!(
        refusal(d.act("F1", "opsPropose", p).await),
        RECEIVER_INVALID
    );
    assert_eq!(db.version(&d.branch).await, v + 1);
    assert_eq!(d.custody.state(&d.branch).await.unwrap(), state);
    db.drop_schema().await;
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_records_get_ids_of_this_server_and_events_say_who_when_and_what() {
    let db = TestDb::start().await;
    let mut d = Desk::new(&db).await;
    // a clock with micro- and nanoseconds
    let t0 = d.tick() + chrono::Duration::microseconds(987) + chrono::Duration::nanoseconds(321);
    d.custody
        .command(
            &d.branch,
            "opsSetEntity",
            &json!({ "company": " XC ", "site": " B ", "confirmSeparated": true }),
            &actor("owner"),
            t0,
        )
        .await
        .unwrap();
    d.ok("owner", "opsEnable", json!({})).await;
    d.ok("F1", "opsClock", json!({ "direction": "IN" })).await;

    let events = d.custody.events(&d.branch).await.unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::SetEntity, EventKind::Enable, EventKind::ClockIn]
    );
    let ids: BTreeSet<&str> = events.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "every event has an id of its own");
    assert!(events.iter().all(|e| is_uuid(&e.id)));
    // the owner has no employee id: the event names the session id
    assert_eq!(
        (events[0].actor.as_str(), events[0].role),
        ("owner-1", CustodyRole::Owner)
    );
    assert_eq!(
        (events[2].actor.as_str(), events[2].role),
        ("F1", CustodyRole::Front)
    );
    // times are cut to whole milliseconds, in the state and in the event alike
    let cut = at_millis(t0.timestamp_millis());
    assert_ne!(cut, t0);
    assert_eq!(events[0].at, cut);
    assert_eq!(
        events[0].details,
        json!({ "company": "XC", "site": "B", "confirmedBy": "owner-1", "at": iso(cut) })
    );
    assert_eq!(events[1].details, json!({}));
    let state = d.custody.state(&d.branch).await.unwrap();
    assert_eq!(
        state.entity.as_ref().unwrap().at.epoch_millis(),
        cut.timestamp_millis()
    );
    assert!(
        is_uuid(&state.attendance[0].id),
        "{}",
        state.attendance[0].id
    );
    db.drop_schema().await;
}

// ── what the other tickets call ─────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_draft_is_completed_inside_the_callers_transaction_or_not_at_all() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    let draft = d
        .ok(
            "F1",
            "opsBeginTransaction",
            json!({ "type": "SELL", "currency": "USD", "amount": "25.50" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(is_uuid(&draft));
    let before = d.custody.state(&d.branch).await.unwrap();
    assert_eq!(before.drafts.len(), 1);
    assert_eq!(before.drafts[0].id, draft);
    assert_eq!(before.drafts[0].status, DraftStatus::Open);
    let events = d.custody.events(&d.branch).await.unwrap().len();

    let complete = |fail: bool| {
        let (service, branch, draft) = (d.custody.clone(), d.branch.clone(), draft.clone());
        let tx = db.branch_tx();
        async move {
            let b = branch.clone();
            let r: AppResult<()> = tx
                .run(&branch, move |tx| {
                    Box::pin(async move {
                        service
                            .complete_draft_in(
                                tx,
                                &b,
                                &SaveAttempt {
                                    customer_id: "CX-00000001".into(),
                                    draft_id: Some(draft.clone()),
                                    side: Side::Sell,
                                    currency: code("USD"),
                                    amount: money("25.50"),
                                    prior: PriorSave::None,
                                },
                                "XC-20261006-000001",
                            )
                            .await?;
                        if fail {
                            return Err(AppError::Internal("the save failed afterwards".into()));
                        }
                        Ok(())
                    })
                })
                .await;
            r
        }
    };
    assert!(complete(true).await.is_err());
    assert_eq!(
        d.custody.state(&d.branch).await.unwrap(),
        before,
        "undone with the save"
    );

    complete(false).await.unwrap();
    let after = d.custody.state(&d.branch).await.unwrap();
    assert_eq!(after.drafts[0].status, DraftStatus::Completed);
    assert_eq!(
        after.drafts[0].document_ref.as_deref(),
        Some("XC-20261006-000001")
    );
    assert_eq!(after.drafts[0].amount, money("25.50"));
    // nothing else moved, and completing a draft writes no event
    let mut same = after.clone();
    same.drafts = before.drafts.clone();
    assert_eq!(same, before);
    assert_eq!(d.custody.events(&d.branch).await.unwrap().len(), events);
    db.drop_schema().await;
}

// ── storage ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_state_is_kept_as_rows_of_the_designs_tables() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    d.outside.add_customer("CX-00000001");
    d.ok(
        "F1",
        "opsHold",
        json!({ "customerId": "CX-00000001", "reason": "check" }),
    )
    .await;
    d.ok(
        "F1",
        "opsBeginTransaction",
        json!({ "type": "BUY", "currency": "USD", "amount": 1 }),
    )
    .await;
    let state = d.custody.state(&d.branch).await.unwrap();
    let events = d.custody.events(&d.branch).await.unwrap();

    for (table, expected) in [
        ("ops_state", 1usize),
        ("attendance", state.attendance.len()),
        ("ops_events", events.len()),
        ("customer_holds", state.holds.len()),
        ("tx_drafts", state.drafts.len()),
        ("cash_exceptions", state.exceptions.len()),
    ] {
        let sql = match table {
            "ops_state" => "SELECT count(*) FROM ops_state WHERE branch_id = $1",
            "attendance" => "SELECT count(*) FROM attendance WHERE branch_id = $1",
            "ops_events" => "SELECT count(*) FROM ops_events WHERE branch_id = $1",
            "customer_holds" => "SELECT count(*) FROM customer_holds WHERE branch_id = $1",
            "tx_drafts" => "SELECT count(*) FROM tx_drafts WHERE branch_id = $1",
            _ => "SELECT count(*) FROM cash_exceptions WHERE branch_id = $1",
        };
        let rows: i64 = sqlx::query_scalar(sql)
            .bind(d.branch.as_str())
            .fetch_one(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("{table}: {e}"));
        assert_eq!(rows as usize, expected, "rows in {table}");
    }
    assert!(events.len() >= 15);
    db.drop_schema().await;
}
