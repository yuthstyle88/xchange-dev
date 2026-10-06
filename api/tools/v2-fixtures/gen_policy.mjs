// Asks v2 who may do what, and records the answer.
//   node gen_policy.mjs [path/to/server.mjs] > ../../crates/xc-auth/tests/fixtures/v2_policy_cases.json
//
// Unlike its neighbours this script needs the whole deployed bundle (`server.mjs`,
// `xchange-online@3ffc5a5`, 8301 lines), not `engine.mjs`: v2 decides authorisation in the
// route table and in the handlers around the engine, which `engine.mjs` does not hold.
// The bundle is not in this repository; the default path is `./server.mjs`.
//
// How v2 is run. The text from `// src/online/app/documents/route.ts` to `http.createServer(`
// (server.mjs:7082-8292: the handlers, the staff bridge, devices, kiosk, the staff routes, the route table and
// `handle`) is evaluated unchanged, together with `identity`/`authorizeWrite` and the custody
// module (`runOperations`). Every request below goes through v2's own `handle(request)`; the
// custody rows call v2's own `runOperations`. What is replaced, and nothing else:
//   - `import … from "node:crypto"` lines are dropped and the names are passed in;
//   - `import.meta.url` (one use, the static-file directory) is a fixed string, and v2's
//     `console` is silent;
//   - the things below the gates are stand-ins: `execute` (the engine entry), the two
//     database handles and the file bucket. A stand-in records that it was reached, and with
//     which branch, then stops the request. Two queries are answered instead of stopping,
//     because they are how v2 learns who is calling: the owner-session and the
//     employee-session look-ups.
// So "allowed" means: v2 let the call through every check that looks at who is calling, and
// went on to the engine or the database. Whether the action then succeeds is not recorded.
//
// The script fails, and writes nothing, when v2 answers with a sentence it was not expected
// to say, when a stand-in is missing, or when a role, an action or a path is always allowed
// or always refused where that would be a sign the probe is not reaching the check.
import fs from 'node:fs';
import crypto from 'node:crypto';

const file = process.argv[2] || new URL('./server.mjs', import.meta.url);
const src = fs.readFileSync(file, 'utf8');
const slice = (from, to) => { const a = src.indexOf(from), b = src.indexOf(to, a); if (a < 0 || b < 0) throw Error(from + ' not found in server.mjs'); return src.slice(a, b); };

// ── the world below the gates ───────────────────────────────────────────────
const REACHED = '__REACHED__';
const world = { actor: null, reached: [] };
const reach = (what, branch) => { world.reached.push({ what, branch: branch ?? null }); throw Error(REACHED); };
const OWNER_ROW = { id: 'owner', username: 'owner', name: 'Owner', version: 1, active: true };
const employeeRow = (a) => ({ id: 'E-' + a.role, username: a.role.toLowerCase(), name: a.role, role: a.role, branch_id: a.own, active: 1, must_change: 0, version: 1 });
const branchIn = (args) => (args || []).find((x) => x === OWN || x === OTHER || x === 'MAIN');
const statement = (sql, args = []) => ({
  bind: (...more) => statement(sql, more),
  first: async () => /JOIN employee_sessions/.test(sql) ? (world.actor?.kind === 'staff' ? employeeRow(world.actor) : null) : reach('db', branchIn(args)),
  all: async () => reach('db', branchIn(args)),
  run: async () => reach('db', branchIn(args)),
});
const scope = {
  process: { env: { AUTH_ENABLED: '1', DEVICES_ENABLED: '1', OWNER_ID: 'owner', CUSTOMER_HOST: 'customer.test' } },
  env: { DB: { prepare: (sql) => statement(sql) }, BUCKET: { get: async () => reach('bucket'), head: async () => reach('bucket') }, BOOKING_BRIDGE_SECRET: 'b'.repeat(32) },
  pool: { query: async (sql, args) => /JOIN owner_sessions/.test(sql) ? { rows: world.actor?.kind === 'owner' ? [OWNER_ROW] : [] } : reach('db', branchIn(args)), connect: async () => reach('db') },
  // server.mjs:6998 — the engine reads `payload.branchId || "MAIN"`; the stand-in records the same.
  execute: async (action, payload) => reach('engine:' + action, payload?.branchId || 'MAIN'),
  index_html_default: '<html><head></head><body></body></html>',
  path2: { join: (...p) => p.join('/'), dirname: (p) => p, resolve: (...p) => p.join('/'), sep: '/', extname: () => '' },
  fileURLToPath: (u) => u, fs2: { createReadStream: () => reach('page'), existsSync: () => false }, Readable2: null,
  // pages and files the owner is served from below the gate
  GET: () => reach('page'), GET2: () => reach('page'), GET3: () => reach('page'), getFile: async () => reach('file'),
  htmlResponse: () => reach('page'), fetchBOT: async () => reach('reference-rates'),
  console: { error() {}, log() {} },
};
for (const n of ['createHash', 'createHash3', 'createHash4', 'createHash5', 'createHash6', 'createHash7']) scope[n] = crypto.createHash;
for (const n of ['randomUUID2', 'randomUUID5', 'randomUUID6', 'randomUUID7', 'randomUUID8', 'randomUUID9']) scope[n] = () => '00000000-0000-4000-8000-000000000000';
for (const n of ['timingSafeEqual', 'timingSafeEqual2', 'timingSafeEqual3']) scope[n] = crypto.timingSafeEqual;
for (const n of ['randomBytes', 'randomBytes2']) scope[n] = crypto.randomBytes;
for (const n of ['scrypt', 'scrypt2']) scope[n] = crypto.scrypt;

