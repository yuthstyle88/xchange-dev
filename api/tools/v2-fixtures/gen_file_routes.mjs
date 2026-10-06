// Drives the HTTP entry of the v2 server for uploading a file and reading one back, and records
// what it answered.
//   node gen_file_routes.mjs > ../../tests/fixtures/file_routes/v2_file_route_cases.json
//
// It needs `file_routes.mjs` beside it: fifteen pieces of the deployed bundle's `server.mjs`
// (`xchange-online@3ffc5a5`), unchanged, each followed by an empty line, in this order:
//   5535-5544  identity, authorizeWrite
//   7072-7080  getFile, recordUpload
//   7094-7105  src/online/app/files/[key]/route.ts              (GET5)
//   7124-7151  src/online/lib/upload.ts                          (uploadEvidence)
//   7153-7161  src/online/app/api/upload/route.ts                (POST2)
//   7178-7181  bridgeAuthorized
//   7284-7369  src/online/lib/employees.ts
//   7396-7465  src/online/app/api/staff-bridge/route.ts          (POST6)
//   7467-7482  shims/staff-proxy.mjs
//   7484-7519  src/booking/app/api/staff/[...path]/route.ts      (POST7, GET6)
//   7760-7766  hookUpload
//   7804-7890  src/auth.mjs
//   8078-8124  src/staff-routes.mjs
//   8126-8233  src/server.mjs: its settings, the route list, withActor … staticFile
//   8260-8292  src/server.mjs: handle
// The bundle is not in this repository, so this cannot run in CI; its output is checked in and
// is what the tests read.
//
// The pieces are loaded as one module, so every request goes through v2's own `handle(req)`:
// its route list, its sign-in gate, then the handler. Supplied from here:
//   env.DB, pool   → one in-memory store that interprets the code's own SQL texts; it throws on
//                    any statement it does not know
//   env.BUCKET     → an in-memory object store (put / get)
//   execute        → the engine. One call reaches it from these routes: `adminDeskConfig`, which
//                    uploadEvidence makes to have the branch judged. It is answered with the two
//                    sentences the engine has for a branch (ticket T6b-1 pinned them from v2):
//                    not a branch name, and no such branch
//   the page handlers the route list names but these requests never reach → undefined
// AUTH_ENABLED=1, DEVICES_ENABLED unset. Sessions are real v2 sign-ins; the fixture names who is
// asking (`as`) instead of carrying tokens. Keys v2 drew are written $kN.
import fs from 'node:fs';
import crypto from 'node:crypto';

const here = new URL('.', import.meta.url);
const pieces = fs.readFileSync(new URL('./file_routes.mjs', here), 'utf8');
for (const needle of ['function authorizeWrite(', 'async function getFile(', 'async function GET5(', 'async function uploadEvidence(', 'async function POST2(', 'async function POST6(', 'async function POST7(', 'async function GET6(', 'async function hookUpload(', 'async function api2(', 'async function upload(', 'async function file(', 'var routes = [', 'async function handle(']) {
  if (!pieces.includes(needle)) throw Error('file_routes.mjs: ' + needle + ' not found');
}

// ── the store ────────────────────────────────────────────────────────────
const t = { employees: [], employee_audit: [], employee_sessions: [], owners: [], owner_sessions: [], booking_limits: [], file_records: [] };
const objects = new Map();
const norm = (s) => s.replace(/\s+/g, ' ').trim();
const limit = (key, expires) => { let r = t.booking_limits.find((x) => x.key === key); if (r) r.count += 1; else t.booking_limits.push(r = { key, count: 1, expires }); return [{ count: r.count }]; };
const bump = (id, f) => { const e = t.employees.find((x) => x.id === id); if (e) { f(e); e.version += 1; } return []; };
const SQL = {
  'INSERT INTO employee_audit(id,actor,subject,action,at) VALUES(?,?,?,?,?)': ([id, actor, subject, action, at]) => (t.employee_audit.push({ id, actor, subject, action, at }), []),
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
  'SELECT * FROM file_records WHERE key=? AND owner_id=?': ([key, owner]) => t.file_records.filter((f) => f.key === key && f.owner_id === owner),
  'INSERT INTO file_records(key,branch_id,owner_id,mime,hash,created) VALUES(?,?,?,?,?,?)': ([key, branch_id, owner_id, mime, hash, created]) => (t.file_records.push({ key, branch_id, owner_id, mime, hash, created }), []),
  'SELECT * FROM file_records WHERE key=? AND branch_id=?': ([key, branch]) => t.file_records.filter((f) => f.key === key && f.branch_id === branch),
  'SELECT * FROM file_records WHERE key=$1 AND branch_id=$2': ([key, branch]) => t.file_records.filter((f) => f.key === key && f.branch_id === branch),
};
const run = (sql, args) => { const f = SQL[norm(sql)]; if (!f) throw Error('store: unknown statement: ' + norm(sql)); return structuredClone(f(args.map((v) => (v === undefined ? null : v)))); };
const stmt = (sql, args = []) => ({ bind: (...a) => stmt(sql, a), first: async () => run(sql, args)[0] ?? null, all: async () => ({ results: run(sql, args) }), run: async () => ({ results: run(sql, args) }) });
const BUCKET = {
  put: async (key, bytes, options = {}) => { objects.set(key, { bytes: Buffer.from(bytes), httpMetadata: options.httpMetadata || {} }); },
  get: async (key) => { const o = objects.get(key); return o ? { key, size: o.bytes.length, httpMetadata: o.httpMetadata, body: o.bytes } : null; },
};

