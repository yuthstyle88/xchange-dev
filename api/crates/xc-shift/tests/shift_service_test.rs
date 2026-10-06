//! Written before the implementation (ticket T7). Do not edit, delete or weaken.
//!
//! Specification of the shift use cases of `xc-shift`: opening a shift, moving
//! stock in and out, closing against a count, and the entry point a saved
//! transaction uses to change what the open shift should hold.
//!
//! `tests/fixtures/v2_shift_cases.json` holds what the previous system ("v2")
//! answered, step by step, for whole shifts; the service must give the same
//! sentence for every refusal and leave the branch holding the same figures.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it every database test here FAILS; none skips.

mod shift_service_common;

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use shift_service_common::{
    Scenario, Service, TestDb, at, code, fixture, money, outcome, refusal, resolved,
    shown_balances, shown_delta, shown_movement, shown_shift, tx_delta, within,
};
use tokio::sync::Barrier;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::domain::shift::commands::close_shift::CloseShift;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};
use xc_shift::domain::shift::entities::stock_movement::StockMovement;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

const ALREADY_OPEN: &str = "มีกะเปิดอยู่แล้ว กรุณารีเฟรช";
const SHIFT_CHANGED: &str = "กะเปลี่ยนหรือปิดแล้ว กรุณารีเฟรช";
const NO_OPEN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
const BRANCH_NOT_FOUND: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
const ABANDONED: &str = "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด";
const USD_SHORT: &str = "ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน";

fn open_cmd(request_id: &str, opening: Value) -> OpenShift {
    OpenShift::from_payload(&json!({ "requestId": request_id, "opening": opening }))
        .expect("a well-formed open payload")
}

fn move_cmd(
    request_id: &str,
    shift_id: &str,
    currency: &str,
    direction: &str,
    amount: Value,
) -> MoveStock {
    MoveStock::from_payload(&json!({
        "requestId": request_id, "shiftId": shift_id, "currency": currency,
        "direction": direction, "amount": amount, "reason": "test",
    }))
    .expect("a well-formed movement payload")
}

fn close_cmd(
    request_id: &str,
    shift_id: &str,
    counted: Value,
    expected: Value,
    note: &str,
) -> CloseShift {
    CloseShift::from_payload(&json!({
        "requestId": request_id, "shiftId": shift_id, "counted": counted,
        "expected": expected, "note": note,
    }))
    .expect("a well-formed close payload")
}

fn rid(label: &str, n: usize) -> String {
    format!("req-{label}-{n:04}-000000000000")
}

async fn open_with(db: &TestDb, branch: &BranchId, opening: Value) -> Shift {
    db.service()
        .open(
            branch,
            open_cmd(&rid("setup", 0), opening),
            "opener",
            at(0, 0),
        )
        .await
        .expect("open the shift")
}

async fn expected_of(service: &Service, branch: &BranchId, currency: &str) -> String {
    let shift = service
        .active(branch)
        .await
        .expect("read the open shift")
        .expect("a shift is open");
    shown_balances(&shift.expected)
        .get(currency)
        .cloned()
        .unwrap_or_else(|| "0.00".to_owned())
}

// ── the sentences ───────────────────────────────────────────────────────────

#[test]
fn every_refusal_reads_exactly_as_v2_wrote_it() {
    let table: Vec<(ShiftError, &str)> = vec![
        (ShiftError::TextInvalid, "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด"),
        (ShiftError::RequestIdInvalid, "รหัสคำขอไม่ถูกต้อง"),
        (ShiftError::AmountMissing, "กรุณากรอกจำนวนเงิน"),
        (
            ShiftError::AmountInvalid,
            "จำนวนเงินต้องไม่ติดลบ ไม่เกิน 100,000,000 และมีทศนิยมไม่เกิน 2 ตำแหน่ง",
        ),
        (ShiftError::MovementInvalid, "รายการรับ–จ่ายไม่ถูกต้อง"),
        (ShiftError::BranchNotFound, BRANCH_NOT_FOUND),
        (ShiftError::RequestAbandoned, ABANDONED),
        (ShiftError::RequestChanged, "คำขอเดิมมีข้อมูลเปลี่ยน"),
        (ShiftError::ShiftAlreadyOpen, ALREADY_OPEN),
        (ShiftError::ShiftChanged, SHIFT_CHANGED),
        (ShiftError::NoOpenShift, NO_OPEN_SHIFT),
        (ShiftError::Insufficient(code("USD")), USD_SHORT),
        (
            ShiftError::Insufficient(CurrencyCode::THB),
            "ยอด THB ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน",
        ),
        (
            ShiftError::ExpectedChanged,
            "ยอดเปลี่ยนระหว่างตรวจนับ กรุณารีเฟรชและตรวจสอบอีกครั้ง",
        ),
        (ShiftError::DifferenceNeedsNote, "กรุณาระบุเหตุผลยอดต่างก่อนปิดกะ"),
    ];
    let mut ours = BTreeSet::new();
    for (error, sentence) in table {
        assert_eq!(error.to_string(), sentence);
        // A refusal travels as `AppError::Domain` holding the sentence and nothing else.
        match AppError::from(error) {
            AppError::Domain(text) => assert_eq!(text, sentence),
            other => panic!("{sentence}: converted to {other:?}"),
        }
        ours.insert(sentence.to_owned());
    }
    // "not enough" names the currency, any of the register
    for &c in CurrencyCode::all() {
        ours.insert(format!(
            "ยอด {} ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน",
            c.as_str()
        ));
        assert!(ours.contains(&ShiftError::Insufficient(c).to_string()));
    }
    // Every sentence v2 produced while the fixture was recorded is one of ours.
    for sentence in fixture().sentences.keys() {
        assert!(
            ours.contains(sentence),
            "the fixture holds a sentence with no ShiftError: {sentence}"
        );
    }
}

