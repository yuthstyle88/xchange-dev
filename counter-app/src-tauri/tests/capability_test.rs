//! Design §2 "IPC pinned": what the capability files allow. Written with the skeleton, which is
//! complete for these; they guard the contract. Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! | File | Required |
//! |---|---|
//! | `bridge.json` | remote URLs are exactly the configured server's `/desk*` and `/kiosk-staff*`; permissions are exactly the three bridge commands, with no `core:` or plugin entry |
//! | `local.json` | local only, no remote; exactly pair and exit |

use serde_json::{Value, json};
use xc_counter::config::AppConfig;

fn file(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON")
}

#[test]
fn the_bridge_names_one_origin_two_paths_and_three_commands() {
    let bridge = file(include_str!("../capabilities/bridge.json"));
    let server = AppConfig::default().server;
    let origin = server.origin().ascii_serialization();
    assert!(origin.starts_with("https://"), "{origin}");
    assert_eq!(
        bridge["remote"]["urls"],
        json!([format!("{origin}/desk*"), format!("{origin}/kiosk-staff*")])
    );
    assert_eq!(
        bridge["permissions"],
        json!(["allow-info", "allow-desk-binding", "allow-scan-passport"])
    );
}

#[test]
fn the_local_pages_get_pair_and_exit_and_no_remote() {
    let local = file(include_str!("../capabilities/local.json"));
    assert_eq!(local["local"], json!(true));
    assert!(local.get("remote").is_none());
    assert_eq!(
        local["permissions"],
        json!(["allow-pair", "allow-exit-kiosk"])
    );
}
