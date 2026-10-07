// Drives the v2 server's two pages that list the documents of one exchange and records what
// they answered.
//   node gen_documents.mjs > ../../tests/fixtures/documents/v2_document_cases.json
//
// It needs `documents.mjs` beside it: fourteen pieces of the deployed bundle's `server.mjs`
// (`xchange-online@3ffc5a5`), unchanged, each followed by an empty line, in this order:
//   5535-5550  identity, authorizeWrite, htmlResponse
//   7083-7092  src/online/app/documents/route.ts                 (esc, GET4)
//   7178-7181  bridgeAuthorized
//   7284-7369  src/online/lib/employees.ts
//   7396-7465  src/online/app/api/staff-bridge/route.ts           (POST6)
//   7467-7482  shims/staff-proxy.mjs
//   7484-7509  src/booking/app/api/staff/[...path]/route.ts       (POST7)
//   7528-7536  src/booking/app/staff/documents/route.ts           (esc2, GET8)
//   7574-7574  esc3
//   7767-7793  src/devices.mjs: KINDS, EXT, documents(url, actor)
//   7804-7890  src/auth.mjs
//   8078-8124  src/staff-routes.mjs
//   8126-8233  src/server.mjs: its settings, the route list, withActor … staticFile
//   8260-8292  src/server.mjs: handle
// The bundle is not in this repository, so this cannot run in CI; its output is checked in and
// is what the tests read.
//
// Every request goes through v2's own `handle(req)`. The pieces are loaded twice: once with
// DEVICES_ENABLED=1 (as the installed desk runs; `GET /documents` is then `documents(url, actor)`
// of devices.mjs) and once without (`GET /documents` is then `GET4`), the second only as a
// record of the other shape. Supplied from here:
//   env.DB, pool   → one in-memory store that interprets the code's own SQL texts
//   execute        → the engine's action `documents`, **written out here from the engine's text**
//                    (engine piece of the same bundle, `runEngine`, the branch `action3 ===
//                    "documents"` and `branchBook_`), after the check `execute` itself makes
//                    before the engine (`server.mjs:6999`): a branch name that is not 1-70 of
//                    A-Za-z0-9- is `สาขาไม่ถูกต้อง` (ticket #139); the branch must exist
//                    (`ไม่พบสาขาที่เลือก กรุณารีเฟรช`), the exchange is looked for by its number
//                    among the branch's LIVE exchanges only (`ไม่พบเอกสารรายการ`), and the answer is
//                    `{ref, links}` with the links that are not empty, in the order passport,
//                    payment proof, receipt. The engine itself is not run by this script.
//   archiveReceipt → always fails. v2 then logs and lists the documents without the receipt
//                    image; making that image is ticket T13b's.
// Sessions are real v2 sign-ins; the fixture names who is asking (`as`).
import fs from 'node:fs';

const here = new URL('.', import.meta.url);
const pieces = fs.readFileSync(new URL('./documents.mjs', here), 'utf8');
for (const needle of ['function htmlResponse(', 'async function GET4(', 'async function POST6(', 'async function GET8(', 'var esc3 = ', 'async function documents(url, actor)', 'async function api2(', 'var routes = [', 'async function handle(']) {
  if (!pieces.includes(needle)) throw Error('documents.mjs: ' + needle + ' not found');
}

