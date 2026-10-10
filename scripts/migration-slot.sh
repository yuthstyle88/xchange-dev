#!/usr/bin/env bash
# Central migration-version allocator (#1082).
#
#   scripts/migration-slot.sh next [--name <snake>]
#       Print the next free migration version: one hour after the highest
#       version on origin/main's api/migrations/ and in every OPEN PR's files.
#
#   scripts/migration-slot.sh renumber <pr-number|branch>
#       Run on the PR's branch just before merge. Every migration the branch
#       adds whose version is <= origin/main's highest is git-mv'd to the next
#       free version (SQL bytes unchanged) and committed as
#       "chore(migrations): renumber <old> → <new> at merge time".
#       Already above main: no-op, exit 0.
set -euo pipefail

usage() {
  echo "Usage: $0 next [--name <snake>] | renumber <pr-number|branch>" >&2
}

is_leap_year() {
  local y=$1
  if (( (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0) )); then
    return 0
  else
    return 1
  fi
}

days_in_month() {
  local y=$1 m=$2
  case "$m" in
    1|3|5|7|8|10|12) echo 31 ;;
    4|6|9|11) echo 30 ;;
    2)
      if is_leap_year "$y"; then
        echo 29
      else
        echo 28
      fi
      ;;
    *) echo 31 ;;
  esac
}

