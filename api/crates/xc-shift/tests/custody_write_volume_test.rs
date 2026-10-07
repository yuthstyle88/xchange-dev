//! What one custody command writes (ticket #105).
//!
//! A command stores the branch's custody state. It must write the rows that are
//! new or that changed and leave every other row alone: a branch's drafts,
//! requests, signatures, holds and exceptions only ever grow, so a command that
//! rewrites them all costs more with every day the branch has lived.
//!
//! "Left alone" is measured with PostgreSQL's `xmin`: the id of the transaction
//! that wrote the row version. A row that is written again — also with the very
//! same values — gets a new one.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`). Without it they FAIL; none skips.

mod custody_common;
mod custody_service_common;
mod shift_service_common;

use std::collections::{BTreeMap, BTreeSet};

use custody_service_common::{Desk, rows, totals};
use serde_json::json;
use shift_service_common::TestDb;

const TABLES: [&str; 7] = [
    "ops_state",
    "attendance",
    "custody_requests",
    "custody_signatures",
    "customer_holds",
    "tx_drafts",
    "cash_exceptions",
];

/// `(table, row key)` → `xmin`, for every custody row of the desk's branch.
type RowVersions = BTreeMap<(&'static str, String), String>;

async fn row_versions(db: &TestDb, d: &Desk) -> RowVersions {
    let mut versions = RowVersions::new();
    for table in TABLES {
        let sql = match table {
            "ops_state" => "SELECT branch_id, xmin::text FROM ops_state WHERE branch_id = $1",
            "attendance" => "SELECT id, xmin::text FROM attendance WHERE branch_id = $1",
            "custody_requests" => {
                "SELECT id, xmin::text FROM custody_requests WHERE branch_id = $1"
            }
            "custody_signatures" => {
                "SELECT s.request_id || '#' || s.position, s.xmin::text \
                 FROM custody_signatures s JOIN custody_requests r ON r.id = s.request_id \
                 WHERE r.branch_id = $1"
            }
            "customer_holds" => "SELECT id, xmin::text FROM customer_holds WHERE branch_id = $1",
            "tx_drafts" => "SELECT id, xmin::text FROM tx_drafts WHERE branch_id = $1",
            _ => "SELECT id, xmin::text FROM cash_exceptions WHERE branch_id = $1",
        };
        let found: Vec<(String, String)> = sqlx::query_as(sql)
            .bind(d.branch.as_str())
            .fetch_all(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("{table}: {e}"));
        for (key, xmin) in found {
            versions.insert((table, key), xmin);
        }
    }
    versions
}

/// What one command did to the rows: written again, added, removed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Written {
    rewritten: BTreeSet<(&'static str, String)>,
    added: BTreeSet<(&'static str, String)>,
    removed: BTreeSet<(&'static str, String)>,
}

fn written(before: &RowVersions, after: &RowVersions) -> Written {
    let mut w = Written::default();
    for (row, xmin) in after {
        match before.get(row) {
            None => {
                w.added.insert(row.clone());
            }
            Some(was) if was != xmin => {
                w.rewritten.insert(row.clone());
            }
            Some(_) => {}
        }
    }
    for row in before.keys() {
        if !after.contains_key(row) {
            w.removed.insert(row.clone());
        }
    }
    w
}

fn only(rows: &[(&'static str, &str)]) -> BTreeSet<(&'static str, String)> {
    rows.iter().map(|(t, k)| (*t, (*k).to_owned())).collect()
}

fn count(versions: &RowVersions, table: &str) -> usize {
    versions.keys().filter(|(t, _)| *t == table).count()
}

/// A branch that has lived a little: everyone clocked in, the safe counted in,
/// the drawer opened ten dollars short of its target (a cash exception), one
/// transfer finished, one transfer waiting for its receiver, a customer on
/// hold, three drafts. Returns the desk and the id of the waiting transfer.
async fn a_branch_with_history(db: &TestDb) -> (Desk, String) {
    let mut d = Desk::with_safe(db).await;
    let drawer = [("THB", 100_000), ("USD", 1_990)];
    let open = d
        .propose(
            "F1",
            "OPEN",
            json!({
                "counted": totals(&drawer), "denominations": rows(&drawer),
                "note": "ten dollars short of the target",
            }),
        )
        .await;
    d.ok("BE1", "opsCommit", json!({ "id": open })).await;
    d.ok(
        "BE1",
        "opsStartService",
        json!({ "equipmentReady": true, "ratesVerified": true, "rateUpdated": "2026-10-06T03:00:00.000Z" }),
    )
    .await;

    let done = d
        .propose(
            "F1",
            "SAFE_IN",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": "300.25" }),
        )
        .await;
    d.ok("BE1", "opsDispatch", json!({ "id": done })).await;
    d.ok("FIN1", "opsSign", json!({ "id": done })).await;
    d.ok("BE1", "opsCommit", json!({ "id": done })).await;

    d.outside.add_customer("CX-00000001");
    d.ok(
        "MON1",
        "opsHold",
        json!({ "customerId": "CX-00000001", "reason": "check" }),
    )
    .await;
    for amount in [1, 2, 3] {
        d.ok(
            "F1",
            "opsBeginTransaction",
            json!({ "type": "BUY", "currency": "USD", "amount": amount }),
        )
        .await;
    }

    let waiting = d
        .propose(
            "F1",
            "SAFE_IN",
            json!({ "receiverId": "FIN1", "currency": "USD", "amount": 50 }),
        )
        .await;
    d.ok("BE1", "opsDispatch", json!({ "id": waiting })).await;
    (d, waiting)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_command_writes_the_rows_it_changes_and_no_others() {
    let db = TestDb::start().await;
    let (mut d, waiting) = a_branch_with_history(&db).await;

    let before = row_versions(&db, &d).await;
    // the history is really there: a test on empty tables would prove nothing
    for (table, at_least) in [
        ("ops_state", 1),
        ("attendance", 8),
        ("custody_requests", 4),
        ("custody_signatures", 6),
        ("customer_holds", 1),
        ("tx_drafts", 3),
        ("cash_exceptions", 1),
    ] {
        assert!(
            count(&before, table) >= at_least,
            "{table}: {} rows, expected at least {at_least}",
            count(&before, table)
        );
    }

    // 1. The receiver signs the waiting transfer: one signature row is added.
    //    The request's own row holds nothing a signature changes, so it is
    //    not written; nothing else is either.
    d.ok("FIN1", "opsSign", json!({ "id": waiting })).await;
    let after_sign = row_versions(&db, &d).await;
    let w = written(&before, &after_sign);
    assert_eq!(w.rewritten, only(&[]), "rows written again by one opsSign");
    assert_eq!(w.added.len(), 1, "rows added by one opsSign: {:?}", w.added);
    assert!(
        w.added
            .iter()
            .all(|(table, key)| *table == "custody_signatures"
                && key.starts_with(&format!("{waiting}#"))),
        "{:?}",
        w.added
    );
    assert!(w.removed.is_empty(), "{:?}", w.removed);

    // 1b. Backend commits it: that request's row (it is completed now) and the
    //     branch's row (the safe holds fifty dollars more). No other request,
    //     no signature of another request, no draft, hold or exception.
    d.ok("BE1", "opsCommit", json!({ "id": waiting })).await;
    let after_commit = row_versions(&db, &d).await;
    let w = written(&after_sign, &after_commit);
    assert_eq!(
        w.rewritten,
        only(&[
            ("custody_requests", &waiting),
            ("ops_state", d.branch.as_str())
        ]),
        "rows written again by one opsCommit"
    );
    assert!(
        w.added
            .iter()
            .all(|(table, key)| *table == "custody_signatures"
                && key.starts_with(&format!("{waiting}#"))),
        "rows added by one opsCommit: {:?}",
        w.added
    );
    assert!(w.removed.is_empty(), "{:?}", w.removed);
    let after_sign = after_commit;

    // 2. One employee clocks out: that attendance row, and nothing else.
    let state = d.custody.state(&d.branch).await.unwrap();
    let f2 = state
        .attendance
        .iter()
        .find(|a| a.employee_id == "F2")
        .expect("F2 clocked in")
        .id
        .clone();
    d.ok("F2", "opsClock", json!({ "direction": "OUT" })).await;
    let after_clock = row_versions(&db, &d).await;
    let w = written(&after_sign, &after_clock);
    assert_eq!(
        w,
        Written {
            rewritten: only(&[("attendance", &f2)]),
            ..Written::default()
        },
        "rows written by one opsClock"
    );

    // 3. A new draft: one row added, no existing row written again.
    let draft = d
        .ok(
            "F1",
            "opsBeginTransaction",
            json!({ "type": "SELL", "currency": "USD", "amount": 4 }),
        )
        .await["id"]
        .as_str()
        .expect("the draft's id")
        .to_owned();
    let after_draft = row_versions(&db, &d).await;
    let w = written(&after_clock, &after_draft);
    assert_eq!(
        w,
        Written {
            added: only(&[("tx_drafts", &draft)]),
            ..Written::default()
        },
        "rows written by one opsBeginTransaction"
    );

    db.drop_schema().await;
}

/// The number of rows a command writes does not grow with what the branch
/// already holds: the same command after twenty more drafts writes as few.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_a_command_writes_does_not_grow_with_the_history() {
    let db = TestDb::start().await;
    let (mut d, _waiting) = a_branch_with_history(&db).await;

    for round in 0..2 {
        if round == 1 {
            for amount in 10..30 {
                d.ok(
                    "F1",
                    "opsBeginTransaction",
                    json!({ "type": "BUY", "currency": "USD", "amount": amount }),
                )
                .await;
            }
        }
        d.outside.add_customer(&format!("CX-0000001{round}"));
        let before = row_versions(&db, &d).await;
        d.ok(
            "MON1",
            "opsHold",
            json!({ "customerId": format!("CX-0000001{round}"), "reason": "check" }),
        )
        .await;
        let w = written(&before, &row_versions(&db, &d).await);
        assert!(
            w.rewritten.is_empty(),
            "round {round}: a new hold wrote {} existing rows again: {:?}",
            w.rewritten.len(),
            w.rewritten
        );
        assert_eq!(w.added.len(), 1, "round {round}: {:?}", w.added);
        assert!(w.added.iter().all(|(table, _)| *table == "customer_holds"));
    }

    db.drop_schema().await;
}

// ── what is stored can be read back ─────────────────────────────────────────

/// Amounts are kept in the smallest unit as `i128`. Whatever the repository
/// is given to store it reads back: a figure beyond 64 bits is neither
/// dropped on the way in nor a failure of the whole command.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_amount_beyond_64_bits_is_stored_and_read_back() {
    use xc_kernel::money::MoneyDelta;
    use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
    use xc_shift::domain::custody::value_objects::phase::Phase;
    use xc_shift::domain::shift::value_objects::balances::Balances;
    use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
    use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

    let db = TestDb::start().await;
    let repo = PgCustodyRepository::new(db.pool.clone());
    let usd = shift_service_common::code("USD");
    let thb = shift_service_common::code("THB");

    for (label, minor) in [
        ("the largest 64-bit figure", i64::MAX as i128),
        ("one more than 64 bits hold", i64::MAX as i128 + 1),
        ("far beyond", 12_345_678_901_234_567_890_123_i128),
    ] {
        let branch = db.branch("wide").await;
        let before = repo.load_state(&branch).await.expect("an empty state");
        let mut after = before.clone();
        after.enabled = true;
        after.phase = Phase::Closed;
        let amount = MoneyDelta::from_minor(minor)
            .to_money()
            .expect("a positive amount");
        after.safe = Some(Balances::of(&[(usd, amount), (thb, amount)]));

        let mut tx = db.pool.begin().await.expect("begin");
        repo.store_state_in(&mut tx, &branch, &before, &after)
            .await
            .unwrap_or_else(|e| panic!("{label}: storing failed: {e:?}"));
        tx.commit().await.expect("commit");

        let read = repo.load_state(&branch).await.expect("load");
        assert_eq!(read.safe, after.safe, "{label}");
        assert_eq!(read, after, "{label}");
    }

    // and a difference (signed) the same way, inside a request
    let (mut d, waiting) = a_branch_with_history(&db).await;
    let before = d.custody.state(&d.branch).await.unwrap();
    let mut after = before.clone();
    let wide = MoneyDelta::from_minor(-(i64::MAX as i128) - 2);
    let request = after
        .requests
        .iter_mut()
        .find(|r| r.id == waiting)
        .expect("the waiting transfer");
    request.variance = Some(StockDelta::of(&[(usd, wide)]));
    let mut tx = db.pool.begin().await.expect("begin");
    repo.store_state_in(&mut tx, &d.branch, &before, &after)
        .await
        .expect("storing a wide difference");
    tx.commit().await.expect("commit");
    assert_eq!(d.custody.state(&d.branch).await.unwrap(), after);
    // the desk still works on it
    d.ok("FIN1", "opsSign", json!({ "id": waiting })).await;

    db.drop_schema().await;
}

/// A count of notes is a whole number in what the screens send and in what is
/// stored. `5.0` is not what any writer produces: it is refused when sent, and
/// every stored count is a JSON integer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_count_is_a_whole_number_when_sent_and_when_stored() {
    let db = TestDb::start().await;
    let mut d = Desk::with_safe(&db).await;
    let state = d.custody.state(&d.branch).await.unwrap();

    let drawer = [("THB", 100_000), ("USD", 2_000)];
    for count in [json!(20.0), json!(20.5), json!("20"), json!(-1)] {
        let p = d.proposal(
            "OPEN",
            json!({
                "counted": totals(&drawer),
                "denominations": [
                    { "currency": "THB", "denomination": 100, "count": 1000 },
                    { "currency": "USD", "denomination": 100, "count": count },
                ],
            }),
        );
        let got = d.act("F1", "opsPropose", p).await;
        assert!(
            matches!(got, Err(pos_error::AppError::Domain(_))),
            "a count sent as {count} must be refused, got {got:?}"
        );
    }
    assert_eq!(
        d.custody.state(&d.branch).await.unwrap(),
        state,
        "a refused proposal stores nothing"
    );

    // the same sheet with whole numbers is accepted …
    d.propose(
        "F1",
        "OPEN",
        json!({ "counted": totals(&drawer), "denominations": rows(&drawer) }),
    )
    .await;
    // … and every count the repository wrote is a JSON integer
    let counts: Vec<(String, String)> = sqlx::query_as(
        "SELECT jsonb_typeof(row -> 'count'), row ->> 'count' \
         FROM custody_requests r, \
              jsonb_array_elements(r.data -> 'counted' -> 'denominations') AS row \
         WHERE r.branch_id = $1",
    )
    .bind(d.branch.as_str())
    .fetch_all(&db.pool)
    .await
    .expect("the stored counts");
    assert!(counts.len() >= 4, "{counts:?}");
    for (kind, text) in &counts {
        assert_eq!(kind, "number");
        assert!(
            !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()),
            "a stored count reads {text:?}"
        );
    }

    db.drop_schema().await;
}

/// The first thing ever stored for a branch may be a list item alone. The
/// branch's own row is written with it, or the state would load as empty.
/// And an item that changed its place is never silently left where it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_first_store_and_a_moved_item_are_not_lost() {
    use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
    use xc_shift::domain::custody::state::CustodyState;
    use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

    let db = TestDb::start().await;
    let repo = PgCustodyRepository::new(db.pool.clone());
    let (d, _waiting) = a_branch_with_history(&db).await;
    let lived = d.custody.state(&d.branch).await.unwrap();

    // the lists of a lived branch under fresh ids, and none of the branch's own values
    let mut after = CustodyState::initial();
    after.drafts = lived.drafts.clone();
    after.holds = lived.holds.clone();
    after.attendance = lived.attendance.clone();
    for id in after
        .drafts
        .iter_mut()
        .map(|x| &mut x.id)
        .chain(after.holds.iter_mut().map(|x| &mut x.id))
        .chain(after.attendance.iter_mut().map(|x| &mut x.id))
    {
        id.push_str("-copy");
    }
    assert!(after.drafts.len() >= 3 && !after.holds.is_empty() && after.attendance.len() >= 8);

    let fresh = db.branch("fresh").await;
    let before = repo.load_state(&fresh).await.expect("nothing stored yet");
    assert_eq!(before, CustodyState::initial());
    let mut tx = db.pool.begin().await.expect("begin");
    repo.store_state_in(&mut tx, &fresh, &before, &after)
        .await
        .expect("the first store");
    tx.commit().await.expect("commit");
    assert_eq!(repo.load_state(&fresh).await.expect("load"), after);

    // two drafts change places: stored so, or refused — never accepted and dropped
    let mut moved = after.clone();
    moved.drafts.swap(0, 2);
    let mut tx = db.pool.begin().await.expect("begin");
    match repo.store_state_in(&mut tx, &fresh, &after, &moved).await {
        Ok(()) => {
            tx.commit().await.expect("commit");
            assert_eq!(repo.load_state(&fresh).await.expect("load"), moved);
        }
        Err(_) => {
            drop(tx);
            assert_eq!(repo.load_state(&fresh).await.expect("load"), after);
        }
    }

    db.drop_schema().await;
}
