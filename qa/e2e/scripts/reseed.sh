#!/usr/bin/env bash
# New test accounts on the host, written into users.json.
#   bash scripts/reseed.sh [ssh-host] [base-url]
# Needs /tmp/seed-test-booking.sh on the host (deploy/server/seed-test-booking.sh, PR 3837).
set -euo pipefail
HOST="${1:-do-sgp1}"
BASE="${2:-https://xc-v3.spacing.cloud}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NEW="$(mktemp)"
trap 'rm -f "$NEW"' EXIT
# shellcheck disable=SC2029
ssh "$HOST" "bash /tmp/seed-test-booking.sh --json $BASE" > "$NEW"
node -e '
  const fs = require("fs");
  const [newPath, outPath] = process.argv.slice(1);
  const fresh = JSON.parse(fs.readFileSync(newPath, "utf8"));
  fs.writeFileSync(outPath, JSON.stringify(fresh, null, 2) + "\n", { mode: 0o600 });
  console.error("users.json written: customer " + fresh.customer.phone + ", teller " + fresh.teller.phone +
    ", teller2 " + (fresh.teller2 || {}).phone +
    ", branch manager " + (fresh.branchManager || {}).phone +
    ", admin " + (fresh.admin?.identifier || fresh.admin?.phone || "") +
    ", executive " + (fresh.executive?.phone || "") +
    ", owner " + (fresh.owner?.phone || ""));
' "$NEW" "$HERE/users.json"
