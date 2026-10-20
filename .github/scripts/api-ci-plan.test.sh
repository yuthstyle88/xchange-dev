#!/usr/bin/env bash
# Up-front tests for the CI-scope ticket (api.yml runs only what a change can affect), written by the
# ticket author claude/studio-claude-4 before the implementation.
# Do not edit, delete or weaken these cases; add your own below the marked line.
#
# Contract under test:
#   .github/scripts/api-ci-plan.sh <event> <head-ref> <changed-paths-file> <cargo-metadata-json>
#     <event>     pull_request | push | anything else (workflow_dispatch, …)
#     <head-ref>  the PR's head branch (github.head_ref); empty for a push
#   prints exactly these five lines, in this order, and exits 0:
#     packages=<--workspace | -p a -p b … (sorted) | empty>   what clippy and nextest are given
#     run_tests=<true|false>       run the Test step
#     guards=<true|false>          also run the guard tests listed in api-guard-tests.txt
#     device_agent=<true|false>    run the device-agent-linux job's clippy and tests
#     release_check=<true|false>   run `cargo check --release --locked` (the screens-embedded build)
#   A missing or unreadable changed-paths file means "unknown change" and plans everything.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PLAN="$SCRIPT_DIR/api-ci-plan.sh"
GUARDS="$SCRIPT_DIR/api-guard-tests.txt"
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)

if [ ! -x "$PLAN" ]; then
  echo "Error: $PLAN not found or not executable" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# The shape of `cargo metadata --format-version 1 --no-deps`: xchange (root) depends on every crate it
# uses; xc-cash depends on xc-shift; xc-device-agent has no dependents; xc-kernel is under everything.
cat > "$TMP_DIR/metadata.json" <<'JSON'
{"packages":[
 {"name":"xchange","manifest_path":"/w/api/Cargo.toml","dependencies":[{"name":"xc-kernel"},{"name":"xc-shift"},{"name":"xc-cash"},{"name":"xc-auth"}]},
 {"name":"xc-kernel","manifest_path":"/w/api/crates/xc-kernel/Cargo.toml","dependencies":[]},
 {"name":"xc-auth","manifest_path":"/w/api/crates/xc-auth/Cargo.toml","dependencies":[{"name":"xc-kernel"}]},
 {"name":"xc-shift","manifest_path":"/w/api/crates/xc-shift/Cargo.toml","dependencies":[{"name":"xc-kernel"}]},
 {"name":"xc-cash","manifest_path":"/w/api/crates/xc-cash/Cargo.toml","dependencies":[{"name":"xc-shift"},{"name":"xc-kernel"}]},
 {"name":"xc-device-agent","manifest_path":"/w/api/crates/xc-device-agent/Cargo.toml","dependencies":[{"name":"reqwest"}]}
]}
JSON

