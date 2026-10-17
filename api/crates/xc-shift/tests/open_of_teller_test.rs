//! CA-10p (design `docs/design/blueprint/cash-shift-transfer.md` §4.7 @ `4ed83b68`): the per-teller shift reads
//! `ShiftRepository::open_of_teller` / `open_of_teller_in` and the per-branch list `open_of_branch`.
//!
//! Tests written before the implementation (ticket CA-10p) — do not edit or delete.
//!
//! Every shift is seeded by SQL, so these tests pin the reads and nothing about how a shift opens. Two OPEN
//! shifts at one branch need `opened_by_count` set until CA-10's M3 (the v2 per-branch index binds only
//! marker-less shifts); CA-10 drops that column from these seeds as a setup-only edit.

mod shift_service_common;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use shift_service_common::{TestDb, at, unique};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::entities::open_shift_row::OpenShiftRow;
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

async fn org(db: &TestDb) -> Uuid {
    sqlx::query_scalar("SELECT id FROM organizations WHERE code = 'XC'")
        .fetch_one(&db.pool)
        .await
        .expect("the XC organisation")
}

/// A user named `name`; answers its id.
async fn user(db: &TestDb, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) VALUES ($1, $2, $3, 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(org(db).await)
    .bind(name)
    .execute(&db.pool)
    .await
    .expect("insert a user");
    id
}

/// How a seeded shift is marked.
#[derive(Clone, Copy)]
enum Opened {
    /// v2's path: no `opened_by_count`.
    V2,
    /// A verified OPEN count: `opened_by_count` names a `cash_counts` row.
    ByCount,
}

/// A shift of `teller` (or of nobody) at `branch`; answers its id.
async fn shift(
    db: &TestDb,
    branch: &BranchId,
    teller: Option<Uuid>,
    status: &str,
    opened_at: DateTime<Utc>,
    opened: Opened,
) -> String {
    let id = unique("SH");
    let count: Option<Uuid> = match opened {
        Opened::V2 => None,
        Opened::ByCount => {
            let count = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
                 VALUES ($1, $2, $3, 'OPEN', $4, now(), $5)",
            )
            .bind(count)
            .bind(org(db).await)
            .bind(branch.as_str())
            .bind(teller.unwrap_or_else(Uuid::new_v4))
            .bind(format!("idem-{count}"))
            .execute(&db.pool)
            .await
            .expect("insert the OPEN count");
            Some(count)
        }
    };
    let _ = count;
    let closed = status == "CLOSED";
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id, \
             closed_at, closed_by) \
         VALUES ($1, $2, $3, $4, 'test', $1, $5, CASE WHEN $6 THEN $4 + interval '1 hour' END, \
             CASE WHEN $6 THEN 'test' END)",
    )
    .bind(&id)
    .bind(branch.as_str())
    .bind(status)
    .bind(opened_at)
    .bind(teller)
    .bind(closed)
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("insert a {status} shift: {e}"));
    id
}

fn repo(db: &TestDb) -> PgShiftRepository {
    PgShiftRepository::new(db.pool.clone())
}

fn internal_naming<T: std::fmt::Debug>(res: AppResult<T>, what: &str) {
    match res {
        Err(AppError::Internal(msg)) => {
            assert!(msg.contains(what), "the fault names {what}: {msg}")
        }
        other => panic!("expected AppError::Internal naming {what}, got {other:?}"),
    }
}

// ── open_of_teller ──────────────────────────────────────────────────────────

/// Row 1: the teller's OPEN shift is found by teller, on whichever branch it is (A8: across branches).
#[tokio::test]
async fn open_of_teller_finds_the_tellers_open_shift_on_any_branch() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let airport = db.branch("AIR").await;
    let ploy = user(&db, "Ploy").await;
    let nok = user(&db, "Nok").await;
    shift(&db, &main, Some(nok), "OPEN", at(0, 0), Opened::V2).await;
    let ploys = shift(
        &db,
        &airport,
        Some(ploy),
        "OPEN",
        at(10, 0),
        Opened::ByCount,
    )
    .await;

    let found = repo(&db)
        .open_of_teller(&ploy.to_string())
        .await
        .expect("a read, not a fault")
        .expect("Ploy has an OPEN shift");
    assert_eq!(found.id, ploys);
    assert_eq!(found.branch_id, airport);
    db.drop_schema().await;
}

/// Row 2: no OPEN shift (only a CLOSED one, or none at all) is `None`.
#[tokio::test]
async fn open_of_teller_is_none_without_an_open_shift() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let ploy = user(&db, "Ploy").await;
    shift(&db, &main, Some(ploy), "CLOSED", at(0, 0), Opened::ByCount).await;
    assert_eq!(
        repo(&db).open_of_teller(&ploy.to_string()).await.unwrap(),
        None
    );
    assert_eq!(
        repo(&db)
            .open_of_teller(&Uuid::new_v4().to_string())
            .await
            .unwrap(),
        None,
        "a teller with no shift at all"
    );
    db.drop_schema().await;
}

