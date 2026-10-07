#!/usr/bin/env bash
# First install of the XCHANGE v3 server on this host (Ubuntu, systemd,
# PostgreSQL on the same machine). Run as a sudo-capable user, never as root
# directly. Safe to run again: every step looks before it writes, and a
# re-run keeps what it replaces as xchange.prev and configuration.prev,
# like update.sh.
#
# Nothing in this repository runs this for you. It is run on a host only on the
# owner's order (deploy/README.md).
#
# Variables: see the block at the top of common.sh; override from the
# environment, e.g.  PORT=8094 APP_ENVIRONMENT=staging ./install.sh
set -euo pipefail
# shellcheck source=deploy/server/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

[ "$(id -u)" -ne 0 ] || die "run as a sudo-capable user, not as root"
need sudo systemctl curl openssl sha256sum stat sed psql
adopt_installed_settings
check_names
check_stage xchange configuration/base.yaml extract-v2-pages.mjs
[ -f "$HERE/xc-v3.service" ] && [ -f "$HERE/smoke.sh" ] || die "xc-v3.service and smoke.sh must sit next to this script"
(cd / && sudo -u postgres psql -X -Atc 'select 1' >/dev/null) || die "PostgreSQL does not answer for the postgres superuser"

FIRST_INSTALL=0
sudo test -f "$UNIT_FILE" || FIRST_INSTALL=1

# 0. judge every directory before anything is created: a run that refuses
#    must leave nothing behind — not even the service user.
judge_dir "$INSTALL_DIR" root root 755
judge_dir "$CONFIG_DIR" root root 755
judge_dir "$ETC_DIR" root "$SERVICE_USER" 750
judge_dir "$UPLOADS_DIR" "$SERVICE_USER" "$SERVICE_USER" 700
# WEB_DIR only when step 6 will create it (missing or empty).
if ! sudo test -d "$WEB_DIR" || [ -z "$(sudo ls -A "$WEB_DIR")" ]; then
  judge_dir "$WEB_DIR" root root 755
fi

# 1. the service user: no home, no shell
if ! id "$SERVICE_USER" >/dev/null 2>&1; then
  sudo useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$SERVICE_USER"
  say "system user created: $SERVICE_USER"
fi

# 2. directories
#    ensure_dir creates what is missing (missing parents root:root 755) and
#    never changes a directory that already exists, parent or not.
ensure_dir "$INSTALL_DIR" root root 755
ensure_dir "$CONFIG_DIR" root root 755
ensure_dir "$ETC_DIR" root "$SERVICE_USER" 750
ensure_dir "$UPLOADS_DIR" "$SERVICE_USER" "$SERVICE_USER" 700
# an existing parent the service user cannot enter would lock the service
# out of its uploads; this script does not change such a directory, so say so
warn_if_unreachable "$SERVICE_USER" "$UPLOADS_DIR"

# 3. the env file, written once; the database password is generated here and
#    goes from openssl into the file without touching a command line or the screen
ROLE_EXISTS="$(pg -Atc "select 1 from pg_roles where rolname = '$DB_ROLE'")"
if ! sudo test -f "$ENV_FILE"; then
  [ -z "$ROLE_EXISTS" ] || die "role $DB_ROLE exists but $ENV_FILE does not: its password is unknown here. Restore the env file, or pick another DB_ROLE/DB_NAME, or reset the role's password by hand and write the env file from env.example."
  if sudo ss -ltnH "sport = :$PORT" 2>/dev/null | grep -q .; then
    die "port $PORT is already in use on this host; choose another PORT"
  fi
  {
    printf '%s\n' \
      "APP_ENVIRONMENT=$APP_ENVIRONMENT" \
      "APP_CONFIG_DIR=$CONFIG_DIR" \
      "APP_APPLICATION__HOST=127.0.0.1" \
      "APP_APPLICATION__PORT=$PORT" \
      "APP_DATABASE__HOST=$DB_HOST" \
      "APP_DATABASE__USERNAME=$DB_ROLE" \
      "APP_DATABASE__DATABASE_NAME=$DB_NAME" \
      "APP_WEB__DIR=$WEB_DIR" \
      "APP_FILES__DIR=$UPLOADS_DIR"
    # the leading letter keeps the value text: settings are type-parsed
    printf 'APP_DATABASE__PASSWORD=p'
    openssl rand -hex 24
  } | sudo install -o root -g "$SERVICE_USER" -m 640 /dev/stdin "$ENV_FILE"
  say "env file written: $ENV_FILE (root:$SERVICE_USER 640)"
