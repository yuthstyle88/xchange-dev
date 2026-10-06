// Drives the HTTP entry of the v2 server for sign-in, the staff door and the employee screen,
// and records what it answered.
//   node gen_auth_http.mjs > ../../tests/fixtures/auth/v2_auth_http_cases.json
//
// It needs `auth_http.mjs` beside it: ten pieces of the deployed bundle's `server.mjs`
// (`xchange-online@3ffc5a5`), unchanged, each followed by an empty line, in this order:
//   5535-5544  identity, authorizeWrite
//   7178-7181  bridgeAuthorized
//   7284-7369  src/online/lib/employees.ts
//   7370-7395  src/online/app/api/employees/route.ts            (POST5)
//   7396-7465  src/online/app/api/staff-bridge/route.ts          (POST6)
//   7467-7482  shims/staff-proxy.mjs
//   7484-7509  src/booking/app/api/staff/[...path]/route.ts      (POST7 only)
//   7804-7890  src/auth.mjs
//   8126-8233  src/server.mjs: its settings, the route list, withActor … staticFile
//   8260-8292  src/server.mjs: handle
// The bundle is not in this repository, so this cannot run in CI; its output is checked in and
// is what the tests read.
//
// The pieces are loaded as one module, so every request goes through v2's own `handle(req)`:
// its route list, its 401/403 gate, then the handler. Supplied from here:
//   env.DB, pool   → one in-memory store that interprets the code's own SQL texts; it is not
//                    PostgreSQL, and it throws on any statement it does not know
//   execute        → the engine. Only three calls reach it from these routes: `adminSnapshot`
//                    (answered with the branch this script describes, or the engine's sentence
//                    for a branch that does not exist), `opsSetEntity` and `opsEnable`
//                    (answered with a marker; every call is recorded)
//   the page and file handlers the route list names but these requests never reach → undefined
// AUTH_ENABLED=1, DEVICES_ENABLED unset, as the deployed desk. The clock and the random source
// are node's own: tokens and ids are written as $tN / $eN, and nothing recorded depends on time
// except the 15-minute attempt window (the run fails loudly if it crosses one).
import fs from 'node:fs';
import crypto from 'node:crypto';

const here = new URL('.', import.meta.url);
const pieces = fs.readFileSync(new URL('./auth_http.mjs', here), 'utf8');
for (const needle of ['function authorizeWrite(', 'function bridgeAuthorized(', 'async function manageEmployees(', 'async function POST5(', 'async function POST6(', 'async function staffProxy(', 'async function POST7(', 'async function api2(', 'var routes = [', 'async function handle(']) {
  if (!pieces.includes(needle)) throw Error('auth_http.mjs: ' + needle + ' not found');
}

