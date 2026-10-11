#![allow(dead_code)]

//! Pure build-support functions for the reconcile-check registry seam (#1146,
//! `docs/design/build-time-registries.md` §1): `xc-cash/build.rs` turns the files of
//! `src/application/reconcile/checks/` into the generated `reconcile_checks.rs`.

#[allow(clippy::duplicate_mod)]
#[path = "../../../build_support/module_name.rs"]
mod module_name;

use module_name::is_valid_module_name;

/// One reconcile check file: its module name and its file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDecl {
    pub module: String,
    pub file: String,
}

/// Why a reconcile check file is refused. The build fails and names the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckTableError {
    /// The file stem is not a valid Rust module name.
    NotAModuleName { file: String },
    /// The source has no `pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck>`.
    MissingCheckFn { file: String },
}

impl std::fmt::Display for CheckTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAModuleName { file } => {
                write!(f, "check file {file} is not a valid Rust module name")
            }
            Self::MissingCheckFn { file } => {
                write!(
                    f,
                    "check file {file} is missing `pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck>`"
                )
            }
        }
    }
}

impl std::error::Error for CheckTableError {}

/// (file name, source) of each file in `src/application/reconcile/checks/` -> the check files,
/// sorted by module name. `mod.rs` and files not ending in `.rs` are skipped.
pub fn reconcile_checks_table(files: &[(&str, &str)]) -> Result<Vec<CheckDecl>, CheckTableError> {
    let mut decls = Vec::new();
    for &(file, source) in files {
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let module = &file[..file.len() - 3];
        if !is_valid_module_name(module) {
            return Err(CheckTableError::NotAModuleName {
                file: file.to_string(),
            });
        }
        if !source.contains("pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck>") {
            return Err(CheckTableError::MissingCheckFn {
                file: file.to_string(),
            });
        }
        decls.push(CheckDecl {
            module: module.to_string(),
            file: file.to_string(),
        });
    }
    decls.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(decls)
}

/// The generated source for `$OUT_DIR/reconcile_checks.rs`, included by `reconcile/mod.rs`:
/// `#[path = "<dir>/<file>"]` + `pub mod <module>;` per check,
/// `pub fn standard_checks() -> Vec<std::sync::Arc<dyn ReconcileCheck>> { vec![<module>::check(), ...] }`,
/// and `pub const CHECK_MODULES: &[&str] = &[...];` sorted by module name.
pub fn render(decls: &[CheckDecl], dir: &str) -> String {
    let mut out = String::new();
    for decl in decls {
        out.push_str(&format!(
            "#[path = \"{}/{}\"]\npub mod {};\n",
            dir, decl.file, decl.module
        ));
    }
    out.push_str("pub fn standard_checks() -> Vec<std::sync::Arc<dyn ReconcileCheck>> {\n");
    out.push_str("    vec![");
    for (i, decl) in decls.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{}::check()", decl.module));
    }
    out.push_str("]\n}\n");
    out.push_str("pub const CHECK_MODULES: &[&str] = &[");
    for (i, decl) in decls.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("\"{}\"", decl.module));
    }
    out.push_str("];\n");
    out
}
