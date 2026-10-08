// Records what v2 answers on `POST /api/drawer-passport` when the passport drawer is switched
// off — what every v2 server without `XC_PASSPORT_DRAWER=1` answers — for row PD-0
// (`docs/design/xchange-v3.md` §5.5.5, rows 1-12 of its table).
//   node gen_drawer_passport.mjs <10-06 bundle dir> > ../../tests/fixtures/drawer_passport/v2_drawer_passport_cases.json
//
// The bundle is the 10-06 one (`XC-Desk-v2-Passport-Drawer-20261006.zip`, sha256 6514b4c8…): its
// `passport-drawer.mjs` sits beside `server.mjs`. Neither is in this repository; the script reads
// the module from the path given and imports it unchanged. What is supplied from here, and
// nothing else:
//   env   → AUTH_ENABLED=1, XC_PASSPORT_DRAWER and XC_DRAWER_BRANCH unset (the module's own
//           default `enabled`, `passport-drawer.mjs:13`, reads them): the drawer is off everywhere
//   pool  → throws on any use: switched off, v2 must answer every case without the database
//   send  → throws: switched off, no pulse may be sent
// Row 1 (nobody signed in) is the router's, not the module's: `server.mjs:8289` answers every
// non-public POST route 401 `{"error":"กรุณาเข้าสู่ระบบ"}` before the handler runs. It is
// written here as that line says, and the script checks that the line still says it.
//
// The callers are v2's actors (`server.mjs:8146`, staff from `actorOf`): the owner
// `{kind:"owner", role:"OWNER"}` and employees `{kind:"staff", role, branchId:"MAIN"}`. Every
// case names the row of §5.5.5 it reaches. The script fails, and writes nothing, when a case is
// answered differently from what it expects, or when a row of 1-12 has no case.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const dir = process.argv[2];
if (!dir) throw Error('usage: node gen_drawer_passport.mjs <10-06 bundle dir>');
const server = fs.readFileSync(path.join(dir, 'server.mjs'), 'utf8');
const ROUTER_401 = 'Response.json({ error: "\\u0E01\\u0E23\\u0E38\\u0E13\\u0E32\\u0E40\\u0E02\\u0E49\\u0E32\\u0E2A\\u0E39\\u0E48\\u0E23\\u0E30\\u0E1A\\u0E1A" }, { status: 401 })';
if (!server.includes(ROUTER_401)) throw Error('server.mjs: the router\'s 401 is not the one this script writes');
if (!server.includes('["POST", /^\\/api\\/drawer-passport$/, "user", (req,p,a) => passportDrawer.api(req,a)]')) throw Error('server.mjs: the drawer-passport route is not the one this script knows');

process.env.AUTH_ENABLED = '1';
delete process.env.XC_PASSPORT_DRAWER;
delete process.env.XC_DRAWER_BRANCH;
const { createPassportDrawer } = await import(pathToFileURL(path.join(dir, 'passport-drawer.mjs')).href);
const untouchable = { connect() { throw Error('the database was used'); }, query() { throw Error('the database was used'); } };
const drawer = createPassportDrawer(untouchable, { send() { throw Error('a pulse was sent'); } });

const ORIGIN = 'http://desk.test';
const ORDER = '5e5e0001-0000-4000-8000-000000000001';
const OWNER = { kind: 'owner', id: 'owner', username: 'owner', email: 'owner', role: 'OWNER' };
const staff = (role) => ({ kind: 'staff', id: 'emp-' + role, username: 'emp-' + role.toLowerCase(), email: 'emp-' + role.toLowerCase(), role, branchId: 'MAIN' });
const WHO = { OWNER, FRONT: staff('FRONT'), BACKEND: staff('BACKEND'), MANAGER: staff('MANAGER'), RATES: staff('RATES') };
const json = (o) => JSON.stringify(o);
// `base` (a JSON object) with a last key "pad" of x, `n` characters in all.
const padded = (n, base) => { const head = base.slice(0, -1) + ',"pad":"', tail = '"}'; const body = head + 'x'.repeat(n - head.length - tail.length) + tail; if (body.length !== n) throw Error('padding ' + body.length + ' != ' + n); return body; };