// ── the store ────────────────────────────────────────────────────────────
const t = { employees: [], employee_audit: [], employee_sessions: [], owners: [], owner_sessions: [], booking_limits: [] };
const norm = (s) => s.replace(/\s+/g, ' ').trim();
const limit = (key, expires) => {
  let r = t.booking_limits.find((x) => x.key === key);
  if (r) r.count += 1; else t.booking_limits.push(r = { key, count: 1, expires });
  return [{ count: r.count }];
};
const bump = (id, f) => { const e = t.employees.find((x) => x.id === id); if (e) { f(e); e.version += 1; } return []; };
const SQL = {
  'INSERT INTO employee_audit(id,actor,subject,action,at) VALUES(?,?,?,?,?)': ([id, actor, subject, action, at]) => (t.employee_audit.push({ id, actor, subject, action, at }), []),
  'SELECT * FROM employees ORDER BY username': () => [...t.employees].sort((a, b) => (a.username < b.username ? -1 : a.username > b.username ? 1 : 0)),
  'INSERT INTO employees(id,username,name,role,branch_id,password_hash,active,must_change,version,created_at) VALUES(?,?,?,?,?,?,1,1,1,?)': ([id, username, name, role, branch_id, password_hash, created_at]) => {
    if (t.employees.some((e) => e.username === username)) throw Error('duplicate key value violates unique constraint "employees_username_key"');
    t.employees.push({ id, username, name, role, branch_id, password_hash, active: 1, must_change: 1, version: 1, created_at });
    return [];
  },
  'SELECT * FROM employees WHERE id=?': ([id]) => t.employees.filter((e) => e.id === id),
  'UPDATE employees SET password_hash=?,must_change=1,version=version+1 WHERE id=?': ([h, id]) => bump(id, (e) => { e.password_hash = h; e.must_change = 1; }),
  'UPDATE employees SET active=0,version=version+1 WHERE id=?': ([id]) => bump(id, (e) => { e.active = 0; }),
  'UPDATE employees SET password_hash=?,must_change=0,version=version+1 WHERE id=?': ([h, id]) => bump(id, (e) => { e.password_hash = h; e.must_change = 0; }),
  'INSERT INTO booking_limits(key,count,expires) VALUES(?,1,?) ON CONFLICT(key) DO UPDATE SET count=count+1 RETURNING count': ([key, expires]) => limit(key, expires),
  'INSERT INTO booking_limits(key,count,expires) VALUES($1,1,$2) ON CONFLICT(key) DO UPDATE SET count=booking_limits.count+1 RETURNING count': ([key, expires]) => limit(key, expires),
  'SELECT * FROM employees WHERE username=?': ([u]) => t.employees.filter((e) => e.username === u),
  'INSERT INTO employee_sessions(hash,employee_id,version,expires) VALUES(?,?,?,?)': ([hash, employee_id, version, expires]) => (t.employee_sessions.push({ hash, employee_id, version, expires }), []),
  'SELECT e.* FROM employees e JOIN employee_sessions s ON s.employee_id=e.id WHERE s.hash=? AND s.expires>? AND s.version=e.version AND e.active=1': ([hash, now]) =>
    t.employee_sessions.filter((s) => s.hash === hash && s.expires > now).map((s) => t.employees.find((e) => e.id === s.employee_id && e.version === s.version && e.active === 1)).filter(Boolean),
  'DELETE FROM employee_sessions WHERE hash=?': ([hash]) => { t.employee_sessions = t.employee_sessions.filter((s) => s.hash !== hash); return []; },
  'SELECT o.* FROM owners o JOIN owner_sessions s ON s.owner_id=o.id WHERE s.hash=$1 AND s.expires>$2 AND s.version=o.version AND o.active': ([hash, now]) =>
    t.owner_sessions.filter((s) => s.hash === hash && s.expires > now).map((s) => t.owners.find((o) => o.id === s.owner_id && o.version === s.version && o.active)).filter(Boolean),
  'INSERT INTO owner_sessions(hash,owner_id,version,expires) VALUES($1,$2,$3,$4)': ([hash, owner_id, version, expires]) => (t.owner_sessions.push({ hash, owner_id, version, expires }), []),
  'SELECT * FROM owners WHERE username=$1': ([u]) => t.owners.filter((o) => o.username === u),
  'DELETE FROM owner_sessions WHERE hash=$1': ([hash]) => { t.owner_sessions = t.owner_sessions.filter((s) => s.hash !== hash); return []; },
};
const run = (sql, args) => {
  const f = SQL[norm(sql)];
  if (!f) throw Error('store: unknown statement: ' + norm(sql));
  return structuredClone(f(args.map((v) => (v === undefined ? null : v))));
};
const stmt = (sql, args = []) => ({
  bind: (...a) => stmt(sql, a),
  first: async () => run(sql, args)[0] ?? null,
  all: async () => ({ results: run(sql, args) }),
  run: async () => ({ results: run(sql, args) }),
});

// ── the engine, as far as these routes reach it ──────────────────────────
const NO_SUCH_BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช'; // the engine's sentence; pinned from v2 by ticket T5's fixture
let branches = {}; // branch id → the `operations` part of its snapshot
let engineCalls = [];
async function execute(action, payload, actor) {
  if (action === 'adminSnapshot') {
    const id = payload.branchId || 'MAIN';
    if (!(id in branches)) throw Error(NO_SUCH_BRANCH);
    return { operations: structuredClone(branches[id]) };
  }
  if (action === 'opsSetEntity' || action === 'opsEnable') {
    engineCalls.push({ action, payload: symbols(structuredClone(payload)), actorId: actor.id });
    return { saved: true, by: 'custody', action };
  }
  throw Error('execute: these routes never call ' + action);
}

globalThis.__v2 = { env: { DB: { prepare: (sql) => stmt(sql) }, BOOKING_BRIDGE_SECRET: 'bridge-secret-of-at-least-32-characters' }, pool: { query: async (sql, args = []) => ({ rows: run(sql, args) }) }, execute };
process.env.AUTH_ENABLED = '1';
delete process.env.DEVICES_ENABLED; delete process.env.OWNER_ID; delete process.env.OWNER_EMAIL;
delete process.env.DESK_HOST; delete process.env.ADMIN_ORIGIN; delete process.env.CUSTOMER_HOST;

