#!/usr/bin/env bash
# Up-front self-test for main-push-base.sh (#737), written by the ticket author
# before the script exists. The implementer must not edit this file.
#
# Contract: `main-push-base.sh <workflow-name> <head-sha>` run inside the repo,
# reading the workflow runs list from the file named by MAIN_PUSH_RUNS_JSON.
# It prints the head_sha of the newest (by created_at) run of that workflow on
# branch main, event push, conclusion success, whose sha is a strict ancestor
# of <head-sha>; or prints nothing. Malformed input exits non-zero.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SUT="$SCRIPT_DIR/main-push-base.sh"

if [ ! -f "$SUT" ]; then
  echo "Error: $SUT not found" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

REPO_DIR="$TMP_DIR/repo"
git init -q "$REPO_DIR"
cd "$REPO_DIR"
git config user.email "test@example.com"
git config user.name "Test User"

commit() { echo "$1" >> f.txt; git add f.txt; git commit -q -m "$1"; git rev-parse HEAD; }
C1=$(commit c1)
C2=$(commit c2)
C3=$(commit c3)
C4=$(commit c4)
# An unrelated history: never an ancestor of C4.
git checkout -q --orphan other
git rm -q -rf . >/dev/null 2>&1 || true
X=$(commit x)
git checkout -q "$C4"

# run <name> <branch> <event> <conclusion> <sha> <created_at>
run() {
  printf '{"name":"%s","head_branch":"%s","event":"%s","conclusion":"%s","head_sha":"%s","created_at":"%s"}' \
    "$1" "$2" "$3" "$4" "$5" "$6"
}
runs() { local IFS=,; printf '{"workflow_runs":[%s]}' "$*"; }

FAILED=0
check() {
  local label="$1" expected="$2" json="$3" head="${4:-$C4}" wf="${5:-api}"
  printf '%s' "$json" > "$TMP_DIR/runs.json"
  local actual
  if ! actual=$(MAIN_PUSH_RUNS_JSON="$TMP_DIR/runs.json" bash "$SUT" "$wf" "$head"); then
    echo "✗ $label: exited non-zero"; FAILED=1; return
  fi
  if [ "$actual" = "$expected" ]; then
    echo "✓ $label"
  else
    echo "✗ $label: expected '${expected}' got '${actual}'"; FAILED=1
  fi
}

# 1. Previous run succeeded: base is the previous commit (today's behaviour).
check "previous success" "$C3" "$(runs \
  "$(run api main push success "$C3" 2026-10-09T10:03:00Z)" \
  "$(run api main push success "$C2" 2026-10-09T10:02:00Z)")"

# 2. Previous run cancelled (WF-1), the one before succeeded: base is the older success.
check "previous cancelled" "$C2" "$(runs \
  "$(run api main push cancelled "$C3" 2026-10-09T10:03:00Z)" \
  "$(run api main push success "$C2" 2026-10-09T10:02:00Z)")"

# 3. No successful run at all: empty (caller builds everything).
check "no success" "" "$(runs \
  "$(run api main push failure "$C3" 2026-10-09T10:03:00Z)" \
  "$(run api main push cancelled "$C2" 2026-10-09T10:02:00Z)")"

# 4. Newest success is not an ancestor of head: skip it.
check "not an ancestor" "$C2" "$(runs \
  "$(run api main push success "$X" 2026-10-09T10:05:00Z)" \
  "$(run api main push success "$C2" 2026-10-09T10:02:00Z)")"

# 5. Other workflow, other branch, other event are ignored.
check "filters" "$C1" "$(runs \
  "$(run web main push success "$C3" 2026-10-09T10:04:00Z)" \
  "$(run api batch/merge-1 push success "$C3" 2026-10-09T10:04:00Z)" \
  "$(run api main pull_request success "$C3" 2026-10-09T10:04:00Z)" \
  "$(run api main workflow_dispatch success "$C3" 2026-10-09T10:04:00Z)" \
  "$(run api main push success "$C1" 2026-10-09T10:01:00Z)")"

# 6. A success on head itself (a re-run) is not a base.
check "head itself" "$C3" "$(runs \
  "$(run api main push success "$C4" 2026-10-09T10:06:00Z)" \
  "$(run api main push success "$C3" 2026-10-09T10:03:00Z)")"

# 7. Newest by created_at wins, whatever the list order.
check "sorted by created_at" "$C3" "$(runs \
  "$(run api main push success "$C1" 2026-10-09T10:01:00Z)" \
  "$(run api main push success "$C3" 2026-10-09T10:03:00Z)" \
  "$(run api main push success "$C2" 2026-10-09T10:02:00Z)")"

# 8. The workflow name argument is honoured (web asks for web runs only).
check "workflow arg" "$C2" "$(runs \
  "$(run api main push success "$C3" 2026-10-09T10:03:00Z)" \
  "$(run web main push success "$C2" 2026-10-09T10:02:00Z)")" "$C4" web

# 9. Empty runs list: empty output, exit 0.
check "empty list" "" '{"workflow_runs":[]}'

# 10. Malformed JSON: non-zero exit (caller then builds everything).
printf 'not json' > "$TMP_DIR/bad.json"
if MAIN_PUSH_RUNS_JSON="$TMP_DIR/bad.json" bash "$SUT" api "$C4" >/dev/null 2>&1; then
  echo "✗ malformed json: exited 0"; FAILED=1
else
  echo "✓ malformed json"
fi

if [ "$FAILED" -ne 0 ]; then
  echo "main-push-base.test.sh: FAILED"
  exit 1
fi
echo "main-push-base.test.sh: all passed"
