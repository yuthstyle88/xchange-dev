//! CA-3b. The kiosk lock (design §2, §4 `kioskLock`).
//!
//! What macOS lets an app catch, per shortcut:
//!
//! | Shortcut | How it is stopped | Caught as a key event? |
//! |---|---|---|
//! | Cmd+Q | the app menu has no Quit item while locked; `RunEvent::ExitRequested` is refused | yes |
//! | Cmd+W | the window's close is refused; `WindowEvent::CloseRequested` is prevented | yes |
//! | Cmd+H | the app menu has no Hide item while locked | yes |
//! | Cmd+M | the app menu has no Minimize item while locked | yes |
//! | Cmd+Tab | `NSApplicationPresentationOptions::DisableProcessSwitching` | **no**: the system handles it before any app, so only this option stops it |
//! | Cmd+Option+Esc | `NSApplicationPresentationOptions::DisableForceQuit` | **no**: same; this option needs `HideDock` set too |
//!
//! These presentation options are honoured only while the app is frontmost, and a logged-in user
//! with a hard power-off can still leave. The lock keeps staff on the desk; it does not defend
//! against someone at the machine with admin rights.
//!
//! **Exit needs a one-time code from a manager, checked by the server** (design §8.8, xchange#6569).
//! A manager or the owner issues it at the desk (`counterUnlockIssue`); staff type it on
//! `ui/exit.html`; [`exit_with_pin`] sends it to `POST /api/device-agent/kiosk-exit` under this
//! counter's agent token and unlocks only on 204. The app never judges the code itself: a refusal,
//! or no answer at all, leaves the lock on. (The command keeps its `pin` name from before the
//! route existed; the value is the code.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Manager, RunEvent, WebviewWindow, WindowEvent};
use url::Url;
use xc_device_agent::client::{ApiError, HttpAgentApi};

use crate::keychain::TokenStore;

static LOCKED: AtomicBool = AtomicBool::new(false);
static CLOSE_GUARD_INSTALLED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
mod presentation {
    pub const DEFAULT: usize = 0;
    pub const HIDE_DOCK: usize = 1 << 1;
    pub const HIDE_MENU_BAR: usize = 1 << 3;
    pub const DISABLE_PROCESS_SWITCHING: usize = 1 << 5;
    pub const DISABLE_FORCE_QUIT: usize = 1 << 6;
    pub const DISABLE_SESSION_TERMINATION: usize = 1 << 7;
    pub const DISABLE_HIDE_APPLICATION: usize = 1 << 8;

    pub const LOCKED: usize = HIDE_DOCK
        | HIDE_MENU_BAR
        | DISABLE_PROCESS_SWITCHING
        | DISABLE_FORCE_QUIT
        | DISABLE_SESSION_TERMINATION
        | DISABLE_HIDE_APPLICATION;
}

#[cfg(target_os = "macos")]
fn set_presentation_options(window: &WebviewWindow, options: usize) -> Result<(), ExitError> {
    window
        .run_on_main_thread(move || unsafe {
            let app: *mut objc2::runtime::AnyObject =
                objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
            let _: () = objc2::msg_send![app, setPresentationOptions: options];
        })
        .map_err(|e| ExitError::Window(e.to_string()))
}

/// The shortcuts the lock stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    CmdQ,
    CmdW,
    CmdTab,
    CmdH,
    CmdM,
    CmdOptionEsc,
}

/// Whether the lock stops `shortcut` now. Pure.
pub fn intercepts(shortcut: Shortcut, locked: bool) -> bool {
    match shortcut {
        Shortcut::CmdQ
        | Shortcut::CmdW
        | Shortcut::CmdTab
        | Shortcut::CmdH
        | Shortcut::CmdM
        | Shortcut::CmdOptionEsc => locked,
    }
}

/// Why the app did not leave kiosk mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitError {
    /// The server refused the code: wrong, expired, used, or issued for another counter.
    Refused,
    /// The server could not be asked. The lock stays on.
    Server(ApiError),
    /// The window could not be unlocked.
    Window(String),
}

impl std::fmt::Display for ExitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused => write!(f, "รหัสไม่ถูกต้อง หมดอายุ หรือใช้ไปแล้ว"),
            Self::Server(err) => write!(f, "server: {err}"),
            Self::Window(err) => write!(f, "window: {err}"),
        }
    }
}

impl std::error::Error for ExitError {}

/// Locks `window`: full screen, no chrome, the presentation options and menu above.
pub fn lock(window: &WebviewWindow) -> Result<(), ExitError> {
    if !CLOSE_GUARD_INSTALLED.swap(true, Ordering::SeqCst) {
        window.on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && LOCKED.load(Ordering::SeqCst)
            {
                api.prevent_close();
            }
        });
    }

    window
        .app_handle()
        .plugin(
            tauri::plugin::Builder::<_, ()>::new("kiosk")
                .on_event(|_app, event| {
                    if let RunEvent::ExitRequested { api, .. } = event
                        && LOCKED.load(Ordering::SeqCst)
                    {
                        api.prevent_exit();
                    }
                })
                .build(),
        )
        .map_err(|e| ExitError::Window(e.to_string()))?;

    LOCKED.store(true, Ordering::SeqCst);

    let _ = window.app_handle().remove_menu();
    window
        .set_closable(false)
        .map_err(|e| ExitError::Window(e.to_string()))?;
    window
        .set_minimizable(false)
        .map_err(|e| ExitError::Window(e.to_string()))?;
    window
        .set_decorations(false)
        .map_err(|e| ExitError::Window(e.to_string()))?;
    window
        .set_fullscreen(true)
        .map_err(|e| ExitError::Window(e.to_string()))?;

    #[cfg(target_os = "macos")]
    set_presentation_options(window, presentation::LOCKED)?;

    Ok(())
}

