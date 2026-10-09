#!/usr/bin/env bash
# Finds the base commit on a push to main for incremental CI builds (#737).
# On a push to main, returns the head sha of the newest successful push run of
# the specified workflow on main that is an ancestor of the current head.
# Prints the sha on stdout or prints nothing; exits 0 in both cases.
# Exits non-zero on malformed input.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 <workflow-name> <head-sha>" >&2
  exit 1
fi

wf="$1"
head="$2"

if [ -z "$wf" ] || [ -z "$head" ]; then
  echo "Error: workflow-name and head-sha must not be empty" >&2
  exit 1
fi

if [ -n "${MAIN_PUSH_RUNS_JSON:-}" ]; then
  if [ ! -f "$MAIN_PUSH_RUNS_JSON" ]; then
    echo "Error: file not found: $MAIN_PUSH_RUNS_JSON" >&2
    exit 1
  fi
  runs_stream=$(cat "$MAIN_PUSH_RUNS_JSON")
else
  repo="${GITHUB_REPOSITORY:-}"
  if [ -z "$repo" ]; then
    echo "Error: GITHUB_REPOSITORY is not set" >&2
    exit 1
  fi
  runs_stream=$(gh api "repos/${repo}/actions/runs?branch=main&event=push&per_page=50")
fi

candidate_shas=$(echo "$runs_stream" | jq -r --arg wf "$wf" '
  .workflow_runs
  | map(select(
      .name == $wf and
      .head_branch == "main" and
      .event == "push" and
      .conclusion == "success"
    ))
  | sort_by(.created_at)
  | reverse
  | .[].head_sha
')

for sha in $candidate_shas; do
  if [ -n "$sha" ] && [ "$sha" != "$head" ] && git merge-base --is-ancestor "$sha" "$head" 2>/dev/null; then
    echo "$sha"
    exit 0
  fi
done

exit 0
