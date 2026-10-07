// Records what the 10-06 bundle answers on `POST /api/payment-methods` (design §5.5.2, row PM-API):
// every refusal of the route in v2's order, and the answers of `list` and `save` for the owner and
// for employees of two branches.
//   node gen_payment_methods.mjs <10-06 bundle dir> > ../../tests/fixtures/payment_methods/v2_payment_methods_cases.json
//
// It needs the 10-06 bundle (`XC-Desk-v2-Passport-Drawer-20261006.zip`, sha256 6514b4c8…4375f570;
// `server.mjs` sha256 99ddcd10…8a1b0b). Its `payment-settings.mjs` is imported from the directory
// given, unchanged, and every step below is a `Request` handed to that module's own
// `api(request, actor, checkBranch)`. The bundle is not in this repository, so this cannot run in
// CI; its output is checked in and is what the tests read.
//
// What is supplied from here, and nothing else:
//   Date        → a clock this script sets (the module reads `new Date()` when it saves)
//   pool        → an in-memory store that answers the module's statements, each known by its whole
//                 shape (below); it throws on any other statement, and on every statement once the
//                 script says the database is down. BEGIN / COMMIT / ROLLBACK keep and restore a copy.
//   actor       → who v2's router found signed in (`server.mjs:8285-8288`): `null` for nobody, the
//                 owner, or an employee with their branch; the router answers nobody itself with
//                 the same 401 sentence as the module's first check, so the module is asked here.
//   checkBranch → the route's own argument (`server.mjs:8154`): `model(branch)` (`:7913`, the
//                 engine's `adminSnapshot`), then `ไม่พบสาขา` when the branch is not among the
//                 branches. For a branch the engine does not know, `adminSnapshot` throws its own
//                 sentence first (`branchBook_`, `server.mjs:6798`) — an error without a status —
//                 and that is what is thrown here too.
//
// The script fails, and writes nothing, when the module answers a sentence that is not in the list
// below, when a sentence of the list was never heard, or when a statement was never run.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const dir = process.argv[2];
if (!dir) throw Error('usage: node gen_payment_methods.mjs <10-06 bundle dir>');
const file = path.join(fs.statSync(dir).isDirectory() ? dir : path.dirname(dir), 'payment-settings.mjs');
const v2 = await import(pathToFileURL(file).href);
if (typeof v2.createPaymentSettings !== 'function') throw Error('payment-settings.mjs: createPaymentSettings is not exported');

// ── the clock ────────────────────────────────────────────────────────────────
const RealDate = Date;
let clock = RealDate.parse('2026-10-08T03:00:00.000Z');
class Clock extends RealDate { constructor(...a) { if (a.length) super(...a); else super(clock); } static now() { return clock; } }
globalThis.Date = Clock;

