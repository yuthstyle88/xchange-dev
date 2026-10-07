#!/usr/bin/env bash
# Self-test of sweep-merged-branches.sh (issue #162). Written before the script; do not
# edit, delete or weaken a case to make the script pass.
set -euo pipefail
export LC_ALL=C

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SWEEP="$SCRIPT_DIR/sweep-merged-branches.sh"

if [ ! -x "$SWEEP" ]; then
  echo "Error: $SWEEP not found or not executable" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

ORIGIN="$TMP_DIR/origin.git"
WORK="$TMP_DIR/work"
git init -q --bare "$ORIGIN"
git init -q "$WORK"
cd "$WORK"
git config user.email "test@example.com"
git config user.name "Test User"
git checkout -q -b main
git remote add origin "$ORIGIN"

commit() {
  git add -A
  git commit -q -m "$1"
}

# --- base -----------------------------------------------------------------------------
mkdir -p src
printf 'a1\na2\na3\na4\na5\n' > src/a.txt
printf 'b1\nb2\n' > src/b.txt
printf '\000\001\002' > bin.dat
commit "base"
BASE=$(git rev-parse HEAD)
git push -q origin main

branch_at() { # name sha
  git push -q origin "$2:refs/heads/$1"
}
branch_with() { # name, then the commands that make its one commit
  local name=$1
  shift
  git checkout -q -b "$name" "$BASE"
  "$@"
  commit "$name"
  git push -q origin "$name"
  git checkout -q main
}

add_test_file() { mkdir -p tests && printf 'fn one() {}\nfn two() {}\n' > tests/t.rs; }
add_other_test_file() { mkdir -p tests && printf 'fn three() {}\n' > tests/u.rs; }
add_unique_line() { printf 'a1\na2\na3\na4\na5\nUNIQUE LINE\n' > src/a.txt; }
add_unique_file() { mkdir -p x && printf 'only here\n' > x/only_here.txt; }
remove_a3() { printf 'a1\na2\na4\na5\n' > src/a.txt; }
change_binary() { printf '\000\001\003' > bin.dat; }
add_pull_ref_file() { mkdir -p p && printf 'kept by a pull request\n' > p/p.txt; }

branch_at test/ancestor "$BASE"
branch_with test/squashed add_test_file
branch_with test/squashed-then-edited add_other_test_file
branch_with test/unique-line add_unique_line
branch_with test/unique-file add_unique_file
branch_with test/removes-line remove_a3
branch_with test/binary change_binary
branch_with test/pull-ref add_pull_ref_file
git push -q origin "test/pull-ref:refs/pull/7/head"
for protected in release/1.0 deploy/staging backup/x archive/x auto/dev-sync; do
  branch_at "$protected" "$BASE"
done
branch_at feat/open-pr "$BASE"
branch_at test/lease "$BASE"

# --- main moves on: the two test branches land as squash commits, one is edited after ----
printf 'b1\nb2\nb3\n' > src/b.txt
commit "unrelated change on main"
add_test_file
commit "squash of test/squashed"
add_other_test_file
commit "squash of test/squashed-then-edited"
printf 'fn three() {}\nfn four() {}\n' > tests/u.rs
commit "a later edit on main"
git push -q origin main

KEEP_FILE="$TMP_DIR/keep"
printf 'feat/open-pr\n' > "$KEEP_FILE"

failures=0
check() { # output-file name expected-verdict expected-proof
  local got
  got=$(awk -F'\t' -v n="$2" '$2 == n { print $1 "\t" $4 }' "$1")
  if [ "$got" = "$3	$4" ]; then
    echo "ok - $2: $3 $4"
  else
    echo "not ok - $2: expected '$3 $4', got '${got:-no line}'"
    failures=$((failures + 1))
  fi
}

OUT="$TMP_DIR/dry.tsv"
SWEEP_KEEP_FILE="$KEEP_FILE" SWEEP_MIN_AGE_HOURS=0 "$SWEEP" > "$OUT"

check "$OUT" test/ancestor DELETE ancestor
check "$OUT" test/squashed DELETE content-in-main
check "$OUT" test/squashed-then-edited DELETE content-in-main
check "$OUT" test/unique-line KEEP unique-content
check "$OUT" test/unique-file KEEP unique-content
check "$OUT" test/removes-line KEEP unique-content
check "$OUT" test/binary KEEP unique-content
check "$OUT" test/pull-ref DELETE pull-ref
for protected in release/1.0 deploy/staging backup/x archive/x auto/dev-sync; do
  check "$OUT" "$protected" KEEP protected
done
check "$OUT" feat/open-pr KEEP open-pr

if awk -F'\t' '$2 == "main"' "$OUT" | grep -q .; then
  echo "not ok - main must never be listed"
  failures=$((failures + 1))
else
  echo "ok - main is never listed"
fi

# A dry run deletes nothing.
if git ls-remote --exit-code --heads origin test/ancestor > /dev/null; then
  echo "ok - a dry run deletes nothing"
else
  echo "not ok - a dry run deleted test/ancestor"
  failures=$((failures + 1))
fi

# Young branches are kept whatever the proof.
YOUNG="$TMP_DIR/young.tsv"
SWEEP_KEEP_FILE="$KEEP_FILE" SWEEP_MIN_AGE_HOURS=1 "$SWEEP" > "$YOUNG"
check "$YOUNG" test/ancestor KEEP young

# A branch pushed to after it was classified is not deleted: the lease is the sha seen.
if (
  # shellcheck source=/dev/null
  source "$SWEEP"
  delete_branch origin test/lease 0000000000000000000000000000000000000001
) > /dev/null 2>&1; then
  echo "not ok - delete_branch ignored a stale sha"
  failures=$((failures + 1))
elif git ls-remote --exit-code --heads origin test/lease > /dev/null; then
  echo "ok - a stale sha deletes nothing"
else
  echo "not ok - test/lease was deleted under a stale sha"
  failures=$((failures + 1))
fi

# --apply deletes exactly the DELETE lines and prints how to restore each.
APPLIED="$TMP_DIR/apply.tsv"
SWEEP_KEEP_FILE="$KEEP_FILE" SWEEP_MIN_AGE_HOURS=0 "$SWEEP" --apply > "$APPLIED"
remaining=$(git ls-remote --heads origin | sed 's#.*refs/heads/##' | sort | tr '\n' ' ')
expected="archive/x auto/dev-sync backup/x deploy/staging feat/open-pr main release/1.0 test/binary test/removes-line test/unique-file test/unique-line "
if [ "$remaining" = "$expected" ]; then
  echo "ok - --apply deleted exactly the DELETE branches"
else
  echo "not ok - after --apply: '$remaining', expected '$expected'"
  failures=$((failures + 1))
fi
restores=$(grep -c '^restore: git push origin [0-9a-f]\{40\}:refs/heads/' "$APPLIED" || true)
if [ "$restores" = "5" ]; then
  echo "ok - --apply printed one restore line per deleted branch"
else
  echo "not ok - expected 5 restore lines, got $restores"
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all checks passed"
