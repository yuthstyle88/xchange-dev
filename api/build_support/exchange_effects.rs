#![allow(dead_code)]

//! Pure build-support function for the exchange-effect seam (XS-0 #1375, design
//! `docs/design/blueprint/membership-points.md` §6.2 "Registration", `broker-seam.md` §11.1): turns the files
//! of `src/bootstrap/exchange_effects/` into the generated effect list, so that a destination is added by
//! adding its file and no shared file is edited. The same mechanism as `outbox_handlers.rs` (lane controller's
//! ruling on #1375: this file stands for the design's stale `registrations.rs`).
//!
//! `build.rs` calls it in one block and writes `$OUT_DIR/exchange_effects.rs`.

#[allow(clippy::duplicate_mod)]
#[path = "module_name.rs"]
mod module_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectDecl {
    /// The file stem, used as the generated module name.
    pub module: String,
    /// The file name, joined to the directory by `build.rs` for `#[path]`.
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectTableError {
    /// The file stem is not a Rust module name (`module_name::is_valid_module_name`).
    NotAModuleName { file: String },
    /// The file has no `pub fn effect(`.
    MissingEffectFn { file: String },
}

impl std::fmt::Display for EffectTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAModuleName { file } => {
                write!(
                    f,
                    "exchange effect file {file} is not a valid Rust module name"
                )
            }
            Self::MissingEffectFn { file } => {
                write!(f, "exchange effect file {file} is missing `pub fn effect(`")
            }
        }
    }
}

impl std::error::Error for EffectTableError {}

/// (file name, source) of each file in `src/bootstrap/exchange_effects/` → the effect files, sorted by module.
/// `mod.rs` and files not ending in `.rs` are skipped.
pub fn exchange_effects_table(files: &[(&str, &str)]) -> Result<Vec<EffectDecl>, EffectTableError> {
    let mut effects = Vec::new();

    for &(file, source) in files {
        if !file.ends_with(".rs") || file == "mod.rs" {
            continue;
        }

        let stem = &file[..file.len() - 3];
        if !module_name::is_valid_module_name(stem) {
            return Err(EffectTableError::NotAModuleName {
                file: file.to_string(),
            });
        }

        if !source.contains("pub fn effect(") {
            return Err(EffectTableError::MissingEffectFn {
                file: file.to_string(),
            });
        }

        effects.push(EffectDecl {
            module: stem.to_string(),
            file: file.to_string(),
        });
    }

    effects.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(effects)
}