// ── the store ────────────────────────────────────────────────────────────────
const W = { t: { settings: [], audit: [], ledger: 0 }, saved: null, down: false, statements: new Set() };
const json = (text) => JSON.parse(text);
const STATEMENTS = {
  readOne: [/^SELECT \* FROM payment_method_settings WHERE branch_id=\$1$/, ([b]) => W.t.settings.filter((r) => r.branch_id === b).map((r) => structuredClone(r))],
  firstRow: [/^INSERT INTO payment_method_settings\(branch_id,methods\) VALUES\(\$1,\$2::jsonb\) ON CONFLICT DO NOTHING$/, ([b, methods]) => {
    if (!W.t.settings.some((r) => r.branch_id === b)) W.t.settings.push({ branch_id: b, version: 0, methods: json(methods), updated_at: null, updated_by: null });
    return [];
  }],
  lockOne: [/^SELECT \* FROM payment_method_settings WHERE branch_id=\$1 FOR UPDATE$/, ([b]) => W.t.settings.filter((r) => r.branch_id === b).map((r) => structuredClone(r))],
  update: [/^UPDATE payment_method_settings SET methods=\$2::jsonb,version=\$3,updated_at=\$4,updated_by=\$5 WHERE branch_id=\$1$/, ([b, methods, version, at, by]) => {
    for (const r of W.t.settings) if (r.branch_id === b) Object.assign(r, { methods: json(methods), version, updated_at: at, updated_by: by });
    return [];
  }],
  audit: [/^INSERT INTO payment_method_audit\(branch_id,version,actor,created_at,before_methods,after_methods\) VALUES\(\$1,\$2,\$3,\$4,\$5::jsonb,\$6::jsonb\)$/, ([b, version, actor, at, before, after]) => {
    if (W.t.audit.some((r) => r.branch_id === b && r.version === version)) throw Error('store: duplicate audit row');
    W.t.audit.push({ branch_id: b, version, actor, created_at: at, before_methods: json(before), after_methods: json(after) });
    return [];
  }],
  ledger: [/^UPDATE ledger_control SET revision=revision\+1 WHERE id=1$/, () => { W.t.ledger++; return []; }],
  history: [/^SELECT version,actor,created_at,before_methods,after_methods FROM payment_method_audit WHERE branch_id=\$1 ORDER BY version DESC LIMIT 20$/, ([b]) => W.t.audit.filter((r) => r.branch_id === b).sort((x, y) => y.version - x.version).slice(0, 20).map(({ branch_id, ...r }) => structuredClone(r))],
};
function query(sql, args = []) {
  if (W.down) throw Error('connect ECONNREFUSED 127.0.0.1:5432');
  if (sql === 'BEGIN') { if (W.saved) throw Error('store: nested transaction'); W.saved = structuredClone(W.t); return { rows: [] }; }
  if (sql === 'COMMIT') { W.saved = null; return { rows: [] }; }
  if (sql === 'ROLLBACK') { if (W.saved) W.t = W.saved; W.saved = null; return { rows: [] }; }
  const known = Object.entries(STATEMENTS).filter(([, [shape]]) => shape.test(sql));
  if (known.length !== 1) throw Error('store: unknown statement: ' + sql);
  const [name, [, run]] = known[0];
  if (args.some((x) => x === undefined)) throw Error('store: undefined parameter in: ' + sql);
  W.statements.add(name);
  const rows = run(args);
  return { rows, rowCount: rows.length };
}
const pool = { query: async (sql, args) => query(sql, args), connect: async () => ({ query: async (sql, args) => query(sql, args), release() {} }) };
const settings = v2.createPaymentSettings(pool);

// ── the shop: branches, people ───────────────────────────────────────────────
const BRANCHES = ['MAIN', 'AIRPORT'];
const ENGINE_NO_SUCH_BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
async function checkBranch(branch) {
  // model(branch) — the engine's adminSnapshot — refuses a branch it does not know before the route looks
  if (!BRANCHES.includes(branch)) throw Error(ENGINE_NO_SUCH_BRANCH);
  const m = { branches: BRANCHES.map((id) => ({ id })) };
  if (!m.branches.some((b) => b.id === branch)) throw Object.assign(Error('ไม่พบสาขา'), { status: 400 });
}
const PEOPLE = [
  { key: 'OWNER', kind: 'owner', id: 'owner', username: 'owner', role: 'OWNER', branchId: null },
  { key: 'FRONT', kind: 'staff', id: '00000000-0000-4000-8000-0000000000f1', username: 'front.main', role: 'FRONT', branchId: 'MAIN' },
  { key: 'MANAGER', kind: 'staff', id: '00000000-0000-4000-8000-0000000000f2', username: 'manager.main', role: 'MANAGER', branchId: 'MAIN' },
  { key: 'MONITOR', kind: 'staff', id: '00000000-0000-4000-8000-0000000000f3', username: 'monitor.main', role: 'MONITOR', branchId: 'MAIN' },
  { key: 'AIRPORT_FRONT', kind: 'staff', id: '00000000-0000-4000-8000-0000000000f4', username: 'front.airport', role: 'FRONT', branchId: 'AIRPORT' },
];
const actorOf = (key) => (key ? structuredClone(PEOPLE.find((p) => p.key === key)) : null);