// ── the payloads ────────────────────────────────────────────────────────────

#[test]
fn a_payload_is_refused_for_its_form_exactly_where_v2_refused_it() {
    let f = fixture();
    assert!(f.parse_cases.len() >= 150, "the fixture lost cases");
    let (mut refused, mut accepted) = (0, 0);
    for (n, case) in f.parse_cases.iter().enumerate() {
        // "$SHIFT:0" stands for some shift id; its value plays no part in the form check
        let payload = resolved(
            &case.payload,
            &["SH-11111111-1111-4111-8111-111111111111".to_owned()],
        );
        let got = match case.action.as_str() {
            "open" => OpenShift::from_payload(&payload).err(),
            "move" => MoveStock::from_payload(&payload).err(),
            "close" => CloseShift::from_payload(&payload).err(),
            other => panic!("fixture action {other}"),
        }
        .map(|e| e.to_string());
        assert_eq!(
            got, case.parse_error,
            "parse case {n}: {} {}",
            case.action, case.payload
        );
        if got.is_some() {
            refused += 1;
        } else {
            accepted += 1;
        }
    }
    assert!(refused >= 90 && accepted >= 50, "{refused} / {accepted}");
}

#[test]
fn a_number_is_read_as_written_never_through_binary_floating_point() {
    // 0.1 + 0.2 in a float is 0.30000000000000004; the text "0.3" is three tenths.
    for amount in [json!(0.3), json!("0.30"), json!(1234.56), json!(100000000)] {
        MoveStock::from_payload(&json!({
            "requestId": "req-number-0001-00000", "shiftId": "SH-x", "currency": "USD",
            "direction": "IN", "amount": amount, "reason": "r",
        }))
        .unwrap_or_else(|e| panic!("{amount}: {e}"));
    }
    for amount in [
        json!(true),
        json!([5]),
        json!({ "v": 5 }),
        json!(1e30),
        json!(-0.0001),
    ] {
        let got = MoveStock::from_payload(&json!({
            "requestId": "req-number-0001-00000", "shiftId": "SH-x", "currency": "USD",
            "direction": "IN", "amount": amount, "reason": "r",
        }));
        assert_eq!(
            got.err().map(|e| e.to_string()),
            Some(ShiftError::AmountInvalid.to_string()),
            "{amount}"
        );
    }
}

// ── whole shifts, against v2 ────────────────────────────────────────────────

