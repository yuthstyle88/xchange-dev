//! Written before the implementation (ticket #571). Do not edit, delete or weaken.
//!
//! Specification of `CustodyPages`: the bounded, paged read port the screens use
//! instead of `CustodyRepository::load_state` (design
//! `docs/design/xchange-v3-wave3-staff.md` §3.2, tests T-P1…T-P10 of §8.1 row S-0a).
//!
//! - `requests`, `holds`, `attendance`: newest first (`position DESC`); `before` is
//!   an id from the previous page and answers the rows strictly older than it; an
//!   id the branch does not hold answers an empty page; at most `limit` rows; one
//!   branch only. A request carries its signatures in `position` order.
//! - `floor`: the branch's one `ops_state` row (None when it has none) and `me`'s
//!   latest attendance row when that row has no `clockOut` (absent or null).
//! - `requests_between`, `exceptions_between`: `position ASC`, the rows whose
//!   `data->>'at'` (epoch ms) is in `[from, to)`.
//! - `open_holds`: the OPEN holds of the branch, of every day.
//!
//! T-P9 (`load_state` answers exactly as before the decoders move to `decode.rs`)
//! has no test here: the existing custody tests (`custody_service_test.rs`,
//! `custody_write_volume_test.rs`, `undecodable_custody_*_test.rs`) pin it and stay
//! unchanged. `seed_is_what_load_state_answers` below only proves that the seed
//! these tests write is what `load_state` reads back.
//!
//! The rows are written through `CustodyRepository::store_state_in`, the same path
//! the service writes them with, so every row here is one the decoders accept. The
//! 10,000-row seed of T-P10 is one `INSERT … SELECT generate_series` per table, in
//! the JSON shapes `custody_repository.rs` writes (`attendance_data`,
//! `request_data`); `t_p10_floor_reads_one_ops_row_and_at_most_one_attendance_row`
//! loads it with `load_state` first, which proves it decodes.
//!
//! `OpsRow`'s fields are [proposed] in the ticket; these tests read only `enabled`,
//! `phase`, `custodian` and `cutoff` (the names `CustodyState` gives the same
//! `ops_state` columns), and of `cutoff` only whether it is present.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`, PostgreSQL 15 or later). Without it they FAIL; none skips.

mod shift_service_common;

use std::collections::BTreeMap;
use std::str::FromStr;

use serde_json::json;
use shift_service_common::{MUST_FINISH, TestDb, unique, within};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::repositories::custody_pages::{CustodyPages, FloorFacts};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::cutoff::Cutoff;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_pages::PgCustodyPages;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

/// 2026-10-07 03:00:00 UTC in epoch milliseconds; every time here is after it.
const T0: i64 = 1_791_342_000_000;
const MINUTE: i64 = 60_000;
const DAY: i64 = 24 * 60 * MINUTE;

fn t(millis_after_t0: i64) -> CustodyTime {
    CustodyTime::from_epoch_millis(T0 + millis_after_t0)
}

// ── the rows ────────────────────────────────────────────────────────────────

fn attendance(id: &str, employee: &str, clock_in: i64, clock_out: Option<i64>) -> Attendance {
    Attendance {
        id: id.to_string(),
        employee_id: employee.to_string(),
        role: CustodyRole::Front,
        clock_in: t(clock_in),
        clock_out: clock_out.map(t),
    }
}

fn hold(id: &str, at: i64, status: HoldStatus) -> CustomerHold {
    let released = status == HoldStatus::Released;
    CustomerHold {
        id: id.to_string(),
        customer_id: format!("cust-{id}"),
        reason: format!("reason {id}"),
        by: "monitor".to_string(),
        at: t(at),
        status,
        reviewed_by: released.then(|| "manager".to_string()),
        reviewed_at: released.then(|| t(at + MINUTE)),
        note: released.then(|| "checked".to_string()),
    }
}

fn exception(id: &str, at: i64) -> CashException {
    CashException {
        id: id.to_string(),
        request_id: format!("rq-of-{id}"),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: None,
        safe_variance: None,
        note: format!("exception {id}"),
        at: t(at),
        reviewed_by: None,
        reviewed_at: None,
    }
}

fn cutoff(by: &str) -> Cutoff {
    Cutoff {
        at: t(10 * MINUTE),
        by: by.to_string(),
        shift_id: "shift-1".to_string(),
        last_ref: None,
        pending: vec!["pending-1".to_string()],
    }
}

