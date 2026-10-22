#!/usr/bin/env bash
# Variables and helpers shared by install.sh and update.sh. Sourced, not run.
# Every variable can be overridden from the environment:
#   PORT=8095 DB_NAME=xc_v3_b ./install.sh
# shellcheck shell=bash

# ---- variables -------------------------------------------------------------
# Name of the unit, the system user/group and the directories derived below.
SERVICE_NAME="${SERVICE_NAME:-xc-v3}"
SERVICE_USER="${SERVICE_USER:-$SERVICE_NAME}"
# Port on 127.0.0.1 (first install only; later runs read it from the env file).
PORT="${PORT:-8094}"
# development | staging | production (first install only).
APP_ENVIRONMENT="${APP_ENVIRONMENT:-staging}"
# Binary, configuration/ and web/ live here (root-owned, read-only to the service).
INSTALL_DIR="${INSTALL_DIR:-/opt/$SERVICE_NAME}"
# The env file lives here (root:<service group>, 750; the file 640).
# The override is a test seam. On a host it must stay /etc/xc-v3, because
# the binary reads /etc/xc-v3/pii.key (api/src/bootstrap/mod.rs:177, #1084).
ETC_DIR="${ETC_DIR:-/etc/$SERVICE_NAME}"
# Uploaded files and sealed verification documents; the only directories the service may write.
UPLOADS_DIR="${UPLOADS_DIR:-/var/lib/$SERVICE_NAME/uploads}"
VERIFICATION_STORE_DIR="${VERIFICATION_STORE_DIR:-/var/lib/$SERVICE_NAME/verification-documents}"
# PostgreSQL role and database (role owns the database).
DB_ROLE="${DB_ROLE:-xc_v3}"
DB_NAME="${DB_NAME:-xc_v3}"
DB_HOST="${DB_HOST:-127.0.0.1}"
# The installed v2 bundle on this host (server.mjs + public/); pages are
# extracted from it here and are never copied to another machine or this repo.
V2_BUNDLE_DIR="${V2_BUNDLE_DIR:-/opt/xc-desk-v2}"
# Set to 1 when the v2 bundle is readable by root only.
EXTRACT_AS_ROOT="${EXTRACT_AS_ROOT:-0}"
# Directory holding what the build host produced (see build.md):
#   xchange  configuration/  extract-v2-pages.mjs  SHA256SUMS  [BUILD]
# Default: the directory these scripts are in.
STAGE="${STAGE:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
# Seconds the smoke check waits for the server to answer after a (re)start.
SMOKE_WAIT_SECS="${SMOKE_WAIT_SECS:-30}"
# ---------------------------------------------------------------------------

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="$ETC_DIR/env"
UNIT_FILE="/etc/systemd/system/$SERVICE_NAME.service"
BIN="$INSTALL_DIR/xchange"
CONFIG_DIR="$INSTALL_DIR/configuration"
# shellcheck disable=SC2034  # used by install.sh
WEB_DIR="$INSTALL_DIR/web"

die() {
  echo "ERROR: $*" >&2
  exit 1
}

say() { echo "==> $*"; }

need() {
  local c
  for c in "$@"; do
    command -v "$c" >/dev/null 2>&1 || die "command not found: $c"
  done
}

# psql as the postgres superuser; from / so it never complains about the cwd.
pg() { (cd / && sudo -u postgres psql -v ON_ERROR_STOP=1 -X -q "$@"); }

