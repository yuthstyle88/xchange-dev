//! Row AG-5 (`docs/design/xchange-v3.md` §5.4c *Restart and offline*): the agent's journal.
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        "xc-agent-journal-{}-{nanos}-{n}",
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

#[test]
fn what_is_owed_survives_reopening_oldest_first() {
    let path = scratch_file();
    let now = SystemTime::now();
    {
        let mut j = Journal::open(path.clone()).expect("open");
        j.sending("j1", "l1", now).expect("sending j1");
        j.sending("j2", "l2", now).expect("sending j2");
        j.outcome(&report("j2", "l2", Outcome::DrawerOpened), now)
            .expect("outcome j2");
    }
    let j = Journal::open(path).expect("reopen");
    assert_eq!(
        j.pending(now),
        vec![
            sending("j1", "l1"),
            Pending::Unacknowledged(report("j2", "l2", Outcome::DrawerOpened)),
        ],
        "a send with no outcome, then an outcome not acknowledged"
    );
}

#[test]
fn an_acknowledged_lease_is_not_owed() {
    let path = scratch_file();
    let now = SystemTime::now();
    let mut j = Journal::open(path.clone()).expect("open");
    j.sending("j1", "l1", now).expect("sending");
    j.outcome(&report("j1", "l1", Outcome::Failed), now)
        .expect("outcome");
    j.acknowledged("j1", "l1").expect("acknowledged");
    j.sending("j2", "l2", now).expect("sending");
    assert_eq!(j.pending(now), vec![sending("j2", "l2")]);
    assert_eq!(
        Journal::open(path).expect("reopen").pending(now),
        vec![sending("j2", "l2")],
        "on disk too"
    );
}

/// A crash in the middle of a write leaves a torn last line: it is ignored, the lines before
/// it are kept, and the journal can be written again.
#[test]
fn a_torn_last_line_is_ignored() {
    let path = scratch_file();
    let now = SystemTime::now();
    Journal::open(path.clone())
        .expect("open")
        .sending("j1", "l1", now)
        .expect("sending");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("the journal is a file at its path")
        .write_all(b"{\"kind\":\"outc")
        .expect("tear the last line");
    let mut j = Journal::open(path.clone()).expect("a torn line is not an error");
    assert_eq!(j.pending(now), vec![sending("j1", "l1")]);
    j.sending("j2", "l2", now)
        .expect("write after the torn line");
    assert_eq!(
        Journal::open(path).expect("reopen").pending(now),
        vec![sending("j1", "l1"), sending("j2", "l2")]
    );
}

/// An outcome is reported again for 24 hours, then no longer owed. A send with no outcome is
/// always owed (it is answered `AGENT_RESTARTED` at once).
#[test]
fn an_outcome_is_owed_for_a_day() {
    let path = scratch_file();
    let then = SystemTime::now() - Duration::from_secs(3 * 24 * 3600);
    let mut j = Journal::open(path).expect("open");
    j.sending("j1", "l1", then).expect("sending");
    j.outcome(&report("j1", "l1", Outcome::DrawerOpened), then)
        .expect("outcome");
    j.sending("j2", "l2", then).expect("sending");
    let day = Duration::from_secs(24 * 3600);
    assert_eq!(
        j.pending(then + day - Duration::from_secs(1)).len(),
        2,
        "within the day"
    );
    assert_eq!(
        j.pending(then + day + Duration::from_secs(1)),
        vec![sending("j2", "l2")]
    );
}

#[cfg(unix)]
#[test]
fn the_journal_file_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let path = scratch_file();
    Journal::open(path.clone())
        .expect("open")
        .sending("j1", "l1", SystemTime::now())
        .expect("sending");
    let mode = std::fs::metadata(&path)
        .expect("the journal exists at its path")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
}
