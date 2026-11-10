//! XC Counter: a thin Mac shell around the staff desk. It draws no screen of its own beyond the
//! pair-code field and the manager-code exit; it adds only what a browser cannot do: device
//! access, kiosk lock and counter identity (`docs/design/xc-counter-app.md` §2).
//!
//! The skeleton owns this file, `Cargo.toml`, `tauri.conf.json`, `capabilities/` and
//! `bridge/init.js` (§8.6). Each ticket fills only its own module.

#[cfg(not(target_os = "macos"))]
compile_error!("XC Counter builds for macOS only (design §2)");

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::watch;
use url::Url;
use xc_device_agent::client::ApiError;

pub mod agent;
pub mod bridge;
pub mod config;
pub mod keychain;
pub mod kiosk;
pub mod navigation;
pub mod pairing;
pub mod scan;

use config::AppConfig;
use keychain::{KeychainTokenStore, TokenStore};
use navigation::NavDecision;
use scan::Scanner;
use scan::mac::MacScanner;

/// What [`run`] does at startup before or after `GET /api/device-agent/info` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Startup {
    pub lock: bool,
    pub start_runner: bool,
}

/// Pure startup decision: lock only when the server's `info` answer says `kioskLock` is `true`
/// (§5.2: the server resolves `NULL` to `true`); runner starts when paired.
pub fn startup(paired: bool, caps: Option<agent::Capabilities>) -> Startup {
    Startup {
        lock: caps.is_some_and(|c| c.kiosk_lock),
        start_runner: paired,
    }
}

/// Answer from `GET /api/device-agent/info` for [`on_info`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoAnswer {
    Ok(agent::Capabilities),
    Unauthorized,
    Failed,
}

impl From<Result<agent::CounterInfo, ApiError>> for InfoAnswer {
    fn from(res: Result<agent::CounterInfo, ApiError>) -> Self {
        match res {
            Ok(info) => Self::Ok(info.capabilities),
            Err(ApiError::Unauthorized) => Self::Unauthorized,
            Err(_) => Self::Failed,
        }
    }
}

/// What the app does after reading `GET /api/device-agent/info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoAction {
    pub lock: Option<bool>,
    pub show_pair: bool,
    pub stop_runner: bool,
}

/// Pure decision after `GET /api/device-agent/info`: a 401 (`Unauthorized`) unlocks, stops the
/// runner and navigates to `pair.html`; any other failure keeps the current lock state.
pub fn on_info(prev_locked: bool, answer: InfoAnswer) -> InfoAction {
    let _ = prev_locked;
    match answer {
        InfoAnswer::Ok(caps) => InfoAction {
            lock: Some(caps.kiosk_lock),
            show_pair: false,
            stop_runner: false,
        },
        InfoAnswer::Unauthorized => InfoAction {
            lock: Some(false),
            show_pair: true,
            stop_runner: true,
        },
        InfoAnswer::Failed => InfoAction {
            lock: None,
            show_pair: false,
            stop_runner: false,
        },
    }
}

/// Applies the server's `kioskLock` answer to `window`.
///
/// Debug builds never lock (`#[cfg(debug_assertions)]`), so a developer or a manual hardware
/// check on a dev build can always leave. A locked release build leaves with a manager's one-time
/// code (`ui/exit.html`, design §8.8), or by the owner turning `kioskLock` off on
/// Admin → เคาน์เตอร์, which applies within 60 s.
fn apply_lock(window: &tauri::WebviewWindow, lock: bool) -> Result<(), kiosk::ExitError> {
    if lock {
        #[cfg(not(debug_assertions))]
        return kiosk::lock(window);
        #[cfg(debug_assertions)]
        {
            let _ = window;
            Ok(())
        }
    } else {
        kiosk::unlock(window)
    }
}

/// What every command reads. The token stays behind `tokens`; no command returns it.
pub struct AppState {
    pub config: AppConfig,
    pub tokens: Arc<dyn TokenStore>,
    pub scanner: Arc<dyn Scanner>,
    pub runner_stop: Mutex<watch::Sender<bool>>,
    pub refresh_wake: watch::Sender<()>,
}

