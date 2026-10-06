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
ETC_DIR="${ETC_DIR:-/etc/$SERVICE_NAME}"
# Uploaded files; the only directory the service may write.
UPLOADS_DIR="${UPLOADS_DIR:-/var/lib/$SERVICE_NAME/uploads}"
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
  for d in "$INSTALL_DIR" "$ETC_DIR" "$UPLOADS_DIR"; do
    check_own_path "$d"
  done
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

# ensure_dir <path> <owner> <group> <mode>
# Creates the directory with that owner and mode when it is missing; missing
# parents are created root:root 755. NEVER changes a directory that already
# exists — neither a parent nor the directory itself: when it exists with
# another owner or mode this stops and says so, for a person to decide.
ensure_dir() {
  local path="$1" owner="$2" group="$3" mode="$4" p missing=() i have
  if sudo test -e "$path"; then
    sudo test -d "$path" || die "$path exists and is not a directory"
    have="$(sudo stat -c '%U %G %a' "$path")"
    [ "$have" = "$owner $group $mode" ] ||
      die "$path already exists as '$have', expected '$owner $group $mode'; not changed — fix it by hand or choose another path"
    return 0
  fi
  p="$(dirname "$path")"
  while ! sudo test -e "$p"; do
    missing=("$p" "${missing[@]}")
    p="$(dirname "$p")"
  done
  sudo test -d "$p" || die "$p exists and is not a directory"
  for i in "${missing[@]}"; do
    sudo mkdir -m 755 "$i"
    sudo chown root:root "$i"
  done
  sudo mkdir -m "$mode" "$path"
  sudo chown "$owner:$group" "$path"
  sudo chmod "$mode" "$path"
}

# The stage must be ours and closed to other users, hold what is needed, and
# match the checksums written on the build host.
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
  grep -q ' [ *]\?xchange$' "$STAGE/SHA256SUMS" ||
    die "SHA256SUMS does not list the binary 'xchange'"
}

# The unit file with the six "(rendered)" lines set from the variables above.
render_unit() {
  sed \
    -e "s|^User=.*|User=$SERVICE_USER|" \
    -e "s|^Group=.*|Group=$SERVICE_USER|" \
    -e "s|^WorkingDirectory=.*|WorkingDirectory=$INSTALL_DIR|" \
    -e "s|^EnvironmentFile=.*|EnvironmentFile=$ENV_FILE|" \
    -e "s|^ExecStart=.*|ExecStart=$BIN|" \
    -e "s|^ReadWritePaths=.*|ReadWritePaths=$UPLOADS_DIR|" \
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
# says where the uploads live: the unit's ReadWritePaths must follow it, or a
# later run without the same override would lock the service out of its files.
adopt_installed_settings() {
  local v
  sudo test -f "$ENV_FILE" || return 0
  v="$(env_value APP_FILES__DIR)"
  if [ -n "$v" ] && [ "$v" != "$UPLOADS_DIR" ]; then
    say "uploads directory taken from $ENV_FILE: $v (not $UPLOADS_DIR)"
    UPLOADS_DIR="$v"
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

# print_rollback <binary-replaced 0|1> <configuration-replaced 0|1>
# Prints the commands that undo what THIS run replaced, and only for copies
# that are really on disk.
print_rollback() {
  local bin=0 conf=0
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
}
