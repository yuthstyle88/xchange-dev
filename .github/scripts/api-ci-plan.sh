#!/usr/bin/env bash
# Plan CI scope for api.yml based on event, head ref, changed paths, and cargo metadata.
# Contract:
#   api-ci-plan.sh <event> <head-ref> <changed-paths-file> <cargo-metadata-json>
# Output: five key=value lines on stdout:
#   packages=<--workspace | -p a -p b ... | empty>
#   run_tests=<true|false>
#   guards=<true|false>
#   device_agent=<true|false>
#   release_check=<true|false>

set -euo pipefail

if [ "$#" -lt 4 ]; then
  echo "Usage: $0 <event> <head-ref> <changed-paths-file> <cargo-metadata-json>" >&2
  exit 1
fi

EVENT="$1"
HEAD_REF="$2"
CHANGED_FILE="$3"
METADATA_FILE="$4"
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

print_all() {
  echo "packages=--workspace"
  echo "run_tests=true"
  echo "guards=true"
  echo "device_agent=true"
  echo "release_check=true"
  exit 0
}

# Batch PRs and events other than pull_request / push plan everything (unless workflow_dispatch names packages).
if [ "$EVENT" = "pull_request" ] && [[ "$HEAD_REF" == batch/merge-* ]]; then
  print_all
fi
if [ "$EVENT" = "workflow_dispatch" ] && [ -n "${DISPATCH_PACKAGES:-}" ] && [[ "${DISPATCH_PACKAGES}" =~ [^[:space:]] ]]; then
  exec python3 - "$METADATA_FILE" "$DISPATCH_PACKAGES" <<'PY'
import json
import sys

metadata_file, raw = sys.argv[1], sys.argv[2]
with open(metadata_file, "r", encoding="utf-8") as f:
    known = {p["name"] for p in json.load(f).get("packages", []) if "name" in p}

requested = sorted(set(raw.split()))
unknown = [p for p in requested if p not in known]
if unknown:
    sys.stderr.write(f"Error: unknown package(s) in DISPATCH_PACKAGES: {', '.join(unknown)}\n")
    sys.exit(1)

pkgs = " ".join(f"-p {p}" for p in requested)
dev = "true" if "xc-device-agent" in requested else "false"
print(f"packages={pkgs}")
print("run_tests=true")
print("guards=false")
print(f"device_agent={dev}")
print("release_check=true")
PY
fi
if [ "$EVENT" != "pull_request" ] && [ "$EVENT" != "push" ]; then
  print_all
fi

# Missing or unreadable inputs => unknown change => plan everything (or PR_ALL on non-batch PR).
if [ ! -r "$CHANGED_FILE" ] || [ ! -r "$METADATA_FILE" ]; then
  packages="--workspace"
else
  PLAN_CHANGED="$CHANGED_FILE"
  if [ "$EVENT" = "push" ]; then
    PLAN_CHANGED=$(mktemp)
    trap 'rm -f "$PLAN_CHANGED"' EXIT
    grep -vE '^(\./)?\.github(/|$)' "$CHANGED_FILE" > "$PLAN_CHANGED" || true
  fi
  if ! packages=$("$SCRIPT_DIR/api-test-plan.sh" "$PLAN_CHANGED" "$METADATA_FILE"); then
    packages="--workspace"
  fi
fi

has_pkg() {
  local needle="$1"
  [ "$packages" = "--workspace" ] || [[ " $packages " == *" -p $needle "* ]]
}

if has_pkg "xc-device-agent"; then
  device_agent="true"
else
  device_agent="false"
fi

if [ "$EVENT" = "pull_request" ]; then
  run_tests="false"
  guards="false"
  if has_pkg "xchange"; then
    release_check="true"
  else
    release_check="false"
  fi
else
  # push to main
  if [ -n "$packages" ]; then
    run_tests="true"
  else
    run_tests="false"
  fi
  guards="true"
  release_check="true"
fi

echo "packages=$packages"
echo "run_tests=$run_tests"
echo "guards=$guards"
echo "device_agent=$device_agent"
echo "release_check=$release_check"
