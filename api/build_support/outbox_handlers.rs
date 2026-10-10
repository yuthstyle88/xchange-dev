#![allow(dead_code)]

//! Pure build-support function for the outbox-handler seam (PL-0a, design
//! `docs/design/blueprint/broker-seam.md` §11.1 row "Outbox handlers"): turns the files of
//! `src/bootstrap/outbox_handlers/` into the generated handler list, so that a destination is
//! added by adding its file and no shared file is edited.
//!
//! Scaffolding written by the ticket author for the up-front tests; the bodies are PL-0a's.

#[path = "module_name.rs"]
mod module_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerDecl {
    /// The file stem, used as the generated module name.
    pub module: String,
    /// The file name, joined to the directory by `build.rs` for `#[path]`.
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerTableError {
    /// The file stem is not a Rust module name (lowercase ASCII letter first, then lowercase
    /// letters, digits or `_`, and not a keyword) — the same rule as `views.rs`.
    NotAModuleName { file: String },
    /// The file has no `pub fn handler(`.
    MissingHandlerFn { file: String },
}

impl std::fmt::Display for HandlerTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAModuleName { file } => {
                write!(f, "handler file {file} is not a valid Rust module name")
            }
            Self::MissingHandlerFn { file } => {
                write!(f, "handler file {file} is missing `pub fn handler(`")
            }
        }
    }
}

impl std::error::Error for HandlerTableError {}

/// (file name, source) of each file in `src/bootstrap/outbox_handlers/` → the handler files,
/// sorted by module. `mod.rs` and files not ending in `.rs` are skipped.
pub fn outbox_handlers_table(
    files: &[(&str, &str)],
) -> Result<Vec<HandlerDecl>, HandlerTableError> {
    let mut handlers = Vec::new();

    for &(file, source) in files {
        if !file.ends_with(".rs") || file == "mod.rs" {
            continue;
        }

        let stem = &file[..file.len() - 3];
        if !module_name::is_valid_module_name(stem) {
            return Err(HandlerTableError::NotAModuleName {
                file: file.to_string(),
            });
        }

        if !source.contains("pub fn handler(") {
            return Err(HandlerTableError::MissingHandlerFn {
                file: file.to_string(),
            });
        }

        handlers.push(HandlerDecl {
            module: stem.to_string(),
            file: file.to_string(),
        });
    }

    handlers.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(handlers)
}
