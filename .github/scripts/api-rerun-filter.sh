#!/usr/bin/env bash
# Compute cargo-nextest arguments for a CI test run or re-run (#3079, #3112).
#
# Mode 1, re-run of the same run (attempt > 1):
#   api-rerun-filter.sh <test-args> <run-attempt> [junit-or-failed-tests-file]
# Output (single line on stdout):
#   - When run-attempt > 1 and the artifact file lists failed tests:
#       <test-args> -E 'test(=name1) | test(=name2)'
#   - Otherwise (attempt 1, missing/unreadable/malformed artifact, or 0 failures):
#       <test-args>
#
# Mode 2, a new run starting from the last run of the same branch (#3112):
#   api-rerun-filter.sh --since-last <failed-list-file> <diff-packages>
#   <failed-list-file>: "<binary-id> <test-name>" per line from api-failed-from-log.sh.
#   <diff-packages>: api-test-plan.sh's answer ("--workspace", "-p a -p b ..." or "").
# Output: key=value lines on stdout:
#   mode=fallback            and nothing else: run today's plan.
#   mode=since-last
#   packages=<-p a -p b ...>
#   filter=<filterset>
#   failed=<count>

set -euo pipefail

if [ "${1:-}" = "--since-last" ]; then
  if [ "$#" -lt 2 ]; then
    echo "Usage: $0 --since-last <failed-list-file> [diff-packages]" >&2
    exit 1
  fi
  exec python3 - "$2" "${3:-}" <<'PY'
import re
import sys

failed_file, diff_packages = sys.argv[1], sys.argv[2].strip()

def fallback(why):
    sys.stderr.write(f"since-last: fallback to today's plan: {why}\n")
    print("mode=fallback")
    sys.exit(0)

if diff_packages == "--workspace":
    fallback("the diff since the last run touches the whole workspace")

safe = re.compile(r"^[A-Za-z0-9_:/.\-]+$")

diff = []
if diff_packages:
    tokens = diff_packages.split()
    if len(tokens) % 2 or any(
        t != "-p" or not safe.match(p) for t, p in zip(tokens[0::2], tokens[1::2])
    ):
        fallback(f"unreadable diff packages {diff_packages!r}")
    diff = tokens[1::2]

try:
    with open(failed_file, "r", encoding="utf-8") as f:
        lines = f.read().splitlines()
except Exception as e:
    fallback(f"failed list unreadable: {failed_file}: {e}")

failed = []
seen = set()
for line in lines:
    line = line.strip()
    if not line:
        continue
    parts = line.split()
    if len(parts) != 2:
        fallback(f"malformed failed-test line: {line!r}")
    binary, name = parts
    if not safe.match(binary) or not safe.match(name):
        fallback(f"failed test name needs quoting: {binary} {name}")
    if (binary, name) not in seen:
        seen.add((binary, name))
        failed.append((binary, name))

packages = set(diff)
terms = []
for binary, name in failed:
    packages.add(binary.split("::", 1)[0])
    terms.append(f"(binary_id(={binary}) & test(={name}))")
terms += [f"package(={p})" for p in sorted(diff)]

print("mode=since-last")
print("packages=" + " ".join(f"-p {p}" for p in sorted(packages)))
print("filter=" + (" | ".join(terms) if failed else ""))
print(f"failed={len(failed)}")
PY
fi

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

def add_failure(binary, name):
    if not name:
        return
    term = f"(binary_id(={binary}) & test(={name}))" if binary else f"test(={name})"
    if term not in seen:
        seen.add(term)
        failed.append(term)

if content.startswith("<"):
    try:
        root = ET.fromstring(content)
        for tc in root.iter("testcase"):
            if tc.find("failure") is not None or tc.find("error") is not None:
                add_failure(tc.get("classname", "").strip(), tc.get("name", "").strip())
    except Exception:
        print(test_args)
        sys.exit(0)
else:
    for line in content.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            parts = line.split()
            if len(parts) >= 2:
                add_failure(parts[0], parts[1])
            else:
                add_failure("", parts[0])

if not failed:
    print(test_args)
else:
    expr = " | ".join(failed)
    print(f"{test_args} -E '{expr}'")
PY
