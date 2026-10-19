#!/usr/bin/env bash
# Select the runner pool and the cargo job ceiling for the api.yml `test` job (#2261, #2574, #3072).
# Placement pins removed per runner-switch rule (#3072). The test job runs on Linux and macOS alike (owner
# 2026-10-18: a test job that runs on both OSes is not pinned to Linux; owner 2026-10-20: open the Macs to take it).
# The pool is the `heavy-build` capability label (owner 2026-10-20: on the Studio and every Dell runner), so the job
# never lands on the Mac mini, jam, Warut's MacBook or Windows; within the pool, runners.sh decides what is switched on.
# Reads RUN_ID and RUN_ATTEMPT from the environment (kept for the interface) and prints:
#   runs_on=<JSON array>
#   cargo_jobs=<int>
set -euo pipefail

if [ -z "${RUN_ID:-}" ]; then
  echo 'runs_on=["self-hosted","heavy-build"]'
  echo 'cargo_jobs=12'
  exit 0
fi

echo 'runs_on=["self-hosted","heavy-build"]'
echo 'cargo_jobs=12'
