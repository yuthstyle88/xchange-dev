//! CA-4a. `xcCounter.scanPassport()`, in its own file so CA-3 and CA-4 never share one (§8.6).

use tauri::State;
use tauri::ipc::Response;

use crate::AppState;
use crate::scan::{ScanRequest, Scanner};

/// One scan with the passport settings. `Ok` is the JPEG; `Err` is `ScanFailure::reason`.
pub async fn run_scan(scanner: &dyn Scanner) -> Result<Vec<u8>, &'static str> {
    scanner
        .scan(ScanRequest::passport())
        .await
        .map_err(|failure| failure.reason())
}

/// Answers raw bytes, which `bridge/init.js` wraps in a `Blob`, or the reason string.
#[tauri::command]
pub async fn scan_passport(state: State<'_, AppState>) -> Result<Response, String> {
    run_scan(state.scanner.as_ref())
        .await
        .map(Response::new)
        .map_err(str::to_owned)
}
