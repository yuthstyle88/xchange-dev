#!/usr/bin/env bash
# Creates a marked per-run Postgres root under ${XC_PG_BASE:-/tmp} after reaping
# leftover marked roots from cancelled or dead earlier jobs (#2798).
set -euo pipefail

base="${XC_PG_BASE:-/tmp}"
current_runner="${RUNNER_NAME:-}"

pid_alive() {
  ps -p "$1" >/dev/null 2>&1
}

postmaster_of() {
  local pidfile="$1/data/postmaster.pid"
  if [ -f "$pidfile" ]; then
    head -n 1 "$pidfile" 2>/dev/null || true
  fi
}

is_postmaster_for() {
  local pid="$1" dir="$2" cmd
  case "$pid" in
    '' | *[!0-9]*) return 1 ;;
  esac
  pid_alive "$pid" || return 1
  cmd=$(ps -o command= -p "$pid" 2>/dev/null || true)
  case "$cmd" in
    *"-D $dir/data"*) return 0 ;;
    *) return 1 ;;
  esac
}

reap_one() {
  local dir="$1"
  local marker="$dir/.xc-ci-owner"
  [ -d "$dir" ] || return 0
  [ -f "$marker" ] || return 0

  local dir_runner="" dir_run="" k v
  while IFS='=' read -r k v; do
    case "$k" in
      RUNNER_NAME) dir_runner="$v" ;;
      GITHUB_RUN_ID) dir_run="$v" ;;
    esac
  done <"$marker"

  local same_runner=0
  if [ -n "$current_runner" ] && [ "$dir_runner" = "$current_runner" ]; then
    same_runner=1
  fi

  local pid
  pid=$(postmaster_of "$dir")

  if [ -z "$pid" ]; then
    if [ "$same_runner" -eq 1 ]; then
      if rm -rf "$dir" 2>/dev/null; then
        echo "ci-postgres-root: reaped $dir (runner=$dir_runner run=$dir_run pid=none alive=no)" >&2
      else
        echo "ci-postgres-root: warning: failed to remove $dir" >&2
      fi
    fi
    return 0
  fi

  if is_postmaster_for "$pid" "$dir"; then
    if [ "$same_runner" -eq 0 ]; then
      return 0
    fi
    kill -QUIT "$pid" 2>/dev/null || true
    local i=0
    while [ "$i" -lt 100 ]; do
      if ! pid_alive "$pid"; then
        break
      fi
      sleep 0.1
      i=$((i + 1))
    done
    if pid_alive "$pid"; then
      echo "ci-postgres-root: warning: postmaster $pid in $dir ignored SIGQUIT after 10s; keeping dir" >&2
      return 0
    fi
    if rm -rf "$dir" 2>/dev/null; then
      echo "ci-postgres-root: reaped $dir (runner=$dir_runner run=$dir_run pid=$pid alive=yes)" >&2
    else
      echo "ci-postgres-root: warning: failed to remove $dir" >&2
    fi
    return 0
  fi

  if rm -rf "$dir" 2>/dev/null; then
    echo "ci-postgres-root: reaped $dir (runner=$dir_runner run=$dir_run pid=$pid alive=no)" >&2
  else
    echo "ci-postgres-root: warning: failed to remove $dir" >&2
  fi
}

shopt -s nullglob
for candidate in "$base"/xcpg.*; do
  reap_one "$candidate" || true
done
shopt -u nullglob

new_dir=$(mktemp -d "$base/xcpg.XXXXXX")
printf 'RUNNER_NAME=%s\nGITHUB_RUN_ID=%s\nGITHUB_RUN_ATTEMPT=%s\nGITHUB_JOB=%s\nCREATED_AT=%s\n' \
  "$current_runner" \
  "${GITHUB_RUN_ID:-}" \
  "${GITHUB_RUN_ATTEMPT:-}" \
  "${GITHUB_JOB:-}" \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$new_dir/.xc-ci-owner"

printf '%s\n' "$new_dir"
