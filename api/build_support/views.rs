#![allow(dead_code)]

//! Pure build-support functions for generating v3 view and DTO tables (§7.7, §7.7b).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDecl {
    pub module: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewTableError {
    NotAModuleName { file: String },
    MissingRegister { file: String },
    MissingExportTypes { file: String },
    NameInBoth { module: String },
}

#[allow(clippy::duplicate_mod)]
#[path = "module_name.rs"]
mod module_name;

use module_name::is_valid_module_name;

/// (file name, source) of each file in http/v3/views/ → the views, sorted by module.
pub fn views_table(files: &[(&str, &str)]) -> Result<Vec<ViewDecl>, ViewTableError> {
    let mut decls = Vec::new();
    for &(file, source) in files {
        if !file.ends_with(".rs") {
            continue;
        }
        let module = &file[..file.len() - 3];
        if !is_valid_module_name(module) {
            return Err(ViewTableError::NotAModuleName {
                file: file.to_owned(),
            });
        }
        if !source.contains("pub fn register(") {
            return Err(ViewTableError::MissingRegister {
                file: file.to_owned(),
            });
        }
        if !source.contains("pub fn export_types(") {
            return Err(ViewTableError::MissingExportTypes {
                file: file.to_owned(),
            });
        }
        decls.push(ViewDecl {
            module: module.to_owned(),
            file: file.to_owned(),
        });
    }
    decls.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(decls)
}

/// (file name, source) of each file in dto/v3/ → the module names, sorted. `mod.rs` and non-`.rs` files are ignored.
pub fn dto_modules(files: &[(&str, &str)]) -> Result<Vec<String>, ViewTableError> {
    let mut modules = Vec::new();
    for &(file, _source) in files {
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let module = &file[..file.len() - 3];
        if !is_valid_module_name(module) {
            return Err(ViewTableError::NotAModuleName {
                file: file.to_owned(),
            });
        }
        modules.push(module.to_owned());
    }
    modules.sort();
    Ok(modules)
}

/// Ensures that view modules and action modules share no module name (§12.20.3).
pub fn disjoint(views: &[ViewDecl], actions: &[ViewDecl]) -> Result<(), ViewTableError> {
    let mut view_names = std::collections::BTreeSet::new();
    for v in views {
        view_names.insert(&v.module);
    }
    let mut overlaps = std::collections::BTreeSet::new();
    for a in actions {
        if view_names.contains(&a.module) {
            overlaps.insert(&a.module);
        }
    }
    if let Some(&first) = overlaps.iter().next() {
        return Err(ViewTableError::NameInBoth {
            module: first.clone(),
        });
    }
    Ok(())
}