const prelude = `
import fs2 from "node:fs";
import path2 from "node:path";
import { fileURLToPath } from "node:url";
import { timingSafeEqual } from "node:crypto";
const { env, pool, execute } = globalThis.__v2;
var e = env;
var GET, GET2, GET3, GET4, GET5, GET6, GET7, GET8, POST, POST2, POST3, POST4, hookRpc, hookUpload, rpc, upload, file, documents, receipt, api, api3, beforeSave, afterSave, uploadEvidence, index_html_default;
`;
const bundle = new URL('./auth_http.bundle.tmp.mjs', here);
fs.writeFileSync(bundle, prelude + pieces + '\nexport { handle, passwordHash };\n');
let v2;
try { v2 = await import(bundle.href); } finally { fs.rmSync(bundle); }

// ── symbols: the fixture never carries a v2 id or token ───────────────────
const tokenName = new Map(), tokenValue = new Map();
const nameToken = (hex) => { if (!tokenName.has(hex)) { const n = '$t' + (tokenName.size + 1); tokenName.set(hex, n); tokenValue.set(n, hex); } return tokenName.get(hex); };
const realToken = (x) => (typeof x === 'string' && tokenValue.has(x) ? tokenValue.get(x) : x);
const employeeName = (id) => { const i = t.employees.findIndex((e) => e.id === id); return i < 0 ? id : '$e' + (i + 1); };
const realEmployee = (x) => { const m = /^\$e(\d+)$/.exec(typeof x === 'string' ? x : ''); return m && t.employees[m[1] - 1] ? t.employees[m[1] - 1].id : x; };
function symbols(x) { return typeof x === 'string' ? employeeName(x) : Array.isArray(x) ? x.map(symbols) : x && typeof x === 'object' ? Object.fromEntries(Object.entries(x).map(([k, v]) => [k, symbols(v)])) : x; }
const resolve = (x) => (typeof x === 'string' ? realEmployee(x) : Array.isArray(x) ? x.map(resolve) : x && typeof x === 'object' ? Object.fromEntries(Object.entries(x).map(([k, v]) => [k, resolve(v)])) : x);

// ── one request ──────────────────────────────────────────────────────────
const ORIGIN = 'https://desk.example';
const stories = [];
let steps;
/** `r`: { method?, path, origin?, marker?, cookies?: {owner, staff}, headers?, json?, raw? } — `origin` and `marker`
 *  default to what the kept pages send; `null` leaves the header out. */
