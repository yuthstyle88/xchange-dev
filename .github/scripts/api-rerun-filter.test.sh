#!/usr/bin/env bash
# Self-test for .github/scripts/api-rerun-filter.sh (#3079, #3911).
# Rows 1, 2 and 7-10 are #3911's provided tests (claude/a90ab16c): a re-run names each failed test with its
# binary, so nextest lists only those binaries (0.9.25: a filter that cannot match a binary skips listing it).
# The implementer must not edit, delete or weaken them.
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
  "-p xc-shift -p xchange -E '(binary_id(=xchange::money_silent_zero_ratchet_test) & test(=ceilings_hold)) | (binary_id(=xc-shift::shift_service_test) & test(=open_shift_rejects_duplicate))'" \
  "$got"

# 2. Re-run attempt 2 after 1 failed test => only that test runs
got=$("$FILTER_SCRIPT" "--workspace" 2 "$TMP_DIR/junit-1-failure.xml")
assert_eq "re-run attempt 2 with 1 JUnit failure => filter of that 1 test" \
  "--workspace -E '(binary_id(=xchange::money_silent_zero_ratchet_test) & test(=ceilings_hold))'" \
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

# 7. The failed-tests file api-failed-from-log.sh writes ("<binary-id> <test-name>") => one group per test
printf '%s\n' \
  "xchange::pages_test every_page_answers_as_v2_does" \
  "xc-cash::ledger_postings_test sells_post" > "$TMP_DIR/two-lines.txt"
got=$("$FILTER_SCRIPT" "--workspace" 2 "$TMP_DIR/two-lines.txt")
assert_eq "re-run from a binary+name list => one binary_id group per failed test" \
  "--workspace -E '(binary_id(=xchange::pages_test) & test(=every_page_answers_as_v2_does)) | (binary_id(=xc-cash::ledger_postings_test) & test(=sells_post))'" \
  "$got"

# 8. The same test name failing in two binaries => two groups (test(=name) alone would also run it elsewhere)
printf '%s\n' \
  "xchange::a_test it_works" \
  "xc-shift::b_test it_works" > "$TMP_DIR/same-name.txt"
got=$("$FILTER_SCRIPT" "--workspace" 2 "$TMP_DIR/same-name.txt")
assert_eq "same test name in two binaries => two groups" \
  "--workspace -E '(binary_id(=xchange::a_test) & test(=it_works)) | (binary_id(=xc-shift::b_test) & test(=it_works))'" \
  "$got"

# 9. An old-format line (name only, no binary) => test(=name), as before
printf '%s\n' "ceilings_hold" > "$TMP_DIR/name-only.txt"
got=$("$FILTER_SCRIPT" "--workspace" 2 "$TMP_DIR/name-only.txt")
assert_eq "name-only line keeps the old test(=name) form" \
  "--workspace -E 'test(=ceilings_hold)'" \
  "$got"

# 10. A duplicated binary+name line is listed once
printf '%s\n' \
  "xchange::pages_test every_page_answers_as_v2_does" \
  "xchange::pages_test every_page_answers_as_v2_does" > "$TMP_DIR/dup.txt"
got=$("$FILTER_SCRIPT" "--workspace" 2 "$TMP_DIR/dup.txt")
assert_eq "a repeated failure is listed once" \
  "--workspace -E '(binary_id(=xchange::pages_test) & test(=every_page_answers_as_v2_does))'" \
  "$got"

echo "passed: $pass, failed: $fail"
[ "$fail" -eq 0 ]