/// Replays one recorded scenario on a branch of its own and compares, after
/// every step, the answer and everything the branch holds.
async fn replay(db: &TestDb, scenario: &Scenario) -> usize {
    let service = db.service();
    let branch = db.branch("replay").await;
    let mut shift_ids: Vec<String> = Vec::new();
    for (n, step) in scenario.steps.iter().enumerate() {
        let here = format!("{} — step {n} ({})", scenario.name, step.r#do);
        let now = at(n as i64, 0);
        let got: Option<String> = match step.r#do.as_str() {
            "open" => {
                let payload = resolved(step.payload.as_ref().unwrap(), &shift_ids);
                match OpenShift::from_payload(&payload) {
                    Err(e) => Some(e.to_string()),
                    Ok(cmd) => {
                        let (shift, refused) =
                            outcome(service.open(&branch, cmd, "replay", now).await);
                        if let Some(shift) = shift {
                            if !shift_ids.contains(&shift.id) {
                                shift_ids.push(shift.id);
                            }
                        }
                        refused
                    }
                }
            }
            "move" => {
                let payload = resolved(step.payload.as_ref().unwrap(), &shift_ids);
                match MoveStock::from_payload(&payload) {
                    Err(e) => Some(e.to_string()),
                    Ok(cmd) => outcome(service.move_stock(&branch, cmd, "replay", now).await).1,
                }
            }
            "close" => {
                let payload = resolved(step.payload.as_ref().unwrap(), &shift_ids);
                match CloseShift::from_payload(&payload) {
                    Err(e) => Some(e.to_string()),
                    Ok(cmd) => outcome(service.close(&branch, cmd, "replay", now).await).1,
                }
            }
            "tx" => {
                let delta = tx_delta(step.tx.as_ref().unwrap());
                let (shift_id, refused) = outcome(db.apply_delta(&branch, delta).await);
                if let Some(shift_id) = shift_id {
                    assert_eq!(
                        Some(&shift_id),
                        shift_ids.last(),
                        "{here}: the id returned is the open shift's"
                    );
                }
                refused
            }
            "abandon" => {
                if step.resolution.as_deref() == Some("CANCELLED") {
                    db.abandon(&branch, step.request_id.as_ref().unwrap()).await;
                }
                None
            }
            other => panic!("fixture step {other}"),
        };
        assert_eq!(got, step.v2_error, "{here}: the answer");

        let shifts: Vec<_> = service
            .shifts(&branch)
            .await
            .expect("list shifts")
            .iter()
            .map(shown_shift)
            .collect();
        assert_eq!(shifts, step.after.shifts, "{here}: the shifts afterwards");
        let movements: Vec<_> = service
            .movements(&branch)
            .await
            .expect("list movements")
            .iter()
            .map(|m| shown_movement(m, &shift_ids))
            .collect();
        assert_eq!(
            movements, step.after.movements,
            "{here}: the movements afterwards"
        );
    }
    scenario.steps.len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_written_scenarios_end_where_v2_ended() {
    let f = fixture();
    let db = TestDb::start().await;
    let (mut scenarios, mut steps) = (0, 0);
    for s in f
        .scenarios
        .iter()
        .filter(|s| !s.name.starts_with("seeded walk"))
    {
        steps += replay(&db, s).await;
        scenarios += 1;
    }
    assert!(scenarios >= 6 && steps >= 95, "{scenarios} / {steps}");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_seeded_walks_end_where_v2_ended() {
    let f = fixture();
    let db = TestDb::start().await;
    let (mut scenarios, mut steps) = (0, 0);
    for s in f
        .scenarios
        .iter()
        .filter(|s| s.name.starts_with("seeded walk"))
    {
        steps += replay(&db, s).await;
        scenarios += 1;
    }
    assert!(scenarios >= 24 && steps >= 330, "{scenarios} / {steps}");
    db.drop_schema().await;
}

// ── what is recorded ────────────────────────────────────────────────────────

fn is_id(text: &str, prefix: &str) -> bool {
    let Some(rest) = text.strip_prefix(prefix) else {
        return false;
    };
    rest.len() == 36
        && rest.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_opened_shift_records_who_when_and_with_what() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("open").await;
    // nanoseconds and microseconds are cut: the screens compare times to the millisecond
    let now = at(5, 123) + chrono::Duration::nanoseconds(456_789);

    let cmd = open_cmd(
        "  req-open-0001-000000000000  ",
        json!({ "THB": 500000, "USD": "1000.50", "JPY": 0 }),
    );
    let shift = service.open(&branch, cmd, "somchai", now).await.unwrap();

    assert!(is_id(&shift.id, "SH-"), "id {}", shift.id);
    assert_eq!(shift.branch_id, branch);
    assert_eq!(shift.status, ShiftStatus::Open);
    assert_eq!(shift.opened_at, at(5, 123));
    assert_eq!(shift.opened_by, "somchai");
    assert_eq!(shift.open_request_id, "req-open-0001-000000000000");
    assert_eq!(shift.opening.get(CurrencyCode::THB), money("500000"));
    assert_eq!(shift.opening.get(code("USD")), money("1000.50"));
    assert_eq!(shift.opening.non_zero().len(), 2);
    assert_eq!(shift.expected, shift.opening);
    assert_eq!(shift.closed_at, None);
    assert_eq!(shift.closed_by, None);
    assert_eq!(shift.counted, None);
    assert_eq!(shift.difference, None);
    assert_eq!(shift.note, "");
    assert_eq!(shift.close_request_id, None);

    // what was returned is what was stored
    assert_eq!(service.active(&branch).await.unwrap(), Some(shift.clone()));
    assert_eq!(service.shifts(&branch).await.unwrap(), vec![shift]);
    assert_eq!(service.movements(&branch).await.unwrap(), vec![]);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_movement_records_who_when_and_why_and_moves_the_expectation() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("move").await;
    let shift = open_with(&db, &branch, json!({ "USD": 100 })).await;

    let cmd = MoveStock::from_payload(&json!({
        "requestId": "req-move-0001-000000000000", "shiftId": shift.id, "currency": " USD ",
        "direction": "OUT", "amount": "40.25", "reason": "  to the safe  ",
    }))
    .unwrap();
    let m: StockMovement = service
        .move_stock(&branch, cmd, "malee", at(60, 7))
        .await
        .unwrap();

    assert!(is_id(&m.id, "MV-"), "id {}", m.id);
    assert_eq!(m.shift_id, shift.id);
    assert_eq!(m.at, at(60, 7));
    assert_eq!(m.by, "malee");
    assert_eq!(m.currency, code("USD"));
    assert_eq!(m.direction, Direction::Out);
    assert_eq!(m.amount, money("40.25"));
    assert_eq!(m.reason, "to the safe");
    assert_eq!(m.request_id, "req-move-0001-000000000000");
    assert_eq!(service.movements(&branch).await.unwrap(), vec![m.clone()]);
    assert_eq!(expected_of(&service, &branch, "USD").await, "59.75");

    // The same request again, later and by someone else: the saved movement comes
    // back as it was saved, and nothing moves a second time.
    let again = service
        .move_stock(
            &branch,
            MoveStock::from_payload(&json!({
                "requestId": "req-move-0001-000000000000", "shiftId": shift.id, "currency": "USD",
                "direction": "OUT", "amount": 40.25, "reason": "to the safe",
            }))
            .unwrap(),
            "someone else",
            at(999, 0),
        )
        .await
        .unwrap();
    assert_eq!(again, m);
    assert_eq!(service.movements(&branch).await.unwrap().len(), 1);
    assert_eq!(expected_of(&service, &branch, "USD").await, "59.75");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_shift_records_the_count_the_difference_and_the_reason() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("close").await;
    let opened = open_with(&db, &branch, json!({ "THB": 1000, "USD": 50 })).await;

    let closed = service
        .close(
            &branch,
            close_cmd(
                "req-close-0001-00000000000",
                &opened.id,
                json!({ "THB": 990.5, "USD": 51 }),
                json!({ "THB": 1000, "USD": 50 }),
                "  nine fifty short, a dollar over  ",
            ),
            "supervisor",
            at(3600, 250),
        )
        .await
        .unwrap();

    assert_eq!(closed.id, opened.id);
    assert_eq!(closed.status, ShiftStatus::Closed);
    assert_eq!(closed.opened_at, opened.opened_at);
    assert_eq!(closed.opened_by, "opener");
    assert_eq!(closed.opening, opened.opening);
    assert_eq!(closed.closed_at, Some(at(3600, 250)));
    assert_eq!(closed.closed_by.as_deref(), Some("supervisor"));
    assert_eq!(closed.expected, opened.opening);
    let counted = closed.counted.as_ref().expect("the count is kept");
    assert_eq!(counted.get(CurrencyCode::THB), money("990.5"));
    assert_eq!(counted.get(code("USD")), money("51"));
    let difference = shown_delta(closed.difference.as_ref().expect("the difference is kept"));
    assert_eq!(difference.get("THB").map(String::as_str), Some("-9.50"));
    assert_eq!(difference.get("USD").map(String::as_str), Some("1.00"));
    assert_eq!(difference.len(), 2);
    assert_eq!(closed.note, "nine fifty short, a dollar over");
    assert_eq!(
        closed.close_request_id.as_deref(),
        Some("req-close-0001-00000000000")
    );

    assert_eq!(service.active(&branch).await.unwrap(), None);
    assert_eq!(service.shifts(&branch).await.unwrap(), vec![closed.clone()]);

    // The same close again returns the closed shift; the open request still returns it too.
    let again = service
        .close(
            &branch,
            close_cmd(
                "req-close-0001-00000000000",
                &opened.id,
                json!({ "THB": 990.5, "USD": 51 }),
                json!({}),
                "nine fifty short, a dollar over",
            ),
            "x",
            at(9999, 0),
        )
        .await
        .unwrap();
    assert_eq!(again, closed);
    let reopened = service
        .open(
            &branch,
            open_cmd(&rid("setup", 0), json!({ "THB": 1000, "USD": 50 })),
            "x",
            at(9999, 0),
        )
        .await
        .unwrap();
    assert_eq!(reopened, closed);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shifts_and_movements_are_listed_oldest_first() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("list").await;
    let mut ids = Vec::new();
    for n in 0..3 {
        let shift = service
            .open(
                &branch,
                open_cmd(&rid("open", n), json!({ "USD": 10 })),
                "a",
                at(n as i64 * 100, 0),
            )
            .await
            .unwrap();
        for k in 0..2 {
            service
                .move_stock(
                    &branch,
                    move_cmd(
                        &rid("move", n * 2 + k),
                        &shift.id,
                        "USD",
                        "IN",
                        json!(k + 1),
                    ),
                    "a",
                    at(n as i64 * 100 + k as i64 + 1, 0),
                )
                .await
                .unwrap();
        }
        service
            .close(
                &branch,
                close_cmd(
                    &rid("close", n),
                    &shift.id,
                    json!({ "USD": 13 }),
                    json!({ "USD": 13 }),
                    "",
                ),
                "a",
                at(n as i64 * 100 + 50, 0),
            )
            .await
            .unwrap();
        ids.push(shift.id);
    }
    let listed: Vec<String> = service
        .shifts(&branch)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(listed, ids);
    let movements = service.movements(&branch).await.unwrap();
    let got: Vec<(String, String)> = movements
        .iter()
        .map(|m| (m.shift_id.clone(), m.request_id.clone()))
        .collect();
    let want: Vec<(String, String)> = (0..6)
        .map(|i| (ids[i / 2].clone(), rid("move", i)))
        .collect();
    assert_eq!(got, want);
    db.drop_schema().await;
}

// ── branches ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_that_does_not_exist_is_refused_and_leaves_no_trace() {
    let db = TestDb::start().await;
    let service = db.service();
    let ghost = BranchId::new(shift_service_common::unique("ghost"));

    let opened = service
        .open(
            &ghost,
            open_cmd(&rid("open", 1), json!({ "USD": 1 })),
            "a",
            at(0, 0),
        )
        .await;
    assert_eq!(refusal(opened), BRANCH_NOT_FOUND);
    let moved = service
        .move_stock(
            &ghost,
            move_cmd(&rid("move", 1), "SH-x", "USD", "IN", json!(1)),
            "a",
            at(0, 0),
        )
        .await;
    assert_eq!(refusal(moved), BRANCH_NOT_FOUND);
    let closed = service
        .close(
            &ghost,
            close_cmd(&rid("close", 1), "SH-x", json!({}), json!({}), ""),
            "a",
            at(0, 0),
        )
        .await;
    assert_eq!(refusal(closed), BRANCH_NOT_FOUND);

    // The per-branch lock row is created on first use; a branch that does not
    // exist must not get one.
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM branch_versions WHERE branch_id = $1")
        .bind(ghost.as_str())
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_branch_knows_nothing_of_another() {
    let db = TestDb::start().await;
    let service = db.service();
    let (a, b) = (db.branch("a").await, db.branch("b").await);

    let shift_a = service
        .open(
            &a,
            open_cmd(&rid("open", 1), json!({ "USD": 100 })),
            "a",
            at(0, 0),
        )
        .await
        .unwrap();
    // an open shift in A does not stop B opening
    let shift_b = service
        .open(
            &b,
            open_cmd(&rid("open", 2), json!({ "USD": 7 })),
            "b",
            at(1, 0),
        )
        .await
        .unwrap();
    assert_ne!(shift_a.id, shift_b.id);

    // A's shift id means nothing in B
    let moved = service
        .move_stock(
            &b,
            move_cmd(&rid("move", 1), &shift_a.id, "USD", "OUT", json!(50)),
            "b",
            at(2, 0),
        )
        .await;
    assert_eq!(refusal(moved), SHIFT_CHANGED);
    let closed = service
        .close(
            &b,
            close_cmd(
                &rid("close", 1),
                &shift_a.id,
                json!({ "USD": 100 }),
                json!({ "USD": 100 }),
                "",
            ),
            "b",
            at(3, 0),
        )
        .await;
    assert_eq!(refusal(closed), SHIFT_CHANGED);

    // a request A gave up on is still usable in B
    db.abandon(&a, &rid("move", 9)).await;
    let in_a = service
        .move_stock(
            &a,
            move_cmd(&rid("move", 9), &shift_a.id, "USD", "IN", json!(1)),
            "a",
            at(4, 0),
        )
        .await;
    assert_eq!(refusal(in_a), ABANDONED);
    service
        .move_stock(
            &b,
            move_cmd(&rid("move", 9), &shift_b.id, "USD", "IN", json!(1)),
            "b",
            at(4, 0),
        )
        .await
        .expect("B never gave up on that request");

    assert_eq!(expected_of(&service, &a, "USD").await, "100.00");
    assert_eq!(expected_of(&service, &b, "USD").await, "8.00");
    assert_eq!(service.shifts(&a).await.unwrap().len(), 1);
    assert_eq!(service.movements(&a).await.unwrap().len(), 0);
    assert_eq!(service.movements(&b).await.unwrap().len(), 1);
    db.drop_schema().await;
}

// ── one transaction, one version ────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_accepted_write_raises_the_branch_version_by_one_and_a_refusal_by_none() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("version").await;
    assert_eq!(db.version(&branch).await, 0);

    let shift = service
        .open(
            &branch,
            open_cmd(&rid("open", 1), json!({ "USD": 100 })),
            "a",
            at(0, 0),
        )
        .await
        .unwrap();
    assert_eq!(db.version(&branch).await, 1);

    service
        .move_stock(
            &branch,
            move_cmd(&rid("move", 1), &shift.id, "USD", "OUT", json!(30)),
            "a",
            at(1, 0),
        )
        .await
        .unwrap();
    assert_eq!(db.version(&branch).await, 2);

    db.apply_delta(
        &branch,
        StockDelta::for_movement(code("USD"), Direction::Out, money("20")),
    )
    .await
    .unwrap();
    assert_eq!(db.version(&branch).await, 3);

    // refusals: nothing is written, the version stays
    let refused = service
        .open(&branch, open_cmd(&rid("open", 2), json!({})), "a", at(2, 0))
        .await;
    assert_eq!(refusal(refused), ALREADY_OPEN);
    let refused = service
        .move_stock(
            &branch,
            move_cmd(&rid("move", 2), &shift.id, "USD", "OUT", json!(50.01)),
            "a",
            at(2, 0),
        )
        .await;
    assert_eq!(refusal(refused), USD_SHORT);
    let refused = db
        .apply_delta(
            &branch,
            StockDelta::for_movement(code("USD"), Direction::Out, money("50.01")),
        )
        .await;
    assert_eq!(refusal(refused), USD_SHORT);
    let refused = service
        .close(
            &branch,
            close_cmd(
                &rid("close", 1),
                &shift.id,
                json!({ "USD": 49 }),
                json!({ "USD": 50 }),
                "",
            ),
            "a",
            at(2, 0),
        )
        .await;
    assert_eq!(refusal(refused), "กรุณาระบุเหตุผลยอดต่างก่อนปิดกะ");
    assert_eq!(db.version(&branch).await, 3);
    assert_eq!(service.shifts(&branch).await.unwrap().len(), 1);
    assert_eq!(service.movements(&branch).await.unwrap().len(), 1);
    assert_eq!(expected_of(&service, &branch, "USD").await, "50.00");

    service
        .close(
            &branch,
            close_cmd(
                &rid("close", 2),
                &shift.id,
                json!({ "USD": 50 }),
                json!({ "USD": 50 }),
                "",
            ),
            "a",
            at(3, 0),
        )
        .await
        .unwrap();
    assert_eq!(db.version(&branch).await, 4);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_in_transaction_forms_commit_or_vanish_with_their_caller() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("compose").await;

    // A caller (the custody service, ticket T10) opens a shift and moves stock in
    // one transaction of its own, then fails: none of it may remain.
    let (svc, b) = (service.clone(), branch.clone());
    let failed: AppResult<()> = db
        .branch_tx()
        .run(&branch, move |tx| {
            Box::pin(async move {
                // the caller reads the open shift through its own transaction
                assert_eq!(svc.active_in(tx, &b).await?, None);
                let shift = svc
                    .open_in(
                        tx,
                        &b,
                        open_cmd(&rid("open", 1), json!({ "USD": 10 })),
                        "t10",
                        at(0, 0),
                    )
                    .await?;
                // … and sees what it has not committed yet
                assert_eq!(svc.active_in(tx, &b).await?, Some(shift.clone()));
                svc.move_stock_in(
                    tx,
                    &b,
                    move_cmd(&rid("move", 1), &shift.id, "USD", "IN", json!(5)),
                    "t10",
                    at(1, 0),
                )
                .await?;
                Err(AppError::Internal("the caller changed its mind".to_owned()))
            })
        })
        .await;
    assert!(matches!(failed, Err(AppError::Internal(_))), "{failed:?}");
    assert_eq!(service.shifts(&branch).await.unwrap(), vec![]);
    assert_eq!(service.movements(&branch).await.unwrap(), vec![]);
    assert_eq!(db.version(&branch).await, 0);

    // The same steps, and a close, in one transaction that succeeds: all of it,
    // under a single version step.
    let (svc, b) = (service.clone(), branch.clone());
    let closed: Shift = db
        .branch_tx()
        .run(&branch, move |tx| {
            Box::pin(async move {
                let shift = svc
                    .open_in(
                        tx,
                        &b,
                        open_cmd(&rid("open", 1), json!({ "USD": 10 })),
                        "t10",
                        at(0, 0),
                    )
                    .await?;
                svc.move_stock_in(
                    tx,
                    &b,
                    move_cmd(&rid("move", 1), &shift.id, "USD", "IN", json!(5)),
                    "t10",
                    at(1, 0),
                )
                .await?;
                let held = svc
                    .apply_delta_in(
                        tx,
                        &b,
                        &StockDelta::for_movement(code("USD"), Direction::Out, money("2.5")),
                    )
                    .await?;
                assert_eq!(held, shift.id);
                let now_open = svc.active_in(tx, &b).await?.expect("the shift is open");
                assert_eq!(shown_balances(&now_open.expected)["USD"], "12.50");
                svc.close_in(
                    tx,
                    &b,
                    close_cmd(
                        &rid("close", 1),
                        &shift.id,
                        json!({ "USD": 12.5 }),
                        json!({ "USD": 12.5 }),
                        "",
                    ),
                    "t10",
                    at(2, 0),
                )
                .await
            })
        })
        .await
        .unwrap();
    assert_eq!(closed.status, ShiftStatus::Closed);
    assert_eq!(shown_balances(&closed.expected)["USD"], "12.50");
    assert_eq!(db.version(&branch).await, 1);
    assert_eq!(service.shifts(&branch).await.unwrap(), vec![closed]);
    assert_eq!(service.movements(&branch).await.unwrap().len(), 1);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saved_transaction_changes_the_open_shift_or_is_refused() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("delta").await;

    let buy = StockDelta::for_movement(code("EUR"), Direction::In, money("10"));
    assert_eq!(
        refusal(db.apply_delta(&branch, buy.clone()).await),
        NO_OPEN_SHIFT
    );

    let shift = open_with(&db, &branch, json!({ "THB": 100, "USD": 5 })).await;
    assert_eq!(db.apply_delta(&branch, buy).await.unwrap(), shift.id);
    // a change that touches nothing is accepted and changes nothing
    assert_eq!(
        db.apply_delta(&branch, StockDelta::none()).await.unwrap(),
        shift.id
    );
    // both legs of a cash purchase land together, or neither does
    let both = StockDelta::of(&[
        (code("USD"), MoneyDelta::ZERO.plus(money("3"))),
        (CurrencyCode::THB, MoneyDelta::ZERO.minus(money("100.01"))),
    ]);
    assert_eq!(
        refusal(db.apply_delta(&branch, both).await),
        "ยอด THB ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน"
    );
    let now = service.active(&branch).await.unwrap().unwrap();
    let held = shown_balances(&now.expected);
    assert_eq!(held["THB"], "100.00");
    assert_eq!(held["USD"], "5.00");
    assert_eq!(held["EUR"], "10.00");
    assert_eq!(held.len(), 3);
    // the opening is what was counted in; it never moves
    assert_eq!(now.opening, shift.opening);
    db.drop_schema().await;
}

// ── two at once ─────────────────────────────────────────────────────────────

/// Runs the futures at the same instant and returns their results in order.
async fn together<T, F>(jobs: Vec<F>) -> Vec<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
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
async fn of_many_opens_at_once_exactly_one_wins_and_the_rest_are_told_why() {
    let db = TestDb::start().await;
    for round in 0..5 {
        let branch = db.branch("race_open").await;
        let jobs: Vec<_> = (0..8)
            .map(|n| {
                let (service, branch) = (db.service(), branch.clone());
                async move {
                    service
                        .open(
                            &branch,
                            open_cmd(&rid("open", n), json!({ "USD": n + 1 })),
                            "racer",
                            at(0, 0),
                        )
                        .await
                }
            })
            .collect();
        let results = together(jobs).await;
        let won: Vec<&Shift> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(won.len(), 1, "round {round}: {results:?}");
        for r in results.iter().filter(|r| r.is_err()) {
            // a sentence, never a raw database error about a unique index
            match r {
                Err(AppError::Domain(s)) => assert_eq!(s, ALREADY_OPEN, "round {round}"),
                other => panic!("round {round}: {other:?}"),
            }
        }
        let stored = db.service().shifts(&branch).await.unwrap();
        assert_eq!(stored.len(), 1, "round {round}");
        assert_eq!(&stored[0], won[0], "round {round}");
        assert_eq!(db.version(&branch).await, 1, "round {round}");
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn withdrawals_at_once_never_take_more_than_the_till_holds() {
    let db = TestDb::start().await;
    for round in 0..5 {
        let branch = db.branch("race_out").await;
        let shift = open_with(&db, &branch, json!({ "USD": 100 })).await;
        // ten people each take 30 from a till of 100: three can
        let jobs: Vec<_> = (0..10)
            .map(|n| {
                let (service, branch, shift_id) = (db.service(), branch.clone(), shift.id.clone());
                async move {
                    service
                        .move_stock(
                            &branch,
                            move_cmd(&rid("move", n), &shift_id, "USD", "OUT", json!(30)),
                            "racer",
                            at(1, 0),
                        )
                        .await
                }
            })
            .collect();
        let results = together(jobs).await;
        let (mut ok, mut short) = (0, 0);
        for r in &results {
            match r {
                Ok(_) => ok += 1,
                Err(AppError::Domain(s)) if s == USD_SHORT => short += 1,
                other => panic!("round {round}: {other:?}"),
            }
        }
        assert_eq!((ok, short), (3, 7), "round {round}");
        assert_eq!(
            expected_of(&db.service(), &branch, "USD").await,
            "10.00",
            "round {round}"
        );
        assert_eq!(
            db.service().movements(&branch).await.unwrap().len(),
            3,
            "round {round}"
        );
        assert_eq!(db.version(&branch).await, 1 + 3, "round {round}");
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn sales_and_withdrawals_at_once_share_the_same_till() {
    let db = TestDb::start().await;
    let db = Arc::new(db);
    for round in 0..5 {
        let branch = db.branch("race_mixed").await;
        let shift = open_with(&db, &branch, json!({ "USD": 200 })).await;
        // six sales of 30 (through `apply_delta_in`) and six withdrawals of 30
        // against 200: six of the twelve fit, whichever they are
        let mut jobs: Vec<std::pin::Pin<Box<dyn Future<Output = AppResult<()>> + Send>>> =
            Vec::new();
        for n in 0..6 {
            let (db2, branch2) = (db.clone(), branch.clone());
            jobs.push(Box::pin(async move {
                db2.apply_delta(
                    &branch2,
                    StockDelta::for_movement(code("USD"), Direction::Out, money("30")),
                )
                .await
                .map(|_| ())
            }));
            let (service, branch2, shift_id) = (db.service(), branch.clone(), shift.id.clone());
            jobs.push(Box::pin(async move {
                service
                    .move_stock(
                        &branch2,
                        move_cmd(&rid("move", n), &shift_id, "USD", "OUT", json!(30)),
                        "racer",
                        at(1, 0),
                    )
                    .await
                    .map(|_| ())
            }));
        }
        let results = together(jobs).await;
        let ok = results.iter().filter(|r| r.is_ok()).count();
        for r in results.iter().filter(|r| r.is_err()) {
            match r {
                Err(AppError::Domain(s)) => assert_eq!(s, USD_SHORT, "round {round}"),
                other => panic!("round {round}: {other:?}"),
            }
        }
        assert_eq!(ok, 6, "round {round}");
        assert_eq!(
            expected_of(&db.service(), &branch, "USD").await,
            "20.00",
            "round {round}"
        );
    }
    Arc::try_unwrap(db)
        .ok()
        .expect("no task still holds the database")
        .drop_schema()
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn one_request_sent_many_times_at_once_is_applied_once() {
    let db = TestDb::start().await;
    for round in 0..5 {
        let branch = db.branch("race_same").await;
        let shift = open_with(&db, &branch, json!({ "USD": 100 })).await;
        let jobs: Vec<_> = (0..6)
            .map(|_| {
                let (service, branch, shift_id) = (db.service(), branch.clone(), shift.id.clone());
                async move {
                    service
                        .move_stock(
                            &branch,
                            move_cmd(&rid("move", 1), &shift_id, "USD", "IN", json!(5)),
                            "double click",
                            at(1, 0),
                        )
                        .await
                }
            })
            .collect();
        let results = together(jobs).await;
        let ids: BTreeSet<String> = results
            .into_iter()
            .map(|r| r.expect("every copy of the request is answered").id)
            .collect();
        assert_eq!(ids.len(), 1, "round {round}: one movement, returned to all");
        assert_eq!(
            db.service().movements(&branch).await.unwrap().len(),
            1,
            "round {round}"
        );
        assert_eq!(
            expected_of(&db.service(), &branch, "USD").await,
            "105.00",
            "round {round}"
        );
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_close_and_a_withdrawal_at_once_cannot_both_count_the_same_money() {
    let db = TestDb::start().await;
    for round in 0..8 {
        let branch = db.branch("race_close").await;
        let shift = open_with(&db, &branch, json!({ "USD": 100 })).await;
        let (s1, b1, id1) = (db.service(), branch.clone(), shift.id.clone());
        let (s2, b2, id2) = (db.service(), branch.clone(), shift.id.clone());
        let jobs: Vec<std::pin::Pin<Box<dyn Future<Output = AppResult<()>> + Send>>> = vec![
            Box::pin(async move {
                s1.close(
                    &b1,
                    close_cmd(
                        &rid("close", 1),
                        &id1,
                        json!({ "USD": 100 }),
                        json!({ "USD": 100 }),
                        "",
                    ),
                    "closer",
                    at(2, 0),
                )
                .await
                .map(|_| ())
            }),
            Box::pin(async move {
                s2.move_stock(
                    &b2,
                    move_cmd(&rid("move", 1), &id2, "USD", "OUT", json!(40)),
                    "taker",
                    at(2, 0),
                )
                .await
                .map(|_| ())
            }),
        ];
        let results = together(jobs).await;
        let service = db.service();
        let shifts = service.shifts(&branch).await.unwrap();
        let movements = service.movements(&branch).await.unwrap();
        match (&results[0], &results[1]) {
            // the close went first: the withdrawal finds no open shift
            (Ok(()), Err(AppError::Domain(s))) => {
                assert_eq!(s, SHIFT_CHANGED, "round {round}");
                assert_eq!(shifts[0].status, ShiftStatus::Closed);
                assert_eq!(shown_balances(&shifts[0].expected)["USD"], "100.00");
                assert!(movements.is_empty());
            }
            // the withdrawal went first: the count no longer matches
            (Err(AppError::Domain(s)), Ok(())) => {
                assert_eq!(
                    s, "ยอดเปลี่ยนระหว่างตรวจนับ กรุณารีเฟรชและตรวจสอบอีกครั้ง",
                    "round {round}"
                );
                assert_eq!(shifts[0].status, ShiftStatus::Open);
                assert_eq!(shown_balances(&shifts[0].expected)["USD"], "60.00");
                assert_eq!(movements.len(), 1);
            }
            other => panic!("round {round}: both or neither went through: {other:?}"),
        }
    }
    db.drop_schema().await;
}

// ── the database refuses what the code must never do ────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_database_itself_refuses_a_second_open_shift_in_a_branch() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("index").await;
    let first = open_with(&db, &branch, json!({})).await;
    service
        .close(
            &branch,
            close_cmd(&rid("close", 1), &first.id, json!({}), json!({}), ""),
            "a",
            at(1, 0),
        )
        .await
        .unwrap();
    service
        .open(&branch, open_cmd(&rid("open", 2), json!({})), "a", at(2, 0))
        .await
        .unwrap();

    // Bypassing the service: flip the closed shift back to OPEN next to the open one.
    let forced = sqlx::query("UPDATE shifts SET status = 'OPEN' WHERE id = $1")
        .bind(&first.id)
        .execute(&db.pool)
        .await;
    assert!(
        matches!(forced, Err(sqlx::Error::Database(_))),
        "two OPEN shifts in one branch were stored: {forced:?}"
    );
    // and what refuses it is a unique index over the branch, restricted to OPEN rows
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname = current_schema() AND tablename = 'shifts'",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(
        indexes.iter().any(|d| {
            let d = d.to_uppercase();
            d.starts_with("CREATE UNIQUE INDEX")
                && d.contains("BRANCH_ID")
                && d.contains("WHERE")
                && d.contains("'OPEN'")
        }),
        "no partial unique index on shifts(branch_id) WHERE status = 'OPEN': {indexes:?}"
    );
    // and a status that is neither OPEN nor CLOSED is not storable
    let odd = sqlx::query("UPDATE shifts SET status = 'PAUSED' WHERE id = $1")
        .bind(&first.id)
        .execute(&db.pool)
        .await;
    assert!(odd.is_err(), "status PAUSED was stored");
    db.drop_schema().await;
}
