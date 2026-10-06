#!/usr/bin/env bash
set -euo pipefail

dir="${1:-}"
workflow_file="${2:-}"
event="${3:-}"
base="${4:-}"
head="${5:-}"

if [ -z "$dir" ] || [ -z "$workflow_file" ] || [ -z "$event" ]; then
  echo "Usage: $0 <dir> <workflow-file> <event> [base] [head]" >&2
  echo "true"
  exit 0
fi

# Any event other than pull_request or push always builds
if [ "$event" != "pull_request" ] && [ "$event" != "push" ]; then
  echo "Event '$event' is not pull_request or push — running build." >&2
  echo "true"
  exit 0
fi

# For push events, if before is empty or all zeros (new branch/tag), run build
if [ "$event" = "push" ]; then
  if [ -z "$base" ] || [ "$base" = "0000000000000000000000000000000000000000" ]; then
    echo "Push event with empty or initial before sha — running build." >&2
    echo "true"
    exit 0
  fi
fi

if [ -z "$base" ] || [ -z "$head" ]; then
  echo "Missing base or head commit sha — running build." >&2
  echo "true"
  exit 0
fi

range="$base...$head"

# 1. Decide "build" by exit code of git diff with pathspecs:
# Exit 1 => watched path changed => true
# Any exit other than 0 or 1 => git diff error => true
set +e
git diff --quiet --no-renames "$range" -- "$dir/" "$workflow_file" .github/scripts/
diff_status=$?
set -e

if [ "$diff_status" -eq 1 ]; then
  echo "Changes detected under watched paths ($dir/, $workflow_file, or .github/scripts/) — running build." >&2
  echo "true"
  exit 0
elif [ "$diff_status" -ne 0 ]; then
  echo "git diff exited with $diff_status for range $range — running build." >&2
  echo "true"
  exit 0
fi

# 2. No watched paths changed. Check if the entire diff is empty.
set +e
git diff --quiet --no-renames "$range"
all_diff_status=$?
set -e

if [ "$all_diff_status" -eq 0 ]; then
  if [ "$event" = "pull_request" ]; then
    echo "Empty pull_request diff — running build." >&2
    echo "true"
  else
    echo "Empty push diff — skipping build." >&2
    echo "false"
  fi
  exit 0
elif [ "$all_diff_status" -ne 1 ]; then
  echo "git diff exited with $all_diff_status for range $range — running build." >&2
  echo "true"
  exit 0
fi

# 3. Changes exist outside watched paths. Check if all changes are docs/ or *.md.
# Read names NUL-separated (-z) so non-ASCII, quotes, tabs, and spaces are preserved.
all_docs=true
while IFS= read -r -d '' file; do
  [ -z "$file" ] && continue
  case "$file" in
    docs/* | *.md)
      ;;
    *)
      all_docs=false
      break
      ;;
  esac
done < <(git diff --no-renames --name-only -z "$range")

if [ "$event" = "pull_request" ]; then
  if [ "$all_docs" = true ]; then
    echo "Diff contains only docs/ or *.md — skipping build." >&2
    echo "false"
  else
    echo "No changes under $dir/ or shared CI files — skipping build." >&2
    echo "false"
  fi
elif [ "$event" = "push" ]; then
  echo "Push does not touch $dir/ or shared CI file — skipping build." >&2
  echo "false"
fi
