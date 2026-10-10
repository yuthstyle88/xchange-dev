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
need sudo systemctl curl sha256sum stat sed psql pg_dump
adopt_installed_settings
check_names

# api/src/bootstrap/mod.rs:177: the binary hard-codes /etc/xc-v3/pii.key, which
# matches $ETC_DIR/pii.key under the documented defaults (SERVICE_NAME=xc-v3).
PII_KEY_FILE="$ETC_DIR/pii.key"

if ! sudo test -f "$PII_KEY_FILE"; then
  die "PII key file missing: $PII_KEY_FILE (run install.sh or generate with: { printf 'k1:'; openssl rand -base64 32; } | sudo install -o root -g $SERVICE_USER -m 640 /dev/stdin $PII_KEY_FILE)"
fi
if ! sudo -u "$SERVICE_USER" test -r "$PII_KEY_FILE"; then
  die "PII key file $PII_KEY_FILE is not readable by service user $SERVICE_USER"
fi

check_stage xchange configuration/base.yaml
if [ ! -f "$HERE/xc-v3.service" ] || [ ! -f "$HERE/smoke.sh" ]; then
  die "xc-v3.service and smoke.sh must sit next to this script"
fi
if ! { sudo test -f "$UNIT_FILE" && sudo test -f "$ENV_FILE" && sudo test -x "$BIN"; }; then
  die "$SERVICE_NAME is not installed here (run install.sh first)"
fi
sudo test -d "$UPLOADS_DIR" || die "uploads directory missing: $UPLOADS_DIR (run install.sh; it creates it)"

if [ ! -d "$STAGE/migrations" ]; then
  die "stage carries no migrations/ directory (rehearsal requires migrations)"
fi

