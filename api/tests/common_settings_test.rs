mod common;

use std::path::PathBuf;

use common::settings;

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
