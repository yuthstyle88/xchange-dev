//! CA-4a (design §6): `FakeScanner`. Scripted answers, one scan at a time. Written before the
//! implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! | Case | Required |
//! |---|---|
//! | scripted JPEG | the scan answers those bytes |
//! | scripted Timeout, CoverOpen, Device | the scan answers that failure |
//! | past the end of the script | `NoScanner` |
//! | a second scan while one is in flight | `Busy`, and the first still gets its answer |
//! | after the first finishes | the next scan runs |

use std::sync::Arc;
use std::time::Duration;

use xc_counter::scan::fake::FakeScanner;
use xc_counter::scan::{ScanFailure, ScanRequest, Scanner};

const JPEG: [u8; 4] = [0xFF, 0xD8, 0xFF, 0xD9];

#[tokio::test]
async fn answers_the_script_in_order_then_no_scanner() {
    let scanner = FakeScanner::new(vec![
        Ok(JPEG.to_vec()),
        Err(ScanFailure::Timeout),
        Err(ScanFailure::CoverOpen),
        Err(ScanFailure::Device("jam".to_owned())),
    ]);
    let request = ScanRequest::passport();
    assert_eq!(scanner.scan(request).await, Ok(JPEG.to_vec()));
    assert_eq!(scanner.scan(request).await, Err(ScanFailure::Timeout));
    assert_eq!(scanner.scan(request).await, Err(ScanFailure::CoverOpen));
    assert_eq!(
        scanner.scan(request).await,
        Err(ScanFailure::Device("jam".to_owned()))
    );
    assert_eq!(scanner.scan(request).await, Err(ScanFailure::NoScanner));
    assert_eq!(scanner.requests(), vec![request; 5]);
}

#[tokio::test]
async fn one_scan_at_a_time_second_gets_busy() {
    let scanner = Arc::new(FakeScanner::new(vec![Ok(JPEG.to_vec()), Ok(vec![1, 2, 3])]));
    let hold = scanner.hold_next();

    let first = tokio::spawn({
        let scanner = Arc::clone(&scanner);
        async move { scanner.scan(ScanRequest::passport()).await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !scanner.in_flight() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the first scan is in flight");

    assert_eq!(
        scanner.scan(ScanRequest::passport()).await,
        Err(ScanFailure::Busy)
    );

    hold.release();
    assert_eq!(first.await.expect("joined"), Ok(JPEG.to_vec()));
    assert!(!scanner.in_flight());
    assert_eq!(
        scanner.scan(ScanRequest::passport()).await,
        Ok(vec![1, 2, 3]),
        "Busy took no scripted answer, and the scanner is free again"
    );
}
