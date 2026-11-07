//! The scan seam (design §3): the reason strings, the passport request, and that `bridge/init.js`
//! knows the same reasons. Written with the skeleton, which is complete for these; they guard the
//! contract. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! | `ScanFailure` | `reason()` |
//! |---|---|
//! | `NoScanner` | `noScanner` |
//! | `Busy` | `busy` |
//! | `Timeout` | `timeout` |
//! | `CoverOpen` | `coverOpen` |
//! | `Device(_)` | `device` |

use std::time::Duration;

use xc_counter::scan::{ScanFailure, ScanRequest};

fn all() -> [(ScanFailure, &'static str); 5] {
    [
        (ScanFailure::NoScanner, "noScanner"),
        (ScanFailure::Busy, "busy"),
        (ScanFailure::Timeout, "timeout"),
        (ScanFailure::CoverOpen, "coverOpen"),
        (ScanFailure::Device("jam".to_owned()), "device"),
    ]
}

#[test]
fn reasons_are_the_strings_the_page_reads() {
    for (failure, reason) in all() {
        assert_eq!(failure.reason(), reason);
    }
}

#[test]
fn init_js_lists_every_reason() {
    let js = include_str!("../src/bridge/init.js");
    for (_, reason) in all() {
        assert!(js.contains(&format!("'{reason}'")), "{reason}");
    }
}

#[test]
fn the_passport_request_is_300_dpi_colour_whole_bed_60_s() {
    assert_eq!(
        ScanRequest::passport(),
        ScanRequest {
            dpi: 300,
            color: true,
            timeout: Duration::from_secs(60),
            whole_bed: true,
        }
    );
}
