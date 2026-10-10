//! Ticket #769 — `KioskPorts::check_stale_in` has no default body.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.9 R5 (*a stale line whose policy cannot be found
//! must be an error*). Ticket #769 §1–§3: the trait's default body answers "admitted"
//! (`Ok(true)`), so any implementation that forgets the method fails open; the default goes and
//! every implementation says what it does. The four test fakes that implement `KioskPorts`
//! (`xc-kiosk/tests/kiosk_payment_test.rs`, `xc-kiosk/tests/kiosk_service_common/mod.rs`,
//! `api/tests/kiosk_save_step_test.rs`, `api/tests/kiosk_save_step_time_test.rs`) implement it
//! themselves since the commit that added this file.
//!
//! The test reads the trait's source, as the repository's other source-reading guards do
//! (`known_differences_listed_test.rs`): from `fn check_stale_in` the signature must end at a
//! `;`, never at a `{`. Needs no database; nothing here skips.
//!
//! Signatures this test assumes: the trait stays in
//! `api/crates/xc-kiosk/src/domain/kiosk/repositories/kiosk_ports.rs` with one method named
//! `check_stale_in` (#769 §5, §10 step 5).

use std::fs;
use std::path::Path;

const TRAIT_FILE: &str = "crates/xc-kiosk/src/domain/kiosk/repositories/kiosk_ports.rs";

#[test]
fn kiosk_ports_check_stale_in_is_declared_without_a_body() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TRAIT_FILE);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let starts: Vec<usize> = text
        .match_indices("fn check_stale_in")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        starts.len(),
        1,
        "{TRAIT_FILE}: one `fn check_stale_in`, found {}",
        starts.len()
    );
    let rest = &text[starts[0]..];
    let end = rest
        .find(['{', ';'])
        .unwrap_or_else(|| panic!("{TRAIT_FILE}: `fn check_stale_in` has no end"));
    let signature = &rest[..=end];
    assert!(
        signature.ends_with(';'),
        "{TRAIT_FILE}: `check_stale_in` must be declared without a default body (#769), \
         found:\n{signature}"
    );
}