async function step(note, r) {
  const headers = { ...(r.headers || {}) };
  const origin = r.origin === undefined ? ORIGIN : r.origin, marker = r.marker === undefined ? '1' : r.marker;
  if (origin !== null) headers.origin = origin;
  if (marker !== null) headers['x-xchange-request'] = marker;
  const c = r.cookies || {};
  const cookie = [c.owner != null ? '__Host-xc_owner=' + realToken(c.owner) : '', c.staff != null ? '__Host-xc_staff=' + realToken(c.staff) : ''].filter(Boolean).join('; ');
  if (cookie) headers.cookie = cookie;
  const method = r.method || 'POST';
  const body = method === 'GET' ? undefined : r.raw !== undefined ? r.raw : r.json !== undefined ? JSON.stringify(resolve(r.json)) : '';
  if (body !== undefined && r.raw === undefined) headers['content-type'] = 'application/json';
  const before = engineCalls.length;
  const res = await v2.handle(new Request(ORIGIN + r.path, { method, headers, body }));
  const text = await res.text();
  const type = res.headers.get('content-type') || '';
  const setCookies = {};
  for (const line of res.headers.getSetCookie()) {
    const m = /^__Host-xc_(owner|staff)=([^;]*); Path=\/; HttpOnly; Secure; SameSite=Strict; Max-Age=(\d+)$/.exec(line);
    if (!m) throw Error('unexpected Set-Cookie: ' + line);
    if (m[1] in setCookies) throw Error('a cookie set twice: ' + line);
    setCookies[m[1]] = m[2] === '' ? { cleared: true, maxAge: Number(m[3]) } : { token: nameToken(m[2]), maxAge: Number(m[3]) };
  }
  const request = { method, path: r.path, origin, marker, ...(r.cookies ? { cookies: r.cookies } : {}), ...(r.headers ? { headers: r.headers } : {}), ...(r.raw !== undefined ? { raw: r.raw.length > 300 ? { repeat: r.raw[0], times: r.raw.length } : r.raw } : r.json !== undefined ? { json: r.json } : {}) };
  if (r.raw !== undefined && r.raw.length > 300 && r.raw !== r.raw[0].repeat(r.raw.length)) throw Error('a long raw body must be one repeated character');
  const s = {
    note, request,
    response: {
      status: res.status, contentType: type, cacheControl: res.headers.get('cache-control'), setCookies,
      ...(type.includes('application/json') ? { json: symbols(JSON.parse(text)) } : { text }),
    },
  };
  if (engineCalls.length > before) s.custodyCalls = engineCalls.slice(before);
  steps.push(s);
  return s.response;
}
const OWNER_PASSWORD = 'owner-password-01';
async function story(name, about, setup, f) {
  for (const k of Object.keys(t)) t[k] = [];
  tokenName.clear(); tokenValue.clear(); engineCalls = [];
  branches = structuredClone(setup.branches);
  steps = [];
  if (Date.now() % 9e5 > 9e5 - 120_000) throw Error('less than two minutes to the next attempt window; run again later');
  t.owners.push({ id: 'owner-row-1', username: 'boss', name: 'boss', password_hash: await v2.passwordHash(OWNER_PASSWORD), active: true, version: 1, created_at: new Date().toISOString() });
  await f();
  stories.push({ name, about, owner: { username: 'boss', password: OWNER_PASSWORD }, branches: setup.branches, steps });
}
/** Changes what the engine's snapshot says about custody in a branch, from here on. */
const custody = (branch, operations) => { branches[branch] = resolve(operations); steps.push({ note: 'custody of ' + branch + ' is now as given', custody: { branch, operations } }); };
const is = (res, status, error, note) => { if (res.status !== status || (error !== undefined && res.json?.error !== error)) throw Error(note + ': expected ' + status + ' ' + (error ?? '') + ', got ' + JSON.stringify(res)); return res; };
const ok = (res, note) => { if (res.status !== 200 || res.json?.error) throw Error(note + ': expected v2 to accept, got ' + JSON.stringify(res)); return res; };

const BAD = 'คำขอไม่ถูกต้อง', WRONG = 'ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง', LIMIT = 'ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที';
const SIGN_IN = 'กรุณาเข้าสู่ระบบ', OWNER_ONLY = 'เฉพาะเจ้าของระบบเท่านั้น', AUTH_REQUIRED = 'EMPLOYEE_AUTH_REQUIRED';
const P1 = 'first-password-1', P2 = 'second-password-2';
const person = (over = {}) => ({ username: 'somchai', name: 'สมชาย ใจดี', role: 'FRONT', branchId: 'MAIN', password: P1, ...over });
const IDLE = { enabled: false, custodian: '', requests: [] };
const auth = (action, payload, more = {}) => ({ path: '/api/auth', json: payload === undefined ? { action } : { action, payload }, ...more });
const emp = (action, payload, more = {}) => ({ path: '/api/employees', json: payload === undefined ? { action } : { action, payload }, ...more });
const staff = (op, payload, more = {}) => ({ path: '/api/staff/' + op, ...(payload === undefined ? {} : { json: { payload } }), ...more });

