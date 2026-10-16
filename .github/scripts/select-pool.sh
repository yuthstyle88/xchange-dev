#!/usr/bin/env bash
# Select runner pool and per-pool cargo job count for the api.yml `test` job (#2261, #2574).
# Dell 8 / heavy Macs 4 of every 12 slots. `heavy-build` is on exactly the runners allowed to take a heavy build
# (1 on Mac Studio, 1 on the MacBook: owner, one heavy build at a time per Mac), so GitHub never runs more heavy builds on a machine than it has labelled
# runners. cargo_jobs here is a ceiling; cargo-jobs.sh lowers it to the landing machine's free cores (#2574).
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

if [ "$slot" -lt 8 ]; then
  echo 'runs_on=["self-hosted","Linux","X64","dell-org-ci"]'
  echo 'cargo_jobs=12'
else
  echo 'runs_on=["self-hosted","macOS","ARM64","heavy-build"]'
  echo 'cargo_jobs=8'
fi
