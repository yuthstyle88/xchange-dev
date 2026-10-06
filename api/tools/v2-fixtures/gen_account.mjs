// Drives the account code of the v2 server and records what it answered.
//   node gen_account.mjs > ../../crates/xc-auth/tests/fixtures/v2_account_cases.json
//
// It needs `account.mjs` beside it: three pieces of the deployed bundle's `server.mjs`
// (`xchange-online@3ffc5a5`), unchanged and joined in this order:
//   lines 7284-7369  from `// src/online/lib/employees.ts` up to, not including,
//                    `// src/online/app/api/employees/route.ts`
//   lines 7467-7470  `// shims/staff-proxy.mjs` and its first function, `employeeCookie`
//   lines 7804-7890  from `// src/auth.mjs` up to, not including, `// src/kiosk.mjs`
// The bundle is not in this repository, so this cannot run in CI; its output is checked in and
// is what the tests read.
//
// The code reads the wall clock, draws random bytes and ids, and talks to PostgreSQL through two
// handles (`env.DB`, a D1-shaped wrapper, and `pool`). It is evaluated from its text with those
// supplied from here, so that the output is the same on every run:
//   Date                       → a clock this script sets
//   randomBytes, randomUUID7   → counters
//   env.DB, pool               → one in-memory store that interprets the code's own SQL texts
//                                (below); it is not PostgreSQL, and it throws on any statement
//                                it does not know
//   scrypt, createHash, timingSafeEqual → node's own
// Nothing inside the code is replaced. The shared sign-in, status and sign-out go through v2's
// HTTP function (`api2`) with real `Request` objects; the rest are v2's own functions, called as
// the staff bridge calls them (`server.mjs:7409-7413`).
//
// v2's password hashes (scrypt) are not recorded: v3 hashes with Argon2id (design §6).
import fs from 'node:fs';
import crypto from 'node:crypto';

