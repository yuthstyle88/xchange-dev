//! Ticket #2389 (§5.4c *Restart and offline*): repairing a valid last journal line
//! that lost its trailing newline before a crash.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use xc_device_agent::client::{Outcome, Report};
use xc_device_agent::journal::{Journal, Pending};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_file() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "xc-agent-journal-newline-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir.join("journal.jsonl")
}

fn report(job: &str, lease: &str, outcome: Outcome) -> Report {
    Report {
        job_id: job.to_owned(),
        lease_id: lease.to_owned(),
        outcome,
        reason: if outcome == Outcome::DrawerOpened {
            String::new()
        } else {
            "SEND_FAILED".to_owned()
        },
        detail: json!({}),
    }
}

fn sending(job: &str, lease: &str) -> Pending {
    Pending::Sending {
        job_id: job.to_owned(),
        lease_id: lease.to_owned(),
    }
}

/// Row 1: `sending j1` complete JSON with no trailing `\n` -> after `open`, one more
/// `sending(j2)`, and reopen, `pending` = `[Sending j1, Sending j2]`.
#[test]
fn row_1_sending_without_trailing_newline_survives_next_write_and_reopen() {
    let path = scratch_file();
    let now = SystemTime::now();
    Journal::open(path.clone())
        .expect("open")
        .sending("j1", "l1", now)
        .expect("sending j1");

    let mut raw = std::fs::read(&path).expect("read journal");
    assert_eq!(raw.pop(), Some(b'\n'), "strip trailing newline");
    std::fs::write(&path, &raw).expect("write journal without trailing newline");

    let mut j = Journal::open(path.clone()).expect("open after crash before newline");
    assert_eq!(j.pending(now), vec![sending("j1", "l1")]);

    j.sending("j2", "l2", now).expect("sending j2");

    let reopened = Journal::open(path.clone()).expect("reopen after second write");
    assert_eq!(
        reopened.pending(now),
        vec![sending("j1", "l1"), sending("j2", "l2")]
    );
    assert!(
        std::fs::read(&path)
            .expect("read repaired journal")
            .ends_with(b"\n")
    );
}

/// Row 2: `sending j1\n` + `outcome j1` complete JSON without `\n` -> the outcome is kept
/// (`Unacknowledged j1`) and `Sending j2` is owed; nothing lost.
#[test]
fn row_2_outcome_without_trailing_newline_is_kept_and_next_sending_is_owed() {
    let path = scratch_file();
    let now = SystemTime::now();
    {
        let mut j = Journal::open(path.clone()).expect("open");
        j.sending("j1", "l1", now).expect("sending j1");
        j.outcome(&report("j1", "l1", Outcome::DrawerOpened), now)
            .expect("outcome j1");
    }

    let mut raw = std::fs::read(&path).expect("read journal");
    assert_eq!(raw.pop(), Some(b'\n'), "strip final newline after outcome");
    std::fs::write(&path, &raw).expect("write journal without final newline");

    let mut j = Journal::open(path.clone()).expect("open after crash before outcome newline");
    assert_eq!(
        j.pending(now),
        vec![Pending::Unacknowledged(report(
            "j1",
            "l1",
            Outcome::DrawerOpened
        ))]
    );

    j.sending("j2", "l2", now).expect("sending j2");

    let reopened = Journal::open(path).expect("reopen after second write");
    assert_eq!(
        reopened.pending(now),
        vec![
            Pending::Unacknowledged(report("j1", "l1", Outcome::DrawerOpened)),
            sending("j2", "l2"),
        ]
    );
}

/// Row 3: `sending j1\n` + a torn `{"kind":"outc` -> `[Sending j1, Sending j2]`.
#[test]
fn row_3_torn_unparseable_last_line_is_truncated_and_next_sending_is_kept() {
    let path = scratch_file();
    let now = SystemTime::now();
    Journal::open(path.clone())
        .expect("open")
        .sending("j1", "l1", now)
        .expect("sending j1");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open journal for append")
        .write_all(b"{\"kind\":\"outc")
        .expect("append torn line");

    let mut j = Journal::open(path.clone()).expect("open with torn last line");
    assert_eq!(j.pending(now), vec![sending("j1", "l1")]);
    j.sending("j2", "l2", now).expect("sending j2");

    let reopened = Journal::open(path).expect("reopen");
    assert_eq!(
        reopened.pending(now),
        vec![sending("j1", "l1"), sending("j2", "l2")]
    );
}

/// Row 4: empty file -> `[Sending j2]`.
#[test]
fn row_4_empty_file_stays_empty_on_open_and_records_sending() {
    let path = scratch_file();
    let now = SystemTime::now();
    std::fs::write(&path, b"").expect("create empty journal file");

    let mut j = Journal::open(path.clone()).expect("open empty journal");
    assert!(
        std::fs::read(&path)
            .expect("read empty journal after open")
            .is_empty(),
        "empty journal must remain empty after open"
    );
    assert_eq!(j.pending(now), Vec::<Pending>::new());

    j.sending("j2", "l2", now).expect("sending j2");

    let reopened = Journal::open(path).expect("reopen");
    assert_eq!(reopened.pending(now), vec![sending("j2", "l2")]);
}

/// Row 5: well-formed file ending in `\n` -> unchanged bytes before the new line.
#[test]
fn row_5_well_formed_file_ending_in_newline_keeps_unchanged_bytes_before_new_line() {
    let path = scratch_file();
    let now = SystemTime::now();
    Journal::open(path.clone())
        .expect("open")
        .sending("j1", "l1", now)
        .expect("sending j1");

    let before = std::fs::read(&path).expect("read well-formed journal");
    assert!(before.ends_with(b"\n"));

    let mut j = Journal::open(path.clone()).expect("reopen well-formed journal");
    assert_eq!(
        std::fs::read(&path).expect("read after open"),
        before,
        "well-formed journal bytes must not change on open"
    );

    j.sending("j2", "l2", now).expect("sending j2");

    let after = std::fs::read(&path).expect("read after second write");
    assert!(
        after.starts_with(&before),
        "bytes before the new line must be unchanged"
    );

    let reopened = Journal::open(path).expect("reopen");
    assert_eq!(
        reopened.pending(now),
        vec![sending("j1", "l1"), sending("j2", "l2")]
    );
}
