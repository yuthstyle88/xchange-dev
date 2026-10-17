#!/usr/bin/env bash
# Self-test for ci-postgres-root.sh (#2798): one case per row of the ticket's behaviour table.
# It works only under its own XC_PG_BASE (a mktemp -d whose path contains a space) and leaves
# no process and no directory behind. Postgres is never started: a stand-in `sleep` renamed
# with `exec -a "postgres -D <dir>/data"` plays the postmaster, so the script's `ps` check sees it.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SUT="$SCRIPT_DIR/ci-postgres-root.sh"

if [ ! -f "$SUT" ]; then
  echo "Error: $SUT not found" >&2
  exit 1
fi

OUTER=$(mktemp -d)
BASE="$OUTER/pg base"
mkdir -p "$BASE"
PIDS=()
cleanup() {
  local p
  for p in "${PIDS[@]:-}"; do
    [ -n "$p" ] && kill -KILL "$p" 2>/dev/null || true
  done
  rm -rf "$OUTER"
}
trap cleanup EXIT

fail() {
  echo "FAIL $1" >&2
  exit 1
}

alive() { ps -p "$1" >/dev/null 2>&1; }

# A stand-in postmaster for <dir>: its command line reads "postgres -D <dir>/data …".
# With "ignore-quit" it ignores SIGQUIT (an ignored signal stays ignored across exec).
# Needs perl (present on the macOS and Linux runners).
# Sets SP (not printed: a $(...) subshell would lose the PIDS entry used by cleanup).
start_postmaster() {
  local dir=$1 mode=${2:-}
  local quit=DEFAULT
  [ "$mode" = ignore-quit ] && quit=IGNORE
  # perl, not bash: a non-interactive bash starts background jobs with SIGQUIT ignored, and an
  # inherited ignore cannot be undone by `trap`. perl sets the disposition explicitly before exec.
  perl -e '$SIG{QUIT} = $ARGV[1]; exec { "sleep" } "postgres -D $ARGV[0]/data", "300" or die' "$dir" "$quit" &
  SP=$!
  PIDS+=("$SP")
  sleep 0.3
}

# An unrelated live process (its command line has no "-D <dir>/data"). Sets SP.
start_unrelated() {
  # SIGQUIT at its default, so a QUIT sent to it by mistake kills it and the case sees that.
  perl -e '$SIG{QUIT} = "DEFAULT"; exec { "sleep" } "sleep", "300" or die' &
  SP=$!
  PIDS+=("$SP")
}

# A pid that is not running: start and reap a short process.
dead_pid() {
  sleep 0 &
  local pid=$!
  wait "$pid" 2>/dev/null || true
  echo "$pid"
}

# make_root <name> [runner|-] [pid|-]: an xcpg.* dir; "-" for runner = unmarked; pid writes data/postmaster.pid.
make_root() {
  local name=$1 runner=${2--} pid=${3:--}
  local dir="$BASE/xcpg.$name"
  mkdir -p "$dir/data"
  if [ "$runner" != "-" ]; then
    printf 'RUNNER_NAME=%s\nGITHUB_RUN_ID=1\nGITHUB_RUN_ATTEMPT=1\nGITHUB_JOB=test\nCREATED_AT=2026-10-17T00:00:00Z\n' \
      "$runner" >"$dir/.xc-ci-owner"
  fi
  if [ "$pid" != "-" ]; then
    printf '%s\n%s/data\n' "$pid" "$dir" >"$dir/data/postmaster.pid"
  fi
  echo "$dir"
}

# run_sut <runner>: runs the script, stores stdout in OUT and stderr in ERR, returns its rc.
run_sut() {
  local runner=$1 rc=0
  OUT=$(XC_PG_BASE="$BASE" RUNNER_NAME="$runner" GITHUB_RUN_ID=99 GITHUB_RUN_ATTEMPT=1 GITHUB_JOB=test \
    bash "$SUT" 2>"$OUTER/stderr") || rc=$?
  ERR=$(cat "$OUTER/stderr")
  return "$rc"
}

