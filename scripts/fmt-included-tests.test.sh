#!/usr/bin/env bash
# #4884 self-test for integration-test formatting check in scripts/fmt-included.sh.
#
# Covers:
#   T1 (a) an unformatted changed file under api/tests/ fails
#   T2 (b) an unformatted changed file under api/crates/x/tests/sub/ fails
#   T3 (c) a formatted changed test file passes
#   T4 (d) an unformatted test file that did not change passes
set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SUT="$ROOT/scripts/fmt-included.sh"
[ -f "$SUT" ] || { echo "Error: $SUT not found" >&2; exit 1; }
command -v rustfmt >/dev/null || { echo "Error: rustfmt not on PATH" >&2; exit 1; }

TMP_BASE=${XC_TEST_TMP:-${RUNNER_TEMP:-$PWD}}
TMP_ROOT=$(mktemp -d "$TMP_BASE/fmt-included-tests.XXXXXX")
trap 'rm -rf "$TMP_ROOT"' EXIT

export GIT_CONFIG_GLOBAL="$TMP_ROOT/gitconfig"
export GIT_CONFIG_NOSYSTEM=1
printf '[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n' >"$GIT_CONFIG_GLOBAL"

UNFMT='pub fn f( )->u8{1}'
FMT=$'pub fn f() -> u8 {\n    1\n}'

pass=0
fail=0
ok() { echo "ok   $1"; pass=$((pass + 1)); }
bad() { echo "FAIL $1: $2"; fail=$((fail + 1)); }

new_repo() {
  T=$(mktemp -d "$TMP_ROOT/case.XXXXXX")
  mkdir -p "$T/scripts" "$T/api/tests" "$T/api/crates/x/tests/sub"
  cp "$SUT" "$T/scripts/fmt-included.sh"
  cp "$ROOT/api/build.rs" "$T/api/build.rs"
  for b in "$ROOT"/api/crates/*/build.rs; do
    c=${b#"$ROOT/"}
    mkdir -p "$T/$(dirname "$c")"
    cp "$b" "$T/$c"
  done
  printf '%s\n' "$FMT" >"$T/api/tests/formatted_test.rs"
  printf '%s\n' "$UNFMT" >"$T/api/tests/legacy_test.rs"
  printf '%s\n' "$UNFMT" >"$T/api/crates/x/tests/sub/legacy_crate_test.rs"
  git -C "$T" init -q
  git -C "$T" add -A
  git -C "$T" commit -qm base
  git -C "$T" tag base
}

commit() { git -C "$T" add -A && git -C "$T" commit -qm "$1"; }

run() {
  local errf="$T/.err" outf="$T/.out"
  (cd "$T" && bash scripts/fmt-included.sh "$1") >"$outf" 2>"$errf"
  RC=$?
  ERR=$(cat "$errf")
  OUT="$(cat "$outf")$ERR"
}

# T1 (a): an unformatted changed file under api/tests/ fails
new_repo
printf '%s\n' "$UNFMT" >"$T/api/tests/unfmt_root.rs"
commit "add unformatted test under api/tests/"
run base
if [ "$RC" -eq 1 ] && grep -q 'unfmt_root.rs' <<<"$OUT"; then ok T1; else bad T1 "want exit 1 naming unfmt_root.rs, got exit $RC: $OUT"; fi

# T2 (b): an unformatted changed file under api/crates/x/tests/sub/ fails
new_repo
printf '%s\n' "$UNFMT" >"$T/api/crates/x/tests/sub/unfmt_sub.rs"
commit "add unformatted test under api/crates/x/tests/sub/"
run base
if [ "$RC" -eq 1 ] && grep -q 'unfmt_sub.rs' <<<"$OUT"; then ok T2; else bad T2 "want exit 1 naming unfmt_sub.rs, got exit $RC: $OUT"; fi

# T3 (c): a formatted changed test file passes
new_repo
printf '%s\n' "$FMT" '' 'pub fn g() {}' >"$T/api/tests/formatted_test.rs"
printf '%s\n' "$FMT" >"$T/api/crates/x/tests/sub/formatted_sub.rs"
commit "edit/add formatted test files"
run base
if [ "$RC" -eq 0 ]; then ok T3; else bad T3 "formatted test change must pass, got exit $RC: $OUT"; fi

# T4 (d): an unformatted test file that did not change passes
new_repo
echo readme >"$T/README.md"
commit "touch no test file"
run base
if [ "$RC" -eq 0 ]; then ok T4; else bad T4 "unchanged unformatted test files must pass, got exit $RC: $OUT"; fi

echo "fmt-included-tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
