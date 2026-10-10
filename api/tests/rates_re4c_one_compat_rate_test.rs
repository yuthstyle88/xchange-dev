//! Ticket #757 (RE-4c) — one `CompatRate`: the stub RE-4d declared in
//! `xc-exchange/src/domain/booking/entities/booking.rs` is deleted, and `booking_machine.rs`,
//! `presentation/dto/booking.rs` and `views/admin_bookings.rs` use
//! `xc_kernel::money::CompatRate::per_one` (RE-4b, merged), whose refusal is answered `Internal`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.9 R3 ("through **one** function,
//! `CompatRate::per_one(rate, unit_size)` (`api/crates/xc-kernel/src/money/compat_rate.rs`)")
//! and the table's fallback note ("each declares the same one-line stub in its own file … and
//! deletes the stub when it rebases"); the ticket author's ruling on #757 of 2026-10-10.
//!
//! The guard is `git grep -n 'struct CompatRate' -- api/crates/xc-exchange api/src`, run as a
//! test over the source files (spaces ignored), as `log_no_debug_errors_test.rs` reads
//! `api/src`. No database is used; nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] no `struct CompatRate` anywhere under `api/crates/xc-exchange` or `api/src`;
//! - `pub struct CompatRate` in `api/crates/xc-kernel/src/money/compat_rate.rs` (RE-4b).

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether `line` declares a struct named `CompatRate`, however it is spaced.
fn declares_compat_rate(line: &str) -> bool {
    let words: Vec<&str> = line
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(2)
        .any(|pair| pair[0] == "struct" && pair[1] == "CompatRate")
}

/// Every `struct CompatRate` line under `roots` (relative to `api/`).
fn declarations(roots: &[&str]) -> Vec<String> {
    let api = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for root in roots {
        rust_files(&api.join(root), &mut files);
    }
    files.sort();
    let mut hits = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if declares_compat_rate(line) {
                hits.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    hits
}

#[test]
fn no_copy_of_compat_rate_is_left_in_the_exchange_crate_or_the_api() {
    let hits = declarations(&["crates/xc-exchange", "src"]);
    assert!(
        hits.is_empty(),
        "the per-1 rate goes through xc_kernel::money::CompatRate::per_one only (rates.md §4.9 R3); \
         delete the stub and use the kernel's. {} declaration(s):\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn the_guard_sees_the_kernels_compat_rate() {
    // The guard must not be blind: it finds the one declaration that must exist.
    let hits = declarations(&["crates/xc-kernel/src/money"]);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("compat_rate.rs"), "{hits:?}");
    for line in [
        "pub struct CompatRate;",
        "struct  CompatRate {",
        "pub(crate) struct CompatRate(());",
    ] {
        assert!(declares_compat_rate(line), "{line}");
    }
    for line in [
        "use xc_kernel::money::CompatRate;",
        "pub struct CompatRateError;",
        "// CompatRate::per_one",
    ] {
        assert!(!declares_compat_rate(line), "{line}");
    }
}
