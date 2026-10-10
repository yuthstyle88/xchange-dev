#!/usr/bin/env bash
# Tests for migration-order.sh (#847)
# Verifies that PRs cannot add migrations whose versions sort below or equal to
# main's highest migration version.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SUT="$SCRIPT_DIR/migration-order.sh"

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

# Initial base commit with an existing migration
mkdir -p api/migrations
cat << 'EOF' > api/migrations/20261010000000_init.sql
CREATE TABLE test (id INT);
EOF
git add .
git commit -q -m "initial commit on main"
BASE_BRANCH="main"
git branch -M "$BASE_BRANCH"
BASE_C0=$(git rev-parse HEAD)

FAILED=0
TOTAL=0

assert_case() {
  local desc="$1"
  local expected_exit="$2"
  local expected_output="$3"
  local base_ref="$4"
  local head_ref="$5"
  local forbid_output="${6:-}"

  TOTAL=$((TOTAL + 1))
  local actual_output
  local actual_exit=0
  actual_output=$(bash "$SUT" "$base_ref" "$head_ref" 2>&1) || actual_exit=$?

  local ok=1
  if [ "$actual_exit" -ne "$expected_exit" ]; then
    echo "✗ $desc: expected exit $expected_exit, got $actual_exit"
    echo "  output was: $actual_output"
    ok=0
  fi

  if [ -n "$expected_output" ]; then
    if ! echo "$actual_output" | grep -Fq "$expected_output"; then
      echo "✗ $desc: output did not contain expected: '$expected_output'"
      echo "  actual output: $actual_output"
      ok=0
    fi
  fi

  if [ -n "$forbid_output" ]; then
    if echo "$actual_output" | grep -Fq "$forbid_output"; then
      echo "✗ $desc: output contained forbidden text: '$forbid_output'"
      echo "  actual output: $actual_output"
      ok=0
    fi
  fi

  if [ "$ok" -eq 1 ]; then
    echo "✓ $desc"
  else
    FAILED=$((FAILED + 1))
  fi
}

# --- Case 1: no migration added -> 0 ---
git checkout -q -b pr-no-migration "$BASE_C0"
echo "hello" > api/file.txt
git add api/file.txt
git commit -q -m "touch non-migration file"
HEAD_NO_MIG=$(git rev-parse HEAD)
assert_case "1. no migration added -> 0" 0 \
  "migration order: no new migrations" \
  "$BASE_BRANCH" "$HEAD_NO_MIG"

# --- Case 2: one added above the base max -> 0 ---
git checkout -q -b pr-valid-migration "$BASE_C0"
echo "CREATE TABLE foo ();" > api/migrations/20261015000000_foo.sql
git add api/migrations/20261015000000_foo.sql
git commit -q -m "add valid migration"
HEAD_VALID=$(git rev-parse HEAD)
assert_case "2. one added above the base max -> 0" 0 \
  "migration order: ok (base max 20261010000000)" \
  "$BASE_BRANCH" "$HEAD_VALID"

# --- Case 3: one added below -> 1, and the file is named ---
git checkout -q -b pr-lower-migration "$BASE_C0"
echo "CREATE TABLE bar ();" > api/migrations/20261005000000_bar.sql
git add api/migrations/20261005000000_bar.sql
git commit -q -m "add lower migration"
HEAD_LOWER=$(git rev-parse HEAD)
assert_case "3. one added below -> 1, and the file is named" 1 \
  "migration order: api/migrations/20261005000000_bar.sql (20261005000000) is not after main's highest 20261010000000; rename it to a version after 20261010000000" \
  "$BASE_BRANCH" "$HEAD_LOWER"

# --- Case 4: one added equal to the base max -> 1 ---
git checkout -q -b pr-equal-migration "$BASE_C0"
echo "CREATE TABLE collision ();" > api/migrations/20261010000000_collision.sql
git add api/migrations/20261010000000_collision.sql
git commit -q -m "add collision migration with equal version"
HEAD_EQUAL=$(git rev-parse HEAD)
assert_case "4. one added equal to the base max -> 1" 1 \
  "migration order: api/migrations/20261010000000_collision.sql (20261010000000) is not after main's highest 20261010000000; rename it to a version after 20261010000000" \
  "$BASE_BRANCH" "$HEAD_EQUAL"

