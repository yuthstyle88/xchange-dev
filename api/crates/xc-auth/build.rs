#[path = "build_support/area_rows.rs"]
mod area_rows;

use std::path::Path;

fn main() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let areas_dir = Path::new(manifest_dir).join("src/domain/policy/areas");
    let mut files = Vec::new();
    if areas_dir.exists() {
        for entry in std::fs::read_dir(&areas_dir).expect("read src/domain/policy/areas") {
            let entry = entry.expect("area entry");
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
    let decls = match area_rows::area_rows_table(&file_refs) {
        Ok(decls) => decls,
        Err(e) => panic!("area rows table error: {e:?}"),
    };
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let dest_path = Path::new(&out_dir).join("area_rows.rs");
    let code = area_rows::render(&decls, areas_dir.to_str().expect("valid utf-8 path"));
    std::fs::write(&dest_path, code).expect("failed to write area_rows.rs");

    println!("cargo:rerun-if-changed=src/domain/policy/areas");
    for (name, _) in &files {
        println!("cargo:rerun-if-changed=src/domain/policy/areas/{name}");
    }

    xc_testlist::emit(1);
}
