#!/usr/bin/env bash
# Decide which Rust packages in api/ need testing for a set of changed paths.
# Contract:
#   api-test-plan.sh <changed-paths-file> <cargo-metadata-json>
# Output: exactly one line on stdout:
#   --workspace
#   -p <pkg> ... (sorted, no repeats)
#   <empty line> (no Rust test affected)
# Exit code is always 0. Diagnostics go to stderr only.

set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 <changed-paths-file> <cargo-metadata-json>" >&2
  exit 1
fi

CHANGED_FILE="$1"
METADATA_FILE="$2"

if [ ! -f "$CHANGED_FILE" ]; then
  echo "Error: changed paths file not found: $CHANGED_FILE" >&2
  exit 1
fi

if [ ! -f "$METADATA_FILE" ]; then
  echo "Error: cargo metadata file not found: $METADATA_FILE" >&2
  exit 1
fi

exec python3 - "$CHANGED_FILE" "$METADATA_FILE" <<'PY'
import json
import sys

changed_file = sys.argv[1]
metadata_file = sys.argv[2]

try:
    with open(metadata_file, "r", encoding="utf-8") as f:
        metadata = json.load(f)
except Exception as e:
    sys.stderr.write(f"Error reading metadata: {e}\n")
    print("--workspace")
    sys.exit(0)

packages = metadata.get("packages", [])

def find_pkg_by_dir(dir_name):
    suffix = f"/api/crates/{dir_name}/Cargo.toml"
    for p in packages:
        m = p.get("manifest_path", "").replace("\\", "/")
        if m.endswith(suffix) or m == f"api/crates/{dir_name}/Cargo.toml":
            return p.get("name")
    return None

# Build reverse dependency map: dependency_name -> set of package_names that depend on it
dependents = {p["name"]: set() for p in packages if "name" in p}
for p in packages:
    pkg_name = p.get("name")
    if not pkg_name:
        continue
    for dep in p.get("dependencies", []):
        dep_name = dep.get("name")
        if dep_name in dependents:
            dependents[dep_name].add(pkg_name)

def get_transitive_dependents(pkg_name):
    result = set()
    queue = [pkg_name]
    visited = {pkg_name}
    while queue:
        curr = queue.pop(0)
        result.add(curr)
        for dep in dependents.get(curr, []):
            if dep not in visited:
                visited.add(dep)
                queue.append(dep)
    return result

try:
    with open(changed_file, "r", encoding="utf-8") as f:
        lines = f.readlines()
except Exception as e:
    sys.stderr.write(f"Error reading changed paths: {e}\n")
    print("--workspace")
    sys.exit(0)

needs_workspace = False
affected_packages = set()

for line in lines:
    path = line.strip()
    if not path:
        continue
    if path.startswith("./"):
        path = path[2:]

    # 1. Shared CI / build / migration files that affect every crate
    if path in ("api/Cargo.lock", "api/Cargo.toml", "api/build.rs") or \
       path.startswith("api/rust-toolchain") or \
       path.startswith("api/migrations/") or \
       path.startswith("api/build_support/") or \
       path.startswith("api/.cargo/") or \
       path == ".github/workflows/api.yml" or \
       path == ".github/scripts" or \
       path.startswith(".github/scripts/"):
        needs_workspace = True
        break

    # 2. Crates under api/crates/
    if path.startswith("api/crates/"):
        rest = path[len("api/crates/"):].strip("/")
        parts = rest.split("/")
        dir_name = parts[0] if parts else ""
        pkg_name = find_pkg_by_dir(dir_name) if dir_name else None
        if pkg_name is None:
            # crate dir not in metadata => workspace (fail safe)
            needs_workspace = True
            break
        if pkg_name == "xc-kernel" or dir_name == "xc-kernel":
            # xc-kernel affects every crate
            needs_workspace = True
            break
        affected_packages.update(get_transitive_dependents(pkg_name))
        continue

    # 3. Root package triggers
    if path.startswith("api/src/") or \
       path.startswith("api/tests/") or \
       path.startswith("api/static/") or \
       path.startswith("api/web/forms/") or \
       path.startswith("web/"):
        affected_packages.add("xchange")
        continue

    # 4. Any other path under api/ => workspace (fail safe)
    if path.startswith("api/"):
        needs_workspace = True
        break

    # 5. Any path outside api/, web/, or watched CI files => does not affect Rust tests

if needs_workspace:
    print("--workspace")
elif affected_packages:
    sorted_pkgs = sorted(affected_packages)
    print(" ".join(f"-p {p}" for p in sorted_pkgs))
else:
    print("")

sys.exit(0)
PY