/// Undoes [`lock`].
pub fn unlock(window: &WebviewWindow) -> Result<(), ExitError> {
    LOCKED.store(false, Ordering::SeqCst);
    window.app_handle().remove_plugin("kiosk");

    #[cfg(target_os = "macos")]
    set_presentation_options(window, presentation::DEFAULT)?;

    window
        .set_fullscreen(false)
        .map_err(|e| ExitError::Window(e.to_string()))?;
    window
        .set_decorations(true)
        .map_err(|e| ExitError::Window(e.to_string()))?;
    window
        .set_closable(true)
        .map_err(|e| ExitError::Window(e.to_string()))?;
    window
        .set_minimizable(true)
        .map_err(|e| ExitError::Window(e.to_string()))?;

    if let Ok(menu) = tauri::menu::Menu::default(window.app_handle()) {
        let _ = window.app_handle().set_menu(menu);
    }

    Ok(())
}

/// Asks the server to use up the manager's code `pin`, then unlocks `window`. See the module doc.
pub async fn exit_with_pin(
    window: &WebviewWindow,
    server: &Url,
    tokens: &dyn TokenStore,
    pin: &str,
) -> Result<(), ExitError> {
    redeem(server, tokens, pin).await?;
    unlock(window)
}

/// `POST /api/device-agent/kiosk-exit` with `{ code }`: `Ok` only on 204.
pub async fn redeem(server: &Url, tokens: &dyn TokenStore, code: &str) -> Result<(), ExitError> {
    let token = match tokens.load() {
        Ok(Some(t)) if !t.is_empty() => t,
        Ok(_) => return Err(ExitError::Server(ApiError::Unauthorized)),
        Err(e) => return Err(ExitError::Server(ApiError::Connection(e.to_string()))),
    };
    HttpAgentApi::validate_url(server.as_str()).map_err(ExitError::Server)?;
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| ExitError::Server(ApiError::Connection(e.to_string())))?;
    let url = format!(
        "{}/api/device-agent/kiosk-exit",
        server.as_str().trim_end_matches('/')
    );
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "code": code }))
        .send()
        .await
        // The error text can carry the request; keep the token out of it.
        .map_err(|e| {
            ExitError::Server(ApiError::Connection(
                e.to_string().replace(&token, "<redacted>"),
            ))
        })?;
    match response.status().as_u16() {
        204 => Ok(()),
        403 => Err(ExitError::Refused),
        401 => Err(ExitError::Server(ApiError::Unauthorized)),
        status => Err(ExitError::Server(ApiError::Rejected {
            status,
            error: response.text().await.unwrap_or_default(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::TokenError;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct TestStore(Mutex<Option<String>>);

    impl TokenStore for TestStore {
        fn load(&self) -> Result<Option<String>, TokenError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, token: &str) -> Result<(), TokenError> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }
        fn clear(&self) -> Result<(), TokenError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    fn store() -> TestStore {
        TestStore(Mutex::new(Some("secret-tok".to_owned())))
    }

    /// Answers one request with `status` and gives back what was asked.
    async fn mock_server(status: u16) -> (Url, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).await.unwrap();
            let resp =
                format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn a_204_lets_the_counter_out_and_sends_the_code_under_the_agent_token() {
        let (url, handle) = mock_server(204).await;
        assert_eq!(redeem(&url, &store(), "ABCDE-FGHJK").await, Ok(()));
        let req = handle.await.unwrap();
        assert!(
            req.starts_with("POST /api/device-agent/kiosk-exit "),
            "{req}"
        );
        assert!(req.contains("Bearer secret-tok"), "{req}");
        assert!(req.contains(r#"{"code":"ABCDE-FGHJK"}"#), "{req}");
    }

    #[tokio::test]
    async fn a_403_is_refused_and_the_lock_stays() {
        let (url, _) = mock_server(403).await;
        assert_eq!(
            redeem(&url, &store(), "ABCDE-FGHJK").await,
            Err(ExitError::Refused)
        );
    }

    #[tokio::test]
    async fn no_answer_keeps_the_lock() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        assert!(matches!(
            redeem(&url, &store(), "ABCDE-FGHJK").await,
            Err(ExitError::Server(ApiError::Connection(_)))
        ));
    }

    #[tokio::test]
    async fn an_unpaired_counter_asks_nothing() {
        let url = Url::parse("http://127.0.0.1:9").unwrap();
        let empty = TestStore(Mutex::new(None));
        assert_eq!(
            redeem(&url, &empty, "ABCDE-FGHJK").await,
            Err(ExitError::Server(ApiError::Unauthorized))
        );
    }
}
