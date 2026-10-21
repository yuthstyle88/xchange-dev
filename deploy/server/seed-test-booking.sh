#!/usr/bin/env bash
# Test data for a manual end-to-end booking check on a v3 host (#1683, owner 2026-10-20:
# "สร้าง seed มาเทสก่อน"). Run by the operator on the host, as root, after install/update:
#
#   sudo deploy/server/seed-test-booking.sh https://xc-v3.spacing.cloud
#
# What it does. Every step is idempotent; a second run changes nothing it already did.
#   1. organization_hosts row for the host, so /api/public/host finds org XC (rate-display.md §7 item 2).
#   2. Branch MAIN: booking enabled, Asia/Bangkok, open 00:00–23:59 every day.
#   2b. Branch TEST2 ("Test branch 2"): booking off, Asia/Bangkok, open 00:00–23:59 every day.
#   3. Booking slot rule for MAIN (30 min, capacity 4, horizon 14 days), owned by the active owner.
#   4. Open slots for the next SEED_DAYS days (default 7), the same SQL shape as SlotService::generate_in.
#   5. A published TERMS consent text in th, en and zh, so /app sign-up does not answer consentTextMissing.
#   5b. A published rate set for MAIN (USD and EUR, manual), only when current_publication_id is NULL.
#   5c. A cash-transfer policy row for org XC (THB limit 200,000), only when the organisation has none.
#   5d. rate_alert_settings for org XC (cooldown 3600s, max 10 active, 30 days, ALL_FRESH), only when none.
#   5e. A published MARKETING consent text in th, en and zh, so consent toggles do not answer consentTextMissing.
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
# --json: write the test users file for qa/e2e to stdout and everything else to stderr, so
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
  MANAGER_PHONE="${MANAGER_PHONE:-+6680003$(rnd)}"
  FINANCE_PHONE="${FINANCE_PHONE:-+6680004$(rnd)}"
  MARKETING_PHONE="${MARKETING_PHONE:-+6680005$(rnd)}"
  MANAGER2_PHONE="${MANAGER2_PHONE:-+6680006$(rnd)}"
  FINANCE2_PHONE="${FINANCE2_PHONE:-+6680007$(rnd)}"
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
MANAGER_PHONE="${MANAGER_PHONE:-+66800000103}"
FINANCE_PHONE="${FINANCE_PHONE:-+66800000104}"
MARKETING_PHONE="${MARKETING_PHONE:-+66800000105}"
MANAGER2_PHONE="${MANAGER2_PHONE:-+66800000106}"
FINANCE2_PHONE="${FINANCE2_PHONE:-+66800000107}"
CUSTOMER_PHONE="${CUSTOMER_PHONE:-+66800000102}"
SEED_DAYS="${SEED_DAYS:-7}"
ORG_ID="00000000-0000-0000-0000-000000000001"   # org XC, seeded by 20261011100000_organizations.sql
BRANCH="MAIN"                                     # seeded by 20261007100000_branches_rates.sql, code = id
BRANCH2="TEST2"                                   # second test branch for branch-to-branch transfers (F12b)

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

say "2. branch $BRANCH: booking on, Asia/Bangkok, 00:00–23:59 daily"
# All-day test hours so e2e can run at any hour (#4315, owner 2026-10-21: "ปรับ เวลา ทำการใหม่ได้ นะ" · "เพื่อให้มันเทสได้").
HOURS='{"mon":[{"open":"00:00","close":"23:59"}],"tue":[{"open":"00:00","close":"23:59"}],"wed":[{"open":"00:00","close":"23:59"}],"thu":[{"open":"00:00","close":"23:59"}],"fri":[{"open":"00:00","close":"23:59"}],"sat":[{"open":"00:00","close":"23:59"}],"sun":[{"open":"00:00","close":"23:59"}]}'
q "UPDATE branches SET booking_enabled = true, timezone = 'Asia/Bangkok', opening_hours = '$HOURS'::jsonb
   WHERE organization_id = '$ORG_ID' AND id = '$BRANCH';" >/dev/null

say "2b. branch $BRANCH2: booking off, Asia/Bangkok, 00:00–23:59 daily"
q "INSERT INTO branches (id, organization_id, name, created_at, created_by, request_id, code, timezone, opening_hours, booking_enabled)
   VALUES ('$BRANCH2', '$ORG_ID', 'Test branch 2', now(), '$OWNER_ID', 'seed_test_branch_$BRANCH2', '$BRANCH2', 'Asia/Bangkok', '$HOURS'::jsonb, false)
   ON CONFLICT (id) DO UPDATE SET
     organization_id = EXCLUDED.organization_id,
     name = EXCLUDED.name,
     code = EXCLUDED.code,
     timezone = EXCLUDED.timezone,
     opening_hours = EXCLUDED.opening_hours,
     booking_enabled = EXCLUDED.booking_enabled;" >/dev/null

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

