#!/usr/bin/env bash
# Smoke check of a running XCHANGE v3 server. Changes nothing on the server.
#
#   smoke.sh <base-url> [expected-build-sha]
#
#   smoke.sh http://127.0.0.1:8094                 # on the host, straight to the server
#   smoke.sh https://desk.example.com 69f8a0a      # through nginx, and the build must match
#
# <base-url> is scheme://host[:port] with no path. It is also sent as Origin:
# the server accepts a POST only when Origin equals its own origin, which it
# builds from X-Forwarded-Proto and Host (api/src/presentation/extractors/actor.rs).
# Leave a default port (:80, :443) out of the URL.
#
# Environment:
#   SMOKE_WAIT_SECS  seconds to wait for /healthz to answer first (default 0)
#   SMOKE_BOT=1      also check that this machine reaches the Bank of Thailand
#                    page the reference rates are read from (run on the host)
#
# Exit status: 0 when every check passed, 1 otherwise, 2 on wrong usage.
set -euo pipefail

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: $0 <base-url> [expected-build-sha]" >&2
  exit 2
fi
BASE="${1%/}"
EXPECTED="${2:-}"
if ! [[ "$BASE" =~ ^https?://[^/]+$ ]]; then
  echo "base URL must be scheme://host[:port] with no path: $1" >&2
  exit 2
fi
if [ -n "$EXPECTED" ] && ! [[ "$EXPECTED" =~ ^[0-9a-f]{7,40}$ ]]; then
  echo "expected build must be 7 to 40 hex digits of the commit sha: $EXPECTED" >&2
  exit 2
fi
command -v curl >/dev/null 2>&1 || { echo "curl not found" >&2; exit 2; }

WAIT="${SMOKE_WAIT_SECS:-0}"
FAILED=0
BODY="$(mktemp)"
trap 'rm -f "$BODY"' EXIT

ok() { echo "ok    $*"; }
bad() {
  echo "FAIL  $*"
  FAILED=1
}

# status of one request; body lands in $BODY. 000 when nothing answered.
request() {
  : >"$BODY"
  curl -sS -m 10 -o "$BODY" -w '%{http_code}' "$@" 2>/dev/null || true
}

# value of a string field of a one-object JSON body
field() { sed -n 's/.*"'"$1"'":"\([^"]*\)".*/\1/p' "$BODY" | head -n 1; }

# 0. wait for the server (after a start: it applies migrations before listening)
n=0
while [ "$n" -lt "$WAIT" ]; do
  [ "$(request "$BASE/healthz")" = 200 ] && break
  n=$((n + 1))
  sleep 1
done

# 1. the process answers
code="$(request "$BASE/healthz")"
if [ "$code" = 200 ]; then ok "/healthz 200"; else bad "/healthz answered $code, want 200"; fi

# 2. ready: database reachable, and which build this is
code="$(request "$BASE/health/ready")"
version="$(field version)"
build="$(field build)"
built_at="$(field builtAt)"
channel="$(field channel)"
echo "      version=$version build=$build builtAt=$built_at channel=$channel"
if [ "$code" = 200 ]; then ok "/health/ready 200"; else bad "/health/ready answered $code, want 200"; fi
if grep -q '"database":true' "$BODY"; then ok "database: true"; else bad "database is not true"; fi
for name in version build builtAt channel; do
  [ -n "$(field "$name")" ] || bad "/health/ready has no $name"
done
ui="$(field ui)"
if [ "$ui" = "built" ]; then
  ok "ui built"
else
  bad "ui is '$ui', want built (build web/ before cargo build --release)"
fi
if [ -n "$EXPECTED" ]; then
  case "$build" in
    "$EXPECTED"*) ok "build matches $EXPECTED" ;;
    *) bad "build is '$build', expected $EXPECTED" ;;
  esac
fi

# 3. sign-in status: needs the same-origin headers the kept pages send
code="$(request -X POST "$BASE/api/auth" \
  -H 'Content-Type: application/json' \
  -H 'X-XChange-Request: 1' \
  -H "Origin: $BASE" \
  -d '{"action":"status"}')"
if [ "$code" = 200 ]; then
  ok "POST /api/auth status 200"
else
  bad "POST /api/auth status answered $code, want 200 (when refused: Origin, Host and X-Forwarded-Proto must agree)"
fi

# 4. the sign-in page is served (the pages were extracted into APP_WEB__DIR)
code="$(request "$BASE/login")"
if [ "$code" = 200 ]; then ok "/login 200"; else bad "/login answered $code, want 200 (is APP_WEB__DIR filled?)"; fi

# 5. optional: the reference-rate source is reachable from this machine
if [ "${SMOKE_BOT:-0}" = 1 ]; then
  code="$(request -L https://www.bot.or.th/th/statistics/exchange-rate.html)"
  case "$code" in
    2* | 3*) ok "Bank of Thailand page reachable ($code)" ;;
    *) bad "Bank of Thailand page answered $code" ;;
  esac
fi

if [ "$FAILED" -ne 0 ]; then
  echo "smoke check FAILED for $BASE"
  exit 1
fi
echo "smoke check passed for $BASE"