shopt -s nullglob
MIGRATION_FILES=("$STAGE"/migrations/*.sql)
shopt -u nullglob
for f in "${MIGRATION_FILES[@]}"; do
  listed_in_sums "migrations/$(basename "$f")" ||
    die "SHA256SUMS does not list migrations/$(basename "$f")"
done

PHASE=check
BIN_SAVED=0
CONFIG_SAVED=0
SCRATCH_DB=""

drop_scratch() {
  if [ -n "${SCRATCH_DB:-}" ]; then
    local db="$SCRATCH_DB"
    SCRATCH_DB=""
    pg -c "DROP DATABASE IF EXISTS \"$db\";" >/dev/null 2>&1 || true
  fi
}

on_exit() {
  local rc=$?
  drop_scratch
  if [ "$rc" -ne 0 ] && [ "$PHASE" = swap ]; then
    {
      echo
      echo "ERROR: the update stopped midway (exit $rc). $SERVICE_NAME is STOPPED."
      if [ "$BIN_SAVED" = 0 ] && [ "$CONFIG_SAVED" = 0 ]; then
        echo "Nothing was replaced yet. Start the service as it was:"
        echo "  sudo systemctl start $SERVICE_NAME"
      else
        echo "A replacement was begun and may be incomplete. Put back what was saved and start:"
        print_rollback "$BIN_SAVED" "$CONFIG_SAVED" "${SET_DIR:-}"
      fi
    } >&2
  fi
}
trap on_exit EXIT

# Rehearse pending migrations against a temporary scratch database
say "rehearsing pending migrations against scratch database..."
SCRATCH_DB="${DB_NAME}_rehearsal"

# 1. Drop if exists, then create scratch database
drop_scratch
SCRATCH_DB="${DB_NAME}_rehearsal"
pg -c "CREATE DATABASE \"$SCRATCH_DB\";"

# 2. Dump live database into scratch database (plain format, never pg_restore)
(cd / && sudo -u postgres pg_dump "$DB_NAME") | pg -d "$SCRATCH_DB"

# 3. Query applied migrations from scratch database
APPLIED_VERSIONS="$(pg -d "$SCRATCH_DB" -Atc "SELECT version FROM _sqlx_migrations WHERE success = true")"

# 4. Apply each pending migration in version order
while IFS= read -r f; do
  [ -n "$f" ] || continue
  fname="$(basename "$f")"
  version="${fname%%_*}"
  if echo "$APPLIED_VERSIONS" | grep -qx "$version"; then
    continue
  fi
  say "rehearsing migration: $fname"
  if ! err_out="$(pg -d "$SCRATCH_DB" --single-transaction -f - < "$f" 2>&1)"; then
    echo "ERROR: migration rehearsal failed on file: $fname" >&2
    printf '%s\n' "$err_out" >&2
    drop_scratch
    die "migration rehearsal refused by $fname"
  fi
done < <(printf '%s\n' "${MIGRATION_FILES[@]}" | LC_ALL=C sort)

# 5. Drop scratch database explicitly before backup
drop_scratch

BEFORE="$(applied_migrations)"

BACKUP_DIR="$(env_value XC_BACKUP_DIR)"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/xc-v3}"

if ! sudo systemctl cat xc-v3-backup.service >/dev/null 2>&1; then
  die "xc-v3-backup.service is not installed (BK is required before updating; see deploy/README.md §5)"
fi

BEFORE_SET="$(newest_backup_set "$BACKUP_DIR")"

say "taking backup before update: starting xc-v3-backup.service..."
if ! sudo systemctl start xc-v3-backup.service; then
  die "backup failed: xc-v3-backup.service exited non-zero (BK)"
fi

AFTER_SET="$(newest_backup_set "$BACKUP_DIR")"
if [ -z "$AFTER_SET" ] || [ "$AFTER_SET" = "$BEFORE_SET" ]; then
  die "backup failed: no new backup set was created in $BACKUP_DIR"
fi

SET_DIR="$BACKUP_DIR/$AFTER_SET"
DUMP_PATH="$SET_DIR/db-$AFTER_SET.dump.age"
ARCHIVE_PATH="$SET_DIR/uploads-$AFTER_SET.tar.gz.age"

# Refuse a plain (unencrypted) set
if sudo test -f "$SET_DIR/db-$AFTER_SET.dump"; then
  die "backup set is unencrypted (plain): $SET_DIR"
fi

if ! sudo test -f "$DUMP_PATH"; then
  die "encrypted backup dump file not found: $DUMP_PATH"
fi

if ! sudo test -f "$ARCHIVE_PATH"; then
  die "encrypted backup uploads archive not found: $ARCHIVE_PATH"
fi

say "verifying backup set is encrypted ($DUMP_PATH and $ARCHIVE_PATH)..."
dump_header="$(sudo head -n 1 "$DUMP_PATH" 2>/dev/null || true)"
if [ "$dump_header" != "age-encryption.org/v1" ]; then
  die "backup dump is not age-encrypted ($DUMP_PATH does not start with age-encryption.org/v1)"
fi

archive_header="$(sudo head -n 1 "$ARCHIVE_PATH" 2>/dev/null || true)"
if [ "$archive_header" != "age-encryption.org/v1" ]; then
  die "backup uploads archive is not age-encrypted ($ARCHIVE_PATH does not start with age-encryption.org/v1)"
fi

# From the stop until the start, a failure must not leave the service down
# without a word: say what state it is in and how to bring it back.
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
  print_rollback "$BIN_SAVED" "$CONFIG_SAVED" "${SET_DIR:-}" >&2
  die "smoke check failed after the update; nothing was rolled back automatically"
fi

AFTER="$(applied_migrations)"
say "updated. applied migrations: before $BEFORE, now $AFTER"
if [ "$BEFORE" != "$AFTER" ]; then
  echo "NOTE: this update applied migrations. xchange.prev will not start on this schema;"
  echo "      going back now needs a database restore as well."
  echo "      Backup dump: $DUMP_PATH"
fi
print_rollback "$BIN_SAVED" "$CONFIG_SAVED" "$SET_DIR"
