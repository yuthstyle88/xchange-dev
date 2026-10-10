//! Pure build-support functions for the template registry (APP-3 #993, design `customer-app.md`
//! §5): `xc-notify/build.rs` turns the files of `src/templates/` into the generated list, so adding
//! a template is adding one file. Each template file declares `pub const KEY: &str = "<key>";`.

#[allow(clippy::duplicate_mod)]
#[path = "../../../build_support/module_name.rs"]
mod module_name;

use module_name::is_valid_module_name;

/// One template file: its module name, its file name and its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDecl {
    pub module: String,
    pub file: String,
    pub key: String,
}

/// Why the template files are refused. The build fails and names the file(s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateTableError {
    /// The file stem is not a Rust module name (the rule of `api/build_support/views.rs`).
    NotAModuleName { file: String },
    /// The source has no `pub const KEY: &str = "…";`.
    MissingKey { file: String },
    /// Two or more files declare the same key; `files` in file-name order.
    DuplicateKey { key: String, files: Vec<String> },
}

fn extract_key(source: &str) -> Option<String> {
    let mut rest = source;
    while let Some(idx) = rest.find("pub const KEY") {
        let after = &rest[idx + "pub const KEY".len()..];
        if let Some(eq_idx) = after.find('=') {
            let before_eq = after[..eq_idx].trim();
            if before_eq.is_empty() || before_eq.starts_with(':') {
                let after_eq = after[eq_idx + 1..].trim_start();
                if let Some(stripped) = after_eq.strip_prefix('"') {
                    if let Some(end_quote) = stripped.find('"') {
                        return Some(stripped[..end_quote].to_string());
                    }
                }
            }
        }
        rest = after;
    }
    None
}

/// The `.rs` files of `src/templates/` except `mod.rs`, in file-name order.
pub fn template_table(files: &[(&str, &str)]) -> Result<Vec<TemplateDecl>, TemplateTableError> {
    let mut decls = Vec::new();
    for &(file, source) in files {
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let module = &file[..file.len() - 3];
        if !is_valid_module_name(module) {
            return Err(TemplateTableError::NotAModuleName {
                file: file.to_string(),
            });
        }
        let key = extract_key(source).ok_or_else(|| TemplateTableError::MissingKey {
            file: file.to_string(),
        })?;
        decls.push(TemplateDecl {
            module: module.to_string(),
            file: file.to_string(),
            key,
        });
    }

    decls.sort_by(|a, b| a.file.cmp(&b.file));

    let mut by_key: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for decl in &decls {
        by_key
            .entry(decl.key.clone())
            .or_default()
            .push(decl.file.clone());
    }

    for decl in &decls {
        let matching_files = &by_key[&decl.key];
        if matching_files.len() > 1 {
            return Err(TemplateTableError::DuplicateKey {
                key: decl.key.clone(),
                files: matching_files.clone(),
            });
        }
    }

    Ok(decls)
}

/// `#[path = "<dir>/<file>"] pub mod <module>;` per template, then
/// `pub fn generated() -> Vec<Box<dyn NotificationTemplate>>` building one of each.
pub fn render(decls: &[TemplateDecl], dir: &str) -> String {
    let mut out = String::new();
    for decl in decls {
        out.push_str(&format!(
            "#[path = \"{}/{}\"]\npub mod {};\n",
            dir, decl.file, decl.module
        ));
    }
    out.push_str(
        "pub fn generated() -> Vec<Box<dyn crate::domain::template::NotificationTemplate>> {\n",
    );
    out.push_str("    vec![\n");
    for decl in decls {
        out.push_str(&format!("        Box::new({}::Template),\n", decl.module));
    }
    out.push_str("    ]\n");
    out.push_str("}\n");
    out
}