const body = [
  slice('function identity(request) {', 'function htmlResponse('),
  slice('var zero = () =>', '// src/online/lib/engine.mjs'),
  slice('// src/online/app/documents/route.ts', 'http.createServer('),
].join('\n').replace(/^import \{[^}]*\} from "node:crypto";$/gm, '').replace('import.meta.url', '"file:///v2/server.mjs"');
if (/^import /m.test(body) || body.includes('import.meta')) throw Error('an import survived; the stand-in list is out of date');
// A name the bundle declares must not also be handed in, or the hand-in would win silently.
for (const n of Object.keys(scope)) if (new RegExp('^(?:var|async function|function) ' + n + '\\b', 'm').test(body)) throw Error(n + ' is declared by v2 and must not be replaced');
const v2 = new Function(...Object.keys(scope), body + '\nreturn { handle, staffProxy, runOperations, initialOperations, roles, READS, WRITES, PERMITS, STAFF, allowed, routes };')(...Object.values(scope));

// ── who calls ───────────────────────────────────────────────────────────────
const OWN = 'B2', OTHER = 'B9';
const ROLES = v2.roles.slice(); // server.mjs:7290
const ACTORS = [{ kind: 'anon', role: 'ANON', own: null }, { kind: 'owner', role: 'OWNER', own: null }, ...ROLES.map((role) => ({ kind: 'staff', role, own: OWN }))];
const TOKEN = 'a'.repeat(64);
const cookie = (a) => a.kind === 'owner' ? '__Host-xc_owner=' + TOKEN : a.kind === 'staff' ? '__Host-xc_staff=' + TOKEN : '';
const ORIGIN = 'http://desk.test';

// ── sentences v2 may say at a gate; anything else stops the script ──────────
const SENTENCES = new Map(Object.entries({
  'กรุณาเข้าสู่ระบบ': 401, 'เฉพาะเจ้าของระบบเท่านั้น': 403, 'EMPLOYEE_AUTH_REQUIRED': 401, 'คำขอไม่ถูกต้อง': 400,
  'บทบาทนี้ไม่มีสิทธิ์ทำรายการ': 400, 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้': 400, 'เจ้าของเป็นผู้เปิดใช้ขั้นตอน': 400, 'ใช้ได้เฉพาะ Front': 400,
  'บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า': 400, 'บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้าหรือหลักฐานเงิน': 400, 'บทบาทนี้อัปโหลดหลักฐานไม่ได้': 400,
  'Not found': 404, 'บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้': 400, 'คำขอไม่รองรับ': 400, 'Owner or Rates access required.': 400,
  'Staff access required.': 400, 'Unknown kiosk action.': 400,
  'ไม่มีสิทธิ์ลงชื่อ': 400, 'Backend ใช้ปุ่มตรวจและบันทึกงาน': 400, 'ต้องใช้บัญชีพนักงาน': 400, 'ลงเวลาเข้าก่อนทำรายการ': 400,
}));

