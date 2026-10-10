#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SHOULD_BUILD="$SCRIPT_DIR/should-build.sh"

if [ ! -x "$SHOULD_BUILD" ]; then
  echo "Error: $SHOULD_BUILD not found or not executable" >&2
  exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

REPO_DIR="$TMP_DIR/repo"
git init "$REPO_DIR" >/dev/null 2>&1
cd "$REPO_DIR"

git config user.email "test@example.com"
git config user.name "Test User"

# Initial commit with basic repository layout
mkdir -p app api docs .github/workflows .github/scripts
echo "app content" > app/file.txt
echo "api content" > api/file.txt
echo "doc content" > docs/doc.md
echo "app workflow" > .github/workflows/app.yml
echo "api workflow" > .github/workflows/api.yml
echo "shared script" > .github/scripts/should-build.sh
git add .
git commit -m "initial commit" >/dev/null 2>&1
BASE_SHA=$(git rev-parse HEAD)

# Branch: any path under api/
git checkout -q -b pr-api "$BASE_SHA"
echo "api update" >> api/file.txt
git commit -q -am "touch api"
HEAD_API=$(git rev-parse HEAD)

# Branch: any path under app/
git checkout -q -b pr-app "$BASE_SHA"
echo "app update" >> app/file.txt
git commit -q -am "touch app"
HEAD_APP=$(git rev-parse HEAD)

# Branch: shared CI file (api.yml), with docs on top
git checkout -q -b pr-shared-api "$BASE_SHA"
echo "api workflow update" >> .github/workflows/api.yml
git commit -q -am "touch api.yml"
echo "doc update" >> docs/doc.md
git commit -q -am "touch doc"
HEAD_SHARED_API=$(git rev-parse HEAD)

# Branch: shared CI file (app.yml), with docs on top
git checkout -q -b pr-shared-app "$BASE_SHA"
echo "app workflow update" >> .github/workflows/app.yml
git commit -q -am "touch app.yml"
echo "doc update" >> docs/doc.md
git commit -q -am "touch doc"
HEAD_SHARED_APP=$(git rev-parse HEAD)

# Branch: shared CI file (api.yml), head is a merge commit whose own diff is docs-only
git checkout -q -b pr-shared-merge-api "$BASE_SHA"
echo "api workflow change" >> .github/workflows/api.yml
git commit -q -am "touch api.yml on branch"

git checkout -q -b main-docs-api "$BASE_SHA"
echo "docs update on main" >> docs/doc.md
git commit -q -am "touch doc on main"

git checkout -q pr-shared-merge-api
git merge -q --no-ff main-docs-api -m "merge main-docs into pr-shared-merge-api"
HEAD_MERGE_API=$(git rev-parse HEAD)

# Branch: shared CI file (app.yml), head is a merge commit whose own diff is docs-only
git checkout -q -b pr-shared-merge-app "$BASE_SHA"
echo "app workflow change" >> .github/workflows/app.yml
git commit -q -am "touch app.yml on branch"

git checkout -q -b main-docs-app "$BASE_SHA"
echo "docs update on main 2" >> docs/doc.md
git commit -q -am "touch doc on main 2"

git checkout -q pr-shared-merge-app
git merge -q --no-ff main-docs-app -m "merge main-docs into pr-shared-merge-app"
HEAD_MERGE_APP=$(git rev-parse HEAD)

# Branch: only docs/ or *.md
git checkout -q -b pr-docs "$BASE_SHA"
echo "new doc" > docs/new.md
echo "readme" > README.md
git add docs/new.md README.md
git commit -q -m "docs only"
HEAD_DOCS=$(git rev-parse HEAD)

# Branch: non-ASCII name under api/
git checkout -q -b pr-nonascii-api "$BASE_SHA"
echo "thai api" > "api/ภาษาไทย.rs"
git add "api/ภาษาไทย.rs"
git commit -q -m "add thai file in api"
HEAD_NONASCII_API=$(git rev-parse HEAD)

# Branch: non-ASCII name under app/
git checkout -q -b pr-nonascii-app "$BASE_SHA"
echo "thai app" > "app/ภาษาไทย.dart"
git add "app/ภาษาไทย.dart"
git commit -q -m "add thai file in app"
HEAD_NONASCII_APP=$(git rev-parse HEAD)

