//! Written before the implementation (ticket T10). Do not edit, delete or weaken.
//!
//! Specification of `CustodyService`: the layer that takes an `ops*` request as
//! the screens send it, loads the branch's custody state and the facts around it,
//! lets `CustodyMachine` (ticket T9) decide, stores the new state and its events,
//! and carries out the shift operations the decision calls for through
//! `ShiftService` (ticket T7) — all in ONE `BranchTx::run`, so that a shift
//! operation that fails undoes the whole command.
//!
//! `tests/fixtures/v2_custody_service_cases.json` holds two days of custody as the
//! previous system ("v2") lived them: every answer, and after every accepted step
//! the custody state, the event log, the shifts and the stock movements.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`). Without it every database test here FAILS; none skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use custody_common::{CustodyState, PriorSave, SaveAttempt, actor};
use custody_service_common::{
    Desk, FakeOutside, Ids, Story, at_millis, bangkok, custody, evidence, fixture, iso, rows,
    totals, written,
};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use shift_service_common::{TestDb, code, money, refusal, tx_delta, within};
use tokio::sync::Barrier;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_shift::domain::custody::entities::tx_draft::DraftStatus;
use xc_shift::domain::custody::value_objects::custody_event::EventKind;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::request_status::{RequestStatus, TransferStage};
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::entities::shift::ShiftStatus;

const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const BAD_KIND: &str = "ประเภทงานไม่ถูกต้อง";
const EVIDENCE_COUNT: &str = "แนบหลักฐาน 1–5 ไฟล์";
const EVIDENCE_NOT_OWNED: &str = "หลักฐานไม่ใช่ของผู้ส่งหรือสาขานี้";
const RECEIVER_INVALID: &str = "ผู้รับหรือบทบาทไม่ถูกต้องสำหรับสาขา";
const BRANCH_NOT_FOUND: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
const USE_CUSTODY: &str = "ใช้ขั้นตอนรับ–ส่งเงินและการลงชื่อในเมนูปฏิบัติงาน";
const REQUEST_PENDING: &str = "มีรายการรับ–ส่งที่ยังไม่เสร็จ กรุณาตรวจรายการเดิมก่อน";
const USD_SHORT: &str = "ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน";
/// The shift's sentence for a text that is too long (ticket #68).
const TEXT_INVALID: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
/// The story whose steps 78–82 v3 answers differently on purpose (ticket #68).
const A_DAY: &str = "a day: set-up, safe, opening, service, transfers";

// ── two days, against v2 ────────────────────────────────────────────────────

