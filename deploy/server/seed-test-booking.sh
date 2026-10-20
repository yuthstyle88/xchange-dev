#!/usr/bin/env bash
# Test data for a manual end-to-end booking check on a v3 host (#1683, owner 2026-10-20:
# "สร้าง seed มาเทสก่อน"). Run by the operator on the host, as root, after install/update:
#
#   sudo deploy/server/seed-test-booking.sh https://xc-v3.spacing.cloud
#
# What it does. Every step is idempotent; a second run changes nothing it already did.
#   1. organization_hosts row for the host, so /api/public/host finds org XC (rate-display.md §7 item 2).
#   2. Branch MAIN: booking enabled, Asia/Bangkok, open 09:00–17:00 every day.
#   3. Booking slot rule for MAIN (30 min, capacity 4, horizon 14 days), owned by the active owner.
#   4. Open slots for the next SEED_DAYS days (default 7), the same SQL shape as SlotService::generate_in.
#   5. A published TERMS consent text in th, en and zh, so /app sign-up does not answer consentTextMissing.
#   6. A test teller: an Identity account (password login) bound to an ACTIVE users row with TELLER on MAIN.
#   7. A test customer: an Identity account, then POST /api/app/register (creates the CUSTOMER user and customer).
#
# Passwords are generated here and printed once, on this terminal only. They are written to no file.
# OTP cannot work on this host (Identity runs AUTH_OTP_DELIVERY_MODE=noop), so both accounts use phone + password.
#
# Needs: an ACTIVE owner already (run `xchange create-owner` first). This script never creates one.
#
# Environment (optional):
#   TELLER_PHONE   default +66800000101      CUSTOMER_PHONE  default +66800000102
#   SEED_DAYS      default 7
#
# --json: write the test users file for qa/booking-e2e to stdout and everything else to stderr, so
#   ssh do-sgp1 'bash /tmp/seed-test-booking.sh --json https://xc-v3.spacing.cloud' > users.json
# creates the file with no password on the screen. In this mode the phones default to new random
# test numbers, so every run makes accounts whose password it knows.
set -euo pipefail

JSON=0
if [ "${1:-}" = "--json" ]; then
  JSON=1
  shift
  exec 3>&1 1>&2
  rnd() { printf '%04d' $(( $(od -An -N2 -tu2 /dev/urandom) % 10000 )); }
  TELLER_PHONE="${TELLER_PHONE:-+6680001$(rnd)}"
  CUSTOMER_PHONE="${CUSTOMER_PHONE:-+6680002$(rnd)}"
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh
. "$HERE/common.sh"

if [ $# -ne 1 ] || ! [[ "$1" =~ ^https://[a-z0-9.-]+$ ]]; then
  echo "usage: $0 https://<host>   (lower-case host, no port, no path)" >&2
  exit 2
fi
BASE="$1"
HOST="${BASE#https://}"
TELLER_PHONE="${TELLER_PHONE:-+66800000101}"
CUSTOMER_PHONE="${CUSTOMER_PHONE:-+66800000102}"
SEED_DAYS="${SEED_DAYS:-7}"
ORG_ID="00000000-0000-0000-0000-000000000001"   # org XC, seeded by 20261011100000_organizations.sql
BRANCH="MAIN"                                     # seeded by 20261007100000_branches_rates.sql, code = id

need curl openssl sudo
DB="$(env_value APP_DATABASE__DATABASE_NAME)"
[ -n "$DB" ] || die "APP_DATABASE__DATABASE_NAME not found in $ENV_FILE"
q() { pg -d "$DB" -tA -c "$1"; }

# Identity client settings, the same file the service reads (xc-v3.service EnvironmentFile).
IDENTITY_ENV="$ETC_DIR/identity-client.env"
sudo test -f "$IDENTITY_ENV" || die "$IDENTITY_ENV not found"
idv() { sudo sed -n "s/^$1=//p" "$IDENTITY_ENV" | tail -n 1; }
ID_URL="$(idv IDENTITY_BASE_URL)"; ID_CLIENT="$(idv IDENTITY_CLIENT_ID)"
ID_SECRET="$(idv IDENTITY_CLIENT_SECRET)"; ID_KEY="$(idv IDENTITY_API_KEY)"
[ -n "$ID_URL" ] && [ -n "$ID_CLIENT" ] && [ -n "$ID_SECRET" ] || die "IDENTITY_BASE_URL/CLIENT_ID/CLIENT_SECRET missing in $IDENTITY_ENV"

# A password that starts with a letter and has upper, lower and digits.
new_password() { echo "Xc$(openssl rand -hex 6)A9"; }

# One curl config line on stdout (`curl -K -` reads them from stdin). Secrets and passwords go to curl this way,
# never as arguments, so they are not visible in `ps` / /proc/<pid>/cmdline; printf is a builtin, so neither is it.
cfg() { local v=${2//\\/\\\\}; v=${v//\"/\\\"}; printf '%s = "%s"\n' "$1" "$v"; }

# Registers <phone> with Identity (password login, no forced change). Prints the identityId,
# or nothing when the phone is already registered (its password is then unknown here).
identity_register() {
  local phone="$1" password="$2" out
  out=$({ cfg url "${ID_URL%/}/auth/register"; cfg header 'Content-Type: application/json'
           cfg header "x-client-id: $ID_CLIENT"; cfg header "x-client-secret: $ID_SECRET"
           if [ -n "$ID_KEY" ]; then cfg header "x-api-key: $ID_KEY"; fi
           cfg data "{\"phone\":\"$phone\",\"password\":\"$password\",\"mustChangePassword\":false}"
         } | curl -sS -X POST -K -) || die "Identity unreachable at $ID_URL"
  case "$out" in
    *phone_taken*) return 0 ;;
    *identityId*) sed -n 's/.*"identityId":"\([^"]*\)".*/\1/p' <<<"$out" ;;
    *) die "Identity register for $phone failed: $(sed -E 's/"(password|token)[^,}]*//g' <<<"$out" | head -c 300)" ;;
  esac
}

