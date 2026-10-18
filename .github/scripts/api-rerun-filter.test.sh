#!/usr/bin/env bash
# Self-test for .github/scripts/api-rerun-filter.sh (#3079).
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
FILTER_SCRIPT="$SCRIPT_DIR/api-rerun-filter.sh"

if [ ! -x "$FILTER_SCRIPT" ]; then
  echo "Error: $FILTER_SCRIPT not found or not executable" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

cat > "$TMP_DIR/junit-2-failures.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="4" failures="2" errors="0">
  <testsuite name="xchange::money_silent_zero_ratchet_test" tests="2" failures="1" errors="0">
    <testcase name="ceilings_hold" classname="xchange::money_silent_zero_ratchet_test" time="0.120">
      <failure message="assertion failed">thread 'ceilings_hold' panicked</failure>
    </testcase>
    <testcase name="clean_file_passes" classname="xchange::money_silent_zero_ratchet_test" time="0.010"/>
  </testsuite>
  <testsuite name="xc-shift::shift_service_test" tests="2" failures="1" errors="0">
    <testcase name="open_shift_rejects_duplicate" classname="xc-shift::shift_service_test" time="0.045">
      <failure message="expected DuplicateShift">error details</failure>
    </testcase>
    <testcase name="close_shift_ok" classname="xc-shift::shift_service_test" time="0.020"/>
  </testsuite>
</testsuites>
XML

cat > "$TMP_DIR/junit-1-failure.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="2" failures="1" errors="0">
  <testsuite name="xchange::money_silent_zero_ratchet_test" tests="2" failures="1" errors="0">
    <testcase name="ceilings_hold" classname="xchange::money_silent_zero_ratchet_test" time="0.120">
      <failure message="assertion failed">thread 'ceilings_hold' panicked</failure>
    </testcase>
    <testcase name="clean_file_passes" classname="xchange::money_silent_zero_ratchet_test" time="0.010"/>
  </testsuite>
</testsuites>
XML

cat > "$TMP_DIR/junit-0-failures.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="1" failures="0" errors="0">
  <testsuite name="xchange::smoke" tests="1" failures="0" errors="0">
    <testcase name="smoke_ok" classname="xchange::smoke" time="0.010"/>
  </testsuite>
</testsuites>
XML

pass=0
fail=0

assert_eq() {
  local name="$1" expected="$2" got="$3"
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

# 1. JUnit fixture with 2 failures on re-run (attempt 2) => filter of exactly those 2 tests
got=$("$FILTER_SCRIPT" "-p xc-shift -p xchange" 2 "$TMP_DIR/junit-2-failures.xml")
assert_eq "re-run attempt 2 with 2 JUnit failures => filter of those 2 tests" \
  "-p xc-shift -p xchange -E 'test(=ceilings_hold) | test(=open_shift_rejects_duplicate)'" \
  "$got"

# 2. Re-run attempt 2 after 1 failed test => only that test runs
got=$("$FILTER_SCRIPT" "--workspace" 2 "$TMP_DIR/junit-1-failure.xml")
assert_eq "re-run attempt 2 with 1 JUnit failure => filter of that 1 test" \
  "--workspace -E 'test(=ceilings_hold)'" \
  "$got"

# 3. Re-run with no artifact (missing file, e.g. cancelled job) => $TEST_ARGS
got=$("$FILTER_SCRIPT" "-p xc-shift -p xchange" 2 "$TMP_DIR/does-not-exist.xml")
assert_eq "re-run with missing artifact => TEST_ARGS unchanged" \
  "-p xc-shift -p xchange" \
  "$got"

# 4. Re-run with empty artifact path => $TEST_ARGS
got=$("$FILTER_SCRIPT" "--workspace" 2 "")
assert_eq "re-run with empty artifact path => TEST_ARGS unchanged" \
  "--workspace" \
  "$got"

# 5. First attempt (attempt 1) ignores any old artifact => $TEST_ARGS
got=$("$FILTER_SCRIPT" "-p xc-shift -p xchange" 1 "$TMP_DIR/junit-2-failures.xml")
assert_eq "attempt 1 ignores artifact => TEST_ARGS unchanged" \
  "-p xc-shift -p xchange" \
  "$got"

# 6. Re-run with 0 failures in JUnit XML => $TEST_ARGS
got=$("$FILTER_SCRIPT" "--workspace" 3 "$TMP_DIR/junit-0-failures.xml")
assert_eq "re-run with 0 failures in JUnit => TEST_ARGS unchanged" \
  "--workspace" \
  "$got"

echo "passed: $pass, failed: $fail"
[ "$fail" -eq 0 ]