/** One HTTP request through v2's `handle`. */
async function ask(actor, method, url, payloadBody) {
  world.actor = actor; world.reached = [];
  const headers = { origin: ORIGIN, 'x-xchange-request': '1' };
  if (cookie(actor)) headers.cookie = cookie(actor);
  if (payloadBody !== undefined) headers['content-type'] = typeof payloadBody === 'string' ? 'application/json' : 'image/png';
  const init = { method, headers };
  if (method === 'POST') init.body = typeof payloadBody === 'string' ? payloadBody : payloadBody ?? '{}';
  let res;
  try { res = await v2.handle(new Request(ORIGIN + url, init)); } catch (e) {
    if (e.message !== REACHED) throw e; // v2 let it through and the stand-in stopped it outside any try
    return { allowed: true, branch: world.reached.at(-1).branch };
  }
  if (world.reached.length) return { allowed: true, branch: world.reached.at(-1).branch };
  if (res.status >= 300 && res.status < 400) return { allowed: false, error: null, status: res.status, redirect: new URL(res.headers.get('location')).pathname };
  const text = await res.text();
  let error = text;
  try { const j = JSON.parse(text); if (j && typeof j.error === 'string') error = j.error; } catch { /* plain text */ }
  // the one answer that is complete above the stand-ins: the desk page a Front is served (server.mjs:7416)
  const page = res.ok && /window\.XC_BRANCH="([^"]+)"/.exec(text);
  if (page) return { allowed: true, branch: page[1] };
  if (res.ok) throw Error(`${method} ${url} as ${actor.role}: v2 answered ${res.status} without reaching a stand-in: ${text.slice(0, 120)}`);
  if (!SENTENCES.has(error)) throw Error(`${method} ${url} as ${actor.role}: unexpected sentence ${JSON.stringify(error)} (${res.status})`);
  return { allowed: false, error, status: res.status };
}

// ── action names, read from v2's own text ───────────────────────────────────
const names = (text, re) => [...new Set([...text.matchAll(re)].map((m) => m[1]))];
const custodyText = slice('function runOperations(', '// src/online/lib/engine.mjs');
const OPS = names(custodyText, /action3 === "(ops[A-Za-z]+)"/g);                       // 15 names, server.mjs:5998-6235
const RPC = [...v2.allowed];                                                           // server.mjs:7108
const bridgeText = slice('const reads2 = [', 'const result = await execute(action3');
const BRIDGE = names(bridgeText, /"((?:admin|get|reserve|save|documents)[A-Za-z]*)"/g); // server.mjs:7433-7438
const STAFF_ROUTES = [...v2.READS, ...Object.values(v2.WRITES).flat()];                // server.mjs:8079-8086
// names v2 does not know, and near misses of ones it does: the staff bridge lets through
// whatever starts with "ops" (server.mjs:7435), and every comparison is exact
const UNKNOWN = ['nope', 'opsNope', '', 'ops', 'Ops', 'opsenable', 'adminsnapshot', 'adminSnapshot '];
const RPC_ACTIONS = [...new Set([...RPC, ...BRIDGE, ...STAFF_ROUTES, ...OPS, ...UNKNOWN])];
const deviceText = slice('async function action(name, p, actor) {', 'async function api(req, actor) {');
const DEVICE_ACTIONS = [...new Set([...names(deviceText, /name === "([A-Za-z]+)"/g), ...Object.keys(v2.PERMITS), 'nope'])]; // server.mjs:7563-7663
const kioskText = slice('async function action2(name, p, user) {', 'async function api3(req, user) {');
const KIOSK_ACTIONS = [...names(kioskText, /name === "([A-Za-z]+)"/g), 'nope'];        // server.mjs:7935-8046
const EMPLOYEE_ACTIONS = ['list', 'create', 'reset', 'disable', 'entity', 'enable', 'nope']; // server.mjs:7376-7390, 7309-7334
if (OPS.length !== 15 || RPC.length !== 11 || DEVICE_ACTIONS.length !== 7 || KIOSK_ACTIONS.length !== 14) throw Error(`action lists changed: ops ${OPS.length}, rpc ${RPC.length}, devices ${DEVICE_ACTIONS.length}, kiosk ${KIOSK_ACTIONS.length}`);

