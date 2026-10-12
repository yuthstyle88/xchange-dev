#!/usr/bin/env bash
# Self-test of scripts/migration-slot.sh and of the fix line that
# .github/scripts/migration-order.sh prints on failure.
#
# No network: every case builds a bare "origin" and a clone in a temp dir, and
# puts a stub `gh` first on PATH. The stub answers `gh pr list` from the JSON
# file named by GH_STUB_JSON (applying --jq with jq when the caller passes it),
# logs its arguments to $GH_LOG, fails with GH_STUB_EXIT when that is set, and
# fails loudly on any other gh subcommand.
#
# Fixture main: api/migrations/20261109100000_shift.sql (commit A), then
# api/migrations/20261110100000_rate_feed.sql (commit B). Main's highest is
# 20261110100000; the step is one hour, so the first free slot is
# 20261110110000.
#
# Cases (one per row of the ticket's behaviour table):
#   N1  next with no open PR migrations = main's max + 1h; gh asked for open PRs
#       with a --limit of at least 200
#   N2  next skips a slot an open PR already holds
#   N3  next with two open PRs at the same max (plus PRs with no or older
#       migrations) = that max + 1h
#   N4a next rolls the day   (20261110230000 -> 20261111000000)
#   N4b next rolls the year  (20261231230000 -> 20270101000000)
#   N5  gh fails -> non-zero exit, a message on stderr, nothing on stdout
#   N6  next --name ledger prints api/migrations/<version>_ledger.sql;
#       a name that is not snake_case is refused
#   R1  renumber <pr> below main -> git mv to the next slot, SQL bytes identical
#       (cmp), one commit with the exact subject, a pure rename (R100), old->new
#       printed
#   R2  renumber when already above main -> exit 0, "nothing to renumber",
#       HEAD unchanged
#   R3  renumber <branch> picks a slot above other open PRs too
#   R4  renumber with two below-main migrations keeps their order, one commit
#       each with the exact subject
#   R5  renumber refuses a dirty tree and a checkout that is not the PR's branch
#   R6  renumber with gh failing -> non-zero, nothing renamed, HEAD unchanged
#   T1  next with a 100-file PR listing 120 files via REST, including a
#       migration = skips that migration's slot
#   T2  next with a 100-file PR listing 100 non-migration files via REST =
#       no refusal, main's max + 1h
#   T3  next with a 100-file PR whose REST call fails -> non-zero exit,
#       message on stderr, nothing on stdout
#   T4  renumber with a 100+ file PR holding a migration -> new slot lands
#       above it
#   G1  migration-order.sh failure prints the fix command with PR_NUMBER, and a
#       <pr-number> placeholder without it
set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SUT="$ROOT/scripts/migration-slot.sh"
GATE="$ROOT/.github/scripts/migration-order.sh"
for f in "$SUT" "$GATE"; do
  if [ ! -f "$f" ]; then
    echo "Error: $f not found" >&2
    exit 1
  fi
done

TMP_ROOT=$(mktemp -d)
trap 'rm -rf "$TMP_ROOT"' EXIT

ORIG_PATH="$PATH"
M=api/migrations

GIT_GLOBAL_CONFIG="$TMP_ROOT/gitconfig"
cat << 'EOF' > "$GIT_GLOBAL_CONFIG"
[user]
	name = Test User
	email = test@example.com
[core]
	fsync = none
[credential]
	helper =
EOF
export GIT_CONFIG_GLOBAL="$GIT_GLOBAL_CONFIG"
export GIT_CONFIG_NOSYSTEM=1
export GIT_TERMINAL_PROMPT=0
export MIGRATION_SLOT_NO_FETCH=1

