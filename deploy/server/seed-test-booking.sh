#!/usr/bin/env bash
# Test data for end-to-end checks on a v3 host (#1683, #5910).
# Reads the roster from users.json (--users <file|->) and creates only what is missing from it,
# by role and branch. Never adds a slot the file does not list, and never writes users.json.
#
#   sudo deploy/server/seed-test-booking.sh --users <file|-> https://xc-v3.spacing.cloud
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
#   6. Ensures each slot listed in users.json (owner first, then staff and customer).
#
# Environment (optional):
#   SEED_DAYS      default 7
#   SEED_API_BASE  default = the host argument (seam for /api/app/register)
set -euo pipefail

USERS_ARG=""
if [ "${1:-}" = "--users" ]; then
  [ $# -ge 2 ] || { echo "usage: $0 --users <file|-> https://<host>" >&2; exit 2; }
  USERS_ARG="$2"
  shift 2
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=common.sh disable=SC1091
. "$HERE/common.sh"

if [ -z "$USERS_ARG" ] || [ $# -ne 1 ] || ! [[ "$1" =~ ^https://[a-z0-9.-]+$ ]]; then
  echo "usage: $0 --users <file|-> https://<host>   (lower-case host, no port, no path)" >&2
  exit 2
fi
BASE="$1"
SEED_API_BASE="${SEED_API_BASE:-$BASE}"
HOST="${BASE#https://}"
SEED_DAYS="${SEED_DAYS:-7}"
ORG_ID="00000000-0000-0000-0000-000000000001"   # org XC, seeded by 20261011100000_organizations.sql
BRANCH="MAIN"                                     # seeded by 20261007100000_branches_rates.sql, code = id
BRANCH2="TEST2"                                   # second test branch for branch-to-branch transfers (F12b)

need curl python3 sudo
USERS_FILE="$USERS_ARG"
USERS_TMP=""
if [ "$USERS_ARG" = "-" ]; then
  USERS_TMP="$(mktemp)"
  trap 'rm -f "$USERS_TMP"' EXIT
  cat >"$USERS_TMP"
  USERS_FILE="$USERS_TMP"
fi
[ -f "$USERS_FILE" ] || die "users file not found: $USERS_FILE"

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

# One curl config line on stdout (`curl -K -` reads them from stdin). Secrets and passwords go to curl this way,
# never as arguments, so they are not visible in `ps` / /proc/<pid>/cmdline; printf is a builtin, so neither is it.
cfg() { local v=${2//\\/\\\\}; v=${v//\"/\\\"}; printf '%s = "%s"\n' "$1" "$v"; }

# Signs in <phone> with Identity. Prints identityId on 200, or nothing on 401 invalid_credentials.
id_login() {
  local phone="$1" password="$2" resp code body
  resp=$({ cfg url "${ID_URL%/}/auth/login"; cfg header 'Content-Type: application/json'
           cfg header "x-client-id: $ID_CLIENT"; cfg header "x-client-secret: $ID_SECRET"
           if [ -n "$ID_KEY" ]; then cfg header "x-api-key: $ID_KEY"; fi
           cfg data "{\"identifier\":\"$phone\",\"password\":\"$password\"}"
         } | curl -sS -w '\n%{http_code}' -X POST -K -) || die "Identity unreachable at $ID_URL"
  code=$(tail -n 1 <<<"$resp")
  body=$(sed '$d' <<<"$resp")
  case "$code" in
    200) sed -n 's/.*"identityId":"\([^"]*\)".*/\1/p' <<<"$body" ;;
    401) return 0 ;;
    *) die "Identity login for $phone answered $code: $(sed -E 's/"(password|token)[^,}]*//g' <<<"$body" | head -c 300)" ;;
  esac
}

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

