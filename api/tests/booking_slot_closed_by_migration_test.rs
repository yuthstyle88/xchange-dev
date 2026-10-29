//! #5392 BK-15 (design `docs/design/blueprint/booking-slots.md` §4.1, §8 M3, §9.1 row BK-15): schema M3,
//! `booking_slots.closed_by`. Written before the implementation by the ticket author (claude/studio-claude-16).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - one `api/migrations/*_booking_slot_closed_by.sql`, after `main`'s newest at writing (`20261226130000`), with a
//!   `-- down:` section (the name must not end in `_booking_slots.sql`, which M1's test reserves);
//! - rows that exist before M3, OPEN and CLOSED, keep `closed_by` NULL (no backfill, owner's choice A);
//! - `closed_by` takes only `SLOT`, `CLOSURE`, `RULE` or NULL, and is NULL unless `status = 'CLOSED'`;
//! - `down` drops the column and both checks, and loses nothing else.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod quote_common;

use std::path::{Path, PathBuf};

use quote_common::{ORG_1, TestDb};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

const SUFFIX: &str = "_booking_slot_closed_by.sql";
/// `main`'s newest migration when these tests were written.
const MAINS_NEWEST_AT_WRITING: i64 = 20261226130000;
const MAIN: &str = "MAIN";
/// PostgreSQL's SQLSTATE for a CHECK violation.
const CHECK_VIOLATION: &str = "23514";

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

fn versions() -> Vec<(i64, String)> {
    let mut v: Vec<(i64, String)> = std::fs::read_dir(migrations())
        .expect("read api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .filter(|n| n.ends_with(".sql"))
        .map(|n| {
            (
                n[..n.find('_').expect("<version>_")]
                    .parse()
                    .expect("numeric"),
                n,
            )
        })
        .collect();
    v.sort();
    v
}

/// (version, text) of the one M3.
fn the_file() -> (i64, String) {
    let found: Vec<(i64, String)> = versions()
        .into_iter()
        .filter(|(_, n)| n.ends_with(SUFFIX))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{SUFFIX} (design §8 M3): {found:?}"
    );
    let (version, name) = found[0].clone();
    let text = std::fs::read_to_string(migrations().join(&name)).expect("read the migration");
    (version, text)
}

fn version_before(v: i64) -> i64 {
    versions()
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| *n < v)
        .max()
        .expect("a migration before M3")
}

/// The `-- down:` section, uncommented (the convention of M1, `20261205131000_booking_slots.sql`).
fn down_sql(text: &str) -> String {
    let mut lines = text.lines();
    assert!(
        lines.any(|l| l.trim() == "-- down:"),
        "the file carries a `-- down:` line"
    );
    let mut out = String::new();
    for l in lines {
        let t = l.trim_start();
        let body = t.strip_prefix("--").unwrap_or(t);
        out.push_str(body.strip_prefix(' ').unwrap_or(body));
        out.push('\n');
    }
    assert!(!out.trim().is_empty(), "the down section is not empty");
    out
}

async fn has_column(db: &TestDb) -> bool {
    db.count(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'booking_slots' AND column_name = 'closed_by'",
    )
    .await
        == 1
}

/// An empty future slot of MAIN with `status`; the id.
async fn slot(db: &TestDb, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    let offset = (id.as_u128() % 1_000_000) as i64;
    db.exec(&format!(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, \
             reserved, status) \
         SELECT '{id}', '{ORG_1}'::uuid, '{MAIN}', s::date, s, s + interval '30 minutes', 2, 0, '{status}' \
         FROM (SELECT now() + interval '1 day' + make_interval(secs => {offset})) AS t(s)"
    ))
    .await;
    id
}

fn set(id: Uuid, status: &str, closed_by: &str) -> String {
    format!(
        "UPDATE booking_slots SET status = '{status}', closed_by = {closed_by} WHERE id = '{id}'"
    )
}

#[test]
fn m3_sorts_after_mains_newest_and_carries_a_down_section() {
    let (version, text) = the_file();
    assert!(
        version > MAINS_NEWEST_AT_WRITING,
        "M3 {version} sorts after {MAINS_NEWEST_AT_WRITING}"
    );
    down_sql(&text);
}

#[tokio::test]
async fn rows_before_m3_keep_closed_by_null() {
    let (version, _) = the_file();
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(version))).await;
    let open = slot(&db, "OPEN").await;
    let closed = slot(&db, "CLOSED").await;

    db.migrate_through(None).await;

    assert!(has_column(&db).await, "M3 adds booking_slots.closed_by");
    for id in [open, closed] {
        let by = db
            .text(&format!(
                "SELECT closed_by FROM booking_slots WHERE id = '{id}'"
            ))
            .await;
        assert_eq!(
            by, None,
            "slot {id} from before M3 keeps closed_by NULL (no backfill)"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn closed_by_takes_only_the_three_reasons_and_only_on_a_closed_slot() {
    let db = TestDb::start().await;
    let s = slot(&db, "OPEN").await;

    assert_eq!(
        db.outcome(&set(s, "CLOSED", "'OTHER'")).await,
        CHECK_VIOLATION,
        "an unknown reason"
    );
    assert_eq!(
        db.outcome(&set(s, "OPEN", "'SLOT'")).await,
        CHECK_VIOLATION,
        "a reason on an OPEN slot"
    );
    for reason in ["'SLOT'", "'CLOSURE'", "'RULE'", "NULL"] {
        assert_eq!(
            db.outcome(&set(s, "CLOSED", reason)).await,
            "ok",
            "CLOSED with {reason}"
        );
    }
    assert_eq!(
        db.outcome(&set(s, "OPEN", "NULL")).await,
        "ok",
        "OPEN with NULL"
    );
    db.finish().await;
}

#[tokio::test]
async fn down_drops_the_column_and_its_checks_and_keeps_the_rows() {
    let (_, text) = the_file();
    let db = TestDb::start().await;
    let s = slot(&db, "OPEN").await;
    db.exec(&set(s, "CLOSED", "'CLOSURE'")).await;

    sqlx::raw_sql(AssertSqlSafe(down_sql(&text)))
        .execute(&db.pool)
        .await
        .expect("down applies");

    assert!(!has_column(&db).await, "down drops closed_by");
    let status = db
        .text(&format!(
            "SELECT status FROM booking_slots WHERE id = '{s}'"
        ))
        .await;
    assert_eq!(
        status.as_deref(),
        Some("CLOSED"),
        "the row stays, with its status"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM pg_constraint \
             WHERE conrelid = 'booking_slots'::regclass AND pg_get_constraintdef(oid) LIKE '%closed_by%'"
        )
        .await,
        0,
        "no check names closed_by after down"
    );
    db.finish().await;
}