# Pre-build fixture template once to avoid 14x repetitive git init/push/clone
TPL_ORIGIN="$TMP_ROOT/template.git"
TPL_WORK="$TMP_ROOT/template.work"
git init -q --bare "$TPL_ORIGIN"
git -C "$TPL_ORIGIN" symbolic-ref HEAD refs/heads/main
git init -q "$TMP_ROOT/seed"
(
  cd "$TMP_ROOT/seed" || exit 1
  git checkout -q -b main
  mkdir -p api/migrations
  echo 'CREATE TABLE shift (id INT);' > api/migrations/20261109100000_shift.sql
  git add . && git commit -q -m A
  git tag commitA
  echo 'CREATE TABLE rate_feed (id INT);' > api/migrations/20261110100000_rate_feed.sql
  git add . && git commit -q -m B
  git remote add origin "$TPL_ORIGIN"
  git push -q origin main commitA
)
git clone -q "$TPL_ORIGIN" "$TPL_WORK"

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

# --- fixture -------------------------------------------------------------

# fixture: fresh origin + clone; sets T, WORK, GH_LOG, GH_STUB_JSON, PATH.
fixture() {
  T=$(mktemp -d "$TMP_ROOT/case.XXXXXX")
  WORK="$T/work"
  GH_LOG="$T/gh.log"
  GH_STUB_JSON="$T/prs.json"
  : > "$GH_LOG"
  echo '[]' > "$GH_STUB_JSON"
  unset GH_STUB_EXIT
  unset GH_API_STUB_EXIT GH_API_STUB_FAIL_PR
  export GH_LOG GH_STUB_JSON T

  mkdir -p "$T/bin"
  cat > "$T/bin/gh" << 'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
if [ -n "${GH_STUB_EXIT:-}" ]; then
  echo "gh: stub failure (HTTP 502)" >&2
  exit "$GH_STUB_EXIT"
fi
if [ "${1:-}" = "pr" ] && [ "${2:-}" = "list" ]; then
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
  exit 0
elif [ "${1:-}" = "api" ]; then
  if [ -n "${GH_API_STUB_EXIT:-}" ]; then
    echo "gh: stub api failure (HTTP 404)" >&2
    exit "$GH_API_STUB_EXIT"
  fi
  endpoint="${2:-}"
  if [[ "$endpoint" =~ pulls/([0-9]+)/files ]]; then
    pr_n="${BASH_REMATCH[1]}"
    if [ -n "${GH_API_STUB_FAIL_PR:-}" ] && [ "$pr_n" = "$GH_API_STUB_FAIL_PR" ]; then
      echo "gh: stub api failure for PR #$pr_n (HTTP 404)" >&2
      exit 1
    fi
    file_list="$T/files-$pr_n.txt"
    if [ -f "$file_list" ]; then
      cat "$file_list"
      exit 0
    else
      echo "gh stub: per-PR file list not found: $file_list" >&2
      exit 1
    fi
  else
    echo "gh stub: unexpected api endpoint: $endpoint" >&2
    exit 99
  fi
else
  echo "gh stub: unexpected call: gh $*" >&2
  exit 99
fi
STUB
  chmod +x "$T/bin/gh"
  PATH="$T/bin:$ORIG_PATH"
  export PATH

  cp -R "$TPL_ORIGIN" "$T/origin.git"
  cp -R "$TPL_WORK" "$WORK"
  git -C "$WORK" remote set-url origin "$T/origin.git"
}

# branch_from <ref> <branch> <file>...: a PR branch adding the given migration
# files (each with distinct SQL, including a byte that must survive: a tab and
# no trailing newline), pushed to origin.
branch_from() {
  local ref="$1" br="$2"
  shift 2
  git -C "$WORK" checkout -q -b "$br" "$ref"
  local f
  for f in "$@"; do
    printf 'CREATE TABLE %s (\n\tid INT\n);' "$(basename "$f" .sql)" > "$WORK/$f"
    git -C "$WORK" add "$f"
  done
  git -C "$WORK" commit -q -m "feat: $br"
  git -C "$WORK" push -q origin "$br"
}