# Slot table (§3): prints "<roles>\t<branch|ORG|->\t<name>", or dies on an unknown slot key.
spec_of() {
  case "$1" in
    owner)          printf 'EXECUTIVE,ADMIN\tORG\tTest Owner\n' ;;
    teller)         printf 'TELLER\t%s\tTest Teller\n' "$BRANCH" ;;
    teller2)        printf 'TELLER\t%s\tTest Teller 2\n' "$BRANCH" ;;
    teller3)        printf 'TELLER\t%s\tTest Teller 3\n' "$BRANCH" ;;
    branchManager)  printf 'BRANCH_MANAGER\t%s\tTest Manager\n' "$BRANCH" ;;
    finance)        printf 'FINANCE\t%s\tTest Finance\n' "$BRANCH" ;;
    branch2Teller)  printf 'TELLER\t%s\tTest Teller TEST2 1\n' "$BRANCH2" ;;
    branch2Teller2) printf 'TELLER\t%s\tTest Teller TEST2 2\n' "$BRANCH2" ;;
    branch2Teller3) printf 'TELLER\t%s\tTest Teller TEST2 3\n' "$BRANCH2" ;;
    branchManager2) printf 'BRANCH_MANAGER\t%s\tTest Manager 2\n' "$BRANCH2" ;;
    finance2)       printf 'FINANCE\t%s\tTest Finance 2\n' "$BRANCH2" ;;
    marketing)      printf 'MARKETING\tORG\tTest Marketing\n' ;;
    executive)      printf 'EXECUTIVE\tORG\tTest Executive\n' ;;
    admin)          printf 'ADMIN\tORG\tTest Admin\n' ;;
    customer)       printf 'CUSTOMER\t-\tTest Customer\n' ;;
    *) die "unknown slot key in users.json: $1" ;;
  esac
}

# Ensures one slot (<key> <phone> <password>): finds or registers the Identity account, then ensures its users row.
ensure_slot() {
  local key="$1" phone="$2" password="$3" spec roles br name sub uid r br_sql last_br created_by resp code
  spec="$(spec_of "$key")"
  IFS=$'\t' read -r roles br name <<<"$spec"
  sub="$(id_login "$phone" "$password")"
  if [ -z "$sub" ]; then
    sub="$(identity_register "$phone" "$password")"
    [ -n "$sub" ] || die "slot $key ($phone): phone exists in Identity with another password"
  fi
  uid="$(q "SELECT id FROM users WHERE organization_id = '$ORG_ID' AND auth_subject = '$sub' LIMIT 1;")"
  if [ -n "$uid" ]; then
    echo "    $key ($phone): already present" >&2
    return 0
  fi
  if [ "$key" = "customer" ]; then
    resp=$({ cfg url "${SEED_API_BASE%/}/api/app/register"; cfg header 'Content-Type: application/json'
             cfg header "Origin: $BASE"; cfg header 'X-Xchange-Request: 1'
             cfg data "{\"identifier\":\"$phone\",\"password\":\"$password\",\"displayName\":\"$name\",\"lang\":\"th\",\"marketing\":[]}"
           } | curl -sS -w '\n%{http_code}' -X POST -K -) || die "POST $SEED_API_BASE/api/app/register unreachable"
    code=$(tail -n 1 <<<"$resp")
    case "$code" in
      200 | 409) echo "    $key ($phone): ensured ($code)" >&2; return 0 ;;
      *) die "POST $SEED_API_BASE/api/app/register for $key ($phone) answered $code: $(sed '$d' <<<"$resp" | head -c 400)" ;;
    esac
  fi
  if [ "$br" = "ORG" ]; then
    br_sql="NULL"
    last_br="'$BRANCH'"
  else
    br_sql="'$br'"
    last_br="NULL"
  fi
  created_by="${OWNER_ID:-self}"
  uid="$(q "INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by, last_branch_id)
            VALUES (gen_random_uuid(), '$ORG_ID', '$sub', '$name', 'ACTIVE', 1, now(), '$created_by', $last_br)
            RETURNING id;")"
  IFS=',' read -ra role_list <<<"$roles"
  for r in "${role_list[@]}"; do
    q "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
       VALUES ('$uid', '$r', '$ORG_ID', $br_sql);" >/dev/null
  done
  echo "    $key ($phone): created ($roles on $br)" >&2
}

# Reads slots from $USERS_FILE as "<key>\t<phone>\t<password>" lines, owner first.
SLOTS="$(python3 - "$USERS_FILE" <<'PY'
import json, sys
META = {"baseUrl", "branch", "branch2", "referralCode", "currency", "quantity"}
data = json.load(open(sys.argv[1], encoding="utf-8"))
if not isinstance(data, dict):
    sys.exit("users.json must be a JSON object")
