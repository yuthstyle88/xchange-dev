#!/usr/bin/env bash
set -euo pipefail

# End-to-end verification of xc-device-agent against a running xchange server (AG-6, ticket #353).
# Verifies pairing via 'install --no-service', foreground execution of 'run',
# drawer kick delivery via CUPS (fake lp/lpstat), and audit line recording.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

BASE_TMP="${TMPDIR:-/tmp}"
if ! (mkdir -p "$BASE_TMP/test-exec.$$" && touch "$BASE_TMP/test-exec.$$/test.sh" && chmod +x "$BASE_TMP/test-exec.$$/test.sh" && "$BASE_TMP/test-exec.$$/test.sh" 2>/dev/null); then
  BASE_TMP="$ROOT_DIR/target/tmp"
  mkdir -p "$BASE_TMP"
fi
rm -rf "$BASE_TMP/test-exec.$$" 2>/dev/null || true
WORK_DIR="$(mktemp -d "$BASE_TMP/xc-e2e.XXXXXX")"
SERVER_LOG="$WORK_DIR/server.log"
AGENT_LOG="$WORK_DIR/agent.log"
SERVER_PID=""
AGENT_PID=""

cleanup() {
  local exit_code=$?
  if [ -n "$AGENT_PID" ] && kill -0 "$AGENT_PID" 2>/dev/null; then
    kill "$AGENT_PID" 2>/dev/null || true
    wait "$AGENT_PID" 2>/dev/null || true
  fi
  if [ -n "$SERVER_PID" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [ "$exit_code" -ne 0 ]; then
    echo "=== e2e FAILED (exit code $exit_code) ==="
    if [ -f "$SERVER_LOG" ]; then
      echo "--- Server Log (${SERVER_LOG}) ---"
      cat "$SERVER_LOG"
    fi
    if [ -f "$AGENT_LOG" ]; then
      echo "--- Agent Log (${AGENT_LOG}) ---"
      cat "$AGENT_LOG"
    fi
  fi
  rm -rf "$WORK_DIR"
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

# 0. Locate agent and server binaries
AGENT_BIN="${1:-${AGENT_BIN:-}}"
if [ -z "$AGENT_BIN" ]; then
  if [ -x "dist/xc-device-agent" ]; then
    AGENT_BIN="$(pwd)/dist/xc-device-agent"
  elif [ -x "api/target/release/xc-device-agent" ]; then
    AGENT_BIN="$(pwd)/api/target/release/xc-device-agent"
  elif [ -x "api/target/debug/xc-device-agent" ]; then
    AGENT_BIN="$(pwd)/api/target/debug/xc-device-agent"
  else
    echo "Building xc-device-agent..."
    (cd api && cargo build -p xc-device-agent --bin xc-device-agent)
    AGENT_BIN="$(pwd)/api/target/debug/xc-device-agent"
  fi
fi

if [ ! -x "$AGENT_BIN" ]; then
  echo "ERROR: xc-device-agent binary not executable at '$AGENT_BIN'"
  exit 1
fi
echo "Using agent binary: $AGENT_BIN"

SERVER_BIN="${SERVER_BIN:-}"
if [ -z "$SERVER_BIN" ]; then
  if [ -x "api/target/release/xchange" ]; then
    SERVER_BIN="$(pwd)/api/target/release/xchange"
  elif [ -x "api/target/debug/xchange" ]; then
    SERVER_BIN="$(pwd)/api/target/debug/xchange"
  else
    echo "Building xchange server..."
    (cd api && cargo build -p xchange --bin xchange)
    SERVER_BIN="$(pwd)/api/target/debug/xchange"
  fi
fi

if [ ! -x "$SERVER_BIN" ]; then
  echo "ERROR: xchange server binary not executable at '$SERVER_BIN'"
  exit 1
fi
echo "Using server binary: $SERVER_BIN"

# Database parameters
PGHOST="${PGHOST:-127.0.0.1}"
PGPORT="${PGPORT:-5432}"
PGUSER="${PGUSER:-postgres}"
PGDATABASE="${PGDATABASE:-xchange_test}"

export PGHOST PGPORT PGUSER PGDATABASE

if [ "${USE_DOCKER_PSQL:-0}" = "1" ] || ! command -v psql >/dev/null 2>&1; then
  if command -v docker >/dev/null 2>&1; then
    echo "Using postgres:16-alpine container for psql..."
    psql() {
      docker run --rm --net=host --label fleet.worker=gemini/studio-1 -i \
        -e PGHOST -e PGPORT -e PGUSER -e PGDATABASE \
        postgres:16-alpine psql "$@"
    }
  else
    echo "ERROR: neither psql nor docker is available"
    exit 1
  fi
fi

echo "Connecting to PostgreSQL at $PGHOST:$PGPORT/$PGDATABASE..."
psql -tAc "SELECT 1;" >/dev/null

# 1. Start server on a free port on 127.0.0.1
if command -v python3 >/dev/null 2>&1; then
  PORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
else
  PORT=18080
fi
echo "Starting xchange server on port $PORT..."

(
  cd api
  export APP_APPLICATION__HOST="127.0.0.1"
  export APP_APPLICATION__PORT="$PORT"
  export APP_DATABASE__HOST="$PGHOST"
  export APP_DATABASE__PORT="$PGPORT"
  export APP_DATABASE__USERNAME="$PGUSER"
  export APP_DATABASE__DATABASE_NAME="$PGDATABASE"
  export APP_DATABASE__REQUIRE_SSL="false"
  export APP_CONFIG_DIR="configuration"
  export APP_TELEMETRY__LOG_LEVEL="info"
  exec "$SERVER_BIN"
) > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# Wait for server to become healthy
READY=0
for _ in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$PORT/health/ready" >/dev/null 2>&1; then
    READY=1
    break
  fi
  sleep 0.5
done

if [ "$READY" -ne 1 ]; then
  echo "ERROR: xchange server failed to become ready on http://127.0.0.1:$PORT"
  exit 1
fi
echo "Server is ready on http://127.0.0.1:$PORT"

# 2. Create agent of MAIN straight in device_agents through psql
PAIR_CODE="ABCDE-FGHJK"
CANONICAL_CODE="ABCDEFGHJK"
if command -v shasum >/dev/null 2>&1; then
  CODE_HASH=$(printf '%s' "$CANONICAL_CODE" | shasum -a 256 | awk '{print $1}')
else
  CODE_HASH=$(printf '%s' "$CANONICAL_CODE" | sha256sum | awk '{print $1}')
fi

AGENT_ID="a0000000-0000-4000-8000-000000000001"
echo "Registering agent $AGENT_ID with pairing code $PAIR_CODE (hash $CODE_HASH)..."

psql -v ON_ERROR_STOP=1 <<SQL
DELETE FROM device_jobs WHERE branch_id = 'MAIN';
DELETE FROM device_events WHERE branch_id = 'MAIN';
DELETE FROM device_agents WHERE branch_id = 'MAIN';

INSERT INTO device_agents (
    id, branch_id, name, created_at, created_by, pair_code_hash, pair_code_expires_at
) VALUES (
    '$AGENT_ID',
    'MAIN',
    'E2E Counter Agent',
    now(),
    'owner',
    '$CODE_HASH',
    now() + interval '1 hour'
);
SQL

# 3. Pair the built agent binary against http://127.0.0.1:<port>
AGENT_CONFIG_DIR="$WORK_DIR/agent_config"
mkdir -p "$AGENT_CONFIG_DIR"
export XC_AGENT_CONFIG_DIR="$AGENT_CONFIG_DIR"

echo "Pairing xc-device-agent with server..."
"$AGENT_BIN" install \
  --server "http://127.0.0.1:$PORT" \
  --code "$PAIR_CODE" \
  --cups-queue "POS80_" \
  --no-service

if [ ! -f "$AGENT_CONFIG_DIR/config.json" ] || [ ! -f "$AGENT_CONFIG_DIR/token" ]; then
  echo "ERROR: pairing did not write config.json or token"
  exit 1
fi
echo "Pairing succeeded. Config and token written."

# 4. Prepare fake lp and lpstat commands
FAKE_BIN_DIR="$WORK_DIR/fake_bin"
mkdir -p "$FAKE_BIN_DIR"
LP_ARGS_FILE="$WORK_DIR/lp_args.txt"
LP_STDIN_FILE="$WORK_DIR/lp_stdin.bin"

cat <<'EOF' > "$FAKE_BIN_DIR/lpstat"
#!/bin/sh
exit 0
EOF
chmod +x "$FAKE_BIN_DIR/lpstat"

cat <<EOF > "$FAKE_BIN_DIR/lp"
#!/bin/sh
echo "\$@" > "$LP_ARGS_FILE"
cat > "$LP_STDIN_FILE"
echo "request id is POS80_-42 (1 file(s))"
exit 0
EOF
chmod +x "$FAKE_BIN_DIR/lp"

echo "Starting xc-device-agent run with fake CUPS commands..."
(
  export PATH="$FAKE_BIN_DIR:$PATH"
  export XC_AGENT_CONFIG_DIR="$AGENT_CONFIG_DIR"
  exec "$AGENT_BIN" run
) > "$AGENT_LOG" 2>&1 &
AGENT_PID=$!

# Wait a moment for agent loop to start polling
sleep 1

# 5. Queue one DRAWER_REQUEST of MAIN
JOB_ID="dddddddd-0000-4000-8000-000000000001"
echo "Queuing drawer job $JOB_ID in MAIN..."

psql -v ON_ERROR_STOP=1 <<SQL
INSERT INTO device_jobs (
    id, request_id, request_hash, branch_id, ref, kind, status, actor, created_at, updated_at
) VALUES (
    '$JOB_ID',
    '$JOB_ID',
    'e2e-hash',
    'MAIN',
    'XC-E2E-1',
    'DRAWER_REQUEST',
    'QUEUED',
    'owner',
    now(),
    now()
);
SQL

# 6. Wait up to 15 s for the job to become DRAWER_OPENED
echo "Waiting up to 15s for job $JOB_ID to become DRAWER_OPENED..."
DEADLINE=$((SECONDS + 15))
JOB_STATUS=""
while [ $SECONDS -lt $DEADLINE ]; do
  JOB_STATUS=$(psql -tAc "SELECT status FROM device_jobs WHERE id = '$JOB_ID';" || true)
  if [ "$JOB_STATUS" = "DRAWER_OPENED" ]; then
    break
  fi
  sleep 0.5
done

if [ "$JOB_STATUS" != "DRAWER_OPENED" ]; then
  echo "ERROR: Job status is '$JOB_STATUS', expected 'DRAWER_OPENED'"
  exit 1
fi
echo "Job transitioned to DRAWER_OPENED."

# Verify lp received correct arguments
if [ ! -f "$LP_ARGS_FILE" ]; then
  echo "ERROR: fake lp was not invoked (missing $LP_ARGS_FILE)"
  exit 1
fi
ACTUAL_ARGS="$(cat "$LP_ARGS_FILE")"
EXPECTED_ARGS="-d POS80_ -o raw"
if [ "$ACTUAL_ARGS" != "$EXPECTED_ARGS" ]; then
  echo "ERROR: fake lp expected args '$EXPECTED_ARGS', got '$ACTUAL_ARGS'"
  exit 1
fi
echo "Verified lp arguments: $ACTUAL_ARGS"

# Verify lp received correct stdin bytes: 1b 40 1b 70 00 19 fa
if [ ! -f "$LP_STDIN_FILE" ]; then
  echo "ERROR: fake lp received no stdin (missing $LP_STDIN_FILE)"
  exit 1
fi
ACTUAL_HEX="$(od -An -tx1 "$LP_STDIN_FILE" | tr -d ' \n')"
EXPECTED_HEX="1b401b700019fa"
if [ "$ACTUAL_HEX" != "$EXPECTED_HEX" ]; then
  echo "ERROR: expected kick bytes '$EXPECTED_HEX', got '$ACTUAL_HEX'"
  exit 1
fi
echo "Verified lp kick bytes: $ACTUAL_HEX"

# Verify DRAWER_OPENED audit line in device_events
EVENT_COUNT=$(psql -tAc "
  SELECT count(*) FROM device_events
  WHERE kind = 'DRAWER_OPENED'
    AND actor = 'agent:$AGENT_ID'
    AND (detail->>'cupsJob') = 'POS80_-42';
")
if [ "$EVENT_COUNT" -ne 1 ]; then
  echo "ERROR: expected 1 DRAWER_OPENED audit event with cupsJob 'POS80_-42' by agent:$AGENT_ID, got $EVENT_COUNT"
  exit 1
fi
echo "Verified audit event recorded in device_events."

# Verify token is never leaked in server or agent logs
if grep -q "xcda_" "$SERVER_LOG"; then
  echo "ERROR: raw token leaked in server log"
  exit 1
fi
if grep -q "xcda_" "$AGENT_LOG"; then
  echo "ERROR: raw token leaked in agent log"
  exit 1
fi
echo "Verified token does not appear in logs."

echo "=== End-to-end verification PASSED ==="
