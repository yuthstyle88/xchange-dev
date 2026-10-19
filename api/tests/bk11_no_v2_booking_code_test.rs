//! Ticket #1413 (BK-11): v2's booking code is gone (design `docs/design/blueprint/booking-slots.md` §6, §9 row BK-11,
//! §9.1 row BK-11). Written before the implementation by the ticket's test writer (claude/5781a09c, rule A). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - no source under `api/` names `v2_bookings` or reads `FROM members` (§9.1 BK-11: `git grep -n -E
//!   "v2_bookings|FROM members" -- api` is empty). Migration files and migration tests are not scanned: an applied
//!   migration is never edited, and a `*_migration_test.rs` migrates only up to its own version;
//! - the v2 owner envelope `/api/v2/bookings` is named nowhere in `api/` or `web/src/` (§6: deleted by BK-11);
//! - `BookingsAdmin` is named nowhere in `api/` or `web/src/` (§6: deleted with the envelope);
//! - every path §9 row BK-11 retires is absent. The admin menu is discovered from `web/src/screens/admin/*/menu.ts`
//!   (`menus.ts:47`), so removing `web/src/screens/admin/bookings/` also removes its menu entry (§6: "the old screen
//!   goes in BK-11").
//!
//! Each failure names the files that still hold the symbol, so the list is the work left. No database needed.

use std::path::{Path, PathBuf};

/// This ticket's own up-front tests: they name the retired symbols on purpose.
const OWN_TESTS: [&str; 2] = [
    "bk11_no_v2_booking_code_test.rs",
    "bk11_retire_v2_bookings_migration_test.rs",
];

const SCANNED: [&str; 8] = ["rs", "sql", "toml", "html", "ts", "tsx", "js", "json"];

fn api() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn repo() -> PathBuf {
    api().parent().expect("api/ has a parent").to_path_buf()
}

/// Every scanned file under `root`, skipping build output, dependencies and (when `skip_migrations`) migration
/// files and migration tests.
fn files(root: &Path, skip_migrations: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries {
            let p = e.expect("an entry").path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            if p.is_dir() {
                if matches!(name.as_str(), "target" | "node_modules" | "dist" | ".git")
                    || (skip_migrations && name == "migrations")
                {
                    continue;
                }
                stack.push(p);
                continue;
            }
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or_default();
            if !SCANNED.contains(&ext) || OWN_TESTS.contains(&name.as_str()) {
                continue;
            }
            if skip_migrations && name.ends_with("_migration_test.rs") {
                continue;
            }
            out.push(p);
        }
    }
    out.sort();
    out
}

/// `needle` occurs in `text` not followed by an identifier character (`FROM members` but not `FROM members_x`).
fn names(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(i, _)| {
        text[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

/// The files under `roots` that name any of `needles`, relative to the repo root.
fn holders(roots: &[PathBuf], skip_migrations: bool, needles: &[&str]) -> Vec<String> {
    let repo = repo();
    let mut out = Vec::new();
    for root in roots {
        for f in files(root, skip_migrations) {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            if needles.iter().any(|n| names(&text, n)) {
                out.push(f.strip_prefix(&repo).unwrap_or(&f).display().to_string());
            }
        }
    }
    out
}

#[test]
fn no_api_source_names_v2_bookings_or_reads_members() {
    let left = holders(&[api()], true, &["v2_bookings", "FROM members"]);
    assert!(
        left.is_empty(),
        "these files still name `v2_bookings` or read `FROM members` (§9.1 BK-11):\n{}",
        left.join("\n")
    );
}

#[test]
fn the_v2_envelope_route_is_named_nowhere() {
    let left = holders(
        &[api(), repo().join("web").join("src")],
        true,
        &["/api/v2/bookings"],
    );
    assert!(
        left.is_empty(),
        "these files still name the v2 envelope `/api/v2/bookings` (§6, deleted by BK-11):\n{}",
        left.join("\n")
    );
}

#[test]
fn bookings_admin_is_named_nowhere() {
    let left = holders(
        &[api(), repo().join("web").join("src")],
        true,
        &["BookingsAdmin"],
    );
    assert!(
        left.is_empty(),
        "these files still name `BookingsAdmin` (§6, deleted with the envelope):\n{}",
        left.join("\n")
    );
}

#[test]
fn every_retired_path_is_gone() {
    let repo = repo();
    let left: Vec<&str> = [
        "api/crates/xc-exchange/src/domain/booking",
        "api/crates/xc-exchange/src/application/booking",
        "api/crates/xc-exchange/src/infrastructure/booking",
        "api/crates/xc-exchange/tests/booking_machine_test.rs",
        "api/crates/xc-exchange/tests/booking_published_line_test.rs",
        "api/crates/xc-exchange/tests/booking_repeat_rows_test.rs",
        "api/crates/xc-exchange/tests/booking_repository_db_test.rs",
        "api/crates/xc-exchange/tests/booking_service_test.rs",
        "api/crates/xc-exchange/tests/booking_values_test.rs",
        "api/src/presentation/http/bookings.rs",
        "api/src/presentation/dto/booking.rs",
        "api/src/presentation/http/v3/views/admin_bookings.rs",
        "api/src/presentation/dto/v3/admin_bookings.rs",
        "api/src/application/bookings",
        "api/src/infrastructure/bookings/booking_ports.rs",
        "web/src/shared/api/generated/admin_bookings.ts",
        "web/src/screens/admin/bookings",
    ]
    .into_iter()
    .filter(|p| repo.join(p).exists())
    .collect();
    assert!(
        left.is_empty(),
        "these retired paths still exist (§9 row BK-11):\n{}",
        left.join("\n")
    );
}