const KEY = '0'.repeat(8) + '-0000-4000-8000-' + '0'.repeat(12);
const post = (action, named, extra = {}) => JSON.stringify({ action, payload: { ...(named ? { branchId: named } : {}), ...extra } });
const q = (named) => named ? '?branch=' + named : '';
// payloads carry only what keeps a check that is not about the caller out of the way:
// a cash save must say the cash was counted (server.mjs:7740), a passport check must be confirmed (8014).
const RPC_EXTRA = { countConfirmed: true, confirmed: true, id: 'r1' };

/** path → the v2 routes that share its gate, and how to send one action on each. */
const PATHS = [
  { path: 'Rpc', line: 'server.mjs:8149, owner 7108-7115, staff 8079-8098', routes: [{ route: 'POST /api/rpc', actions: RPC_ACTIONS, send: (a, action, named) => ask(a, 'POST', '/api/rpc', post(action, named, RPC_EXTRA)) }] },
  { path: 'StaffRpc', line: 'server.mjs:8165, 7397-7443', routes: [{ route: 'POST /api/staff/rpc', actions: RPC_ACTIONS, send: (a, action, named) => ask(a, 'POST', '/api/staff/rpc', post(action, named, RPC_EXTRA)) }] },
  { path: 'Upload', line: 'server.mjs:8153, 8104-8108, owner 7154-7157', routes: [{ route: 'POST /api/upload', actions: ['upload'], send: (a, _action, named) => ask(a, 'POST', '/api/upload' + q(named), new Uint8Array([1])) }] },
  { path: 'StaffUpload', line: 'server.mjs:8169, 7401-7404', routes: [{ route: 'POST /api/staff/upload', actions: ['upload'], send: (a, _action, named) => ask(a, 'POST', '/api/staff/upload' + q(named), new Uint8Array([1])) }] },
  { path: 'File', line: 'server.mjs:8148, 8113-8115, owner 7095-7099', routes: [{ route: 'GET /files/{key}', actions: ['file'], send: (a, _action, named) => ask(a, 'GET', '/files/' + KEY + q(named)) }] },
  { path: 'StaffFile', line: 'server.mjs:8174, 7510-7515, 7421-7425', routes: [{ route: 'GET /api/staff/file/{key}', actions: ['file'], send: (a, _action, named) => ask(a, 'GET', '/api/staff/file/' + KEY + q(named)) }] },
  { path: 'StaffDesk', line: 'server.mjs:8163, 7522-7524, 7414-7415', routes: [{ route: 'GET /staff/desk', actions: ['desk'], send: async (a, _action, named) => {
    const out = await ask(a, 'GET', '/staff/desk' + q(named));
    if (out.allowed) return out;
    // The route turns every refusal into a redirect to /staff (7524). What the bridge said
    // behind it is asked the way the route asks it (7523), and kept beside the redirect.
    const res = await v2.staffProxy(new Request(ORIGIN + '/staff/desk', { headers: cookie(a) ? { cookie: cookie(a) } : {} }), 'desk');
    const behind = { error: (await res.json()).error, status: res.status };
    if (!SENTENCES.has(behind.error)) throw Error('desk: unexpected sentence ' + behind.error);
    return { ...out, behind };
  } }] },
  { path: 'Devices', line: 'server.mjs:8178, 7608-7612', routes: [{ route: 'POST /api/devices', actions: DEVICE_ACTIONS, send: (a, action, named) => ask(a, 'POST', '/api/devices', post(action, named)) }] },
  { path: 'Kiosk', line: 'server.mjs:8184, 7932-8046', routes: [{ route: 'POST /api/kiosk', actions: KIOSK_ACTIONS, send: (a, action, named) => ask(a, 'POST', '/api/kiosk', post(action, named, RPC_EXTRA)) }] },
  { path: 'OwnerRoute', line: 'server.mjs:8158-8160, 8278-8279', routes: [
    { route: 'POST /api/employees', actions: EMPLOYEE_ACTIONS, send: (a, action, named) => ask(a, 'POST', '/api/employees', post(action, named, { id: 'E-FRONT' })) },
    { route: 'POST /api/bookings', actions: ['registrations', 'nope'], send: (a, action, named) => ask(a, 'POST', '/api/bookings', post(action, named)) },
    { route: 'POST /api/reference-rates', actions: ['reference-rates'], send: (a) => ask(a, 'POST', '/api/reference-rates', '{}') },
  ] },
  { path: 'UserRoute', line: 'server.mjs:8144-8147, 8176-8177, 8185-8187, 8276-8283', routes: [
    { route: 'GET /documents', actions: ['documents'], send: (a, _action, named) => ask(a, 'GET', '/documents' + (named ? '?branch=' + named + '&ref=X' : '?ref=X')) },
    { route: 'GET /devices/receipt', actions: ['receipt'], send: (a, _action, named) => ask(a, 'GET', '/devices/receipt' + (named ? '?branch=' + named + '&ref=X' : '?ref=X')) },
    { route: 'GET /', actions: ['admin'], send: (a, _action, named) => ask(a, 'GET', '/' + q(named)) },
    { route: 'GET /desk', actions: ['desk'], send: (a, _action, named) => ask(a, 'GET', '/desk' + q(named)) },
  ] },
];

