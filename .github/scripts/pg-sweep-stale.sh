#!/usr/bin/env bash
# Sweep stale throwaway Postgres data dirs and containers left behind by a killed CI job on this runner (#2386).
# One runner runs one job at a time, so any /tmp/xcpg.* marked with THIS runner's name or any container labelled
# xc.runner=$RUNNER_NAME when a new job starts belongs to a dead job. Foreign or unmarked dirs are never touched.
set -euo pipefail

if [ -z "${RUNNER_NAME:-}" ]; then
  echo "RUNNER_NAME is required and must be non-empty" >&2
  exit 1
fi

XC_PG_TMP="${XC_PG_TMP:-/tmp}"

for dir in "$XC_PG_TMP"/xcpg.*; do
  [ -d "$dir" ] || continue
  [ -f "$dir/runner" ] || continue
  owner=""
  IFS= read -r owner < "$dir/runner" || true
  [ "$owner" = "$RUNNER_NAME" ] || continue

  pidfile="$dir/data/postmaster.pid"
  if [ -f "$pidfile" ]; then
    pid=""
    IFS= read -r pid < "$pidfile" || true
    case "$pid" in
      ''|*[!0-9]*) pid="" ;;
    esac
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      cmdline=$(ps -o args= -p "$pid" 2>/dev/null || true)
      case "$cmdline" in
        *"$dir/data"*)
          if [ -n "${PGBIN:-}" ] && [ -x "$PGBIN/pg_ctl" ]; then
            "$PGBIN/pg_ctl" -D "$dir/data" -m immediate stop >/dev/null 2>&1 || true
          fi
          if kill -0 "$pid" 2>/dev/null; then
            kill -TERM "$pid" 2>/dev/null || true
            sleep 0.2
          fi
          if kill -0 "$pid" 2>/dev/null; then
            kill -KILL "$pid" 2>/dev/null || true
          fi
          ;;
      esac
    fi
  fi

  rm -rf "$dir"
  echo "swept $dir"
done

if command -v docker >/dev/null 2>&1; then
  ids=$(docker ps -q --filter "label=xc.runner=$RUNNER_NAME" 2>/dev/null || true)
  if [ -n "$ids" ]; then
    # shellcheck disable=SC2086
    docker rm -f $ids >/dev/null 2>&1 || true
  fi
fi

exit 0
