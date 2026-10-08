#![allow(dead_code)]

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Anyone,
    SignedIn,
    Branch,
    Front,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenDecl {
    pub path: String,
    pub gate: Gate,
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenTableError {
    DuplicatePath {
        path: String,
        first: String,
        second: String,
    },
    MissingPath {
        entry: String,
    },
    MissingGate {
        entry: String,
    },
    UnknownGate {
        entry: String,
        gate: String,
    },
}

fn extract_meta(html: &str, name: &str) -> Option<String> {
    let needle = format!("<meta name=\"{}\" content=\"", name);
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// (entry path relative to dist, e.g. "pages/login.html", its HTML) -> the table.
pub fn screen_table(entries: &[(&str, &str)]) -> Result<Vec<ScreenDecl>, ScreenTableError> {
    let mut decls = Vec::with_capacity(entries.len());

    for &(entry_path, html) in entries {
        let path = match extract_meta(html, "xc-path") {
            Some(p) => p,
            None => {
                return Err(ScreenTableError::MissingPath {
                    entry: entry_path.to_string(),
                });
            }
        };

        let gate_str = match extract_meta(html, "xc-gate") {
            Some(g) => g,
            None => {
                return Err(ScreenTableError::MissingGate {
                    entry: entry_path.to_string(),
                });
            }
        };

        let gate = match gate_str.as_str() {
            "anyone" => Gate::Anyone,
            "signed-in" => Gate::SignedIn,
            "branch" => Gate::Branch,
            "front" => Gate::Front,
            other => {
                return Err(ScreenTableError::UnknownGate {
                    entry: entry_path.to_string(),
                    gate: other.to_string(),
                });
            }
        };

        if let Some(existing) = decls.iter().find(|d: &&ScreenDecl| d.path == path) {
            return Err(ScreenTableError::DuplicatePath {
                path,
                first: existing.entry.clone(),
                second: entry_path.to_string(),
            });
        }

        decls.push(ScreenDecl {
            path,
            gate,
            entry: entry_path.to_string(),
        });
    }

    Ok(decls)
}

/// Err when profile == "release" and dist/.vite/manifest.json is not a file.
pub fn check_dist(profile: &str, dist: &Path) -> Result<(), String> {
    if profile == "release" {
        let manifest = dist.join(".vite/manifest.json");
        if !manifest.is_file() {
            return Err(format!(
                "release build requires {}, which was not found",
                manifest.display()
            ));
        }
    }
    Ok(())
}

/// Creates dist (and its parents) when missing; leaves an existing one untouched.
pub fn ensure_dist(dist: &Path) -> std::io::Result<()> {
    if !dist.exists() {
        std::fs::create_dir_all(dist)?;
    }
    Ok(())
}

/// Every dist/pages/*.html as ("pages/<name>.html", contents), sorted by name; empty when dist/pages is absent.
pub fn read_entries(dist: &Path) -> std::io::Result<Vec<(String, String)>> {
    let pages_dir = dist.join("pages");
    if !pages_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(pages_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file() || file_type.is_symlink() {
            let file_name = entry.file_name();
            let name_lossy = file_name.to_string_lossy();
            if name_lossy.ends_with(".html") {
                let rel = format!("pages/{}", name_lossy);
                let content = std::fs::read_to_string(entry.path())?;
                entries.push((rel, content));
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}
