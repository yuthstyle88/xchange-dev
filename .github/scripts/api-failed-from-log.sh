#!/usr/bin/env bash
# Extract failed cargo-nextest tests from a raw GitHub Actions job log on stdin (#3112).
# Output: "<binary-id> <test-name>" per failed test, deduplicated in first-seen order.
set -euo pipefail

exec python3 -c '
import re
import sys

ansi_re = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
ts_re = re.compile(r"^\d{4}-\d{2}-\d{2}T\S+\s+")
status_re = re.compile(
    r"^\s*(FAIL|TIMEOUT|SIG\S+)\s+\[\s*[0-9.]+s\]\s+(?:\(\s*\d+/\d+\)\s+)?(\S+)\s+(\S+)\s*$"
)

seen = set()
for raw in sys.stdin:
    line = ansi_re.sub("", raw.rstrip("\r\n"))
    line = ts_re.sub("", line)
    m = status_re.match(line)
    if not m:
        continue
    key = (m.group(2), m.group(3))
    if key not in seen:
        seen.add(key)
        print(f"{key[0]} {key[1]}")
'
