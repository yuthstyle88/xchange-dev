#![allow(dead_code)]

//! Pure build-support functions for the policy-rows seam (PL-0b #1014,
//! `docs/design/blueprint/broker-seam.md` §11.1): `xc-auth/build.rs` turns the files of
//! `src/domain/policy/areas/` into the generated `AREA_ROWS`, so an area adds its permission rows by
//! adding its own file and never edits `policy.rs`.

/// One area file: its module name and its file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaDecl {
    pub module: String,
    pub file: String,
}

/// Why an area file is refused. The build fails and names the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AreaTableError {
    /// The file stem is not a Rust module name (the rule of `api/build_support/views.rs`).
    NotAModuleName { file: String },
    /// The source has no `pub const ROWS: &[AreaRow]`.
    MissingRows { file: String },
}

fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}

fn is_valid_module_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return false;
    }
    !is_rust_keyword(name)
}

/// (file name, source) of each file in `src/domain/policy/areas/` → the area files, sorted by module.
/// `mod.rs` and files not ending in `.rs` are skipped.
pub fn area_rows_table(files: &[(&str, &str)]) -> Result<Vec<AreaDecl>, AreaTableError> {
    let mut decls = Vec::new();
    for &(file, source) in files {
        if file == "mod.rs" || !file.ends_with(".rs") {
            continue;
        }
        let module = &file[..file.len() - 3];
        if !is_valid_module_name(module) {
            return Err(AreaTableError::NotAModuleName {
                file: file.to_string(),
            });
        }
        if !source.contains("pub const ROWS: &[AreaRow]") {
            return Err(AreaTableError::MissingRows {
                file: file.to_string(),
            });
        }
        decls.push(AreaDecl {
            module: module.to_string(),
            file: file.to_string(),
        });
    }
    decls.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(decls)
}

/// The generated source for `$OUT_DIR/area_rows.rs`, included by `areas/mod.rs`: one
/// `#[path = "<dir>/<file>"]` + `pub mod <module>;` per area, then
/// `pub static AREA_ROWS: &[&[AreaRow]] = &[<module>::ROWS, …];` in the table's order.
pub fn render(decls: &[AreaDecl], dir: &str) -> String {
    let mut out = String::new();
    for decl in decls {
        out.push_str(&format!(
            "#[path = \"{}/{}\"]\npub mod {};\n",
            dir, decl.file, decl.module
        ));
    }
    out.push_str("pub static AREA_ROWS: &[&[AreaRow]] = &[");
    for (i, decl) in decls.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{}::ROWS", decl.module));
    }
    out.push_str("];\n");
    out
}
