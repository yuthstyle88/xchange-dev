#!/usr/bin/env bash
# Decide which Rust test targets a pull request runs: only the tests it changes or adds.
# The owner's order (2026-10-16): run the tests that changed or were added, not the old ones;
# the full suite runs on main pushes and on batch-merge PRs instead.
#
# Contract:
#   api-changed-tests.sh <changed-paths-file> <cargo-metadata-json> [<repo-root>]
# Output: exactly one line on stdout:
#   <pkg>:<targets>[;<pkg>:<targets>...]   e.g. "xchange:--test a --test b;xc-cash:--lib"
#   <empty line>                            no test file changed or added => run no tests
# <repo-root> (default ".") is where changed files are read from: a deleted test file is
# skipped, and a changed src/ file counts only if it still contains a unit test.
# Exit code is always 0. Diagnostics go to stderr only.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 <changed-paths-file> <cargo-metadata-json> [<repo-root>]" >&2
  exit 1
fi

exec python3 - "$1" "$2" "${3:-.}" <<'PY'
import json
import os
import re
import sys

changed_file, metadata_file, root = sys.argv[1], sys.argv[2], sys.argv[3]

try:
    with open(metadata_file, encoding="utf-8") as f:
        packages = json.load(f).get("packages", [])
    with open(changed_file, encoding="utf-8") as f:
        paths = [l.strip().removeprefix("./") for l in f if l.strip()]
except Exception as e:
    sys.stderr.write(f"Error reading inputs: {e}\n")
    print("")
    sys.exit(0)

def pkg_for_dir(crate_dir):
    for p in packages:
        m = p.get("manifest_path", "").replace("\\", "/")
        if m.endswith(f"/api/crates/{crate_dir}/Cargo.toml") or m == f"api/crates/{crate_dir}/Cargo.toml":
            return p.get("name")
    return None

UNIT_TEST = re.compile(r"#\[(?:[\w:]+::)?test\b|#\[cfg\(test\)\]")

targets = {}  # package -> ordered list of cargo target args

def add(pkg, arg):
    lst = targets.setdefault(pkg, [])
    if arg not in lst:
        lst.append(arg)

for path in paths:
    if not path.endswith(".rs"):
        continue
    full = os.path.join(root, path)
    if not os.path.isfile(full):
        continue  # deleted: nothing left to run
    if path.startswith("api/crates/"):
        crate_dir, _, rest = path[len("api/crates/"):].partition("/")
        pkg = pkg_for_dir(crate_dir)
    elif path.startswith("api/"):
        pkg, rest = "xchange", path[len("api/"):]
    else:
        continue
    if pkg is None:
        continue
    m = re.fullmatch(r"tests/([^/]+)\.rs", rest)
    if m:
        add(pkg, f"--test {m.group(1)}")
        continue
    if rest.startswith("src/"):
        try:
            with open(full, encoding="utf-8") as f:
                has_tests = UNIT_TEST.search(f.read()) is not None
        except Exception:
            has_tests = False
        if has_tests:
            add(pkg, "--bins" if rest == "src/main.rs" or rest.startswith("src/bin/") else "--lib")

print(";".join(f"{pkg}:{' '.join(args)}" for pkg, args in sorted(targets.items())))
PY
