#!/usr/bin/env bash
# Self-test for pg-sweep-stale.sh's rule for ANOTHER runner's marked dirs (#2832, the rule #2798 asked for).
# pg-sweep-stale.test.sh (#2386) covers this runner's dirs, unmarked dirs, docker and the api.yml wiring; this file
# covers only a dir whose <dir>/runner names another runner:
#   - postmaster.pid names a pid that is gone, or a live pid whose command line lacks "<dir>/data" (pid reuse)
#     -> dir removed, "swept <dir>" printed, the reused pid's process never signalled
#   - postmaster.pid names that dir's live postmaster -> kept, not signalled
#   - no postmaster.pid (between mktemp and pg_ctl start) or an unreadable one -> kept
#   - an unmarked dir with a dead pid -> kept (only marked dirs are ever removed)
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SWEEP="$SCRIPT_DIR/pg-sweep-stale.sh"

if [ ! -x "$SWEEP" ]; then
  echo "FAIL: $SWEEP not found or not executable" >&2
  exit 1
fi

fail=0
WORK=$(mktemp -d "${XC_TEST_TMP:-${RUNNER_TEMP:-$PWD}}/pgsweep-test.XXXXXX")
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill -9 "$p" 2>/dev/null; done; rm -rf "$WORK"; }
trap cleanup EXIT

# A stand-in postmaster: a long sleep whose command line carries "<dir>/data", as a real postmaster's does.
fake_postmaster() { # $1 = dir ; prints the pid
  python3 -c 'import sys,time; time.sleep(300)' -D "$1/data" >/dev/null 2>&1 &
  local p=$!; disown "$p" 2>/dev/null; echo "$p"
}
# A pid that is not running: start a short process and reap it.
dead_pid() {
  python3 -c 'pass' &
  local p=$!; wait "$p" 2>/dev/null; echo "$p"
}
make_dir() { # $1 = name, $2 = marker ("" = no marker file), $3 = live|dead|none|garbage ; prints the dir
  local d="$WORK/tmp/$1"
  mkdir -p "$d/data"
  [ -n "$2" ] && printf '%s\n' "$2" > "$d/runner"
  case "$3" in
    live) local p; p=$(fake_postmaster "$d"); PIDS+=("$p"); printf '%s\n' "$p" > "$d/data/postmaster.pid" ;;
    dead) printf '%s\n' "$(dead_pid)" > "$d/data/postmaster.pid" ;;
    garbage) printf 'not-a-pid\n' > "$d/data/postmaster.pid" ;;
  esac
  echo "$d"
}
alive() { kill -0 "$1" 2>/dev/null; }

mkdir -p "$WORK/tmp" "$WORK/bin"
# A docker that does nothing, so the sweep never reaches the real one.
printf '#!/usr/bin/env bash\nexit 0\n' > "$WORK/bin/docker"
chmod +x "$WORK/bin/docker"

ME=dell-org-ci-1
OTHER=dell-org-ci-2

# Row F1: another runner's dir whose postmaster is gone -> removed, "swept <dir>" printed.
f1=$(make_dir xcpg.GGGGGG "$OTHER" dead)
# Row F2: another runner's dir whose pid now belongs to an unrelated process -> not signalled; dir removed.
f2=$(make_dir xcpg.HHHHHH "$OTHER" none)
python3 -c 'import time; time.sleep(300)' >/dev/null 2>&1 & unrelated=$!; disown "$unrelated" 2>/dev/null; PIDS+=("$unrelated")
printf '%s\n' "$unrelated" > "$f2/data/postmaster.pid"
# Row F3: another runner's dir with its live postmaster -> kept, postmaster untouched.
f3=$(make_dir xcpg.IIIIII "$OTHER" live); f3pid=$(cat "$f3/data/postmaster.pid")
# Row F4: another runner's dir with no postmaster.pid yet -> kept.
f4=$(make_dir xcpg.JJJJJJ "$OTHER" none)
# Row F5: another runner's dir whose postmaster.pid is unreadable -> kept.
f5=$(make_dir xcpg.KKKKKK "$OTHER" garbage)
# Row F6: an unmarked dir with a dead pid -> kept.
f6=$(make_dir xcpg.LLLLLL "" dead)

out=$(PATH="$WORK/bin:$PATH" RUNNER_NAME="$ME" XC_PG_TMP="$WORK/tmp" "$SWEEP" 2>&1); rc=$?
[ "$rc" = 0 ] || { echo "FAIL: sweep exited $rc, want 0. Output: $out"; fail=1; }

[ -e "$f1" ] && { echo "FAIL F1: another runner's dir $f1 with a dead postmaster was not removed"; fail=1; }
printf '%s\n' "$out" | grep -qx "swept $f1" || { echo "FAIL F1: output lacks 'swept $f1'. Output: $out"; fail=1; }
alive "$unrelated" || { echo "FAIL F2: unrelated process $unrelated (pid reuse) was signalled"; fail=1; }
[ -e "$f2" ] && { echo "FAIL F2: $f2 (pid reused) not removed"; fail=1; }
[ -d "$f3" ] || { echo "FAIL F3: another runner's dir $f3 with a live postmaster was removed"; fail=1; }
alive "$f3pid" || { echo "FAIL F3: another runner's postmaster $f3pid was stopped"; fail=1; }
[ -d "$f4" ] || { echo "FAIL F4: another runner's dir $f4 without postmaster.pid was removed"; fail=1; }
[ -d "$f5" ] || { echo "FAIL F5: another runner's dir $f5 with an unreadable postmaster.pid was removed"; fail=1; }
[ -d "$f6" ] || { echo "FAIL F6: unmarked dir $f6 was removed"; fail=1; }

if [ "$fail" = 0 ]; then echo "pg-sweep-stale-foreign: all rows pass"; fi
exit $fail