# Branch: name with double quote under api/
git checkout -q -b pr-quote-api "$BASE_SHA"
echo "quote api" > "api/file\"quote.rs"
git add "api/file\"quote.rs"
git commit -q -m "add quote file in api"
HEAD_QUOTE_API=$(git rev-parse HEAD)

# Branch: name with double quote under app/
git checkout -q -b pr-quote-app "$BASE_SHA"
echo "quote app" > "app/file\"quote.dart"
git add "app/file\"quote.dart"
git commit -q -m "add quote file in app"
HEAD_QUOTE_APP=$(git rev-parse HEAD)

# Branch: name with tab under api/
git checkout -q -b pr-tab-api "$BASE_SHA"
printf "tab api\n" > "api/file	tab.rs"
git add "api/file	tab.rs"
git commit -q -m "add tab file in api"
HEAD_TAB_API=$(git rev-parse HEAD)

# Branch: name with tab under app/
git checkout -q -b pr-tab-app "$BASE_SHA"
printf "tab app\n" > "app/file	tab.dart"
git add "app/file	tab.dart"
git commit -q -m "add tab file in app"
HEAD_TAB_APP=$(git rev-parse HEAD)

# Branch: non-ASCII name under docs/ only
git checkout -q -b pr-nonascii-docs "$BASE_SHA"
echo "thai doc" > "docs/คู่มือ.md"
git add "docs/คู่มือ.md"
git commit -q -m "add thai doc"
HEAD_NONASCII_DOCS=$(git rev-parse HEAD)

# Branch: file name with spaces under api/
git checkout -q -b pr-spaces-api "$BASE_SHA"
echo "spaces api" > "api/file with spaces.rs"
git add "api/file with spaces.rs"
git commit -q -m "add spaces file in api"
HEAD_SPACES_API=$(git rev-parse HEAD)

# Branch: file name with spaces under app/
git checkout -q -b pr-spaces-app "$BASE_SHA"
echo "spaces app" > "app/file with spaces.dart"
git add "app/file with spaces.dart"
git commit -q -m "add spaces file in app"
HEAD_SPACES_APP=$(git rev-parse HEAD)

# Branch: shared CI file under .github/scripts/
git checkout -q -b pr-scripts "$BASE_SHA"
echo "helper" > .github/scripts/helper.sh
git add .github/scripts/helper.sh
git commit -q -m "add helper script"
HEAD_SCRIPTS=$(git rev-parse HEAD)

total=0
failed=0

run_test() {
  local desc="$1"
  local expected="$2"
  local target_dir="$3"
  local target_wf="$4"
  local event="$5"
  local base="$6"
  local head="$7"

  total=$((total + 1))
  local actual
  actual=$(bash "$SHOULD_BUILD" "$target_dir" "$target_wf" "$event" "$base" "$head" 2>/dev/null || echo "error")
  if [ "$actual" = "$expected" ]; then
    echo "ok $total - $desc"
  else
    echo "not ok $total - $desc (expected: $expected, got: $actual)"
    failed=$((failed + 1))
  fi
}

echo "# Running should-build.sh test suite"

