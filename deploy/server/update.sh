#!/usr/bin/env bash
# Updates an installed XCHANGE v3 server to the binary and configuration in the
# stage directory. What it replaces it keeps first — the binary as xchange.prev,
# the configuration as configuration.prev — and it prints how to roll back.
# If it fails between stopping and starting the service, it says that the
# service is stopped and prints the commands that bring the previous state back.
# Run as a sudo-capable user; only on the owner's order (deploy/README.md).
#
# Variables: see the block at the top of common.sh.
set -euo pipefail
# shellcheck source=deploy/server/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

[ "$(id -u)" -ne 0 ] || die "run as a sudo-capable user, not as root"
need sudo systemctl curl sha256sum stat sed psql
adopt_installed_settings
check_names
check_stage xchange configuration/base.yaml
[ -f "$HERE/xc-v3.service" ] && [ -f "$HERE/smoke.sh" ] || die "xc-v3.service and smoke.sh must sit next to this script"
if ! { sudo test -f "$UNIT_FILE" && sudo test -f "$ENV_FILE" && sudo test -x "$BIN"; }; then
  die "$SERVICE_NAME is not installed here (run install.sh first)"
fi
sudo test -d "$UPLOADS_DIR" || die "uploads directory missing: $UPLOADS_DIR (run install.sh; it creates it)"

BEFORE="$(applied_migrations)"

# From the stop until the start, a failure must not leave the service down
# without a word: say what state it is in and how to bring it back.
PHASE=check
BIN_SAVED=0
CONFIG_SAVED=0
on_exit() {
  local rc=$?
  if [ "$rc" -ne 0 ] && [ "$PHASE" = swap ]; then
    {
      echo
      echo "ERROR: the update stopped midway (exit $rc). $SERVICE_NAME is STOPPED."
      if [ "$BIN_SAVED" = 0 ] && [ "$CONFIG_SAVED" = 0 ]; then
        echo "Nothing was replaced yet. Start the service as it was:"
        echo "  sudo systemctl start $SERVICE_NAME"
      else
        echo "A replacement was begun and may be incomplete. Put back what was saved and start:"
        print_rollback "$BIN_SAVED" "$CONFIG_SAVED"
      fi
    } >&2
  fi
}
trap on_exit EXIT

PHASE=swap
sudo systemctl stop "$SERVICE_NAME.service"
replace_binary
replace_configuration
install_unit
sudo systemctl start "$SERVICE_NAME.service"
PHASE=started

if ! run_smoke; then
  echo "---- last lines of the service log ----" >&2
  sudo journalctl -u "$SERVICE_NAME.service" -n 20 --no-pager -o cat >&2 || true
  echo "applied migrations: before $BEFORE, now $(applied_migrations)" >&2
  print_rollback "$BIN_SAVED" "$CONFIG_SAVED" >&2
  die "smoke check failed after the update; nothing was rolled back automatically"
fi

AFTER="$(applied_migrations)"
say "updated. applied migrations: before $BEFORE, now $AFTER"
if [ "$BEFORE" != "$AFTER" ]; then
  echo "NOTE: this update applied migrations. xchange.prev will not start on this schema;"
  echo "      going back now needs a database restore as well (backup: not written yet)."
fi
print_rollback "$BIN_SAVED" "$CONFIG_SAVED"
