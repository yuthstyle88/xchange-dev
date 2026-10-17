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
//!
//! CA-10 (#626, #2955; BE controller ruling, #626 issuecomment-6234829938 item 2) deletes v2's whole shift path:
//! `ShiftService::{open, open_in, close, close_in, move_stock, move_stock_in, apply_delta_in, active, active_in}`
//! and the `OpenShift` / `MoveStock` / `CloseShift` commands. The tests whose subject was that path are retired
//! (the payload forms, the v2 replay, open / move / close and their idempotency, the in-transaction forms, the
//! per-branch index, the races of v2 commands). What survives is restated on SQL-seeded shifts and on the
//! per-teller delta `apply_delta_for_teller_in`: the refusal sentences, the listing order, branch isolation, a
//! saved transaction's change to the open shift, and the till under concurrent sales.

mod shift_service_common;

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;

use pos_error::AppError;
use shift_service_common::{TestDb, at, code, fixture, money, refusal, shown_balances, within};
use tokio::sync::Barrier;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

const ALREADY_OPEN: &str = "มีกะเปิดอยู่แล้ว กรุณารีเฟรช";
const SHIFT_CHANGED: &str = "กะเปลี่ยนหรือปิดแล้ว กรุณารีเฟรช";
const NO_OPEN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
const BRANCH_NOT_FOUND: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
const ABANDONED: &str = "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด";
const USD_SHORT: &str = "ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน";

fn rid(label: &str, n: usize) -> String {
    format!("req-{label}-{n:04}-000000000000")
}

/// A seeded OPEN shift of `opener` at `branch` (CA-10: replaces v2's `ShiftService::open` as setup).
async fn open_with(
    db: &TestDb,
    branch: &BranchId,
    opener: &str,
    opening: &[(&str, &str)],
) -> String {
    db.seed_open_shift(branch, opener, &rid("setup", 0), opening, at(0, 0))
        .await
}

/// What the shift `id` should hold in `currency` (CA-10: read by id, not through the per-branch `active`).
async fn expected_of(db: &TestDb, branch: &BranchId, id: &str, currency: &str) -> String {
    let shift = db
        .service()
        .shift(branch, id)
        .await
        .expect("read the shift")
        .expect("the shift exists");
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

// ── the reads ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shifts_and_movements_are_listed_oldest_first() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("list").await;
    let mut ids = Vec::new();
    for n in 0..3 {
        let base = n as i64 * 100;
        let shift = db
            .seed_open_shift(&branch, "a", &rid("open", n), &[("USD", "10")], at(base, 0))
            .await;
        for k in 0..2 {
            db.seed_movement(
                &branch,
                &shift,
                "USD",
                "IN",
                &(k + 1).to_string(),
                &rid("move", n * 2 + k),
                at(base + k as i64 + 1, 0),
            )
            .await;
        }
        db.seed_close(
            &shift,
            "a",
            &rid("close", n),
            &[("USD", "13")],
            "",
            at(base + 50, 0),
        )
        .await;
        ids.push(shift);
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
async fn one_branch_knows_nothing_of_another() {
    let db = TestDb::start().await;
    let service = db.service();
    let (a, b) = (db.branch("a").await, db.branch("b").await);

    let shift_a = open_with(&db, &a, "a", &[("USD", "100")]).await;
    // an open shift in A does not stop B opening
    let shift_b = open_with(&db, &b, "b", &[("USD", "7")]).await;
    assert_ne!(shift_a, shift_b);
    db.seed_movement(&b, &shift_b, "USD", "IN", "1", &rid("move", 9), at(4, 0))
        .await;

    // A's shift id means nothing in B
    assert_eq!(service.shift(&b, &shift_a).await.unwrap(), None);
    // A's teller has no open shift in B: a sale there is refused and B's till does not move
    let teller_a = db.teller("a").await;
    let sale = StockDelta::for_movement(code("USD"), Direction::Out, money("50"));
    assert_eq!(
        refusal(db.apply_delta(&b, &teller_a, sale).await),
        NO_OPEN_SHIFT
    );

    assert_eq!(expected_of(&db, &a, &shift_a, "USD").await, "100.00");
    assert_eq!(expected_of(&db, &b, &shift_b, "USD").await, "8.00");
    assert_eq!(service.shifts(&a).await.unwrap().len(), 1);
    assert_eq!(service.movements(&a).await.unwrap().len(), 0);
    assert_eq!(service.movements(&b).await.unwrap().len(), 1);
    db.drop_schema().await;
}

// ── a saved transaction ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_saved_transaction_changes_the_open_shift_or_is_refused() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.branch("delta").await;
    let teller = db.teller("opener").await;

    let buy = StockDelta::for_movement(code("EUR"), Direction::In, money("10"));
    assert_eq!(
        refusal(db.apply_delta(&branch, &teller, buy.clone()).await),
        NO_OPEN_SHIFT
    );

    let shift = open_with(&db, &branch, "opener", &[("THB", "100"), ("USD", "5")]).await;
    let opened = service.shift(&branch, &shift).await.unwrap().unwrap();
    assert_eq!(db.apply_delta(&branch, &teller, buy).await.unwrap(), shift);
    // a change that touches nothing is accepted and changes nothing
    assert_eq!(
        db.apply_delta(&branch, &teller, StockDelta::none())
            .await
            .unwrap(),
        shift
    );
    // both legs of a cash purchase land together, or neither does
    let both = StockDelta::of(&[
        (code("USD"), MoneyDelta::ZERO.plus(money("3"))),
        (CurrencyCode::THB, MoneyDelta::ZERO.minus(money("100.01"))),
    ]);
    assert_eq!(
        refusal(db.apply_delta(&branch, &teller, both).await),
        "ยอด THB ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน"
    );
    let now = service.shift(&branch, &shift).await.unwrap().unwrap();
    let held = shown_balances(&now.expected);
    assert_eq!(held["THB"], "100.00");
    assert_eq!(held["USD"], "5.00");
    assert_eq!(held["EUR"], "10.00");
    assert_eq!(held.len(), 3);
    // the opening is what was counted in; it never moves
    assert_eq!(now.opening, opened.opening);
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

/// CA-10: restates `sales_and_withdrawals_at_once_share_the_same_till` and
/// `withdrawals_at_once_never_take_more_than_the_till_holds` without v2's `move_stock`: sales are the writer that
/// survives, through the per-teller delta.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn sales_at_once_never_take_more_than_the_till_holds() {
    let db = Arc::new(TestDb::start().await);
    let teller = db.teller("racer").await;
    for round in 0..5 {
        let branch = db.branch("race_sales").await;
        let shift = open_with(&db, &branch, "racer", &[("USD", "200")]).await;
        // twelve sales of 30 against 200: six fit, whichever they are
        let jobs: Vec<_> = (0..12)
            .map(|_| {
                let (db2, branch2, teller2) = (db.clone(), branch.clone(), teller.clone());
                async move {
                    db2.apply_delta(
                        &branch2,
                        &teller2,
                        StockDelta::for_movement(code("USD"), Direction::Out, money("30")),
                    )
                    .await
                }
            })
            .collect();
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
            expected_of(&db, &branch, &shift, "USD").await,
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
