#!/usr/bin/env bash
# Self-test for `api-rerun-filter.sh --since-last` (#3112). Written before the mode by the ticket author
# claude/dell-claude-6; do not edit, delete or weaken. The cases follow claude/fbf02c11's a0a9efca, with the
# input changed from per-partition JUnit artifacts (artifact storage is over quota) to one failed-test list
# made by api-failed-from-log.sh from the last run's job logs.
#
# Contract:
#   api-rerun-filter.sh --since-last <failed-list-file> <diff-packages>
#   <failed-list-file>: "<binary-id> <test-name>" per line, the failures of every partition of the last
#                       finished run of this branch; an empty file = that run had no failure.
#   <diff-packages>: api-test-plan.sh's answer for the diff from the commit the last run tested to this one:
#                    "--workspace", "-p a -p b ..." or "" (nothing Rust changed).
#   Output, key=value lines:
#     mode=fallback   and nothing else => run today's plan (file missing or unreadable, a malformed line,
#                     a name that would need quoting, or a workspace-wide diff);
#     mode=since-last
#     packages=<-p a -p b ...>   packages of the failed tests plus the diff's packages; empty = nothing to run
#     filter=<filterset>         the failed tests (binary and name) plus package(=p) per diff package;
#                                empty = every test of <packages>
#     failed=<count>
#   Mode 1 (`<test-args> <run-attempt> [file]`) is unchanged; api-rerun-filter.test.sh covers it.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
FILTER_SCRIPT="$SCRIPT_DIR/api-rerun-filter.sh"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

printf '%s\n' \
  "xchange::money_silent_zero_ratchet_test ceilings_hold" \
  "xc-shift::shift_service_test open_shift_rejects_duplicate" > "$TMP_DIR/two.txt"
printf '%s\n' "xchange::money_silent_zero_ratchet_test ceilings_hold" > "$TMP_DIR/one.txt"
: > "$TMP_DIR/none.txt"
printf '%s\n' "just_a_name_without_binary" > "$TMP_DIR/malformed.txt"
printf '%s\n' "xchange::a_test name with space" > "$TMP_DIR/spaced.txt"
printf '%s\n' "xchange::a_test it's_quoted" > "$TMP_DIR/quote.txt"

pass=0
fail=0
assert_eq() {
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "ok: $1"; else fail=$((fail + 1)); printf 'FAIL: %s\n  want: %q\n  got:  %q\n' "$1" "$2" "$3"; fi
}
since() { "$FILTER_SCRIPT" --since-last "$@" 2>/dev/null || echo "exit=$?"; }

# 1. Failures in the last run, nothing changed since => only those tests, in their packages.
assert_eq "2 failures, no diff => exactly those 2 tests" \
  "mode=since-last
packages=-p xc-shift -p xchange
filter=(binary_id(=xchange::money_silent_zero_ratchet_test) & test(=ceilings_hold)) | (binary_id(=xc-shift::shift_service_test) & test(=open_shift_rejects_duplicate))
failed=2" \
  "$(since "$TMP_DIR/two.txt" "")"

# 2. One failure plus a diff package => that test plus every test of the diff package.
assert_eq "1 failure + diff package => that test plus the package" \
  "mode=since-last
packages=-p xc-rate -p xchange
filter=(binary_id(=xchange::money_silent_zero_ratchet_test) & test(=ceilings_hold)) | package(=xc-rate)
failed=1" \
  "$(since "$TMP_DIR/one.txt" "-p xc-rate")"

# 3. Last run green, the diff touches one package => that package only, no filter.
assert_eq "green last run + diff package => that package, no filter" \
  "mode=since-last
packages=-p xc-rate
filter=
failed=0" \
  "$(since "$TMP_DIR/none.txt" "-p xc-rate")"

# 4. Last run green, nothing Rust changed => nothing to run.
assert_eq "green last run + no diff => nothing to run" \
  "mode=since-last
packages=
filter=
failed=0" \
  "$(since "$TMP_DIR/none.txt" "")"

# 5. The diff touches the whole workspace => today's plan.
assert_eq "workspace diff => fallback" "mode=fallback" "$(since "$TMP_DIR/two.txt" "--workspace")"

# 6. No list (the last run's logs could not be read) => today's plan, never a guess.
assert_eq "missing list => fallback" "mode=fallback" "$(since "$TMP_DIR/absent.txt" "-p xc-rate")"

# 7. A line without "<binary-id> <test-name>" => today's plan.
assert_eq "malformed line => fallback" "mode=fallback" "$(since "$TMP_DIR/malformed.txt" "")"

# 8. A name that would need quoting in the filterset => today's plan.
assert_eq "spaced name => fallback" "mode=fallback" "$(since "$TMP_DIR/spaced.txt" "")"
assert_eq "quote in name => fallback" "mode=fallback" "$(since "$TMP_DIR/quote.txt" "")"

# 9. Unreadable diff packages => today's plan.
assert_eq "unreadable diff packages => fallback" "mode=fallback" "$(since "$TMP_DIR/one.txt" "xc-rate")"

echo "passed: $pass, failed: $fail"
[ "$fail" -eq 0 ]
