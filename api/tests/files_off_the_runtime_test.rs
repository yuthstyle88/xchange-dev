//! `FileService` does its disk work off the async runtime's thread.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Each test runs the service on a current-thread runtime — what an actix-web
//! worker runs: one thread for every connection that worker holds. A disk call
//! made on that thread stops every other task on it until the call returns.
//!
//! - Writing: while an upload of 15 MB is being written, another task on the same
//!   thread must get to run and see the upload's temporary file.
//! - Reading: the file at `<dir>/<key>` is a named pipe whose writer is another
//!   task on the same thread. A read made on that thread waits for a writer that
//!   can never run, so the test reports it after a bounded wait instead of hanging.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.
#![cfg(any(target_os = "linux", target_os = "macos"))]

mod files_common;

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use files_common::*;

/// `O_NONBLOCK`: opening a pipe's writing end fails at once while nobody reads it.
#[cfg(target_os = "linux")]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(target_os = "macos")]
const O_NONBLOCK: i32 = 0x0004;
/// `ENXIO`: what that open answers while nobody reads (6 on both systems).
const ENXIO: i32 = 6;

/// How long a read of a few bytes may take once the database is ready.
const A_READ_FINISHES: Duration = Duration::from_secs(10);

const KEY: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

// ------------------------------------------------------------------- writing

/// Whether the upload directory holds a temporary file of an upload in progress.
fn a_write_in_progress(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with(".tmp-")),
        Err(_) => false,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn other_tasks_run_while_an_upload_is_written() {
    let db = TestDb::start().await;
    let files = db.files();
    let mut bytes = vec![0u8; MAX_UPLOAD_BYTES];
    bytes[..JPEG.len()].copy_from_slice(JPEG);

    // The temporary file exists from the moment the bytes start until the rename.
    // A watcher on this thread can see it only if the write lets the thread go.
    let mut seen = false;
    for attempt in 0..5 {
        let done = Arc::new(AtomicBool::new(false));
        let watcher = tokio::spawn({
            let dir: PathBuf = db.dir.clone();
            let done = done.clone();
            async move {
                loop {
                    if a_write_in_progress(&dir) {
                        return true;
                    }
                    if done.load(Ordering::SeqCst) {
                        return false;
                    }
                    tokio::task::yield_now().await;
                }
            }
        });
        files
            .upload(&branch(MAIN), OWNER, "image/jpeg", &bytes, at(attempt))
            .await
            .expect("a 15 MB JPEG is accepted");
        done.store(true, Ordering::SeqCst);
        if watcher.await.expect("the watcher does not panic") {
            seen = true;
            break;
        }
    }
    assert!(
        seen,
        "in 5 uploads of 15 MB no other task ran while the bytes were written: \
         the write holds the runtime's thread"
    );
    db.finish().await;
}

// ------------------------------------------------------------------- reading

#[derive(Clone, Copy, Debug)]
enum Way {
    Owner,
    Branch,
}

enum Step {
    /// The database and the pipe are ready; the read starts now.
    Ready,
    /// What the read handed out.
    Read(Option<Vec<u8>>),
}

fn make_pipe(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo {}", path.display());
}

/// Opens the pipe's writing end without ever blocking this thread: retries while
/// nobody has the reading end open.
async fn open_writer(pipe: &Path) -> std::fs::File {
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(pipe)
        {
            Ok(file) => return file,
            Err(e) if e.raw_os_error() == Some(ENXIO) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(e) => panic!("open the writing end of {}: {e}", pipe.display()),
        }
    }
}

/// Reads the file `KEY` the given way, on a current-thread runtime of its own,
/// while another task of that runtime is the only writer of the pipe at
/// `<dir>/KEY`. Returns the bytes handed out, or panics when the read did not
/// finish in time.
fn read_while_another_task_writes(way: Way) -> Option<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime");
        runtime.block_on(async move {
            let db = TestDb::start().await;
            let files = db.files();
            db.seed_row(KEY, MAIN, OWNER, "image/jpeg").await;
            std::fs::create_dir_all(&db.dir).expect("create the upload directory");
            let pipe = db.dir.join(KEY);
            make_pipe(&pipe);
            tx.send(Step::Ready).expect("the test is waiting");

            let reader = tokio::spawn(async move {
                match way {
                    Way::Owner => files.open_owned(KEY, OWNER).await,
                    Way::Branch => files.open_in_branch(KEY, &branch(MAIN)).await,
                }
            });
            let mut writer = open_writer(&pipe).await;
            writer.write_all(JPEG).expect("write to the pipe");
            drop(writer);
            let got = reader
                .await
                .expect("the read does not panic")
                .expect("the read succeeds")
                .map(|f: StoredFile| f.bytes);
            db.finish().await;
            let _ = tx.send(Step::Read(got));
        });
    });

    match rx.recv_timeout(MUST_FINISH * 2) {
        Ok(Step::Ready) => {}
        Ok(Step::Read(_)) => unreachable!("Ready is sent first"),
        Err(RecvTimeoutError::Timeout) => panic!("the database was not ready in time"),
        Err(RecvTimeoutError::Disconnected) => {
            panic!("setting up failed before the read; see the panic above")
        }
    }
    match rx.recv_timeout(A_READ_FINISHES) {
        Ok(Step::Read(got)) => got,
        Ok(Step::Ready) => unreachable!("Ready is sent once"),
        Err(RecvTimeoutError::Timeout) => panic!(
            "{way:?}: the read did not finish in {A_READ_FINISHES:?} — it holds the runtime's \
             thread, so the task writing the file never ran"
        ),
        Err(RecvTimeoutError::Disconnected) => {
            panic!("{way:?}: the read failed; see the panic above")
        }
    }
}

#[test]
fn other_tasks_run_while_the_owner_reads_a_file() {
    assert_eq!(
        read_while_another_task_writes(Way::Owner),
        Some(JPEG.to_vec())
    );
}

#[test]
fn other_tasks_run_while_staff_read_a_file() {
    assert_eq!(
        read_while_another_task_writes(Way::Branch),
        Some(JPEG.to_vec())
    );
}
