#!/usr/bin/env bash
# restore.sh <set-dir> <scratch-database> <scratch-uploads-dir>
# Rehearses restoring a backup set into a scratch database and scratch uploads directory.
# Must be run as root (uses runuser -u postgres --).
# docs/design/xchange-v3.md §7b, issue #121, row BK.
set -euo pipefail

XC_ENV_FILE="${XC_ENV_FILE:-/etc/xc-v3/env}"
XC_BACKUP_IDENTITY="${XC_BACKUP_IDENTITY:-}"

if [ "$#" -ne 3 ]; then
  echo "Usage: $0 <set-dir> <scratch-database> <scratch-uploads-dir>" >&2
  exit 1
fi

set_dir="$1"
scratch_db="$2"
scratch_uploads="$3"

# 1. Read live settings key-by-key from XC_ENV_FILE (never sourcing it)
if [ ! -f "$XC_ENV_FILE" ]; then
  echo "ERROR: environment file '$XC_ENV_FILE' not found" >&2
  exit 1
fi

live_db="$(sed -n 's/^APP_DATABASE__DATABASE_NAME=//p' "$XC_ENV_FILE" | tail -n 1)"
live_uploads="$(sed -n 's/^APP_FILES__DIR=//p' "$XC_ENV_FILE" | tail -n 1)"

if [ -z "$live_db" ]; then
  echo "ERROR: APP_DATABASE__DATABASE_NAME missing from $XC_ENV_FILE" >&2
  exit 1
fi

if [ -z "$live_uploads" ]; then
  echo "ERROR: APP_FILES__DIR missing from $XC_ENV_FILE" >&2
  exit 1
fi

# 2. Refuse empty, missing, or improperly-permissioned XC_BACKUP_IDENTITY
if [ -z "$XC_BACKUP_IDENTITY" ]; then
  echo "ERROR: XC_BACKUP_IDENTITY environment variable is empty or not set" >&2
  exit 1
fi

if [ ! -f "$XC_BACKUP_IDENTITY" ]; then
  echo "ERROR: XC_BACKUP_IDENTITY file '$XC_BACKUP_IDENTITY' does not exist" >&2
  exit 1
fi

id_mode="$(stat -c '%a' "$XC_BACKUP_IDENTITY")"
if [ "$id_mode" != "600" ] && [ "$id_mode" != "400" ]; then
  echo "ERROR: XC_BACKUP_IDENTITY file '$XC_BACKUP_IDENTITY' has permissions '$id_mode' (expected 600 or 400)" >&2
  exit 1
fi

if ! command -v age >/dev/null 2>&1; then
  echo "ERROR: age is not installed" >&2
  exit 1
fi

# 3. Refuse live database name (case-insensitive)
if [ "${scratch_db,,}" = "${live_db,,}" ]; then
  echo "ERROR: scratch database name '$scratch_db' matches live database '$live_db'" >&2
  exit 1
fi

# 4. Refuse invalid scratch database name (must be plain postgres name ^[a-z_][a-z0-9_]{0,62}$)
if ! [[ "$scratch_db" =~ ^[a-z_][a-z0-9_]{0,62}$ ]]; then
  echo "ERROR: scratch database name '$scratch_db' is not a plain PostgreSQL name" >&2
  exit 1
fi

# 5. Refuse live uploads directory, inside it, or containing it
real_live="$(readlink -m "$live_uploads")"
real_scratch="$(readlink -m "$scratch_uploads")"

if [ "$real_scratch" = "$real_live" ] || [[ "$real_scratch" == "$real_live"/* ]] || [[ "$real_live" == "$real_scratch"/* ]]; then
  echo "ERROR: scratch uploads directory '$scratch_uploads' conflicts with live uploads directory '$live_uploads'" >&2
  exit 1
fi

# 6. Scratch directory if it exists must be empty
if [ -e "$scratch_uploads" ]; then
  if [ ! -d "$scratch_uploads" ]; then
    echo "ERROR: scratch uploads path '$scratch_uploads' exists and is not a directory" >&2
    exit 1
  fi
  if [ -n "$(ls -A "$scratch_uploads" 2>/dev/null)" ]; then
    echo "ERROR: scratch uploads directory '$scratch_uploads' already exists and is not empty" >&2
    exit 1
  fi
fi

# 7. Set directory validation: must exist and have both pieces (.age)
if [ ! -d "$set_dir" ]; then
  echo "ERROR: set directory '$set_dir' does not exist" >&2
  exit 1
fi

stamp="${set_dir##*/}"
dump_file="$set_dir/db-$stamp.dump.age"
archive_file="$set_dir/uploads-$stamp.tar.gz.age"

if [ ! -f "$dump_file" ] || [ ! -f "$archive_file" ]; then
  echo "ERROR: set directory '$set_dir' is missing dump or uploads archive (.age)" >&2
  exit 1
fi

# 8. Check if scratch database already exists
if runuser -u postgres -- psql -tAc "SELECT 1 FROM pg_database WHERE datname = '$scratch_db'" 2>/dev/null | grep -q 1; then
  echo "ERROR: scratch database '$scratch_db' already exists" >&2
  exit 1
fi

# 9. Create scratch database and scratch uploads directory
runuser -u postgres -- createdb "$scratch_db"
mkdir -p "$scratch_uploads"
chmod 0700 "$scratch_uploads"

# Clean up on failure
cleanup_on_error() {
  runuser -u postgres -- dropdb "$scratch_db" || true
  rm -rf "$scratch_uploads"
}
trap cleanup_on_error ERR

# 10. Decrypt and pg_restore as postgres, dump on standard input
age -d -i "$XC_BACKUP_IDENTITY" "$dump_file" | runuser -u postgres -- pg_restore --no-owner -d "$scratch_db"

# 11. Decrypt and extract uploads archive into scratch directory
age -d -i "$XC_BACKUP_IDENTITY" "$archive_file" | tar -xzf - -C "$scratch_uploads"

trap - ERR

echo "Rehearsal restore completed successfully."
echo "Scratch database: $scratch_db"
echo "Scratch directory: $scratch_uploads"
echo ""
echo "To clean up:"
echo "  runuser -u postgres -- dropdb $scratch_db"
echo "  rm -rf $scratch_uploads"
