//! Verification that Application::build refuses to start when no PII key file exists
//! and no PII keys are pre-installed in the process (#651).

mod common;

use std::path::Path;

use common::{closed_port, settings};
use xchange::bootstrap::Application;

#[test]
fn pii_key_path_defaults_to_etc_xc_v3_pii_key() {
    let s = settings(&[]);
    assert_eq!(s.pii.key_path, Path::new("/etc/xc-v3/pii.key"));
}

#[tokio::test]
async fn build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed() {
    // In this fresh test binary process, no PII keys are installed
    assert!(
        xc_customer::get_pii_keys().is_err(),
        "sanity check: no keys pre-installed in this process"
    );

    let dir = std::env::temp_dir().join(format!("xc-3918-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let missing_key = dir.join("pii.key");
    let missing_key_str = missing_key.to_string_lossy().into_owned();

    let s = settings(&[
        ("APP_APPLICATION__PORT", &closed_port().to_string()),
        ("APP_APPLICATION__HOST", "127.0.0.1"),
        ("APP_PII__KEY_PATH", &missing_key_str),
    ]);

    let result = Application::build(s).await;
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(_) => panic!("Application::build must fail when PII keys are not installed"),
        Err(err) => {
            let msg = format!("{err:#}");
            assert!(
                msg.contains(&missing_key_str),
                "error message must name {missing_key_str}, got: {msg}"
            );
            assert!(
                msg.contains("does not exist and no keys are installed"),
                "error message must explain missing key file, got: {msg}"
            );
        }
    }
}

#[tokio::test]
async fn build_attempts_to_load_existing_configured_pii_key_file() {
    let dir = std::env::temp_dir().join(format!("xc-3918-exists-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let key_file = dir.join("pii.key");
    std::fs::write(&key_file, b"invalid-pii-key-content").expect("write temp key file");
    let key_file_str = key_file.to_string_lossy().into_owned();

    let s = settings(&[
        ("APP_APPLICATION__PORT", &closed_port().to_string()),
        ("APP_APPLICATION__HOST", "127.0.0.1"),
        ("APP_PII__KEY_PATH", &key_file_str),
    ]);

    let result = Application::build(s).await;
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(_) => panic!("Application::build must fail on invalid key content"),
        Err(err) => {
            let msg = format!("{err:#}");
            assert!(
                !msg.contains("does not exist"),
                "existing key file must not fail with 'does not exist', got: {msg}"
            );
        }
    }
}