pass=0
fail=0
# check <name> <event> <head-ref> <expected five lines joined by |> <changed path>...
check() {
  local name="$1" event="$2" head_ref="$3" expected="$4"
  shift 4
  printf '%s\n' "$@" > "$TMP_DIR/changed.txt"
  run_case "$name" "$event" "$head_ref" "$expected" "$TMP_DIR/changed.txt"
}
run_case() {
  local name="$1" event="$2" head_ref="$3" expected="$4" changed="$5" got
  if ! got=$("$PLAN" "$event" "$head_ref" "$changed" "$TMP_DIR/metadata.json" | paste -sd'|' -); then
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

ALL="packages=--workspace|run_tests=true"
# A non-batch PR runs no tests (BE controller ruling on #2965, owner 2026-10-17): compile only, scoped.
PR_ALL="packages=--workspace|run_tests=false"

# ── pull_request: compile only the changed crates and what depends on them; no tests ─────────────────────────────
check "pr: a leaf crate's source => that crate and its dependents; no tests" pull_request feat/x \
  "packages=-p xc-cash -p xc-shift -p xchange|run_tests=false|guards=false|device_agent=false|release_check=true" \
  api/crates/xc-shift/src/application/shift/service.rs
check "pr: the device agent alone => only it; no release check" pull_request feat/x \
  "packages=-p xc-device-agent|run_tests=false|guards=false|device_agent=true|release_check=false" \
  api/crates/xc-device-agent/src/main.rs
check "pr: a root-package test => xchange only" pull_request feat/x \
  "packages=-p xchange|run_tests=false|guards=false|device_agent=false|release_check=true" \
  api/tests/open_of_teller_view_test.rs
check "pr: a web screen => xchange (it embeds the screens)" pull_request feat/x \
  "packages=-p xchange|run_tests=false|guards=false|device_agent=false|release_check=true" \
  web/src/screens/admin/overview/Overview.tsx
check "pr: docs only => nothing to build or test" pull_request docs/x \
  "packages=|run_tests=false|guards=false|device_agent=false|release_check=false" \
  docs/design/cash-shift-transfer.md README.md
check "pr: Cargo.lock => compile everything, still no tests on a PR" pull_request feat/x \
  "$PR_ALL|guards=false|device_agent=true|release_check=true" \
  api/Cargo.lock
check "pr: api/Cargo.toml => everything" pull_request feat/x \
  "$PR_ALL|guards=false|device_agent=true|release_check=true" \
  api/Cargo.toml
check "pr: a migration => everything" pull_request feat/x \
  "$PR_ALL|guards=false|device_agent=true|release_check=true" \
  api/migrations/20261017120000_x.sql
check "pr: xc-kernel => everything" pull_request feat/x \
  "$PR_ALL|guards=false|device_agent=true|release_check=true" \
  api/crates/xc-kernel/src/money.rs
check "pr: api.yml itself => everything" pull_request ci/x \
  "$PR_ALL|guards=false|device_agent=true|release_check=true" \
  .github/workflows/api.yml

# ── push to main: changed crates + dependents, plus the guard tests ─────────────────────────────
check "push: a leaf crate => scoped packages and the guards" push "" \
  "packages=-p xc-cash -p xc-shift -p xchange|run_tests=true|guards=true|device_agent=false|release_check=true" \
  api/crates/xc-shift/src/application/shift/service.rs
check "push: the device agent alone => it, the guards, and the release check" push "" \
  "packages=-p xc-device-agent|run_tests=true|guards=true|device_agent=true|release_check=true" \
  api/crates/xc-device-agent/src/main.rs
check "push: Cargo.lock => everything and the guards" push "" \
  "$ALL|guards=true|device_agent=true|release_check=true" \
  api/Cargo.lock
run_case "push: no changed-paths file (no successful ancestor run) => everything" push "" \
  "$ALL|guards=true|device_agent=true|release_check=true" \
  "$TMP_DIR/does-not-exist.txt"

# ── a batch PR: the full suite once ──────────────────────────────────────────────────────────────
check "batch PR: one small change => the full suite and the guards" pull_request batch/merge-20261017T1500Z \
  "$ALL|guards=true|device_agent=true|release_check=true" \
  api/crates/xc-shift/src/application/shift/service.rs
check "a branch that only starts like batch is not a batch" pull_request batchy/merge \
  "packages=-p xc-cash -p xc-shift -p xchange|run_tests=false|guards=false|device_agent=false|release_check=true" \
  api/crates/xc-shift/src/application/shift/service.rs

# ── any other event plans everything (fail safe) ───────────────────────────────────────────────
check "workflow_dispatch => everything" workflow_dispatch "" \
  "$ALL|guards=true|device_agent=true|release_check=true" \
  api/crates/xc-shift/src/application/shift/service.rs

# ── the guard list names real test binaries ───────────────────────────────────────────────────
if [ ! -s "$GUARDS" ]; then
  echo "FAIL guard list: $GUARDS missing or empty"
  fail=$((fail + 1))
else
  bad=0
  while read -r pkg bin; do
    case "$pkg" in ''|'#'*) continue ;; esac
    if [ "$pkg" = "xchange" ]; then f="$REPO_ROOT/api/tests/$bin.rs"; else f="$REPO_ROOT/api/crates/$pkg/tests/$bin.rs"; fi
    if [ ! -f "$f" ]; then echo "FAIL guard list: $pkg $bin has no file $f"; bad=1; fi
  done < "$GUARDS"
  if [ "$bad" = 0 ]; then echo "ok   guard list: every entry is an existing test file"; pass=$((pass + 1)); else fail=$((fail + 1)); fi
fi