await story('auth', 'POST /api/auth: the request checks, status, sign-in of the owner and of an employee, sign-out.', { branches: { MAIN: IDLE } }, async () => {
  is(await step('GET is not this route', { method: 'GET', path: '/api/auth' }), 404, undefined, 'get');
  is(await step('no Origin header', auth('status', {}, { origin: null })), 400, BAD, 'no origin');
  is(await step('another origin', auth('status', {}, { origin: 'https://evil.example' })), 400, BAD, 'other origin');
  is(await step('the same host over http', auth('status', {}, { origin: 'http://desk.example' })), 400, BAD, 'http origin');
  is(await step('no X-XChange-Request header', auth('status', {}, { marker: null })), 400, BAD, 'no marker');
  is(await step('X-XChange-Request: true', auth('status', {}, { marker: 'true' })), 400, BAD, 'marker true');
  is(await step('the checks come before the body is read: bad origin and a body that is too long', { path: '/api/auth', raw: 'x'.repeat(4001), origin: null }), 400, BAD, 'order');
  is(await step('a body of 4001 characters', { path: '/api/auth', raw: ' '.repeat(4001) }), 400, 'ข้อมูลคำขอใหญ่เกินกำหนด', '4001');
  is(await step('an empty body', { path: '/api/auth', raw: '' }), 400, BAD, 'empty');
  is(await step('an empty object', { path: '/api/auth', json: {} }), 400, BAD, 'no action');
  is(await step('an unknown action', auth('register', {})), 400, BAD, 'unknown');
  ok(await step('status, nobody', auth('status', {})), 'status');
  ok(await step('status without a payload', auth('status')), 'status no payload');
  ok(await step('status with cookies that are not sessions', auth('status', {}, { cookies: { owner: 'nonsense', staff: 'f'.repeat(64) } })), 'status junk');
  is(await step('sign-in without a payload', auth('login')), 400, WRONG, 'login no payload');
  is(await step('sign-in, wrong password', auth('login', { username: 'boss', password: 'wrong-password-1' })), 400, WRONG, 'wrong');

  const o = ok(await step('the owner signs in: the owner cookie is set for twelve hours and the staff cookie cleared', auth('login', { username: ' Boss ', password: OWNER_PASSWORD })), 'owner login');
  const ot = o.setCookies.owner.token;
  ok(await step('status with the owner cookie', auth('status', {}, { cookies: { owner: ot } })), 'owner status');
  ok(await step('the owner creates an employee', emp('create', person(), { cookies: { owner: ot } })), 'create');
  const s1 = ok(await step('the employee signs in at the shared door: the staff cookie is set and the owner cookie cleared; mustChange', auth('login', { username: 'somchai', password: P1 })), 'emp login');
  const st = s1.setCookies.staff.token;
  ok(await step('status with a first-password session: nobody', auth('status', {}, { cookies: { staff: st } })), 'must change status');
  ok(await step('the employee changes the first password at the staff door', staff('password', { password: P2 }, { cookies: { staff: st } })), 'password');
  const s2 = ok(await step('…and signs in again: mustChange false', auth('login', { username: 'somchai', password: P2 })), 'emp login 2');
  const st2 = s2.setCookies.staff.token;
  ok(await step('status with the staff cookie', auth('status', {}, { cookies: { staff: st2 } })), 'staff status');
  ok(await step('status with both cookies: the owner', auth('status', {}, { cookies: { owner: ot, staff: st2 } })), 'both');
  ok(await step('sign-out with both cookies: both are cleared', auth('logout', {}, { cookies: { owner: ot, staff: st2 } })), 'logout');
  ok(await step('the owner session is over', auth('status', {}, { cookies: { owner: ot } })), 'owner over');
  ok(await step('the staff session is over', auth('status', {}, { cookies: { staff: st2 } })), 'staff over');
  ok(await step('sign-out with no cookie still clears both', auth('logout')), 'logout none');
  ok(await step('sign-out needs the two request headers too', auth('logout', {}, { cookies: { owner: 'x' } })), 'logout junk');
  is(await step('…without them it is refused and nothing is cleared', auth('logout', {}, { marker: null })), 400, BAD, 'logout no marker');
});

