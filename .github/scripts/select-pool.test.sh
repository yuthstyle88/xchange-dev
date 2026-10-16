#!/usr/bin/env bash
# Up-front test for select-pool.sh (xchange api.yml test-job runner pool), written by the BE controller.
# select-pool.sh reads RUN_ID and RUN_ATTEMPT and prints two lines:
#   runs_on=<JSON label array>
#   cargo_jobs=<integer>
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SELECT="$SCRIPT_DIR/select-pool.sh"
API_WORKFLOW="$SCRIPT_DIR/../workflows/api.yml"

if [ ! -x "$SELECT" ]; then
  echo "FAIL: $SELECT not found or not executable" >&2
  exit 1
fi

DELL='["self-hosted","Linux","X64","dell-org-ci"]'
HEAVY='["self-hosted","heavy-build"]'

fail=0
heavy=0
for slot in $(seq 0 11); do
  out=$(RUN_ID=$slot RUN_ATTEMPT=0 "$SELECT")
  pool=$(printf '%s\n' "$out" | sed -n 's/^runs_on=//p')
  jobs=$(printf '%s\n' "$out" | sed -n 's/^cargo_jobs=//p')
  if [ "$pool" = "$HEAVY" ]; then heavy=$((heavy+1)); else echo "FAIL: slot $slot gave pool '$pool', want $HEAVY"; fail=1; fi
  [ "$jobs" = 12 ] || { echo "FAIL: slot $slot gave cargo_jobs='$jobs', want ceiling 12 (cargo-jobs.sh lowers it per machine)"; fail=1; }
done

# Row 1: every slot goes to ONE pool, the runners labelled heavy-build. The label count per machine is the cap on heavy
#        builds it runs at once: owner 2026-10-16 "Heavy-build มี 4 เครื่อง Dell 2 studio 2 warut 1 yut macbook 1" (dell-org-ci-1..2,
#        studio-org-1..2, warut-org-1, mbp-org-1). GitHub hands each heavy build to whichever is idle,
#        so no labelled runner sits idle while a build waits ("Dell ควรมัน รัน อย่างน้อย 2 ตัว ตลอดเวลา").
[ "$heavy" = 12 ] || { echo "FAIL: heavy pool got $heavy of 12 slots, want 12"; fail=1; }

# Row 2: a rerun (RUN_ATTEMPT+1) moves to the next slot, so it can land on another pool.
a=$(RUN_ID=5 RUN_ATTEMPT=1 "$SELECT" | sed -n 's/^runs_on=//p')
b=$(RUN_ID=6 RUN_ATTEMPT=0 "$SELECT" | sed -n 's/^runs_on=//p')
[ "$a" = "$b" ] || { echo "FAIL: (5,1) gave $a but (6,0) gave $b; slot must be (RUN_ID+RUN_ATTEMPT) mod 12"; fail=1; }

# Row 3: missing RUN_ID falls back to the Dell pool (the workflow's documented fallback).
c=$(env -u RUN_ID RUN_ATTEMPT=1 "$SELECT" | sed -n 's/^runs_on=//p')
[ "$c" = "$DELL" ] || { echo "FAIL: missing RUN_ID gave '$c', want Dell fallback"; fail=1; }

# Row 4: api.yml calls the script (no inline copy of the slot logic left behind),
#        and the test job uses its cargo_jobs output for CARGO_BUILD_JOBS.
grep -q 'select-pool.sh' "$API_WORKFLOW" || { echo "FAIL: api.yml does not call select-pool.sh"; fail=1; }
grep -q 'select-pool.test.sh' "$API_WORKFLOW" || { echo "FAIL: api.yml does not run select-pool.test.sh in the changes job"; fail=1; }
if grep -qE 'slot=\$\(\( *\(RUN_ID' "$API_WORKFLOW"; then echo "FAIL: api.yml still has the inline slot arithmetic"; fail=1; fi
grep -qE 'CARGO_JOBS_CAP: *\$\{\{ *needs\.changes\.outputs\.cargo_jobs' "$API_WORKFLOW" \
  || { echo "FAIL: test job CARGO_JOBS_CAP is not taken from needs.changes.outputs.cargo_jobs"; fail=1; }

if [ "$fail" = 0 ]; then echo "select-pool: all rows pass"; fi
exit $fail
