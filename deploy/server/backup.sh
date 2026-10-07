#!/usr/bin/env bash
# backup.sh — daily backup of the XCHANGE v3 server (database and uploads).
# Run through xc-v3-backup.service (timer, by hand, or update.sh); never run as a plain command.
# docs/design/xchange-v3.md §7b, issue #121, row BK.
set -euo pipefail

XC_BACKUP_DIR="${XC_BACKUP_DIR:-/var/backups/xc-v3}"
XC_BACKUP_KEEP="${XC_BACKUP_KEEP:-7}"
XC_BACKUP_OFFHOST="${XC_BACKUP_OFFHOST:-}"
XC_BACKUP_SSH_KEY="${XC_BACKUP_SSH_KEY:-/etc/xc-v3/backup-offhost.key}"
XC_BACKUP_RECIPIENTS="${XC_BACKUP_RECIPIENTS:-/etc/xc-v3/backup.age-recipients}"

APP_DATABASE__HOST="${APP_DATABASE__HOST:-127.0.0.1}"
APP_DATABASE__PORT="${APP_DATABASE__PORT:-5432}"
APP_DATABASE__USERNAME="${APP_DATABASE__USERNAME:-xc_v3}"
APP_DATABASE__PASSWORD="${APP_DATABASE__PASSWORD:-}"
APP_DATABASE__DATABASE_NAME="${APP_DATABASE__DATABASE_NAME:-xc_v3}"
APP_FILES__DIR="${APP_FILES__DIR:-/var/lib/xc-v3/uploads}"

# age must be installed
if ! command -v age >/dev/null 2>&1; then
  echo "ERROR: age is not installed" >&2
  exit 1
fi

# XC_BACKUP_RECIPIENTS file must exist, be readable, and contain at least one age1 recipient line
if [ ! -r "$XC_BACKUP_RECIPIENTS" ] || ! grep -q '^age1' "$XC_BACKUP_RECIPIENTS" 2>/dev/null; then
  echo "ERROR: XC_BACKUP_RECIPIENTS file '$XC_BACKUP_RECIPIENTS' is missing, unreadable, or contains no age1 recipient" >&2
  exit 1
fi

