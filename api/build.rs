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
        && output.status.success()
    {
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

#[path = "build_support/views.rs"]
mod views_gen;

#[path = "build_support/commands.rs"]
mod commands_gen;

#[path = "build_support/outbox_handlers.rs"]
mod outbox_handlers_gen;

#[path = "build_support/approval_subjects.rs"]
mod approval_subjects_gen;

#[path = "build_support/exchange_effects.rs"]
mod exchange_effects_gen;

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

    // Generate v3 DTO modules (§7.7b)
    let dto_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/dto/v3");
    let mut dto_files = Vec::new();
    if dto_dir.exists() {
        for entry in std::fs::read_dir(&dto_dir).expect("read dto/v3") {
            let path = entry.expect("dto entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                dto_files.push((name, content));
            }
        }
    }
    let dto_refs: Vec<(&str, &str)> = dto_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let dto_modules = views_gen::dto_modules(&dto_refs).expect("dto_modules failed");
    let mut dtos_code = String::new();
    for m in &dto_modules {
        let file_path = dto_dir.join(format!("{m}.rs"));
        dtos_code.push_str(&format!(
            "#[path = {:?}]\npub mod {};\n",
            file_path.to_str().unwrap(),
            m
        ));
    }
    std::fs::write(Path::new(&out_dir).join("v3_dtos.rs"), dtos_code).expect("write v3_dtos.rs");
    println!("cargo:rerun-if-changed=src/presentation/dto/v3");

    // Generate v3 views table (§7.7)
    let views_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/http/v3/views");
    let mut view_files = Vec::new();
    if views_dir.exists() {
        for entry in std::fs::read_dir(&views_dir).expect("read http/v3/views") {
            let path = entry.expect("view entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                view_files.push((name, content));
            }
        }
    }
    let view_refs: Vec<(&str, &str)> = view_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let view_table = views_gen::views_table(&view_refs).expect("views_table failed");
    let mut views_code = String::new();
    for v in &view_table {
        let file_path = views_dir.join(&v.file);
        views_code.push_str(&format!(
            "#[path = {:?}]\npub mod {};\n",
            file_path.to_str().unwrap(),
            v.module
        ));
    }
    views_code.push_str("pub const VIEWS: &[ViewEntry] = &[\n");
    for v in &view_table {
        views_code.push_str(&format!(
            "    ViewEntry {{\n        name: {:?},\n        register: {}::register,\n        export_types: {}::export_types,\n    }},\n",
            v.module, v.module, v.module
        ));
    }
    views_code.push_str("];\n");
    std::fs::write(Path::new(&out_dir).join("v3_views.rs"), views_code).expect("write v3_views.rs");
    println!("cargo:rerun-if-changed=src/presentation/http/v3/views");

    // Generate v3 actions table (§12.20.3)
    let actions_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/http/v3/actions");
    let mut action_files = Vec::new();
    if actions_dir.exists() {
        for entry in std::fs::read_dir(&actions_dir).expect("read http/v3/actions") {
            let path = entry.expect("action entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                action_files.push((name, content));
            }
        }
    }
    let action_refs: Vec<(&str, &str)> = action_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let action_table =
        views_gen::views_table(&action_refs).expect("views_table failed for actions");
    views_gen::disjoint(&view_table, &action_table).expect("views and actions must be disjoint");
    let mut actions_code = String::new();
    for a in &action_table {
        let file_path = actions_dir.join(&a.file);
        actions_code.push_str(&format!(
            "#[path = {:?}]\npub mod {};\n",
            file_path.to_str().unwrap(),
            a.module
        ));
    }
    actions_code.push_str("pub const ACTIONS: &[ViewEntry] = &[\n");
    for a in &action_table {
        actions_code.push_str(&format!(
            "    ViewEntry {{\n        name: {:?},\n        register: {}::register,\n        export_types: {}::export_types,\n    }},\n",
            a.module, a.module, a.module
        ));
    }
    actions_code.push_str("];\n");
    std::fs::write(Path::new(&out_dir).join("v3_actions.rs"), actions_code)
        .expect("write v3_actions.rs");
    println!("cargo:rerun-if-changed=src/presentation/http/v3/actions");

    // Generate v3 commands table (§6, §9 CM-1)
    let commands_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/presentation/http/v3/commands");
    let mut command_files = Vec::new();
    if commands_dir.exists() {
        for entry in std::fs::read_dir(&commands_dir).expect("read http/v3/commands") {
            let path = entry.expect("command entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                command_files.push((name, content));
            }
        }
    }
    let command_refs: Vec<(&str, &str)> = command_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let command_table =
        commands_gen::commands_table(&command_refs).expect("commands_table failed for commands");
    let other_names = view_table
        .iter()
        .map(|v| v.module.as_str())
        .chain(action_table.iter().map(|a| a.module.as_str()));
    commands_gen::disjoint_names(&command_table, other_names)
        .expect("commands must be disjoint from views and actions");
    let mut commands_code = String::new();
    for c in &command_table {
        let file_path = commands_dir.join(&c.file);
        commands_code.push_str(&format!(
            "#[path = {:?}]\npub mod {};\n",
            file_path.to_str().unwrap(),
            c.module
        ));
    }
    commands_code.push_str("pub const COMMANDS: &[ViewEntry] = &[\n");
    for c in &command_table {
        commands_code.push_str(&format!(
            "    ViewEntry {{\n        name: {:?},\n        register: {}::register,\n        export_types: {}::export_types,\n    }},\n",
            c.module, c.module, c.module
        ));
    }
    commands_code.push_str("];\n");
    std::fs::write(Path::new(&out_dir).join("v3_commands.rs"), commands_code)
        .expect("write v3_commands.rs");
    println!("cargo:rerun-if-changed=src/presentation/http/v3/commands");

    // Generate outbox handlers table (PL-0a, #910)
    let handlers_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/outbox_handlers");
    let mut handler_files = Vec::new();
    if handlers_dir.exists() {
        for entry in std::fs::read_dir(&handlers_dir).expect("read bootstrap/outbox_handlers") {
            let path = entry.expect("outbox handler entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                handler_files.push((name, content));
            }
        }
    }
    let handler_refs: Vec<(&str, &str)> = handler_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let handler_table = outbox_handlers_gen::outbox_handlers_table(&handler_refs)
        .expect("outbox_handlers_table failed");
    let mut handlers_code = String::new();
    for h in &handler_table {
        let file_path = handlers_dir.join(&h.file);
        handlers_code.push_str(&format!(
            "#[path = {:?}]\nmod {};\n",
            file_path.to_str().unwrap(),
            h.module
        ));
    }
    handlers_code.push_str("pub const OUTBOX_HANDLERS: &[HandlerFn] = &[\n");
    for h in &handler_table {
        handlers_code.push_str(&format!("    {}::handler,\n", h.module));
    }
    handlers_code.push_str("];\n");
    std::fs::write(
        Path::new(&out_dir).join("outbox_handlers.rs"),
        handlers_code,
    )
    .expect("write outbox_handlers.rs");
    println!("cargo:rerun-if-changed=src/bootstrap/outbox_handlers");

    // Generate the exchange-effect list (XS-0, #1375)
    let effects_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/exchange_effects");
    let mut effect_files = Vec::new();
    if effects_dir.exists() {
        for entry in std::fs::read_dir(&effects_dir).expect("read bootstrap/exchange_effects") {
            let path = entry.expect("exchange effect entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                effect_files.push((name, content));
            }
        }
    }
    let effect_refs: Vec<(&str, &str)> = effect_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let effect_table = exchange_effects_gen::exchange_effects_table(&effect_refs)
        .unwrap_or_else(|e| panic!("exchange_effects_table failed: {e}"));
    let mut effects_code = String::new();
    for e in &effect_table {
        let file_path = effects_dir.join(&e.file);
        effects_code.push_str(&format!(
            "#[path = {:?}]\nmod {};\n",
            file_path.to_str().unwrap(),
            e.module
        ));
    }
    effects_code.push_str("pub const EXCHANGE_EFFECTS: &[EffectFn] = &[\n");
    for e in &effect_table {
        effects_code.push_str(&format!("    {}::effect,\n", e.module));
    }
    effects_code.push_str("];\n");
    std::fs::write(
        Path::new(&out_dir).join("exchange_effects.rs"),
        effects_code,
    )
    .expect("write exchange_effects.rs");
    println!("cargo:rerun-if-changed=src/bootstrap/exchange_effects");

    // Generate approval subjects table (#1146 ticket B)
    let subjects_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bootstrap/approval_subjects");
    let mut subject_files = Vec::new();
    if subjects_dir.exists() {
        for entry in std::fs::read_dir(&subjects_dir).expect("read bootstrap/approval_subjects") {
            let path = entry.expect("approval subject entry").path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                subject_files.push((name, content));
            }
        }
    }
    let subject_refs: Vec<(&str, &str)> = subject_files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let subject_table = approval_subjects_gen::approval_subjects_table(&subject_refs)
        .expect("approval_subjects_table failed");
    let mut subjects_code = String::new();
    for s in &subject_table {
        let file_path = subjects_dir.join(&s.file);
        subjects_code.push_str(&format!(
            "#[path = {:?}]\nmod {};\n",
            file_path.to_str().unwrap(),
            s.module
        ));
    }
    subjects_code.push_str("pub const APPROVAL_SUBJECTS: &[RegisterFn] = &[\n");
    for s in &subject_table {
        subjects_code.push_str(&format!("    {}::register,\n", s.module));
    }
    subjects_code.push_str("];\n");
    subjects_code.push_str("pub const APPROVAL_SUBJECT_TYPES: &[&str] = &[\n");
    for s in &subject_table {
        subjects_code.push_str(&format!("    {}::TYPE,\n", s.module));
    }
    subjects_code.push_str("];\n");
    subjects_code.push_str("pub const APPROVAL_SUBJECT_MODULES: &[&str] = &[\n");
    for s in &subject_table {
        subjects_code.push_str(&format!("    {:?},\n", s.module));
    }
    subjects_code.push_str("];\n");
    std::fs::write(
        Path::new(&out_dir).join("approval_subjects.rs"),
        subjects_code,
    )
    .expect("write approval_subjects.rs");
    println!("cargo:rerun-if-changed=src/bootstrap/approval_subjects");
    for (name, _) in &subject_files {
        println!("cargo:rerun-if-changed=src/bootstrap/approval_subjects/{name}");
    }
}
