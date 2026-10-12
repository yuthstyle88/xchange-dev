//! BK-2 #1379 — `PgSlotRepository`'s capacity (design `docs/design/blueprint/booking-slots.md` B3, B4, §4.3; §9.1
//! row BK-2: *"50 concurrent reserves on a slot of capacity 1 → exactly 1 succeeds, 49 `CAPACITY_FULL`,
//! `reserved = 1`; move between two slots under concurrency never leaves `reserved` off by one"*; ticket §8, §9).
//! Written before the implementation by the test writer (claude/dell-claude-4, on the BE lane controller
//! claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `slot_common`); nothing here skips.

mod slot_common;

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use slot_common::{TestDb, status_text};
use tokio::sync::Barrier;
use uuid::Uuid;
use xc_booking::domain::slot::repository::{SlotMove, SlotRepository};
use xc_booking::domain::slot::slot_error::SlotError;
use xc_kernel::ids::BranchId;

const RACERS: usize = 50;
const MOVERS_EACH_WAY: usize = 20;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 7, hour, 0, 0)
        .single()
        .expect("an instant")
}

async fn branch(db: &TestDb) -> BranchId {
    db.add_branch("Asia/Bangkok", None).await
}

async fn reserve_and_commit(db: &TestDb, slot: Uuid) -> Result<(), SlotError> {
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    let out = repo.reserve_in(&mut tx, slot).await;
    match out {
        Ok(_) => {
            tx.commit().await.expect("commit");
            Ok(())
        }
        Err(e) => {
            tx.rollback().await.expect("rollback");
            Err(e)
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn fifty_concurrent_reserves_on_the_last_place_give_it_to_exactly_one() {
    let db = TestDb::start_with(RACERS as u32 + 1).await;
    let b = branch(&db).await;
    let slot = db.add_slot(&b, at(3), 30, 1, 0).await;
    let gate = Arc::new(Barrier::new(RACERS));
    let mut racers = Vec::new();
    for _ in 0..RACERS {
        let pool = db.pool.clone();
        let repo = db.repo();
        let gate = gate.clone();
        racers.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            gate.wait().await;
            match repo.reserve_in(&mut tx, slot).await {
                Ok(s) => {
                    tx.commit().await.expect("commit");
                    Ok(s.reserved)
                }
                Err(e) => {
                    tx.rollback().await.expect("rollback");
                    Err(e)
                }
            }
        }));
    }
    let mut won = 0;
    let mut full = 0;
    let mut other = Vec::new();
    for r in racers {
        match r.await.expect("the racer ran") {
            Ok(reserved) => {
                assert_eq!(reserved, 1, "the winner sees the one place taken");
                won += 1;
            }
            Err(SlotError::CapacityFull) => full += 1,
            Err(e) => other.push(format!("{e:?}")),
        }
    }
    assert!(other.is_empty(), "only CAPACITY_FULL refusals: {other:?}");
    assert_eq!(won, 1, "exactly one reserve succeeds");
    assert_eq!(full, RACERS - 1, "the other 49 are CAPACITY_FULL");
    assert_eq!(
        db.slot_row(slot).await,
        (1, 1, "OPEN".to_owned()),
        "reserved = 1"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_reserve_takes_one_place_and_answers_the_updated_slot() {
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let slot = db.add_slot(&b, at(3), 30, 3, 1).await;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    let s = repo.reserve_in(&mut tx, slot).await.expect("a free place");
    tx.commit().await.expect("commit");
    assert_eq!(s.id, slot);
    assert_eq!(s.branch_id, b);
    assert_eq!((s.capacity, s.reserved, status_text(&s)), (3, 2, "OPEN"));
    assert_eq!(s.start_at, at(3));
    assert_eq!(db.slot_row(slot).await, (3, 2, "OPEN".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn a_full_slot_is_capacity_full_and_unchanged() {
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let slot = db.add_slot(&b, at(3), 30, 2, 2).await;
    let err = reserve_and_commit(&db, slot).await.expect_err("full");
    assert!(matches!(err, SlotError::CapacityFull), "{err:?}");
    assert_eq!(err.code(), "CAPACITY_FULL");
    assert_eq!(db.slot_row(slot).await, (2, 2, "OPEN".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn a_closed_slot_is_capacity_full_and_unchanged() {
    // B3: the reserve's UPDATE matches only `status = 'OPEN'`; no row back = CAPACITY_FULL.
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let slot = db.add_slot(&b, at(3), 30, 2, 0).await;
    db.exec(&format!(
        "UPDATE booking_slots SET status = 'CLOSED' WHERE id = '{slot}'"
    ))
    .await;
    let err = reserve_and_commit(&db, slot).await.expect_err("closed");
    assert!(matches!(err, SlotError::CapacityFull), "{err:?}");
    assert_eq!(db.slot_row(slot).await, (2, 0, "CLOSED".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn a_release_gives_one_place_back_and_a_release_of_nothing_is_stale() {
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let slot = db.add_slot(&b, at(3), 30, 2, 1).await;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    let s = repo.release_in(&mut tx, slot).await.expect("a held place");
    assert_eq!(s.reserved, 0);
    let err = repo
        .release_in(&mut tx, slot)
        .await
        .expect_err("nothing left to release");
    assert!(matches!(err, SlotError::StaleVersion), "{err:?}");
    assert_eq!(err.code(), "STALE_VERSION");
    tx.commit().await.expect("commit");
    assert_eq!(db.slot_row(slot).await, (2, 0, "OPEN".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn a_move_takes_a_place_in_the_new_slot_and_gives_one_back_in_the_old() {
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let from = db.add_slot(&b, at(3), 30, 2, 1).await;
    let to = db.add_slot(&b, at(4), 30, 2, 0).await;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    let moved = repo.move_in(&mut tx, from, to).await.expect("a free place");
    tx.commit().await.expect("commit");
    match moved {
        SlotMove::Moved { from: f, to: t } => {
            assert_eq!((f.id, f.reserved), (from, 0));
            assert_eq!((t.id, t.reserved), (to, 1));
        }
        SlotMove::Unchanged => panic!("two different slots move"),
    }
    assert_eq!(db.slot_row(from).await.1, 0);
    assert_eq!(db.slot_row(to).await.1, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_move_to_a_full_slot_is_capacity_full_and_moves_nothing() {
    // The caller commits even after the refusal: move_in itself must have changed nothing (reserve before release).
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let from = db.add_slot(&b, at(3), 30, 2, 1).await;
    let to = db.add_slot(&b, at(4), 30, 1, 1).await;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    let err = repo.move_in(&mut tx, from, to).await.expect_err("full");
    assert!(matches!(err, SlotError::CapacityFull), "{err:?}");
    tx.commit().await.expect("commit");
    assert_eq!(db.slot_row(from).await, (2, 1, "OPEN".to_owned()));
    assert_eq!(db.slot_row(to).await, (1, 1, "OPEN".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn a_move_to_the_same_slot_is_unchanged() {
    // B4: the same slot is `unchanged`, even when it is full.
    let db = TestDb::start().await;
    let b = branch(&db).await;
    let slot = db.add_slot(&b, at(3), 30, 1, 1).await;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    let moved = repo.move_in(&mut tx, slot, slot).await.expect("unchanged");
    tx.commit().await.expect("commit");
    assert_eq!(moved, SlotMove::Unchanged);
    assert_eq!(db.slot_row(slot).await, (1, 1, "OPEN".to_owned()));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_moves_both_ways_never_leave_reserved_off_by_one() {
    // 20 moves A→B and 20 moves B→A at once. Every move either commits both halves or refuses with
    // CAPACITY_FULL / STALE_VERSION (never a deadlock or another error: B4 locks in ascending id order), and the
    // final counts are exactly the starting counts moved by the committed moves.
    let db = TestDb::start_with(2 * MOVERS_EACH_WAY as u32 + 1).await;
    let b = branch(&db).await;
    let a = db.add_slot(&b, at(3), 30, 4, 2).await;
    let c = db.add_slot(&b, at(4), 30, 4, 2).await;
    let gate = Arc::new(Barrier::new(2 * MOVERS_EACH_WAY));
    let mut movers = Vec::new();
    for i in 0..2 * MOVERS_EACH_WAY {
        let (from, to) = if i % 2 == 0 { (a, c) } else { (c, a) };
        let pool = db.pool.clone();
        let repo = db.repo();
        let gate = gate.clone();
        movers.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            gate.wait().await;
            let out = repo.move_in(&mut tx, from, to).await;
            match out {
                Ok(m) => {
                    tx.commit().await.expect("commit");
                    (from == a, Ok(m))
                }
                Err(e) => {
                    tx.rollback().await.expect("rollback");
                    (from == a, Err(e))
                }
            }
        }));
    }
    let (mut a_to_c, mut c_to_a) = (0i32, 0i32);
    let mut other = Vec::new();
    for m in movers {
        let (from_a, out) = m.await.expect("the mover ran");
        match out {
            Ok(SlotMove::Moved { .. }) if from_a => a_to_c += 1,
            Ok(SlotMove::Moved { .. }) => c_to_a += 1,
            Ok(SlotMove::Unchanged) => other.push("Unchanged for two different slots".to_owned()),
            Err(SlotError::CapacityFull | SlotError::StaleVersion) => {}
            Err(e) => other.push(format!("{e:?}")),
        }
    }
    assert!(
        other.is_empty(),
        "only CAPACITY_FULL / STALE_VERSION refusals: {other:?}"
    );
    let (_, ra, _) = db.slot_row(a).await;
    let (_, rc, _) = db.slot_row(c).await;
    assert_eq!(ra + rc, 4, "no place is created or lost");
    assert_eq!(
        ra,
        2 - a_to_c + c_to_a,
        "A moved by exactly the committed moves"
    );
    assert_eq!(
        rc,
        2 + a_to_c - c_to_a,
        "B moved by exactly the committed moves"
    );
    assert!(a_to_c + c_to_a > 0, "at least one move committed");
    db.finish().await;
}