await story('staff door', 'POST /api/staff/{login,logout,password,me}.', { branches: { MAIN: IDLE } }, async () => {
  const o = ok(await step('the owner signs in', auth('login', { username: 'boss', password: OWNER_PASSWORD })), 'owner login');
  const ot = o.setCookies.owner.token;
  ok(await step('…and creates an employee', emp('create', person(), { cookies: { owner: ot } })), 'create');

  is(await step('GET is not this route', { method: 'GET', path: '/api/staff/me' }), 404, undefined, 'get');
  is(await step('no Origin header', staff('me', {}, { origin: null })), 400, BAD, 'no origin');
  is(await step('no X-XChange-Request header', staff('login', { username: 'somchai', password: P1 }, { marker: null })), 400, BAD, 'no marker');
  is(await step('a body of 120001 characters', { path: '/api/staff/login', raw: ' '.repeat(120001) }), 400, 'ข้อมูลมากเกินไป', 'long');
  is(await step('me without a cookie', staff('me', {})), 401, AUTH_REQUIRED, 'me none');
  is(await step('me with an empty body and no cookie', { path: '/api/staff/me', raw: '' }), 401, AUTH_REQUIRED, 'me empty');
  is(await step('me with the owner cookie only', staff('me', {}, { cookies: { owner: ot } })), 401, AUTH_REQUIRED, 'me owner');
  is(await step('sign-in, wrong password', staff('login', { username: 'somchai', password: 'wrong-password-1' })), 400, WRONG, 'wrong');
  is(await step('sign-in without a payload', staff('login')), 400, WRONG, 'no payload');
  is(await step('the owner cannot sign in here', staff('login', { username: 'boss', password: OWNER_PASSWORD })), 400, WRONG, 'owner here');

  const a = ok(await step('sign-in: the answer carries the employee and no token; only the staff cookie is set, an owner cookie sent along is left alone', staff('login', { username: ' SomChai ', password: P1 }, { cookies: { owner: ot } })), 'login');
  const t1 = a.setCookies.staff.token;
  ok(await step('me with a first-password session answers the employee', staff('me', {}, { cookies: { staff: t1 } })), 'me must change');
  ok(await step('me with an empty body', { path: '/api/staff/me', raw: '', cookies: { staff: t1 } }), 'me empty body');
  is(await step('password: too short — refused, the cookie is kept', staff('password', { password: 'short' }, { cookies: { staff: t1 } })), 400, 'รหัสผ่านต้องมี 12–128 ตัวอักษร', 'short');
  is(await step('password without a payload', staff('password', undefined, { cookies: { staff: t1 } })), 400, 'รหัสผ่านต้องมี 12–128 ตัวอักษร', 'no payload');
  is(await step('password without a cookie', staff('password', { password: P2 })), 401, AUTH_REQUIRED, 'pw none');
  ok(await step('password changed: sign in again, the staff cookie is cleared', staff('password', { password: P2 }, { cookies: { staff: t1 } })), 'changed');
  is(await step('the old session is over', staff('me', {}, { cookies: { staff: t1 } })), 401, AUTH_REQUIRED, 'old');
  const b = ok(await step('sign-in with the new password', staff('login', { username: 'somchai', password: P2 })), 'login 2');
  const t2 = b.setCookies.staff.token;
  ok(await step('me', staff('me', {}, { cookies: { staff: t2 } })), 'me');
  ok(await step('the owner session was not touched by any of this', auth('status', {}, { cookies: { owner: ot } })), 'owner still');
  ok(await step('sign-out: the staff cookie is cleared', staff('logout', {}, { cookies: { staff: t2, owner: ot } })), 'logout');
  is(await step('the session is over', staff('me', {}, { cookies: { staff: t2 } })), 401, AUTH_REQUIRED, 'over');
  ok(await step('sign-out without a cookie is accepted too', staff('logout', {})), 'logout none');
  ok(await step('sign-out with an empty body', { path: '/api/staff/logout', raw: '' }), 'logout empty');
});

