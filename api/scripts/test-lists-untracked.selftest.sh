#!/usr/bin/env bash
# Up-front check for #4194: adding an integration-test file must not require editing any tracked file.
# Written before the implementation by AG controller 4093 (claude/studio-claude-7). Do not edit, delete or weaken.
#
# 1. No tracked tests/all.rs or tests/all_<k>.rs lists a test module (`#[path = "<file>.rs"]`): the lists are
#    generated at build time into OUT_DIR.
# 2. A new test file dropped into a package's tests/ (here xc-kernel) is compiled and listed by `cargo test` with no
#    other change, and is gone again once the file is removed.
# 3. The build writes nothing under api/ (the lists live in OUT_DIR).
# Run from the repo root: bash api/scripts/test-lists-untracked.selftest.sh
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"

fail=0
listed=$(git ls-files -- 'api/tests/all*.rs' 'api/crates/*/tests/all*.rs' | xargs grep -l '#\[path = "' 2>/dev/null || true)
if [ -n "$listed" ]; then
  echo "FAIL 1: tracked test runners still list modules (each new test file must edit them):"
  echo "$listed" | sed 's/^/  /'
  fail=1
else
  echo "ok 1: no tracked runner lists a test module"
fi

before=$(git status --porcelain -- api)
probe=api/crates/xc-kernel/tests/zz_4194_probe_test.rs
trap 'rm -f "$probe"' EXIT
printf '#[test]\nfn zz_4194_probe_is_compiled() {}\n' > "$probe"
if (cd api && cargo test -q -p xc-kernel --test all -- --list 2>/dev/null) | grep -q 'zz_4194_probe_is_compiled'; then
  echo "ok 2: a new test file is compiled with no tracked edit"
else
  echo "FAIL 2: a new test file in api/crates/xc-kernel/tests/ is not compiled until a tracked runner is edited"
  fail=1
fi
rm -f "$probe"
after=$(git status --porcelain -- api)
if [ "$before" != "$after" ]; then
  echo "FAIL 3: building changed files under api/:"; diff <(echo "$before") <(echo "$after") | sed 's/^/  /'
  fail=1
else
  echo "ok 3: the build changed no file under api/"
fi
exit $fail