keys = [k for k in data if k not in META]
if "owner" in keys:
    keys = ["owner"] + [k for k in keys if k != "owner"]
for k in keys:
    v = data[k]
    if not isinstance(v, dict):
        sys.exit(f"slot {k}: expected an object")
    ident = v.get("phone") or v.get("identifier") or ""
    pw = v.get("password") or ""
    if not ident or not pw:
        sys.exit(f"slot {k}: missing phone/identifier or password")
    print(f"{k}\t{ident}\t{pw}")
PY
)" || die "invalid users file: $USERS_FILE"

while IFS=$'\t' read -r key _ _; do
  [ -n "$key" ] || continue
  spec_of "$key" >/dev/null
done <<<"$SLOTS"

say "1. host $HOST → org XC" >&2
q "INSERT INTO organization_hosts (host, organization_id) VALUES ('$HOST', '$ORG_ID') ON CONFLICT (host) DO NOTHING;" >/dev/null

OWNER_LINE="$(awk -F'\t' '$1 == "owner" { print; exit }' <<<"$SLOTS")"
if [ -n "$OWNER_LINE" ]; then
  IFS=$'\t' read -r key phone pw <<<"$OWNER_LINE"
  ensure_slot "$key" "$phone" "$pw"
fi

OWNER_ID="$(q "SELECT u.id FROM users u
  WHERE u.organization_id = '$ORG_ID' AND u.status = 'ACTIVE'
    AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL AND r.valid_to IS NULL)
    AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'ADMIN' AND r.branch_id IS NULL AND r.valid_to IS NULL)
  ORDER BY u.created_at LIMIT 1;")"
[ -n "$OWNER_ID" ] || die "no ACTIVE owner. Include an owner slot in users.json or create one first: $BIN create-owner --name <name> --phone <phone> (password on stdin)"

say "2. branch $BRANCH: booking on, Asia/Bangkok, 00:00–23:59 daily" >&2
# All-day test hours so e2e can run at any hour (#4315, owner 2026-10-21: "ปรับ เวลา ทำการใหม่ได้ นะ" · "เพื่อให้มันเทสได้").
HOURS='{"mon":[{"open":"00:00","close":"23:59"}],"tue":[{"open":"00:00","close":"23:59"}],"wed":[{"open":"00:00","close":"23:59"}],"thu":[{"open":"00:00","close":"23:59"}],"fri":[{"open":"00:00","close":"23:59"}],"sat":[{"open":"00:00","close":"23:59"}],"sun":[{"open":"00:00","close":"23:59"}]}'
q "UPDATE branches SET booking_enabled = true, timezone = 'Asia/Bangkok', opening_hours = '$HOURS'::jsonb
   WHERE organization_id = '$ORG_ID' AND id = '$BRANCH';" >/dev/null

say "2b. branch $BRANCH2: booking off, Asia/Bangkok, 00:00–23:59 daily" >&2
q "INSERT INTO branches (id, organization_id, name, created_at, created_by, request_id, code, timezone, opening_hours, booking_enabled)
   VALUES ('$BRANCH2', '$ORG_ID', 'Test branch 2', now(), '$OWNER_ID', 'seed_test_branch_$BRANCH2', '$BRANCH2', 'Asia/Bangkok', '$HOURS'::jsonb, false)
   ON CONFLICT (id) DO UPDATE SET
     organization_id = EXCLUDED.organization_id,
     name = EXCLUDED.name,
     code = EXCLUDED.code,
     timezone = EXCLUDED.timezone,
     opening_hours = EXCLUDED.opening_hours,
     booking_enabled = EXCLUDED.booking_enabled;" >/dev/null

say "3. slot rule for $BRANCH (30 min, capacity 4, 14 days)" >&2
q "INSERT INTO booking_slot_rules (organization_id, branch_id, slot_minutes, capacity, horizon_days, updated_by)
   VALUES ('$ORG_ID', '$BRANCH', 30, 4, 14, '$OWNER_ID')
   ON CONFLICT (branch_id) DO NOTHING;" >/dev/null