// ── the store ────────────────────────────────────────────────────────────
const t = { employees: [], employee_audit: [], employee_sessions: [], owners: [], owner_sessions: [], booking_limits: [], file_records: [] };
const norm = (s) => s.replace(/\s+/g, ' ').trim();
const limit = (key, expires) => { let r = t.booking_limits.find((x) => x.key === key); if (r) r.count += 1; else t.booking_limits.push(r = { key, count: 1, expires }); return [{ count: r.count }]; };
const bump = (id, f) => { const e = t.employees.find((x) => x.id === id); if (e) { f(e); e.version += 1; } return []; };
const SQL = {
  'INSERT INTO employee_audit(id,actor,subject,action,at) VALUES(?,?,?,?,?)': () => [],
  'INSERT INTO employees(id,username,name,role,branch_id,password_hash,active,must_change,version,created_at) VALUES(?,?,?,?,?,?,1,1,1,?)': ([id, username, name, role, branch_id, password_hash, created_at]) => (t.employees.push({ id, username, name, role, branch_id, password_hash, active: 1, must_change: 1, version: 1, created_at }), []),
  'UPDATE employees SET password_hash=?,must_change=0,version=version+1 WHERE id=?': ([h, id]) => bump(id, (e) => { e.password_hash = h; e.must_change = 0; }),
  'INSERT INTO booking_limits(key,count,expires) VALUES(?,1,?) ON CONFLICT(key) DO UPDATE SET count=count+1 RETURNING count': ([key, expires]) => limit(key, expires),
  'INSERT INTO booking_limits(key,count,expires) VALUES($1,1,$2) ON CONFLICT(key) DO UPDATE SET count=booking_limits.count+1 RETURNING count': ([key, expires]) => limit(key, expires),
  'SELECT * FROM employees WHERE username=?': ([u]) => t.employees.filter((e) => e.username === u),
  'INSERT INTO employee_sessions(hash,employee_id,version,expires) VALUES(?,?,?,?)': ([hash, employee_id, version, expires]) => (t.employee_sessions.push({ hash, employee_id, version, expires }), []),
  'SELECT e.* FROM employees e JOIN employee_sessions s ON s.employee_id=e.id WHERE s.hash=? AND s.expires>? AND s.version=e.version AND e.active=1': ([hash, now]) =>
    t.employee_sessions.filter((s) => s.hash === hash && s.expires > now).map((s) => t.employees.find((e) => e.id === s.employee_id && e.version === s.version && e.active === 1)).filter(Boolean),
  'SELECT o.* FROM owners o JOIN owner_sessions s ON s.owner_id=o.id WHERE s.hash=$1 AND s.expires>$2 AND s.version=o.version AND o.active': ([hash, now]) =>
    t.owner_sessions.filter((s) => s.hash === hash && s.expires > now).map((s) => t.owners.find((o) => o.id === s.owner_id && o.version === s.version && o.active)).filter(Boolean),
  'INSERT INTO owner_sessions(hash,owner_id,version,expires) VALUES($1,$2,$3,$4)': ([hash, owner_id, version, expires]) => (t.owner_sessions.push({ hash, owner_id, version, expires }), []),
  'SELECT * FROM owners WHERE username=$1': ([u]) => t.owners.filter((o) => o.username === u),
  'SELECT mime FROM file_records WHERE key=$1 AND branch_id=$2': ([key, branch]) => t.file_records.filter((f) => f.key === key && f.branch_id === branch).map((f) => ({ mime: f.mime })),
};
const run = (sql, args) => { const f = SQL[norm(sql)]; if (!f) throw Error('store: unknown statement: ' + norm(sql)); return structuredClone(f(args.map((v) => (v === undefined ? null : v)))); };
const stmt = (sql, args = []) => ({ bind: (...a) => stmt(sql, a), first: async () => run(sql, args)[0] ?? null, all: async () => ({ results: run(sql, args) }), run: async () => ({ results: run(sql, args) }) });