// ── the engine, as far as these routes reach it ──────────────────────────
const BAD_BRANCH = 'สาขาไม่ถูกต้อง', NO_SUCH_BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
const BRANCHES = ['MAIN', 'BR-2'];
async function execute(action, payload) {
  if (action !== 'adminDeskConfig') throw Error('execute: these routes never call ' + action);
  const id = String(payload.branchId);
  if (!/^[A-Za-z0-9-]{1,70}$/.test(id)) throw Error(BAD_BRANCH);
  if (!BRANCHES.includes(id)) throw Error(NO_SUCH_BRANCH);
  return {};
}

globalThis.__v2 = { env: { DB: { prepare: (sql) => stmt(sql) }, BUCKET, BOOKING_BRIDGE_SECRET: 'bridge-secret-of-at-least-32-characters' }, pool: { query: async (sql, args = []) => ({ rows: run(sql, args) }) }, execute };
process.env.AUTH_ENABLED = '1';
for (const k of ['DEVICES_ENABLED', 'OWNER_ID', 'OWNER_EMAIL', 'DESK_HOST', 'ADMIN_ORIGIN', 'CUSTOMER_HOST']) delete process.env[k];
const prelude = `
import fs2 from "node:fs";
import path2 from "node:path";
import { fileURLToPath } from "node:url";
import { timingSafeEqual } from "node:crypto";
const { env, pool, execute } = globalThis.__v2;
var e = env;
var db = () => env.DB;
var DEVICES = false;
var GET, GET2, GET3, GET4, GET7, GET8, POST, POST3, POST4, POST5, hookRpc, documents, receipt, api, api3, beforeSave, afterSave, index_html_default;
`;
const bundle = new URL('./file_routes.bundle.tmp.mjs', here);
fs.writeFileSync(bundle, prelude + pieces + '\nexport { handle, passwordHash, manageEmployees };\n');
let v2;
try { v2 = await import(bundle.href); } finally { fs.rmSync(bundle); }