/// Spawns the drawer runner on Tauri's runtime (stopping any prior runner first) and sends the
/// main window to `/desk`.
pub fn start_runner(app: &AppHandle) {
    let state = app.state::<AppState>();
    let server = state.config.server.clone();
    let tokens = Arc::clone(&state.tokens);
    let mut stop_rx = {
        let mut guard = state.runner_stop.lock().unwrap_or_else(|e| e.into_inner());
        let _ = guard.send(true);
        let (tx, rx) = watch::channel(false);
        *guard = tx;
        rx
    };
    let _ = state.refresh_wake.send(());

    if let Ok(config_dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&config_dir);
        let drawer = agent::local_drawer(&config_dir);
        let journal = config_dir.join("journal.jsonl");
        let run_server = server.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let stop = stop_rx.clone();
                let run = agent::run_agent(&run_server, &*tokens, drawer.clone(), &journal, stop);
                // Stopped, or no token and `bridge::pair` starts a new runner; else ask /info again.
                if matches!(run.await, Ok(()) | Err(ApiError::Unauthorized)) {
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                    _ = stop_rx.wait_for(|stopped| *stopped) => break,
                }
            }
        });
    }

    if let Some(window) = app.get_webview_window("main")
        && let Ok(desk) = server.join("/desk")
        && window.url().ok().as_ref() != Some(&desk)
    {
        let _ = window.navigate(desk);
    }
}

/// Runs the app until it quits.
pub fn run() {
    let config = AppConfig::default();
    let server = config.server.clone();
    let tokens: Arc<dyn TokenStore> = Arc::new(KeychainTokenStore::new("token"));
    let scanner: Arc<dyn Scanner> = Arc::new(MacScanner::start());
    let (runner_stop, _) = watch::channel(false);
    let (refresh_wake, _) = watch::channel(());

    tauri::Builder::default()
        .manage(AppState {
            config,
            tokens,
            scanner,
            runner_stop: Mutex::new(runner_stop),
            refresh_wake,
        })
        .invoke_handler(tauri::generate_handler![
            bridge::info,
            bridge::desk_binding,
            bridge::pair,
            bridge::exit_kiosk,
            bridge::scan_cmd::scan_passport,
        ])
        .setup(move |app| {
            let paired = matches!(app.state::<AppState>().tokens.load(), Ok(Some(_)));
            let initial = startup(paired, None);
            let mut window_config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("tauri.conf.json has no main window")?;
            if paired {
                window_config.url = WebviewUrl::External(server.join("/desk")?);
            }

            let nav_server = server.clone();
            let window = WebviewWindowBuilder::from_config(app.handle(), &window_config)?
                .initialization_script(include_str!("bridge/init.js"))
                .on_navigation(move |url| match navigation::decide(url, &nav_server) {
                    NavDecision::Allow => true,
                    NavDecision::OpenInSafari => {
                        navigation::open_in_safari(url);
                        false
                    }
                    NavDecision::Refuse => false,
                })
                .on_new_window(|_url, _features| NewWindowResponse::Deny)
                .build()?;

            if initial.start_runner {
                start_runner(app.handle());
            }

            let state = app.state::<AppState>();
            let info_server = state.config.server.clone();
            let info_tokens = Arc::clone(&state.tokens);
            let mut wake_rx = state.refresh_wake.subscribe();
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut locked = initial.lock;
                let mut paired = initial.start_runner;
                let mut ticker = tokio::time::interval(Duration::from_secs(60));
                loop {
                    tokio::select! {
                        _ = ticker.tick() => {}
                        res = wake_rx.changed() => {
                            if res.is_err() {
                                break;
                            }
                            paired = true;
                            ticker.reset();
                        }
                    }
                    // Not the token: `info`, `desk_binding` or the runner may have cleared it on a
                    // 401 first, and that counter still has to reach the pair page.
                    if !paired && !locked {
                        continue;
                    }
                    let answer = InfoAnswer::from(
                        agent::counter_info(&info_server, info_tokens.as_ref()).await,
                    );
                    let action = on_info(locked, answer);
                    paired &= !action.show_pair;
                    if let Some(want) = action.lock
                        && want != locked
                        && apply_lock(&window, want).is_ok()
                    {
                        locked = want;
                    }
                    if action.stop_runner
                        && let Ok(guard) = app_handle.state::<AppState>().runner_stop.lock()
                    {
                        let _ = guard.send(true);
                    }
                    if action.show_pair
                        && let Ok(pair_url) = Url::parse("tauri://localhost/pair.html")
                        && window.url().ok().as_ref() != Some(&pair_url)
                    {
                        let _ = window.navigate(pair_url);
                    }
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the app builds")
        .run(|app, event| {
            if matches!(event, RunEvent::Exit)
                && let Ok(guard) = app.state::<AppState>().runner_stop.lock()
            {
                let _ = guard.send(true);
            }
        });
}
