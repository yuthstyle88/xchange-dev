mod common;

use std::path::{Path, PathBuf};

use common::{reap_dead_test_docs, settings, test_docs_root};

#[test]
fn settings_verification_store_dir_is_absolute_and_outside_repo() {
    let s = settings(&[]);
    assert!(
        s.verification.store_dir.is_absolute(),
        "store_dir must be absolute, got {:?}",
        s.verification.store_dir
    );

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap_or(&manifest_dir);
    assert!(
        !s.verification.store_dir.starts_with(repo_root),
        "store_dir must be outside repo root {:?}, got {:?}",
        repo_root,
        s.verification.store_dir
    );
}

#[test]
fn settings_respects_explicit_verification_store_dir_override() {
    let custom = std::env::temp_dir().join("custom-verification-store-dir");
    let s = settings(&[("APP_VERIFICATION__STORE_DIR", &custom.to_string_lossy())]);
    assert_eq!(s.verification.store_dir, custom);
}

// ---- #2797: one per-process docs root, and dead processes' dirs are reaped ----
//
// Each test below works in its own directory under `temp_dir()`, never in the
// shared `xc-test-docs` root, so a run never deletes another test's files.

/// A pid above Linux's `pid_max` limit (4194304) and above any real macOS pid.
const DEAD_PID: &str = "4194303";

fn scratch_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("xc-2797-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create scratch root");
    root
}

fn make_dir_with_file(root: &Path, entry: &str) -> PathBuf {
    let dir = root.join(entry).join("0");
    std::fs::create_dir_all(&dir).expect("create entry dir");
    std::fs::write(dir.join("doc.bin"), b"x").expect("write file");
    root.join(entry)
}

#[test]
fn reap_removes_directory_of_dead_pid() {
    let root = scratch_root("dead");
    let dead = make_dir_with_file(&root, DEAD_PID);

    reap_dead_test_docs(&root);

    assert!(
        !dead.exists(),
        "the directory of a pid that is not running must be removed: {dead:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reap_keeps_directory_of_live_pid() {
    let root = scratch_root("live");
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    let live = make_dir_with_file(&root, &child.id().to_string());

    reap_dead_test_docs(&root);
    let kept = live.join("0").join("doc.bin").exists();

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        kept,
        "the directory of a running process (a parallel test) must be kept"
    );
}

#[test]
fn reap_keeps_own_pid_and_non_numeric_entries() {
    let root = scratch_root("own");
    let own = make_dir_with_file(&root, &std::process::id().to_string());
    let foreign = make_dir_with_file(&root, "not-a-pid");
    let old_layout = make_dir_with_file(&root, &format!("xc-test-docs-{DEAD_PID}-0"));

    reap_dead_test_docs(&root);

    assert!(own.exists(), "this process's own directory must be kept");
    assert!(
        foreign.exists(),
        "an entry whose name is not a pid must be kept"
    );
    assert!(
        old_layout.exists(),
        "an old flat-layout entry must be kept (cleaning those is the owner's call)"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn reap_on_missing_root_does_not_panic() {
    let root = std::env::temp_dir().join(format!(
        "xc-2797-{}-missing-never-created",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    reap_dead_test_docs(&root);

    assert!(!root.exists(), "reaping must not create the root");
}

#[test]
fn settings_store_dir_is_under_per_process_root() {
    let s = settings(&[]);
    let own_root = test_docs_root().join(std::process::id().to_string());
    assert!(
        s.verification.store_dir.starts_with(&own_root),
        "store_dir must be under {own_root:?}, got {:?}",
        s.verification.store_dir
    );
    assert_eq!(
        test_docs_root(),
        std::env::temp_dir().join("xc-test-docs"),
        "the shared root is one entry, temp_dir()/xc-test-docs"
    );
    let name = s
        .verification
        .store_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        name.parse::<u64>().is_ok(),
        "the per-call directory is numbered, got {name:?}"
    );
}