// ── sentences the route may answer with; anything else stops the script ─────
const SENTENCES = {
  signIn: 'กรุณาเข้าสู่ระบบ', rejected: 'Request rejected.', tooLarge: 'คำขอใหญ่เกินกำหนด', invalid: 'คำขอไม่ถูกต้อง',
  badBranch: 'สาขาไม่ถูกต้อง', ownerOnly: 'เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้', noBranch: 'ไม่พบสาขา',
  notFive: 'กรุณาตั้งค่าให้ครบทั้ง 5 ช่องทาง', badChannel: 'ข้อมูลช่องทางชำระเงินไม่ถูกต้อง',
  buyOnly: 'การจ่ายเงินบาทให้ลูกค้ารองรับเงินสดและโอนธนาคาร', noDirection: 'ช่องทางที่เปิดต้องเลือกการรับเงินหรือจ่ายเงินอย่างน้อยหนึ่งประเภท',
  badText: 'ข้อความยาวเกินกำหนดหรือไม่ถูกต้อง', onePerSide: 'ต้องเหลือช่องทางรับเงินและจ่ายเงินบาทอย่างน้อยประเภทละ 1 ช่องทาง',
  badVersion: 'เวอร์ชันข้อมูลไม่ถูกต้อง', stale: 'มีผู้แก้ไขการตั้งค่านี้แล้ว กรุณาโหลดล่าสุดก่อนบันทึก',
  database: 'โหลดการตั้งค่าไม่ได้ กรุณาตรวจการเชื่อมต่อและติดตั้งตารางวิธีชำระเงิน',
};
const knownSentence = new Set(Object.values(SENTENCES));
const heard = new Set();
// Row 8 is never heard: for a branch the engine does not know, `model(branch)` throws first (see
// `checkBranch` above), so v2 answers row 17's 500. Recorded as v2 answers it; the tests decide.
const NEVER_REACHED = new Set([SENTENCES.noBranch]);