let src = fs.readFileSync(new URL('./account.mjs', import.meta.url), 'utf8');
for (const needle of ['async function manageEmployees(', 'async function loginEmployee(', 'async function employeeSession(', 'async function changeEmployeePassword(', 'async function api2(', 'async function ownerLogin(', 'function employeeCookie(']) {
  if (!src.includes(needle)) throw Error('account.mjs: ' + needle + ' not found');
}
src = src.replace(/^import .*$/gm, '');
if (/setTimeout|Math\.random|fetch\(/.test(src)) throw Error('account.mjs reads something this script does not replace');

// ── the clock ────────────────────────────────────────────────────────────
const RealDate = Date;
let clock = RealDate.UTC(2026, 9, 7, 3, 0, 0, 0); // 2026-10-07T03:00:00.000Z, the start of a 15-minute window
if (clock % 9e5 !== 0) throw Error('the clock must start on a window boundary');
class FakeDate extends RealDate {
  constructor(...a) { if (a.length) super(...a); else super(clock); }
  static now() { return clock; }
}

// ── counters ─────────────────────────────────────────────────────────────
let bytes = 0, uuids = 0;
const randomBytes = (n) => {
  const out = Buffer.alloc(n);
  let filled = 0;
  while (filled < n) { const h = crypto.createHash('sha256').update('bytes' + bytes++).digest(); h.copy(out, filled); filled += h.length; }
  return out;
};
const randomUUID = () => '00000000-0000-4000-8000-' + String(++uuids).padStart(12, '0');

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
const env = { DB: { prepare: (sql) => stmt(sql) } };
const pool = { query: async (sql, args = []) => ({ rows: run(sql, args) }) };

const v2 = new Function(
  'env', 'pool', 'Date', 'process',
  'randomBytes', 'randomUUID7', 'createHash5', 'scrypt', 'timingSafeEqual2',
  'randomBytes2', 'createHash7', 'scrypt2', 'timingSafeEqual3',
  src + '\nreturn { manageEmployees, loginEmployee, employeeSession, logoutEmployee, changeEmployeePassword, passwordHash, api2 };',
)(env, pool, FakeDate, { env: {} }, randomBytes, randomUUID, crypto.createHash, crypto.scrypt, crypto.timingSafeEqual, randomBytes, crypto.createHash, crypto.scrypt, crypto.timingSafeEqual);

// ── symbols: the fixture never carries a v2 id or token ───────────────────
const tokenName = new Map(), tokenValue = new Map();
const nameToken = (hex) => { if (!tokenName.has(hex)) { const n = '$t' + (tokenName.size + 1); tokenName.set(hex, n); tokenValue.set(n, hex); } return tokenName.get(hex); };
const realToken = (x) => (typeof x === 'string' && tokenValue.has(x) ? tokenValue.get(x) : x);
const employeeName = (id) => { const i = t.employees.findIndex((e) => e.id === id); return i < 0 ? id : '$e' + (i + 1); };
const realEmployee = (x) => { const m = /^\$e(\d+)$/.exec(typeof x === 'string' ? x : ''); return m && t.employees[m[1] - 1] ? t.employees[m[1] - 1].id : x; };
const symbols = (x) => (typeof x === 'string' ? employeeName(x) : Array.isArray(x) ? x.map(symbols) : x && typeof x === 'object' ? Object.fromEntries(Object.entries(x).map(([k, v]) => [k, symbols(v)])) : x);
const resolve = (x) => (typeof x === 'string' ? realEmployee(x) : Array.isArray(x) ? x.map(resolve) : x && typeof x === 'object' ? Object.fromEntries(Object.entries(x).map(([k, v]) => [k, resolve(v)])) : x);

const employeesNow = () => [...t.employees].sort((a, b) => (a.username < b.username ? -1 : 1)).map((e) => ({ id: employeeName(e.id), username: e.username, name: e.name, role: e.role, branchId: e.branch_id, active: !!e.active, mustChange: !!e.must_change, version: e.version, createdAt: e.created_at }));
const auditFrom = (n) => t.employee_audit.slice(n).map((a) => ({ actor: employeeName(a.actor), subject: employeeName(a.subject), action: a.action, at: a.at }));

// ── the operations ───────────────────────────────────────────────────────
const ORIGIN = 'https://desk.example';
const cookieHeader = (c) => [c.owner != null ? '__Host-xc_owner=' + realToken(c.owner) : '', c.staff != null ? '__Host-xc_staff=' + realToken(c.staff) : ''].filter(Boolean).join('; ');
async function http(action, payload, { ip, cookies = {} }) {
  const headers = { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json' };
  if (ip != null) headers['x-real-ip'] = ip;
  const c = cookieHeader(cookies);
  if (c) headers.cookie = c;
  const res = await v2.api2(new Request(ORIGIN + '/api/auth', { method: 'POST', headers, body: JSON.stringify({ action, payload }) }));
  const body = await res.json();
  const set = {};
  for (const line of res.headers.getSetCookie()) {
    const m = /^(__Host-xc_(owner|staff))=([^;]*); Path=\/; HttpOnly; Secure; SameSite=Strict; Max-Age=(\d+)$/.exec(line);
    if (!m) throw Error('unexpected Set-Cookie: ' + line);
    set[m[2]] = m[3] === '' ? { cleared: true, maxAge: Number(m[4]) } : { token: nameToken(m[3]), maxAge: Number(m[4]) };
  }
  if (res.status !== 200) return { refused: body.error };
  return { ok: symbols(body), setCookies: set };
}
const call = async (f) => { try { return { ok: symbols(await f()) }; } catch (e) { return { refused: e.message }; } };
const OPS = {
  login: (a) => http('login', a.payload, a),
  status: (a) => http('status', {}, a),
  logout: (a) => http('logout', {}, a),
  staffLogin: async (a) => {
    const r = await call(() => v2.loginEmployee(a.payload, a.ip));
    if (r.ok) r.ok = { token: nameToken(tokenValueOf(r.ok.token)), employee: r.ok.employee };
    return r;
  },
  session: (a) => call(() => v2.employeeSession(realToken(a.token), a.allowChange)),
  staffLogout: (a) => call(() => v2.logoutEmployee(realToken(a.token))),
  password: (a) => call(() => v2.changeEmployeePassword(realToken(a.token), a.payload)),
  employees: (a) => call(() => v2.manageEmployees(a.action, resolve(a.payload), a.actor)),
};
const tokenValueOf = (x) => x; // the raw hex as v2 returned it (symbols() leaves 64-hex strings alone)

const stories = [];
let steps, last, audited;
async function story(name, about, f) {
  for (const k of Object.keys(t)) t[k] = [];
  tokenName.clear(); tokenValue.clear();
  clock = RealDate.UTC(2026, 9, 7, 3, 0, 0, 0);
  steps = []; last = JSON.stringify(employeesNow()); audited = 0;
  // The owner exists before the story starts, as after v2's `create-owner.mjs`.
  t.owners.push({ id: 'owner-row-1', username: 'boss', name: 'boss', password_hash: await v2.passwordHash(OWNER_PASSWORD), active: true, version: 1, created_at: new RealDate(clock).toISOString() });
  await f();
  stories.push({ name, about, owner: { username: 'boss', name: 'boss', password: OWNER_PASSWORD }, steps });
}
const OWNER_PASSWORD = 'owner-password-01';
async function step(note, op, args = {}) {
  const answer = await OPS[op](args);
  const s = { note, at: clock, op, ...args, ...answer };
  const now = JSON.stringify(employeesNow());
  if (now !== last) { s.employees = JSON.parse(now); last = now; }
  if (t.employee_audit.length > audited) { s.audit = auditFrom(audited); audited = t.employee_audit.length; }
  if (answer.refused && (s.employees || s.audit)) throw Error('a refused step changed what is stored: ' + note);
  steps.push(s);
  return answer;
}
const expectOk = (r, note) => { if (!r.ok) throw Error(note + ': expected v2 to accept, got ' + r.refused); return r; };
const expectRefused = (r, note) => { if (!r.refused) throw Error(note + ': expected v2 to refuse'); return r; };
const tick = (ms) => { clock += ms; };

const WRONG = 'ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง';
const LIMIT = 'ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที';
const mustBe = (r, sentence, note) => { if (r.refused !== sentence) throw Error(note + ': expected «' + sentence + '», got ' + JSON.stringify(r)); };

const P1 = 'first-password-1', P2 = 'second-password-2', P3 = 'third-password-33';
const valid = (over = {}) => ({ username: 'somchai', name: 'สมชาย ใจดี', role: 'FRONT', branchId: 'MAIN', password: P1, ...over });

await story('accounts', 'Creating employees, signing in, the first-password change, sessions, reset and disable.', async () => {
  await step('nobody yet', 'employees', { action: 'list', payload: {}, actor: 'owner' });

  // create — what is refused, in v2's order: the fields first, then the password
  const bad = [
    ['username of two characters', { username: 'ab' }],
    ['username of 41 characters', { username: 'a'.repeat(41) }],
    ['username with a space inside', { username: 'som chai' }],
    ['username with a Thai letter', { username: 'สมชาย' }],
    ['username missing', { username: undefined }],
    ['username null', { username: null }],
    ['name empty', { name: '' }],
    ['name of spaces', { name: '   ' }],
    ['name of 101 characters', { name: 'ก'.repeat(101) }],
    ['role unknown', { role: 'OWNER' }],
    ['role in lower case', { role: 'front' }],
    ['role missing', { role: undefined }],
    ['branch empty', { branchId: '' }],
    ['branch with an underscore', { branchId: 'MAIN_1' }],
    ['branch of 71 characters', { branchId: 'B'.repeat(71) }],
    ['password of 11 characters', { password: 'a'.repeat(11) }],
    ['password of 129 characters', { password: 'a'.repeat(129) }],
    ['password missing', { password: undefined }],
    ['password a number', { password: 123456789012 }],
    ['bad field and bad password together: the field sentence wins', { role: 'NOPE', password: 'short' }],
  ];
  for (const [note, over] of bad) expectRefused(await step('create refused: ' + note, 'employees', { action: 'create', payload: valid(over), actor: 'owner' }), note);

  expectOk(await step('create somchai — the username is trimmed and lower-cased, the name trimmed', 'employees', { action: 'create', payload: valid({ username: '  SomChai ', name: '  สมชาย ใจดี ' }), actor: 'owner' }), 'create');
  expectRefused(await step('create refused: the username is taken (v2 answers with the database error text)', 'employees', { action: 'create', payload: valid({ name: 'อีกคน' }), actor: 'owner' }), 'dup');
  expectRefused(await step('create refused: the username is taken, written in capitals', 'employees', { action: 'create', payload: valid({ username: 'SOMCHAI' }), actor: 'owner' }), 'dup2');
  tick(1000);
  expectOk(await step('create a manager: username of 3 characters, name of 100, branch of 70, password of 12', 'employees', { action: 'create', payload: valid({ username: 'a.b', name: 'ข'.repeat(100), role: 'MANAGER', branchId: 'b'.repeat(70), password: 'x'.repeat(12) }), actor: 'owner' }), 'create2');
  expectOk(await step('create: username of 40 characters with . _ -, password of 128', 'employees', { action: 'create', payload: valid({ username: 'z_9-' + 'q'.repeat(36), name: 'Zed', role: 'DOCUMENT_REVIEW', branchId: 'BR-2', password: 'p'.repeat(128) }), actor: 'owner' }), 'create3');
  expectOk(await step('create: a password of six emoji is 12 UTF-16 units and is accepted', 'employees', { action: 'create', payload: valid({ username: 'emoji', name: 'Emoji', role: 'RATES', password: '😀'.repeat(6) }), actor: 'owner' }), 'create4');
  expectRefused(await step('create refused: five emoji are 10 units', 'employees', { action: 'create', payload: valid({ username: 'emoji5', password: '😀'.repeat(5) }), actor: 'owner' }), 'emoji5');
  for (const role of ['BACKEND', 'MONITOR', 'FINANCE', 'SYSTEM', 'DOCUMENTS']) expectOk(await step('create a ' + role, 'employees', { action: 'create', payload: valid({ username: 'r-' + role.toLowerCase(), name: role, role }), actor: 'owner' }), role);
  await step('the list is ordered by username', 'employees', { action: 'list', payload: {}, actor: 'owner' });

  // employee actions that need an employee
  mustBe(await step('reset refused: unknown employee', 'employees', { action: 'reset', payload: { id: 'nobody', password: P2 }, actor: 'owner' }), 'ไม่พบพนักงาน', 'reset unknown');
  mustBe(await step('reset refused: unknown employee and a bad password — the employee is looked up first', 'employees', { action: 'reset', payload: { id: 'nobody', password: 'short' }, actor: 'owner' }), 'ไม่พบพนักงาน', 'reset order');
  mustBe(await step('disable refused: no id', 'employees', { action: 'disable', payload: {}, actor: 'owner' }), 'ไม่พบพนักงาน', 'disable no id');
  mustBe(await step('unknown action with no employee: the employee is looked up first', 'employees', { action: 'promote', payload: {}, actor: 'owner' }), 'ไม่พบพนักงาน', 'unknown action');
  mustBe(await step('unknown action on a real employee', 'employees', { action: 'promote', payload: { id: '$e1' }, actor: 'owner' }), 'คำขอไม่ถูกต้อง', 'unknown action 2');

  // sign-in
  tick(60_000);
  mustBe(await step('sign-in refused: wrong password', 'login', { payload: { username: 'somchai', password: 'wrong-password-1' }, ip: '10.0.0.1' }), WRONG, 'wrong');
  mustBe(await step('sign-in refused: nobody by that name', 'login', { payload: { username: 'nobody', password: P1 }, ip: '10.0.0.1' }), WRONG, 'nobody');
  mustBe(await step('sign-in refused: no password', 'login', { payload: { username: 'somchai' }, ip: '10.0.0.1' }), WRONG, 'nopass');
  mustBe(await step('sign-in refused: the password is a number', 'login', { payload: { username: 'somchai', password: 12345678901234 }, ip: '10.0.0.1' }), WRONG, 'numpass');
  mustBe(await step('sign-in refused: a password of 129 characters is never checked', 'login', { payload: { username: 'somchai', password: 'a'.repeat(129) }, ip: '10.0.0.1' }), WRONG, 'long');
  mustBe(await step('sign-in refused: a username of 41 characters', 'login', { payload: { username: 'a'.repeat(41), password: P1 }, ip: '10.0.0.1' }), WRONG, '41');
  mustBe(await step('sign-in refused: no username', 'login', { payload: { password: P1 }, ip: '10.0.0.1' }), WRONG, 'nouser');
  mustBe(await step('sign-in refused: empty payload', 'login', { payload: {}, ip: '10.0.0.1' }), WRONG, 'empty');
  const first = expectOk(await step('sign-in: the username is trimmed and lower-cased; the answer says the password must be changed', 'login', { payload: { username: '  SOMCHAI ', password: P1 }, ip: '10.0.0.1' }), 'login');
  const t1 = first.setCookies.staff.token;

  // the first password
  mustBe(await step('a must-change session is refused for work', 'session', { token: t1, allowChange: false }), 'กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน', 'must change');
  expectOk(await step('…but may be read when changing the password is allowed', 'session', { token: t1, allowChange: true }), 'allow');
  await step('status with a must-change session: nobody', 'status', { cookies: { staff: t1 } });
  for (const [note, pw] of [['11 characters', 'b'.repeat(11)], ['129 characters', 'b'.repeat(129)], ['missing', undefined], ['a number', 1234567890123]]) expectRefused(await step('password change refused: ' + note, 'password', { token: t1, payload: { password: pw } }), note);
  expectRefused(await step('password change refused: no session', 'password', { token: '', payload: { password: P2 } }), 'nosession');
  expectRefused(await step('password change refused: an unknown token', 'password', { token: 'f'.repeat(64), payload: { password: P2 } }), 'unknown token');
  mustBe(await step('no session and a bad password: the session is looked at first', 'password', { token: '', payload: { password: 'short' } }), 'EMPLOYEE_AUTH_REQUIRED', 'order');
  tick(5000);
  expectOk(await step('the password is changed', 'password', { token: t1, payload: { password: P2 } }), 'change');
  expectRefused(await step('the session that changed it is over', 'session', { token: t1, allowChange: true }), 'old session');
  expectRefused(await step('…and cannot change it again', 'password', { token: t1, payload: { password: P3 } }), 'again');
  mustBe(await step('the first password no longer signs in', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.0.0.1' }), WRONG, 'old pw');
  const second = expectOk(await step('the new one does, and nothing must be changed', 'login', { payload: { username: 'somchai', password: P2 }, ip: '10.0.0.1' }), 'login2');
  const t2 = second.setCookies.staff.token;
  expectOk(await step('the session works', 'session', { token: t2, allowChange: false }), 'session');
  await step('status with it', 'status', { cookies: { staff: t2 } });
  expectOk(await step('a password may be changed again without being asked to', 'password', { token: t2, payload: { password: P3 } }), 'change2');
  expectRefused(await step('…which ends that session too', 'session', { token: t2, allowChange: false }), 'ended');

  // two sessions, sign-out ends one
  const a = expectOk(await step('sign in at the staff door', 'staffLogin', { payload: { username: 'somchai', password: P3 }, ip: '10.0.0.2' }), 'staff login').ok.token;
  const b = expectOk(await step('sign in again: a second session', 'login', { payload: { username: 'somchai', password: P3 }, ip: '10.0.0.2' }), 'login3').setCookies.staff.token;
  expectOk(await step('sign out of the first', 'staffLogout', { token: a }), 'logout');
  expectRefused(await step('the first is over', 'session', { token: a, allowChange: false }), 'a over');
  expectOk(await step('the second is not', 'session', { token: b, allowChange: false }), 'b alive');
  expectOk(await step('signing out of a session that is not there is fine', 'staffLogout', { token: a }), 'logout twice');

  // malformed tokens
  for (const [note, tok] of [['empty', ''], ['63 characters', 'a'.repeat(63)], ['65 characters', 'a'.repeat(65)], ['capitals', 'A'.repeat(64)], ['not hex', 'g'.repeat(64)]]) expectRefused(await step('session refused: token ' + note, 'session', { token: tok, allowChange: true }), note);

  // twelve hours
  const made = steps.find((s) => s.setCookies?.staff?.token === b).at;
  clock = made + 12 * 36e5 - 1;
  expectOk(await step('one millisecond before twelve hours the session works', 'session', { token: b, allowChange: false }), '12h-1');
  clock = made + 12 * 36e5;
  expectRefused(await step('at twelve hours it is over', 'session', { token: b, allowChange: false }), '12h');

  // reset
  tick(1000);
  const c = expectOk(await step('sign in', 'login', { payload: { username: 'somchai', password: P3 }, ip: '10.0.0.3' }), 'login4').setCookies.staff.token;
  for (const [note, pw] of [['11 characters', 'c'.repeat(11)], ['missing', undefined]]) expectRefused(await step('reset refused: password ' + note, 'employees', { action: 'reset', payload: { id: '$e1', password: pw }, actor: 'owner' }), note);
  expectOk(await step('the session survives a refused reset', 'session', { token: c, allowChange: false }), 'survives');
  expectOk(await step('the owner resets the password', 'employees', { action: 'reset', payload: { id: '$e1', password: P1 }, actor: 'owner' }), 'reset');
  expectRefused(await step('the reset ends the session', 'session', { token: c, allowChange: true }), 'reset ends');
  mustBe(await step('the password before the reset no longer signs in', 'login', { payload: { username: 'somchai', password: P3 }, ip: '10.0.0.3' }), WRONG, 'pre-reset');
  const d = expectOk(await step('the reset password signs in and must be changed', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.0.0.3' }), 'login5').setCookies.staff.token;
  mustBe(await step('must change again', 'session', { token: d, allowChange: false }), 'กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน', 'must change 2');

  // disable
  expectOk(await step('the owner disables the account', 'employees', { action: 'disable', payload: { id: '$e1' }, actor: 'owner' }), 'disable');
  expectRefused(await step('its session is over', 'session', { token: d, allowChange: true }), 'disabled session');
  mustBe(await step('the right password no longer signs in', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.0.0.3' }), WRONG, 'disabled login');
  mustBe(await step('nor at the staff door', 'staffLogin', { payload: { username: 'somchai', password: P1 }, ip: '10.0.0.3' }), WRONG, 'disabled staff login');
  expectOk(await step('disabling again is accepted and counts as another change', 'employees', { action: 'disable', payload: { id: '$e1' }, actor: 'owner' }), 'disable2');
  expectOk(await step('a disabled account can be given a new password; it stays disabled', 'employees', { action: 'reset', payload: { id: '$e1', password: P2 }, actor: 'owner' }), 'reset disabled');
  mustBe(await step('…and still cannot sign in', 'login', { payload: { username: 'somchai', password: P2 }, ip: '10.0.0.3' }), WRONG, 'still disabled');
  await step('the list at the end', 'employees', { action: 'list', payload: {}, actor: 'owner' });
});

await story('owner', 'The owner signs in at the shared door; an owner session and a staff session side by side.', async () => {
  expectOk(await step('an employee to share the door with', 'employees', { action: 'create', payload: valid(), actor: 'owner' }), 'create');
  await step('status with no cookie: nobody', 'status', {});
  mustBe(await step('owner sign-in refused: wrong password', 'login', { payload: { username: 'boss', password: 'wrong-password-1' }, ip: '10.1.0.1' }), WRONG, 'owner wrong');
  mustBe(await step('owner sign-in refused: no password', 'login', { payload: { username: 'boss' }, ip: '10.1.0.1' }), WRONG, 'owner nopass');
  const o = expectOk(await step('the owner signs in: capitals and spaces in the username; the answer has no mustChange', 'login', { payload: { username: ' BOSS ', password: OWNER_PASSWORD }, ip: '10.1.0.1' }), 'owner login');
  const ot = o.setCookies.owner.token;
  await step('status with the owner cookie', 'status', { cookies: { owner: ot } });
  mustBe(await step('the staff door does not know the owner', 'staffLogin', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.1.0.1' }), WRONG, 'staff door');
  expectRefused(await step('an owner token is not a staff session', 'session', { token: ot, allowChange: true }), 'owner token as staff');
  const st = expectOk(await step('the employee signs in', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.1.0.2' }), 'emp login').setCookies.staff.token;
  expectOk(await step('…and changes the first password', 'password', { token: st, payload: { password: P2 } }), 'change');
  const st2 = expectOk(await step('…and signs in again', 'login', { payload: { username: 'somchai', password: P2 }, ip: '10.1.0.2' }), 'emp login 2').setCookies.staff.token;
  await step('status with both cookies: the owner wins', 'status', { cookies: { owner: ot, staff: st2 } });
  await step('status with a staff token in the owner cookie and a staff cookie: the employee', 'status', { cookies: { owner: st2, staff: st2 } });
  await step('status with a malformed owner cookie and a staff cookie: the employee', 'status', { cookies: { owner: 'nonsense', staff: st2 } });
  await step('status with an owner token in the staff cookie only: nobody', 'status', { cookies: { staff: ot } });
  const made = steps.find((s) => s.setCookies?.owner?.token === ot).at;
  const keep = clock;
  clock = made + 12 * 36e5 - 1;
  await step('one millisecond before twelve hours the owner session works', 'status', { cookies: { owner: ot } });
  clock = made + 12 * 36e5;
  await step('at twelve hours it is over', 'status', { cookies: { owner: ot } });
  clock = keep + 1000;
  expectOk(await step('sign-out ends both sessions it is given', 'logout', { cookies: { owner: ot, staff: st2 } }), 'logout');
  await step('the owner session is over', 'status', { cookies: { owner: ot } });
  await step('the staff session is over', 'status', { cookies: { staff: st2 } });
  expectOk(await step('sign-out with no cookie is fine', 'logout', {}), 'logout none');
  expectOk(await step('sign-out with malformed cookies is fine', 'logout', { cookies: { owner: 'x', staff: 'y' } }), 'logout bad');

  // an owner who is not active
  t.owners[0].active = false;
  steps.push({ note: 'the owner row is made inactive (directly in the table; v2 has no action for it)', at: clock, op: 'ownerActive', active: false });
  const o2 = await step('an inactive owner cannot sign in with the right password', 'login', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.1.0.1' });
  mustBe(o2, WRONG, 'inactive owner');
  t.owners[0].active = true;
  steps.push({ note: 'the owner row is made active again', at: clock, op: 'ownerActive', active: true });
  const o3 = expectOk(await step('…and can again', 'login', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.1.0.1' }), 'owner again').setCookies.owner.token;
  t.owners[0].active = false;
  steps.push({ note: 'inactive once more', at: clock, op: 'ownerActive', active: false });
  await step('a session of an inactive owner is nobody', 'status', { cookies: { owner: o3 } });
});

await story('limits', 'Sign-in attempts are counted before the password is checked: 10 per username and 30 per address in a 15-minute window.', async () => {
  expectOk(await step('an employee', 'employees', { action: 'create', payload: valid(), actor: 'owner' }), 'create');
  expectOk(await step('another', 'employees', { action: 'create', payload: valid({ username: 'malee', name: 'มาลี' }), actor: 'owner' }), 'create2');

  // per username, from different addresses
  for (let i = 1; i <= 10; i++) mustBe(await step('wrong password for somchai, attempt ' + i, 'login', { payload: { username: 'somchai', password: 'wrong-password-1' }, ip: '10.2.0.' + i }), WRONG, 'u' + i);
  mustBe(await step('attempt 11 has the right password and is refused', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.2.0.11' }), LIMIT, 'u11');
  mustBe(await step('the same at the staff door: one counter', 'staffLogin', { payload: { username: 'somchai', password: P1 }, ip: '10.2.0.12' }), LIMIT, 'u12');
  mustBe(await step('capitals and spaces are the same username', 'login', { payload: { username: ' SomChai ', password: P1 }, ip: '10.2.0.13' }), LIMIT, 'u13');
  expectOk(await step('another username is not held back', 'login', { payload: { username: 'malee', password: P1 }, ip: '10.2.0.14' }), 'malee');
  clock = RealDate.UTC(2026, 9, 7, 3, 14, 59, 999);
  mustBe(await step('the last millisecond of the window: still refused', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.2.0.15' }), LIMIT, 'edge');
  clock = RealDate.UTC(2026, 9, 7, 3, 15, 0, 0);
  expectOk(await step('the next window starts on the quarter hour: accepted', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.2.0.15' }), 'next window');

  // successes count too
  for (let i = 2; i <= 10; i++) expectOk(await step('right password, attempt ' + i + ' of this window', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.2.1.' + i }), 's' + i);
  mustBe(await step('attempt 11 of this window is refused although every one before it was right', 'login', { payload: { username: 'somchai', password: P1 }, ip: '10.2.1.11' }), LIMIT, 's11');

  // per address, with different usernames; the owner and the staff door share the address counter
  clock = RealDate.UTC(2026, 9, 7, 3, 30, 0, 0);
  for (let i = 1; i <= 12; i++) mustBe(await step('address 10.3.0.1, attempt ' + i + ': an unknown username', 'login', { payload: { username: 'ghost' + i, password: P1 }, ip: '10.3.0.1' }), WRONG, 'ip' + i);
  for (let i = 13; i <= 20; i++) mustBe(await step('address 10.3.0.1, attempt ' + i + ': the owner, wrong password', 'login', { payload: { username: 'boss', password: 'wrong-password-1' }, ip: '10.3.0.1' }), WRONG, 'ip' + i);
  for (let i = 21; i <= 29; i++) mustBe(await step('address 10.3.0.1, attempt ' + i + ': the staff door', 'staffLogin', { payload: { username: 'ghost' + i, password: P1 }, ip: '10.3.0.1' }), WRONG, 'ip' + i);
  expectOk(await step('address 10.3.0.1, attempt 30: malee, right password, accepted', 'login', { payload: { username: 'malee', password: P1 }, ip: '10.3.0.1' }), 'ip30');
  mustBe(await step('attempt 31: malee again, refused', 'login', { payload: { username: 'malee', password: P1 }, ip: '10.3.0.1' }), LIMIT, 'ip31');
  mustBe(await step('attempt 32: the owner with the right password, refused', 'login', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.3.0.1' }), LIMIT, 'ip32');
  mustBe(await step('attempt 33: the staff door, refused', 'staffLogin', { payload: { username: 'malee', password: P1 }, ip: '10.3.0.1' }), LIMIT, 'ip33');
  expectOk(await step('malee from another address is accepted', 'login', { payload: { username: 'malee', password: P1 }, ip: '10.3.0.2' }), 'other ip');

  // the owner's own counter
  clock = RealDate.UTC(2026, 9, 7, 3, 45, 0, 0);
  for (let i = 1; i <= 10; i++) mustBe(await step('wrong password for the owner, attempt ' + i, 'login', { payload: { username: 'boss', password: 'wrong-password-1' }, ip: '10.4.0.' + i }), WRONG, 'o' + i);
  mustBe(await step('attempt 11 has the right password and is refused', 'login', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.4.0.11' }), LIMIT, 'o11');
  clock = RealDate.UTC(2026, 9, 7, 4, 0, 0, 0);
  expectOk(await step('the next window: accepted', 'login', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.4.0.11' }), 'o next');

  // the staff door keeps its own counter for a name, even the owner's
  clock = RealDate.UTC(2026, 9, 7, 4, 30, 0, 0);
  for (let i = 1; i <= 10; i++) mustBe(await step('the staff door, the owner\'s username, attempt ' + i, 'staffLogin', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.6.0.' + i }), WRONG, 'sd' + i);
  mustBe(await step('…the eleventh there is refused by the limit', 'staffLogin', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.6.0.11' }), LIMIT, 'sd11');
  expectOk(await step('…and the owner still signs in at the shared door', 'login', { payload: { username: 'boss', password: OWNER_PASSWORD }, ip: '10.6.0.12' }), 'owner not held');

  // what is not counted, and what is
  clock = RealDate.UTC(2026, 9, 7, 4, 15, 0, 0);
  for (let i = 1; i <= 31; i++) mustBe(await step('a username of 41 characters is refused before anything is counted, ' + i, 'login', { payload: { username: 'x'.repeat(41), password: P1 }, ip: '10.5.0.1' }), WRONG, 'long' + i);
  expectOk(await step('…so the address is not held back', 'login', { payload: { username: 'malee', password: P1 }, ip: '10.5.0.1' }), 'not counted');
  for (let i = 1; i <= 10; i++) mustBe(await step('no username at all is counted as the empty username, ' + i, 'login', { payload: { password: P1 }, ip: '10.5.1.' + i }), WRONG, 'empty' + i);
  mustBe(await step('…and the eleventh is refused', 'login', { payload: {}, ip: '10.5.1.11' }), LIMIT, 'empty11');
  for (let i = 1; i <= 30; i++) mustBe(await step('no address header: counted as the address "unknown", ' + i, 'login', { payload: { username: 'nobody' + i, password: P1 }, ip: null }), WRONG, 'unknown' + i);
  mustBe(await step('…and the 31st is refused', 'login', { payload: { username: 'malee', password: P1 }, ip: null }), LIMIT, 'unknown31');
});

const count = (f) => stories.reduce((n, s) => n + s.steps.filter(f).length, 0);
process.stdout.write(JSON.stringify({
  source: 'xchange-online@3ffc5a5 server.mjs:7284-7369,7467-7470,7804-7890, driven by api/tools/v2-fixtures/gen_account.mjs',
  note: 'Tokens are written $tN and employee ids $eN in the order v2 made them. `at` is the clock in milliseconds. A step carries `employees` (every employee, ordered by username) only when an employee row changed, and `audit` with the audit lines it added. `ip: null` means the request had no address header. `cookies` is what the request carried, `setCookies` what the answer set.',
  counts: { stories: stories.length, steps: count(() => true), accepted: count((s) => s.ok !== undefined), refused: count((s) => s.refused !== undefined) },
  stories,
}, null, 1) + '\n');