for target in "api" "app"; do
  if [ "$target" = "api" ]; then
    wf=".github/workflows/api.yml"
    head_own="$HEAD_API"
    head_other="$HEAD_APP"
    head_shared="$HEAD_SHARED_API"
    head_shared_merge="$HEAD_MERGE_API"
    head_other_wf="$HEAD_SHARED_APP"
    head_nonascii="$HEAD_NONASCII_API"
    head_quote="$HEAD_QUOTE_API"
    head_tab="$HEAD_TAB_API"
    head_spaces="$HEAD_SPACES_API"
  else
    wf=".github/workflows/app.yml"
    head_own="$HEAD_APP"
    head_other="$HEAD_API"
    head_shared="$HEAD_SHARED_APP"
    head_shared_merge="$HEAD_MERGE_APP"
    head_other_wf="$HEAD_SHARED_API"
    head_nonascii="$HEAD_NONASCII_APP"
    head_quote="$HEAD_QUOTE_APP"
    head_tab="$HEAD_TAB_APP"
    head_spaces="$HEAD_SPACES_APP"
  fi

  echo "## Testing for dir: $target (workflow: $wf)"

  # Row 1: pull_request | empty diff | true
  run_test "[$target] pull_request empty diff -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$BASE_SHA"

  # Row 2: pull_request | any path under <dir>/ | true
  run_test "[$target] pull_request path under <dir>/ -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_own"

  # Row 3: pull_request | a shared CI file, with or without docs, whatever the top commit is | true
  run_test "[$target] pull_request shared CI file with docs on top -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_shared"

  # Row 4: pull_request | a shared CI file, head is a merge commit whose own diff is docs-only | true
  run_test "[$target] pull_request shared CI file with merge commit whose diff is docs-only -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_shared_merge"

  # Row 5: pull_request | only docs/ or *.md | false
  run_test "[$target] pull_request only docs/ or *.md -> false" "false" "$target" "$wf" "pull_request" "$BASE_SHA" "$HEAD_DOCS"

  # Row 6: pull_request | only the other app's directory | false
  run_test "[$target] pull_request only other app dir -> false" "false" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_other"

  # Row 7: pull_request | only the other workflow file | false
  run_test "[$target] pull_request only other workflow file -> false" "false" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_other_wf"

  # Row 8: push | <dir>/ or a shared CI file | true
  run_test "[$target] push touches <dir>/ -> true" "true" "$target" "$wf" "push" "$BASE_SHA" "$head_own"
  run_test "[$target] push touches workflow file -> true" "true" "$target" "$wf" "push" "$BASE_SHA" "$head_shared"

  # Row 9: push | neither | false
  run_test "[$target] push touches neither (only docs) -> false" "false" "$target" "$wf" "push" "$BASE_SHA" "$HEAD_DOCS"
  run_test "[$target] push touches neither (only other app dir) -> false" "false" "$target" "$wf" "push" "$BASE_SHA" "$head_other"

  # Row 10: push | before is forty zeros or empty | true
  run_test "[$target] push before is forty zeros -> true" "true" "$target" "$wf" "push" "0000000000000000000000000000000000000000" "$HEAD_DOCS"
  run_test "[$target] push before is empty -> true" "true" "$target" "$wf" "push" "" "$HEAD_DOCS"

  # Row 11: any other event | — | true
  run_test "[$target] any other event (workflow_dispatch) -> true" "true" "$target" "$wf" "workflow_dispatch" "$BASE_SHA" "$HEAD_DOCS"

  # Row 12: any | git diff fails (unknown sha) | true
  run_test "[$target] git diff fails on unknown sha -> true" "true" "$target" "$wf" "pull_request" "0123456789abcdef0123456789abcdef01234567" "fedcba9876543210fedcba9876543210fedcba98"

  # Special character edge cases (rework order item 1):
  # 1. non-ASCII name under <dir>/ => true
  run_test "[$target] pull_request non-ASCII name under <dir>/ -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_nonascii"

  # 2. name containing a double quote under <dir>/ => true
  run_test "[$target] pull_request name with double quote under <dir>/ -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_quote"

  # 3. name containing a tab under <dir>/ => true
  run_test "[$target] pull_request name with tab under <dir>/ -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_tab"

  # 4. non-ASCII name under docs/ only => false
  run_test "[$target] pull_request non-ASCII name under docs/ only -> false" "false" "$target" "$wf" "pull_request" "$BASE_SHA" "$HEAD_NONASCII_DOCS"

  # 5. file name with spaces under <dir>/ => true
  run_test "[$target] pull_request file name with spaces under <dir>/ -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$head_spaces"

  # 6. shared CI file under .github/scripts/ => true
  run_test "[$target] pull_request shared CI file under .github/scripts/ -> true" "true" "$target" "$wf" "pull_request" "$BASE_SHA" "$HEAD_SCRIPTS"
done

echo "# Results: $total tests, $failed failures"
if [ "$failed" -eq 0 ]; then
  echo "All $total cases passed."
  exit 0
else
  echo "$failed of $total cases failed."
  exit 1
fi