await story('employees', 'POST /api/employees: who may call it, the request checks, and what the route adds around the account actions.', { branches: { MAIN: IDLE, 'BR-2': IDLE } }, async () => {
  is(await step('nobody signed in', emp('list', {})), 401, SIGN_IN, 'nobody');
  is(await step('nobody signed in, and the request headers missing: the sign-in check comes first', emp('list', {}, { origin: null, marker: null })), 401, SIGN_IN, 'nobody first');
  is(await step('GET is not this route', { method: 'GET', path: '/api/employees' }), 404, undefined, 'get');
  const ot = ok(await step('the owner signs in', auth('login', { username: 'boss', password: OWNER_PASSWORD })), 'owner login').setCookies.owner.token;
  const own = { cookies: { owner: ot } };
  is(await step('no Origin header', emp('list', {}, { ...own, origin: null })), 400, 'ORIGIN_REJECTED', 'origin');
  is(await step('no X-XChange-Request header', emp('list', {}, { ...own, marker: null })), 400, 'ORIGIN_REJECTED', 'marker');
  is(await step('a body of 6001 characters', { path: '/api/employees', raw: ' '.repeat(6001), ...own }), 400, 'ข้อมูลมากเกินไป', 'long');
  ok(await step('list, nobody yet', emp('list', {}, own)), 'list');
  ok(await step('list without a payload', emp('list', undefined, own)), 'list no payload');

  is(await step('create in a branch that does not exist', emp('create', person({ branchId: 'NOWHERE' }), own)), 400, NO_SUCH_BRANCH, 'no branch');
  is(await step('create in a branch that does not exist, with other fields wrong too: the branch is looked at first', emp('create', person({ branchId: 'NOWHERE', role: 'NOPE', password: 'short' }), own)), 400, NO_SUCH_BRANCH, 'branch first');
  is(await step('create without a branch: the fields are refused', emp('create', person({ branchId: undefined }), own)), 400, 'ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง', 'no branch id');
  is(await step('create with a bad role', emp('create', person({ role: 'NOPE' }), own)), 400, 'ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง', 'role');
  is(await step('create with a short password', emp('create', person({ password: 'short' }), own)), 400, 'รหัสผ่านต้องมี 12–128 ตัวอักษร', 'short');
  ok(await step('create a Front in MAIN', emp('create', person(), own)), 'create 1');
  ok(await step('create a Manager in MAIN', emp('create', person({ username: 'malee', name: 'มาลี', role: 'MANAGER' }), own)), 'create 2');
  ok(await step('create a Front in BR-2', emp('create', person({ username: 'anan', name: 'อนันต์', branchId: 'BR-2' }), own)), 'create 3');
  ok(await step('list', emp('list', {}, own)), 'list 3');

  // an employee is not the owner
  const st0 = ok(await step('somchai signs in (first password)', staff('login', { username: 'somchai', password: P1 })), 'emp login').setCookies.staff.token;
  is(await step('a first-password session is nobody here', emp('list', {}, { cookies: { staff: st0 } })), 401, SIGN_IN, 'must change');
  ok(await step('…changes the password', staff('password', { password: P2 }, { cookies: { staff: st0 } })), 'pw');
  const st = ok(await step('…and signs in again', staff('login', { username: 'somchai', password: P2 })), 'emp login 2').setCookies.staff.token;
  is(await step('an employee is refused', emp('list', {}, { cookies: { staff: st } })), 403, OWNER_ONLY, 'staff');
  is(await step('an employee is refused before the request headers are looked at', emp('list', {}, { cookies: { staff: st }, origin: null })), 403, OWNER_ONLY, 'staff first');
  ok(await step('with both cookies it is the owner', emp('list', {}, { cookies: { owner: ot, staff: st } })), 'both');

  // reset
  is(await step('reset: unknown employee', emp('reset', { id: 'nobody', password: P1 }, own)), 400, 'ไม่พบพนักงาน', 'reset unknown');
  ok(await step('reset somchai', emp('reset', { id: '$e1', password: P1 }, own)), 'reset');
  is(await step('…which ends his session', staff('me', {}, { cookies: { staff: st } })), 401, AUTH_REQUIRED, 'reset ends');
  is(await step('an unknown action on a real employee', emp('promote', { id: '$e1' }, own)), 400, BAD, 'unknown action');
  is(await step('no action at all', { path: '/api/employees', json: {}, ...own }), 400, 'ไม่พบพนักงาน', 'no action');

  // disable, and what custody says about it
  custody('MAIN', { enabled: true, custodian: '$e1', requests: [] });
  is(await step('disable refused: he holds the cash', emp('disable', { id: '$e1' }, own)), 400, 'ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน', 'custodian');
  custody('MAIN', { enabled: true, custodian: '', requests: [{ id: 'r1', status: 'PENDING', sender: '$e1', receiver: '$e2' }] });
  is(await step('disable refused: he sent a request that is pending', emp('disable', { id: '$e1' }, own)), 400, 'ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน', 'sender');
  is(await step('disable refused: she is to receive a request that is pending', emp('disable', { id: '$e2' }, own)), 400, 'ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน', 'receiver');
  ok(await step('custody of MAIN does not hold back an employee of BR-2', emp('disable', { id: '$e3' }, own)), 'other branch');
  custody('MAIN', { enabled: true, custodian: '', requests: [{ id: 'r1', status: 'COMMITTED', sender: '$e1', receiver: '$e2' }, { id: 'r2', status: 'REJECTED', sender: '$e2', receiver: '$e1' }] });
  ok(await step('requests that are no longer pending do not hold back', emp('disable', { id: '$e1' }, own)), 'disable');
  is(await step('disable: unknown employee — custody is not asked, the account action answers', emp('disable', { id: 'nobody' }, own)), 400, 'ไม่พบพนักงาน', 'disable unknown');
  ok(await step('list at the end', emp('list', {}, own)), 'list end');

  // entity and enable belong to custody
  ok(await step('entity is handed to custody with the payload as it came', emp('entity', { branchId: 'MAIN', name: 'บริษัท ตัวอย่าง จำกัด', taxId: '0105500000000' }, own)), 'entity');
  is(await step('enable refused: MAIN lacks an active employee of each of the six roles', emp('enable', { branchId: 'MAIN' }, own)), 400, 'สร้างบัญชีให้ครบ Front, Backend, Manager, Rates, Monitor และ Finance ในสาขาก่อน', 'enable roles');
  for (const role of ['FRONT', 'BACKEND', 'RATES', 'MONITOR', 'FINANCE']) ok(await step('create a ' + role + ' in MAIN', emp('create', person({ username: 'x-' + role.toLowerCase(), name: role, role }), own)), role);
  ok(await step('enable is handed to custody once all six roles are there (the Manager from before, the Front new: the first one is disabled)', emp('enable', { branchId: 'MAIN', note: 'go' }, own)), 'enable');
  is(await step('enable for BR-2, whose only employee is disabled', emp('enable', { branchId: 'BR-2' }, own)), 400, 'สร้างบัญชีให้ครบ Front, Backend, Manager, Rates, Monitor และ Finance ในสาขาก่อน', 'enable other');
});