fn state() -> CustodyState {
    CustodyState {
        enabled: true,
        phase: Phase::Open,
        custodian: Some("custodian".to_string()),
        ..CustodyState::initial()
    }
}

/// `n` items named `<tag>-<what><i>`, one minute apart: position i is the i-th.
fn names(tag: &str, what: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{tag}-{what}{i}")).collect()
}

fn ids<T>(rows: &[T], id: impl Fn(&T) -> &str) -> Vec<String> {
    rows.iter().map(|r| id(r).to_string()).collect()
}

fn newest_first<T: Clone>(rows: &[T]) -> Vec<T> {
    rows.iter().rev().cloned().collect()
}

// ── the database ────────────────────────────────────────────────────────────

/// Writes `after` as the branch's custody state, the way a command stores it: every
/// row through the repository, at the position it has in its list.
async fn store(pool: &PgPool, branch: &BranchId, after: &CustodyState) {
    let repo = PgCustodyRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    repo.store_state_in(&mut tx, branch, &CustodyState::initial(), after)
        .await
        .expect("store the seed through CustodyRepository::store_state_in");
    tx.commit().await.expect("commit the seed");
}

/// A branch with `n` holds, attendance records and exceptions, one
/// minute apart, ids prefixed with a tag unique to the branch.
struct Seeded {
    branch: BranchId,
    state: CustodyState,
}

async fn seeded(db: &TestDb, label: &str, n: usize) -> Seeded {
    let branch = db.branch(label).await;
    let tag = unique(label);
    let mut s = state();
    s.holds = names(&tag, "h", n)
        .iter()
        .enumerate()
        .map(|(i, id)| hold(id, i as i64 * MINUTE, HoldStatus::Open))
        .collect();
    s.attendance = names(&tag, "a", n)
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let employee = if i % 2 == 0 { "e1" } else { "e2" };
            attendance(
                id,
                employee,
                i as i64 * MINUTE,
                Some(i as i64 * MINUTE + 30),
            )
        })
        .collect();
    s.exceptions = names(&tag, "x", n)
        .iter()
        .enumerate()
        .map(|(i, id)| exception(id, i as i64 * MINUTE))
        .collect();
    store(&db.pool, &branch, &s).await;
    Seeded { branch, state: s }
}

fn pages(db: &TestDb) -> PgCustodyPages {
    PgCustodyPages::new(db.pool.clone())
}

async fn load(pool: &PgPool, branch: &BranchId) -> CustodyState {
    PgCustodyRepository::new(pool.clone())
        .load_state(branch)
        .await
        .expect("load_state decodes the seed")
}

// ── the seed itself ─────────────────────────────────────────────────────────

/// The seed every test below writes is what `load_state` reads back.
#[tokio::test]
async fn seed_is_what_load_state_answers() {
    within("seed is what load_state answers", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "seed", 5).await;
        let loaded = load(&db.pool, &s.branch).await;
        assert_eq!(loaded.holds.len(), 5);
        assert_eq!(loaded.attendance.len(), 5);
        assert_eq!(loaded.exceptions.len(), 5);
        assert!(loaded == s.state, "load_state answers the seed as stored");
    })
    .await;
}

// ── T-P1 newest first ───────────────────────────────────────────────────────

#[tokio::test]
async fn t_p1_requests_holds_and_attendance_come_newest_first_by_position() {
    within("T-P1", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "tp1", 5).await;
        let p = pages(&db);

        let holds = p.holds(&s.branch, None, 50).await.expect("holds");
        assert!(
            holds == newest_first(&s.state.holds),
            "holds: position DESC; got {:?}",
            ids(&holds, |h| &h.id)
        );
        let att = p
            .attendance(&s.branch, None, None, 50)
            .await
            .expect("attendance");
        assert!(
            att == newest_first(&s.state.attendance),
            "attendance: position DESC; got {:?}",
            ids(&att, |a| &a.id)
        );
    })
    .await;
}

