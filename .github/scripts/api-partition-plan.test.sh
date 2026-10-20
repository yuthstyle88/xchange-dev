#!/usr/bin/env bash
# Self-test for .github/scripts/api-partition-plan.sh (#3112). Written before the script by the ticket
# author claude/dell-claude-6; do not edit, delete or weaken.
#
# Contract of api-partition-plan.sh:
#   env RUN_TESTS=true|false, PARTITIONS=<n> (default 4), RUN_ID / RUN_ATTEMPT passed through to select-pool.sh
#   prints one line: test_matrix=<compact JSON> for `strategy.matrix`, e.g.
#     {"include":[{"partition":1,"total":4,"runs_on":["self-hosted","heavy-build"]}, ...]}
#   - every partition's runs_on is exactly what select-pool.sh prints (the single pool source, PRs 3795/3799);
#   - no OS, architecture or machine names: placement is decided by which runners are switched on;
#   - RUN_TESTS=true => PARTITIONS entries, partition 1..n, each total=n;
#   - RUN_TESTS=false (a PR that runs no tests) => one entry, partition 1, total 1.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PLAN="$SCRIPT_DIR/api-partition-plan.sh"
POOL="$SCRIPT_DIR/select-pool.sh"

if [ ! -x "$PLAN" ]; then
  echo "FAIL: $PLAN not found or not executable" >&2
  exit 1
fi

POOL_RUNS_ON=$(RUN_ID=7 RUN_ATTEMPT=1 bash "$POOL" | sed -n 's/^runs_on=//p')
[ -n "$POOL_RUNS_ON" ] || { echo "FAIL: select-pool.sh printed no runs_on" >&2; exit 1; }

fail=0
check() { # check <label> <expected partitions> <env...>
  local label="$1" want="$2" out matrix
  shift 2
  out=$(env "$@" RUN_ID=7 RUN_ATTEMPT=1 bash "$PLAN")
  if [ "$(printf '%s\n' "$out" | grep -c '^test_matrix=')" != 1 ]; then
    echo "FAIL: $label: want exactly one test_matrix= line, got: $out"
    fail=1
    return
  fi
  matrix=$(printf '%s\n' "$out" | sed -n 's/^test_matrix=//p')
  if ! python3 - "$matrix" "$want" "$POOL_RUNS_ON" <<'PY'
import json, re, sys
raw, want, pool = sys.argv[1], int(sys.argv[2]), json.loads(sys.argv[3])
m = json.loads(raw)
inc = m["include"]
assert len(inc) == want, f"{len(inc)} partitions, want {want}"
assert [e["partition"] for e in inc] == list(range(1, want + 1)), "partitions not 1..n in order"
assert all(e["total"] == want for e in inc), "total differs from the partition count"
assert all(e["runs_on"] == pool for e in inc), f"runs_on differs from select-pool.sh's {pool}"
assert "\n" not in raw, "matrix must be one line for GITHUB_OUTPUT"
pins = re.compile(r"Linux|macOS|Windows|X64|ARM64|studio|dell|mbp|warut|macmini|jam|lenovo", re.I)
assert not pins.search(raw), "an OS, architecture or machine name in the matrix"
PY
  then
    echo "FAIL: $label: $matrix"
    fail=1
  fi
}

# Row 1: a run with tests splits into 4 partitions, all on select-pool.sh's pool.
check "run_tests=true, default partitions" 4 RUN_TESTS=true
# Row 2: the partition count is a parameter.
check "run_tests=true, PARTITIONS=3" 3 RUN_TESTS=true PARTITIONS=3
# Row 3: a PR that runs no tests needs one job only (fmt, clippy and the other checks).
check "run_tests=false" 1 RUN_TESTS=false PARTITIONS=4

# Row 4: steps whose output `Test` reads in every partition must never be gated on matrix.partition == 1.
API_YML="$SCRIPT_DIR/../workflows/api.yml"
if ! python3 - "$API_YML" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
steps = re.findall(r"(?m)^      - name: .*(?:\n        .*)*", text)
needed = [
    "Start postgres (per run)",
    "Verify rustc 1.98.1",
    "Set cargo jobs from this machine's free cores",
    "Set up Node.js 24",
    '"Web screens: build"',
    "Install cargo-nextest",
    "Read failed tests from previous attempt log (re-run)",
]
for want in needed:
    matched = [s for s in steps if s.splitlines()[0].strip() == f"- name: {want}"]
    assert matched, f"step missing in api.yml: {want}"
    for s in matched:
        # A condition may name partition 1 only inside an `run_tests == 'true' || (...)` escape,
        # which still runs the step in every partition that tests.
        for cond in re.findall(r"(?m)^\s*if:(.*)$", s):
            gated = re.search(r"\bmatrix\.partition\s*==\s*1\b", cond)
            escaped = re.search(r"run_tests\s*==\s*'true'\s*\|\|", cond)
            assert not gated or escaped, f"step {want} is limited to matrix.partition == 1"
PY
then
  echo "FAIL: api.yml gates a step needed by Test on matrix.partition == 1"
  fail=1
fi

[ "$fail" = 0 ] && echo "api-partition-plan: all rows pass"
exit "$fail"