say "1. host $HOST → org XC"
q "INSERT INTO organization_hosts (host, organization_id) VALUES ('$HOST', '$ORG_ID') ON CONFLICT (host) DO NOTHING;" >/dev/null

OWNER_ID="$(q "SELECT u.id FROM users u
  WHERE u.organization_id = '$ORG_ID' AND u.status = 'ACTIVE'
    AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL AND r.valid_to IS NULL)
    AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'ADMIN' AND r.branch_id IS NULL AND r.valid_to IS NULL)
  ORDER BY u.created_at LIMIT 1;")"
[ -n "$OWNER_ID" ] || die "no ACTIVE owner. Create one first: $BIN create-owner --name <name> --phone <phone> (password on stdin)"

say "2. branch $BRANCH: booking on, Asia/Bangkok, 09:00–17:00 daily"
HOURS='{"mon":[{"open":"09:00","close":"17:00"}],"tue":[{"open":"09:00","close":"17:00"}],"wed":[{"open":"09:00","close":"17:00"}],"thu":[{"open":"09:00","close":"17:00"}],"fri":[{"open":"09:00","close":"17:00"}],"sat":[{"open":"09:00","close":"17:00"}],"sun":[{"open":"09:00","close":"17:00"}]}'
q "UPDATE branches SET booking_enabled = true, timezone = 'Asia/Bangkok', opening_hours = '$HOURS'::jsonb
   WHERE organization_id = '$ORG_ID' AND id = '$BRANCH';" >/dev/null

say "3. slot rule for $BRANCH (30 min, capacity 4, 14 days)"
q "INSERT INTO booking_slot_rules (organization_id, branch_id, slot_minutes, capacity, horizon_days, updated_by)
   VALUES ('$ORG_ID', '$BRANCH', 30, 4, 14, '$OWNER_ID')
   ON CONFLICT (branch_id) DO NOTHING;" >/dev/null

say "4. open slots for the next $SEED_DAYS days"
q "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status)
   SELECT gen_random_uuid(), b.organization_id, b.id, d.day,
          (d.day + s.t) AT TIME ZONE b.timezone, (d.day + s.t + interval '30 minutes') AT TIME ZONE b.timezone, 4, 0, 'OPEN'
   FROM branches b
   CROSS JOIN (SELECT ((now() AT TIME ZONE 'Asia/Bangkok')::date + g) AS day FROM generate_series(1, $SEED_DAYS) g) d
   CROSS JOIN (SELECT time '09:00' + n * interval '30 minutes' AS t FROM generate_series(0, 15) n) s
   WHERE b.organization_id = '$ORG_ID' AND b.id = '$BRANCH'
     AND NOT EXISTS (SELECT 1 FROM branch_closures c WHERE c.branch_id = b.id AND c.closed_on = d.day)
   ON CONFLICT (branch_id, start_at) DO NOTHING;" >/dev/null
