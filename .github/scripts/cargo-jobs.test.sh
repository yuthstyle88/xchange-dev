#!/usr/bin/env bash
# Up-front test for cargo-jobs.sh (#2574): the api `test` job sets CARGO_BUILD_JOBS from the machine it
# landed on, so a busy or small runner host is not oversubscribed. Inputs are injectable for the test:
#   CJ_CORES, CJ_LOAD (1-min load average), CJ_MEM_GB; the first argument is the pool's cap.
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CJ="$SCRIPT_DIR/cargo-jobs.sh"
[ -x "$CJ" ] || { echo "FAIL: $CJ not found or not executable" >&2; exit 1; }
fail=0
row() { # name want cap cores load mem
  local got; got=$(CJ_CORES=$4 CJ_LOAD=$5 CJ_MEM_GB=$6 "$CJ" "$3")
  [ "$got" = "$2" ] || { echo "FAIL: $1: cores=$4 load=$5 mem=$6 cap=$3 gave '$got', want $2"; fail=1; }
}
row "idle Dell is held to the pool cap"            12 12 64 3.0  125
row "busy Dell gets what is free"                  12 12 64 50.0 125
row "Dell past its cores still gets the floor"      2 12 64 70.5 125
row "idle Studio is held to the Mac cap"            8  8 20 2.0  128
row "Studio under load gets the free cores"         5  8 20 14.2 128
row "16 GB Mac mini is limited by memory"           4  8  8 0.5  16
row "fractional load rounds the free cores down"    5  8  8 2.9  64
row "a missing cap falls back to 12"               12 "" 64 0.0 125
# The workflow wires it: the test job takes CARGO_BUILD_JOBS from this script, not from a fixed env value.
API="$SCRIPT_DIR/../workflows/api.yml"
grep -q 'cargo-jobs.sh' "$API" || { echo "FAIL: api.yml does not call cargo-jobs.sh"; fail=1; }
grep -q 'cargo-jobs.test.sh' "$API" || { echo "FAIL: api.yml does not run cargo-jobs.test.sh"; fail=1; }
[ "$fail" = 0 ] && echo "cargo-jobs: all rows pass"
exit $fail