say "4. open slots for the next $SEED_DAYS days" >&2
q "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status)
   SELECT gen_random_uuid(), b.organization_id, b.id, d.day,
          (d.day + s.t) AT TIME ZONE b.timezone, (d.day + s.t + interval '30 minutes') AT TIME ZONE b.timezone, 4, 0, 'OPEN'
   FROM branches b
   CROSS JOIN (SELECT ((now() AT TIME ZONE 'Asia/Bangkok')::date + g) AS day FROM generate_series(1, $SEED_DAYS) g) d
   CROSS JOIN (SELECT time '09:00' + n * interval '30 minutes' AS t FROM generate_series(0, 15) n) s
   WHERE b.organization_id = '$ORG_ID' AND b.id = '$BRANCH'
     AND NOT EXISTS (SELECT 1 FROM branch_closures c WHERE c.branch_id = b.id AND c.closed_on = d.day)
   ON CONFLICT (branch_id, start_at) DO NOTHING;" >/dev/null
echo "    open slots on $BRANCH from tomorrow: $(q "SELECT count(*) FROM booking_slots WHERE branch_id = '$BRANCH' AND status = 'OPEN' AND start_at > now();")" >&2

say "5. TERMS consent text (th, en, zh)" >&2
q "INSERT INTO consent_texts (organization_id, purpose, version, lang, body, published_by, published_at) VALUES
   ('$ORG_ID', 'TERMS', 1, 'th', 'ข้อตกลงการใช้บริการ (ฉบับทดสอบ)', '$OWNER_ID', now()),
   ('$ORG_ID', 'TERMS', 1, 'en', 'Terms of service (test version)', '$OWNER_ID', now()),
   ('$ORG_ID', 'TERMS', 1, 'zh', '服务条款（测试版）', '$OWNER_ID', now())
   ON CONFLICT DO NOTHING;" >/dev/null

say "5b. published rate set for $BRANCH (USD, EUR)" >&2
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
     FROM branches b WHERE b.organization_id = '$ORG_ID' AND id = '$BRANCH';
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
  echo "    published rate set $PUB_ID (USD 32.10/32.60, EUR 35.00/35.60)" >&2
else
  echo "    already present ($CUR_PUB)" >&2
fi

say "5c. cash-transfer policy for org XC" >&2
CUR_POLICY="$(q "SELECT version FROM cash_transfer_policy_versions WHERE organization_id = '$ORG_ID' ORDER BY created_at DESC LIMIT 1;")"
if [ -z "$CUR_POLICY" ]; then
  q "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by)
     VALUES ('seed-test-1', '$ORG_ID', '{\"THB\":\"200000\"}'::jsonb, '$OWNER_ID');" >/dev/null
  echo "    cash-transfer policy created: seed-test-1 (THB limit 200000)" >&2
else
  echo "    already present ($CUR_POLICY)" >&2
fi

say "5d. rate_alert_settings for org XC" >&2
q "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, max_active_per_customer, default_lifetime_days, observe_policy, updated_by, updated_at)
   VALUES ('$ORG_ID', 3600, 10, 30, 'ALL_FRESH', '$OWNER_ID', now())
   ON CONFLICT (organization_id) DO NOTHING;" >/dev/null

say "5e. MARKETING consent text (th, en, zh)" >&2
q "INSERT INTO consent_texts (organization_id, purpose, version, lang, body, published_by, published_at) VALUES
   ('$ORG_ID', 'MARKETING', 1, 'th', 'ความยินยอมรับข่าวสารการตลาด (ฉบับทดสอบ)', '$OWNER_ID', now()),
   ('$ORG_ID', 'MARKETING', 1, 'en', 'Marketing consent (test version)', '$OWNER_ID', now()),
   ('$ORG_ID', 'MARKETING', 1, 'zh', '营销同意书（测试版）', '$OWNER_ID', now())
   ON CONFLICT DO NOTHING;" >/dev/null

say "6. roster from users.json" >&2
while IFS=$'\t' read -r key phone pw; do
  [ -n "$key" ] && [ "$key" != "owner" ] || continue
  ensure_slot "$key" "$phone" "$pw"
done <<<"$SLOTS"
