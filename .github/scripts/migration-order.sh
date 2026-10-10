#!/usr/bin/env bash
# Checks that all migrations added in a PR have versions strictly greater than
# the highest migration version on the base branch (#847).
#
# Usage: migration-order.sh <base-tip> <head>
# Exit: 0 if valid or no migrations added, 1 if any migration version is <= base max
#       or malformed.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "Usage: $0 <base-tip> <head>" >&2
  exit 1
fi

base_tip="$1"
head="$2"

if [ -z "$base_tip" ] || [ -z "$head" ]; then
  echo "Error: base-tip and head must not be empty" >&2
  exit 1
fi

merge_base=$(git merge-base "$base_tip" "$head" 2>/dev/null || true)
if [ -z "$merge_base" ]; then
  echo "Error: no common ancestor between $base_tip and $head" >&2
  exit 1
fi

added_files=$(git diff --name-only --diff-filter=AR "$merge_base" "$head" -- 'api/migrations/*.sql')

if [ -z "$added_files" ]; then
  echo "migration order: no new migrations"
  exit 0
fi

base_max=$(git ls-tree --name-only "$base_tip" api/migrations/ 2>/dev/null \
  | grep -E '\.sql$' \
  | sed -E 's#^.*/##' \
  | grep -E '^[0-9]{14}_' \
  | cut -c 1-14 \
  | sort -n \
  | tail -n 1 || true)
base_max="${base_max:-0}"

has_error=0

while IFS= read -r file; do
  [ -z "$file" ] && continue
  fname=$(basename "$file")
  if [[ "$fname" =~ ^([0-9]{14})_.*\.sql$ ]]; then
    v="${BASH_REMATCH[1]}"
    if [[ "$v" < "$base_max" || "$v" == "$base_max" ]]; then
      echo "migration order: $file ($v) is not after main's highest $base_max; rename it to a version after $base_max"
      has_error=1
    fi
  else
    echo "migration order: $file has no 14-digit version"
    has_error=1
  fi
done <<< "$added_files"

if [ "$has_error" -ne 0 ]; then
  pr="${PR_NUMBER:-<pr-number>}"
  echo "scripts/migration-slot.sh renumber $pr"
  exit 1
fi

echo "migration order: ok (base max $base_max)"
exit 0