# ── add your own cases below this line ────────────────────────────────────────────────────────
check "push: only .github/workflows/api.yml => packages empty, run_tests=false, guards still run (#3086)" push "" \
  "packages=|run_tests=false|guards=true|device_agent=false|release_check=true" \
  .github/workflows/api.yml
check "push: .github/scripts/x.sh + api/src/.../admin_cash.rs => -p xchange, not --workspace (#3086)" push "" \
  "packages=-p xchange|run_tests=true|guards=true|device_agent=false|release_check=true" \
  .github/scripts/x.sh api/src/presentation/http/v3/views/admin_cash.rs
check "push: api/Cargo.lock still plans --workspace (#3086)" push "" \
  "$ALL|guards=true|device_agent=true|release_check=true" \
  api/Cargo.lock

# ── #3895 provided tests (claude/a90ab16c): a manual run takes a package list ─────────────────────────
# Contract: on workflow_dispatch only, a non-empty DISPATCH_PACKAGES (space-separated package names, the
# `packages` dispatch input) replaces --workspace with exactly those packages, sorted; run_tests=true,
# guards=false, device_agent=true only when xc-device-agent is named, release_check=true (the pages tests
# read a freshly built web/dist). A name not in cargo metadata fails the step, naming it; never a silent
# full run. Empty or unset => today's full plan. Every other event ignores it.
# The `filter` input is applied by api.yml's Test step, not by this script.
# The implementer must not edit, delete or weaken these cases.
dcheck() { # dcheck <name> <event> <DISPATCH_PACKAGES> <expected>
  local name="$1" event="$2" pk="$3" expected="$4" got
  printf '%s\n' api/crates/xc-shift/src/application/shift/service.rs > "$TMP_DIR/changed.txt"
  if ! got=$(DISPATCH_PACKAGES="$pk" "$PLAN" "$event" "" "$TMP_DIR/changed.txt" "$TMP_DIR/metadata.json" | paste -sd'|' -); then
    echo "FAIL $name: exit non-zero"; fail=$((fail + 1)); return
  fi
  if [ "$got" = "$expected" ]; then echo "ok   $name"; pass=$((pass + 1))
  else echo "FAIL $name"; echo "     expected: [$expected]"; echo "     got:      [$got]"; fail=$((fail + 1)); fi
}
dcheck "dispatch: empty packages input => everything" workflow_dispatch "" \
  "$ALL|guards=true|device_agent=true|release_check=true"
dcheck "dispatch: packages input => exactly those, sorted, tests on" workflow_dispatch "xchange xc-cash" \
  "packages=-p xc-cash -p xchange|run_tests=true|guards=false|device_agent=false|release_check=true"
dcheck "dispatch: extra spaces and repeats are tolerated" workflow_dispatch "  xc-cash   xchange xc-cash " \
  "packages=-p xc-cash -p xchange|run_tests=true|guards=false|device_agent=false|release_check=true"
dcheck "dispatch: naming the device agent turns its job on" workflow_dispatch "xc-device-agent" \
  "packages=-p xc-device-agent|run_tests=true|guards=false|device_agent=true|release_check=true"
dcheck "schedule ignores the input => everything" schedule "xc-cash" \
  "$ALL|guards=true|device_agent=true|release_check=true"
dcheck "pull_request ignores the input" pull_request "xc-cash" \
  "packages=-p xc-cash -p xc-shift -p xchange|run_tests=false|guards=false|device_agent=false|release_check=true"
# an unknown package fails, naming it
printf '%s\n' api/crates/xc-shift/src/application/shift/service.rs > "$TMP_DIR/changed.txt"
if out=$(DISPATCH_PACKAGES="xc-cash xc-nope" "$PLAN" workflow_dispatch "" "$TMP_DIR/changed.txt" "$TMP_DIR/metadata.json" 2>&1); then
  echo "FAIL dispatch: an unknown package must fail the step, got exit 0: $out"; fail=$((fail + 1))
elif printf '%s' "$out" | grep -q 'xc-nope'; then
  echo "ok   dispatch: an unknown package fails, naming it"; pass=$((pass + 1))
else
  echo "FAIL dispatch: the failure does not name xc-nope: $out"; fail=$((fail + 1))
fi

echo "passed: $pass, failed: $fail"
[ "$fail" = 0 ]
