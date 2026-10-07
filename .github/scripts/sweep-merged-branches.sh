#!/usr/bin/env bash
# Lists every branch of the remote with DELETE or KEEP and why; with --apply deletes
# the DELETE ones. Issue #162: test-first branches are never a PR head, so GitHub's
# delete-on-merge never sees them, and a squash merge leaves their commit off `main`.
#
# A branch is DELETE only when one proof holds:
#   ancestor         its tip is an ancestor of main
#   pull-ref         its tip is an ancestor of some refs/pull/*/head (GitHub keeps those)
#   content-in-main  for every file it changed, main has the same blob, or main holds
#                    every line it added and none of the lines it removed (lines compared
#                    as a multiset, whitespace at the ends ignored; binary files: blob only)
# and none of these holds: protected name, head of an open PR, younger than the minimum age.
#
# Usage: sweep-merged-branches.sh [--apply]
# Env:   SWEEP_REMOTE (origin) · SWEEP_KEEP_FILE (branch names to keep, one per line)
#        SWEEP_MIN_AGE_HOURS (6)
# Out:   one line per branch: DELETE|KEEP <tab> name <tab> sha <tab> proof or reason
#        with --apply, a "restore:" line per deleted branch.
set -euo pipefail
export LC_ALL=C

PULL_NS="refs/remotes/sweep-pull"

is_protected() { # name
  case "$1" in
    main | HEAD | release/* | deploy/* | backup/* | archive/* | auto/*) return 0 ;;
    *) return 1 ;;
  esac
}

is_open_pr_head() { # name keep-file
  [ -n "$2" ] && [ -f "$2" ] && grep -Fxq -- "$1" "$2"
}

is_young() { # sha min-age-hours
  local committed now
  committed=$(git log -1 --format=%ct "$1")
  now=$(date +%s)
  [ $((now - committed)) -lt $(($2 * 3600)) ]
}

tip_in_main() { # sha main-ref
  git merge-base --is-ancestor "$1" "$2"
}

tip_in_pull_ref() { # sha
  [ -n "$(git for-each-ref --contains "$1" --count=1 --format='%(refname)' "$PULL_NS/")" ]
}

# Lines of a blob, trimmed, blank ones dropped, sorted (duplicates kept).
lines_of() { # treeish:path
  git show "$1" 2> /dev/null | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' | grep -v '^$' | sort || true
}

content_in_main() { # sha main-ref
  local tip=$1 main=$2 base work f tip_blob main_blob status=0
  base=$(git merge-base "$main" "$tip") || return 1
  work=$(mktemp -d)
  while IFS= read -r -d '' f; do
    tip_blob=$(git rev-parse -q --verify "$tip:$f" 2> /dev/null || true)
    main_blob=$(git rev-parse -q --verify "$main:$f" 2> /dev/null || true)
    [ "$tip_blob" = "$main_blob" ] && continue # same blob, or deleted on both sides
    if [ -z "$tip_blob" ] || [ -z "$main_blob" ]; then
      status=1 # the branch deleted a file main keeps, or added one main lacks
      break
    fi
    if git diff --numstat --no-renames "$base" "$tip" -- "$f" | grep -q '^-'; then
      status=1 # binary and not the same blob
      break
    fi
    lines_of "$base:$f" > "$work/base"
    lines_of "$tip:$f" > "$work/tip"
    lines_of "$main:$f" > "$work/main"
    comm -13 "$work/base" "$work/tip" > "$work/added"   # tip minus base
    comm -23 "$work/base" "$work/tip" > "$work/removed" # base minus tip
    comm -13 "$work/tip" "$work/main" > "$work/extra"   # main minus tip
    if [ -n "$(comm -23 "$work/added" "$work/main")" ] ||
      [ -n "$(comm -12 <(sort -u "$work/removed") <(sort -u "$work/extra"))" ]; then
      status=1
      break
    fi
  done < <(git diff --name-only -z --no-renames "$base" "$tip")
  rm -rf "$work"
  return "$status"
}

classify() { # name sha main-ref keep-file min-age → "VERDICT<tab>proof"
  local name=$1 sha=$2 main=$3
  if is_protected "$name"; then
    printf 'KEEP\tprotected\n'
  elif is_open_pr_head "$name" "$4"; then
    printf 'KEEP\topen-pr\n'
  elif is_young "$sha" "$5"; then
    printf 'KEEP\tyoung\n'
  elif tip_in_main "$sha" "$main"; then
    printf 'DELETE\tancestor\n'
  elif tip_in_pull_ref "$sha"; then
    printf 'DELETE\tpull-ref\n'
  elif content_in_main "$sha" "$main"; then
    printf 'DELETE\tcontent-in-main\n'
  else
    printf 'KEEP\tunique-content\n'
  fi
}

# Deletes the branch only if it still points at the sha that was classified.
delete_branch() { # remote name sha
  git push --quiet --force-with-lease="refs/heads/$2:$3" "$1" ":refs/heads/$2"
}

main() {
  local apply=false remote keep_file min_age main_ref ref name sha verdict proof
  if [ "${1:-}" = "--apply" ]; then
    apply=true
  elif [ -n "${1:-}" ]; then
    echo "usage: $0 [--apply]" >&2
    return 2
  fi
  remote=${SWEEP_REMOTE:-origin}
  keep_file=${SWEEP_KEEP_FILE:-}
  min_age=${SWEEP_MIN_AGE_HOURS:-6}
  main_ref="refs/remotes/$remote/main"

  git fetch --quiet --prune "$remote" \
    "+refs/heads/*:refs/remotes/$remote/*" "+refs/pull/*/head:$PULL_NS/*"

  git for-each-ref --format='%(refname) %(objectname)' "refs/remotes/$remote/" |
    while read -r ref sha; do
      name=${ref#"refs/remotes/$remote/"}
      [ "$name" = "HEAD" ] && continue
      [ "$name" = "main" ] && continue
      IFS=$'\t' read -r verdict proof < <(classify "$name" "$sha" "$main_ref" "$keep_file" "$min_age")
      printf '%s\t%s\t%s\t%s\n' "$verdict" "$name" "$sha" "$proof"
      if [ "$apply" = true ] && [ "$verdict" = DELETE ]; then
        if delete_branch "$remote" "$name" "$sha"; then
          echo "restore: git push $remote $sha:refs/heads/$name"
        else
          echo "not deleted: $name moved since it was read" >&2
        fi
      fi
    done
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
