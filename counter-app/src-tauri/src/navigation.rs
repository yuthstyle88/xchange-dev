//! CA-3a. The origin lock: which URLs the window may show (design §2 "IPC pinned").

use std::process::Command;

use url::Url;

/// What to do with a navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDecision {
    /// The window goes there.
    Allow,
    /// The window stays; Safari opens the link.
    OpenInSafari,
    /// The window stays and nothing opens.
    Refuse,
}

const ALLOWED_SERVER_PATHS: &[&str] = &["/desk", "/kiosk-staff", "/login", "/staff/desk"];

/// Decides one navigation. Pure. The table is at the top of `tests/navigation_test.rs`.
pub fn decide(url: &Url, server: &Url) -> NavDecision {
    if !url.username().is_empty() || url.password().is_some() {
        return NavDecision::Refuse;
    }

    match url.scheme() {
        "tauri" => {
            if url.host_str() == Some("localhost")
                && matches!(url.path(), "/pair.html" | "/exit.html")
            {
                NavDecision::Allow
            } else {
                NavDecision::Refuse
            }
        }
        "https" => {
            if url.host() == server.host() {
                if url.port_or_known_default() == server.port_or_known_default()
                    && is_allowed_server_path(url.path())
                {
                    NavDecision::Allow
                } else {
                    NavDecision::Refuse
                }
            } else {
                NavDecision::OpenInSafari
            }
        }
        _ => NavDecision::Refuse,
    }
}

fn is_allowed_server_path(path: &str) -> bool {
    ALLOWED_SERVER_PATHS.iter().any(|&allowed| {
        path == allowed
            || path
                .strip_prefix(allowed)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Hands `url` to Safari. Never called with anything [`decide`] did not answer `OpenInSafari` for.
pub fn open_in_safari(url: &Url) {
    let _ = Command::new("/usr/bin/open").arg(url.as_str()).status();
}
