//! CA-3d. The commands the pages may call (design §3, §8.6). Thin: each one delegates. Which page
//! may call which is in `capabilities/`. No command returns the agent token.

use tauri::{AppHandle, State, WebviewWindow};

use crate::AppState;
use crate::agent::{self, CounterInfo, DeskBinding};
use crate::kiosk;
use crate::pairing::{self, Paired};

pub mod scan_cmd;

/// `xcCounter.info()`.
#[tauri::command]
pub async fn info(state: State<'_, AppState>) -> Result<CounterInfo, String> {
    agent::counter_info(&state.config.server, state.tokens.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// `xcCounter.bindingNonce()`.
#[tauri::command]
pub async fn desk_binding(state: State<'_, AppState>) -> Result<DeskBinding, String> {
    agent::desk_binding(&state.config.server, state.tokens.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// `ui/pair.html`.
#[tauri::command]
pub async fn pair(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> Result<Paired, String> {
    let paired = pairing::pair(&state.config.server, &code, state.tokens.as_ref())
        .await
        .map_err(|e| e.to_string())?;
    crate::start_runner(&app);
    Ok(paired)
}

/// `ui/exit.html`.
#[tauri::command]
pub async fn exit_kiosk(
    window: WebviewWindow,
    state: State<'_, AppState>,
    pin: String,
) -> Result<(), String> {
    kiosk::exit_with_pin(&window, &state.config.server, state.tokens.as_ref(), &pin)
        .await
        .map_err(|e| e.to_string())
}
