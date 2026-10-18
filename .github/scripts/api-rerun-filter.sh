#!/usr/bin/env bash
# Compute cargo-nextest arguments for a CI test run or re-run.
# Contract:
#   api-rerun-filter.sh <test-args> <run-attempt> [junit-or-failed-tests-file]
# Output (single line on stdout):
#   - When run-attempt > 1 and the artifact file lists failed tests:
#       <test-args> -E 'test(=name1) | test(=name2)'
#   - Otherwise (attempt 1, missing/unreadable/malformed artifact, or 0 failures):
#       <test-args>

set -euo pipefail

TEST_ARGS="${1:-}"
RUN_ATTEMPT="${2:-1}"
ARTIFACT_FILE="${3:-}"

if [ -z "$TEST_ARGS" ]; then
  echo ""
  exit 0
fi

if ! [[ "$RUN_ATTEMPT" =~ ^[0-9]+$ ]] || [ "$RUN_ATTEMPT" -le 1 ]; then
  echo "$TEST_ARGS"
  exit 0
fi

if [ -z "$ARTIFACT_FILE" ] || [ ! -r "$ARTIFACT_FILE" ]; then
  echo "$TEST_ARGS"
  exit 0
fi

exec python3 - "$TEST_ARGS" "$ARTIFACT_FILE" <<'PY'
import sys
import xml.etree.ElementTree as ET

test_args = sys.argv[1]
artifact_file = sys.argv[2]

try:
    with open(artifact_file, "r", encoding="utf-8") as f:
        content = f.read().strip()
except Exception:
    print(test_args)
    sys.exit(0)

if not content:
    print(test_args)
    sys.exit(0)

failed = []
seen = set()

def add_name(name):
    if name and name not in seen:
        seen.add(name)
        failed.append(name)

if content.startswith("<"):
    try:
        root = ET.fromstring(content)
        for tc in root.iter("testcase"):
            if tc.find("failure") is not None or tc.find("error") is not None:
                add_name(tc.get("name", "").strip())
    except Exception:
        print(test_args)
        sys.exit(0)
else:
    for line in content.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            add_name(line)

if not failed:
    print(test_args)
else:
    expr = " | ".join(f"test(={name})" for name in failed)
    print(f"{test_args} -E '{expr}'")
PY