// [row, label, who, headers override, body]
const LIVE = (o = {}) => json({ branchId: 'MAIN', orderId: ORDER, kiosk: false, mode: 'LIVE', ...o });
const CASES = [
  [1, 'nobody signed in', null, {}, LIVE()],
  [2, 'another origin', 'FRONT', { origin: 'http://elsewhere.test' }, LIVE()],
  [2, 'no X-Xchange-Request', 'FRONT', { 'x-xchange-request': null }, LIVE()],
  [3, 'a body of 2,001 characters', 'FRONT', {}, padded(2001, LIVE())],
  [3, 'a body of exactly 2,000 characters is read', 'FRONT', {}, padded(2000, LIVE())],
  [4, 'a role outside OWNER/FRONT/BACKEND/MANAGER', 'RATES', {}, LIVE()],
  [4, 'the role is asked before the branch', 'RATES', {}, LIVE({ branchId: 'no such branch' })],
  [5, 'the owner names a malformed branch', 'OWNER', {}, LIVE({ branchId: 'no such branch' })],
  [5, 'a staff member naming a malformed branch works on their own', 'MANAGER', {}, LIVE({ branchId: 'no such branch' })],
  [6, 'mode DEMO', 'FRONT', {}, LIVE({ mode: 'DEMO' })],
  [6, 'no mode', 'BACKEND', {}, json({ branchId: 'MAIN', orderId: ORDER })],
  [6, 'mode DEMO with a malformed order id', 'OWNER', {}, LIVE({ mode: 'DEMO', orderId: 'nope' })],
  [7, 'LIVE, switched off', 'FRONT', {}, LIVE()],
  [7, 'LIVE in a branch that does not exist, switched off', 'OWNER', {}, LIVE({ branchId: 'B404' })],
  [7, 'LIVE without a branch: MAIN', 'OWNER', {}, json({ orderId: ORDER, kiosk: false, mode: 'LIVE' })],
  [8, 'LIVE with a malformed order id answers row 7 first', 'FRONT', {}, LIVE({ orderId: 'nope' })],
  [9, 'LIVE for a kiosk order nobody holds answers row 7 first', 'MANAGER', {}, LIVE({ kiosk: true })],
  [10, 'the order asked by FRONT, now by BACKEND: row 7 first', 'BACKEND', {}, LIVE()],
  [11, 'the same caller asks again: row 7 first', 'FRONT', {}, LIVE()],
  [12, 'a first ask of a fresh order: row 7 first', 'OWNER', {}, LIVE({ orderId: '5e5e0001-0000-4000-8000-000000000002', kiosk: true })],
];
const EXPECTED = {
  1: [401, { error: 'กรุณาเข้าสู่ระบบ' }],
  2: [400, { error: 'Request rejected' }],
  3: [400, { error: 'Request too large' }],
  4: [400, { error: 'บัญชีนี้ไม่มีสิทธิ์เปิดลิ้นชัก' }],
  5: [400, { error: 'สาขาไม่ถูกต้อง' }],
  6: [200, { state: 'SIMULATED', physicalResultConfirmed: false }],
};
const DISABLED = [200, { state: 'DISABLED', physicalResultConfirmed: false }];

const out = [];
for (const [row, label, who, override, body] of CASES) {
  let status, answer, cache;
  if (!who) { status = 401; answer = { error: 'กรุณาเข้าสู่ระบบ' }; cache = null; }
  else {
    const headers = { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json' };
    for (const [k, v] of Object.entries(override)) { if (v === null) delete headers[k]; else headers[k] = v; }
    const res = await drawer.api(new Request(ORIGIN + '/api/drawer-passport', { method: 'POST', headers, body }), WHO[who]);
    status = res.status; answer = await res.json(); cache = res.headers.get('cache-control');
  }
  // the two cases that pass the row they test and go on to row 7
  const passes = label.startsWith('a body of exactly') || label.startsWith('a staff member naming');
  const [wantStatus, wantAnswer] = passes ? DISABLED : EXPECTED[row] || DISABLED;
  if (status !== wantStatus || json(answer) !== json(wantAnswer)) throw Error(`row ${row} (${label}): ${status} ${json(answer)}`);
  const sent = {};
  for (const [k, v] of Object.entries(override)) sent[k] = v;
  out.push({ row, label, who, headers: sent, body: body.length > 300 ? JSON.parse(body.slice(0, body.indexOf(',"pad":"')) + '}') : JSON.parse(body), padTo: body.length > 300 ? body.length : null, status, answer, cacheControl: cache });
}
for (let row = 1; row <= 12; row++) if (!out.some((c) => c.row === row)) throw Error('row ' + row + ' has no case');
process.stdout.write(json({
  source: '10-06 bundle (XC-Desk-v2-Passport-Drawer-20261006.zip, sha256 6514b4c8…4375f570): passport-drawer.mjs (sha256 5929e78f…), server.mjs:8153 (the route), server.mjs:8289 (the router\'s 401); XC_PASSPORT_DRAWER unset',
  order: ORDER,
  note: 'body is what was sent, as JSON in the order written. When padTo is a number, the body sent was that JSON text with a last key "pad" of x, padTo characters in all.',
  cases: out,
}, null, 1) + '\n');