# prs_json '<number> <branch> <path>[,<path>...]' ...
prs_json() {
  local first=1 spec n br paths p
  {
    printf '['
    for spec in "$@"; do
      n=$(echo "$spec" | cut -d' ' -f1)
      br=$(echo "$spec" | cut -d' ' -f2)
      paths=$(echo "$spec" | cut -d' ' -f3)
      [ "$first" -eq 1 ] || printf ','
      first=0
      printf '{"number":%s,"headRefName":"%s","files":[' "$n" "$br"
      local pf=1
      for p in $(echo "$paths" | tr ',' ' '); do
        [ "$pf" -eq 1 ] || printf ','
        pf=0
        printf '{"path":"%s","additions":1,"deletions":0}' "$p"
      done
      printf ']}'
    done
    printf ']\n'
  } > "$GH_STUB_JSON"
}

listing() {
  (cd "$WORK/$M" && printf '%s ' *.sql)
}

run_sut() {
  OUT="$T/out"
  ERR="$T/err"
  RC=0
  (cd "$WORK" && bash "$SUT" "$@") > "$OUT" 2> "$ERR" || RC=$?
}

expect_stdout() {
  local want="$1" got
  got=$(cat "$OUT")
  if [ "$RC" -ne 0 ]; then
    fail "exit $RC (stderr: $(tr '\n' ' ' < "$ERR"))"
  fi
  if [ "$got" != "$want" ]; then
    fail "stdout '$got', want exactly '$want'"
  fi
}

# --- next ----------------------------------------------------------------

begin "N1 next = main's max + 1h; gh asked for open PRs with --limit >= 200"
fixture
run_sut next
expect_stdout 20261110110000
if ! grep -q -- 'pr list' "$GH_LOG" || ! grep -q -- '--state open' "$GH_LOG"; then
  fail "gh was not asked 'pr list --state open' (log: $(tr '\n' ' ' < "$GH_LOG"))"
fi
limit=$(sed -nE 's/.*--limit[ =]([0-9]+).*/\1/p' "$GH_LOG" | head -n 1)
if [ -z "$limit" ] || [ "$limit" -lt 200 ]; then
  fail "gh pr list --limit is '${limit:-unset}', want >= 200 (default 30 misses PRs)"
fi
end

begin "N2 next skips a slot an open PR holds"
fixture
prs_json "1070 feat/ledger $M/20261110110000_ledger.sql,api/src/lib.rs"
run_sut next
expect_stdout 20261110120000
end

begin "N3 next with two open PRs at the same max"
fixture
prs_json "1070 feat/a $M/20261110110000_a.sql" \
  "1071 feat/b $M/20261110110000_b.sql" \
  "1072 feat/c web/src/x.ts" \
  "1073 feat/d $M/20261109110000_old.sql"
run_sut next
expect_stdout 20261110120000
end

begin "N4a next rolls the day (23:00 + 1h)"
fixture
prs_json "1070 feat/late $M/20261110230000_late.sql"
run_sut next
expect_stdout 20261111000000
end

begin "N4b next rolls the year (Dec 31 23:00 + 1h)"
fixture
prs_json "1070 feat/nye $M/20261231230000_nye.sql"
run_sut next
expect_stdout 20270101000000
end

begin "N5 gh failure -> non-zero, message, no version printed"
fixture
GH_STUB_EXIT=1
export GH_STUB_EXIT
run_sut next
unset GH_STUB_EXIT
if [ "$RC" -eq 0 ]; then fail "exit 0, want non-zero"; fi
if [ "$RC" -eq 2 ]; then fail "exit 2 is the scaffold's 'not implemented'"; fi
if [ -s "$OUT" ]; then fail "stdout not empty: $(cat "$OUT")"; fi
if ! grep -qi 'gh' "$ERR"; then fail "stderr does not name gh: $(cat "$ERR")"; fi
end