# --- Case 5: two added, one below -> 1, and only that one is named ---
git checkout -q -b pr-two-migrations "$BASE_C0"
echo "CREATE TABLE bad ();" > api/migrations/20261005000000_bad.sql
echo "CREATE TABLE good ();" > api/migrations/20261020000000_good.sql
git add api/migrations/20261005000000_bad.sql api/migrations/20261020000000_good.sql
git commit -q -m "add one bad and one good migration"
HEAD_TWO=$(git rev-parse HEAD)
assert_case "5. two added, one below -> 1, and only that one is named" 1 \
  "migration order: api/migrations/20261005000000_bad.sql (20261005000000) is not after main's highest 20261010000000; rename it to a version after 20261010000000" \
  "$BASE_BRANCH" "$HEAD_TWO" \
  "20261020000000_good.sql"

# --- Case 6: an existing migration renamed to a lower version (diff status R) -> 1 ---
# Add an existing migration on main so base max becomes 20261020000000
git checkout -q "$BASE_BRANCH"
echo "CREATE TABLE existing ();" > api/migrations/20261020000000_existing.sql
git add api/migrations/20261020000000_existing.sql
git commit -q -m "add second migration on main"
BASE_C1=$(git rev-parse HEAD)

git checkout -q -b pr-renamed-migration "$BASE_C1"
git mv api/migrations/20261020000000_existing.sql api/migrations/20261005000000_renamed.sql
git commit -q -m "rename existing migration to lower version"
HEAD_RENAME=$(git rev-parse HEAD)
assert_case "6. an existing migration renamed to a lower version (diff status R) -> 1" 1 \
  "migration order: api/migrations/20261005000000_renamed.sql (20261005000000) is not after main's highest 20261020000000; rename it to a version after 20261020000000" \
  "$BASE_BRANCH" "$HEAD_RENAME"

# --- Case 7: a file added on the base after the PR branched, higher than the PR's file -> 1 ---
# PR branched from BASE_C0 (when highest was 20261010000000), adds 20261015000000.
# Main then moved to BASE_C1 (which added 20261020000000).
# Against BASE_BRANCH (tip = BASE_C1), PR's 20261015000000 is now lower than main's highest!
assert_case "7. file added on base after PR branched, higher than PR file -> 1" 1 \
  "migration order: api/migrations/20261015000000_foo.sql (20261015000000) is not after main's highest 20261020000000; rename it to a version after 20261020000000" \
  "$BASE_BRANCH" "$HEAD_VALID"

# --- Case 8: a malformed name -> 1 ---
git checkout -q -b pr-malformed "$BASE_C0"
echo "CREATE TABLE malformed ();" > api/migrations/malformed.sql
git add api/migrations/malformed.sql
git commit -q -m "add malformed migration name"
HEAD_MALFORMED=$(git rev-parse HEAD)
assert_case "8. a malformed name -> 1" 1 \
  "migration order: api/migrations/malformed.sql has no 14-digit version" \
  "$BASE_BRANCH" "$HEAD_MALFORMED"

# --- Case 9: a non-.sql file under api/migrations/ is ignored -> 0 ---
git checkout -q -b pr-nonsql-file "$BASE_C0"
echo "# Migration Notes" > api/migrations/README.md
git add api/migrations/README.md
git commit -q -m "add README in api/migrations"
HEAD_NONSQL=$(git rev-parse HEAD)
assert_case "9. a non-.sql file under api/migrations/ is ignored -> 0" 0 \
  "migration order: no new migrations" \
  "$BASE_BRANCH" "$HEAD_NONSQL"

echo ""
if [ "$FAILED" -gt 0 ]; then
  echo "Failed $FAILED / $TOTAL tests"
  exit 1
fi

echo "All $TOTAL migration-order tests passed."
exit 0
