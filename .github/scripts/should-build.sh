#!/usr/bin/env bash
set -euo pipefail

is_pathspec=false
if [ "${1:-}" = "--pathspec" ] || [ "${1:-}" = "--paths" ] || [ "${1:-}" = "--pathspecs" ]; then
  is_pathspec=true
  shift
  target_paths="${1:-}"
  workflow_file="${2:-}"
  event="${3:-}"
  base="${4:-}"
  head="${5:-}"
  target_label="$target_paths"
  read -r -a watched_target <<< "${target_paths//,/ }"
elif [[ "${1:-}" == --pathspec=* ]] || [[ "${1:-}" == --paths=* ]] || [[ "${1:-}" == --pathspecs=* ]]; then
  is_pathspec=true
  target_paths="${1#*=}"
  workflow_file="${2:-}"
  event="${3:-}"
  base="${4:-}"
  head="${5:-}"
  target_label="$target_paths"
  read -r -a watched_target <<< "${target_paths//,/ }"
else
  dir="${1:-}"
  workflow_file="${2:-}"
  event="${3:-}"
  base="${4:-}"
  head="${5:-}"
  if [ -n "$dir" ]; then
    if [[ "$dir" == *" "* ]] || [[ "$dir" == *","* ]]; then
      target_label="$dir"
      read -r -a watched_target <<< "${dir//,/ }"
    else
      dir_norm="${dir%/}"
      target_label="$dir_norm/"
      watched_target=("$dir_norm/")
    fi
  else
    target_label=""
    watched_target=()
  fi
fi

if [ -z "$target_label" ] || [ -z "$workflow_file" ] || [ -z "$event" ]; then
  echo "Usage: $0 <dir|--pathspec \"paths...\"> <workflow-file> <event> [base] [head]" >&2
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

# For pull_request events, compute the range against the current base branch
# (e.g. origin/<base_ref> or <base_ref>), rather than relying on the event's
# potentially stale base.sha (#1686).
if [ "$event" = "pull_request" ]; then
  base_ref="${PR_BASE_REF:-${GITHUB_BASE_REF:-main}}"
  for candidate in "origin/$base_ref" "$base_ref"; do
    if git rev-parse --verify "$candidate" >/dev/null 2>&1; then
      if mb=$(git merge-base "$candidate" "$head" 2>/dev/null) && [ -n "$mb" ]; then
        range="$mb...$head"
        break
      fi
    fi
  done
fi

# 1. Decide "build" by exit code of git diff with pathspecs:
# Exit 1 => watched path changed => true
# Any exit other than 0 or 1 => git diff error => true
set +e
git diff --quiet --no-renames "$range" -- "${watched_target[@]}" "$workflow_file" .github/scripts/
diff_status=$?
set -e

if [ "$diff_status" -eq 1 ]; then
  echo "Changes detected under watched paths ($target_label, $workflow_file, or .github/scripts/) — running build." >&2
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
    echo "No changes under $target_label or shared CI files — skipping build." >&2
    echo "false"
  fi
elif [ "$event" = "push" ]; then
  echo "Push does not touch $target_label or shared CI file — skipping build." >&2
  echo "false"
fi
