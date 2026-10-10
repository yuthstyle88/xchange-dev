//! Verification that Application::build refuses to start when no PII key file exists
//! and no PII keys are pre-installed in the process (#651).

mod common;

use common::{closed_port, settings};
use xchange::bootstrap::Application;

#[tokio::test]
async fn build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed() {
    // In this fresh test binary process, no PII keys are installed
    assert!(
        xc_customer::get_pii_keys().is_err(),
        "sanity check: no keys pre-installed in this process"
    );

    let s = settings(&[
        ("APP_APPLICATION__PORT", &closed_port().to_string()),
        ("APP_APPLICATION__HOST", "127.0.0.1"),
    ]);

    let result = Application::build(s).await;
    match result {
        Ok(_) => panic!("Application::build must fail when PII keys are not installed"),
        Err(err) => {
            let msg = format!("{err:#}");
            assert!(
                msg.contains("/etc/xc-v3/pii.key"),
                "error message must name /etc/xc-v3/pii.key, got: {msg}"
            );
        }
    }
}