// ── the custody module is asked directly: its role check sits inside the engine ──
const CUSTODY_ROLE_SENTENCES = ['บทบาทนี้ไม่มีสิทธิ์ทำรายการ', 'ไม่มีสิทธิ์ลงชื่อ', 'Backend ใช้ปุ่มตรวจและบันทึกงาน', 'ต้องใช้บัญชีพนักงาน'];
function custody(actor, action) {
  // A branch where the procedure is on, the caller has clocked in and one hand-over is
  // waiting, so that the checks v2 makes before it looks at the role (6015, 6032, 6148-6150)
  // pass. opsSign is asked about a request in transit addressed to the caller, so that
  // 6163-6166 pass for whoever may sign. The owner has no employee id and cannot clock in
  // (6017): for the owner the clock-in sentence of 6032 is the answer to who is calling.
  const employee = actor.kind === 'staff' ? 'E-' + actor.role : undefined;
  const state = v2.initialOperations();
  state.enabled = true; state.entity = { company: 'C', site: 'S' }; state.phase = 'OPEN';
  state.requests.push({ id: 'r1', requestId: KEY, kind: 'SAFE_IN', status: 'PENDING', sender: 'E-X', receiver: employee ?? 'E-Y', signatures: [], ...(action === 'opsSign' ? { stage: 'IN_TRANSIT' } : {}) });
  if (employee) state.attendance.push({ id: 'a1', employeeId: employee, name: actor.role, role: actor.role, in: '2026-10-06T05:00:00.000Z', out: null });
  if (action === 'opsSetEntity' || action === 'opsEnable') state.enabled = false;
  const model = { stock: { shifts: [], movements: [] }, transactions: [], customers: [], updated: '' };
  const stock = { active: () => null, expected: () => ({}), balances: (v) => v };
  const who = { id: actor.kind === 'owner' ? 'owner' : employee, employeeId: employee, role: actor.kind === 'owner' ? undefined : actor.role, name: actor.role };
  try { v2.runOperations(action, { id: 'r1' }, who, state, model, stock, () => ({})); } catch (e) {
    if (actor.kind === 'owner' && e.message === 'ลงเวลาเข้าก่อนทำรายการ') return { allowed: false, error: e.message, status: 400 };
    if (CUSTODY_ROLE_SENTENCES.includes(e.message)) return { allowed: false, error: e.message, status: 400 };
    // An unknown ops action falls off the end of the module (6250) whoever calls.
    if (!OPS.includes(action) && e.message === 'คำขอไม่ถูกต้อง') return { allowed: false, error: e.message, status: 400 };
    if (!OPS.includes(action)) throw Error('custody ' + action + ': ' + e.message);
    return { allowed: true, branch: null }; // refused later, for a reason that is not the role
  }
  return { allowed: true, branch: null };
}

