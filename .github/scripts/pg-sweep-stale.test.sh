#!/usr/bin/env bash
# Up-front test for pg-sweep-stale.sh (#2386), written by the ticket author. Do not edit, delete or weaken it.
# pg-sweep-stale.sh runs at the start of api.yml's "Start postgres (per run)" step. A job that was killed (earlyoom,
# runner restart) never reaches "Stop postgres and remove its data", so its /tmp/xcpg.* data dir (tmpfs = RAM on Dell)
# and its postmaster stay behind. One runner runs one job at a time, so anything marked with THIS runner's name is
# from a dead job and is safe to remove. Anything else (another runner's, or unmarked hand-made dirs) is never touched.
#
# Contract:
#   env RUNNER_NAME   required, non-empty; otherwise exit non-zero and touch nothing
#   env XC_PG_TMP     parent dir to sweep (default /tmp)
#   env PGBIN         optional; if set, "$PGBIN/pg_ctl -D <dir>/data -m immediate stop" is tried first
#   a dir is "ours" iff <dir>/runner exists and its first line equals RUNNER_NAME
#   for ours: if <dir>/data/postmaster.pid names a live pid whose command line contains "<dir>/data", stop it
#     (pg_ctl, then TERM, then KILL); a pid whose command line does not contain "<dir>/data" is never signalled
#     (pid reuse); then rm -rf <dir> and print "swept <dir>"
#   docker: if docker is on PATH, remove containers labelled xc.runner=$RUNNER_NAME
#     (docker ps -q --filter label=xc.runner=<name>, then docker rm -f <ids>)
#   exit 0 when nothing is left to sweep
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SWEEP="$SCRIPT_DIR/pg-sweep-stale.sh"
API_WORKFLOW="$SCRIPT_DIR/../workflows/api.yml"

if [ ! -x "$SWEEP" ]; then
  echo "FAIL: $SWEEP not found or not executable" >&2
  exit 1
fi

fail=0
WORK=$(mktemp -d)
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill -9 "$p" 2>/dev/null; done; rm -rf "$WORK"; }
trap cleanup EXIT

# A stand-in postmaster: a long sleep whose command line carries "<dir>/data", as a real postmaster's does.
fake_postmaster() { # $1 = dir ; prints the pid
  python3 -c 'import sys,time; time.sleep(300)' -D "$1/data" >/dev/null 2>&1 &
  local p=$!; disown "$p" 2>/dev/null; echo "$p"
}
make_dir() { # $1 = name, $2 = marker ("" = no marker file), $3 = live|dead|none ; prints the dir
  local d="$WORK/tmp/$1"
  mkdir -p "$d/data"
  [ -n "$2" ] && printf '%s\n' "$2" > "$d/runner"
  case "$3" in
    live) local p; p=$(fake_postmaster "$d"); PIDS+=("$p"); printf '%s\n' "$p" > "$d/data/postmaster.pid" ;;
    dead) printf '%s\n' 999999 > "$d/data/postmaster.pid" ;;
  esac
  echo "$d"
}
alive() { kill -0 "$1" 2>/dev/null; }

mkdir -p "$WORK/tmp" "$WORK/bin"
# A fake docker on PATH that logs its arguments and lists one container for ps.
cat > "$WORK/bin/docker" <<'DOCKER'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$DOCKER_LOG"
if [ "${1:-}" = ps ]; then echo c0ffee; fi
exit 0
DOCKER
chmod +x "$WORK/bin/docker"
export DOCKER_LOG="$WORK/docker.log"

ME=dell-org-ci-1
OTHER=dell-org-ci-2

# Row S1: our dir with a live postmaster -> postmaster stopped, dir removed, "swept <dir>" printed.
s1=$(make_dir xcpg.AAAAAA "$ME" live); s1pid=$(cat "$s1/data/postmaster.pid")
# Row S2: our dir whose postmaster already died (the 3.0G xcpg.J3K5qL case) -> dir removed.
s2=$(make_dir xcpg.BBBBBB "$ME" dead)
# Row S3: another runner's dir with a live postmaster -> untouched, its postmaster still alive.
s3=$(make_dir xcpg.CCCCCC "$OTHER" live); s3pid=$(cat "$s3/data/postmaster.pid")
# Row S4: an unmarked, hand-made dir (like xcpg.x763) with a live postmaster -> untouched.
s4=$(make_dir xcpg.x763 "" live); s4pid=$(cat "$s4/data/postmaster.pid")
# Row S5: our dir whose pid file names an unrelated live process (pid reuse) -> that process is NOT signalled; dir removed.
s5=$(make_dir xcpg.DDDDDD "$ME" none)
python3 -c 'import time; time.sleep(300)' >/dev/null 2>&1 & unrelated=$!; disown "$unrelated" 2>/dev/null; PIDS+=("$unrelated")
printf '%s\n' "$unrelated" > "$s5/data/postmaster.pid"
# Row S6: a dir that only looks like ours by prefix of the runner name -> untouched (exact match only).
s6=$(make_dir xcpg.EEEEEE "${ME}0" none)

out=$(PATH="$WORK/bin:$PATH" RUNNER_NAME="$ME" XC_PG_TMP="$WORK/tmp" "$SWEEP" 2>&1); rc=$?
[ "$rc" = 0 ] || { echo "FAIL: sweep exited $rc, want 0. Output: $out"; fail=1; }