else
  say "env file kept as it is: $ENV_FILE"
  for key in APP_WEB__DIR APP_FILES__DIR APP_DATABASE__PASSWORD APP_CONFIG_DIR; do
    sudo grep -q "^$key=" "$ENV_FILE" || echo "WARNING: $ENV_FILE has no $key (see env.example)" >&2
  done
fi

# 4. role and database of its own. The password is read from the env file
#    through a pipe; statement logging is switched off for that one session so
#    the CREATE ROLE line cannot land in PostgreSQL's log, and psql's own
#    output is discarded: an error message would quote the statement.
if [ -z "$ROLE_EXISTS" ]; then
  sudo sed -n 's/^APP_DATABASE__PASSWORD=//p' "$ENV_FILE" | tail -n 1 | grep -Eq '^[A-Za-z][A-Za-z0-9]{15,}$' ||
    die "the password in $ENV_FILE is not letters and digits only; create role $DB_ROLE by hand"
  {
    printf "SET log_statement = 'none';\nSET log_min_error_statement = 'panic';\n"
    printf "CREATE ROLE %s LOGIN PASSWORD '" "$DB_ROLE"
    sudo sed -n 's/^APP_DATABASE__PASSWORD=//p' "$ENV_FILE" | tail -n 1 | tr -d '\n'
    printf "';\n"
  } | pg >/dev/null 2>&1 ||
    die "creating role $DB_ROLE failed (psql's output is withheld: it can quote the password). Check that PostgreSQL runs and that the role name is free."
  say "database role created: $DB_ROLE"
fi
if [ -z "$(pg -Atc "select 1 from pg_database where datname = '$DB_NAME'")" ]; then
  pg -c "CREATE DATABASE $DB_NAME OWNER $DB_ROLE" >/dev/null
  say "database created: $DB_NAME (owner $DB_ROLE)"
fi

# 5. binary and configuration (root-owned; the service can read, not change)
#    The same swap as update.sh: what a re-run replaces it keeps first.
BIN_SAVED=0
CONFIG_SAVED=0
replace_binary
replace_configuration
say "binary installed: $BIN sha256 $(sha_of "$BIN")"

# 6. pages: extracted on this host from the installed v2 bundle, once.
#    To extract again after the v2 bundle changed: move $WEB_DIR away, re-run.
if ! sudo test -d "$WEB_DIR" || [ -z "$(sudo ls -A "$WEB_DIR")" ]; then
  need node
  NODE="$(command -v node)"
  [ -d "$V2_BUNDLE_DIR" ] || sudo test -d "$V2_BUNDLE_DIR" || die "v2 bundle not found: $V2_BUNDLE_DIR (set V2_BUNDLE_DIR)"
  TMP="$(mktemp -d)"
  # the extractor evaluates text taken from the bundle; run it unprivileged
  # unless the bundle can only be read by root
  if [ "$EXTRACT_AS_ROOT" = 1 ]; then
    sudo "$NODE" "$STAGE/extract-v2-pages.mjs" "$V2_BUNDLE_DIR" "$TMP/web" >/dev/null
  else
    "$NODE" "$STAGE/extract-v2-pages.mjs" "$V2_BUNDLE_DIR" "$TMP/web" >/dev/null ||
      die "extraction failed; if $V2_BUNDLE_DIR is readable by root only, re-run with EXTRACT_AS_ROOT=1"
  fi
  ensure_dir "$WEB_DIR" root root 755
  sudo cp -R "$TMP/web/." "$WEB_DIR/"
  sudo chown -R root:root "$WEB_DIR"
  sudo chmod -R a+rX,go-w "$WEB_DIR"
  sudo rm -rf "${TMP:?}"
  say "pages extracted from $V2_BUNDLE_DIR into $WEB_DIR"
else
  say "pages kept as they are: $WEB_DIR"
fi

# 7. unit, start (migrations run at start), smoke check
install_unit
sudo systemctl enable -q "$SERVICE_NAME.service"
sudo systemctl restart "$SERVICE_NAME.service"
if ! run_smoke; then
  echo "---- last lines of the service log ----" >&2
  sudo journalctl -u "$SERVICE_NAME.service" -n 20 --no-pager -o cat >&2 || true
  die "smoke check failed; the service is left as it is for inspection"
fi

say "installed. applied migrations: $(applied_migrations)"
if [ "$FIRST_INSTALL" = 1 ]; then
  cat <<TXT
Next (deploy/README.md):
  - create the first owner account (section "First owner account")
  - nginx is the host owner's to apply: deploy/nginx/*.conf.example
  - remove the stage directory when done: $STAGE
TXT
else
  # a re-run replaced what differed and kept it first; say the way back
  print_rollback "$BIN_SAVED" "$CONFIG_SAVED"
fi