begin "N6 next --name prints the file path; a non-snake name is refused"
fixture
run_sut next --name ledger
expect_stdout "$M/20261110110000_ledger.sql"
run_sut next --name Ledger-Accounts
if [ "$RC" -eq 0 ] || [ "$RC" -eq 2 ]; then fail "--name Ledger-Accounts exit $RC, want 1"; fi
if [ -s "$OUT" ]; then fail "--name Ledger-Accounts printed: $(cat "$OUT")"; fi
end

# --- renumber ------------------------------------------------------------

begin "R1 renumber <pr> below main: pure rename, bytes identical, exact commit"
fixture
branch_from commitA feat/outbox "$M/20261109110000_outbox.sql"
cp "$WORK/$M/20261109110000_outbox.sql" "$T/orig.sql"
before=$(git -C "$WORK" rev-parse HEAD)
prs_json "1016 feat/outbox $M/20261109110000_outbox.sql"
run_sut renumber 1016
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(tr '\n' ' ' < "$ERR"))"; fi
if [ -e "$WORK/$M/20261109110000_outbox.sql" ]; then fail "old file still there"; fi
if [ ! -f "$WORK/$M/20261110110000_outbox.sql" ]; then
  fail "new file $M/20261110110000_outbox.sql missing"
elif ! cmp -s "$T/orig.sql" "$WORK/$M/20261110110000_outbox.sql"; then
  fail "SQL bytes changed"
fi
subject=$(git -C "$WORK" log -1 --format=%s)
want_subject="chore(migrations): renumber 20261109110000 → 20261110110000 at merge time"
if [ "$subject" != "$want_subject" ]; then fail "commit subject '$subject', want '$want_subject'"; fi
if [ "$(git -C "$WORK" rev-parse HEAD~1 2>/dev/null)" != "$before" ]; then fail "want exactly one new commit on top of the PR tip"; fi
status=$(git -C "$WORK" diff -M100% --name-status HEAD~1 HEAD 2>/dev/null | tr '\t' ' ')
if [ "$status" != "R100 $M/20261109110000_outbox.sql $M/20261110110000_outbox.sql" ]; then
  fail "commit is not one pure rename: '$status'"
fi
if [ -n "$(git -C "$WORK" status --porcelain)" ]; then fail "tree left dirty"; fi
if ! grep -qF '20261109110000 → 20261110110000' "$OUT"; then fail "stdout does not print old → new: $(cat "$OUT")"; fi
end

begin "R2 renumber when already above main is a no-op"
fixture
branch_from main feat/ok "$M/20261110110000_ok.sql"
before=$(git -C "$WORK" rev-parse HEAD)
prs_json "1080 feat/ok $M/20261110110000_ok.sql"
run_sut renumber 1080
if [ "$RC" -ne 0 ]; then fail "exit $RC, want 0 (stderr: $(tr '\n' ' ' < "$ERR"))"; fi
if [ "$(git -C "$WORK" rev-parse HEAD)" != "$before" ]; then fail "HEAD moved"; fi
if ! grep -q 'nothing to renumber' "$OUT"; then fail "stdout lacks 'nothing to renumber': $(cat "$OUT")"; fi
end

begin "R3 renumber <branch> picks above other open PRs too"
fixture
branch_from commitA feat/outbox "$M/20261109110000_outbox.sql"
prs_json "1016 feat/outbox $M/20261109110000_outbox.sql" \
  "1070 feat/ledger $M/20261110110000_ledger.sql" \
  "1071 feat/fx $M/20261110120000_fx.sql"
run_sut renumber feat/outbox
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(tr '\n' ' ' < "$ERR"))"; fi
if [ ! -f "$WORK/$M/20261110130000_outbox.sql" ]; then
  fail "want $M/20261110130000_outbox.sql; have: $(listing)"
fi
end