/// Position, not time and not id, decides "newest": a row stored at a later
/// position with an earlier time and a smaller id still comes first.
#[tokio::test]
async fn t_p1_position_decides_newest_not_time_or_id() {
    within("T-P1 position", async {
        let db = TestDb::start().await;
        let branch = db.branch("tp1pos").await;
        let tag = unique("tp1pos");
        let mut s = state();
        s.holds = vec![
            hold(&format!("{tag}-b"), 5 * MINUTE, HoldStatus::Open),
            hold(&format!("{tag}-c"), 9 * MINUTE, HoldStatus::Open),
            hold(&format!("{tag}-a"), MINUTE, HoldStatus::Open),
        ];
        store(&db.pool, &branch, &s).await;

        let got = pages(&db).holds(&branch, None, 50).await.expect("holds");
        assert_eq!(
            ids(&got, |h| &h.id),
            vec![format!("{tag}-a"), format!("{tag}-c"), format!("{tag}-b")],
            "newest = highest position"
        );
    })
    .await;
}

// ── T-P2 before ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn t_p2_before_resumes_strictly_after_the_given_id() {
    within("T-P2", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "tp2", 5).await;
        let p = pages(&db);
        let b = &s.branch;

        let h = &s.state.holds;
        let got = p.holds(b, Some(&h[3].id), 50).await.expect("holds");
        assert!(
            got == vec![h[2].clone(), h[1].clone(), h[0].clone()],
            "holds before h3 = h2, h1, h0; got {:?}",
            ids(&got, |x| &x.id)
        );

        let a = &s.state.attendance;
        let got = p
            .attendance(b, None, Some(&a[3].id), 50)
            .await
            .expect("attendance");
        assert!(
            got == vec![a[2].clone(), a[1].clone(), a[0].clone()],
            "attendance before a3 = a2, a1, a0; got {:?}",
            ids(&got, |x| &x.id)
        );
    })
    .await;
}

#[tokio::test]
async fn t_p2_before_the_last_row_answers_an_empty_page() {
    within("T-P2 last", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "tp2last", 3).await;
        let p = pages(&db);
        let b = &s.branch;

        let oldest_hold = &s.state.holds[0].id;
        let oldest_att = &s.state.attendance[0].id;
        assert!(
            p.holds(b, Some(oldest_hold), 50)
                .await
                .expect("holds")
                .is_empty()
        );
        assert!(
            p.attendance(b, None, Some(oldest_att), 50)
                .await
                .expect("attendance")
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn t_p2_an_unknown_before_id_answers_an_empty_page() {
    within("T-P2 unknown", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "tp2unknown", 3).await;
        let p = pages(&db);
        let b = &s.branch;
        let unknown = unique("no-such-row");

        assert!(
            p.holds(b, Some(&unknown), 50)
                .await
                .expect("holds")
                .is_empty()
        );
        assert!(
            p.attendance(b, None, Some(&unknown), 50)
                .await
                .expect("attendance")
                .is_empty()
        );
    })
    .await;
}

// ── T-P3 limit ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn t_p3_limit_is_honoured() {
    within("T-P3", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "tp3", 5).await;
        let p = pages(&db);
        let b = &s.branch;

        let h = &s.state.holds;
        let holds = p.holds(b, None, 3).await.expect("holds");
        assert!(holds == vec![h[4].clone(), h[3].clone(), h[2].clone()]);

        let a = &s.state.attendance;
        let att = p.attendance(b, None, None, 1).await.expect("attendance");
        assert!(att == vec![a[4].clone()]);
    })
    .await;
}

// ── T-P4 one branch only ────────────────────────────────────────────────────

