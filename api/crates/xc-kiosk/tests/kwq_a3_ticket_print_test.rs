//! KWQ-A3 — `TicketPrintJob` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D9,
//! §4.4 B7, B10): the one ESC/POS renderer of the queue ticket. The bytes carry the number, the
//! branch, the local time and the request QR; the same job renders the same bytes.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.

use chrono::{FixedOffset, TimeZone};
use xc_kiosk::domain::kiosk::value_objects::ticket_print_job::TicketPrintJob;

fn job() -> TicketPrintJob {
    let bangkok = FixedOffset::east_opt(7 * 3600).expect("offset");
    TicketPrintJob {
        display_number: "W007".to_owned(),
        branch_name: "Silom Branch".to_owned(),
        issued_at: bangkok
            .with_ymd_and_hms(2026, 10, 22, 1, 5, 42)
            .single()
            .expect("time"),
        qr_url: "https://xc-v3.example/q#4f1c2a9e-0d3b-4c7a-9e21-7b5d3c1a0f88".to_owned(),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn the_ticket_starts_with_init_and_ends_with_a_cut() {
    let bytes = job().render();
    assert_eq!(&bytes[..2], &[0x1B, b'@'], "ESC @ first");
    let tail = &bytes[bytes.len() - 4..];
    assert_eq!(&tail[..2], &[0x1D, b'V'], "GS V (cut) last");
}

#[test]
fn the_ticket_carries_the_number_the_branch_the_local_time_and_the_qr() {
    let bytes = job().render();
    assert!(contains(&bytes, b"W007"), "the number");
    assert!(contains(&bytes, b"Silom Branch"), "the branch");
    assert!(contains(&bytes, b"2026-10-22 01:05"), "the time in the job's own offset, to the minute");
    assert!(
        contains(&bytes, b"https://xc-v3.example/q#4f1c2a9e-0d3b-4c7a-9e21-7b5d3c1a0f88"),
        "the request QR's data (native QR store command)"
    );
}

#[test]
fn the_same_job_renders_the_same_bytes() {
    assert_eq!(job().render(), job().render());
}

#[test]
fn a_different_number_renders_different_bytes() {
    let mut other = job();
    other.display_number = "W008".to_owned();
    assert_ne!(job().render(), other.render());
    assert!(!contains(&other.render(), b"W007"));
}