begin "R4 renumber two below-main migrations: order kept, one commit each"
fixture
branch_from commitA feat/two "$M/20261109110000_a.sql" "$M/20261109120000_b.sql"
before=$(git -C "$WORK" rev-parse HEAD)
prs_json "1090 feat/two $M/20261109110000_a.sql,$M/20261109120000_b.sql"
run_sut renumber 1090
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(tr '\n' ' ' < "$ERR"))"; fi
if [ ! -f "$WORK/$M/20261110110000_a.sql" ] || [ ! -f "$WORK/$M/20261110120000_b.sql" ]; then
  fail "want a -> 20261110110000, b -> 20261110120000; have: $(listing)"
fi
subjects=$(git -C "$WORK" log --format=%s "$before..HEAD" 2>/dev/null)
want_subjects="chore(migrations): renumber 20261109120000 → 20261110120000 at merge time
chore(migrations): renumber 20261109110000 → 20261110110000 at merge time"
if [ "$subjects" != "$want_subjects" ]; then fail "commits were: $subjects"; fi
end

begin "R5 renumber refuses a dirty tree and the wrong checkout"
fixture
branch_from commitA feat/outbox "$M/20261109110000_outbox.sql"
prs_json "1016 feat/outbox $M/20261109110000_outbox.sql"
before=$(git -C "$WORK" rev-parse HEAD)
echo 'stray' > "$WORK/README.local"
git -C "$WORK" add README.local
run_sut renumber 1016
if [ "$RC" -eq 0 ] || [ "$RC" -eq 2 ]; then fail "dirty tree: exit $RC, want 1"; fi
if [ "$(git -C "$WORK" rev-parse HEAD)" != "$before" ]; then fail "dirty tree: HEAD moved"; fi
if [ ! -f "$WORK/$M/20261109110000_outbox.sql" ]; then fail "dirty tree: file renamed"; fi
git -C "$WORK" reset -q --hard
git -C "$WORK" checkout -q main
run_sut renumber 1016
if [ "$RC" -eq 0 ] || [ "$RC" -eq 2 ]; then fail "checkout main, renumber 1016: exit $RC, want 1"; fi
if ! grep -q 'feat/outbox' "$ERR"; then fail "wrong checkout: stderr does not name feat/outbox: $(cat "$ERR")"; fi
end

begin "R6 renumber with gh failing changes nothing"
fixture
branch_from commitA feat/outbox "$M/20261109110000_outbox.sql"
before=$(git -C "$WORK" rev-parse HEAD)
GH_STUB_EXIT=1
export GH_STUB_EXIT
run_sut renumber feat/outbox
unset GH_STUB_EXIT
if [ "$RC" -eq 0 ] || [ "$RC" -eq 2 ]; then fail "exit $RC, want non-zero and not the scaffold's 2"; fi
if [ "$(git -C "$WORK" rev-parse HEAD)" != "$before" ]; then fail "HEAD moved"; fi
if [ -n "$(git -C "$WORK" status --porcelain)" ]; then fail "tree changed"; fi
end

# --- 100+ file PRs and REST pagination (T1-T4, #1529) ---------------------

begin "T1 next with 100-file PR listing 120 files via REST skips migration slot"
fixture
python3 -c '
import json
files = [{"path": f"src/file_{i}.rs", "additions": 1, "deletions": 0} for i in range(100)]
data = [{"number": 120, "headRefName": "feat/big", "files": files}]
print(json.dumps(data))
' > "$GH_STUB_JSON"
python3 -c '
for i in range(119):
    print(f"src/file_{i}.rs")
print("api/migrations/20261111120000_foo.sql")
' > "$T/files-120.txt"
run_sut next
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(cat "$ERR"))"; fi
got=$(cat "$OUT")
if [ "$got" != "20261111130000" ]; then fail "stdout '$got', want '20261111130000'"; fi
end

begin "T2 next with 100-file PR listing 100 non-migration files via REST does not refuse"
fixture
python3 -c '
import json
files = [{"path": f"src/file_{i}.rs", "additions": 1, "deletions": 0} for i in range(100)]
data = [{"number": 121, "headRefName": "feat/clean", "files": files}]
print(json.dumps(data))
' > "$GH_STUB_JSON"
python3 -c '
for i in range(100):
    print(f"src/file_{i}.rs")
