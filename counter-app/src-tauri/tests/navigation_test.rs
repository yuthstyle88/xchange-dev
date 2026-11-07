//! CA-3a (design §2 "IPC pinned"): the origin lock. Written before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Server: `https://xc-v3.spacing.cloud`.
//!
//! | URL | Decision |
//! |---|---|
//! | server `/desk`, `/desk/backdated`, `/desk?x=1`, `/kiosk-staff`, `/kiosk-staff/a` | Allow |
//! | server `/admin`, `/`, `/api/x`, `/desktop`, `/kiosk-staffing` | Refuse |
//! | `http://` on the server's host | Refuse |
//! | another `https` origin | OpenInSafari |
//! | `http://` on another host | Refuse |
//! | `javascript:`, `file:`, `data:` | Refuse |
//! | userinfo: `https://xc-v3.spacing.cloud@evil.example/desk`, `https://u:p@xc-v3.spacing.cloud/desk` | Refuse |
//! | the server's host on another port | Refuse |
//! | the app's own `tauri://localhost/pair.html`, `/exit.html` | Allow |
//! | any other `tauri:` URL | Refuse |

use url::Url;
use xc_counter::navigation::{NavDecision, decide};

fn check(rows: &[&str], expected: NavDecision) {
    let server = Url::parse("https://xc-v3.spacing.cloud").expect("server");
    for row in rows {
        let url = Url::parse(row).expect(row);
        assert_eq!(decide(&url, &server), expected, "{row}");
    }
}

#[test]
fn desk_and_kiosk_staff_on_the_server_are_allowed() {
    check(
        &[
            "https://xc-v3.spacing.cloud/desk",
            "https://xc-v3.spacing.cloud/desk/backdated",
            "https://xc-v3.spacing.cloud/desk?x=1",
            "https://xc-v3.spacing.cloud/kiosk-staff",
            "https://xc-v3.spacing.cloud/kiosk-staff/a",
        ],
        NavDecision::Allow,
    );
}

#[test]
fn other_paths_on_the_server_are_refused() {
    check(
        &[
            "https://xc-v3.spacing.cloud/admin",
            "https://xc-v3.spacing.cloud/",
            "https://xc-v3.spacing.cloud/api/x",
            "https://xc-v3.spacing.cloud/desktop",
            "https://xc-v3.spacing.cloud/kiosk-staffing",
        ],
        NavDecision::Refuse,
    );
}

#[test]
fn http_is_refused() {
    check(
        &["http://xc-v3.spacing.cloud/desk", "http://example.com/desk"],
        NavDecision::Refuse,
    );
}

#[test]
fn another_https_origin_opens_in_safari() {
    check(
        &[
            "https://example.com/",
            "https://example.com/desk",
            "https://sub.xc-v3.spacing.cloud/desk",
        ],
        NavDecision::OpenInSafari,
    );
}

#[test]
fn javascript_file_and_data_are_refused() {
    check(
        &[
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hello",
        ],
        NavDecision::Refuse,
    );
}

#[test]
fn userinfo_tricks_are_refused() {
    check(
        &[
            "https://xc-v3.spacing.cloud@evil.example/desk",
            "https://u:p@xc-v3.spacing.cloud/desk",
        ],
        NavDecision::Refuse,
    );
}

#[test]
fn a_port_mismatch_is_refused() {
    check(
        &["https://xc-v3.spacing.cloud:8443/desk"],
        NavDecision::Refuse,
    );
}

#[test]
fn only_the_apps_two_local_pages_are_allowed() {
    check(
        &["tauri://localhost/pair.html", "tauri://localhost/exit.html"],
        NavDecision::Allow,
    );
    check(
        &["tauri://localhost/other.html", "tauri://evil/pair.html"],
        NavDecision::Refuse,
    );
}
