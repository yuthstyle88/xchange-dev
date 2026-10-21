#!/usr/bin/env bash
# Up-front test for cargo-jobs.sh (#2574, #4221): the api `test` job sets CARGO_BUILD_JOBS from the machine it
# landed on, so a busy or small runner host is not oversubscribed. Inputs are injectable for the test:
#   CJ_CORES, CJ_LOAD (1-min load average), CJ_MEM_GB, CJ_CGROUP_MEM, CJ_PROC_SELF_CGROUP, CJ_SYS_FS_CGROUP;
#   the first argument is the pool's cap.
set -euo pipefail
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CJ="$SCRIPT_DIR/cargo-jobs.sh"
[ -x "$CJ" ] || { echo "FAIL: $CJ not found or not executable" >&2; exit 1; }
fail=0
row() { # name want cap cores load mem [cgroup_mem]
  local got
  got=$(CJ_CORES=$4 CJ_LOAD=$5 CJ_MEM_GB=${6:-} CJ_CGROUP_MEM=${7:-} \
    CJ_PROC_SELF_CGROUP="${CJ_PROC_SELF_CGROUP:-/dev/null}" "$CJ" "$3" 2>/dev/null)
  [ "$got" = "$2" ] || { echo "FAIL: $1: cores=$4 load=$5 mem=${6:-} cg=${7:-} cap=$3 gave '$got', want $2"; fail=1; }
}
# Changed from 12 -> 10 (#4221): without a cgroup cap, floor(125 * 0.6 / 7) = 10 caps below the pool cap of 12.
row "idle Dell is held to the pool cap"            10 12 64 3.0  125
# Changed from 12 -> 10 (#4221): free cores (14) and pool cap (12) exceed floor(125 * 0.6 / 7) = 10.
row "busy Dell gets what is free"                  10 12 64 50.0 125
row "Dell past its cores still gets the floor"      2 12 64 70.5 125
row "idle Studio is held to the Mac cap"            8  8 20 2.0  128
row "Studio under load gets the free cores"         5  8 20 14.2 128
# Changed from 4 -> 2 (#4221): floor(16 * 0.6 / 7) = 1, raised to the minimum floor of 2.
row "16 GB Mac mini is limited by memory"           2  8  8 0.5  16
row "fractional load rounds the free cores down"    5  8  8 2.9  64
# Changed from 12 -> 10 (#4221): missing cap still defaults to 12, then floor(125 * 0.6 / 7) = 10 caps it.
row "a missing cap falls back to 12"               10 "" 64 0.0  125

# §8a behaviour table (#4221)
row "J1: Dell runner with a 24 GiB cgroup"                     3 12 64 3   125 25769803776
row "J2: cgroup max -> Linux MemTotal fallback"               10 12 64 3   125 max
row "J3: macOS 36 GB, no cgroup (jam)"                         3 12 12 1   36
row "J4: large machine where the CPU count decides"            6  8  8 2   128
row "J5: small machine floor"                                  2  8  8 0.5 16

cg_tmp=$(mktemp -d)
trap 'rm -rf "$cg_tmp"' EXIT
mkdir -p "$cg_tmp/sys/slice/unit/leaf"
printf '0::/slice/unit/leaf\n' > "$cg_tmp/proc_cgroup"
printf 'max\n'         > "$cg_tmp/sys/slice/unit/leaf/memory.max"
printf '25769803776\n' > "$cg_tmp/sys/slice/unit/memory.max"
printf '64424509440\n' > "$cg_tmp/sys/slice/memory.max"
CJ_PROC_SELF_CGROUP="$cg_tmp/proc_cgroup" CJ_SYS_FS_CGROUP="$cg_tmp/sys" \
  row "J6: cgroup walk picks smallest numeric memory.max"      3 12 64 3   ""

row "J7: busy Dell with a cgroup, CPU lower than memory"       2 12 64 62  ""  25769803776

# The workflow wires it: the test job takes CARGO_BUILD_JOBS from this script, not from a fixed env value.
API="$SCRIPT_DIR/../workflows/api.yml"
grep -q 'cargo-jobs.sh' "$API" || { echo "FAIL: api.yml does not call cargo-jobs.sh"; fail=1; }
grep -q 'cargo-jobs.test.sh' "$API" || { echo "FAIL: api.yml does not run cargo-jobs.test.sh"; fail=1; }
[ "$fail" = 0 ] && echo "cargo-jobs: all rows pass"
exit $fail
