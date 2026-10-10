#!/usr/bin/env bash
# Extra edge-case tests for scripts/migration-slot.sh (#1082).
#
# Covers edge cases from section 9 of #1082:
#   M1  Leap day rollover (20280228230000 -> 20280229000000)
#   M2  gh's files list capped at 100 per PR -> exit 1 naming the PR ("file list may be truncated")
#   M3  renumber also rejects when an open PR has 100 files
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SUT="$ROOT/scripts/migration-slot.sh"

TMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/more.XXXXXX")
trap 'rm -rf "$TMP_ROOT"' EXIT

FAILED=0
TOTAL=0
CASE=""
CASE_OK=1

begin() {
  CASE="$1"
  CASE_OK=1
  TOTAL=$((TOTAL + 1))
}

fail() {
  echo "✗ $CASE: $1"
  CASE_OK=0
}

end() {
  if [ "$CASE_OK" -eq 1 ]; then
    echo "✓ $CASE"
  else
    FAILED=$((FAILED + 1))
  fi
}

fixture() {
  T=$(mktemp -d "$TMP_ROOT/case.XXXXXX")
  WORK="$T/work"
  GH_LOG="$T/gh.log"
  GH_STUB_JSON="$T/prs.json"
  : > "$GH_LOG"
  echo '[]' > "$GH_STUB_JSON"
  export GH_LOG GH_STUB_JSON

  mkdir -p "$T/bin"
  cat > "$T/bin/gh" << 'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
if [ "${1:-}" != "pr" ] || [ "${2:-}" != "list" ]; then
  echo "gh stub: unexpected call: gh $*" >&2
  exit 99
fi
jq_expr=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --jq|-q) jq_expr="$2"; shift 2 ;;
    --jq=*) jq_expr="${1#--jq=}"; shift ;;
    *) shift ;;
  esac
done
if [ -n "$jq_expr" ]; then
  jq -r "$jq_expr" "$GH_STUB_JSON"
else
  cat "$GH_STUB_JSON"
fi
STUB
  chmod +x "$T/bin/gh"
  PATH="$T/bin:$ORIG_PATH"
  export PATH

  git init -q --bare "$T/origin.git"
  git -C "$T/origin.git" symbolic-ref HEAD refs/heads/main
  git init -q "$T/seed"
  (
    cd "$T/seed" || exit 1
    git config user.email test@example.com
    git config user.name "Test User"
    git checkout -q -b main
    mkdir -p api/migrations
    echo 'CREATE TABLE shift (id INT);' > api/migrations/20261109100000_shift.sql
    git add . && git commit -q -m A
    echo 'CREATE TABLE rate_feed (id INT);' > api/migrations/20261110100000_rate_feed.sql
    git add . && git commit -q -m B
    git remote add origin "$T/origin.git"
    git push -q origin main
  )
  git clone -q "$T/origin.git" "$WORK"
  git -C "$WORK" config user.email test@example.com
  git -C "$WORK" config user.name "Test User"
}

run_sut() {
  OUT="$T/out"
  ERR="$T/err"
  RC=0
  (cd "$WORK" && bash "$SUT" "$@") > "$OUT" 2> "$ERR" || RC=$?
}

ORIG_PATH="$PATH"
M=api/migrations

begin "M1 leap day rollover (20280228230000 -> 20280229000000)"
fixture
echo '[{"number":2028,"headRefName":"feat/leap","files":[{"path":"'"$M"'/20280228230000_leap.sql"}]}]' > "$GH_STUB_JSON"
run_sut next
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(cat "$ERR"))"; fi
got=$(cat "$OUT")
if [ "$got" != "20280229000000" ]; then fail "stdout '$got', want '20280229000000'"; fi
end

begin "M2 gh files list capped at 100 per PR fails next loudly"
fixture
# Generate a PR with exactly 100 files
python3 -c '
import json
files = [{"path": f"api/migrations/file_{i}.sql"} for i in range(100)]
data = [{"number": 999, "headRefName": "feat/big", "files": files}]
print(json.dumps(data))
' > "$GH_STUB_JSON"
run_sut next
if [ "$RC" -eq 0 ]; then fail "exit 0, want 1"; fi
if [ -s "$OUT" ]; then fail "stdout not empty: $(cat "$OUT")"; fi
if ! grep -q "PR #999 file list may be truncated" "$ERR"; then
  fail "stderr does not name PR and truncation: $(cat "$ERR")"
fi
end

begin "M3 gh files list capped at 100 per PR fails renumber loudly"
fixture
git -C "$WORK" checkout -q -b feat/outbox
echo 'CREATE TABLE outbox (id INT);' > "$WORK/$M/20261109110000_outbox.sql"
git -C "$WORK" add . && git -C "$WORK" commit -q -m "feat"
python3 -c '
import json
files = [{"path": f"api/migrations/file_{i}.sql"} for i in range(100)]
data = [{"number": 888, "headRefName": "feat/big", "files": files},
        {"number": 1016, "headRefName": "feat/outbox", "files": [{"path": "api/migrations/20261109110000_outbox.sql"}]}]
print(json.dumps(data))
' > "$GH_STUB_JSON"
run_sut renumber 1016
if [ "$RC" -eq 0 ]; then fail "exit 0, want 1"; fi
if ! grep -q "PR #888 file list may be truncated" "$ERR"; then
  fail "stderr does not name PR #888 and truncation: $(cat "$ERR")"
fi
end

echo
if [ "$FAILED" -ne 0 ]; then
  echo "$FAILED of $TOTAL migration-slot.more cases failed."
  exit 1
fi
echo "All $TOTAL migration-slot.more cases passed."
