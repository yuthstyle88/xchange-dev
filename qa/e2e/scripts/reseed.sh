#!/usr/bin/env bash
# Ensures the roster from users.json on the host without creating a second set (#5910).
#   bash scripts/reseed.sh [ssh-host] [base-url]
# Needs /tmp/seed-test-booking.sh on the host (deploy/server/seed-test-booking.sh).
set -euo pipefail
HOST="${1:-do-sgp1}"
BASE="${2:-https://xc-v3.spacing.cloud}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
USERS_FILE="$HERE/users.json"
if [ ! -f "$USERS_FILE" ]; then
  echo "ERROR: $USERS_FILE not found — the roster is made once through the admin UI and saved to users.json" >&2
  exit 1
fi
# shellcheck disable=SC2029
ssh "$HOST" "bash /tmp/seed-test-booking.sh --users - $BASE" < "$USERS_FILE"
