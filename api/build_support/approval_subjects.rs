#![allow(dead_code)]

//! Pure build-support function for approval subjects (design
//! `docs/design/build-time-registries.md` §2): turns the files of
//! `src/bootstrap/approval_subjects/` into the generated subject list,
//! so that an approval subject is added by adding its file and no shared file is edited.

#[allow(clippy::duplicate_mod)]
#[path = "module_name.rs"]
mod module_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectDecl {
    /// The file stem, used as the generated module name.
    pub module: String,
    /// The file name, joined to the directory by `build.rs` for `#[path]`.
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectTableError {
    /// The file stem is not a Rust module name (lowercase ASCII letter first, then lowercase
    /// letters, digits or `_`, and not a keyword) — the same rule as `views.rs`.
    NotAModuleName { file: String },
    /// The file has no `pub const TYPE: &str`.
    MissingType { file: String },
    /// The file has no `pub fn register(`.
    MissingRegisterFn { file: String },
}

impl std::fmt::Display for SubjectTableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAModuleName { file } => {
                write!(
                    f,
                    "approval subject file {file} is not a valid Rust module name"
                )
            }
            Self::MissingType { file } => {
                write!(
                    f,
                    "approval subject file {file} is missing `pub const TYPE: &str`"
                )
            }
            Self::MissingRegisterFn { file } => {
                write!(
                    f,
                    "approval subject file {file} is missing `pub fn register(`"
                )
            }
        }
    }
}

impl std::error::Error for SubjectTableError {}

/// (file name, source) of each file in `src/bootstrap/approval_subjects/` →
/// the subject files, sorted by module. `mod.rs` and files not ending in `.rs` are skipped.
pub fn approval_subjects_table(
    files: &[(&str, &str)],
) -> Result<Vec<SubjectDecl>, SubjectTableError> {
    let mut subjects = Vec::new();

    for &(file, source) in files {
        if !file.ends_with(".rs") || file == "mod.rs" {
            continue;
        }

        let stem = &file[..file.len() - 3];
        if !module_name::is_valid_module_name(stem) {
            return Err(SubjectTableError::NotAModuleName {
                file: file.to_string(),
            });
        }

        if !source.contains("pub const TYPE: &str") {
            return Err(SubjectTableError::MissingType {
                file: file.to_string(),
            });
        }

        if !source.contains("pub fn register(") {
            return Err(SubjectTableError::MissingRegisterFn {
                file: file.to_string(),
            });
        }

        subjects.push(SubjectDecl {
            module: stem.to_string(),
            file: file.to_string(),
        });
    }

    subjects.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(subjects)
}