// ── the exchanges this script describes ──────────────────────────────────
// `ref` and the keys are placeholders: the tests put in the number and the keys v3 gives.
const BRANCHES = ['MAIN', 'BR-2'];
const EXCHANGES = [
  { name: 'cash', branchId: 'MAIN', mode: 'LIVE', passport: 'image/jpeg', slip: null },
  { name: 'transfer', branchId: 'MAIN', mode: 'LIVE', passport: 'image/png', slip: 'application/pdf' },
  { name: 'other', branchId: 'BR-2', mode: 'LIVE', passport: 'image/webp', slip: 'image/jpeg' },
  { name: 'demo', branchId: 'MAIN', mode: 'DEMO', passport: 'image/jpeg', slip: null },
];
for (const x of EXCHANGES) {
  x.ref = '$' + x.name + '.ref';
  x.files = { passport: '$' + x.name + '.passport', slip: x.slip ? '$' + x.name + '.slip' : '', receipt: '$' + x.name + '.receipt' };
  t.file_records.push({ key: x.files.passport, branch_id: x.branchId, mime: x.passport });
  if (x.slip) t.file_records.push({ key: x.files.slip, branch_id: x.branchId, mime: x.slip });
  t.file_records.push({ key: x.files.receipt, branch_id: x.branchId, mime: 'text/html' });
}
const NO_SUCH_BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช', NO_DOCUMENTS = 'ไม่พบเอกสารรายการ', BAD_BRANCH = 'สาขาไม่ถูกต้อง';
async function execute(action, input) {
  if (action !== 'documents') throw Error('execute: these routes never call ' + action);
  const branch = input.branchId || 'MAIN';
  // v2's `execute` (server.mjs:6999), before the engine runs — ticket #139
  if (!/^[A-Za-z0-9-]{1,70}$/.test(branch)) throw Error(BAD_BRANCH);
  if (!BRANCHES.includes(branch)) throw Error(NO_SUCH_BRANCH);
  const x = EXCHANGES.find((e) => e.mode === 'LIVE' && e.branchId === branch && e.ref === input.ref);
  if (!x) throw Error(NO_DOCUMENTS);
  const url = (key) => (key ? '/files/' + key : '');
  return { ref: x.ref, links: [['พาสปอร์ต', url(x.files.passport)], ['หลักฐานชำระเงิน', url(x.files.slip)], ['ใบเสร็จ', url(x.files.receipt)]].filter((l) => l[1]) };
}

globalThis.__v2 = { env: { DB: { prepare: (sql) => stmt(sql) }, BOOKING_BRIDGE_SECRET: 'bridge-secret-of-at-least-32-characters' }, pool: { query: async (sql, args = []) => ({ rows: run(sql, args) }) }, execute };
const prelude = `
import fs2 from "node:fs";
import path2 from "node:path";
import { fileURLToPath } from "node:url";
import { timingSafeEqual } from "node:crypto";
const { env, pool, execute } = globalThis.__v2;
var e = env;
var archiveReceipt = async () => { throw Error("the receipt image is not made here (T13b)"); };
var GET, GET2, GET3, GET5, GET6, GET7, POST, POST2, POST3, POST4, POST5, hookRpc, hookUpload, receipt, api, api3, beforeSave, afterSave, uploadEvidence, index_html_default;
`;
async function load(devices) {
  process.env.AUTH_ENABLED = '1';
  for (const k of ['DEVICES_ENABLED', 'OWNER_ID', 'OWNER_EMAIL', 'DESK_HOST', 'ADMIN_ORIGIN', 'CUSTOMER_HOST']) delete process.env[k];
  if (devices) process.env.DEVICES_ENABLED = '1';
  const bundle = new URL('./documents.bundle.' + (devices ? 'on' : 'off') + '.tmp.mjs', here);
  fs.writeFileSync(bundle, prelude + pieces + '\nexport { handle, passwordHash, manageEmployees };\n');
  try { return await import(bundle.href); } finally { fs.rmSync(bundle); }
}
const realError = console.error;
console.error = () => {}; // v2 logs the failed receipt image on every page