// ── one step ────────────────────────────────────────────────────────────────
const ORIGIN = 'http://desk.test';
const URL_ = ORIGIN + '/api/payment-methods';
const steps = [];
async function step(label, as, body, { headers = {}, row = null, advance = 1, down = false } = {}) {
  clock += advance * 1000;
  if (down) W.down = true;
  const raw = typeof body === 'string' ? body : body.big ? body.big.head + body.big.unit.repeat(body.big.count) + body.big.tail : JSON.stringify(body);
  const h = { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json' };
  for (const [k, v] of Object.entries(headers)) { if (v === null) delete h[k]; else h[k] = v; }
  const res = await settings.api(new Request(URL_, { method: 'POST', headers: h, body: raw }), actorOf(as), checkBranch);
  const answer = await res.json();
  if (res.headers.get('cache-control') !== 'no-store') throw Error(label + ': not no-store');
  if (answer.error !== undefined) {
    if (!knownSentence.has(answer.error)) throw Error(label + ': a sentence this script does not know: ' + answer.error);
    heard.add(answer.error);
  }
  if (row !== null && answer.error === undefined) throw Error(label + ': expected the refusal of row ' + row);
  if (row === null && answer.error !== undefined) throw Error(label + ': refused unexpectedly: ' + answer.error);
  const s = { n: steps.length + 1, label, at: new RealDate(clock).toISOString(), as };
  if (Object.keys(headers).length) s.headers = headers;
  if (typeof body === 'string') s.raw = raw; else if (body.big) s.big = { ...body.big, units: raw.length }; else s.body = body;
  if (down) s.databaseDown = true;
  if (row !== null) s.row = row;
  Object.assign(s, { status: res.status, answer });
  steps.push(s);
  return answer;
}

// rows as the admin page sends them
const rows = () => v2.defaults();
const edit = (rs, id, change) => rs.map((r) => (r.id === id ? { ...r, ...change } : r));
const save = (version, methods, extra = {}) => ({ action: 'save', payload: { version, methods, ...extra } });
const list = (payload) => (payload === undefined ? { action: 'list' } : { action: 'list', payload });
// a body of exactly `units` UTF-16 units: a list whose payload carries padding of `pad`
// (written in the fixture as `big`: head, unit, count, tail)
function padded(units, pad) {
  const head = '{"action":"list","payload":{"branchId":"MAIN","pad":"', tail = '"}}';
  const count = units - head.length - tail.length;
  if (count < 1 || pad.length !== 1) throw Error('bad padding');
  return { big: { head, unit: pad, count, tail } };
}

// ── reading, then the owner's first save ─────────────────────────────────────
await step('the owner reads MAIN: the defaults at version 0', 'OWNER', list({}));
await step('no payload at all reads MAIN', 'OWNER', list());
await step('an empty branchId reads MAIN', 'OWNER', list({ branchId: '' }));
await step('an employee reads their own branch, cannot edit, sees no history', 'FRONT', list({}));
await step('an employee naming another branch reads their own', 'AIRPORT_FRONT', list({ branchId: 'MAIN' }));
await step('an employee naming a malformed branch still reads their own', 'FRONT', list({ branchId: 'no such branch!' }));
const first = edit(edit(edit(rows(), 'Credit Card', { enabled: false }), 'Cash', { account: '  Drawer 1  ', instruction: '\tPay at the counter\n' }), 'PromptPay', { account: 'PP 000', instruction: 'Scan · สแกนจ่าย' });
await step('the owner saves MAIN from version 0: Credit Card off, texts trimmed', 'OWNER', save(0, first, { branchId: 'MAIN' }), { advance: 5 });
await step('the owner reads MAIN: version 1, who and when, one change', 'OWNER', list({ branchId: 'MAIN' }));
await step('an employee of MAIN reads the new rows without history', 'MANAGER', list({}));
await step('AIRPORT is untouched: the owner reads its defaults', 'OWNER', list({ branchId: 'AIRPORT' }));
await step('an employee of AIRPORT reads its defaults', 'AIRPORT_FRONT', list({}));
const airport = edit(edit(rows(), 'PromptPay', { enabled: false }), 'Cash', { sell: false });
await step('the owner saves AIRPORT from its own version 0', 'OWNER', save(0, airport, { branchId: 'AIRPORT' }), { advance: 5 });
await step('MAIN is untouched by the AIRPORT save', 'OWNER', list({ branchId: 'MAIN' }));
await step('an employee of AIRPORT reads the AIRPORT rows', 'AIRPORT_FRONT', list({ branchId: 'MAIN' }));
const second = edit(first, 'Bank Transfer', { instruction: '' });
await step('the owner saves MAIN again from version 1', 'OWNER', save(1, second, { branchId: 'MAIN' }), { advance: 5 });
await step('the history is newest first', 'OWNER', list({ branchId: 'MAIN' }));

// ── row 1: nobody signed in comes first ──────────────────────────────────────
await step('nobody reads', null, list({}), { row: 1 });
await step('nobody, from another origin', null, list({}), { row: 1, headers: { origin: 'http://elsewhere.test' } });
await step('nobody, with a body too large', null, padded(12001, 'x'), { row: 1 });
// ── row 2: the origin rule ───────────────────────────────────────────────────
await step('the owner without the marker header', 'OWNER', list({}), { row: 2, headers: { 'x-xchange-request': null } });
await step('the owner with the marker header not 1', 'OWNER', list({}), { row: 2, headers: { 'x-xchange-request': 'yes' } });
await step('the owner from another origin', 'OWNER', list({}), { row: 2, headers: { origin: 'http://elsewhere.test' } });
await step('the owner with no origin', 'OWNER', list({}), { row: 2, headers: { origin: null } });
await step('an employee from another origin, with a body too large', 'FRONT', padded(12001, 'x'), { row: 2, headers: { origin: 'http://elsewhere.test' } });
// ── row 3: the size of the body, in UTF-16 units ─────────────────────────────
await step('a body of 12,001 units', 'OWNER', padded(12001, 'x'), { row: 3 });
await step('a body of 12,000 units is read', 'OWNER', padded(12000, 'x'));
await step('12,000 Thai letters are 12,000 units, not bytes', 'OWNER', padded(12000, 'ก'));
await step('a body too large with no payload object', 'OWNER', { big: { head: '{"action":"list","payload":[', unit: '1,', count: 6000, tail: '1]}' } }, { row: 3 });
// ── row 4: the payload ───────────────────────────────────────────────────────
await step('a payload that is a list', 'OWNER', { action: 'list', payload: [] }, { row: 4 });
await step('a payload that is text', 'OWNER', { action: 'list', payload: 'MAIN' }, { row: 4 });
await step('a payload that is null', 'OWNER', { action: 'list', payload: null }, { row: 4 });
await step('a payload that is a number', 'OWNER', { action: 'list', payload: 5 }, { row: 4 });
await step('a payload that is a list, with an unknown action', 'OWNER', { action: 'delete', payload: [] }, { row: 4 });
await step('an employee with a payload that is a list', 'FRONT', { action: 'save', payload: [] }, { row: 4 });
// ── row 5: the branch ────────────────────────────────────────────────────────
await step('the owner names a branch with a space', 'OWNER', list({ branchId: 'no such branch' }), { row: 5 });
await step('the owner names a branch of 71 characters', 'OWNER', list({ branchId: 'A'.repeat(71) }), { row: 5 });
await step('a malformed branch comes before an unknown action', 'OWNER', { action: 'delete', payload: { branchId: 'MAIN!' } }, { row: 5 });
await step('a malformed branch comes before the owner-only rule', 'OWNER', save(1, second, { branchId: 'MAIN!' }), { row: 5 });
// ── row 6: the action ────────────────────────────────────────────────────────
await step('an unknown action', 'OWNER', { action: 'delete', payload: {} }, { row: 6 });
await step('no action', 'OWNER', { payload: {} }, { row: 6 });
await step('an action in capitals', 'OWNER', { action: 'LIST', payload: {} }, { row: 6 });
await step('an employee with an unknown action', 'FRONT', { action: 'delete', payload: {} }, { row: 6 });
// ── row 7: only the owner saves ──────────────────────────────────────────────
await step('a manager saves', 'MANAGER', save(2, second), { row: 7 });
await step('a front saves', 'FRONT', save(2, second), { row: 7 });
await step('an employee of AIRPORT saves bad rows', 'AIRPORT_FRONT', save('x', []), { row: 7 });
// ── row 8: the branch must exist ─────────────────────────────────────────────
await step('the owner reads a branch that does not exist', 'OWNER', list({ branchId: 'NOWHERE' }), { row: 8 });
await step('the owner names a branch of 70 characters that does not exist', 'OWNER', list({ branchId: 'A'.repeat(70) }), { row: 8 });
await step('the owner saves bad rows to a branch that does not exist', 'OWNER', save(0, [], { branchId: 'NOWHERE' }), { row: 8 });
// ── rows 9 to 14: the rows, channel by channel in v2's order ─────────────────
const v = 2; // MAIN's version now
await step('no methods', 'OWNER', { action: 'save', payload: { version: v } }, { row: 9 });
await step('methods that are not a list', 'OWNER', save(v, { Cash: true }), { row: 9 });
await step('four rows', 'OWNER', save(v, rows().slice(0, 4)), { row: 9 });
await step('six rows', 'OWNER', save(v, [...rows(), rows()[0]]), { row: 9 });
await step('Cash twice and no Bank Transfer', 'OWNER', save(v, [...rows().slice(0, 4), rows()[0]]), { row: 9 });
await step('a row that is null: five ids, one of them missing, so a channel is missing', 'OWNER', save(v, [...rows().slice(0, 4), null]), { row: 10 });
await step('an unknown channel instead of Alipay', 'OWNER', save(v, edit(rows(), 'Alipay', { id: 'Cheque' })), { row: 10 });
await step('SCB Transfer instead of Bank Transfer', 'OWNER', save(v, edit(rows(), 'Bank Transfer', { id: 'SCB Transfer' })), { row: 10 });
await step('enabled as text', 'OWNER', save(v, edit(rows(), 'PromptPay', { enabled: 'true' })), { row: 10 });
await step('buy as a number', 'OWNER', save(v, edit(rows(), 'Cash', { buy: 1 })), { row: 10 });
await step('sell missing', 'OWNER', save(v, rows().map((r) => (r.id === 'Alipay' ? { id: r.id, enabled: r.enabled, buy: r.buy, account: '', instruction: '' } : r))), { row: 10 });
await step('BUY on Credit Card', 'OWNER', save(v, edit(rows(), 'Credit Card', { buy: true })), { row: 11 });
await step('BUY on Alipay while Alipay is off', 'OWNER', save(v, edit(rows(), 'Alipay', { enabled: false, buy: true })), { row: 11 });
await step('PromptPay on with neither direction', 'OWNER', save(v, edit(rows(), 'PromptPay', { buy: false, sell: false })), { row: 12 });
await step('Cash on with neither direction comes before BUY on Credit Card', 'OWNER', save(v, edit(edit(rows(), 'Cash', { buy: false, sell: false }), 'Credit Card', { buy: true })), { row: 12 });
await step('BUY on Credit Card comes before Credit Card with a bad text', 'OWNER', save(v, edit(rows(), 'Credit Card', { buy: true, account: 'x'.repeat(101) })), { row: 11 });
await step('a bad text on Cash comes before BUY on Credit Card', 'OWNER', save(v, edit(edit(rows(), 'Cash', { account: 'x'.repeat(101) }), 'Credit Card', { buy: true })), { row: 13 });
await step('an account of 101 characters', 'OWNER', save(v, edit(rows(), 'Alipay', { account: 'x'.repeat(101) })), { row: 13 });
await step('an account of 99 characters and two spaces: counted before trimming', 'OWNER', save(v, edit(rows(), 'Alipay', { account: 'x'.repeat(99) + '  ' })), { row: 13 });
await step('an instruction of 401 characters', 'OWNER', save(v, edit(rows(), 'Cash', { instruction: 'ก'.repeat(401) })), { row: 13 });
await step('an account of 51 emoji: 102 units', 'OWNER', save(v, edit(rows(), 'Cash', { account: '💵'.repeat(51) })), { row: 13 });
await step('a bell in an account', 'OWNER', save(v, edit(rows(), 'Cash', { account: 'Drawer\u0007' })), { row: 13 });
await step('a unit separator in an instruction', 'OWNER', save(v, edit(rows(), 'Cash', { instruction: 'a\u001fb' })), { row: 13 });
await step('a vertical tab in an instruction', 'OWNER', save(v, edit(rows(), 'Cash', { instruction: 'a\u000bb' })), { row: 13 });
await step('an account that is a number', 'OWNER', save(v, edit(rows(), 'Cash', { account: 5 })), { row: 13 });
await step('an account that is null', 'OWNER', save(v, edit(rows(), 'Cash', { account: null })), { row: 13 });
await step('no instruction', 'OWNER', save(v, rows().map((r) => (r.id === 'Cash' ? { id: r.id, enabled: true, buy: true, sell: true, account: '' } : r))), { row: 13 });
await step('a bad account on a channel that is off', 'OWNER', save(v, edit(rows(), 'Alipay', { enabled: false, account: 'x'.repeat(101) })), { row: 13 });
await step('no channel on for BUY', 'OWNER', save(v, edit(edit(rows(), 'Cash', { buy: false }), 'Bank Transfer', { buy: false })), { row: 14 });
await step('BUY only on channels that are off', 'OWNER', save(v, edit(edit(rows(), 'Cash', { enabled: false }), 'Bank Transfer', { enabled: false })), { row: 14 });
await step('no channel on for SELL', 'OWNER', save(v, rows().map((r) => ({ ...r, enabled: ['Cash', 'Bank Transfer'].includes(r.id), sell: false }))), { row: 14 });
await step('every channel off', 'OWNER', save(v, rows().map((r) => ({ ...r, enabled: false }))), { row: 14 });
await step('bad rows come before a bad version', 'OWNER', save('two', edit(rows(), 'Credit Card', { buy: true })), { row: 11 });
await step('bad rows come before a stale version', 'OWNER', save(0, rows().slice(1)), { row: 9 });
// ── row 15: the version ──────────────────────────────────────────────────────
await step('no version', 'OWNER', { action: 'save', payload: { methods: rows() } }, { row: 15 });
await step('a version as text', 'OWNER', save(String(v), rows()), { row: 15 });
await step('a version with a fraction', 'OWNER', save(1.5, rows()), { row: 15 });
await step('a negative version', 'OWNER', save(-1, rows()), { row: 15 });
await step('a version of null', 'OWNER', save(null, rows()), { row: 15 });
await step('a version beyond a safe integer', 'OWNER', save(2 ** 53, rows()), { row: 15 });
// ── row 16: a stale version ──────────────────────────────────────────────────
await step('a save from version 1 when MAIN is at 2', 'OWNER', save(1, rows()), { row: 16 });
await step('a save from version 0 when MAIN is at 2', 'OWNER', save(0, rows()), { row: 16 });
await step('a save from version 3 when MAIN is at 2', 'OWNER', save(3, rows()), { row: 16 });
await step('a stale save of AIRPORT', 'OWNER', save(0, rows(), { branchId: 'AIRPORT' }), { row: 16 });
await step('nothing was written by the refusals', 'OWNER', list({}));
await step('a version written 2.0 is the whole number 2', 'OWNER', JSON.stringify(save(2, second)).replace('"version":2,', '"version":2.0,'), { advance: 5 });
await step('the owner reads MAIN at version 3', 'OWNER', list({}));
// ── row 17: the database ─────────────────────────────────────────────────────
await step('the database is down: an employee reads', 'FRONT', list({}), { row: 17, down: true });
await step('the database is down: the owner saves', 'OWNER', save(3, second), { row: 17 });

for (const s of Object.values(SENTENCES)) if (!heard.has(s) && !NEVER_REACHED.has(s)) throw Error('never heard: ' + s);
for (const s of NEVER_REACHED) if (heard.has(s)) throw Error('heard after all: ' + s);
for (const name of Object.keys(STATEMENTS)) if (!W.statements.has(name)) throw Error('statement never run: ' + name);

process.stdout.write(JSON.stringify({
  source: 'payment-settings.mjs of the 10-06 bundle (XC-Desk-v2-Passport-Drawer-20261006.zip, sha256 6514b4c8…4375f570) through its own api(), by gen_payment_methods.mjs',
  origin: ORIGIN,
  route: '/api/payment-methods',
  branches: BRANCHES,
  people: PEOPLE,
  steps,
}, null, 1) + '\n');