sleep 1
alive "$s1pid" && { echo "FAIL S1: our live postmaster $s1pid was not stopped"; fail=1; }
[ -e "$s1" ] && { echo "FAIL S1: $s1 not removed"; fail=1; }
printf '%s\n' "$out" | grep -qx "swept $s1" || { echo "FAIL S1: output lacks 'swept $s1'. Output: $out"; fail=1; }
[ -e "$s2" ] && { echo "FAIL S2: $s2 (dead postmaster) not removed"; fail=1; }
[ -d "$s3" ] || { echo "FAIL S3: another runner's dir $s3 was removed"; fail=1; }
alive "$s3pid" || { echo "FAIL S3: another runner's postmaster $s3pid was stopped"; fail=1; }
[ -d "$s4" ] || { echo "FAIL S4: unmarked dir $s4 was removed"; fail=1; }
alive "$s4pid" || { echo "FAIL S4: unmarked dir's postmaster $s4pid was stopped"; fail=1; }
alive "$unrelated" || { echo "FAIL S5: unrelated process $unrelated (pid reuse) was signalled"; fail=1; }
[ -e "$s5" ] && { echo "FAIL S5: $s5 not removed"; fail=1; }
[ -d "$s6" ] || { echo "FAIL S6: $s6 (runner '${ME}0') was removed by a prefix match"; fail=1; }

# Row S7: docker containers labelled with this runner are removed.
grep -qE -- "^ps .*label=xc\.runner=$ME( |\$)" "$DOCKER_LOG" 2>/dev/null \
  || { echo "FAIL S7: docker ps was not filtered on label=xc.runner=$ME. Log: $(cat "$DOCKER_LOG" 2>/dev/null)"; fail=1; }
grep -qx -- "rm -f c0ffee" "$DOCKER_LOG" 2>/dev/null \
  || { echo "FAIL S7: docker rm -f c0ffee not called. Log: $(cat "$DOCKER_LOG" 2>/dev/null)"; fail=1; }

# Row S8: nothing to sweep -> exit 0.
rm -f "$DOCKER_LOG"
empty="$WORK/empty"; mkdir -p "$empty"
PATH="$WORK/bin:$PATH" RUNNER_NAME="$ME" XC_PG_TMP="$empty" "$SWEEP" >/dev/null 2>&1 \
  || { echo "FAIL S8: sweep of an empty dir did not exit 0"; fail=1; }

# Row S9: RUNNER_NAME empty or unset -> non-zero exit, nothing removed (an empty name must never match).
s9=$(make_dir xcpg.FFFFFF "" none)
printf '\n' > "$s9/runner"   # a marker with an empty first line
if PATH="$WORK/bin:$PATH" RUNNER_NAME="" XC_PG_TMP="$WORK/tmp" "$SWEEP" >/dev/null 2>&1; then
  echo "FAIL S9: empty RUNNER_NAME exited 0, want non-zero"; fail=1
fi
if PATH="$WORK/bin:$PATH" env -u RUNNER_NAME XC_PG_TMP="$WORK/tmp" "$SWEEP" >/dev/null 2>&1; then
  echo "FAIL S9: unset RUNNER_NAME exited 0, want non-zero"; fail=1
fi
[ -d "$s9" ] || { echo "FAIL S9: $s9 removed with an empty RUNNER_NAME"; fail=1; }

# Row S10: api.yml wiring.
grep -q 'pg-sweep-stale.test.sh' "$API_WORKFLOW" || { echo "FAIL S10: api.yml does not run pg-sweep-stale.test.sh"; fail=1; }
grep -q 'pg-sweep-stale.sh' "$API_WORKFLOW" || { echo "FAIL S10: api.yml does not call pg-sweep-stale.sh"; fail=1; }
grep -qE 'RUNNER_NAME.*> *"?\$PGROOT/runner"?' "$API_WORKFLOW" \
  || { echo "FAIL S10: api.yml does not write \$RUNNER_NAME into \$PGROOT/runner after mktemp"; fail=1; }
grep -qE -- '--label +"?xc\.runner=\$\{?RUNNER_NAME' "$API_WORKFLOW" \
  || { echo "FAIL S10: the docker fallback does not label its container xc.runner=\$RUNNER_NAME"; fail=1; }
# The sweep must run before mktemp in the same step, so a killed job's leftovers go before a new dir is made.
sweep_line=$(grep -n 'pg-sweep-stale.sh' "$API_WORKFLOW" | grep -v test | head -1 | cut -d: -f1)
mktemp_line=$(grep -n 'mktemp -d /tmp/xcpg' "$API_WORKFLOW" | head -1 | cut -d: -f1)
if [ -n "$sweep_line" ] && [ -n "$mktemp_line" ] && [ "$sweep_line" -gt "$mktemp_line" ]; then
  echo "FAIL S10: pg-sweep-stale.sh (line $sweep_line) runs after mktemp (line $mktemp_line)"; fail=1
fi
# The existing teardown stays (it is the normal path; the sweep only catches killed jobs).
grep -q 'name: Stop postgres and remove its data' "$API_WORKFLOW" \
  || { echo "FAIL S10: the 'Stop postgres and remove its data' always() step is gone"; fail=1; }

if [ "$fail" = 0 ]; then echo "pg-sweep-stale: all rows pass"; fi
exit $fail
