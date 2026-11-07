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
//! **Exit needs a manager PIN checked by the server, and the server has no route for it.** A
//! search of `api/src` and `api/crates` at `4897292` finds no manager-PIN verify endpoint, so
//! [`exit_with_pin`] names no route until one exists. Do not invent it here.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{Manager, RunEvent, WebviewWindow, WindowEvent};
use url::Url;
use xc_device_agent::client::ApiError;

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
    /// The server refused the PIN.
    WrongPin,
    /// The server could not be asked. The lock stays on.
    Server(ApiError),
    /// The window could not be unlocked.
    Window(String),
}

impl std::fmt::Display for ExitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPin => write!(f, "wrong PIN"),
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

/// Asks the server to check `pin`, then unlocks `window`. See the module doc: the route is missing.
pub async fn exit_with_pin(
    _window: &WebviewWindow,
    _server: &Url,
    _tokens: &dyn TokenStore,
    _pin: &str,
) -> Result<(), ExitError> {
    todo!("CA-3b: needs a server route that does not exist yet")
}
