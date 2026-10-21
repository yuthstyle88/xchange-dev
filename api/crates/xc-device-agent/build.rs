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
    if let Ok(sha) = std::env::var("XC_AGENT_BUILD_SHA") {
        let trimmed = sha.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
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
    if let Ok(built_at) = std::env::var("XC_AGENT_BUILT_AT") {
        let trimmed = built_at.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(built_at) = std::env::var("BUILT_AT") {
        let trimmed = built_at.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    format_rfc3339_utc(std::time::SystemTime::now())
}

fn main() {
    println!("cargo:rerun-if-env-changed=XC_AGENT_BUILD_SHA");
    println!("cargo:rerun-if-env-changed=XC_AGENT_BUILT_AT");
    println!("cargo:rerun-if-env-changed=BUILD_SHA");
    println!("cargo:rerun-if-env-changed=BUILT_AT");

    let build = get_build();
    let built_at = get_built_at();

    println!("cargo:rustc-env=XC_AGENT_BUILD={build}");
    println!("cargo:rustc-env=XC_AGENT_BUILT_AT={built_at}");

    xc_testlist::emit(1);
}
