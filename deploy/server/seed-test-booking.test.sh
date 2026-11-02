#!/usr/bin/env bash
# Ticket #5910 — the test system is built once from an empty database, and every later reseed reuses it.
# Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken;
# add cases in a file of your own.
#
# Owner's rules (relayed by the lane controller claude/608dff, 2026-11-02):
#   "ตามเสนอ reseed ใช้บัญชีชุดเดิม" · "สาขา 1 มีพนักงานไม่เกิน 10" · "แคชเชียร์มี 3 คน" · "ตำแหน่งอื่น อย่างละคน"
#   "staff ให้ สร้าง ครั้งเดียว พอ" · "การเทส ครั้ง ต่อไป ห้าม สร้าง บัญชี ใหม่ มาเทส ให้ เอาตาม user.json"
#
# Controller's note 06:4x ICT: claude-4 creates the roster ONCE through the admin UI and writes users.json, so
# users.json is the source of truth. The seed reads it (`--users <file|->`) and creates only what is missing from
# it, by role and branch. It never adds a slot the file does not list, never writes users.json, and on a system
# where every listed account exists it creates nothing.
#
# What runs: deploy/server/seed-test-booking.sh against
#   - a real PostgreSQL database named by SEED_TEST_DATABASE_URL (it must be empty and its name must end in
#     `_test`); this file applies api/migrations/*.sql to it in file-name order, as the API does at boot;
#   - a fake Identity and a fake `POST /api/app/register`, served by python3 on 127.0.0.1 (below). The seed reads
#     IDENTITY_BASE_URL from $ETC_DIR/identity-client.env and the API base from SEED_API_BASE (the ticket's seam);
#   - a `sudo` stub first on PATH: `sudo -u postgres psql …` runs psql against SEED_TEST_DATABASE_URL, and any
#     other `sudo <cmd>` runs <cmd>. Nothing here needs root.
#
# Rows (the ticket's behaviour table):
#   U1  today's system: the roster was made through the UI (rows + Identity accounts exist for every users.json
#       slot) ⇒ the seed creates no users row, no role row and no Identity account; users.json is not rewritten.
#   U2  after a wipe (empty users table, Identity keeps the accounts) ⇒ the seed creates exactly one users row per
#       users.json slot, with the slot's role and branch, bound to the slot's existing Identity account, and the
#       slot's password still signs in. A second run creates nothing.
#   U3  a slot whose phone Identity does not know ⇒ it is registered with the file's password and gets its row;
#       a second run creates nothing.
#   U4  a role or branch the file does not list is never created (no TEST2 slot in the file ⇒ no TEST2 staff).
#   U5  no users file ⇒ the seed refuses (non-zero exit) and creates no users row.
#   U6  the non-account seed objects are not duplicated by a second run.
#
# Run: SEED_TEST_DATABASE_URL=postgres://postgres:pw@127.0.0.1:55910/seed_test bash deploy/server/seed-test-booking.test.sh
# Needs: bash, psql, python3, curl, openssl. Not wired into CI: the deploy-scripts runner has no PostgreSQL.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SEED="$ROOT/deploy/server/seed-test-booking.sh"
URL="${SEED_TEST_DATABASE_URL:-}"
fail() { echo "FAIL: $*" >&2; exit 1; }
[ -n "$URL" ] || fail "set SEED_TEST_DATABASE_URL to an empty PostgreSQL database whose name ends in _test"
DBNAME="${URL##*/}"; DBNAME="${DBNAME%%\?*}"
[[ "$DBNAME" == *_test ]] || fail "the database name must end in _test, got $DBNAME"
for t in psql python3 curl openssl; do command -v "$t" >/dev/null || fail "missing tool: $t"; done

WORK="$(mktemp -d)"
SERVER_PID=""
cleanup() { if [ -n "$SERVER_PID" ]; then kill "$SERVER_PID" 2>/dev/null || true; wait "$SERVER_PID" 2>/dev/null || true; fi; rm -rf "$WORK"; }
trap cleanup EXIT

sql() { psql "$URL" -v ON_ERROR_STOP=1 -X -q -tA -c "$1"; }

