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
STUDIO='["self-hosted","macOS","ARM64","heavy-build"]'

fail=0
dell=0; studio=0
for slot in $(seq 0 11); do
  out=$(RUN_ID=$slot RUN_ATTEMPT=0 "$SELECT")
  pool=$(printf '%s\n' "$out" | sed -n 's/^runs_on=//p')
  jobs=$(printf '%s\n' "$out" | sed -n 's/^cargo_jobs=//p')
  case "$pool" in
    "$DELL")   dell=$((dell+1));     want_jobs=12 ;;
    "$STUDIO") studio=$((studio+1)); want_jobs=8 ;;
    *) echo "FAIL: slot $slot gave unknown pool '$pool'"; fail=1; continue ;;
  esac
  if [ "$jobs" != "$want_jobs" ]; then
    echo "FAIL: slot $slot pool $pool gave cargo_jobs='$jobs', want $want_jobs"; fail=1
  fi
done

# Row 1: weights over one 12-slot cycle are Dell 8, heavy Macs 4 (#2574: 4 dell-org-ci runners vs 2 runners labelled
#        heavy-build: 1 on Mac Studio, 1 on the MacBook; owner: one heavy build at a time per Mac). Warut is out of the pool
# (owner 2026-10-16: "ย้าย ที่รันใน warut ไปรันที่ อื่นแทน"; measured load 99 on 10 cores).
[ "$dell" = 8 ]   || { echo "FAIL: Dell got $dell of 12 slots, want 8"; fail=1; }
[ "$studio" = 4 ] || { echo "FAIL: heavy Macs got $studio of 12 slots, want 4"; fail=1; }
if grep -q "warut-mbp" "$SELECT"; then echo "FAIL: select-pool.sh still names warut-mbp"; fail=1; fi

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