# The new dir must be one marked xcpg.* dir under BASE, printed alone on stdout.
check_created() {
  local case=$1
  [ "$(printf '%s\n' "$OUT" | wc -l | tr -d ' ')" = 1 ] || fail "$case: stdout must be exactly one line, got: $OUT"
  case "$OUT" in
    "$BASE"/xcpg.*) ;;
    *) fail "$case: printed path is not under the base: $OUT" ;;
  esac
  [ -d "$OUT" ] || fail "$case: printed dir does not exist: $OUT"
  grep -q '^RUNNER_NAME=' "$OUT/.xc-ci-owner" 2>/dev/null || fail "$case: new dir has no .xc-ci-owner with RUNNER_NAME="
  rm -rf "$OUT"
}

reset_base() {
  rm -rf "$BASE"
  mkdir -p "$BASE"
}

# 1. no xcpg.* at all: nothing reaped; one marked dir created and printed; rc 0
reset_base
run_sut R || fail "empty base: rc must be 0"
check_created "empty base"
echo "ok empty base creates one marked dir and prints only its path"

# 2. new marker records the job identity
reset_base
run_sut R || fail "marker content: rc must be 0"
grep -qx 'RUNNER_NAME=R' "$OUT/.xc-ci-owner" || fail "marker must record RUNNER_NAME=R"
grep -qx 'GITHUB_RUN_ID=99' "$OUT/.xc-ci-owner" || fail "marker must record GITHUB_RUN_ID=99"
grep -qx 'GITHUB_JOB=test' "$OUT/.xc-ci-owner" || fail "marker must record GITHUB_JOB=test"
rm -rf "$OUT"
echo "ok marker records RUNNER_NAME, GITHUB_RUN_ID and GITHUB_JOB"

# 3. unmarked dir, with a live postmaster: kept, postmaster untouched
reset_base
d=$(make_root unmarked - -)
start_postmaster "$d"
p=$SP
printf '%s\n%s/data\n' "$p" "$d" >"$d/data/postmaster.pid"
run_sut R || fail "unmarked: rc must be 0"
[ -d "$d" ] || fail "unmarked dir must be kept"
alive "$p" || fail "unmarked dir's postmaster must not be signalled"
check_created "unmarked"
echo "ok unmarked dir is kept and its postmaster untouched"

# 4. unmarked dir with a dead pid: still kept (only marked dirs are ours)
reset_base
d=$(make_root unmarkeddead - "$(dead_pid)")
run_sut R || fail "unmarked dead: rc must be 0"
[ -d "$d" ] || fail "unmarked dir with a dead pid must be kept"
check_created "unmarked dead"
echo "ok unmarked dir with a dead pid is kept"

# 5. marked, same runner, postmaster alive with matching command: QUIT, removed
reset_base
d=$(make_root samealive R -)
start_postmaster "$d"
p=$SP
printf '%s\n%s/data\n' "$p" "$d" >"$d/data/postmaster.pid"
run_sut R || fail "same runner alive: rc must be 0"
[ ! -e "$d" ] || fail "same runner, postmaster alive: dir must be removed"
sleep 0.3
alive "$p" && fail "same runner, postmaster alive: postmaster must be stopped"
check_created "same runner alive"
echo "ok same runner + live postmaster: stopped and removed"

# 6. marked, same runner, no postmaster.pid: removed
reset_base
d=$(make_root samenopid R -)
run_sut R || fail "same runner no pid: rc must be 0"
[ ! -e "$d" ] || fail "same runner, no postmaster.pid: dir must be removed"
check_created "same runner no pid"
echo "ok same runner + no postmaster.pid: removed"

# 7. marked, other runner, postmaster alive: kept
reset_base
d=$(make_root otheralive other -)
start_postmaster "$d"
p=$SP
printf '%s\n%s/data\n' "$p" "$d" >"$d/data/postmaster.pid"
run_sut R || fail "other runner alive: rc must be 0"
[ -d "$d" ] || fail "other runner, postmaster alive: dir must be kept"
alive "$p" || fail "other runner, postmaster alive: postmaster must not be signalled"
check_created "other runner alive"
echo "ok other runner + live postmaster: kept"

