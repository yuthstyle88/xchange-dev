#[path = "build_support/template_table.rs"]
mod template_table;

use std::path::Path;

fn main() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let templates_dir = Path::new(manifest_dir).join("src/templates");
    let mut files = Vec::new();
    if templates_dir.exists() {
        for entry in std::fs::read_dir(&templates_dir).expect("read src/templates") {
            let entry = entry.expect("template entry");
            let path = entry.path();
            if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .expect("utf-8 file name")
                    .to_owned();
                let content = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
                files.push((name, content));
            }
        }
    }
    let file_refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let decls = match template_table::template_table(&file_refs) {
        Ok(decls) => decls,
        Err(e) => panic!("template table error: {e:?}"),
    };
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let dest_path = Path::new(&out_dir).join("templates.rs");
    let code = template_table::render(&decls, templates_dir.to_str().expect("valid utf-8 path"));
    std::fs::write(&dest_path, code).expect("failed to write templates.rs");

    println!("cargo:rerun-if-changed=src/templates");
    for (name, _) in &files {
        println!("cargo:rerun-if-changed=src/templates/{name}");
    }
    println!("cargo:rerun-if-changed=build_support/template_table.rs");

    xc_testlist::emit(1);
}