// ── the matrix ──────────────────────────────────────────────────────────────
const cases = [], disagreements = [];
const row = (path, route, actor, action, named, out) => ({ path, route, actor: actor.role, action, own: actor.own, named, ...out });
for (const p of PATHS) for (const r of p.routes) for (const actor of ACTORS) for (const action of r.actions) {
  cases.push(row(p.path, r.route, actor, action, OTHER, await r.send(actor, action, OTHER)));
}
for (const actor of ACTORS.filter((a) => a.kind !== 'anon')) for (const action of [...OPS, 'opsNope']) cases.push(row('Custody', 'engine', actor, action, null, custody(actor, action)));

// ── the branch rule: nothing named, the caller's own branch, somebody else's ──
const branchCases = [];
const BRANCH_PROBES = [
  ['Rpc', 'POST /api/rpc', 'adminSnapshot'], ['StaffRpc', 'POST /api/staff/rpc', 'adminSnapshot'], ['Upload', 'POST /api/upload', 'upload'],
  ['StaffUpload', 'POST /api/staff/upload', 'upload'], ['Devices', 'POST /api/devices', 'snapshot'], ['Kiosk', 'POST /api/kiosk', 'rates'],
  ['Kiosk', 'POST /api/kiosk', 'notifications'], ['UserRoute', 'GET /documents', 'documents'], ['UserRoute', 'GET /devices/receipt', 'receipt'],
];
for (const [path, route, action] of BRANCH_PROBES) {
  const r = PATHS.find((p) => p.path === path).routes.find((x) => x.route === route);
  for (const actor of ACTORS) for (const named of [null, OWN, OTHER]) {
    const out = await r.send(actor, action, named);
    if (out.allowed) branchCases.push(row(path, route, actor, action, named, out));
  }
}

// ── checks: a matrix that cannot be wrong in an obvious way ─────────────────
const fail = (m) => { throw Error('suspicious: ' + m); };
const allowedOf = (f) => cases.filter(f).filter((c) => c.allowed).length, total = (f) => cases.filter(f).length;
for (const p of [...PATHS.map((x) => x.path), 'Custody']) {
  const a = allowedOf((c) => c.path === p), n = total((c) => c.path === p);
  if (a === 0 || a === n) fail(`path ${p} is ${a === 0 ? 'never' : 'always'} allowed (${n} cases)`);
}
for (const actor of ACTORS) {
  const a = allowedOf((c) => c.actor === actor.role), n = total((c) => c.actor === actor.role);
  if (a === 0 || a === n) fail(`${actor.role} is ${a === 0 ? 'never' : 'always'} allowed (${n} cases)`);
}
// every known action on a path that has a role table: somebody may, somebody may not
// (the two owner-only custody actions are refused to every employee on the staff bridge, server.mjs:7436)
const OWNER_OPS = ['opsEnable', 'opsSetEntity'];
const known = { Rpc: [...RPC], StaffRpc: [...BRIDGE, ...OPS.filter((x) => !OWNER_OPS.includes(x))], Devices: Object.keys(v2.PERMITS), Kiosk: KIOSK_ACTIONS.filter((x) => !['rates', 'quote', 'confirm', 'status', 'customerReceipt', 'nope'].includes(x)), Custody: OPS };
for (const action of OWNER_OPS) if (allowedOf((c) => c.path === 'StaffRpc' && c.action === action) !== 0) fail('StaffRpc lets somebody through with ' + action);
for (const [p, list] of Object.entries(known)) for (const action of list) {
  const a = allowedOf((c) => c.path === p && c.action === action), n = total((c) => c.path === p && c.action === action);
  if (a === 0 || a === n) fail(`${p} ${action} is ${a === 0 ? 'never' : 'always'} allowed`);
}
// an unknown action is let through by nobody, except where v2 looks at the name only after the gate
for (const c of cases) if (UNKNOWN.includes(c.action) && c.allowed && !['Devices', 'OwnerRoute'].includes(c.path) && !(c.path === 'StaffRpc' && c.action.startsWith('ops'))) fail(`${c.path} lets ${c.actor} through with the unknown action ${JSON.stringify(c.action)}`);
// routes that share a path must agree for every caller
for (const p of PATHS.filter((x) => x.routes.length > 1)) for (const actor of ACTORS) {
  const seen = new Set(cases.filter((c) => c.path === p.path && c.actor === actor.role).map((c) => JSON.stringify([c.allowed, c.error ?? null])));
  if (seen.size !== 1) fail(`${p.path}: its routes do not agree for ${actor.role}: ${[...seen].join(' | ')}`);
}
// staff always end on their own branch; the owner on the branch named, MAIN when none is
for (const c of [...cases, ...branchCases]) {
  if (!c.allowed || c.branch === null) continue;
  const want = c.own ?? c.named ?? 'MAIN';
  // the kiosk reads its public model as the shop, on the branch it computed (7905, 7933)
  if (c.branch !== want) fail(`${c.route} ${c.action} as ${c.actor} naming ${c.named}: ran on ${c.branch}, expected ${want}`);
}
if (!branchCases.some((c) => c.own && c.named === OTHER) || !branchCases.some((c) => !c.own && c.named === null && c.branch === 'MAIN')) fail('the branch probes do not cover staff naming another branch and nobody naming one');
for (const s of SENTENCES.keys()) if (!cases.some((c) => c.error === s || c.behind?.error === s) && !['คำขอไม่รองรับ', 'Unknown kiosk action.'].includes(s)) fail('sentence never seen: ' + s);

