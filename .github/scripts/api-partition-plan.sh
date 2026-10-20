#!/usr/bin/env bash
# Build the strategy.matrix JSON for the api.yml `test` job (#3112).
# Every partition uses the runner pool from select-pool.sh (single pool source, PRs 3795/3799).
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUNS_ON=$(bash "$SCRIPT_DIR/select-pool.sh" | sed -n 's/^runs_on=//p')

if [ "${RUN_TESTS:-false}" = "true" ]; then
  N="${PARTITIONS:-4}"
else
  N=1
fi

exec python3 - "$N" "$RUNS_ON" <<'PY'
import json
import sys

n = int(sys.argv[1])
runs_on = json.loads(sys.argv[2])
matrix = {
    "include": [
        {"partition": k, "total": n, "runs_on": runs_on}
        for k in range(1, n + 1)
    ]
}
print("test_matrix=" + json.dumps(matrix, separators=(",", ":")))
PY
