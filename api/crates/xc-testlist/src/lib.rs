use std::path::{Path, PathBuf};

fn is_runner(name: &str) -> bool {
    if name == "all.rs" {
        return true;
    }
    if let Some(rest) = name
        .strip_prefix("all_")
        .and_then(|s| s.strip_suffix(".rs"))
    {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit());
    }
    false
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn render(tests_dir: &Path, files: &[String]) -> String {
    let mut out = String::new();
    if files
        .iter()
        .any(|f| f == "campaigns_view_expired_boundary_test.rs")
    {
        out.push_str("pub use campaigns_view_expired_boundary_test::{bootstrap, presentation};\n");
    }
    for f in files {
        let stem = &f[..f.len() - 3];
        let abs = tests_dir.join(f);
        let abs_str = abs.to_str().expect("utf-8 test path");
        out.push_str(&format!("#[path = {abs_str:?}]\nmod {stem};\n"));
    }
    out
}

/// Emit `OUT_DIR/test_list_all.rs` (`partitions == 1`) or `OUT_DIR/test_list_all_1.rs`..`all_N.rs`
/// (`partitions == N`) from `CARGO_MANIFEST_DIR/tests/*.rs`.
pub fn emit(partitions: u32) {
    assert!(partitions >= 1, "partitions must be >= 1");
    println!("cargo:rerun-if-changed=tests");

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo"),
    );
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let tests_dir = manifest_dir.join("tests");

    let mut files = Vec::new();
    if tests_dir.is_dir() {
        for entry in std::fs::read_dir(&tests_dir).expect("read tests/") {
            let entry = entry.expect("tests/ dir entry");
            if !entry.file_type().expect("tests/ file type").is_file() {
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .expect("utf-8 test file name");
            if name.ends_with(".rs") && !is_runner(&name) {
                files.push(name);
            }
        }
    }
    files.sort();

    if partitions == 1 {
        let code = render(&tests_dir, &files);
        std::fs::write(out_dir.join("test_list_all.rs"), code)
            .expect("write OUT_DIR/test_list_all.rs");
    } else {
        let mut chunks: Vec<Vec<String>> = vec![Vec::new(); partitions as usize];
        for f in files {
            let idx = (crc32(f.as_bytes()) % partitions) as usize;
            chunks[idx].push(f);
        }
        for (idx, chunk) in chunks.iter().enumerate() {
            let k = idx + 1;
            let code = render(&tests_dir, chunk);
            std::fs::write(out_dir.join(format!("test_list_all_{k}.rs")), code)
                .unwrap_or_else(|e| panic!("write OUT_DIR/test_list_all_{k}.rs: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{crc32, is_runner};

    #[test]
    fn crc32_matches_zlib_and_runner_filter_skips_only_runners() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            1 + (crc32(b"campaigns_view_expired_boundary_test.rs") % 4),
            1
        );
        assert!(is_runner("all.rs"));
        assert!(is_runner("all_1.rs"));
        assert!(is_runner("all_4.rs"));
        assert!(!is_runner("all_test.rs"));
        assert!(!is_runner("bounded_test.rs"));
    }
}