// ── who is asking ────────────────────────────────────────────────────────
const ORIGIN = 'https://desk.example';
const OWNER_PASSWORD = 'owner-password-01', P1 = 'first-password-1', P2 = 'second-password-2';
const tokens = {}, people = {};
const on = await load(true), off = await load(false);
const jsonPost = (v2, path, body, cookie) => v2.handle(new Request(ORIGIN + path, { method: 'POST', headers: { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json', ...(cookie ? { cookie } : {}) }, body: JSON.stringify(body) }));
const cookieOf = (res, which) => { const line = res.headers.getSetCookie().find((c) => c.startsWith('__Host-xc_' + which + '=') && !c.startsWith('__Host-xc_' + which + '=;')); if (!line) throw Error('no ' + which + ' cookie'); return line.split(';')[0].split('=')[1]; };
t.owners.push({ id: 'owner-row-1', username: 'boss', name: 'boss', password_hash: await on.passwordHash(OWNER_PASSWORD), active: true, version: 1, created_at: new Date().toISOString() });
tokens.owner = { owner: cookieOf(await jsonPost(on, '/api/auth', { action: 'login', payload: { username: 'boss', password: OWNER_PASSWORD } }), 'owner') };
for (const [name, role, branchId, change] of [['front', 'FRONT', 'MAIN', true], ['manager', 'MANAGER', 'MAIN', true], ['rates', 'RATES', 'MAIN', true], ['system', 'SYSTEM', 'MAIN', true], ['front2', 'FRONT', 'BR-2', true], ['newcomer', 'FRONT', 'MAIN', false]]) {
  await on.manageEmployees('create', { username: name, name, role, branchId, password: P1 }, 'owner');
  const first = cookieOf(await jsonPost(on, '/api/staff/login', { payload: { username: name, password: P1 } }), 'staff');
  if (change) {
    if (!(await jsonPost(on, '/api/staff/password', { payload: { password: P2 } }, '__Host-xc_staff=' + first)).ok) throw Error('password change of ' + name);
    tokens[name] = { staff: cookieOf(await jsonPost(on, '/api/staff/login', { payload: { username: name, password: P2 } }), 'staff') };
  } else tokens[name] = { staff: first };
  people[name] = { id: 'employee-' + name, role, branchId, mustChange: !change };
}

// ── one request ──────────────────────────────────────────────────────────
function recorder(v2, steps) {
  return async function step(note, r) {
    const method = r.method || 'GET', who = r.as || [];
    const headers = {};
    const cookie = who.flatMap((n) => [tokens[n].owner ? '__Host-xc_owner=' + tokens[n].owner : '', tokens[n].staff ? '__Host-xc_staff=' + tokens[n].staff : '']).filter(Boolean).join('; ');
    if (cookie) headers.cookie = cookie;
    const res = await v2.handle(new Request(ORIGIN + r.path, { method, headers, redirect: 'manual', ...(method === 'POST' ? { body: '' } : {}) }));
    const text = await res.text();
    const response = { status: res.status, headers: {}, text };
    for (const h of ['content-type', 'cache-control', 'x-content-type-options', 'referrer-policy', 'location']) if (res.headers.get(h) !== null) response.headers[h] = res.headers.get(h);
    steps.push({ note, request: { method, path: r.path, as: who }, response });
    return response;
  };
}
const is = (res, status, note, text) => { if (res.status !== status || (text !== undefined && res.text !== text)) throw Error(note + ': expected ' + status + ' ' + (text ?? '') + ', got ' + JSON.stringify(res).slice(0, 400)); return res; };
const X = Object.fromEntries(EXCHANGES.map((x) => [x.name, encodeURIComponent(x.ref)]));

const steps = [];
const step = recorder(on, steps);
// ── GET /documents (devices on) ──────────────────────────────────────────
is(await step('documents: nobody signed in is sent to the sign-in page', { path: '/documents?ref=' + X.cash }), 303, 'nobody');
is(await step('documents: a first-password employee is sent there too', { path: '/documents?ref=' + X.cash, as: ['newcomer'] }), 303, 'newcomer');
is(await step('POST /documents is not a route', { method: 'POST', path: '/documents?ref=' + X.cash, as: ['owner'] }), 404, 'post', 'Not found');
is(await step('documents, owner: a cash exchange — passport and receipt', { path: '/documents?ref=' + X.cash, as: ['owner'] }), 200, 'o cash');
is(await step('documents, owner: the same with the branch named', { path: '/documents?branch=MAIN&ref=' + X.cash, as: ['owner'] }), 200, 'o cash main');
is(await step('documents, owner: ?branch= (empty) is MAIN', { path: '/documents?branch=&ref=' + X.cash, as: ['owner'] }), 200, 'o cash empty');
is(await step('documents, owner: a transfer — passport, payment proof, receipt', { path: '/documents?ref=' + X.transfer, as: ['owner'] }), 200, 'o transfer');
is(await step('documents, owner: an exchange of another branch, with that branch named', { path: '/documents?branch=BR-2&ref=' + X.other, as: ['owner'] }), 200, 'o other');
is(await step('documents, owner: an exchange of another branch without naming it', { path: '/documents?ref=' + X.other, as: ['owner'] }), 404, 'o other main', NO_DOCUMENTS);
is(await step('documents, owner: the wrong branch named', { path: '/documents?branch=BR-2&ref=' + X.cash, as: ['owner'] }), 404, 'o wrong', NO_DOCUMENTS);
is(await step('documents, owner: a branch that does not exist', { path: '/documents?branch=NOWHERE&ref=' + X.cash, as: ['owner'] }), 404, 'o nowhere', NO_SUCH_BRANCH);
is(await step('documents, owner: a name that cannot be a branch id is refused before the branch is looked for', { path: '/documents?branch=MAIN_1&ref=' + X.cash, as: ['owner'] }), 404, 'o bad', BAD_BRANCH);
is(await step('documents, owner: a number nobody was given', { path: '/documents?ref=XC-NOPE', as: ['owner'] }), 404, 'o unknown', NO_DOCUMENTS);
is(await step('documents, owner: no ref', { path: '/documents', as: ['owner'] }), 404, 'o none', NO_DOCUMENTS);
is(await step('documents, owner: an empty ref', { path: '/documents?ref=', as: ['owner'] }), 404, 'o empty', NO_DOCUMENTS);
is(await step('documents, owner: a demo exchange has no documents page', { path: '/documents?ref=' + X.demo, as: ['owner'] }), 404, 'o demo', NO_DOCUMENTS);
is(await step('documents, owner and an employee signed in together: the owner (another branch may be named)', { path: '/documents?branch=BR-2&ref=' + X.other, as: ['owner', 'front'] }), 200, 'both');
is(await step('documents, Front: an exchange of the own branch', { path: '/documents?ref=' + X.transfer, as: ['front'] }), 200, 'f');
is(await step('documents, Front: ?branch=BR-2 is replaced by the own branch — the own exchange is found', { path: '/documents?branch=BR-2&ref=' + X.cash, as: ['front'] }), 200, 'f forced');
is(await step('documents, Front: an exchange of another branch is not', { path: '/documents?branch=BR-2&ref=' + X.other, as: ['front'] }), 404, 'f other', NO_DOCUMENTS);
is(await step('documents, Front of BR-2: the own branch without naming it', { path: '/documents?ref=' + X.other, as: ['front2'] }), 200, 'f2');
is(await step('documents, Manager', { path: '/documents?ref=' + X.cash, as: ['manager'] }), 200, 'm');
is(await step('documents, Rates: the list is shown to every signed-in employee at this page', { path: '/documents?ref=' + X.cash, as: ['rates'] }), 200, 'r');
is(await step('documents, System: the same', { path: '/documents?ref=' + X.cash, as: ['system'] }), 200, 's');

// ── GET /staff/documents ─────────────────────────────────────────────────
is(await step('staff documents: no session is sent to the staff page', { path: '/staff/documents?ref=' + X.cash }), 302, 'sd none');
is(await step('staff documents: the owner is not an employee', { path: '/staff/documents?ref=' + X.cash, as: ['owner'] }), 302, 'sd owner');
is(await step('staff documents: a first-password employee', { path: '/staff/documents?ref=' + X.cash, as: ['newcomer'] }), 302, 'sd newcomer');
is(await step('POST /staff/documents is not a route', { method: 'POST', path: '/staff/documents?ref=' + X.cash, as: ['front'] }), 404, 'sd post', 'Not found');
is(await step('staff documents, Front: a cash exchange', { path: '/staff/documents?ref=' + X.cash, as: ['front'] }), 200, 'sd cash');
is(await step('staff documents, Front: a transfer', { path: '/staff/documents?ref=' + X.transfer, as: ['front'] }), 200, 'sd transfer');
is(await step('staff documents, Front with the owner signed in too', { path: '/staff/documents?ref=' + X.cash, as: ['owner', 'front'] }), 200, 'sd both');
is(await step('staff documents, Manager', { path: '/staff/documents?ref=' + X.transfer, as: ['manager'] }), 200, 'sd m');
is(await step('staff documents, Front of BR-2', { path: '/staff/documents?ref=' + X.other, as: ['front2'] }), 200, 'sd f2');
is(await step('staff documents, Front: ?branch=BR-2 is ignored — an exchange of another branch is not found', { path: '/staff/documents?branch=BR-2&ref=' + X.other, as: ['front'] }), 302, 'sd other');
is(await step('staff documents, Front: a number nobody was given', { path: '/staff/documents?ref=XC-NOPE', as: ['front'] }), 302, 'sd unknown');
is(await step('staff documents, Front: no ref', { path: '/staff/documents', as: ['front'] }), 302, 'sd none ref');
is(await step('staff documents, Front: a demo exchange', { path: '/staff/documents?ref=' + X.demo, as: ['front'] }), 302, 'sd demo');
is(await step('staff documents, Rates: this role reads no documents here', { path: '/staff/documents?ref=' + X.cash, as: ['rates'] }), 302, 'sd rates');
is(await step('staff documents, System: the same', { path: '/staff/documents?ref=' + X.cash, as: ['system'] }), 302, 'sd system');

// ── GET /documents with devices off: the other shape, as a record only ───
const offSteps = [];
const stepOff = recorder(off, offSteps);
is(await stepOff('devices off — documents, owner: a transfer', { path: '/documents?ref=' + X.transfer, as: ['owner'] }), 200, 'off transfer');
is(await stepOff('devices off — documents, owner: a number nobody was given', { path: '/documents?ref=XC-NOPE', as: ['owner'] }), 404, 'off unknown', NO_DOCUMENTS);
is(await stepOff('devices off — documents, Front', { path: '/documents?ref=' + X.cash, as: ['front'] }), 200, 'off front');
is(await stepOff('devices off — documents, owner: a name that is no branch', { path: '/documents?branch=MAIN_1&ref=' + X.cash, as: ['owner'] }), 404, 'off bad');
console.error = realError;

process.stdout.write(JSON.stringify({
  source: 'xchange-online@3ffc5a5 server.mjs through its own handle(req), driven by api/tools/v2-fixtures/gen_documents.mjs',
  note: 'In a path and in a page, `$<name>.ref` stands for the document number of the exchange <name> and `$<name>.passport` / `.slip` / `.receipt` for the keys of its files (in a path the `$` is written %24). `as` names who is signed in (see `people`; "owner" is the owner). `steps` were recorded with DEVICES_ENABLED=1, as the installed desk runs. `devicesOff` is the same page as v2 serves it without devices — a record of the other shape, not a requirement.',
  branches: BRANCHES,
  exchanges: EXCHANGES.map((x) => ({ name: x.name, branchId: x.branchId, mode: x.mode, passport: x.passport, slip: x.slip })),
  people,
  counts: { requests: steps.length, byStatus: Object.fromEntries([200, 302, 303, 404].map((c) => [c, steps.filter((s) => s.response.status === c).length])), devicesOff: offSteps.length },
  steps,
  devicesOff: offSteps,
}, null, 1) + '\n');
