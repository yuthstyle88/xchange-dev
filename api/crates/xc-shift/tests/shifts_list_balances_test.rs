//! `PgShiftRepository::shifts` reads every shift's balances in one query instead of one query per shift
//! (`api/crates/xc-shift/src/infrastructure/shift/shift_repository.rs:363-396`, the N+1). Written before the
//! implementation by claude/d7aa64 (BE controller claude/7ca80c); do not edit, delete or weaken.
//!
//! The risk of the change is mixing rows between shifts when they are read together, so these pins give several
//! shifts different balances and require each to come back with its own. They are green on main and must stay so.

mod shift_service_common;

use std::collections::BTreeMap;

use shift_service_common::{TestDb, shown_shift};
use xc_kernel::ids::BranchId;

async fn any_user(db: &TestDb) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_one(&db.pool)
        .await
        .expect("TestDb::start seeds the openers")
}

/// `(code, opening, expected, counted, difference)` as text.
type BalanceRow<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, Option<&'a str>);

/// A shift row with `balances` = `(code, opening, expected, counted, difference)`.
async fn seed_shift(
    db: &TestDb,
    branch: &BranchId,
    id: &str,
    status: &str,
    balances: &[BalanceRow<'_>],
) {
    let teller = any_user(db).await;
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, close_note, \
                             open_request_id, teller_id) \
         VALUES ($1, $2, $3, now(), 'owner', \
                 CASE WHEN $3 = 'CLOSED' THEN now() END, CASE WHEN $3 = 'CLOSED' THEN 'owner' END, '', $1, $4)",
    )
    .bind(id)
    .bind(branch.as_str())
    .bind(status)
    .bind(teller)
    .execute(&db.pool)
    .await
    .expect("insert a shift");
    for (code, opening, expected, counted, difference) in balances {
        sqlx::query(
            "INSERT INTO shift_balances (shift_id, code, opening, expected, counted, difference) \
             VALUES ($1, $2, $3::numeric, $4::numeric, $5::numeric, $6::numeric)",
        )
        .bind(id)
        .bind(code)
        .bind(opening)
        .bind(expected)
        .bind(counted)
        .bind(difference)
        .execute(&db.pool)
        .await
        .expect("insert a shift balance");
    }
}

fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[tokio::test]
async fn each_listed_shift_carries_its_own_balances() {
    let db = TestDb::start().await;
    let branch = db.branch("n1").await;
    seed_shift(
        &db,
        &branch,
        "S-1",
        "CLOSED",
        &[
            ("THB", "1000.00", "1500.00", Some("1490.00"), Some("-10.00")),
            ("USD", "100.00", "80.00", Some("80.00"), Some("0.00")),
        ],
    )
    .await;
    seed_shift(
        &db,
        &branch,
        "S-2",
        "CLOSED",
        &[("EUR", "50.00", "70.00", Some("75.00"), Some("5.00"))],
    )
    .await;
    seed_shift(
        &db,
        &branch,
        "S-3",
        "OPEN",
        &[("THB", "2000.00", "2100.00", None, None)],
    )
    .await;

    let shifts = db.service().shifts(&branch).await.expect("list shifts");
    let ids: Vec<&str> = shifts.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["S-1", "S-2", "S-3"], "order by seq, as today");

    let s1 = shown_shift(&shifts[0]);
    assert_eq!(s1.opening, m(&[("THB", "1000.00"), ("USD", "100.00")]));
    assert_eq!(s1.expected, m(&[("THB", "1500.00"), ("USD", "80.00")]));
    assert_eq!(s1.counted, Some(m(&[("THB", "1490.00"), ("USD", "80.00")])));
    assert_eq!(s1.difference, Some(m(&[("THB", "-10.00")])));

    let s2 = shown_shift(&shifts[1]);
    assert_eq!(
        s2.opening,
        m(&[("EUR", "50.00")]),
        "S-2 must not see S-1's rows"
    );
    assert_eq!(s2.expected, m(&[("EUR", "70.00")]));
    assert_eq!(s2.counted, Some(m(&[("EUR", "75.00")])));
    assert_eq!(s2.difference, Some(m(&[("EUR", "5.00")])));

    let s3 = shown_shift(&shifts[2]);
    assert_eq!(s3.opening, m(&[("THB", "2000.00")]));
    assert_eq!(s3.expected, m(&[("THB", "2100.00")]));
    assert_eq!(s3.counted, None, "an open shift has no count");
    assert_eq!(s3.difference, None);

    db.drop_schema().await;
}

#[tokio::test]
async fn a_shift_with_no_balance_rows_lists_empty_balances() {
    let db = TestDb::start().await;
    let branch = db.branch("n1e").await;
    seed_shift(&db, &branch, "S-E1", "CLOSED", &[]).await;
    seed_shift(
        &db,
        &branch,
        "S-E2",
        "OPEN",
        &[("THB", "10.00", "10.00", None, None)],
    )
    .await;

    let shifts = db.service().shifts(&branch).await.expect("list shifts");
    let e1 = shown_shift(&shifts[0]);
    assert!(e1.opening.is_empty() && e1.expected.is_empty(), "{e1:?}");
    assert_eq!(
        e1.counted,
        Some(BTreeMap::new()),
        "a closed shift's count is present, and empty"
    );
    assert_eq!(shown_shift(&shifts[1]).opening, m(&[("THB", "10.00")]));

    db.drop_schema().await;
}

#[tokio::test]
async fn another_branchs_shifts_and_balances_are_not_listed() {
    let db = TestDb::start().await;
    let a = db.branch("n1a").await;
    let b = db.branch("n1b").await;
    seed_shift(
        &db,
        &a,
        "S-A",
        "CLOSED",
        &[("THB", "1.00", "1.00", Some("1.00"), None)],
    )
    .await;
    seed_shift(
        &db,
        &b,
        "S-B",
        "CLOSED",
        &[("THB", "9.00", "9.00", Some("9.00"), None)],
    )
    .await;

    let shifts = db.service().shifts(&a).await.expect("list shifts");
    assert_eq!(shifts.len(), 1);
    assert_eq!(shifts[0].id, "S-A");
    assert_eq!(shown_shift(&shifts[0]).opening, m(&[("THB", "1.00")]));

    db.drop_schema().await;
}
