//! CA-4b: With no scanner attached, `MacScanner::start()` returns without blocking and `probe()`
//! answers `ScanFailure::NoScanner` within 5 s via the `ICDeviceBrowser` callback on the main
//! thread's `CFRunLoop` (no timer fallback). Also tests whole-word cover-open matching.

#![cfg(target_os = "macos")]

use std::process::Command;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use xc_counter::scan::mac::{MacScanner, is_cover_open_text};
use xc_counter::scan::{ScanFailure, ScanRequest, Scanner};

const CHILD_ENV: &str = "XC_MAC_SCANNER_MAIN_THREAD_TEST";

unsafe extern "C" {
    fn atexit(cb: extern "C" fn()) -> i32;
}

extern "C" fn run_on_os_main_thread() {
    assert!(
        MainThreadMarker::new().is_some(),
        "must drive CFRunLoop on the OS main thread"
    );

    let started = Instant::now();
    let scanner = Arc::new(MacScanner::start());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "MacScanner::start() must not block"
    );

    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn({
        let scanner = Arc::clone(&scanner);
        move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("tokio runtime");
            let res = rt.block_on(async move {
                let probe = tokio::time::timeout(Duration::from_secs(5), scanner.probe())
                    .await
                    .expect("probe() answered within 5 s");
                let scan = tokio::time::timeout(
                    Duration::from_secs(5),
                    scanner.scan(ScanRequest::passport()),
                )
                .await
                .expect("scan() answered within 5 s");
                (probe, scan)
            });
            let _ = done_tx.send(res);
        }
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    let (probe_res, scan_res) = loop {
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.02, true);
        if let Ok(res) = done_rx.try_recv() {
            break res;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for main-thread ICDeviceBrowser callback");
        }
    };

    assert_eq!(probe_res, Err(ScanFailure::NoScanner));
    assert_eq!(scan_res, Err(ScanFailure::NoScanner));
}

#[test]
fn start_does_not_block_and_probe_answers_no_scanner_within_5_s() {
    if std::env::var_os(CHILD_ENV).is_some() {
        // Register to run when `libtest::main` returns on the OS main thread.
        unsafe {
            assert_eq!(atexit(run_on_os_main_thread), 0);
        }
        return;
    }

    let status = Command::new(std::env::current_exe().expect("test binary path"))
        .env(CHILD_ENV, "1")
        .arg("--exact")
        .arg("start_does_not_block_and_probe_answers_no_scanner_within_5_s")
        .arg("--nocapture")
        .status()
        .expect("spawns child test process on main thread");
    assert!(status.success(), "main-thread CFRunLoop test failed");
}

#[test]
fn cover_open_matches_whole_words_only() {
    assert!(!is_cover_open_text("Could not open: invalid device"));
    assert!(!is_cover_open_text("discover"));
    assert!(!is_cover_open_text("Failed to discover open scanner"));
    assert!(is_cover_open_text("Scanner cover is open"));
    assert!(is_cover_open_text("Scanner lid open"));
}
