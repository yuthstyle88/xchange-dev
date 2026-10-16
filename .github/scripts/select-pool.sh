#!/usr/bin/env bash
# Select the runner pool and the cargo job ceiling for the api.yml `test` job (#2261, #2574).
# One pool: the runners labelled heavy-build. How many a machine carries is the cap on heavy builds it runs at once
# (owner 2026-10-16: Dell 2, Mac Studio 2, Warut 1, MacBook 1; Mac mini none). GitHub hands each build to whichever of
# them is idle, so a labelled runner never sits idle while a build waits. cargo_jobs is a ceiling; cargo-jobs.sh
# lowers it to the landing machine's free cores and memory.
# Reads RUN_ID and RUN_ATTEMPT from the environment (kept for the interface) and prints:
#   runs_on=<JSON array>
#   cargo_jobs=<int>
set -euo pipefail

if [ -z "${RUN_ID:-}" ]; then
  echo 'runs_on=["self-hosted","Linux","X64","dell-org-ci"]'
  echo 'cargo_jobs=12'
  exit 0
fi

echo 'runs_on=["self-hosted","heavy-build"]'
echo 'cargo_jobs=12'
