#!/usr/bin/env bash
# Tests for api-changed-tests.sh: a pull request runs only the tests it changes or adds.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PLAN="$SCRIPT_DIR/api-changed-tests.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/metadata.json" <<'JSON'
{"packages":[
 {"name":"xchange","manifest_path":"/w/api/Cargo.toml","dependencies":[]},
 {"name":"xc-cash","manifest_path":"/w/api/crates/xc-cash/Cargo.toml","dependencies":[]}
]}
JSON

# A fake checkout: files that exist on the PR head.
R="$TMP/repo"
mkdir -p "$R/api/tests/common" "$R/api/src/shared" "$R/api/crates/xc-cash/tests" "$R/api/crates/xc-cash/src"
echo '#[tokio::test] async fn a() {}' > "$R/api/tests/rate_lock_test.rs"
echo '#[test] fn b() {}' > "$R/api/tests/booking_test.rs"
echo 'pub fn helper() {}' > "$R/api/tests/common/mod.rs"
echo 'pub fn no_tests() {}' > "$R/api/src/shared/money.rs"
printf 'pub fn f() {}\n#[cfg(test)]\nmod tests {}\n' > "$R/api/src/shared/clock.rs"
printf 'fn main() {}\n#[test]\nfn t() {}\n' > "$R/api/src/main.rs"
echo '#[sqlx::test] async fn c() {}' > "$R/api/crates/xc-cash/tests/count_test.rs"
printf '#[cfg(test)]\nmod t {}\n' > "$R/api/crates/xc-cash/src/count.rs"

pass=0
fail=0
check() {
  local name="$1" expected="$2"
  shift 2
  printf '%s\n' "$@" > "$TMP/changed.txt"
  local got
  got=$(bash "$PLAN" "$TMP/changed.txt" "$TMP/metadata.json" "$R")
  if [ "$got" = "$expected" ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL: $name" >&2
    echo "  expected: [$expected]" >&2
    echo "  got:      [$got]" >&2
  fi
}

check "a changed test file runs only that file" \
  "xchange:--test rate_lock_test" api/tests/rate_lock_test.rs
check "two test files, one package" \
  "xchange:--test booking_test --test rate_lock_test" api/tests/booking_test.rs api/tests/rate_lock_test.rs
check "a src change with no test in it runs nothing" \
  "" api/src/shared/money.rs
check "a src file that holds unit tests runs the lib tests" \
  "xchange:--lib" api/src/shared/clock.rs
check "main.rs unit tests run as --bins" \
  "xchange:--bins" api/src/main.rs
check "a deleted test file runs nothing" \
  "" api/tests/gone_test.rs
check "a shared helper under tests/common is not a test target" \
  "" api/tests/common/mod.rs
check "crate tests and crate unit tests" \
  "xc-cash:--test count_test --lib" api/crates/xc-cash/tests/count_test.rs api/crates/xc-cash/src/count.rs
check "two packages, sorted" \
  "xc-cash:--test count_test;xchange:--test rate_lock_test" api/tests/rate_lock_test.rs api/crates/xc-cash/tests/count_test.rs
check "non-Rust and non-api files run nothing" \
  "" web/src/App.tsx api/migrations/0001.sql README.md
check "the same file listed twice is one target" \
  "xchange:--test rate_lock_test" api/tests/rate_lock_test.rs ./api/tests/rate_lock_test.rs

echo "api-changed-tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
