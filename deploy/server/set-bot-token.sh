#!/usr/bin/env bash
# Sets the Bank of Thailand API token (APP_BOT__API_TOKEN) in the server's env
# file, so the official BoT provider can fetch reference rates. Run it on the
# server host as a sudo-capable user. The token is read from the terminal with
# echo off (or from stdin when piped) and is never printed, logged, put on a
# command line or written anywhere but the env file.
#
#   ./set-bot-token.sh             ask for the token, write it, restart the service
#   ./set-bot-token.sh --check     also call the BoT API once and print the HTTP status
#   ./set-bot-token.sh --no-restart
#   ./set-bot-token.sh --remove    delete the line (the server falls back to the scraper)
#
# The token is the "Token" of the app's credential on the BoT API portal
# (auth method authToken). The "Token Hash" is not used by xchange; never enter it.
# Rotating the credential on the portal = run this script again with the new token.
#
# Variables: see the block at the top of common.sh (ETC_DIR, SERVICE_NAME, ...).
set -euo pipefail
# shellcheck source=deploy/server/common.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

KEY="APP_BOT__API_TOKEN"
BOT_API_BASE="${BOT_API_BASE:-https://gateway.api.bot.or.th/Stat-ExchangeRate/v2}"
CHECK=0
RESTART=1
REMOVE=0
for arg in "$@"; do
  case "$arg" in
    --check) CHECK=1 ;;
    --no-restart) RESTART=0 ;;
    --remove) REMOVE=1 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) die "unknown option: $arg (see --help)" ;;
  esac
done

[ "$(id -u)" -ne 0 ] || die "run as a sudo-capable user, not as root"
need sudo systemctl sed grep
sudo test -f "$ENV_FILE" || die "$ENV_FILE not found: run install.sh first"

# Rewrites the env file without $KEY and, with "with-token", with "$KEY=$TOKEN"
# appended. Nothing touches the disk until the whole new content is built in
# memory and checked: a failed sudo read, a grep error, or a result that lost
# any other setting stops the script with the env file unchanged. The new
# content goes to `sudo install … /dev/stdin` (as install.sh does), so no temp
# file ever holds the token.
rewrite_env() {
  local current kept rc before after
  current="$(sudo cat "$ENV_FILE")" || die "cannot read $ENV_FILE (sudo failed): nothing changed"
  [ -n "$current" ] || die "$ENV_FILE is empty: refusing to rewrite it"
  rc=0
  kept="$(printf '%s\n' "$current" | grep -v "^${KEY}=")" || rc=$?
  [ "$rc" -le 1 ] || die "grep failed (exit $rc): nothing changed"
  # Count the non-blank lines other than $KEY before and after; they must match.
  before="$(printf '%s\n' "$current" | grep -v "^${KEY}=" | grep -c '[^[:space:]]' || true)"
  after="$(printf '%s\n' "$kept" | grep -c '[^[:space:]]' || true)"
  [ "$before" = "$after" ] || die "rewrite would change other settings ($before -> $after non-blank lines): nothing changed"
  if [ "${1:-}" = "with-token" ]; then
    if [ -n "$kept" ]; then kept="$kept"$'\n'; fi
    kept="$kept$KEY=$TOKEN"
  fi
  printf '%s\n' "$kept" | sudo install -o root -g "$SERVICE_USER" -m 640 /dev/stdin "$ENV_FILE"
}

if [ "$REMOVE" -eq 1 ]; then
  rewrite_env
  say "$KEY removed from $ENV_FILE"
else
  if [ -t 0 ]; then
    printf 'BoT API Token (input hidden): ' >&2
    IFS= read -rs TOKEN
    printf '\n' >&2
  else
    IFS= read -r TOKEN || true
  fi
  TOKEN="${TOKEN%$'\r'}"
  [ -n "$TOKEN" ] || die "empty token: nothing written"
  case "$TOKEN" in
    *[[:space:]]*) unset TOKEN; die "the token contains whitespace: nothing written (paste the Token only)" ;;
  esac
  LEN=${#TOKEN}
  rewrite_env with-token
  say "$KEY written to $ENV_FILE ($LEN characters, value not shown)"

  if [ "$CHECK" -eq 1 ]; then
    need curl date
    # Yesterday in Bangkok: the daily average of a business day is published at 18:00.
    DAY="$(TZ=Asia/Bangkok date -d 'yesterday' +%F 2>/dev/null || TZ=Asia/Bangkok date -v-1d +%F)"
    # The header goes through stdin (-H @-), never through argv, so ps cannot see it.
    CODE="$(printf 'Authorization: %s\n' "$TOKEN" | curl -sS -o /dev/null -w '%{http_code}' -m 15 \
      -H @- "$BOT_API_BASE/DAILY_AVG_EXG_RATE/?start_period=$DAY&end_period=$DAY&currency=USD")" || CODE="curl-failed"
    say "BoT API check for $DAY (USD): HTTP $CODE"
    case "$CODE" in
      200) : ;;
      401|403) echo "WARNING: BoT refused the token (HTTP $CODE). Check you copied the Token, not the Token Hash." >&2 ;;
      *) echo "WARNING: unexpected answer from BoT (HTTP $CODE); the server falls back to the scraper until it works." >&2 ;;
    esac
  fi
  unset TOKEN
fi

if [ "$RESTART" -eq 1 ]; then
  sudo systemctl restart "$SERVICE_NAME"
  say "$SERVICE_NAME restarted"
else
  say "not restarted: run 'sudo systemctl restart $SERVICE_NAME' for the change to take effect"
fi
