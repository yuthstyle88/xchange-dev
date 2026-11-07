//! Declares the app's commands, so each one is allowed only where a file in `capabilities/` says
//! so (design §2 "IPC pinned").

fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "info",
        "desk_binding",
        "scan_passport",
        "pair",
        "exit_kiosk",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("tauri-build");
}