# For a non-local database host, PGSSLMODE=require is required
is_local_db_host() {
  local h="${1,,}"
  [ "$h" = "localhost" ] && return 0
  [[ "$h" == /* ]] && return 0
  [[ "$h" =~ ^127\. ]] && return 0
  [ "$h" = "::1" ] && return 0
  return 1
}

if ! is_local_db_host "$APP_DATABASE__HOST"; then
  if [ -z "${PGSSLMODE:-}" ] || [ "$PGSSLMODE" != "verify-full" ]; then
    export PGSSLMODE=require
  fi
fi

# XC_BACKUP_KEEP must be an integer >= 1
if ! [[ "$XC_BACKUP_KEEP" =~ ^[0-9]+$ ]] || [ "$XC_BACKUP_KEEP" -lt 1 ]; then
  echo "ERROR: XC_BACKUP_KEEP must be an integer >= 1, got '$XC_BACKUP_KEEP'" >&2
  exit 1
fi

# XC_BACKUP_DIR mode and owner check
running_uid="$(id -u)"
if [ -e "$XC_BACKUP_DIR" ]; then
  if [ ! -d "$XC_BACKUP_DIR" ]; then
    echo "ERROR: XC_BACKUP_DIR '$XC_BACKUP_DIR' exists and is not a directory" >&2
    exit 1
  fi
  dir_mode="$(stat -c '%a' "$XC_BACKUP_DIR")"
  dir_uid="$(stat -c '%u' "$XC_BACKUP_DIR")"
  if [ "$dir_mode" != "700" ] || [ "$dir_uid" != "$running_uid" ]; then
    echo "ERROR: XC_BACKUP_DIR '$XC_BACKUP_DIR' already exists with mode '$dir_mode' and owner '$dir_uid' (expected mode 700, owner $running_uid)" >&2
    exit 1
  fi
else
  mkdir -p "$XC_BACKUP_DIR"
  chmod 0700 "$XC_BACKUP_DIR"
fi

# One run at a time: flock on $XC_BACKUP_DIR/.lock
exec 9>"$XC_BACKUP_DIR/.lock"
if ! flock -n 9; then
  echo "ERROR: another backup is currently running in $XC_BACKUP_DIR" >&2
  exit 1
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
set_dir="$XC_BACKUP_DIR/$stamp"
if [ -e "$set_dir" ]; then
  echo "ERROR: backup set '$stamp' ($set_dir) already exists" >&2
  exit 1
fi

work_dir="$XC_BACKUP_DIR/.work-$stamp"
rm -rf "$work_dir"
mkdir -m 0700 "$work_dir"
trap 'rm -rf "$work_dir"' EXIT

dump_file="$work_dir/db-$stamp.dump"
PGPASSWORD="$APP_DATABASE__PASSWORD" pg_dump -h "$APP_DATABASE__HOST" -p "$APP_DATABASE__PORT" -U "$APP_DATABASE__USERNAME" -Fc -f "$dump_file" "$APP_DATABASE__DATABASE_NAME"

# Check the plain dump with pg_restore --list before encrypting
if ! dump_list="$(pg_restore --list "$dump_file" </dev/null 2>&1)"; then
  echo "ERROR: pg_restore --list failed on dump (expected _sqlx_migrations): $dump_list" >&2
  exit 1
fi
if ! printf '%s\n' "$dump_list" | grep -qs '_sqlx_migrations'; then
  echo "ERROR: backup dump does not contain _sqlx_migrations" >&2
  exit 1
fi

# Encrypt dump to age recipient and remove the plain dump
age -R "$XC_BACKUP_RECIPIENTS" -o "$dump_file.age" "$dump_file"
rm -f "$dump_file"

# Stream tar into age to produce encrypted archive
archive_file="$work_dir/uploads-$stamp.tar.gz.age"
tar -czf - -C "$APP_FILES__DIR" . | age -R "$XC_BACKUP_RECIPIENTS" -o "$archive_file"

chmod 0700 "$work_dir"
mv "$work_dir" "$set_dir"
chmod 0700 "$set_dir"
trap - EXIT

# Retention: keep newest XC_BACKUP_KEEP sets
local_sets=()
for entry in "$XC_BACKUP_DIR"/*; do
  if [ -d "$entry" ]; then
    base="${entry##*/}"
    if [[ "$base" =~ ^[0-9]{8}T[0-9]{6}Z$ ]]; then
      local_sets+=("$base")
    fi
  fi
done

count=0
if [ "${#local_sets[@]}" -gt 0 ]; then
  mapfile -t sorted_sets < <(printf '%s\n' "${local_sets[@]}" | sort)
  count="${#sorted_sets[@]}"
  if [ "$count" -gt "$XC_BACKUP_KEEP" ]; then
    prune_count=$((count - XC_BACKUP_KEEP))
    for ((i = 0; i < prune_count; i++)); do
      rm -rf "${XC_BACKUP_DIR:?}/${sorted_sets[i]}"
    done
    sorted_sets=("${sorted_sets[@]:$prune_count}")
    count="${#sorted_sets[@]}"
  fi
fi

# Off-host copy
if [ -z "$XC_BACKUP_OFFHOST" ]; then
  echo "off-host copy skipped: XC_BACKUP_OFFHOST is empty"
else
  rsync_args=(-a)
  if [ "$count" -ge "$XC_BACKUP_KEEP" ]; then
    rsync_args+=(--delete)
  fi
  rsync_args+=(-e "ssh -i $XC_BACKUP_SSH_KEY -o BatchMode=yes")
  stamp_glob='[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z'
  rsync_args+=(--include="/$stamp_glob/")
  rsync_args+=(--include="/$stamp_glob/*.age")
  rsync_args+=(--exclude='*')
  rsync_args+=("${XC_BACKUP_DIR%/}/" "${XC_BACKUP_OFFHOST%/}/")
  rsync "${rsync_args[@]}"
fi

echo "$set_dir"
