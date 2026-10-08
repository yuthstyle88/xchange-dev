fn format_rfc3339_utc(now: std::time::SystemTime) -> String {
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86400) as i64;
    let rem_secs = (secs % 86400) as u32;
    let hour = rem_secs / 3600;
    let minute = (rem_secs % 3600) / 60;
    let second = rem_secs % 60;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn get_build() -> String {
    if let Ok(sha) = std::env::var("BUILD_SHA") {
        let trimmed = sha.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
    {
        if output.status.success() {
            let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !sha.is_empty() {
                let is_dirty = std::process::Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .map(|o| {
                        o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty()
                    })
                    .unwrap_or(false);
                if is_dirty {
                    return format!("{sha}-dirty");
                }
                return sha;
            }
        }
    }
    "unknown".to_string()
}

fn get_built_at() -> String {
    if let Ok(built_at) = std::env::var("BUILT_AT") {
        let trimmed = built_at.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    format_rfc3339_utc(std::time::SystemTime::now())
}

#[path = "build_support/screens.rs"]
mod screens;

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=nonexistent_file_to_force_rerun");
    println!("cargo:rerun-if-env-changed=BUILD_SHA");
    println!("cargo:rerun-if-env-changed=BUILT_AT");
    println!("cargo:rerun-if-changed=migrations");

    let build = get_build();
    let built_at = get_built_at();

    println!("cargo:rustc-env=XCHANGE_BUILD={build}");
    println!("cargo:rustc-env=XCHANGE_BUILT_AT={built_at}");

    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    screens::ensure_dist(&dist).expect("failed to ensure ../web/dist directory exists");

    let profile = std::env::var("PROFILE").unwrap_or_default();
    if let Err(m) = screens::check_dist(&profile, &dist) {
        panic!(
            "{m}. Run `npm --prefix web ci && npm --prefix web run build` before a release build."
        );
    }

    let entries = screens::read_entries(&dist).expect("failed to read web/dist entries");
    let entry_refs: Vec<(&str, &str)> = entries
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    let table =
        screens::screen_table(&entry_refs).expect("failed to parse screen table from web/dist");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("v3_screens.rs");
    let mut code = String::new();
    code.push_str("pub(crate) const SCREENS: &[(&str, Gate, &str)] = &[\n");
    for s in &table {
        let gate_repr = match s.gate {
            screens::Gate::Anyone => "Gate::Anyone",
            screens::Gate::SignedIn => "Gate::SignedIn",
            screens::Gate::Branch => "Gate::Branch",
            screens::Gate::Front => "Gate::Front",
        };
        code.push_str(&format!(
            "    ({:?}, {}, {:?}),\n",
            s.path, gate_repr, s.entry
        ));
    }
    code.push_str("];\n");
    std::fs::write(&dest_path, code).expect("failed to write v3_screens.rs");

    println!("cargo:rerun-if-changed=../web/dist");
}