/// Replays one recorded story on a branch of its own. After every step the
/// answer and everything the branch holds are compared with v2's.
async fn replay(db: &TestDb, story: &Story) -> (usize, usize) {
    let outside = FakeOutside::new();
    let service = custody(db, &outside);
    let branch = db.branch("story").await;
    let mut ids = Ids::default();
    let mut was = written(db, &service, &branch, &Value::Null, &mut ids).await;
    let (mut accepted, mut refused) = (0, 0);

    for (n, step) in story.steps.iter().enumerate() {
        outside.set_facts(&step.facts);
        if let Some(kind) = &step.outside {
            // rates, customers and unfinished transactions exist only as answers of
            // the ports; a completed transaction also changes what the till holds
            if kind == "tx" {
                db.apply_delta(&branch, tx_delta(step.tx.as_ref().unwrap()))
                    .await
                    .unwrap_or_else(|e| panic!("{} — step {n}: a sale: {e:?}", story.name));
            }
            let mut is = written(db, &service, &branch, &Value::Null, &mut ids).await;
            is.insert("result".into(), was["result"].clone());
            was = is;
            continue;
        }
        let (who, action, payload) = (
            step.actor.as_ref().unwrap(),
            step.action.as_ref().unwrap(),
            step.payload.as_ref().unwrap(),
        );
        let here = format!(
            "{} — step {n}: {who} {action} {payload}{}",
            story.name,
            step.why
                .as_ref()
                .map(|w| format!(" ({w})"))
                .unwrap_or_default()
        );
        // Decided difference from v2 (ticket #68, owner 2026-10-06): v2 accepted at step 78 an
        // EXTERNAL_IN whose note makes the commit's movement reason longer than 200 units, and
        // then refused the commit (step 82), leaving the money in transit. v3 refuses the note at
        // propose with the shift's sentence; steps 79–82 act on the request it never creates.
        if story.name == A_DAY && (79..=82).contains(&n) {
            continue;
        }
        let v2_error = if story.name == A_DAY && n == 78 {
            Some(TEXT_INVALID.to_string())
        } else {
            step.v2_error.clone()
        };
        let now = at_millis(step.now.unwrap());
        let got = service
            .command(&branch, action, &ids.resolve(payload), &actor(who), now)
            .await;

        match (&v2_error, got) {
            (Some(sentence), Err(AppError::Domain(text))) => {
                assert_eq!(&text, sentence, "{here}");
                // a refusal leaves everything as it was
                let mut is = written(db, &service, &branch, &Value::Null, &mut ids).await;
                is.insert("result".into(), was["result"].clone());
                assert_eq!(is, was, "{here}: a refused command changed the branch");
                refused += 1;
            }
            (Some(sentence), other) => {
                panic!("{here}: must be refused with «{sentence}», got {other:?}")
            }
            (None, Err(e)) => panic!("{here}: must be accepted, got {e:?}"),
            (None, Ok(result)) => {
                let is = written(db, &service, &branch, &result, &mut ids).await;
                let changed = step
                    .changed
                    .as_ref()
                    .expect("an accepted step records what changed");
                for (key, value) in changed {
                    assert_eq!(is.get(key), Some(value), "{here}: `{key}` afterwards");
                }
                for (key, value) in &is {
                    if !changed.contains_key(key) {
                        assert_eq!(Some(value), was.get(key), "{here}: `{key}` must not change");
                    }
                }
                for key in was.keys().filter(|k| !is.contains_key(*k)) {
                    // only an element of a list that this step made shorter may go
                    let list = key.split('#').next().unwrap();
                    assert!(
                        key.contains('#') && changed.contains_key(&format!("{list}#len")),
                        "{here}: `{key}` disappeared"
                    );
                }
                was = is;
                accepted += 1;
            }
        }
    }
    (accepted, refused)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_day_of_custody_ends_where_v2_ended() {
    let f = fixture();
    let db = TestDb::start().await;
    let (accepted, refused) = replay(&db, &f.stories[0]).await;
    // 37 accepted in v2; #68 makes step 78 a refusal and skips 79–81 (accepted) and 82 (refused)
    assert!(accepted >= 33 && refused >= 41, "{accepted} / {refused}");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_evening_of_custody_ends_where_v2_ended() {
    let f = fixture();
    let db = TestDb::start().await;
    let (accepted, refused) = replay(&db, &f.stories[1]).await;
    assert!(accepted >= 44 && refused >= 11, "{accepted} / {refused}");
    db.drop_schema().await;
}

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

// ── what a request must look like before the branch is consulted ────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_that_cannot_be_read_is_refused_before_anything_else() {
    let db = TestDb::start().await;
    // custody is not even enabled here: the form of the request is judged first
    let mut d = Desk::new(&db).await;
    for (action, payload, sentence) in [
        ("opsDance", json!({}), BAD_REQUEST),
        ("", json!({}), BAD_REQUEST),
        ("adminOpenShift", json!({}), BAD_REQUEST),
        ("opsClock", json!({ "direction": "UP" }), BAD_REQUEST),
        ("opsClock", json!({}), BAD_REQUEST),
        ("opsPropose", json!({ "kind": "GIFT" }), BAD_KIND),
        ("opsPropose", json!({}), BAD_KIND),
        (
            "opsPropose",
            json!({ "kind": "SAFE_OPEN", "evidenceKeys": "x" }),
            EVIDENCE_COUNT,
        ),
        ("opsPropose", json!({ "kind": "SAFE_OPEN" }), EVIDENCE_COUNT),
        (
            "opsPropose",
            json!({ "kind": "SAFE_OPEN", "evidenceKeys": (1..=6).map(evidence).collect::<Vec<_>>() }),
            EVIDENCE_COUNT,
        ),
    ] {
        let got = d.act("F1", action, payload.clone()).await;
        assert_eq!(refusal(got), sentence, "{action} {payload}");
    }
    assert_eq!(db.version(&d.branch).await, 0, "nothing was written");
    db.drop_schema().await;
}

// ── the ports ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn evidence_must_belong_to_the_proposer_in_this_branch() {
    let db = TestDb::start().await;
    let mut d = Desk::with_safe(&db).await;
    let drawer = [("THB", 100_000), ("USD", 2_000)];
    let p = d.proposal(
        "OPEN",
        json!({ "counted": totals(&drawer), "denominations": rows(&drawer) }),
    );
    let key = p["evidenceKeys"][0].as_str().unwrap().to_owned();

    d.outside.disown_evidence(&key);
    let before = d.custody.state(&d.branch).await.unwrap();
    let got = d.act("F1", "opsPropose", p.clone()).await;
    assert_eq!(refusal(got), EVIDENCE_NOT_OWNED);
    assert_eq!(d.custody.state(&d.branch).await.unwrap(), before);
    // the question was asked about the proposer's own id, in this branch, for exactly these keys
    let asked = d.outside.evidence_questions();
    let last = asked.last().expect("the files port was asked");
    assert_eq!(
        last,
        &(d.branch.as_str().to_owned(), "F1".to_owned(), vec![key])
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_receiver_must_be_an_employee_of_the_branch_in_a_role_that_can_receive() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    d.outside.set_employee("GONE", None); // not an employee, or switched off
    d.outside.set_employee("F9", Some(CustodyRole::Front));

    // a transfer may be received by Front, Manager or Finance; a handover only by Front
    for (kind, receiver, accepted) in [
        ("SAFE_OUT", "GONE", false),
        ("SAFE_OUT", "BE2", false),
        ("SAFE_OUT", "MON1", false),
        ("EXTERNAL_IN", "MGR1", true),
    ] {
        let p = d.proposal(
            kind,
            json!({ "receiverId": receiver, "currency": "USD", "amount": 10 }),
        );
        let got = d.act("F1", "opsPropose", p).await;
        if accepted {
            let id = got.unwrap()["id"].as_str().unwrap().to_owned();
            d.ok(
                "MON1",
                "opsReject",
                json!({ "id": id, "reason": "only a probe" }),
            )
            .await;
        } else {
            assert_eq!(refusal(got), RECEIVER_INVALID, "{kind} to {receiver}");
        }
    }

    d.ok("BE1", "opsCutoff", json!({})).await;
    let drawer = [("THB", 100_000), ("USD", 2_000)];
    let safe = [("THB", 500_000), ("USD", 10_000)];
    for (receiver, accepted) in [
        ("FIN1", false),
        ("MGR1", false),
        ("GONE", false),
        ("F9", true),
    ] {
        let p = d.proposal(
            "HANDOVER",
            json!({
                "receiverId": receiver, "counted": totals(&drawer), "denominations": rows(&drawer),
                "safeCount": totals(&safe), "safeDenominations": rows(&safe),
            }),
        );
        let got = d.act("F1", "opsPropose", p).await;
        if accepted {
            got.expect("a Front of the branch may receive a handover");
        } else {
            assert_eq!(refusal(got), RECEIVER_INVALID, "HANDOVER to {receiver}");
        }
    }
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

    // refusals of every origin: the machine, the form, a port
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_shift_operation_that_fails_undoes_the_whole_command() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;

    // 2,500 dollars cannot leave a drawer that holds 2,000: the machine has no rule
    // against it, the stock movement does — and its refusal is the command's
    let id = d
        .propose(
            "F1",
            "SAFE_IN",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": 2500 }),
        )
        .await;
    let (state, events, v) = (
        d.custody.state(&d.branch).await.unwrap(),
        d.custody.events(&d.branch).await.unwrap(),
        db.version(&d.branch).await,
    );
    let got = d.act("BE1", "opsDispatch", json!({ "id": id })).await;
    assert_eq!(refusal(got), USD_SHORT);
    assert_eq!(
        d.custody.state(&d.branch).await.unwrap(),
        state,
        "still not in transit"
    );
    assert_eq!(
        d.custody.events(&d.branch).await.unwrap(),
        events,
        "no DISPATCH event"
    );
    assert_eq!(db.version(&d.branch).await, v);
    assert_eq!(db.service().movements(&d.branch).await.unwrap(), vec![]);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_shift_operations_carry_the_ids_and_the_names_custody_gives_them() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    let shift = db.service().active(&d.branch).await.unwrap().unwrap();
    // the shift was opened under the OPEN request's own client id
    assert_eq!(shift.open_request_id, custody_service_common::request_id(2));

    // drawer → safe: the money leaves the drawer at dispatch, under the request's client id
    let p = d.proposal(
        "SAFE_IN",
        json!({ "receiverId": "FIN1", "currency": "USD", "amount": "300.25" }),
    );
    let client_id = p["requestId"].as_str().unwrap().to_owned();
    let id = d.ok("F1", "opsPropose", p).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    d.ok("BE1", "opsDispatch", json!({ "id": id })).await;
    let movements = db.service().movements(&d.branch).await.unwrap();
    assert_eq!(movements.len(), 1);
    let m = &movements[0];
    assert_eq!(m.request_id, client_id);
    assert_eq!(m.shift_id, shift.id);
    assert_eq!(m.by, "BE1");
    assert_eq!(m.amount, money("300.25"));
    assert_eq!(m.reason, format!("IN_TRANSIT · SAFE_IN · {id}"));
    assert_eq!(
        m.at,
        d.now(),
        "the movement carries the time of the dispatch"
    );

    // a second dispatch is accepted and moves nothing again
    d.ok("BE1", "opsDispatch", json!({ "id": id })).await;
    assert_eq!(db.service().movements(&d.branch).await.unwrap().len(), 1);
    d.ok("FIN1", "opsSign", json!({ "id": id })).await;
    d.ok("BE1", "opsCommit", json!({ "id": id })).await;
    let state = d.custody.state(&d.branch).await.unwrap();
    assert_eq!(
        state.safe.as_ref().unwrap().get(code("USD")),
        money("10300.25")
    );
    let r = state.requests.last().unwrap();
    assert_eq!(
        (r.status, r.stage),
        (RequestStatus::Completed, Some(TransferStage::Received))
    );

    // safe → drawer: the drawer receives at commit, under a fresh id, named after the note
    let id = d
        .propose(
            "F1",
            "SAFE_OUT",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": 50, "note": "small notes" }),
        )
        .await;
    d.ok("BE1", "opsDispatch", json!({ "id": id })).await;
    d.ok("FIN1", "opsSign", json!({ "id": id })).await;
    d.ok("BE2", "opsCommit", json!({ "id": id })).await;
    let movements = db.service().movements(&d.branch).await.unwrap();
    assert_eq!(movements.len(), 2);
    let m = &movements[1];
    assert_eq!(m.reason, "SAFE_OUT · small notes");
    assert_eq!(m.by, "BE2");
    assert_ne!(m.request_id, movements[0].request_id);
    assert!(
        is_uuid(&m.request_id),
        "a fresh request id: {}",
        m.request_id
    );
    let till = db.service().active(&d.branch).await.unwrap().unwrap();
    assert_eq!(till.expected.get(code("USD")), money("1749.75"));
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
async fn direct_shift_actions_are_refused_while_custody_is_enabled() {
    let db = TestDb::start().await;
    let mut d = Desk::new(&db).await;
    let check = |d: &Desk| {
        let (service, branch) = (d.custody.clone(), d.branch.clone());
        let tx = db.branch_tx();
        async move {
            let b = branch.clone();
            tx.run(&branch, move |tx| {
                Box::pin(async move { service.check_direct_shift_action_in(tx, &b).await })
            })
            .await
        }
    };
    // off: the owner's screen may open, close and move stock itself
    check(&d).await.expect("custody is off");
    d.ok(
        "owner",
        "opsSetEntity",
        json!({ "company": "XC", "site": "B", "confirmSeparated": true }),
    )
    .await;
    check(&d)
        .await
        .expect("an entity alone does not enable custody");
    d.ok("owner", "opsEnable", json!({})).await;
    assert_eq!(refusal(check(&d).await), USE_CUSTODY);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gate_on_a_save_reads_the_stored_state_and_the_real_shift() {
    let db = TestDb::start().await;
    let mut d = Desk::serving(&db).await;
    d.outside.add_customer("CX-00000001");
    let draft = d
        .ok(
            "F1",
            "opsBeginTransaction",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let save = |draft_id: Option<String>| SaveAttempt {
        customer_id: "CX-00000001".into(),
        draft_id,
        side: Side::Buy,
        currency: code("USD"),
        amount: money("100"),
        prior: PriorSave::None,
    };
    let gate = |d: &Desk, who: &'static str, save: SaveAttempt, hhmm: &'static str| {
        let (service, branch) = (d.custody.clone(), d.branch.clone());
        let tx = db.branch_tx();
        async move {
            let b = branch.clone();
            tx.run(&branch, move |tx| {
                Box::pin(async move {
                    service
                        .check_save_in(tx, &b, &save, &actor(who), bangkok(hhmm))
                        .await
                })
            })
            .await
        }
    };

    gate(&d, "F1", save(None), "12:30")
        .await
        .expect("the custodian may save");
    assert_eq!(
        refusal(gate(&d, "F2", save(None), "12:30").await),
        "ต้องเป็น Front ผู้รับผิดชอบลิ้นชักที่ลงเวลาเข้าแล้ว"
    );
    // after 22:50 only a transaction begun before (a draft of this shift) may still be saved:
    // the draft's shift id is compared with the shift that is really open
    assert_eq!(
        refusal(gate(&d, "F1", save(None), "22:55").await),
        "ปิดรับรายการใหม่แล้ว หรือกำลังส่งกะ/ตรวจนับเงิน"
    );
    gate(&d, "F1", save(Some(draft.clone())), "22:55")
        .await
        .expect("the draft was begun in the open shift");

    d.ok(
        "MON1",
        "opsHold",
        json!({ "customerId": "CX-00000001", "reason": "check" }),
    )
    .await;
    assert_eq!(
        refusal(gate(&d, "F1", save(None), "12:30").await),
        "ลูกค้านี้อยู่ระหว่างพักตรวจ ให้ Monitor ยืนยันผลก่อนทำรายการ"
    );

    // a branch where custody is off has no gate
    let other = Desk::new(&db).await;
    gate(&other, "F2", save(None), "03:00")
        .await
        .expect("custody is off");
    db.drop_schema().await;
}

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
                            .complete_draft_in(tx, &b, &draft, "XC-20261006-000001")
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

// ── two at once ─────────────────────────────────────────────────────────────

type Job<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Runs the futures at the same instant and returns their results in order.
async fn together<T: Send + 'static>(jobs: Vec<Job<T>>) -> Vec<T> {
    let barrier = Arc::new(Barrier::new(jobs.len()));
    let handles: Vec<_> = jobs
        .into_iter()
        .map(|job| {
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                job.await
            })
        })
        .collect();
    let mut out = Vec::new();
    for h in handles {
        out.push(
            within("a concurrent command", h)
                .await
                .expect("task panicked"),
        );
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn of_many_proposals_at_once_exactly_one_becomes_the_pending_request() {
    let db = TestDb::start().await;
    for round in 0..3 {
        let mut d = Desk::serving(&db).await;
        let now = d.tick();
        let jobs: Vec<Job<AppResult<Value>>> = (0..6)
            .map(|n| {
                let p = d.proposal(
                    "SAFE_OUT",
                    json!({ "receiverId": "FIN1", "currency": "USD", "amount": n + 1 }),
                );
                let (service, branch) = (d.custody.clone(), d.branch.clone());
                Box::pin(async move {
                    service
                        .command(&branch, "opsPropose", &p, &actor("F1"), now)
                        .await
                }) as Job<_>
            })
            .collect();
        let results = together(jobs).await;
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "round {round}: {results:?}"
        );
        for r in results.iter().filter(|r| r.is_err()) {
            match r {
                Err(AppError::Domain(s)) => assert_eq!(s, REQUEST_PENDING, "round {round}"),
                other => panic!("round {round}: {other:?}"),
            }
        }
        let state = d.custody.state(&d.branch).await.unwrap();
        let pending = state
            .requests
            .iter()
            .filter(|r| r.status == RequestStatus::Pending)
            .count();
        assert_eq!(pending, 1, "round {round}");
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn one_proposal_sent_many_times_at_once_is_stored_once() {
    let db = TestDb::start().await;
    for round in 0..3 {
        let mut d = Desk::serving(&db).await;
        let now = d.tick();
        let p = d.proposal(
            "SAFE_OUT",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": 5 }),
        );
        let before = d.custody.state(&d.branch).await.unwrap().requests.len();
        let jobs: Vec<Job<AppResult<Value>>> = (0..6)
            .map(|_| {
                let (service, branch, p) = (d.custody.clone(), d.branch.clone(), p.clone());
                Box::pin(async move {
                    service
                        .command(&branch, "opsPropose", &p, &actor("F1"), now)
                        .await
                }) as Job<_>
            })
            .collect();
        let ids: BTreeSet<String> = together(jobs)
            .await
            .into_iter()
            .map(|r| {
                r.expect("every copy is answered")["id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            ids.len(),
            1,
            "round {round}: one request, its id returned to all"
        );
        let state = d.custody.state(&d.branch).await.unwrap();
        assert_eq!(state.requests.len(), before + 1, "round {round}");
        let proposes = d
            .custody
            .events(&d.branch)
            .await
            .unwrap()
            .iter()
            .filter(|e| {
                e.kind == EventKind::Propose && e.details["id"] == json!(ids.iter().next().unwrap())
            })
            .count();
        assert_eq!(proposes, 1, "round {round}: one PROPOSE event");
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn one_commit_sent_many_times_at_once_opens_one_shift() {
    let db = TestDb::start().await;
    for round in 0..3 {
        let (mut d, open_id) = Desk::opening_proposed(&db).await;
        let now = d.tick();
        let jobs: Vec<Job<AppResult<Value>>> = (0..6)
            .map(|n| {
                let (service, branch, id) = (d.custody.clone(), d.branch.clone(), open_id.clone());
                // two Backends press the button, three times each
                let who = if n % 2 == 0 { "BE1" } else { "BE2" };
                Box::pin(async move {
                    service
                        .command(&branch, "opsCommit", &json!({ "id": id }), &actor(who), now)
                        .await
                }) as Job<_>
            })
            .collect();
        for r in together(jobs).await {
            assert_eq!(
                r.expect("a commit of a completed request is accepted"),
                json!({ "saved": true }),
                "round {round}"
            );
        }
        let shifts = db.service().shifts(&d.branch).await.unwrap();
        assert_eq!(shifts.len(), 1, "round {round}");
        assert_eq!(shifts[0].status, ShiftStatus::Open);
        let commits = d
            .custody
            .events(&d.branch)
            .await
            .unwrap()
            .iter()
            .filter(|e| e.kind == EventKind::Commit && e.details["id"] == json!(open_id))
            .count();
        assert_eq!(commits, 1, "round {round}: one COMMIT event");
        let state = d.custody.state(&d.branch).await.unwrap();
        let r = state.requests.iter().find(|r| r.id == open_id).unwrap();
        assert_eq!(
            r.signatures.len(),
            2,
            "round {round}: the proposer and ONE Backend"
        );
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_dispatch_and_a_withdrawal_at_once_share_the_same_till() {
    let db = TestDb::start().await;
    for round in 0..4 {
        let mut d = Desk::serving(&db).await; // the drawer holds 2,000 dollars
        let shift = db.service().active(&d.branch).await.unwrap().unwrap();
        let id = d
            .propose(
                "F1",
                "SAFE_IN",
                json!({ "receiverId": "FIN1", "currency": "USD", "amount": 1500 }),
            )
            .await;
        let now = d.tick();
        let (service, branch) = (d.custody.clone(), d.branch.clone());
        let (shifts, branch2, shift_id) = (db.service(), d.branch.clone(), shift.id.clone());
        let jobs: Vec<Job<AppResult<()>>> = vec![
            Box::pin(async move {
                service
                    .command(
                        &branch,
                        "opsDispatch",
                        &json!({ "id": id }),
                        &actor("BE1"),
                        now,
                    )
                    .await
                    .map(|_| ())
            }),
            Box::pin(async move {
                let cmd = MoveStock::from_payload(&json!({
                    "requestId": "req-race-0001-000000000000", "shiftId": shift_id, "currency": "USD",
                    "direction": "OUT", "amount": 1500, "reason": "race",
                }))
                .unwrap();
                shifts
                    .move_stock(&branch2, cmd, "racer", now)
                    .await
                    .map(|_| ())
            }),
        ];
        let results = together(jobs).await;
        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(
            ok, 1,
            "round {round}: both took 1,500 from 2,000: {results:?}"
        );
        for r in results.iter().filter(|r| r.is_err()) {
            match r {
                Err(AppError::Domain(s)) => assert_eq!(s, USD_SHORT, "round {round}"),
                other => panic!("round {round}: {other:?}"),
            }
        }
        let till = db.service().active(&d.branch).await.unwrap().unwrap();
        assert_eq!(
            till.expected.get(code("USD")),
            money("500"),
            "round {round}"
        );
        assert_eq!(
            db.service().movements(&d.branch).await.unwrap().len(),
            1,
            "round {round}"
        );
    }
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
    let signatures: usize = state.requests.iter().map(|r| r.signatures.len()).sum();

    for (table, expected) in [
        ("ops_state", 1usize),
        ("attendance", state.attendance.len()),
        ("custody_requests", state.requests.len()),
        ("custody_signatures", signatures),
        ("ops_events", events.len()),
        ("customer_holds", state.holds.len()),
        ("tx_drafts", state.drafts.len()),
        ("cash_exceptions", state.exceptions.len()),
    ] {
        let sql = match table {
            "ops_state" => "SELECT count(*) FROM ops_state WHERE branch_id = $1",
            "attendance" => "SELECT count(*) FROM attendance WHERE branch_id = $1",
            "custody_requests" => "SELECT count(*) FROM custody_requests WHERE branch_id = $1",
            "custody_signatures" => {
                "SELECT count(*) FROM custody_signatures s JOIN custody_requests r ON r.id = s.request_id \
                 WHERE r.branch_id = $1"
            }
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
    assert!(state.requests.len() >= 2 && signatures >= 4 && events.len() >= 15);

    // the database itself refuses a second pending request in a branch
    let p = d.proposal(
        "SAFE_OUT",
        json!({ "receiverId": "FIN1", "currency": "USD", "amount": 1 }),
    );
    d.ok("F1", "opsPropose", p).await;
    let forced = sqlx::query("UPDATE custody_requests SET status = 'PENDING' WHERE branch_id = $1")
        .bind(d.branch.as_str())
        .execute(&db.pool)
        .await;
    assert!(
        matches!(forced, Err(sqlx::Error::Database(_))),
        "several PENDING requests in one branch were stored: {forced:?}"
    );
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname = current_schema() AND tablename = 'custody_requests'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(
        indexes.iter().any(|i| {
            let i = i.to_uppercase();
            i.starts_with("CREATE UNIQUE INDEX")
                && i.contains("BRANCH_ID")
                && i.contains("WHERE")
                && i.contains("'PENDING'")
        }),
        "no partial unique index on custody_requests(branch_id) WHERE status = 'PENDING': {indexes:?}"
    );
    db.drop_schema().await;
}