# ---------------------------------------------------------------- an empty database, migrated as at boot
[ "$(sql "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'")" = 0 ] \
  || fail "$DBNAME is not empty"
for f in "$ROOT"/api/migrations/*.sql; do
  psql "$URL" -v ON_ERROR_STOP=1 -X -q --single-transaction -f "$f" >/dev/null || fail "migration $(basename "$f")"
done

# ---------------------------------------------------------------- the sudo stub, the env files, the fakes
mkdir -p "$WORK/bin" "$WORK/etc"
cat > "$WORK/bin/sudo" <<STUB
#!/usr/bin/env bash
if [ "\${1:-}" = "-u" ]; then shift 2; fi
if [ "\${1:-}" = "psql" ]; then
  shift
  args=(); while [ \$# -gt 0 ]; do
    if [ "\$1" = "-d" ]; then shift 2; continue; fi
    args+=("\$1"); shift
  done
  exec psql "$URL" "\${args[@]}"
fi
exec "\$@"
STUB
chmod +x "$WORK/bin/sudo"
export PATH="$WORK/bin:$PATH"

PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
cat > "$WORK/etc/env" <<EOF
APP_DATABASE__DATABASE_NAME=$DBNAME
EOF
cat > "$WORK/etc/identity-client.env" <<EOF
IDENTITY_BASE_URL=http://127.0.0.1:$PORT
IDENTITY_CLIENT_ID=seed-test
IDENTITY_CLIENT_SECRET=seed-test-secret
IDENTITY_INTERNAL_TOKEN=seed-test-internal
EOF
export ETC_DIR="$WORK/etc"
export SEED_API_BASE="http://127.0.0.1:$PORT"

cat > "$WORK/fake.py" <<'PY'
# A fake Identity (register, login, admin password) and a fake POST /api/app/register.
import json, subprocess, sys, uuid
from http.server import BaseHTTPRequestHandler, HTTPServer
URL, PORT, LOG = sys.argv[1], int(sys.argv[2]), sys.argv[3]
ids = {}   # phone -> [identityId, password]
ORG = "00000000-0000-0000-0000-000000000001"
def psql(q):
    return subprocess.run(["psql", URL, "-X", "-q", "-tA", "-v", "ON_ERROR_STOP=1", "-c", q],
                          check=True, capture_output=True, text=True).stdout.strip()
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def answer(self, code, body):
        b = json.dumps(body, separators=(",", ":")).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n) or b"{}")
        with open(LOG, "a") as f: f.write(self.path + "\n")
        p = self.path
        if p == "/auth/register":
            if body["phone"] in ids: return self.answer(409, {"error": "phone_taken"})
            ids[body["phone"]] = [str(uuid.uuid4()), body["password"]]
            return self.answer(201, {"identityId": ids[body["phone"]][0]})
        if p == "/auth/login":
            rec = ids.get(body.get("identifier"))
            if not rec or rec[1] != body.get("password"): return self.answer(401, {"error": "invalid_credentials"})
            return self.answer(200, {"identityId": rec[0], "access_token": "fake." + rec[0] + ".sig", "token_type": "Bearer"})
        if p.startswith("/admin/identities/") and p.endswith("/password"):
            if self.headers.get("x-internal-token") != "seed-test-internal": return self.answer(401, {"error": "forbidden"})
            sub = p.split("/")[3]
            for rec in ids.values():
                if rec[0] == sub: rec[1] = body["newPassword"]; return self.answer(200, {})
            return self.answer(404, {"error": "not_found"})
        if p == "/api/app/register":
            rec = ids.get(body.get("identifier"))
            if not rec or rec[1] != body.get("password"): return self.answer(401, {"error": {"code": "invalidCredentials"}})
            if psql(f"SELECT count(*) FROM users WHERE auth_subject = '{rec[0]}'") != "0":
                return self.answer(409, {"error": {"code": "alreadyRegistered"}})
            psql(f"WITH u AS (INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by) "
                 f"VALUES (gen_random_uuid(), '{ORG}', '{rec[0]}', 'Test Customer', 'ACTIVE', 1, now(), 'self') RETURNING id) "
                 f"INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) SELECT id, 'CUSTOMER', '{ORG}', NULL FROM u")
            return self.answer(200, {"user": {"name": "Test Customer"}, "customerId": "CX-TEST"})
        return self.answer(404, {"error": "no such route " + p})
    def do_GET(self):
        if self.path == "/state":
            return self.answer(200, {k: v[1] for k, v in ids.items()})
        return self.answer(404, {})
HTTPServer(("127.0.0.1", PORT), H).serve_forever()
PY
python3 "$WORK/fake.py" "$URL" "$PORT" "$WORK/calls.log" &
SERVER_PID=$!
for _ in $(seq 1 50); do curl -s "http://127.0.0.1:$PORT/state" >/dev/null 2>&1 && break; sleep 0.1; done

# ---------------------------------------------------------------- helpers
BASE="https://xc.test"
ORG="00000000-0000-0000-0000-000000000001"
seed_with() {  # seed_with <users file>: must succeed
  bash "$SEED" --users "$1" "$BASE" >"$WORK/out" 2>"$WORK/err" || { cat "$WORK/err" >&2; fail "seed run with --users $1"; }
}
users_snapshot() { sql "SELECT coalesce(string_agg(id::text || ':' || auth_subject || ':' || created_at::text, ',' ORDER BY id), '') FROM users"; }
roles_snapshot() { sql "SELECT coalesce(string_agg(user_id::text || ':' || role_code || ':' || coalesce(branch_id, '*'), ',' ORDER BY user_id, role_code), '') FROM user_roles"; }
registers() { grep -c '^/auth/register$' "$WORK/calls.log" 2>/dev/null || true; }
role_count() {  # role_count <role> <branch|ORG>
  if [ "$2" = ORG ]; then
    sql "SELECT count(DISTINCT user_id) FROM user_roles WHERE role_code = '$1' AND branch_id IS NULL AND valid_to IS NULL"
  else
    sql "SELECT count(DISTINCT user_id) FROM user_roles WHERE role_code = '$1' AND branch_id = '$2' AND valid_to IS NULL"
  fi
}
identity_register() {  # identity_register <phone> <password> → identityId (the UI's path, simulated)
  curl -s -H 'Content-Type: application/json' -d "{\"phone\":\"$1\",\"password\":\"$2\"}" \
    "http://127.0.0.1:$PORT/auth/register" | python3 -c 'import json,sys; print(json.load(sys.stdin)["identityId"])'
}
ui_create() {  # ui_create <phone> <password> <name> <role> <branch|ORG> [<role2>]: what the admin UI leaves behind
  local sub; sub="$(identity_register "$1" "$2")"
  local br="'$5'"; [ "$5" = ORG ] && br=NULL
  sql "WITH u AS (INSERT INTO users (id, organization_id, auth_subject, name, status, version, created_at, created_by)
         VALUES (gen_random_uuid(), '$ORG', '$sub', '$3', 'ACTIVE', 1, now(), 'ui') RETURNING id)
       INSERT INTO user_roles (user_id, role_code, organization_id, branch_id)
       SELECT id, '$4', '$ORG'::uuid, $br::text FROM u
       UNION ALL SELECT id, '${6:-$4}', '$ORG'::uuid, $br::text FROM u WHERE '${6:-}' <> ''" >/dev/null
}
signs_in() {  # signs_in <phone> <password>
  local body code
  body=$(printf '{"identifier":"%s","password":"%s"}' "$1" "$2")
  code=$(curl -s -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -d "$body" \
           "http://127.0.0.1:$PORT/auth/login")
  [ "$code" = 200 ]
}
seed_objects() {
  sql "SELECT (SELECT count(*) FROM branches WHERE id = 'TEST2') || ',' ||
              (SELECT count(*) FROM booking_slot_rules WHERE branch_id = 'MAIN') || ',' ||
              (SELECT count(*) FROM consent_texts) || ',' ||
              (SELECT count(*) FROM rate_publications WHERE branch_id = 'MAIN') || ',' ||
              (SELECT count(*) FROM cash_transfer_policy_versions) || ',' ||
              (SELECT count(*) FROM rate_alert_settings) || ',' ||
              (SELECT count(*) FROM organization_hosts)"
}

# The roster claude-4 makes through the UI (controller's note): owner, MAIN 3 tellers + 1 manager + 1 finance,
# org-wide executive and marketing, and one customer. No TEST2 slot, no admin slot.
pw() { echo "Xc$(openssl rand -hex 6)A9"; }
P_OWNER=$(pw); P_T1=$(pw); P_T2=$(pw); P_T3=$(pw); P_MGR=$(pw); P_FIN=$(pw); P_EXE=$(pw); P_MKT=$(pw); P_CUS=$(pw)
cat > "$WORK/users.json" <<JSON
{
  "baseUrl": "$BASE",
  "owner":    { "phone": "+66990000100", "password": "$P_OWNER" },
  "teller":   { "phone": "+66990000101", "password": "$P_T1" },
  "teller2":  { "phone": "+66990000102", "password": "$P_T2" },
  "teller3":  { "phone": "+66990000103", "password": "$P_T3" },
  "branchManager": { "phone": "+66990000104", "password": "$P_MGR" },
  "finance":  { "phone": "+66990000105", "password": "$P_FIN" },
  "executive": { "phone": "+66990000106", "password": "$P_EXE" },
  "marketing": { "phone": "+66990000107", "password": "$P_MKT" },
  "customer": { "phone": "+66990000108", "password": "$P_CUS" },
  "branch": "MAIN",
  "currency": "USD",
  "quantity": "100"
}
JSON
cp "$WORK/users.json" "$WORK/users.orig.json"

# ---------------------------------------------------------------- U5: no users file ⇒ refused, nothing created
if bash "$SEED" "$BASE" >"$WORK/out" 2>"$WORK/err"; then fail "U5: the seed ran without a users file"; fi
[ "$(sql "SELECT count(*) FROM users")" = 0 ] || fail "U5: a run without a users file created users rows"

# ---------------------------------------------------------------- U1: today's system, made through the UI
ui_create +66990000100 "$P_OWNER" "Test Owner" EXECUTIVE ORG ADMIN
ui_create +66990000101 "$P_T1" "Test Teller" TELLER MAIN
ui_create +66990000102 "$P_T2" "Test Teller 2" TELLER MAIN
ui_create +66990000103 "$P_T3" "Test Teller 3" TELLER MAIN
ui_create +66990000104 "$P_MGR" "Test Manager" BRANCH_MANAGER MAIN
ui_create +66990000105 "$P_FIN" "Test Finance" FINANCE MAIN
ui_create +66990000106 "$P_EXE" "Test Executive" EXECUTIVE ORG
ui_create +66990000107 "$P_MKT" "Test Marketing" MARKETING ORG
ui_create +66990000108 "$P_CUS" "Test Customer" CUSTOMER ORG
U_BEFORE="$(users_snapshot)"; R_BEFORE="$(roles_snapshot)"; REG_BEFORE="$(registers)"
seed_with "$WORK/users.json"
[ "$(users_snapshot)" = "$U_BEFORE" ] || fail "U1: the seed created or changed users rows on a complete system"
[ "$(roles_snapshot)" = "$R_BEFORE" ] || fail "U1: the seed created or changed role rows on a complete system"
[ "$(registers)" = "$REG_BEFORE" ] || fail "U1: the seed registered an Identity account on a complete system"
cmp -s "$WORK/users.json" "$WORK/users.orig.json" || fail "U1: users.json was rewritten"
[ "$(role_count TELLER TEST2)$(role_count BRANCH_MANAGER TEST2)$(role_count FINANCE TEST2)" = 000 ] \
  || fail "U4: TEST2 staff were created though users.json lists none"
[ "$(sql "SELECT count(*) FROM user_roles WHERE role_code = 'ADMIN' AND branch_id IS NULL")" = 1 ] \
  || fail "U4: an ADMIN slot was created though users.json lists only the owner's ADMIN"
OBJ1="$(seed_objects)"
echo "ok U1 a complete system: nothing created, users.json untouched; U4 nothing beyond the file"

# ---------------------------------------------------------------- U2: after a wipe, Identity keeps the accounts
sql "TRUNCATE users CASCADE" >/dev/null
[ "$(sql "SELECT count(*) FROM users")" = 0 ] || fail "U2 setup: users not empty"
REG_BEFORE="$(registers)"
seed_with "$WORK/users.json"
[ "$(sql "SELECT count(*) FROM users")" = 9 ] || fail "U2: $(sql "SELECT count(*) FROM users") users rows, expected 9 (one per slot)"
[ "$(registers)" = "$REG_BEFORE" ] || fail "U2: the seed registered a new Identity account for a phone Identity already has"
[ "$(role_count TELLER MAIN)" = 3 ] || fail "U2: MAIN has $(role_count TELLER MAIN) tellers, expected 3"
[ "$(role_count BRANCH_MANAGER MAIN)$(role_count FINANCE MAIN)" = 11 ] || fail "U2: MAIN manager/finance not one each"
[ "$(role_count EXECUTIVE ORG)$(role_count ADMIN ORG)$(role_count MARKETING ORG)$(role_count CUSTOMER ORG)" = 2111 ] \
  || fail "U2: org-wide roles wrong: EXECUTIVE $(role_count EXECUTIVE ORG), ADMIN $(role_count ADMIN ORG), MARKETING $(role_count MARKETING ORG), CUSTOMER $(role_count CUSTOMER ORG)"
[ "$(role_count TELLER TEST2)$(role_count BRANCH_MANAGER TEST2)$(role_count FINANCE TEST2)" = 000 ] \
  || fail "U4 (after a wipe): TEST2 staff were created though users.json lists none"
[ "$(sql "SELECT count(*) FROM users u JOIN user_roles r ON r.user_id = u.id WHERE r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL
         AND EXISTS (SELECT 1 FROM user_roles a WHERE a.user_id = u.id AND a.role_code = 'ADMIN' AND a.branch_id IS NULL)")" = 1 ] \
  || fail "U2: the owner slot is not one account with EXECUTIVE + ADMIN org-wide"
for pair in "+66990000100 $P_OWNER" "+66990000101 $P_T1" "+66990000104 $P_MGR" "+66990000108 $P_CUS"; do
  signs_in $pair || fail "U2: ${pair%% *} no longer signs in with the users.json password"
done
cmp -s "$WORK/users.json" "$WORK/users.orig.json" || fail "U2: users.json was rewritten"
U2="$(users_snapshot)"; R2="$(roles_snapshot)"
seed_with "$WORK/users.json"
[ "$(users_snapshot)" = "$U2" ] && [ "$(roles_snapshot)" = "$R2" ] || fail "U2: the second run after a wipe changed users or roles"
[ "$(seed_objects)" = "$OBJ1" ] || fail "U6: seed objects changed: $OBJ1 -> $(seed_objects)"
echo "ok U2 after a wipe: one row per slot, bound to the existing Identity account; a second run changes nothing; U6"

# ---------------------------------------------------------------- U3: a slot Identity does not know
python3 - "$WORK/users.json" <<'PY'
import json, sys
p = sys.argv[1]; u = json.load(open(p))
u["finance2"] = {"phone": "+66990000109", "password": "Xc0123456789abA9"}
json.dump(u, open(p, "w"), indent=2)
PY
cp "$WORK/users.json" "$WORK/users.orig.json"
sql "INSERT INTO branches (id, name) VALUES ('TEST2', 'Test branch 2') ON CONFLICT DO NOTHING" >/dev/null
REG_BEFORE="$(registers)"
seed_with "$WORK/users.json"
[ "$(registers)" = "$((REG_BEFORE + 1))" ] || fail "U3: expected exactly one Identity registration, got $(( $(registers) - REG_BEFORE ))"
[ "$(role_count FINANCE TEST2)" = 1 ] || fail "U3: finance2 (FINANCE on TEST2) was not created"
[ "$(sql "SELECT count(*) FROM users")" = 10 ] || fail "U3: $(sql "SELECT count(*) FROM users") users rows, expected 10"
signs_in +66990000109 Xc0123456789abA9 || fail "U3: finance2 does not sign in with its users.json password"
U3="$(users_snapshot)"; REG_BEFORE="$(registers)"
seed_with "$WORK/users.json"
[ "$(users_snapshot)" = "$U3" ] && [ "$(registers)" = "$REG_BEFORE" ] || fail "U3: the second run created again"
cmp -s "$WORK/users.json" "$WORK/users.orig.json" || fail "U3: users.json was rewritten"
echo "ok U3 a missing slot is created once with the file's password"
echo "PASS seed-test-booking.test.sh"