await story('addresses', 'Which header names the caller for the attempt limit: X-Real-IP at /api/auth; CF-Connecting-IP, then X-Real-IP, at the staff door.', { branches: { MAIN: IDLE } }, async () => {
  for (let i = 1; i <= 30; i++) is(await step('/api/auth from 10.0.0.1, attempt ' + i + ' (CF-Connecting-IP differs every time and is ignored)', auth('login', { username: 'ghost' + i, password: P1 }, { headers: { 'x-real-ip': '10.0.0.1', 'cf-connecting-ip': '172.16.0.' + i } })), 400, WRONG, 'a' + i);
  is(await step('the 31st from 10.0.0.1 is refused', auth('login', { username: 'boss', password: OWNER_PASSWORD }, { headers: { 'x-real-ip': '10.0.0.1', 'cf-connecting-ip': '172.16.0.99' } })), 400, LIMIT, 'a31');
  ok(await step('from 10.0.0.2 the owner signs in', auth('login', { username: 'boss', password: OWNER_PASSWORD }, { headers: { 'x-real-ip': '10.0.0.2' } })), 'other');
  for (let i = 1; i <= 30; i++) is(await step('the staff door, CF-Connecting-IP 172.20.0.1, attempt ' + i + ' (X-Real-IP differs every time and is ignored)', staff('login', { username: 'phantom' + i, password: P1 }, { headers: { 'cf-connecting-ip': '172.20.0.1', 'x-real-ip': '10.1.0.' + i } })), 400, WRONG, 's' + i);
  is(await step('the 31st with that CF-Connecting-IP is refused', staff('login', { username: 'phantom31', password: P1 }, { headers: { 'cf-connecting-ip': '172.20.0.1', 'x-real-ip': '10.1.0.31' } })), 400, LIMIT, 's31');
  is(await step('the staff door without CF-Connecting-IP falls back to X-Real-IP: 172.20.0.1 there is the same caller', staff('login', { username: 'phantom32', password: P1 }, { headers: { 'x-real-ip': '172.20.0.1' } })), 400, LIMIT, 's32');
  is(await step('…and /api/auth counts that address too: one counter per address for both doors', auth('login', { username: 'ghost99', password: P1 }, { headers: { 'x-real-ip': '172.20.0.1' } })), 400, LIMIT, 'shared');
  for (let i = 1; i <= 30; i++) is(await step('no address header at all, attempt ' + i, auth('login', { username: 'spirit' + i, password: P1 })), 400, WRONG, 'u' + i);
  is(await step('…the 31st is refused: such callers share one counter', staff('login', { username: 'spirit31', password: P1 })), 400, LIMIT, 'u31');
});

const all = stories.flatMap((s) => s.steps).filter((s) => s.request);
process.stdout.write(JSON.stringify({
  source: 'xchange-online@3ffc5a5 server.mjs through its own handle(req), driven by api/tools/v2-fixtures/gen_auth_http.mjs',
  note: 'Tokens are written $tN and employee ids $eN in the order v2 made them. `origin` / `marker` null = the header is absent. `cookies` is what the request carried, `setCookies` what the answer set. A long raw body is written {repeat, times}. A step with `custody` is not a request: it changes what custody says about a branch from there on.',
  counts: { stories: stories.length, requests: all.length, byStatus: Object.fromEntries([200, 400, 401, 403, 404].map((c) => [c, all.filter((s) => s.response.status === c).length])) },
  stories,
}, null, 1) + '\n');
