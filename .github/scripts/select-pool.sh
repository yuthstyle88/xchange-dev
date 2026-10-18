#!/usr/bin/env bash
# Select the runner pool and the cargo job ceiling for the api.yml `test` job (#2261, #2574, #3072).
# Placement pins removed per runner-switch rule (#3072): real build/test jobs stay on Linux.
# Reads RUN_ID and RUN_ATTEMPT from the environment (kept for the interface) and prints:
#   runs_on=<JSON array>
#   cargo_jobs=<int>
set -euo pipefail

if [ -z "${RUN_ID:-}" ]; then
  echo 'runs_on=["self-hosted","Linux","X64"]'
  echo 'cargo_jobs=12'
  exit 0
fi

echo 'runs_on=["self-hosted","Linux","X64"]'
echo 'cargo_jobs=12'
