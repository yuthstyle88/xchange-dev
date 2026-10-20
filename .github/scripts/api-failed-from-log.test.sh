#!/usr/bin/env bash
# Self-test for .github/scripts/api-failed-from-log.sh (#3112). Written before the script by the ticket
# author claude/dell-claude-6; do not edit, delete or weaken.
#
# Why: GitHub artifact storage is over quota (owner 2026-10-17; batch 3739 "Artifact storage quota has been
# hit"), so the failed tests of an earlier run or attempt are read from its job log instead, through
# `gh api repos/<o>/<r>/actions/jobs/<id>/logs --allow-escape-sequences`, which needs no artifact storage.
#
# Contract of api-failed-from-log.sh:
#   reads a raw GitHub Actions job log on stdin (timestamp prefix, ANSI colour codes kept);
#   prints one line per failed test, "<binary-id> <test-name>", each once, in first-seen order;
#   a failed test is a nextest status line FAIL, TIMEOUT or SIG<anything> (not a "TRY n FAIL" retry line,
#   not cargo's own "test x ... FAILED" or "test result: FAILED" lines);
#   no failed test, or input that is not a nextest log => prints nothing, exits 0.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PARSE="$SCRIPT_DIR/api-failed-from-log.sh"

if [ ! -x "$PARSE" ]; then
  echo "FAIL: $PARSE not found or not executable" >&2
  exit 1
fi

E=$'\e'
ts() { printf '2026-10-19T22:54:%02d.5552080Z ' "$1"; }
status() { # status <sec> <word> <binary> <name> : one nextest status line as GitHub's raw log has it
  printf '%s%s[31;1m        %s%s[0m [   0.154s] ( 431/4120) %s[35;1m%s%s[0m %s[34;1m%s%s[0m\n' \
    "$(ts "$1")" "$E" "$2" "$E" "$E" "$3" "$E" "$E" "$4" "$E"
}
pass_line() {
  printf '%s%s[32;1m        PASS%s[0m [   4.033s] ( 109/4120) %s[35;1m%s%s[0m %s[34;1m%s%s[0m\n' \
    "$(ts "$1")" "$E" "$E" "$E" "$2" "$E" "$E" "$3" "$E"
}

LOG=$(
  printf '%s%s[32;1m    Starting%s[0m %s[1m4120%s[0m tests across %s[1m535%s[0m binaries\n' "$(ts 1)" "$E" "$E" "$E" "$E" "$E" "$E"
  pass_line 2 xchange::agent_claim_test a_late_failed_report_replaces_the_lapse_and_repeat_is_idempotent
  status 4 FAIL xchange::bootstrap_pii_key_test build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed
  printf '%s    test build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed ... FAILED\n' "$(ts 4)"
  printf '%s    test result: FAILED. 0 passed; 1 failed; 0 ignored\n' "$(ts 4)"
  status 5 'TRY 1 FAIL' xc-shift::shift_service_test flaky_then_passes
  status 6 TIMEOUT xc-rate::rate_publish_test publish_waits_forever
  status 7 SIGSEGV xchange presentation::http::app_verification_upload::tests::body_limit
  pass_line 8 xchange::booking_desk_phone_invalid_test p1_desk_booking_with_invalid_phone_refused
  printf '%s%s[31;1m     Summary%s[0m [ 947.757s] 4120 tests run: 4117 passed, 3 failed\n' "$(ts 9)" "$E" "$E"
  status 9 FAIL xchange::bootstrap_pii_key_test build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed
  status 9 TIMEOUT xc-rate::rate_publish_test publish_waits_forever
)

fail=0
assert_eq() {
  if [ "$2" = "$3" ]; then echo "ok: $1"; else printf 'FAIL: %s\n  want: %q\n  got:  %q\n' "$1" "$2" "$3"; fail=1; fi
}

# Row 1: FAIL, TIMEOUT and SIG* lines give "<binary-id> <test-name>", once each, in first-seen order;
#        the live line and the summary's repeat are one test; a TRY retry and cargo's own lines are not.
got=$(printf '%s\n' "$LOG" | bash "$PARSE")
assert_eq "failed tests from a raw log" \
  "xchange::bootstrap_pii_key_test build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed
xc-rate::rate_publish_test publish_waits_forever
xchange presentation::http::app_verification_upload::tests::body_limit" \
  "$got"

# Row 2: a green log => nothing.
got=$( { pass_line 1 xchange::a_test one; pass_line 2 xchange::a_test two; } | bash "$PARSE")
assert_eq "green log => empty" "" "$got"

# Row 3: input that is not a nextest log => nothing, exit 0.
got=$(printf 'not a log\n\n' | bash "$PARSE")
assert_eq "garbage => empty" "" "$got"

# Row 4: the colour codes may already be stripped (gh run view --log) => same answer.
got=$(printf '%s\n' "$LOG" | sed 's/\x1b\[[0-9;]*m//g' | bash "$PARSE")
assert_eq "plain log => same tests" \
  "xchange::bootstrap_pii_key_test build_refuses_to_start_when_no_pii_key_file_and_no_keys_installed
xc-rate::rate_publish_test publish_waits_forever
xchange presentation::http::app_verification_upload::tests::body_limit" \
  "$got"

[ "$fail" = 0 ] && echo "api-failed-from-log: all rows pass"
exit "$fail"
