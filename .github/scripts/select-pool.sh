#!/usr/bin/env bash
# Select runner pool and per-pool cargo job count for the api.yml `test` job (#2261).
# Reads RUN_ID and RUN_ATTEMPT from the environment and prints:
#   runs_on=<JSON array>
#   cargo_jobs=<int>
set -euo pipefail

if [ -z "${RUN_ID:-}" ]; then
  echo 'runs_on=["self-hosted","Linux","X64","dell-org-ci"]'
  echo 'cargo_jobs=12'
  exit 0
fi

attempt="${RUN_ATTEMPT:-0}"
slot=$(( (RUN_ID + attempt) % 12 ))

if [ "$slot" -lt 6 ]; then
  echo 'runs_on=["self-hosted","Linux","X64","dell-org-ci"]'
  echo 'cargo_jobs=12'
elif [ "$slot" -lt 10 ]; then
  echo 'runs_on=["self-hosted","macOS","ARM64","mac-studio"]'
  echo 'cargo_jobs=4'
else
  echo 'runs_on=["self-hosted","macOS","ARM64","warut-mbp"]'
  echo 'cargo_jobs=6'
fi