' > "$T/files-121.txt"
run_sut next
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(cat "$ERR"))"; fi
got=$(cat "$OUT")
if [ "$got" != "20261110110000" ]; then fail "stdout '$got', want '20261110110000'"; fi
end

begin "T3 next with 100-file PR whose REST call fails exits non-zero"
fixture
python3 -c '
import json
files = [{"path": f"src/file_{i}.rs", "additions": 1, "deletions": 0} for i in range(100)]
data = [{"number": 122, "headRefName": "feat/failing", "files": files}]
print(json.dumps(data))
' > "$GH_STUB_JSON"
GH_API_STUB_FAIL_PR=122
export GH_API_STUB_FAIL_PR
run_sut next
unset GH_API_STUB_FAIL_PR
if [ "$RC" -eq 0 ] || [ "$RC" -eq 2 ]; then fail "exit $RC, want 1"; fi
if [ -s "$OUT" ]; then fail "stdout not empty: $(cat "$OUT")"; fi
if ! grep -q "gh api failed for PR #122" "$ERR"; then
  fail "stderr does not name PR and API failure: $(cat "$ERR")"
fi
end

begin "T4 renumber with 100+ file PR holding a migration lands above it"
fixture
branch_from commitA feat/outbox "$M/20261109110000_outbox.sql"
python3 -c '
import json
files_888 = [{"path": f"src/file_{i}.rs", "additions": 1, "deletions": 0} for i in range(100)]
files_1016 = [{"path": "api/migrations/20261109110000_outbox.sql", "additions": 1, "deletions": 0}]
data = [
    {"number": 888, "headRefName": "feat/big", "files": files_888},
    {"number": 1016, "headRefName": "feat/outbox", "files": files_1016}
]
print(json.dumps(data))
' > "$GH_STUB_JSON"
python3 -c '
for i in range(119):
    print(f"src/file_{i}.rs")
print("api/migrations/20261112100000_big_mig.sql")
' > "$T/files-888.txt"
run_sut renumber 1016
if [ "$RC" -ne 0 ]; then fail "exit $RC (stderr: $(cat "$ERR"))"; fi
if [ ! -f "$WORK/$M/20261112110000_outbox.sql" ]; then
  fail "want $M/20261112110000_outbox.sql, files: $(ls "$WORK/$M")"
fi
if ! grep -qF '20261109110000 → 20261112110000' "$OUT"; then
  fail "stdout does not report old -> new: $(cat "$OUT")"
fi
end

# --- the gate's fix line -------------------------------------------------

begin "G1 migration-order.sh failure prints the renumber command"
fixture
branch_from commitA feat/outbox "$M/20261109110000_outbox.sql"
gate_out=$(cd "$WORK" && PR_NUMBER=1016 bash "$GATE" origin/main feat/outbox 2>&1)
gate_rc=$?
if [ "$gate_rc" -ne 1 ]; then fail "gate exit $gate_rc, want 1 (its rule is unchanged)"; fi
if ! printf '%s\n' "$gate_out" | grep -qF 'scripts/migration-slot.sh renumber 1016'; then
  fail "with PR_NUMBER=1016 the output lacks 'scripts/migration-slot.sh renumber 1016': $gate_out"
fi
gate_out=$(cd "$WORK" && env -u PR_NUMBER bash "$GATE" origin/main feat/outbox 2>&1)
if ! printf '%s\n' "$gate_out" | grep -qF 'scripts/migration-slot.sh renumber <pr-number>'; then
  fail "without PR_NUMBER the output lacks 'scripts/migration-slot.sh renumber <pr-number>': $gate_out"
fi
end

echo
if [ "$FAILED" -ne 0 ]; then
  echo "$FAILED of $TOTAL migration-slot cases failed."
  exit 1
fi
echo "All $TOTAL migration-slot cases passed."