/// Row 3: the `_in` form answers the same in the caller's transaction, and sees a shift written in it.
#[tokio::test]
async fn open_of_teller_in_reads_in_the_callers_transaction() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let ploy = user(&db, "Ploy").await;
    let mut tx = db.pool.begin().await.unwrap();
    let id = unique("SH");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'test', $1, $3)",
    )
    .bind(&id)
    .bind(main.as_str())
    .bind(ploy)
    .execute(&mut *tx)
    .await
    .unwrap();
    let found = repo(&db)
        .open_of_teller_in(&mut tx, &ploy.to_string())
        .await
        .unwrap()
        .expect("the uncommitted shift, read in its own transaction");
    assert_eq!(found.id, id);
    tx.rollback().await.unwrap();
    db.drop_schema().await;
}

/// Row 4: two OPEN shifts for one teller is a fault naming the teller, never "the first one".
/// (Possible until M3: a v2-opened and a count-opened shift of the same person.)
#[tokio::test]
async fn two_open_shifts_for_one_teller_is_a_fault() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let airport = db.branch("AIR").await;
    let ploy = user(&db, "Ploy").await;
    shift(&db, &main, Some(ploy), "OPEN", at(0, 0), Opened::V2).await;
    shift(&db, &airport, Some(ploy), "OPEN", at(5, 0), Opened::ByCount).await;
    internal_naming(
        repo(&db).open_of_teller(&ploy.to_string()).await,
        &ploy.to_string(),
    );
    db.drop_schema().await;
}

// ── open_of_branch ──────────────────────────────────────────────────────────

/// Row 5: every OPEN shift of the branch with its teller's name, by `opened_at` then shift id; CLOSED shifts and
/// other branches' shifts are not in it.
#[tokio::test]
async fn open_of_branch_lists_the_open_shifts_with_their_tellers_in_order() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let airport = db.branch("AIR").await;
    let ploy = user(&db, "Ploy").await;
    let nok = user(&db, "Nok").await;
    let krit = user(&db, "Krit").await;
    let later = shift(&db, &main, Some(ploy), "OPEN", at(20, 0), Opened::ByCount).await;
    let earlier = shift(&db, &main, Some(nok), "OPEN", at(10, 0), Opened::ByCount).await;
    shift(&db, &main, Some(krit), "CLOSED", at(0, 0), Opened::ByCount).await;
    shift(&db, &airport, Some(krit), "OPEN", at(5, 0), Opened::ByCount).await;

    let rows = repo(&db)
        .open_of_branch(&main)
        .await
        .expect("a read, not a fault");
    assert_eq!(
        rows,
        vec![
            OpenShiftRow {
                shift_id: earlier,
                teller_id: nok.to_string(),
                teller_name: "Nok".to_string(),
                opened_at: at(10, 0),
            },
            OpenShiftRow {
                shift_id: later,
                teller_id: ploy.to_string(),
                teller_name: "Ploy".to_string(),
                opened_at: at(20, 0),
            },
        ]
    );
    db.drop_schema().await;
}

/// Row 5, the tie: two shifts opened at the same instant are ordered by shift id.
#[tokio::test]
async fn open_of_branch_breaks_an_opened_at_tie_by_shift_id() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let ploy = user(&db, "Ploy").await;
    let nok = user(&db, "Nok").await;
    let a = shift(&db, &main, Some(ploy), "OPEN", at(10, 0), Opened::ByCount).await;
    let b = shift(&db, &main, Some(nok), "OPEN", at(10, 0), Opened::ByCount).await;
    let mut expected = vec![a, b];
    expected.sort();
    let ids: Vec<String> = repo(&db)
        .open_of_branch(&main)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.shift_id)
        .collect();
    assert_eq!(ids, expected);
    db.drop_schema().await;
}

/// Row 6: no OPEN shift is an empty list.
#[tokio::test]
async fn open_of_branch_is_empty_without_an_open_shift() {
    let db = TestDb::start().await;
    let main = db.branch("MAIN").await;
    let ploy = user(&db, "Ploy").await;
    shift(&db, &main, Some(ploy), "CLOSED", at(0, 0), Opened::ByCount).await;
    assert_eq!(repo(&db).open_of_branch(&main).await.unwrap(), Vec::new());
    db.drop_schema().await;
}

// Row 7 (an OPEN shift with no teller, or a teller that is no user) has no test on purpose: `shifts.teller_id` is
// NOT NULL and references `users` (`api/migrations/20261012100000_cash.sql:6`, `:115`), so the row cannot be stored.
