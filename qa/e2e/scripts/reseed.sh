#!/usr/bin/env bash
# New test accounts on the host, written into users.json; the admin entry already in users.json is kept.
#   bash scripts/reseed.sh [ssh-host] [base-url]
# Needs /tmp/seed-test-booking.sh on the host (deploy/server/seed-test-booking.sh, PR 3837).
set -euo pipefail
HOST="${1:-do-sgp1}"
BASE="${2:-https://xc-v3.spacing.cloud}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NEW="$(mktemp)"
trap 'rm -f "$NEW"' EXIT
ssh "$HOST" "bash /tmp/seed-test-booking.sh --json $BASE" > "$NEW"
node -e '
  const fs = require("fs");
  const [newPath, outPath] = process.argv.slice(1);
  const fresh = JSON.parse(fs.readFileSync(newPath, "utf8"));
  let old = {};
  try { old = JSON.parse(fs.readFileSync(outPath, "utf8")); } catch {}
  if (old.admin && old.admin.password && !String(old.admin.password).startsWith("<")) fresh.admin = old.admin;
  fs.writeFileSync(outPath, JSON.stringify(fresh, null, 2) + "\n", { mode: 0o600 });
  console.error("users.json written: customer " + fresh.customer.phone + ", teller " + fresh.teller.phone +
    ", teller2 " + (fresh.teller2 || {}).phone +
    ", branch manager " + (fresh.branchManager || {}).phone + (fresh.admin.password.startsWith("<") ? ", admin NOT set" : ", admin kept"));
' "$NEW" "$HERE/users.json"