#[tokio::test]
async fn t_p4_another_branchs_rows_are_never_answered() {
    within("T-P4", async {
        let db = TestDb::start().await;
        let mine = seeded(&db, "tp4mine", 3).await;
        let other = seeded(&db, "tp4other", 4).await;
        let p = pages(&db);
        let b = &mine.branch;

        let got = p.holds(b, None, 50).await.expect("holds");
        assert!(got == newest_first(&mine.state.holds));
        let got = p.attendance(b, None, None, 50).await.expect("attendance");
        assert!(got == newest_first(&mine.state.attendance));
        let got = p
            .attendance(b, Some("e1"), None, 50)
            .await
            .expect("attendance of e1");
        assert!(
            got.iter().all(|a| mine.state.attendance.contains(a)),
            "e1 also works at the other branch; only this branch's records"
        );

        let got = p
            .exceptions_between(b, T0 - DAY, T0 + DAY)
            .await
            .expect("exceptions_between");
        assert!(got == mine.state.exceptions);
        assert_eq!(p.open_holds(b).await.expect("open_holds"), 3);

        // An id of the other branch is not an id this branch holds.
        let foreign_hold = &other.state.holds[3].id;
        let foreign_att = &other.state.attendance[3].id;
        assert!(
            p.holds(b, Some(foreign_hold), 50)
                .await
                .expect("holds")
                .is_empty()
        );
        assert!(
            p.attendance(b, None, Some(foreign_att), 50)
                .await
                .expect("attendance")
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn t_p4_floor_answers_only_its_own_branch() {
    within("T-P4 floor", async {
        let db = TestDb::start().await;
        let mine = db.branch("tp4floor").await;
        let other = db.branch("tp4floorother").await;
        let tag = unique("tp4floor");

        let mut s = state();
        s.phase = Phase::Ready;
        s.custodian = Some("mine".to_string());
        s.attendance = vec![attendance(&format!("{tag}-mine"), "me", 0, Some(MINUTE))];
        store(&db.pool, &mine, &s).await;

        let mut o = state();
        o.phase = Phase::Counting;
        o.custodian = Some("theirs".to_string());
        o.attendance = vec![attendance(&format!("{tag}-other"), "me", 2 * MINUTE, None)];
        store(&db.pool, &other, &o).await;

        let FloorFacts {
            ops,
            my_open_attendance,
        } = pages(&db).floor(&mine, "me").await.expect("floor");
        let ops = ops.expect("this branch has its ops_state row");
        assert_eq!(ops.phase, Phase::Ready);
        assert_eq!(ops.custodian.as_deref(), Some("mine"));
        assert!(
            my_open_attendance.is_none(),
            "me is clocked in at the other branch only"
        );
    })
    .await;
}

// ── T-P5 one employee's attendance ──────────────────────────────────────────

#[tokio::test]
async fn t_p5_attendance_only_e_answers_es_records_only() {
    within("T-P5", async {
        let db = TestDb::start().await;
        let s = seeded(&db, "tp5", 7).await;
        let p = pages(&db);
        let b = &s.branch;

        let of = |e: &str| -> Vec<Attendance> {
            s.state
                .attendance
                .iter()
                .rev()
                .filter(|a| a.employee_id == e)
                .cloned()
                .collect()
        };
        let e1 = of("e1");
        let e2 = of("e2");
        assert_eq!(e1.len(), 4);
        assert_eq!(e2.len(), 3);

        let got = p
            .attendance(b, Some("e1"), None, 50)
            .await
            .expect("attendance of e1");
        assert!(
            got == e1,
            "e1's records, newest first; got {:?}",
            ids(&got, |a| &a.id)
        );
        let got = p
            .attendance(b, Some("e2"), None, 50)
            .await
            .expect("attendance of e2");
        assert!(got == e2, "e2's records; got {:?}", ids(&got, |a| &a.id));

        // with a page: limit and before apply to e's records
        let got = p
            .attendance(b, Some("e1"), None, 2)
            .await
            .expect("first page of e1");
        assert!(got == e1[..2].to_vec());
        let got = p
            .attendance(b, Some("e1"), Some(&e1[1].id), 50)
            .await
            .expect("e1 before its second newest");
        assert!(got == e1[2..].to_vec());

        assert!(
            p.attendance(b, Some("nobody"), None, 50)
                .await
                .expect("attendance of nobody")
                .is_empty()
        );
    })
    .await;
}

// ── T-P7 [from, to) ─────────────────────────────────────────────────────────

#[tokio::test]
async fn t_p7_between_includes_from_and_excludes_to() {
    within("T-P7", async {
        let db = TestDb::start().await;
        let branch = db.branch("tp7").await;
        let tag = unique("tp7");
        let from = 60 * MINUTE;
        let to = 120 * MINUTE;
        // (name, time after T0), stored in this order
        let times = [
            ("before-from", from - 1),
            ("at-from", from),
            ("inside", from + 1),
            ("last-inside", to - 1),
            ("at-to", to),
            ("after-to", to + 1),
        ];
        let mut s = state();
        s.exceptions = times
            .iter()
            .map(|(n, at)| exception(&format!("{tag}-x-{n}"), *at))
            .collect();
        store(&db.pool, &branch, &s).await;
        let p = pages(&db);

        let got = p
            .exceptions_between(&branch, T0 + from, T0 + to)
            .await
            .expect("exceptions_between");
        assert!(
            got == s.exceptions[1..4].to_vec(),
            "exceptions at from, inside, to-1 in position ASC; got {:?}",
            ids(&got, |x| &x.id)
        );

        assert!(
            p.exceptions_between(&branch, T0 + from, T0 + from)
                .await
                .expect("from == to")
                .is_empty(),
            "[from, from) holds nothing"
        );
    })
    .await;
}

/// Ascending means by position: an exception stored later with an earlier time still
/// comes after.
#[tokio::test]
async fn t_p7_between_comes_in_position_order() {
    within("T-P7 order", async {
        let db = TestDb::start().await;
        let branch = db.branch("tp7order").await;
        let tag = unique("tp7order");
        let mut s = state();
        s.exceptions = vec![
            exception(&format!("{tag}-x0"), 30 * MINUTE),
            exception(&format!("{tag}-x1"), 10 * MINUTE),
            exception(&format!("{tag}-x2"), 20 * MINUTE),
        ];
        store(&db.pool, &branch, &s).await;
        let p = pages(&db);

        let got = p
            .exceptions_between(&branch, T0, T0 + DAY)
            .await
            .expect("exceptions_between");
        assert!(got == s.exceptions, "position ASC");
    })
    .await;
}

// ── T-P8 open holds ─────────────────────────────────────────────────────────

#[tokio::test]
async fn t_p8_open_holds_counts_open_holds_of_every_day_and_no_released_one() {
    within("T-P8", async {
        let db = TestDb::start().await;
        let branch = db.branch("tp8").await;
        let other = db.branch("tp8other").await;
        let tag = unique("tp8");

        let mut s = state();
        s.holds = vec![
            hold(&format!("{tag}-open-day-1"), 0, HoldStatus::Open),
            hold(
                &format!("{tag}-released-day-1"),
                MINUTE,
                HoldStatus::Released,
            ),
            hold(&format!("{tag}-open-day-9"), 8 * DAY, HoldStatus::Open),
            hold(
                &format!("{tag}-released-day-30"),
                29 * DAY,
                HoldStatus::Released,
            ),
            hold(&format!("{tag}-open-day-40"), 39 * DAY, HoldStatus::Open),
        ];
        store(&db.pool, &branch, &s).await;

        let mut o = state();
        o.holds = vec![
            hold(&format!("{tag}-other-open"), 0, HoldStatus::Open),
            hold(&format!("{tag}-other-open-2"), DAY, HoldStatus::Open),
        ];
        store(&db.pool, &other, &o).await;

        let p = pages(&db);
        assert_eq!(p.open_holds(&branch).await.expect("open_holds"), 3);
        assert_eq!(p.open_holds(&other).await.expect("open_holds"), 2);
        let empty = db.branch("tp8empty").await;
        assert_eq!(p.open_holds(&empty).await.expect("open_holds"), 0);
    })
    .await;
}

// ── floor ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn floor_answers_the_ops_row_and_my_open_attendance() {
    within("floor", async {
        let db = TestDb::start().await;
        let branch = db.branch("floor").await;
        let tag = unique("floor");
        let mut s = state();
        s.phase = Phase::CutOff;
        s.enabled = true;
        s.custodian = Some("the-custodian".to_string());
        s.cutoff = Some(cutoff("the-custodian"));
        let mine = attendance(&format!("{tag}-me-open"), "me", 3 * MINUTE, None);
        s.attendance = vec![
            attendance(&format!("{tag}-me-old"), "me", 0, Some(MINUTE)),
            mine.clone(),
            attendance(&format!("{tag}-you"), "you", 4 * MINUTE, None),
        ];
        store(&db.pool, &branch, &s).await;

        let facts = pages(&db).floor(&branch, "me").await.expect("floor");
        let ops = facts.ops.expect("the branch has its ops_state row");
        assert!(ops.enabled);
        assert_eq!(ops.phase, Phase::CutOff);
        assert_eq!(ops.custodian.as_deref(), Some("the-custodian"));
        assert!(ops.cutoff.is_some(), "the cutoff is present");
        assert!(
            facts.my_open_attendance == Some(mine),
            "me's latest record, which has no clockOut"
        );

        let facts = pages(&db).floor(&branch, "you").await.expect("floor");
        assert_eq!(
            facts.my_open_attendance.map(|a| a.id),
            Some(format!("{tag}-you"))
        );
        let facts = pages(&db).floor(&branch, "nobody").await.expect("floor");
        assert!(facts.my_open_attendance.is_none());
    })
    .await;
}

#[tokio::test]
async fn floor_answers_no_cutoff_when_the_branch_has_none() {
    within("floor no cutoff", async {
        let db = TestDb::start().await;
        let branch = db.branch("floornocut").await;
        let mut s = state();
        s.enabled = false;
        s.phase = Phase::Ready;
        s.custodian = None;
        s.cutoff = None;
        store(&db.pool, &branch, &s).await;

        let facts = pages(&db).floor(&branch, "me").await.expect("floor");
        let ops = facts.ops.expect("the branch has its ops_state row");
        assert!(!ops.enabled);
        assert_eq!(ops.phase, Phase::Ready);
        assert_eq!(ops.custodian, None);
        assert!(ops.cutoff.is_none(), "no cutoff");
        assert!(facts.my_open_attendance.is_none());
    })
    .await;
}

#[tokio::test]
async fn floor_with_no_ops_state_row_answers_ops_none() {
    within("floor no ops", async {
        let db = TestDb::start().await;
        let branch = db.branch("floornoops").await;
        let facts = pages(&db).floor(&branch, "me").await.expect("floor");
        assert!(facts.ops.is_none(), "a branch never stored has no ops row");
        assert!(facts.my_open_attendance.is_none());
    })
    .await;
}

/// Only the latest record counts: an older one that was never clocked out does not
/// make me clocked in.
#[tokio::test]
async fn floor_when_my_latest_attendance_has_clock_out_answers_none() {
    within("floor latest closed", async {
        let db = TestDb::start().await;
        let branch = db.branch("floorclosed").await;
        let tag = unique("floorclosed");
        let mut s = state();
        s.attendance = vec![
            attendance(&format!("{tag}-me-never-closed"), "me", 0, None),
            attendance(&format!("{tag}-me-latest"), "me", MINUTE, Some(2 * MINUTE)),
            attendance(&format!("{tag}-you-open"), "you", 3 * MINUTE, None),
        ];
        store(&db.pool, &branch, &s).await;

        let facts = pages(&db).floor(&branch, "me").await.expect("floor");
        assert!(facts.ops.is_some());
        assert!(
            facts.my_open_attendance.is_none(),
            "me's latest record has a clockOut"
        );
    })
    .await;
}

/// A `clockOut` that is absent from `data` is open, like a null one.
#[tokio::test]
async fn floor_reads_an_absent_clock_out_as_open() {
    within("floor absent clockOut", async {
        let db = TestDb::start().await;
        let branch = db.branch("floorabsent").await;
        store(&db.pool, &branch, &state()).await;
        let id = unique("floorabsent");
        sqlx::query(
            "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
             VALUES ($1, $2, 0, 'me', $3)",
        )
        .bind(&id)
        .bind(branch.as_str())
        .bind(json!({ "role": "FRONT", "clockIn": T0 }))
        .execute(&db.pool)
        .await
        .expect("insert an attendance row without clockOut");

        let facts = pages(&db).floor(&branch, "me").await.expect("floor");
        assert!(
            facts.my_open_attendance == Some(attendance(&id, "me", 0, None)),
            "no clockOut key = still clocked in"
        );
    })
    .await;
}

// ── T-P10 floor reads no history ────────────────────────────────────────────
//
// How rows read are counted, and why it is deterministic.
//
// Each test has its own schema (`TestDb::start`), so `pg_stat_user_tables` filtered
// on that schema sees only this test's tables. Its `seq_tup_read` (rows a
// sequential scan read) plus `idx_tup_fetch` (rows an index scan fetched from the
// table) is the number of rows read from a table.
//
// Those counters are kept per backend and published to the shared statistics only
// at a flush, normally at most once a second, so a plain before/after read is
// timing-dependent. Here every statement — the seed, `load_state`, `floor` and the
// two readings — runs on ONE connection (a pool of one), and the readings are each
// preceded by `SELECT pg_stat_force_next_flush()` on that same connection
// (PostgreSQL 15+): the backend then publishes everything it counted when that
// statement ends, before answering. Nothing else reads these tables in between,
// `ANALYZE` (which does not count as a scan) runs before the first reading so the
// planner sees 10,001 rows and autovacuum has no reason to analyse them again.
// Wall time is not measured.
//
// The seed: 10,000 closed attendance records of `me`, then me's open record at
// the newest position. The design's query
// (`… WHERE branch_id = $1 AND employee_id = $2 ORDER BY position DESC LIMIT 1`)
// fetches that one row through `UNIQUE (branch_id, position)`; `ops_state` is one
// primary-key row. Reading the history (a scan of `attendance`) shows up as
// thousands.

const TABLES: [&str; 5] = [
    "ops_state",
    "attendance",
    "customer_holds",
    "tx_drafts",
    "cash_exceptions",
];

/// A pool of exactly one connection to the test's schema.
async fn one_connection(schema: &str) -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let options = PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL");
    PgPoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .acquire_timeout(MUST_FINISH)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(options.options([("search_path", schema)]))
        .await
        .expect("connect one connection to the test schema")
}

/// Rows read so far from each custody table, as published after a forced flush of
/// this connection's own counters.
async fn rows_read(one: &PgPool, schema: &str) -> BTreeMap<String, i64> {
    sqlx::query("SELECT pg_stat_force_next_flush()")
        .execute(one)
        .await
        .expect("pg_stat_force_next_flush (PostgreSQL 15 or later)");
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT relname::text, COALESCE(seq_tup_read, 0) + COALESCE(idx_tup_fetch, 0) \
         FROM pg_stat_user_tables WHERE schemaname = $1",
    )
    .bind(schema)
    .fetch_all(one)
    .await
    .expect("read pg_stat_user_tables");
    let rows: BTreeMap<String, i64> = rows.into_iter().collect();
    for table in TABLES {
        assert!(rows.contains_key(table), "no statistics row for {table}");
    }
    rows
}

#[tokio::test]
async fn t_p10_floor_reads_one_ops_row_and_at_most_one_attendance_row() {
    within("T-P10", async {
        let db = TestDb::start().await;
        let branch = db.branch("tp10").await;
        let one = one_connection(&db.schema).await;
        let tag = unique("tp10");

        let mut s = state();
        s.cutoff = Some(cutoff("custodian"));
        store(&one, &branch, &s).await;

        sqlx::query(
            "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
             SELECT $1 || '-a' || g, $2, g, 'me', \
                    jsonb_build_object('role', 'FRONT', 'clockIn', $3::bigint + g * 60000, \
                                       'clockOut', $3::bigint + g * 60000 + 30000) \
             FROM generate_series(0, 9999) AS g",
        )
        .bind(&tag)
        .bind(branch.as_str())
        .bind(T0)
        .execute(&one)
        .await
        .expect("seed 10,000 closed attendance records");
        let open = attendance(&format!("{tag}-open"), "me", 10_000 * MINUTE, None);
        sqlx::query(
            "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
             VALUES ($1, $2, 10000, 'me', $3)",
        )
        .bind(&open.id)
        .bind(branch.as_str())
        .bind(json!({ "role": "FRONT", "clockIn": T0 + 10_000 * MINUTE, "clockOut": null }))
        .execute(&one)
        .await
        .expect("seed me's open record at the newest position");

        // the seed decodes: what load_state reads is what was written
        let loaded = load(&one, &branch).await;
        assert_eq!(loaded.attendance.len(), 10_001);
        assert_eq!(loaded.attendance.last(), Some(&open));
        assert!(
            loaded.attendance[..10_000]
                .iter()
                .all(|a| a.clock_out.is_some())
        );

        for table in TABLES {
            sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
                .execute(&one)
                .await
                .expect("analyze");
        }

        let p = PgCustodyPages::new(one.clone());
        let before = rows_read(&one, &db.schema).await;
        let facts = p.floor(&branch, "me").await.expect("floor");
        let after = rows_read(&one, &db.schema).await;

        let ops = facts.ops.expect("the ops_state row");
        assert_eq!(ops.phase, Phase::Open);
        assert!(ops.enabled);
        assert_eq!(ops.custodian.as_deref(), Some("custodian"));
        assert!(ops.cutoff.is_some());
        assert!(facts.my_open_attendance == Some(open), "me's open record");

        let read = |table: &str| after[table] - before[table];
        assert!(
            read("ops_state") <= 1,
            "floor read {} ops_state rows; it reads the one row",
            read("ops_state")
        );
        assert!(
            read("attendance") <= 1,
            "floor read {} attendance rows of 10,001; it reads at most me's latest",
            read("attendance")
        );
        for table in &TABLES[2..] {
            assert_eq!(read(table), 0, "floor read {table}, which it never needs");
        }
    })
    .await;
}