say "5b. published rate set for $BRANCH (USD, EUR)"
CUR_PUB="$(q "SELECT current_publication_id FROM branches WHERE organization_id = '$ORG_ID' AND id = '$BRANCH';")"
if [ -z "$CUR_PUB" ]; then
  REQ_ID="$(q "SELECT gen_random_uuid();")"
  PUB_ID="$(q "SELECT gen_random_uuid();")"
  q "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id,
       object_version_hash, policy_version, proposed_payload, requested_by, requested_at,
       status, idempotency_key)
     SELECT '$REQ_ID', organization_id, 'RATE_PUBLICATION', id, '$PUB_ID', 'test',
       '1', '{}'::jsonb, '$OWNER_ID', now(), 'APPROVED', 'seed_test_rates_$BRANCH'
     FROM branches WHERE organization_id = '$ORG_ID' AND id = '$BRANCH';
     INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at,
       approved_request_id, draft_id, published_by)
     SELECT '$PUB_ID', b.organization_id, b.id,
       COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1,
       now(), '$REQ_ID', NULL, '$OWNER_ID'
     FROM branches b WHERE b.organization_id = '$ORG_ID' AND b.id = '$BRANCH';
     INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code,
       denomination_group, value_tier, unit_size, buy_rate, sell_rate, source_snapshot)
     SELECT gen_random_uuid(), p.organization_id, p.id, c.code, 'ALL', 'ALL', c.unit_size,
       v.buy * c.unit_size, v.sell * c.unit_size,
       jsonb_build_object('currency', v.code, 'buy_mode', 'MANUAL', 'sell_mode', 'MANUAL', 'reason', 'seeded for the qa/e2e suite')
     FROM rate_publications p
     CROSS JOIN (VALUES ('USD', 32.10, 32.60), ('EUR', 35.00, 35.60)) AS v(code, buy, sell)
     JOIN currencies c ON c.code = v.code
     WHERE p.id = '$PUB_ID';
     UPDATE branches SET current_publication_id = '$PUB_ID'
     WHERE organization_id = '$ORG_ID' AND id = '$BRANCH';" >/dev/null
  echo "    published rate set $PUB_ID (USD 32.10/32.60, EUR 35.00/35.60)"
else
  echo "    already present ($CUR_PUB)"
fi

say "5c. cash-transfer policy for org XC"
CUR_POLICY="$(q "SELECT version FROM cash_transfer_policy_versions WHERE organization_id = '$ORG_ID' ORDER BY created_at DESC LIMIT 1;")"
if [ -z "$CUR_POLICY" ]; then
  q "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by)
     VALUES ('seed-test-1', '$ORG_ID', '{\"THB\":\"200000\"}'::jsonb, '$OWNER_ID');" >/dev/null
  echo "    cash-transfer policy created: seed-test-1 (THB limit 200000)"
else
  echo "    already present ($CUR_POLICY)"
fi

say "5d. rate_alert_settings for org XC"
q "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, default_lifetime_days, observe_policy, updated_by, updated_at)
   VALUES ('$ORG_ID', 3600, 10, 30, 'ALL_FRESH', '$OWNER_ID', now())
   ON CONFLICT (organization_id) DO NOTHING;" >/dev/null

say "5e. MARKETING consent text (th, en, zh)"
q "INSERT INTO consent_texts (organization_id, purpose, version, lang, body, published_by, published_at) VALUES
   ('$ORG_ID', 'MARKETING', 1, 'th', 'ความยินยอมรับข่าวสารการตลาด (ฉบับทดสอบ)', '$OWNER_ID', now()),
   ('$ORG_ID', 'MARKETING', 1, 'en', 'Marketing consent (test version)', '$OWNER_ID', now()),
   ('$ORG_ID', 'MARKETING', 1, 'zh', '营销同意书（测试版）', '$OWNER_ID', now())
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

say "6b. test branch manager $MANAGER_PHONE (BRANCH_MANAGER on $BRANCH: certifies shift counts)"
MANAGER_PW="$(new_password)"
MANAGER_SUB="$(identity_register "$MANAGER_PHONE" "$MANAGER_PW")"
if [ -n "$MANAGER_SUB" ]; then
  q "WITH u AS (
       INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
       VALUES (gen_random_uuid(), '$ORG_ID', '$MANAGER_SUB', 'Test Manager', 'ACTIVE', 1, now(), '$OWNER_ID')
       ON CONFLICT (auth_subject) DO NOTHING RETURNING id)
     INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
     SELECT id, 'BRANCH_MANAGER', '$ORG_ID', '$BRANCH' FROM u;" >/dev/null
  [ "$JSON" = 1 ] || echo "    branch manager created: $MANAGER_PHONE  password $MANAGER_PW"
else
  MANAGER_PW="(unchanged: $MANAGER_PHONE was already registered)"
fi

