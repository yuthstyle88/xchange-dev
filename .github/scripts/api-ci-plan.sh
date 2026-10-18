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

# Batch PRs and events other than pull_request / push plan everything.
if [ "$EVENT" = "pull_request" ] && [[ "$HEAD_REF" == batch/merge-* ]]; then
  print_all
fi
if [ "$EVENT" != "pull_request" ] && [ "$EVENT" != "push" ]; then
  print_all
fi

# Missing or unreadable inputs => unknown change => plan everything (or PR_ALL on non-batch PR).
if [ ! -r "$CHANGED_FILE" ] || [ ! -r "$METADATA_FILE" ]; then
  packages="--workspace"
else
  if ! packages=$("$SCRIPT_DIR/api-test-plan.sh" "$CHANGED_FILE" "$METADATA_FILE"); then
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
