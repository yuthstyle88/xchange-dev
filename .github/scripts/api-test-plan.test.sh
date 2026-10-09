#!/usr/bin/env bash
# Up-front tests for #695 (CI-T1), written by the ticket author claude/608dff before the implementation.
# Do not edit, delete or weaken these cases; add your own below the marked line.
#
# Contract under test:
#   .github/scripts/api-test-plan.sh <changed-paths-file> <cargo-metadata-json>
#   prints ONE line on stdout: the arguments for `cargo test` in api/, or an empty line when
#   no Rust test can be affected. Exit 0 in every case below.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PLAN="$SCRIPT_DIR/api-test-plan.sh"

if [ ! -x "$PLAN" ]; then
  echo "Error: $PLAN not found or not executable" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# A small workspace graph in the shape of `cargo metadata --format-version 1 --no-deps`.
# xchange (the root package) depends on xc-auth, xc-kernel, xc-low via xc-mid; xc-approval depends on
# xc-auth; xc-top -> xc-mid -> xc-low is a transitive chain; xc-device-agent has no dependents.
cat > "$TMP_DIR/metadata.json" <<'JSON'
{"packages":[
 {"name":"xchange","manifest_path":"/w/api/Cargo.toml","dependencies":[{"name":"xc-auth"},{"name":"xc-kernel"},{"name":"xc-mid"},{"name":"serde"}]},
 {"name":"xc-kernel","manifest_path":"/w/api/crates/xc-kernel/Cargo.toml","dependencies":[]},
 {"name":"xc-auth","manifest_path":"/w/api/crates/xc-auth/Cargo.toml","dependencies":[{"name":"xc-kernel"}]},
 {"name":"xc-approval","manifest_path":"/w/api/crates/xc-approval/Cargo.toml","dependencies":[{"name":"xc-auth"},{"name":"xc-kernel"}]},
 {"name":"xc-device-agent","manifest_path":"/w/api/crates/xc-device-agent/Cargo.toml","dependencies":[{"name":"reqwest"}]},
 {"name":"xc-low","manifest_path":"/w/api/crates/xc-low/Cargo.toml","dependencies":[{"name":"xc-kernel"}]},
 {"name":"xc-mid","manifest_path":"/w/api/crates/xc-mid/Cargo.toml","dependencies":[{"name":"xc-low"}]},
 {"name":"xc-top","manifest_path":"/w/api/crates/xc-top/Cargo.toml","dependencies":[{"name":"xc-mid"}]}
]}
JSON

pass=0
fail=0
check() {
  local name="$1" expected="$2"
  shift 2
  printf '%s\n' "$@" > "$TMP_DIR/changed.txt"
  local got
  if ! got=$("$PLAN" "$TMP_DIR/changed.txt" "$TMP_DIR/metadata.json"); then
    echo "FAIL $name: exit non-zero"
    fail=$((fail + 1))
    return
  fi
  if [ "$got" = "$expected" ]; then
    echo "ok   $name"
    pass=$((pass + 1))
  else
    echo "FAIL $name"
    echo "     expected: [$expected]"
    echo "     got:      [$got]"
    fail=$((fail + 1))
  fi
}

# --- Nothing a Rust test can see ---------------------------------------------------------------
check "docs only => no Rust test"                 "" "docs/design/x.md" "README.md"
check "empty change list => no Rust test"         ""

# --- Everything: shared inputs every package reads ---------------------------------------------
check "Cargo.lock => workspace"                   "--workspace" "api/Cargo.lock"
check "root Cargo.toml => workspace"              "--workspace" "api/Cargo.toml"
check "a migration => workspace (crates' DB tests run api/migrations)" "--workspace" "api/migrations/20261012100000_x.sql"
check "build.rs => workspace"                     "--workspace" "api/build.rs"
check "build_support => workspace"                "--workspace" "api/build_support/screens.rs"
check "xc-kernel => workspace (every crate depends on it)" "--workspace" "api/crates/xc-kernel/src/lib.rs"
check "the api workflow => workspace"             "--workspace" ".github/workflows/api.yml"
check "a CI script => workspace"                  "--workspace" ".github/scripts/should-build.sh"
check "rust-toolchain => workspace"               "--workspace" "api/rust-toolchain.toml"
check "an unknown path under api/ => workspace (fail safe)" "--workspace" "api/something-new.txt"
check "a crate not in the metadata => workspace (fail safe)" "--workspace" "api/crates/xc-ghost/src/lib.rs"
check "workspace wins over everything else"       "--workspace" "web/src/a.ts" "api/src/a.rs" "api/Cargo.lock"

# --- The root package only: crates never depend on it ------------------------------------------
check "api/src => root package"                   "-p xchange" "api/src/presentation/http/v3/command.rs"
check "api/tests => root package"                 "-p xchange" "api/tests/c_plain_test.rs"
check "api/static => root package"                "-p xchange" "api/static/xc-forms.js"
check "api/web/forms => root package"             "-p xchange" "api/web/forms/src/index.ts"

# --- A crate: itself plus every package that depends on it, transitively -----------------------
check "xc-auth => itself + xc-approval + xchange" "-p xc-approval -p xc-auth -p xchange" "api/crates/xc-auth/src/token/jwk.rs"
check "a crate's own Cargo.toml counts as that crate" "-p xc-approval -p xc-auth -p xchange" "api/crates/xc-auth/Cargo.toml"
check "a crate's tests count as that crate"       "-p xc-approval -p xc-auth -p xchange" "api/crates/xc-auth/tests/token_verifier_test.rs"
check "no dependents => itself only"              "-p xc-device-agent" "api/crates/xc-device-agent/src/main.rs"
check "transitive dependents are included"       "-p xc-low -p xc-mid -p xc-top -p xchange" "api/crates/xc-low/src/lib.rs"
check "two crates => union, sorted, no repeats"   "-p xc-approval -p xc-auth -p xc-device-agent -p xchange" "api/crates/xc-device-agent/src/a.rs" "api/crates/xc-auth/src/a.rs" "api/crates/xc-auth/src/b.rs"

# --- web/ only: the root package, because it embeds web/dist at compile time -------------------
# (api/src/presentation/ui.rs: #[folder = "../web/dist/"]), so any root test can see a web change.
# No crate depends on web/, so crates are skipped.
check "web only => root package"                  "-p xchange" "web/src/screens/devices/devices.css" "web/src/shared/ui/AppShell.tsx"
check "web + a crate => that crate + root"        "-p xc-device-agent -p xchange" "web/src/a.css" "api/crates/xc-device-agent/src/a.rs"
check "web + api/src => root package"             "-p xchange" "web/src/a.css" "api/src/a.rs"
check "web + docs => same as web only"            "-p xchange" "web/src/a.css" "docs/a.md"
check "web/package-lock.json => root package"     "-p xchange" "web/package-lock.json"

# --- Implementer's own cases go below this line -------------------------------------------------

echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