echo "    open slots on $BRANCH from tomorrow: $(q "SELECT count(*) FROM booking_slots WHERE branch_id = '$BRANCH' AND status = 'OPEN' AND start_at > now();")"

say "5. TERMS consent text (th, en, zh)"
q "INSERT INTO consent_texts (organization_id, purpose, version, lang, body, published_by, published_at) VALUES
   ('$ORG_ID', 'TERMS', 1, 'th', 'ข้อตกลงการใช้บริการ (ฉบับทดสอบ)', '$OWNER_ID', now()),
   ('$ORG_ID', 'TERMS', 1, 'en', 'Terms of service (test version)', '$OWNER_ID', now()),
   ('$ORG_ID', 'TERMS', 1, 'zh', '服务条款（测试版）', '$OWNER_ID', now())
   ON CONFLICT DO NOTHING;" >/dev/null

say "6. test teller $TELLER_PHONE (TELLER on $BRANCH)"
TELLER_PW="$(new_password)"
TELLER_SUB="$(identity_register "$TELLER_PHONE" "$TELLER_PW")"
if [ -n "$TELLER_SUB" ]; then
  q "WITH u AS (
       INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
       VALUES (gen_random_uuid(), '$ORG_ID', '$TELLER_SUB', 'Test Teller', 'ACTIVE', 1, now(), '$OWNER_ID')
       ON CONFLICT (auth_subject) DO NOTHING RETURNING id)
     INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
     SELECT id, 'TELLER', '$ORG_ID', '$BRANCH' FROM u;" >/dev/null
  [ "$JSON" = 1 ] || echo "    teller created: $TELLER_PHONE  password $TELLER_PW"
else
  TELLER_PW="(unchanged: $TELLER_PHONE was already registered)"
fi

say "7. test customer $CUSTOMER_PHONE"
CUSTOMER_PW="$(new_password)"
if [ -n "$(identity_register "$CUSTOMER_PHONE" "$CUSTOMER_PW")" ]; then
  [ "$JSON" = 1 ] || echo "    customer Identity account created: $CUSTOMER_PHONE  password $CUSTOMER_PW"
  resp=$({ cfg url "$BASE/api/app/register"; cfg header 'Content-Type: application/json'
            cfg header "Origin: $BASE"; cfg header 'X-Xchange-Request: 1'
            cfg data "{\"identifier\":\"$CUSTOMER_PHONE\",\"password\":\"$CUSTOMER_PW\",\"displayName\":\"Test Customer\",\"lang\":\"th\",\"marketing\":[]}"
          } | curl -sS -w '\n%{http_code}' -X POST -K -)
  code=$(tail -n 1 <<<"$resp")
  # the answer never echoes the identifier, password or a token (customer_register.rs:14)
  [ "$code" = 200 ] || die "POST $BASE/api/app/register answered $code: $(sed '$d' <<<"$resp" | head -c 400)"
else
  CUSTOMER_PW="(unchanged: $CUSTOMER_PHONE was already registered)"
fi

if [ "$JSON" = 1 ]; then
  case "$TELLER_PW$CUSTOMER_PW" in *unchanged*) die "an account already existed, so its password is unknown; run again (new random phones)";; esac
  cat >&3 <<JSON
{
  "baseUrl": "$BASE",
  "customer": { "phone": "$CUSTOMER_PHONE", "password": "$CUSTOMER_PW" },
  "teller":   { "phone": "$TELLER_PHONE", "password": "$TELLER_PW" },
  "admin":    { "identifier": "<the owner's sign-in>", "password": "<the owner's password>" },
  "referralCode": "TEST1234",
  "branch": "$BRANCH",
  "currency": "USD",
  "quantity": "100"
}
JSON
  say "users.json written to stdout (teller $TELLER_PHONE, customer $CUSTOMER_PHONE)"
  exit 0
fi

echo
echo "Done. Test accounts (shown once, not saved anywhere):"
echo "  teller    $TELLER_PHONE   $TELLER_PW    sign in at $BASE/login"
echo "  customer  $CUSTOMER_PHONE   $CUSTOMER_PW    sign in at $BASE/login?next=/app/book (the /app sign-in page is OTP-only)"
echo "  check:    curl -s $BASE/api/public/host   → {\"org\":\"XC\"}"
