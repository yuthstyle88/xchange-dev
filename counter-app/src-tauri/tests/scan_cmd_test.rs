//! CA-4a (design §3, §8.6): `FakeScanner` through the mapping behind the `scan_passport` command,
//! with no running Tauri app. Written before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! | Scanner answers | `run_scan` answers |
//! |---|---|
//! | JPEG bytes | `Ok(bytes)` |
//! | `NoScanner`, `Timeout`, `CoverOpen`, `Device(_)`, `Busy` | `Err(reason)` |
//! | any | the request sent is `ScanRequest::passport()` |

use std::sync::Arc;

use xc_counter::bridge::scan_cmd::run_scan;
use xc_counter::scan::fake::FakeScanner;
use xc_counter::scan::{ScanFailure, ScanRequest};

#[tokio::test]
async fn a_scan_answers_the_bytes_with_the_passport_request() {
    let scanner = FakeScanner::new(vec![Ok(vec![0xFF, 0xD8])]);
    assert_eq!(run_scan(&scanner).await, Ok(vec![0xFF, 0xD8]));
    assert_eq!(scanner.requests(), vec![ScanRequest::passport()]);
}

#[tokio::test]
async fn each_failure_answers_its_reason() {
    for (failure, reason) in [
        (ScanFailure::Timeout, "timeout"),
        (ScanFailure::CoverOpen, "coverOpen"),
        (ScanFailure::Device("jam".to_owned()), "device"),
        (ScanFailure::Busy, "busy"),
    ] {
        let scanner = FakeScanner::new(vec![Err(failure)]);
        assert_eq!(run_scan(&scanner).await, Err(reason));
    }
    let scanner = FakeScanner::new(vec![]);
    assert_eq!(run_scan(&scanner).await, Err("noScanner"));
}

#[tokio::test]
async fn a_scan_in_flight_makes_the_next_answer_busy() {
    let scanner = Arc::new(FakeScanner::new(vec![Ok(vec![1])]));
    let hold = scanner.hold_next();
    let first = tokio::spawn({
        let scanner = Arc::clone(&scanner);
        async move { run_scan(scanner.as_ref()).await }
    });
    while !scanner.in_flight() {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    assert_eq!(run_scan(scanner.as_ref()).await, Err("busy"));
    hold.release();
    assert_eq!(first.await.expect("joined"), Ok(vec![1]));
}
