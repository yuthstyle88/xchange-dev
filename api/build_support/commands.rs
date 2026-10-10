#![allow(dead_code)]

//! Pure build-support functions for generating v3 command tables (§6).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDecl {
    pub module: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandTableError {
    NotAModuleName { file: String },
    MissingRegister { file: String },
    MissingExportTypes { file: String },
    NameInBoth { module: String },
}

#[allow(clippy::duplicate_mod)]
#[path = "module_name.rs"]
mod module_name;

use module_name::is_valid_module_name;

/// (file name, source) of each file in http/v3/commands/ -> the commands, sorted by module.
pub fn commands_table(files: &[(&str, &str)]) -> Result<Vec<CommandDecl>, CommandTableError> {
    let mut decls = Vec::new();
    for &(file, source) in files {
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let module = &file[..file.len() - 3];
        if !is_valid_module_name(module) {
            return Err(CommandTableError::NotAModuleName {
                file: file.to_owned(),
            });
        }
        if !source.contains("pub fn register(") {
            return Err(CommandTableError::MissingRegister {
                file: file.to_owned(),
            });
        }
        if !source.contains("pub fn export_types(") {
            return Err(CommandTableError::MissingExportTypes {
                file: file.to_owned(),
            });
        }
        decls.push(CommandDecl {
            module: module.to_owned(),
            file: file.to_owned(),
        });
    }
    decls.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(decls)
}

/// Ensures that command modules share no module name with other modules (§12.20.3).
pub fn disjoint_names<'a, I>(
    commands: &[CommandDecl],
    other_names: I,
) -> Result<(), CommandTableError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut cmd_set = std::collections::BTreeSet::new();
    for c in commands {
        cmd_set.insert(c.module.as_str());
    }
    for name in other_names {
        if cmd_set.contains(name) {
            return Err(CommandTableError::NameInBoth {
                module: name.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CMD: &str = r#"
pub fn register(cfg: &mut actix_web::web::ServiceConfig, state: &AppState) {}
pub fn export_types(dir: &std::path::Path) -> Result<(), ts_rs::ExportError> { Ok(()) }
"#;

    #[test]
    fn ignores_mod_rs_and_non_rs_files() {
        let files = [
            ("mod.rs", VALID_CMD),
            ("README.md", "hello"),
            ("test.txt", "abc"),
            ("open_shift.rs", VALID_CMD),
        ];
        let decls = commands_table(&files).unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].module, "open_shift");
    }

    #[test]
    fn sorts_modules_alphabetically() {
        let files = [
            ("zebra_cmd.rs", VALID_CMD),
            ("alpha_cmd.rs", VALID_CMD),
            ("beta_cmd.rs", VALID_CMD),
        ];
        let decls = commands_table(&files).unwrap();
        let names: Vec<&str> = decls.iter().map(|d| d.module.as_str()).collect();
        assert_eq!(names, vec!["alpha_cmd", "beta_cmd", "zebra_cmd"]);
    }

    #[test]
    fn refuses_invalid_module_name() {
        let files = [("123bad.rs", VALID_CMD)];
        assert!(matches!(
            commands_table(&files),
            Err(CommandTableError::NotAModuleName { .. })
        ));
    }

    #[test]
    fn refuses_missing_register() {
        let files = [("cmd.rs", "pub fn export_types(...) {}")];
        assert!(matches!(
            commands_table(&files),
            Err(CommandTableError::MissingRegister { .. })
        ));
    }

    #[test]
    fn refuses_missing_export_types() {
        let files = [("cmd.rs", "pub fn register(...) {}")];
        assert!(matches!(
            commands_table(&files),
            Err(CommandTableError::MissingExportTypes { .. })
        ));
    }

    #[test]
    fn disjoint_check() {
        let files = [("cmd_a.rs", VALID_CMD)];
        let decls = commands_table(&files).unwrap();
        assert!(disjoint_names(&decls, ["view_a", "action_b"]).is_ok());
        assert!(disjoint_names(&decls, ["cmd_a"]).is_err());
    }
}
