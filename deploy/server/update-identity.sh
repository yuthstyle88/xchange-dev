#!/usr/bin/env bash
# Updates an installed xc-identity server to the identity-api binary in the
# stage directory. Configures build metadata (APP_BUILD, APP_BUILT_AT,
# APP_CHANNEL) via a systemd drop-in unit.
# Rolls back to the previous binary and drop-in configuration if /health
# does not report the expected build sha.
# Run as a sudo-capable user; only on the owner's order (deploy/HOST-do-sgp1.md).
set -euo pipefail

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

[ "$(id -u)" -ne 0 ] || die "run as a sudo-capable user, not as root"
need sudo systemctl curl sha256sum stat sed

SERVICE_NAME="${SERVICE_NAME:-xc-identity}"
INSTALL_DIR="${INSTALL_DIR:-/opt/$SERVICE_NAME}"
SYSTEMD_DIR="${SYSTEMD_DIR:-/etc/systemd/system}"
DROPIN_DIR="$SYSTEMD_DIR/$SERVICE_NAME.service.d"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:8095/health}"
SMOKE_WAIT_SECS="${SMOKE_WAIT_SECS:-30}"
APP_CHANNEL="${APP_CHANNEL:-staging}"
BUILT_AT="${BUILT_AT:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"

STAGE="${1:-${STAGE:-}}"
SHA="${2:-${SHA:-}}"

[ -n "$STAGE" ] || die "usage: $0 <stage-dir> [sha] (or set STAGE=...)"
[ -d "$STAGE" ] || die "stage directory missing: $STAGE"

[ -f "$STAGE/identity-api" ] || die "missing in stage: $STAGE/identity-api"
[ -f "$STAGE/SHA256SUMS" ] || die "missing in stage: $STAGE/SHA256SUMS"

say "verifying checksum in $STAGE..."
(cd "$STAGE" && sha256sum -c SHA256SUMS) || die "checksum mismatch in $STAGE"

if [ -z "$SHA" ]; then
  if [ -f "$STAGE/BUILD" ]; then
    SHA="$(tr -d '[:space:]' < "$STAGE/BUILD")"
  fi
fi
[ -n "$SHA" ] || die "build sha missing: specify as second argument, set SHA=..., or provide $STAGE/BUILD"

sudo mkdir -p "$INSTALL_DIR"
sudo mkdir -p "$DROPIN_DIR"

PREV_BIN=0
if sudo test -f "$INSTALL_DIR/identity-api"; then
  say "keeping identity-api.prev..."
  sudo cp -p "$INSTALL_DIR/identity-api" "$INSTALL_DIR/identity-api.prev"
  PREV_BIN=1
fi

PREV_BUILD=""
if sudo test -f "$INSTALL_DIR/BUILD"; then
  PREV_BUILD="$(sudo cat "$INSTALL_DIR/BUILD" 2>/dev/null || true)"
fi

PREV_CONF=0
if sudo test -f "$DROPIN_DIR/build.conf"; then
  sudo cp -p "$DROPIN_DIR/build.conf" "$DROPIN_DIR/build.conf.prev"
  PREV_CONF=1
fi

say "installing binary to $INSTALL_DIR/identity-api..."
sudo install -o root -g root -m 755 "$STAGE/identity-api" "$INSTALL_DIR/identity-api"

say "writing $INSTALL_DIR/BUILD ($SHA)..."
tmp_build="$(mktemp "${TMPDIR:-/tmp}/xc-id-build.XXXXXX")"
printf '%s\n' "$SHA" > "$tmp_build"
sudo install -o root -g root -m 644 "$tmp_build" "$INSTALL_DIR/BUILD"
rm -f "$tmp_build"

say "writing drop-in $DROPIN_DIR/build.conf..."
tmp_conf="$(mktemp "${TMPDIR:-/tmp}/xc-id-conf.XXXXXX")"
cat > "$tmp_conf" <<EOF
[Service]
Environment=APP_BUILD=$SHA
Environment=APP_BUILT_AT=$BUILT_AT
Environment=APP_CHANNEL=$APP_CHANNEL
EOF
sudo install -o root -g root -m 644 "$tmp_conf" "$DROPIN_DIR/build.conf"
rm -f "$tmp_conf"

say "reloading systemd and restarting $SERVICE_NAME..."
sudo systemctl daemon-reload
sudo systemctl restart "$SERVICE_NAME"

say "polling $HEALTH_URL until build equals $SHA..."
BODY="$(mktemp "${TMPDIR:-/tmp}/xc-id-health.XXXXXX")"
trap 'rm -f "$BODY"' EXIT

request() {
  : >"$BODY"
  curl -sS -m 10 -o "$BODY" "$@" >/dev/null 2>&1 || true
}

field() {
  sed -n 's/.*"'"$1"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$BODY" | head -n 1
}

ok=0
deadline=$(( $(date +%s) + SMOKE_WAIT_SECS ))
cur_build=""
while true; do
  request "$HEALTH_URL"
  cur_build="$(field build)"
  if [ "$cur_build" = "$SHA" ]; then
    ok=1
    break
  fi
  if [ "$(date +%s)" -ge "$deadline" ]; then
    break
  fi
  sleep 1
done

if [ "$ok" -ne 1 ]; then
  echo "ERROR: health check failed: build did not match $SHA within ${SMOKE_WAIT_SECS}s (got: '$cur_build')" >&2
  say "rolling back to previous state..."
  if [ "$PREV_BIN" -eq 1 ]; then
    sudo cp -p "$INSTALL_DIR/identity-api.prev" "$INSTALL_DIR/identity-api"
  fi
  if [ -n "$PREV_BUILD" ]; then
    tmp_b="$(mktemp "${TMPDIR:-/tmp}/xc-id-prev-b.XXXXXX")"
    printf '%s\n' "$PREV_BUILD" > "$tmp_b"
    sudo install -o root -g root -m 644 "$tmp_b" "$INSTALL_DIR/BUILD"
    rm -f "$tmp_b"
  else
    sudo rm -f "$INSTALL_DIR/BUILD"
  fi
  if [ "$PREV_CONF" -eq 1 ]; then
    sudo cp -p "$DROPIN_DIR/build.conf.prev" "$DROPIN_DIR/build.conf"
    sudo rm -f "$DROPIN_DIR/build.conf.prev"
  else
    sudo rm -f "$DROPIN_DIR/build.conf"
  fi
  sudo systemctl daemon-reload
  sudo systemctl restart "$SERVICE_NAME"
  die "update failed: /health build did not report $SHA within ${SMOKE_WAIT_SECS}s (rolled back to .prev)"
fi

if [ "$PREV_CONF" -eq 1 ]; then
  sudo rm -f "$DROPIN_DIR/build.conf.prev"
fi

say "update successful: $SERVICE_NAME is running build $SHA"