// ── who is asking ────────────────────────────────────────────────────────
const ORIGIN = 'https://desk.example';
const OWNER_PASSWORD = 'owner-password-01', P1 = 'first-password-1', P2 = 'second-password-2';
const tokens = {}; // name → { owner?: token, staff?: token }
const people = {}; // name → { id, role, branchId, mustChange }
async function raw(path, { method = 'POST', headers = {}, body } = {}) {
  return v2.handle(new Request(ORIGIN + path, { method, headers, body }));
}
const jsonPost = (path, body, cookie) => raw(path, { headers: { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json', ...(cookie ? { cookie } : {}) }, body: JSON.stringify(body) });
const cookieOf = (res, which) => { const line = res.headers.getSetCookie().find((c) => c.startsWith('__Host-xc_' + which + '=') && !c.startsWith('__Host-xc_' + which + '=;')); if (!line) throw Error('no ' + which + ' cookie'); return line.split(';')[0].split('=')[1]; };
async function setUp() {
  t.owners.push({ id: 'owner-row-1', username: 'boss', name: 'boss', password_hash: await v2.passwordHash(OWNER_PASSWORD), active: true, version: 1, created_at: new Date().toISOString() });
  tokens.owner = { owner: cookieOf(await jsonPost('/api/auth', { action: 'login', payload: { username: 'boss', password: OWNER_PASSWORD } }), 'owner') };
  for (const [name, role, branchId, change] of [['front', 'FRONT', 'MAIN', true], ['finance', 'FINANCE', 'MAIN', true], ['manager', 'MANAGER', 'MAIN', true], ['rates', 'RATES', 'MAIN', true], ['system', 'SYSTEM', 'MAIN', true], ['monitor', 'MONITOR', 'MAIN', true], ['front2', 'FRONT', 'BR-2', true], ['newcomer', 'FRONT', 'MAIN', false]]) {
    await v2.manageEmployees('create', { username: name, name, role, branchId, password: P1 }, 'owner');
    const first = cookieOf(await jsonPost('/api/staff/login', { payload: { username: name, password: P1 } }), 'staff');
    if (change) {
      const r = await jsonPost('/api/staff/password', { payload: { password: P2 } }, '__Host-xc_staff=' + first);
      if (!r.ok) throw Error('password change of ' + name);
      tokens[name] = { staff: cookieOf(await jsonPost('/api/staff/login', { payload: { username: name, password: P2 } }), 'staff') };
    } else tokens[name] = { staff: first };
    const e = t.employees.find((x) => x.username === name);
    people[name] = { id: 'employee-' + name, role, branchId, mustChange: !change, realId: e.id };
  }
}

// ── symbols ──────────────────────────────────────────────────────────────
const keyNames = new Map(), keyValues = new Map();
const nameKey = (k) => { if (!keyNames.has(k)) { const n = '$k' + (keyNames.size + 1); keyNames.set(k, n); keyValues.set(n, k); } return keyNames.get(k); };
const realPath = (p) => p.replace(/\$k\d+/g, (n) => keyValues.get(n) ?? n);
const ownerName = (id) => (id === 'owner' ? 'owner' : Object.values(people).find((p) => p.realId === id)?.id ?? id);

// ── one request ──────────────────────────────────────────────────────────
const steps = [];
const hex = (s) => Buffer.from(s, 'latin1').toString('hex');
const JPEG = 'ffd8ffe000104a464946', PNG = '89504e470d0a1a0a0000000d', PDF = hex('%PDF-1.7\n'), WEBP = hex('RIFF') + '24000000' + hex('WEBPVP8 ');
const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');
/** `r`: { as?: [names], method?, path, origin?, marker?, mime?, head?, pad?, v3? } */
async function step(note, r) {
  const method = r.method || 'GET';
  const headers = {};
  const origin = r.origin === undefined ? (method === 'POST' ? ORIGIN : null) : r.origin, marker = r.marker === undefined ? (method === 'POST' ? '1' : null) : r.marker;
  if (origin !== null) headers.origin = origin;
  if (marker !== null) headers['x-xchange-request'] = marker;
  const who = r.as || [];
  const cookie = who.flatMap((n) => [tokens[n].owner ? '__Host-xc_owner=' + tokens[n].owner : '', tokens[n].staff ? '__Host-xc_staff=' + tokens[n].staff : '']).filter(Boolean).join('; ');
  if (cookie) headers.cookie = cookie;
  let body;
  if (method === 'POST') {
    body = Buffer.concat([Buffer.from(r.head || '', 'hex'), Buffer.alloc(r.pad || 0)]);
    if (r.mime != null) headers['content-type'] = r.mime;
    headers['content-length'] = String(body.length);
  }
  const before = t.file_records.length;
  const res = await v2.handle(new Request(ORIGIN + realPath(r.path), { method, headers, body, redirect: 'manual' }));
  const bytes = Buffer.from(await res.arrayBuffer());
  const type = res.headers.get('content-type');
  const response = { status: res.status, headers: {} };
  for (const h of ['content-type', 'cache-control', 'x-content-type-options', 'content-security-policy', 'content-disposition', 'location']) if (res.headers.get(h) !== null) response.headers[h] = res.headers.get(h).replace(/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/g, (k) => nameKey(k));
  if ((type || '').includes('application/json')) {
    const j = JSON.parse(bytes.toString('utf8'));
    if (j.uploadKey) j.uploadKey = nameKey(j.uploadKey);
    response.json = j;
  } else if ((type || '').startsWith('text/plain') || res.status >= 300) response.text = bytes.toString('utf8');
  else response.body = { length: bytes.length, sha256: sha(bytes) };
  const request = { method, path: r.path, as: who, ...(method === 'POST' ? { origin, marker, mime: r.mime ?? null, head: r.head || '', pad: r.pad || 0 } : {}) };
  const s = { note, request, response };
  if (r.v3) s.v3 = r.v3;
  if (t.file_records.length > before) {
    if (t.file_records.length !== before + 1) throw Error('one upload stores one file');
    const f = t.file_records.at(-1), o = objects.get(f.key);
    if (!o || !o.bytes.equals(body) || f.hash !== sha(body)) throw Error('v2 stored something else');
    s.stored = { key: nameKey(f.key), branchId: f.branch_id, ownerId: ownerName(f.owner_id), mime: f.mime, sha256: f.hash };
  }
  steps.push(s);
  return response;
}
/** A file put there directly, as the save of an exchange stores a receipt (not an upload). */
function seed(note, key, branchId, ownerName_, mime, text) {
  const bytes = Buffer.from(text, 'utf8');
  const owner = ownerName_ === 'owner' ? 'owner' : people[ownerName_].realId;
  t.file_records.push({ key, branch_id: branchId, owner_id: owner, mime, hash: sha(bytes), created: new Date().toISOString() });
  objects.set(key, { bytes, httpMetadata: { contentType: mime } });
  steps.push({ note, seed: { key, branchId, ownerId: ownerName_ === 'owner' ? 'owner' : people[ownerName_].id, mime, text } });
}
const is = (res, status, error, note) => { if (res.status !== status || (error !== undefined && (res.json?.error ?? res.text) !== error)) throw Error(note + ': expected ' + status + ' ' + (error ?? '') + ', got ' + JSON.stringify(res)); return res; };

await setUp();
const up = (path, as, over = {}) => ({ method: 'POST', path, as, mime: 'image/jpeg', head: JPEG, pad: 20, ...over });
const SIGN_IN = 'กรุณาเข้าสู่ระบบ', AUTH_REQUIRED = 'EMPLOYEE_AUTH_REQUIRED', BAD = 'คำขอไม่ถูกต้อง', NO_UPLOAD = 'บทบาทนี้อัปโหลดหลักฐานไม่ได้';
const UNSUPPORTED = 'ชนิดไฟล์ไม่รองรับ', TOO_LARGE = 'ไฟล์ใหญ่เกิน 15 MB', NOT_THAT = 'ชนิดไฟล์ไม่ตรงกับเนื้อหา', MUST_CHANGE = 'กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน';
const LIMIT = 15 * 1024 * 1024;
const noSession = { status: 401, json: { error: AUTH_REQUIRED } };

// ── POST /api/upload ─────────────────────────────────────────────────────
is(await step('upload: nobody signed in', up('/api/upload', [])), 401, SIGN_IN, 'nobody');
is(await step('upload: nobody, and the request headers missing — the sign-in check comes first', up('/api/upload', [], { origin: null, marker: null })), 401, SIGN_IN, 'nobody first');
is(await step('upload: a first-password employee is nobody here', up('/api/upload', ['newcomer'])), 401, SIGN_IN, 'newcomer');
is(await step('GET /api/upload is not a route', { path: '/api/upload', as: ['owner'] }), 404, 'Not found', 'get');
is(await step('upload, owner: no Origin header', up('/api/upload', ['owner'], { origin: null })), 400, 'ORIGIN_REJECTED', 'o origin');
is(await step('upload, owner: another origin', up('/api/upload', ['owner'], { origin: 'https://evil.example' })), 400, 'ORIGIN_REJECTED', 'o other');
is(await step('upload, owner: no marker header', up('/api/upload', ['owner'], { marker: null })), 400, 'ORIGIN_REJECTED', 'o marker');
is(await step('upload, owner: no branch named → MAIN', up('/api/upload', ['owner'])), 200, undefined, 'o main');
is(await step('upload, owner: ?branch= (empty) → MAIN', up('/api/upload?branch=', ['owner'], { mime: 'image/png', head: PNG })), 200, undefined, 'o empty');
is(await step('upload, owner: ?branch=BR-2', up('/api/upload?branch=BR-2', ['owner'], { mime: 'application/pdf', head: PDF })), 200, undefined, 'o br2');
is(await step('upload, owner: a WebP', up('/api/upload?branch=MAIN', ['owner'], { mime: 'image/webp', head: WEBP })), 200, undefined, 'o webp');
is(await step('upload, owner: a branch that does not exist', up('/api/upload?branch=NOWHERE', ['owner'])), 400, NO_SUCH_BRANCH, 'o nowhere');
is(await step('upload, owner: not a branch name', up('/api/upload?branch=MAIN_1', ['owner'])), 400, BAD_BRANCH, 'o bad');
is(await step('upload, owner: a bad branch and a bad type — the branch sentence wins', up('/api/upload?branch=NOWHERE', ['owner'], { mime: 'text/plain' })), 400, NO_SUCH_BRANCH, 'o both');
is(await step('upload, owner: a type that is not accepted', up('/api/upload', ['owner'], { mime: 'image/gif', head: hex('GIF89a') })), 400, UNSUPPORTED, 'o gif');
is(await step('upload, owner: a type with a parameter', up('/api/upload', ['owner'], { mime: 'image/jpeg; charset=binary' })), 400, UNSUPPORTED, 'o param');
is(await step('upload, owner: no Content-Type', up('/api/upload', ['owner'], { mime: null })), 400, UNSUPPORTED, 'o none');
is(await step('upload, owner: content that is not what the type says', up('/api/upload', ['owner'], { head: PNG })), 400, NOT_THAT, 'o not');
is(await step('upload, owner: an empty body', up('/api/upload', ['owner'], { head: '', pad: 0 })), 400, NOT_THAT, 'o empty body');
is(await step('upload, owner: exactly 15 MB', up('/api/upload', ['owner'], { pad: LIMIT - 10 })), 200, undefined, 'o 15');
is(await step('upload, owner: one byte more', up('/api/upload', ['owner'], { pad: LIMIT - 9 })), 400, TOO_LARGE, 'o 15+1');
is(await step('upload, owner and an employee signed in together: it is the owner', up('/api/upload?branch=BR-2', ['owner', 'manager'])), 200, undefined, 'both');

is(await step('upload, Front: stored in the employee\'s own branch', up('/api/upload', ['front'])), 200, undefined, 'f');
is(await step('upload, Front: ?branch=BR-2 is ignored — still the own branch', up('/api/upload?branch=BR-2', ['front'], { mime: 'image/png', head: PNG })), 200, undefined, 'f br2');
is(await step('upload, Front: ?branch=NOWHERE is ignored too', up('/api/upload?branch=NOWHERE', ['front'])), 200, undefined, 'f nowhere');
is(await step('upload, Finance', up('/api/upload', ['finance'], { mime: 'application/pdf', head: PDF })), 200, undefined, 'fin');
is(await step('upload, Front of BR-2', up('/api/upload', ['front2'])), 200, undefined, 'f2');
is(await step('upload, employee: no Origin header — the staff sentence, not the owner\'s', up('/api/upload', ['front'], { origin: null })), 400, BAD, 'f origin');
is(await step('upload, employee: no marker header', up('/api/upload', ['front'], { marker: null })), 400, BAD, 'f marker');
for (const role of ['manager', 'rates', 'system', 'monitor']) is(await step('upload, ' + role + ': this role may not upload', up('/api/upload', [role])), 400, NO_UPLOAD, role);
is(await step('upload, Manager with the headers missing: the headers are judged first', up('/api/upload', ['manager'], { origin: null })), 400, BAD, 'm origin');
is(await step('upload, Manager with a bad type: the role is judged first', up('/api/upload', ['manager'], { mime: 'text/plain' })), 400, NO_UPLOAD, 'm type');
is(await step('upload, Front: a type that is not accepted', up('/api/upload', ['front'], { mime: 'text/html', head: hex('<html>') })), 400, UNSUPPORTED, 'f type');
is(await step('upload, Front: content that is not what the type says', up('/api/upload', ['front'], { mime: 'image/webp', head: JPEG })), 400, NOT_THAT, 'f not');
is(await step('upload, Front: more than 15 MB', up('/api/upload', ['front'], { pad: LIMIT })), 400, TOO_LARGE, 'f big');

// ── POST /api/staff/upload ───────────────────────────────────────────────
is(await step('staff upload: no session', up('/api/staff/upload', [])), 401, AUTH_REQUIRED, 's none');
is(await step('staff upload: the owner is not an employee', up('/api/staff/upload', ['owner'])), 401, AUTH_REQUIRED, 's owner');
is(await step('staff upload: no Origin header', up('/api/staff/upload', ['front'], { origin: null })), 400, BAD, 's origin');
is(await step('staff upload: no Origin header and no session — the headers are judged first', up('/api/staff/upload', [], { origin: null })), 400, BAD, 's origin first');
is(await step('staff upload: no marker header', up('/api/staff/upload', ['front'], { marker: null })), 400, BAD, 's marker');
is(await step('GET /api/staff/upload is not a route', { path: '/api/staff/upload', as: ['front'] }), 404, 'Not found', 's get');
is(await step('staff upload: a first-password employee (v2 names the password; v3 has no session for such an employee)', up('/api/staff/upload', ['newcomer'], { v3: noSession })), 400, MUST_CHANGE, 's newcomer');
is(await step('staff upload, Front', up('/api/staff/upload', ['front'])), 200, undefined, 's f');
is(await step('staff upload, Front with the owner signed in too: the employee', up('/api/staff/upload', ['owner', 'front'], { mime: 'image/webp', head: WEBP })), 200, undefined, 's both');
is(await step('staff upload, Finance', up('/api/staff/upload', ['finance'], { mime: 'image/png', head: PNG })), 200, undefined, 's fin');
is(await step('staff upload, Front of BR-2, ?branch=MAIN ignored', up('/api/staff/upload?branch=MAIN', ['front2'], { mime: 'application/pdf', head: PDF })), 200, undefined, 's f2');
for (const role of ['manager', 'rates', 'system', 'monitor']) is(await step('staff upload, ' + role + ': this role may not upload', up('/api/staff/upload', [role])), 400, NO_UPLOAD, 's ' + role);
is(await step('staff upload, Manager with a bad type: the role is judged first', up('/api/staff/upload', ['manager'], { mime: 'text/plain' })), 400, NO_UPLOAD, 's m type');
is(await step('staff upload: a type that is not accepted', up('/api/staff/upload', ['front'], { mime: 'image/gif', head: hex('GIF89a') })), 400, UNSUPPORTED, 's type');
is(await step('staff upload: no Content-Type', up('/api/staff/upload', ['front'], { mime: null })), 400, UNSUPPORTED, 's none type');
is(await step('staff upload: content that is not what the type says', up('/api/staff/upload', ['front'], { head: PDF })), 400, NOT_THAT, 's not');
is(await step('staff upload: an empty body', up('/api/staff/upload', ['front'], { head: '', pad: 0 })), 400, NOT_THAT, 's empty');
is(await step('staff upload: exactly 15 MB', up('/api/staff/upload', ['front'], { pad: LIMIT - 10 })), 200, undefined, 's 15');
is(await step('staff upload: one byte more', up('/api/staff/upload', ['front'], { pad: LIMIT - 9 })), 400, TOO_LARGE, 's 15+1');

// ── what is there to read ────────────────────────────────────────────────
const stored = (who, branch, mime) => steps.find((s) => s.stored && s.stored.ownerId === who && s.stored.branchId === branch && s.stored.mime === mime && s.request.pad < 1000)?.stored.key;
const K = {
  ownerMainJpeg: stored('owner', 'MAIN', 'image/jpeg'), ownerMainPng: stored('owner', 'MAIN', 'image/png'), ownerMainWebp: stored('owner', 'MAIN', 'image/webp'), ownerBr2Pdf: stored('owner', 'BR-2', 'application/pdf'),
  frontJpeg: stored('employee-front', 'MAIN', 'image/jpeg'), financePdf: stored('employee-finance', 'MAIN', 'application/pdf'), front2Jpeg: stored('employee-front2', 'BR-2', 'image/jpeg'),
};
for (const [n, k] of Object.entries(K)) if (!k) throw Error('no stored file for ' + n);
const RECEIPT = 'receipt-' + 'ab'.repeat(32), RECEIPT_STAFF = 'receipt-' + 'cd'.repeat(32), LONG = '0123456789abcdef-'.repeat(3).slice(0, 40), SHORT_RECEIPT = 'receipt-' + 'e'.repeat(36);
seed('a receipt page stored by the owner\'s save of an exchange in MAIN', RECEIPT, 'MAIN', 'owner', 'text/html', '<!doctype html><title>receipt</title><p>ใบเสร็จ</p>');
seed('a receipt page stored by Front\'s save in MAIN', RECEIPT_STAFF, 'MAIN', 'front', 'text/html', '<!doctype html><title>receipt</title><p>ใบเสร็จของพนักงาน</p>');
seed('a file under a 40-character key in MAIN (only the staff ways in take such a key)', LONG, 'MAIN', 'owner', 'image/png', '\x89PNG\r\n\x1a\nlong');
seed('a file under receipt- and 36 characters in MAIN', SHORT_RECEIPT, 'MAIN', 'owner', 'image/jpeg', 'short receipt');
seed('a file whose recorded type is none v2 knows', 'f'.repeat(36), 'MAIN', 'owner', 'application/zip', 'zip');

// ── GET /files/{key} ─────────────────────────────────────────────────────
const get = (path, as, over = {}) => ({ path, as, ...over });
is(await step('file: nobody signed in is sent to the sign-in page', get('/files/' + K.ownerMainJpeg, [])), 303, undefined, 'nobody');
is(await step('file: a first-password employee is sent there too', get('/files/' + K.frontJpeg, ['newcomer'])), 303, undefined, 'newcomer');
is(await step('POST /files/{key} is not a route', { method: 'POST', path: '/files/' + K.ownerMainJpeg, as: ['owner'] }), 404, 'Not found', 'post');
is(await step('file, owner: a JPEG the owner uploaded', get('/files/' + K.ownerMainJpeg, ['owner'])), 200, undefined, 'o jpeg');
is(await step('file, owner: the same with ?download=1', get('/files/' + K.ownerMainJpeg + '?download=1', ['owner'])), 200, undefined, 'o dl');
is(await step('file, owner: ?download=true is not a download', get('/files/' + K.ownerMainJpeg + '?download=true', ['owner'])), 200, undefined, 'o dl true');
is(await step('file, owner: a PNG, download', get('/files/' + K.ownerMainPng + '?download=1', ['owner'])), 200, undefined, 'o png');
is(await step('file, owner: a WebP, download', get('/files/' + K.ownerMainWebp + '?download=1', ['owner'])), 200, undefined, 'o webp');
is(await step('file, owner: a PDF of another branch, download — the branch plays no part', get('/files/' + K.ownerBr2Pdf + '?download=1', ['owner'])), 200, undefined, 'o pdf');
is(await step('file, owner: a receipt page', get('/files/' + RECEIPT, ['owner'])), 200, undefined, 'o receipt');
is(await step('file, owner: a receipt page, download', get('/files/' + RECEIPT + '?download=1', ['owner'])), 200, undefined, 'o receipt dl');
is(await step('file, owner: a type v2 has no ending for, download', get('/files/' + 'f'.repeat(36) + '?download=1', ['owner'])), 200, undefined, 'o bin');
is(await step('file, owner: a file an employee uploaded is not the owner\'s', get('/files/' + K.frontJpeg, ['owner'])), 404, 'Not found', 'o staff file');
is(await step('file, owner: a receipt an employee\'s save stored is not the owner\'s', get('/files/' + RECEIPT_STAFF, ['owner'])), 404, 'Not found', 'o staff receipt');
is(await step('file, owner: a 40-character key is not looked up this way', get('/files/' + LONG, ['owner'])), 404, 'Not found', 'o long');
is(await step('file, owner: receipt- and 36 characters is not looked up this way', get('/files/' + SHORT_RECEIPT, ['owner'])), 404, 'Not found', 'o short receipt');
is(await step('file, owner: a key nobody stored', get('/files/' + '0'.repeat(8) + '-0000-4000-8000-' + '0'.repeat(12), ['owner'])), 404, 'Not found', 'o unknown');
is(await step('file, owner: a key in capitals', get('/files/' + 'A'.repeat(36), ['owner'])), 404, 'Not found', 'o caps');
is(await step('file, owner: a key of 35 characters', get('/files/' + 'a'.repeat(35), ['owner'])), 404, 'Not found', 'o 35');
is(await step('file, owner and employee signed in together: the owner\'s way', get('/files/' + K.frontJpeg, ['owner', 'front'])), 404, 'Not found', 'both');

is(await step('file, Front: own upload — no Content-Disposition this way', get('/files/' + K.frontJpeg, ['front'])), 200, undefined, 'f own');
is(await step('file, Front: ?download=1 changes nothing this way', get('/files/' + K.frontJpeg + '?download=1', ['front'])), 200, undefined, 'f dl');
is(await step('file, Front: the owner\'s upload in the same branch', get('/files/' + K.ownerMainJpeg, ['front'])), 200, undefined, 'f owner file');
is(await step('file, Manager: a colleague\'s upload in the same branch', get('/files/' + K.financePdf, ['manager'])), 200, undefined, 'm');
is(await step('file, Monitor: a receipt page', get('/files/' + RECEIPT, ['monitor'])), 200, undefined, 'mon');
is(await step('file, Front: the 40-character key', get('/files/' + LONG, ['front'])), 200, undefined, 'f long');
is(await step('file, Front: receipt- and 36 characters', get('/files/' + SHORT_RECEIPT, ['front'])), 200, undefined, 'f short');
is(await step('file, Front: a file of another branch', get('/files/' + K.ownerBr2Pdf, ['front'])), 404, 'Not found', 'f other');
is(await step('file, Front of BR-2: a file of MAIN', get('/files/' + K.frontJpeg, ['front2'])), 404, 'Not found', 'f2');
is(await step('file, Front of BR-2: own branch', get('/files/' + K.front2Jpeg, ['front2'])), 200, undefined, 'f2 own');
for (const role of ['rates', 'system']) is(await step('file, ' + role + ': this role reads no file', get('/files/' + K.frontJpeg, [role])), 404, 'Not found', role);
is(await step('file, Front: a key in capitals', get('/files/' + 'A'.repeat(36), ['front'])), 404, 'Not found', 'f caps');
is(await step('file, Front: a key of 65 characters', get('/files/' + 'a'.repeat(65), ['front'])), 404, 'Not found', 'f 65');
is(await step('file, Front: a key nobody stored', get('/files/' + 'b'.repeat(36), ['front'])), 404, 'Not found', 'f unknown');

// ── GET /api/staff/file/{key} ────────────────────────────────────────────
const NO_READ = 'บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้าหรือหลักฐานเงิน', NO_FILE = 'ไม่พบไฟล์';
is(await step('staff file: no session', get('/api/staff/file/' + K.frontJpeg, [])), 401, AUTH_REQUIRED, 'sf none');
is(await step('staff file: the owner is not an employee', get('/api/staff/file/' + K.ownerMainJpeg, ['owner'])), 401, AUTH_REQUIRED, 'sf owner');
is(await step('staff file: a first-password employee (v2 names the password; v3 has no session for such an employee)', get('/api/staff/file/' + K.frontJpeg, ['newcomer'], { v3: noSession })), 400, MUST_CHANGE, 'sf newcomer');
is(await step('POST /api/staff/file/{key} is not a route', { method: 'POST', path: '/api/staff/file/' + K.frontJpeg, as: ['front'] }), 404, 'Not found', 'sf post');
is(await step('staff file: one path part too many', get('/api/staff/file/' + K.frontJpeg + '/x', ['front'])), 404, 'Not found', 'sf extra');
is(await step('staff file, Front: own upload', get('/api/staff/file/' + K.frontJpeg, ['front'])), 200, undefined, 'sf own');
is(await step('staff file, Front: needs neither Origin nor the marker header, and ?download=1 changes nothing', get('/api/staff/file/' + K.frontJpeg + '?download=1', ['front'])), 200, undefined, 'sf dl');
is(await step('staff file, Front with the owner signed in too', get('/api/staff/file/' + K.ownerMainPng, ['owner', 'front'])), 200, undefined, 'sf both');
is(await step('staff file, Finance: a PDF', get('/api/staff/file/' + K.financePdf, ['finance'])), 200, undefined, 'sf pdf');
is(await step('staff file, Manager: a WebP', get('/api/staff/file/' + K.ownerMainWebp, ['manager'])), 200, undefined, 'sf webp');
is(await step('staff file, Monitor: a receipt page is always a download named receipt.html', get('/api/staff/file/' + RECEIPT, ['monitor'])), 200, undefined, 'sf receipt');
is(await step('staff file, Front: the 40-character key', get('/api/staff/file/' + LONG, ['front'])), 200, undefined, 'sf long');
is(await step('staff file, Front: receipt- and 36 characters', get('/api/staff/file/' + SHORT_RECEIPT, ['front'])), 200, undefined, 'sf short');
is(await step('staff file, Front: a type v2 has no ending for', get('/api/staff/file/' + 'f'.repeat(36), ['front'])), 200, undefined, 'sf zip');
for (const role of ['rates', 'system']) is(await step('staff file, ' + role + ': this role reads no file', get('/api/staff/file/' + K.frontJpeg, [role])), 400, NO_READ, 'sf ' + role);
is(await step('staff file, Rates and a key that cannot be one: the role is judged first', get('/api/staff/file/NOPE', ['rates'])), 400, NO_READ, 'sf rates key');
is(await step('staff file, Front: a file of another branch', get('/api/staff/file/' + K.ownerBr2Pdf, ['front'])), 400, NO_FILE, 'sf other');
is(await step('staff file, Front of BR-2: a file of MAIN', get('/api/staff/file/' + K.frontJpeg, ['front2'])), 400, NO_FILE, 'sf f2');
is(await step('staff file, Front: a key nobody stored', get('/api/staff/file/' + 'b'.repeat(36), ['front'])), 400, NO_FILE, 'sf unknown');
is(await step('staff file, Front: a key in capitals', get('/api/staff/file/' + 'A'.repeat(36), ['front'])), 400, NO_FILE, 'sf caps');
is(await step('staff file, Front: a key of 35 characters', get('/api/staff/file/' + 'a'.repeat(35), ['front'])), 400, NO_FILE, 'sf 35');
is(await step('staff file, Front: a key of 65 characters', get('/api/staff/file/' + 'a'.repeat(65), ['front'])), 400, NO_FILE, 'sf 65');

const requests = steps.filter((s) => s.request);
process.stdout.write(JSON.stringify({
  source: 'xchange-online@3ffc5a5 server.mjs through its own handle(req), driven by api/tools/v2-fixtures/gen_file_routes.mjs',
  note: 'Keys v2 drew are written $kN in the order it drew them. `as` names who is signed in (see `people`; "owner" is the owner). A POST body is `head` (hex) followed by `pad` zero bytes; `origin` / `marker` null = the header is absent; `mime` null = no Content-Type. A step with `seed` is not a request: a file put in the store directly. `stored` is the one row and object an accepted upload left. `v3` is the answer v3 gives where it differs from v2 on purpose.',
  branches: BRANCHES,
  people: Object.fromEntries(Object.entries(people).map(([n, p]) => [n, { id: p.id, role: p.role, branchId: p.branchId, mustChange: p.mustChange }])),
  counts: { requests: requests.length, seeds: steps.length - requests.length, stored: steps.filter((s) => s.stored).length, byStatus: Object.fromEntries([200, 303, 400, 401, 404].map((c) => [c, requests.filter((s) => s.response.status === c).length])) },
  steps,
}, null, 1) + '\n');