# 8. marked, other runner, postmaster pid dead: removed
reset_base
d=$(make_root otherdead other "$(dead_pid)")
run_sut R || fail "other runner dead: rc must be 0"
[ ! -e "$d" ] || fail "other runner, dead pid: dir must be removed"
check_created "other runner dead"
echo "ok other runner + dead pid: removed"

# 9. marked, other runner, no postmaster.pid: kept (between mktemp and pg_ctl start)
reset_base
d=$(make_root othernopid other -)
run_sut R || fail "other runner no pid: rc must be 0"
[ -d "$d" ] || fail "other runner, no postmaster.pid: dir must be kept"
check_created "other runner no pid"
echo "ok other runner + no postmaster.pid: kept"

# 10. pid reused by an unrelated process, same runner: not signalled, dir removed
reset_base
start_unrelated
u=$SP
d=$(make_root reusedsame R "$u")
run_sut R || fail "reused pid same runner: rc must be 0"
alive "$u" || fail "reused pid (same runner): the unrelated process must not be signalled"
[ ! -e "$d" ] || fail "reused pid (same runner): dir must be removed"
check_created "reused pid same runner"
echo "ok reused pid + same runner: unrelated process untouched, dir removed"

# 11. pid reused by an unrelated process, other runner: not signalled, treated as dead, removed
reset_base
start_unrelated
u=$SP
d=$(make_root reusedother other "$u")
run_sut R || fail "reused pid other runner: rc must be 0"
alive "$u" || fail "reused pid (other runner): the unrelated process must not be signalled"
[ ! -e "$d" ] || fail "reused pid (other runner): dir must be removed as a dead postmaster"
check_created "reused pid other runner"
echo "ok reused pid + other runner: unrelated process untouched, dir removed"

# 12. RUNNER_NAME empty: same-runner rule off; a marked dir with an empty runner and no pid is kept,
#     a marked dir with a dead pid is still removed
reset_base
k=$(make_root emptykeep "" -)
r=$(make_root emptydead "" "$(dead_pid)")
run_sut "" || fail "empty runner: rc must be 0"
[ -d "$k" ] || fail "empty RUNNER_NAME: a marked dir without postmaster.pid must be kept"
[ ! -e "$r" ] || fail "empty RUNNER_NAME: a marked dir with a dead pid must be removed"
check_created "empty runner"
echo "ok empty RUNNER_NAME: same-runner rule off, dead-pid reaping still on"

# 13. a file (not a dir) named xcpg.*: skipped
reset_base
printf 'x' >"$BASE/xcpg.afile"
run_sut R || fail "file named xcpg.*: rc must be 0"
[ -f "$BASE/xcpg.afile" ] || fail "a file named xcpg.* must be left alone"
check_created "file named xcpg"
echo "ok a file named xcpg.* is skipped"

# 14. postmaster ignores QUIT for 10 s: warning on stderr, dir kept, new dir still created, rc 0
reset_base
d=$(make_root stubborn R -)
start_postmaster "$d" ignore-quit
p=$SP
printf '%s\n%s/data\n' "$p" "$d" >"$d/data/postmaster.pid"
run_sut R || fail "stubborn postmaster: rc must be 0"
[ -d "$d" ] || fail "a postmaster that ignores QUIT: its dir must be kept"
[ -n "$ERR" ] || fail "a postmaster that ignores QUIT: a warning must go to stderr"
check_created "stubborn postmaster"
echo "ok a postmaster that ignores QUIT: warned, kept, new dir still created"

# 15. nothing left behind by the cases above, other than the stand-ins this file kills on exit
reset_base
[ -z "$(ls -A "$BASE")" ] || fail "base must be empty after the cases"
echo "ok all cases passed"
