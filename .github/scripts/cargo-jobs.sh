#!/usr/bin/env bash
# Print the cargo job count for the api `test` job on the machine it landed on (#2574, #4221).
# jobs = max(2, min(cap, free cores, bymem)), where:
#   - free cores = floor(cores - load1)
#   - bymem = floor(cgroup_bytes / 7 GiB) when a numeric cgroup v2 limit applies,
#             else floor(physical_GiB * 0.6 / 7)
# Usage: cargo-jobs.sh <cap>   (cap = the pool's ceiling from select-pool.sh; empty means 12)
# Test hooks: CJ_CORES, CJ_LOAD, CJ_CGROUP_MEM, CJ_PROC_SELF_CGROUP, CJ_SYS_FS_CGROUP, CJ_MEM_GB.
set -euo pipefail

cap="${1:-}"; [ -n "$cap" ] || cap=12

if [ -n "${CJ_CORES:-}" ]; then cores="$CJ_CORES"
elif command -v nproc >/dev/null 2>&1; then cores=$(nproc)
else cores=$(sysctl -n hw.ncpu); fi

if [ -n "${CJ_LOAD:-}" ]; then load="$CJ_LOAD"
elif [ -r /proc/loadavg ]; then load=$(cut -d' ' -f1 /proc/loadavg)
else load=$(sysctl -n vm.loadavg | awk '{print $2}'); fi

cg_bytes=""
if [ -n "${CJ_CGROUP_MEM:-}" ]; then
  case "$CJ_CGROUP_MEM" in
    *[!0-9]*) ;;
    *) cg_bytes="$CJ_CGROUP_MEM" ;;
  esac
else
  proc_cg="${CJ_PROC_SELF_CGROUP:-/proc/self/cgroup}"
  sys_cg="${CJ_SYS_FS_CGROUP:-/sys/fs/cgroup}"
  if [ -r "$proc_cg" ] && [ -d "$sys_cg" ]; then
    cg_rel=$(sed -n 's/^0:://p' "$proc_cg" | head -n 1)
    if [ -n "$cg_rel" ]; then
      rel="${cg_rel#/}"
      while :; do
        if [ -n "$rel" ]; then dir="$sys_cg/$rel"; else dir="$sys_cg"; fi
        if [ -r "$dir/memory.max" ]; then
          read -r val < "$dir/memory.max" || val=""
          case "$val" in
            ''|*[!0-9]*) ;;
            *) if [ -z "$cg_bytes" ] || [ "$val" -lt "$cg_bytes" ]; then cg_bytes="$val"; fi ;;
          esac
        fi
        [ -n "$rel" ] || break
        case "$rel" in
          */*) rel="${rel%/*}" ;;
          *) rel="" ;;
        esac
      done
    fi
  fi
fi

mem=""
if [ -z "$cg_bytes" ]; then
  if [ -n "${CJ_MEM_GB:-}" ]; then mem="$CJ_MEM_GB"
  elif [ -r /proc/meminfo ]; then mem=$(awk '/^MemTotal:/{print int($2/1048576)}' /proc/meminfo)
  elif raw=$(sysctl -n hw.memsize 2>/dev/null) && [ -n "$raw" ]; then mem=$(( raw / 1073741824 ))
  fi
fi

awk -v cap="$cap" -v c="$cores" -v l="$load" -v cg="$cg_bytes" -v m="$mem" 'BEGIN {
  free = int(c - l); j = cap
  if (free < j) j = free
  if (cg != "") {
    bymem = int(cg / (7 * 1073741824))
    if (bymem < j) j = bymem
    if (j < 2) j = 2
    printf "cargo-jobs: memory cgroup %.1f GiB -> %d\n", cg / 1073741824, j > "/dev/stderr"
  } else if (m != "") {
    bymem = int(m * 0.6 / 7)
    if (bymem < j) j = bymem
    if (j < 2) j = 2
    printf "cargo-jobs: memory physical %d GiB (60%%) -> %d\n", m, j > "/dev/stderr"
  } else {
    if (j < 2) j = 2
    printf "cargo-jobs: no memory source readable -> %d\n", j > "/dev/stderr"
  }
  print j
}'