add_one_hour() {
  local v="$1"
  local y m d h rest
  y=$(( 10#${v:0:4} ))
  m=$(( 10#${v:4:2} ))
  d=$(( 10#${v:6:2} ))
  h=$(( 10#${v:8:2} ))
  rest="${v:10:4}"

  h=$(( h + 1 ))
  if [ "$h" -ge 24 ]; then
    h=0
    d=$(( d + 1 ))
    local max_days
    max_days=$(days_in_month "$y" "$m")
    if [ "$d" -gt "$max_days" ]; then
      d=1
      m=$(( m + 1 ))
      if [ "$m" -gt 12 ]; then
        m=1
        y=$(( y + 1 ))
      fi
    fi
  fi

  printf '%04d%02d%02d%02d%s\n' "$y" "$m" "$d" "$h" "$rest"
}

get_base_max() {
  local max
  max=$(git ls-tree --name-only origin/main api/migrations/ 2>/dev/null \
    | grep -E '\.sql$' \
    | sed -E 's#^.*/##' \
    | grep -E '^[0-9]{14}_' \
    | cut -c 1-14 \
    | sort -n \
    | tail -n 1 || true)
  echo "${max:-0}"
}

fetch_open_prs() {
  local pr_json
  if ! pr_json=$(gh pr list --state open --limit 500 --json number,headRefName,files 2>&1); then
    echo "migration-slot: gh pr list failed: $pr_json" >&2
    return 1
  fi

  local truncated_pr
  truncated_pr=$(echo "$pr_json" | jq -r '.[] | select((.files | length) == 100) | .number' | head -n 1)
  if [ -n "$truncated_pr" ]; then
    echo "migration-slot: PR #$truncated_pr file list may be truncated (100 files)" >&2
    return 1
  fi

  echo "$pr_json"
}

cmd_next() {
  local opt_name=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --name)
        if [ "$#" -lt 2 ]; then
          echo "migration-slot: --name requires an argument" >&2
          exit 1
        fi
        opt_name="$2"
        shift 2
        ;;
      --name=*)
        opt_name="${1#--name=}"
        shift
        ;;
      *)
        echo "migration-slot: unrecognized option: $1" >&2
        exit 1
        ;;
    esac
  done

  if [ -n "$opt_name" ]; then
    if [[ ! "$opt_name" =~ ^[a-z0-9_]+$ ]]; then
      echo "migration-slot: invalid name '$opt_name': must match ^[a-z0-9_]+$" >&2
      exit 1
    fi
  fi

  git fetch -q origin main 2>/dev/null || true

  local pr_json
  if ! pr_json=$(fetch_open_prs); then
    exit 1
  fi

  local base_max
  base_max=$(get_base_max)

  local pr_versions
  pr_versions=$(echo "$pr_json" | jq -r \
    '[.[].files[].path | select(startswith("api/migrations/")) | split("/")[-1] | capture("^(?<v>[0-9]{14})_.*\\.sql$")? // empty | .v] | .[]')

  local current_max="$base_max"
  if [ -n "$pr_versions" ]; then
    for v in $pr_versions; do
      if [[ "$v" > "$current_max" ]]; then
        current_max="$v"
      fi
    done
  fi

  local next_version
  next_version=$(add_one_hour "$current_max")

  if [ -n "$opt_name" ]; then
    echo "api/migrations/${next_version}_${opt_name}.sql"
  else
    echo "$next_version"
  fi
}

cmd_renumber() {
  if [ "$#" -lt 1 ]; then
    echo "Usage: $0 renumber <pr-number|branch>" >&2
    exit 1
  fi

  local target="$1"

  if [ -n "$(git status --porcelain)" ]; then
    echo "migration-slot: working tree is dirty" >&2
    exit 1
  fi

  git fetch -q origin main 2>/dev/null || true

  local pr_json
  if ! pr_json=$(fetch_open_prs); then
    exit 1
  fi

  local pr_info
  if [[ "$target" =~ ^[0-9]+$ ]]; then
    pr_info=$(echo "$pr_json" | jq -c --argjson num "$target" '.[] | select(.number == $num)' | head -n 1)
  else
    pr_info=$(echo "$pr_json" | jq -c --arg branch "$target" '.[] | select(.headRefName == $branch)' | head -n 1)
  fi

  if [ -z "$pr_info" ]; then
    echo "migration-slot: PR '$target' not found among open PRs" >&2
    exit 1
  fi

  local pr_number expected_branch
  pr_number=$(echo "$pr_info" | jq -r '.number')
  expected_branch=$(echo "$pr_info" | jq -r '.headRefName')

  local current_branch
  current_branch=$(git symbolic-ref --short HEAD 2>/dev/null || true)
  if [ "$current_branch" != "$expected_branch" ]; then
    echo "migration-slot: current checkout '$current_branch' is not PR branch '$expected_branch'" >&2
    exit 1
  fi

  local merge_base
  merge_base=$(git merge-base origin/main HEAD 2>/dev/null || true)
  if [ -z "$merge_base" ]; then
    echo "migration-slot: no common ancestor with origin/main" >&2
    exit 1
  fi

  local base_max
  base_max=$(get_base_max)

  local added_files
  added_files=$(git diff --name-only --diff-filter=AR "$merge_base" HEAD -- 'api/migrations/*.sql')

  local to_renumber=()
  if [ -n "$added_files" ]; then
    while IFS= read -r f; do
      [ -z "$f" ] && continue
      local fname v
      fname=$(basename "$f")
      if [[ "$fname" =~ ^([0-9]{14})_.*\.sql$ ]]; then
        v="${BASH_REMATCH[1]}"
        if [[ "$v" < "$base_max" || "$v" == "$base_max" ]]; then
          to_renumber+=("$f")
        fi
      fi
    done <<< "$added_files"
  fi

  if [ "${#to_renumber[@]}" -eq 0 ]; then
    echo "migration-slot: nothing to renumber; all branch migrations are above origin/main ($base_max)"
    exit 0
  fi

  local sorted_to_renumber=()
  while IFS= read -r f; do
    [ -n "$f" ] && sorted_to_renumber+=("$f")
  done < <(printf '%s\n' "${to_renumber[@]}" | sort)

  local other_pr_versions
  other_pr_versions=$(echo "$pr_json" | jq -r --argjson pr_num "$pr_number" \
    '[.[] | select(.number != $pr_num) | .files[].path | select(startswith("api/migrations/")) | split("/")[-1] | capture("^(?<v>[0-9]{14})_.*\\.sql$")? // empty | .v] | .[]')

  local current_max="$base_max"
  if [ -n "$other_pr_versions" ]; then
    for v in $other_pr_versions; do
      if [[ "$v" > "$current_max" ]]; then
        current_max="$v"
      fi
    done
  fi

  for old_file in "${sorted_to_renumber[@]}"; do
    local fname old_version name_suffix new_version new_file
    fname=$(basename "$old_file")
    if [[ "$fname" =~ ^([0-9]{14})_(.*)$ ]]; then
      old_version="${BASH_REMATCH[1]}"
      name_suffix="${BASH_REMATCH[2]}"
      new_version=$(add_one_hour "$current_max")
      new_file="api/migrations/${new_version}_${name_suffix}"
      git mv "$old_file" "$new_file"
      git commit -q -m "chore(migrations): renumber $old_version → $new_version at merge time"
      echo "$old_version → $new_version"
      current_max="$new_version"
    fi
  done
}

cmd="${1:-}"
shift || true
case "$cmd" in
  next)
    cmd_next "$@"
    ;;
  renumber)
    cmd_renumber "$@"
    ;;
  *)
    usage
    exit 1
    ;;
esac
