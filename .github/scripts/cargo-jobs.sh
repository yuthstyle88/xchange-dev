#!/usr/bin/env bash
# Print the cargo job count for the api `test` job on the machine it landed on (#2574).
# jobs = min(cap, free cores, memory/4 GB), at least 2. "Free cores" is cores minus the 1-minute load
# average, rounded down, so a busy or small runner host is not oversubscribed by one more heavy build.
# Usage: cargo-jobs.sh <cap>   (cap = the pool's ceiling from select-pool.sh; empty means 12)
# Test hooks: CJ_CORES, CJ_LOAD, CJ_MEM_GB replace the measured values.
set -euo pipefail

cap="${1:-}"; [ -n "$cap" ] || cap=12

if [ -n "${CJ_CORES:-}" ]; then cores="$CJ_CORES"
elif command -v nproc >/dev/null 2>&1; then cores=$(nproc)
else cores=$(sysctl -n hw.ncpu); fi

if [ -n "${CJ_LOAD:-}" ]; then load="$CJ_LOAD"
elif [ -r /proc/loadavg ]; then load=$(cut -d' ' -f1 /proc/loadavg)
else load=$(sysctl -n vm.loadavg | awk '{print $2}'); fi

if [ -n "${CJ_MEM_GB:-}" ]; then mem="$CJ_MEM_GB"
elif [ -r /proc/meminfo ]; then mem=$(awk '/^MemTotal:/{print int($2/1048576)}' /proc/meminfo)
else mem=$(( $(sysctl -n hw.memsize) / 1073741824 )); fi

awk -v cap="$cap" -v c="$cores" -v l="$load" -v m="$mem" 'BEGIN {
  free = int(c - l); bymem = int(m / 4); j = cap
  if (free < j) j = free
  if (bymem < j) j = bymem
  if (j < 2) j = 2
  print j
}'