// ── where two v2 paths give different answers to the same caller and action ──
const cell = (path, actor, action) => cases.find((c) => c.path === path && c.actor === actor && c.action === action);
for (const actor of ROLES) for (const action of RPC_ACTIONS.filter((x) => !UNKNOWN.includes(x))) {
  const a = cell('Rpc', actor, action), b = cell('StaffRpc', actor, action);
  if (a.allowed !== b.allowed) disagreements.push({ actor, action, Rpc: a.allowed ? 'allowed' : a.error, StaffRpc: b.allowed ? 'allowed' : b.error });
}

const refusals = [...new Set(cases.filter((c) => !c.allowed).map((c) => c.behind ?? c).filter((c) => c.error !== null).map((c) => c.error + '\t' + c.status))].sort().map((x) => { const [error, status] = x.split('\t'); return { error, status: Number(status) }; });
const counts = {
  cases: cases.length, allowed: cases.filter((c) => c.allowed).length, refused: cases.filter((c) => !c.allowed).length,
  redirects: cases.filter((c) => c.redirect).length, branchCases: branchCases.length, paths: PATHS.length + 1, actors: ACTORS.length,
  distinctRefusals: refusals.length, disagreements: disagreements.length,
};
const lines = (a) => '[\n' + a.map((x) => '    ' + JSON.stringify(x)).join(',\n') + '\n  ]';
process.stdout.write('{\n  "source": "v2 server.mjs (xchange-online@3ffc5a5), driven through handle() and runOperations() by api/tools/v2-fixtures/gen_policy.mjs",\n'
  + '  "counts": ' + JSON.stringify(counts) + ',\n'
  + '  "roles": ' + JSON.stringify(ROLES) + ',\n'
  + '  "paths": ' + lines([...PATHS.map((p) => ({ path: p.path, v2: p.line, routes: p.routes.map((r) => r.route) })), { path: 'Custody', v2: 'server.mjs:5969-6250', routes: ['engine'] }]) + ',\n'
  + '  "refusals": ' + lines(refusals) + ',\n'
  + '  "disagreements": ' + lines(disagreements) + ',\n'
  + '  "cases": ' + lines(cases) + ',\n'
  + '  "branchCases": ' + lines(branchCases) + '\n}\n');