check_names() {
  local n
  for n in "$DB_ROLE" "$DB_NAME"; do
    [[ "$n" =~ ^[a-z_][a-z0-9_]{0,62}$ ]] || die "not a plain PostgreSQL name: $n"
  done
  [[ "$SERVICE_NAME" =~ ^[a-z][a-z0-9-]*$ ]] || die "bad SERVICE_NAME: $SERVICE_NAME"
  [[ "$PORT" =~ ^[0-9]+$ ]] || die "bad PORT: $PORT"
  local d
  for d in "$INSTALL_DIR" "$ETC_DIR" "$UPLOADS_DIR" "$VERIFICATION_STORE_DIR"; do
    check_own_path "$d"
  done
  if [ "$VERIFICATION_STORE_DIR" = "$UPLOADS_DIR" ] || [[ "$VERIFICATION_STORE_DIR" == "$UPLOADS_DIR"/* ]] || [[ "$UPLOADS_DIR" == "$VERIFICATION_STORE_DIR"/* ]]; then
    die "VERIFICATION_STORE_DIR ($VERIFICATION_STORE_DIR) and UPLOADS_DIR ($UPLOADS_DIR) must be separate directories (never under files.dir)"
  fi
  case "$APP_ENVIRONMENT" in
    development | staging | production) ;;
    *) die "APP_ENVIRONMENT must be development, staging or production" ;;
  esac
}

# A directory this installation creates and owns: a plain absolute path, no
# "." or ".." component, no doubled or trailing slash, at least two levels deep,
# and never a directory the system or other services live in.
check_own_path() {
  local d="$1"
  [[ "$d" =~ ^/[A-Za-z0-9._/-]+$ ]] || die "not a plain absolute path: $d"
  [[ "$d" =~ (^|/)\.\.?(/|$) ]] && die "path must not contain . or .. components: $d"
  [[ "$d" == *//* || "$d" == */ ]] && die "path must not have a doubled or trailing slash: $d"
  [[ "$d" =~ ^/[^/]+/[^/]+ ]] || die "refusing a top-level directory: $d"
  case "$d" in
    /var/lib | /var/log | /var/www | /var/backups | /var/cache | /var/tmp | /usr/local | /usr/lib | /usr/bin | /usr/share | /etc/systemd | /etc/nginx | /home/* | /root/* | /tmp/* | /proc/* | /sys/* | /dev/* | /run/* | /boot/*)
      die "refusing a system or shared directory: $d"
      ;;
  esac
  return 0
}

# judge_dir <path> <owner> <group> <mode>
# The read-only half of ensure_dir: judges what ensure_dir would judge, but
# creates nothing — so install.sh can refuse before it has created anything
# at all (including the service user). An existing path must be a directory
# with exactly this owner, group and mode; a missing path's nearest existing
# ancestor must be a directory.
judge_dir() {
  local path="$1" owner="$2" group="$3" mode="$4" p have
  if sudo test -e "$path"; then
    sudo test -d "$path" || die "$path exists and is not a directory"
    have="$(sudo stat -c '%U %G %a' "$path")"
    [ "$have" = "$owner $group $mode" ] ||
      die "$path already exists as '$have', expected '$owner $group $mode'; not changed — fix it by hand or choose another path"
    return 0
  fi
  p="$(dirname "$path")"
  while ! sudo test -e "$p"; do
    p="$(dirname "$p")"
  done
  sudo test -d "$p" || die "$p exists and is not a directory"
}

# ensure_dir <path> <owner> <group> <mode>
# Creates the directory with that owner and mode when it is missing; missing
# parents are created root:root 755. NEVER changes a directory that already
# exists — neither a parent nor the directory itself: when it exists with
# another owner or mode this stops and says so, for a person to decide.
ensure_dir() {
  local path="$1" owner="$2" group="$3" mode="$4" p missing=() i
  judge_dir "$path" "$owner" "$group" "$mode"
  sudo test -e "$path" && return 0
  p="$(dirname "$path")"
  while ! sudo test -e "$p"; do
    missing=("$p" "${missing[@]}")
    p="$(dirname "$p")"
  done
  for i in "${missing[@]}"; do
    sudo mkdir -m 755 "$i"
    sudo chown root:root "$i"
  done
  sudo mkdir -m "$mode" "$path"
  sudo chown "$owner:$group" "$path"
  sudo chmod "$mode" "$path"
}

# listed_in_sums <name>
# Whether SHA256SUMS carries <name> by exact name: the name field of a line
# (everything after "<64 hex><space><space-or-star>") must equal <name>, so a
# merely similar name — old/xchange, xchange.bak, "old xchange" — never
# counts, and a binary-mode "hash *name" line does.
listed_in_sums() {
  awk -v want="$1" '
    substr($0, 67) == want { found = 1 }
    END { exit(found ? 0 : 1) }
  ' "$STAGE/SHA256SUMS"
}

# The stage must be ours and closed to other users, hold what is needed, and
# match the checksums written on the build host: sha256sum -c checks only the
# files SHA256SUMS lists, so every file named here and every staged
# configuration/*.yaml must be listed too, each by its exact name — anything
# the stage holds but does not list is never compared.
check_stage() {
  [ -d "$STAGE" ] || die "stage directory missing: $STAGE"
  local owner mode
  owner="$(stat -c '%U' "$STAGE")"
  mode="$(stat -c '%a' "$STAGE")"
  [ "$owner" = "$(id -un)" ] || [ "$owner" = root ] ||
    die "stage $STAGE belongs to $owner, not to $(id -un)"
  case "$mode" in
    *[2367][0-7] | *[2367]) die "stage $STAGE is writable by group or others (mode $mode): chmod 700 it" ;;
  esac
  local f
  for f in "$@" SHA256SUMS; do
    [ -e "$STAGE/$f" ] || die "missing in stage: $STAGE/$f"
  done
  (cd "$STAGE" && sha256sum --quiet -c SHA256SUMS) ||
    die "checksum mismatch in $STAGE — copy the files again"
  for f in "$@"; do
    listed_in_sums "$f" || die "SHA256SUMS does not list $f"
  done
  for f in "$STAGE"/configuration/*.yaml; do
    [ -e "$f" ] || continue
    listed_in_sums "configuration/$(basename "$f")" ||
      die "SHA256SUMS does not list configuration/$(basename "$f")"
  done
}

# can_enter_as <user> <dir> — whether <user> can cross <dir> (execute on it).
can_enter_as() { sudo -u "$1" test -x "$2"; }

# warn_if_unreachable <user> <path> — walks the ancestors of <path> from /
# down and, at the first one <user> cannot enter, prints one WARNING line
# naming it: the service could not reach <path> through it, and this script
# does not change directories it did not create. Returns 0 in every case —
# it warns, it never stops the install.
warn_if_unreachable() {
  local user="$1" path="$2" d
  local -a chain=()
  d="$path"
  while [ "$d" != / ]; do
    d="$(dirname "$d")"
    chain=("$d" "${chain[@]}")
  done
  for d in "${chain[@]}"; do
    if ! can_enter_as "$user" "$d"; then
      echo "WARNING: $user cannot enter $d, so the service cannot reach $path; this script does not change $d" >&2
      return 0
    fi
  done
  return 0
}

# The unit file with the six "(rendered)" lines set from the variables above.
render_unit() {
  sed \
    -e "s|^User=.*|User=$SERVICE_USER|" \
    -e "s|^Group=.*|Group=$SERVICE_USER|" \
    -e "s|^WorkingDirectory=.*|WorkingDirectory=$INSTALL_DIR|" \
    -e "s|^EnvironmentFile=/etc/xc-v3/env$|EnvironmentFile=$ENV_FILE|" \
    -e "s|^ExecStart=.*|ExecStart=$BIN|" \
    -e "s|^ReadWritePaths=.*|ReadWritePaths=$UPLOADS_DIR $VERIFICATION_STORE_DIR|" \
    "$HERE/xc-v3.service"
}

# Installs the unit when it differs from what is there. Prints nothing secret.
install_unit() {
  local tmp
  tmp="$(mktemp)"
  render_unit >"$tmp"
  if ! sudo cmp -s "$tmp" "$UNIT_FILE"; then
    sudo install -o root -g root -m 644 "$tmp" "$UNIT_FILE"
    sudo systemctl daemon-reload
    say "unit written: $UNIT_FILE"
  fi
  rm -f "$tmp"
}

# One value of the env file (never call this for the password).
env_value() { sudo sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1; }

# On a host that is already installed the env file, not this run's variables,
# says where the uploads and verification documents live: the unit's
# ReadWritePaths must follow it, or a later run without the same override would
# lock the service out of its files.
adopt_installed_settings() {
  local v
  sudo test -f "$ENV_FILE" || return 0
  v="$(env_value APP_FILES__DIR)"
  if [ -n "$v" ] && [ "$v" != "$UPLOADS_DIR" ]; then
    say "uploads directory taken from $ENV_FILE: $v (not $UPLOADS_DIR)"
    UPLOADS_DIR="$v"
  fi
  v="$(env_value APP_VERIFICATION__STORE_DIR)"
  if [ -n "$v" ] && [ "$v" != "$VERIFICATION_STORE_DIR" ]; then
    say "verification store directory taken from $ENV_FILE: $v (not $VERIFICATION_STORE_DIR)"
    VERIFICATION_STORE_DIR="$v"
  fi
}

installed_port() {
  local p
  p="$(env_value APP_APPLICATION__PORT)"
  echo "${p:-$PORT}"
}

applied_migrations() {
  pg -d "$DB_NAME" -Atc "select count(*) from _sqlx_migrations where success" 2>/dev/null || echo 0
}

sha_of() { sudo sha256sum "$1" | cut -d' ' -f1; }

# replace_binary — installs the stage's binary as $BIN and keeps the
# installed one as $BIN.prev when it differs; nothing is replaced when it
# is already the same binary (the real .prev is kept, not overwritten with
# itself); with no installed binary it just installs. Sets BIN_SAVED=1 only
# when a copy was saved.
replace_binary() {
  local new old
  if ! sudo test -e "$BIN"; then
    sudo install -o root -g root -m 755 "$STAGE/xchange" "$BIN"
    BIN_SAVED=0
    say "binary installed: $BIN (there was no previous one)"
    return 0
  fi
  new="$(sha256sum "$STAGE/xchange" | cut -d' ' -f1)"
  old="$(sha_of "$BIN")"
  if [ "$new" = "$old" ]; then
    # same binary again: keep the real previous one, do not overwrite it with itself
    say "binary unchanged ($new); xchange.prev kept"
    BIN_SAVED=0
  else
    sudo cp -p "$BIN" "$BIN.prev"
    # shellcheck disable=SC2034  # read by install.sh after the call
    BIN_SAVED=1
    sudo install -o root -g root -m 755 "$STAGE/xchange" "$BIN"
    say "binary replaced: $old -> $new (previous kept as $BIN.prev)"
  fi
}

# replace_configuration — makes $CONFIG_DIR mirror the stage's configuration/
# both ways: every staged *.yaml installed, every installed *.yaml the stage
# no longer carries removed. What is replaced is kept first as
# $CONFIG_DIR.prev; when the installed set and contents already match the
# stage's, nothing is touched and the real .prev is kept. Sets
# CONFIG_SAVED=1 only when a copy was saved.
replace_configuration() {
  local f installed=0 changed=0
  for f in "$CONFIG_DIR"/*.yaml; do
    [ -e "$f" ] && installed=1
  done
  if [ "$installed" = 0 ]; then
    sudo install -o root -g root -m 644 "$STAGE"/configuration/*.yaml "$CONFIG_DIR/"
    CONFIG_SAVED=0
    say "configuration installed: $CONFIG_DIR (there was no previous one)"
    return 0
  fi
  for f in "$STAGE"/configuration/*.yaml; do
    [ -e "$f" ] || die "no *.yaml in $STAGE/configuration"
    sudo cmp -s "$f" "$CONFIG_DIR/$(basename "$f")" || changed=1
  done
  for f in "$CONFIG_DIR"/*.yaml; do
    [ -e "$f" ] || continue
    [ -e "$STAGE/configuration/$(basename "$f")" ] || changed=1
  done
  if [ "$changed" = 0 ]; then
    say "configuration unchanged; configuration.prev kept"
    CONFIG_SAVED=0
    return 0
  fi
  sudo rm -rf "${CONFIG_DIR:?}.prev"
  sudo cp -a "$CONFIG_DIR" "$CONFIG_DIR.prev"
  # shellcheck disable=SC2034  # read by install.sh after the call
  CONFIG_SAVED=1
  sudo install -o root -g root -m 644 "$STAGE"/configuration/*.yaml "$CONFIG_DIR/"
  for f in "$CONFIG_DIR"/*.yaml; do
    [ -e "$f" ] || continue
    [ -e "$STAGE/configuration/$(basename "$f")" ] || sudo rm -f "$f"
  done
  say "configuration replaced (previous kept as $CONFIG_DIR.prev)"
}

# Build sha the stage says it carries (file BUILD, written on the build host).
expected_build() {
  if [ -n "${EXPECTED_BUILD:-}" ]; then
    echo "$EXPECTED_BUILD"
  elif [ -f "$STAGE/BUILD" ]; then
    tr -d '[:space:]' <"$STAGE/BUILD"
  fi
}

run_smoke() {
  local build
  build="$(expected_build)"
  # shellcheck disable=SC2086  # $build is empty or one sha, on purpose unquoted
  SMOKE_WAIT_SECS="$SMOKE_WAIT_SECS" bash "$HERE/smoke.sh" "http://127.0.0.1:$(installed_port)" $build
}

# newest_backup_set <dir>
# Prints the newest backup set timestamp (YYYYMMDDTHHMMSSZ) in <dir>, or empty.
newest_backup_set() {
  local dir="$1"
  sudo ls -1 "$dir" 2>/dev/null | grep -Ex '[0-9]{8}T[0-9]{6}Z' | sort | tail -n 1 || true
}

# print_rollback <binary-replaced 0|1> <configuration-replaced 0|1> [backup-set-dir]
# Prints the commands that undo what THIS run replaced, and only for copies
# that are really on disk.
print_rollback() {
  local bin=0 conf=0 set_dir="${3:-}"
  [ "${1:-0}" = 1 ] && sudo test -f "$BIN.prev" && bin=1
  [ "${2:-0}" = 1 ] && sudo test -d "$CONFIG_DIR.prev" && conf=1
  if [ "$bin" = 0 ] && [ "$conf" = 0 ]; then
    echo "Rollback: this run replaced neither the binary nor the configuration; there is nothing to put back."
    return 0
  fi
  echo "Rollback of what this run replaced (enough only while no new migration was"
  echo "applied — an older binary refuses to start on a newer schema):"
  echo "  sudo systemctl stop $SERVICE_NAME"
  [ "$bin" = 1 ] && echo "  sudo install -o root -g root -m 755 $BIN.prev $BIN"
  [ "$conf" = 1 ] && echo "  sudo rm -rf $CONFIG_DIR && sudo cp -a $CONFIG_DIR.prev $CONFIG_DIR"
  echo "  sudo systemctl start $SERVICE_NAME"
  echo "  bash $HERE/smoke.sh http://127.0.0.1:$(installed_port)"
  if [ -n "$set_dir" ]; then
    echo
    echo "To restore the database (and uploads) from the backup taken before this update:"
    echo "  Rehearsal: sudo XC_BACKUP_IDENTITY=<identity> /opt/xc-v3/restore.sh $set_dir <scratch-db> <scratch-dir>"
    echo "  Real restore: see deploy/README.md §5 \"Real restore\""
  fi
}
