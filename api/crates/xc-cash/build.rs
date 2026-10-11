#[path = "build_support/reconcile_checks.rs"]
mod reconcile_checks;

use std::path::Path;

fn main() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let checks_dir = Path::new(manifest_dir).join("src/application/reconcile/checks");
    let mut files = Vec::new();
    if checks_dir.exists() {
        for entry in std::fs::read_dir(&checks_dir).expect("read src/application/reconcile/checks")
        {
            let entry = entry.expect("check entry");
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
    let decls = match reconcile_checks::reconcile_checks_table(&file_refs) {
        Ok(decls) => decls,
        Err(e) => panic!("reconcile checks table error: {e}"),
    };
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let dest_path = Path::new(&out_dir).join("reconcile_checks.rs");
    let code = reconcile_checks::render(&decls, checks_dir.to_str().expect("valid utf-8 path"));
    std::fs::write(&dest_path, code).expect("failed to write reconcile_checks.rs");

    println!("cargo:rerun-if-changed=src/application/reconcile/checks");
    for (name, _) in &files {
        println!("cargo:rerun-if-changed=src/application/reconcile/checks/{name}");
    }
}