# A5 (cash-shift-transfer.md:17): a transfer within the limit is approved by the source branch's BRANCH_MANAGER and
# never by its creator, so cash reaches the safe (EXTERNAL_IN, BANK -> SAFE) only when someone else creates it.
# FINANCE holds CashManage (role_permissions.rs), so it records the move and the branch manager approves it.
say "6c. test finance $FINANCE_PHONE (FINANCE on $BRANCH: records cash into the safe for the manager to approve)"
FINANCE_PW="$(new_password)"
FINANCE_SUB="$(identity_register "$FINANCE_PHONE" "$FINANCE_PW")"
if [ -n "$FINANCE_SUB" ]; then
  q "WITH u AS (
       INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
       VALUES (gen_random_uuid(), '$ORG_ID', '$FINANCE_SUB', 'Test Finance', 'ACTIVE', 1, now(), '$OWNER_ID')
       ON CONFLICT (auth_subject) DO NOTHING RETURNING id)
     INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
     SELECT id, 'FINANCE', '$ORG_ID', '$BRANCH' FROM u;" >/dev/null
  [ "$JSON" = 1 ] || echo "    finance created: $FINANCE_PHONE  password $FINANCE_PW"
else
  FINANCE_PW="(unchanged: $FINANCE_PHONE was already registered)"
fi

say "6d. test marketing $MARKETING_PHONE (MARKETING on $BRANCH: smokes #campaigns, #campaign-performance, #members)"
MARKETING_PW="$(new_password)"
MARKETING_SUB="$(identity_register "$MARKETING_PHONE" "$MARKETING_PW")"
if [ -n "$MARKETING_SUB" ]; then
  q "WITH u AS (
       INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
       VALUES (gen_random_uuid(), '$ORG_ID', '$MARKETING_SUB', 'Test Marketing', 'ACTIVE', 1, now(), '$OWNER_ID')
       ON CONFLICT (auth_subject) DO NOTHING RETURNING id)
     INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
     SELECT id, 'MARKETING', '$ORG_ID', '$BRANCH' FROM u;" >/dev/null
  [ "$JSON" = 1 ] || echo "    marketing created: $MARKETING_PHONE  password $MARKETING_PW"
else
  MARKETING_PW="(unchanged: $MARKETING_PHONE was already registered)"
fi

say "6e. test branch manager 2 $MANAGER2_PHONE (BRANCH_MANAGER on $BRANCH2: approves and receives branch transfers)"
MANAGER2_PW="$(new_password)"
MANAGER2_SUB="$(identity_register "$MANAGER2_PHONE" "$MANAGER2_PW")"
if [ -n "$MANAGER2_SUB" ]; then
  q "WITH u AS (
       INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
       VALUES (gen_random_uuid(), '$ORG_ID', '$MANAGER2_SUB', 'Test Manager 2', 'ACTIVE', 1, now(), '$OWNER_ID')
       ON CONFLICT (auth_subject) DO NOTHING RETURNING id)
     INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
     SELECT id, 'BRANCH_MANAGER', '$ORG_ID', '$BRANCH2' FROM u;" >/dev/null
  [ "$JSON" = 1 ] || echo "    branch manager 2 created: $MANAGER2_PHONE  password $MANAGER2_PW"
else
  MANAGER2_PW="(unchanged: $MANAGER2_PHONE was already registered)"
fi

say "6f. test finance 2 $FINANCE2_PHONE (FINANCE on $BRANCH2: creates and dispatches branch transfers from $BRANCH2)"
FINANCE2_PW="$(new_password)"
FINANCE2_SUB="$(identity_register "$FINANCE2_PHONE" "$FINANCE2_PW")"
if [ -n "$FINANCE2_SUB" ]; then
  q "WITH u AS (
       INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
       VALUES (gen_random_uuid(), '$ORG_ID', '$FINANCE2_SUB', 'Test Finance 2', 'ACTIVE', 1, now(), '$OWNER_ID')
       ON CONFLICT (auth_subject) DO NOTHING RETURNING id)
     INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
     SELECT id, 'FINANCE', '$ORG_ID', '$BRANCH2' FROM u;" >/dev/null
  [ "$JSON" = 1 ] || echo "    finance 2 created: $FINANCE2_PHONE  password $FINANCE2_PW"
else
  FINANCE2_PW="(unchanged: $FINANCE2_PHONE was already registered)"
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
  case "$TELLER_PW$CUSTOMER_PW$MANAGER_PW$FINANCE_PW$MARKETING_PW$MANAGER2_PW$FINANCE2_PW" in *unchanged*) die "an account already existed, so its password is unknown; run again (new random phones)";; esac
  cat >&3 <<JSON
{
  "baseUrl": "$BASE",
  "customer": { "phone": "$CUSTOMER_PHONE", "password": "$CUSTOMER_PW" },
  "teller":   { "phone": "$TELLER_PHONE", "password": "$TELLER_PW" },
  "branchManager": { "phone": "$MANAGER_PHONE", "password": "$MANAGER_PW" },
  "finance":  { "phone": "$FINANCE_PHONE", "password": "$FINANCE_PW" },
  "marketing": { "phone": "$MARKETING_PHONE", "password": "$MARKETING_PW" },
  "branchManager2": { "phone": "$MANAGER2_PHONE", "password": "$MANAGER2_PW" },
  "finance2": { "phone": "$FINANCE2_PHONE", "password": "$FINANCE2_PW" },
  "admin":    { "identifier": "<the owner's sign-in>", "password": "<the owner's password>" },
  "referralCode": "TEST1234",
  "branch": "$BRANCH",
  "branch2": "$BRANCH2",
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
