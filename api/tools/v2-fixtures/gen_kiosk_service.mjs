// Records what the kiosk module of the v2 server stores and answers when its actions are played
// one after another — the part of the kiosk that needs a database (ticket T12b).
//   node gen_kiosk_service.mjs path/to/server.mjs path/to/10-06-bundle-dir > ../../crates/xc-kiosk/tests/fixtures/v2_kiosk_service_cases.json
//
// `gen_kiosk.mjs` (T12a) recorded the rules one call at a time; this script records stories: who
// sees which request from which branch, which quotes are swept away and when, what the queue
// lists, how long a receipt can be read, which currencies the kiosk offers, and every audit line.
//
// It needs the deployed bundle (`server.mjs`, `xchange-online@3ffc5a5`), which is not in this
// repository; the path is the first argument. From it, one piece is evaluated unchanged: the text
// from the line `// src/currencies.mjs` up to, not including, `// src/staff-routes.mjs` (the
// currency list and the kiosk module: `action2`, `beforeSave`, `afterSave`).
//
// The module reads the wall clock, draws ids, talks to PostgreSQL, asks the engine for the branch
// and writes audit lines. Those five names are supplied from here and nothing inside is replaced:
//   Date         → a clock this script sets
//   randomUUID9  → a counter
//   pool         → an in-memory store that interprets the module's own SQL texts; it is not
//                  PostgreSQL, and it throws on any statement shape it does not know
//   execute      → `adminSnapshot` answers the branch this script describes (and v2's own sentence
//                  for a branch that does not exist, which #54's fixture recorded from the engine);
//                  `adminSaveRate` stores the rate in that description, or refuses when told to
//   logEvent     → collects the audit lines
//
// The role table is not written here any more (row POL, design `docs/design/xchange-v3.md` §5.5.7):
// `Policy` is its one home and `gen_policy.mjs` records it (path `Kiosk`). The service no longer asks
// who may call; the machine still refuses a caller without a name a step that needs a holder. So every
// step is still played against v2 (the store moves as v2 moved it), and the output leaves out the steps
// v2 refused with one of its two access sentences either to an employee outside the desk (not OWNER,
// FRONT, BACKEND, MANAGER) or for `list`, `notifications`, `manage`, `configure`, whose only check of
// the caller was the role table.
//
// The script stops, and writes nothing, when v2 says a sentence it was not expected to say, when a
// refused call wrote something, or when a figure v2 computed is not the exact decimal.
//
// Row Q1 (design §5.5.3, ticket #328): `accept` and the queue's page size are recorded from the
// 10-06 bundle (`XC-Desk-v2-Passport-Drawer-20261006.zip`, design §5.5), whose kiosk module hands
// `accept` and `list` to its `queue-work.mjs`. The directory of that bundle is the second argument:
//   node gen_kiosk_service.mjs path/to/10-05/server.mjs path/to/10-06-bundle-dir > ...
// From it, the same slice of its `server.mjs` is evaluated unchanged, with `queueWork` made by its own
// `queue-work.mjs` over the same store (`paymentSettings` and `paymentAllowed` are supplied as names
// that stop the script if reached: no step here goes through them). Every `accept` step is played on
// the 10-06 module; everything else on the 10-05 module as before. Every `list` step is still recorded
// from the 10-05 module and is played on the 10-06 one as well: the script stops if the two differ, so
// the recorded queues are what both bundles list (page 0, all four statuses).
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

const file = process.argv[2], newDir = process.argv[3];
if (!file || !newDir) throw Error('usage: node gen_kiosk_service.mjs path/to/server.mjs path/to/10-06-bundle-dir');
const bundle = fs.readFileSync(file, 'utf8');
const from = bundle.indexOf('// src/currencies.mjs'), to = bundle.indexOf('// src/staff-routes.mjs');
if (from < 0 || to < from) throw Error('the kiosk module was not found in ' + file);
const src = bundle.slice(from, to);
for (const needle of ['async function action2(', 'async function beforeSave(', 'async function afterSave(', 'var currencies_default = [']) if (!src.includes(needle)) throw Error('kiosk module: ' + needle + ' not found');
if (/^import /m.test(src)) throw Error('the kiosk module must not contain an import line');
if (/new Date|setTimeout|Math\.random|fetch\(/.test(src)) throw Error('the kiosk module reads something this script does not replace');

// ── the 10-06 bundle, told apart by its own hashes (design §5.5) ────────────
const sha256 = (f) => createHash('sha256').update(fs.readFileSync(f)).digest('hex');
const NEW_SERVER = path.join(newDir, 'server.mjs'), NEW_QUEUE_WORK = path.join(newDir, 'queue-work.mjs');
if (sha256(NEW_SERVER) !== '99ddcd107bd4107226ce3065efcefea11a8b2a58d86a45b9a5872e571a8b1b0b') throw Error(NEW_SERVER + ' is not the server.mjs of the 10-06 bundle');
if (sha256(NEW_QUEUE_WORK) !== 'e03bc3499d599cf397d61a4d9138c7ae99269cf390ee076d60d10d3c348bfb30') throw Error(NEW_QUEUE_WORK + ' is not the queue-work.mjs of the 10-06 bundle');
const bundle6 = fs.readFileSync(NEW_SERVER, 'utf8');
const from6 = bundle6.indexOf('// src/currencies.mjs'), to6 = bundle6.indexOf('// src/staff-routes.mjs');
if (from6 < 0 || to6 < from6) throw Error('the kiosk module was not found in ' + NEW_SERVER);
const src6 = bundle6.slice(from6, to6);
for (const needle of ['async function action2(', 'queueWork.accept(', 'queueWork.list(']) if (!src6.includes(needle)) throw Error('10-06 kiosk module: ' + needle + ' not found');
if (/^import /m.test(src6)) throw Error('the 10-06 kiosk module must not contain an import line');
// its one `new Date()` is the `serverNow` of `notifications`, which is never played on it: `Date` is
// supplied without a constructor, so reaching it stops the script
if (/setTimeout|Math\.random|fetch\(/.test(src6)) throw Error('the 10-06 kiosk module reads something this script does not replace');
const { createQueueWork } = await import(pathToFileURL(NEW_QUEUE_WORK).href);

// ── the store ────────────────────────────────────────────────────────────────
const COLS = {
  // the last two are the 10-06 bundle's (`queue-work.sql`); the 10-05 module inserts without them
  kiosk_requests: ['id', 'branch_id', 'data', 'status', 'created', 'staff', 'started_at', 'completed_at'],
  kiosk_currencies: ['branch_id', 'code', 'enabled'],
  kiosk_transactions: ['request_id', 'submission_id', 'transaction_ref'],
  kiosk_receipts: ['id', 'image', 'updated'],
};
const KEY = { kiosk_requests: ['id'], kiosk_currencies: ['branch_id', 'code'], kiosk_transactions: ['request_id'], kiosk_receipts: ['id'] };
const NUMERIC = new Set(['created', 'updated']);
class Store {
  constructor() { this.t = Object.fromEntries(Object.keys(COLS).map((k) => [k, []])); }
  query(sql, args = []) {
    const bad = () => { throw Error('store: unknown statement: ' + sql); };
    const term = (x) => /^\$\d+$/.test(x) ? args[Number(x.slice(1)) - 1] : /^'[^']*'$/.test(x) ? x.slice(1, -1) : bad();
    const table = (name) => this.t[name] ?? bad();
    const col = (name, c) => COLS[name].includes(c) ? c : bad();
    const where = (name, text) => {
      const tests = text.split(' AND ').map((c) => {
        let m;
        if ((m = c.match(/^(\w+) IN \(([^)]+)\)$/))) { const k = col(name, m[1]), list = m[2].split(',').map(term); return (r) => list.includes(r[k]); }
        if ((m = c.match(/^(\w+)(=|<|>)(\$\d+|'[^']*')$/))) {
          const k = col(name, m[1]), v = term(m[3]);
          if (v == null) return () => false; // a comparison with NULL is never true
          if (m[2] === '=') return NUMERIC.has(k) ? bad() : (r) => r[k] === v;
          if (!NUMERIC.has(k) || typeof v !== 'number') bad();
          return m[2] === '<' ? (r) => r[k] < v : (r) => r[k] > v;
        }
        return bad();
      });
      return (r) => tests.every((f) => f(r));
    };
    let m;
    // the 10-06 queue (`queue-work.mjs` `list`): one page of a branch's requests of some statuses,
    // newest first then id descending, with the document number of the saved transaction
    if ((m = sql.match(/^SELECT r\.\*,t\.transaction_ref FROM kiosk_requests r LEFT JOIN kiosk_transactions t ON t\.request_id=r\.id WHERE r\.branch_id=\$1 AND r\.status=ANY\(\$2::text\[\]\) ORDER BY r\.created DESC,r\.id DESC LIMIT (\d+) OFFSET \$3$/))) {
      const [branch, statuses, offset] = args;
      if (!Array.isArray(statuses) || !Number.isSafeInteger(offset)) bad();
      const rows = table('kiosk_requests').filter((r) => r.branch_id === branch && statuses.includes(r.status));
      // the tie on `created` is broken by id under PostgreSQL's collation, which this store does not have
      if (new Set(rows.map((r) => r.created)).size !== rows.length) throw Error('store: two requests of one queue share a quote time');
      rows.sort((a, b) => b.created - a.created);
      const page = rows.slice(offset, offset + Number(m[1]));
      return { rows: page.map((r) => ({ ...Object.fromEntries(COLS.kiosk_requests.map((c) => [c, NUMERIC.has(c) ? String(r[c]) : r[c]])), transaction_ref: this.t.kiosk_transactions.find((x) => x.request_id === r.id)?.transaction_ref ?? null })), rowCount: page.length };
    }
    if ((m = sql.match(/^SELECT (\*|[\w,]+) FROM (\w+) WHERE (.+?)( ORDER BY created DESC)?( LIMIT (\d+))?$/))) {
      let rows = table(m[2]).filter(where(m[2], m[3]));
      if (m[4]) rows = rows.slice().sort((a, b) => b.created - a.created);
      if (m[5]) rows = rows.slice(0, Number(m[6]));
      const cols = m[1] === '*' ? COLS[m[2]] : m[1].split(',').map((c) => col(m[2], c));
      // PostgreSQL hands a bigint back as text; v2 reads it with Number()
      return { rows: rows.map((r) => Object.fromEntries(cols.map((c) => [c, NUMERIC.has(c) ? String(r[c]) : r[c]]))), rowCount: rows.length };
    }
    if ((m = sql.match(/^DELETE FROM (\w+) WHERE (.+)$/))) {
      const f = where(m[1], m[2]), before = table(m[1]).length;
      this.t[m[1]] = table(m[1]).filter((r) => !f(r));
      return { rows: [], rowCount: before - this.t[m[1]].length };
    }
    if ((m = sql.match(/^INSERT INTO (\w+) VALUES\(([^)]+)\)(?: ON CONFLICT(?: \(([\w,]+)\))? DO (NOTHING|UPDATE SET (.+)))?$/))) {
      const name = m[1], values = m[2].split(',').map(term), cols = COLS[name] ?? bad();
      if (values.length > cols.length || values.some((v) => v === undefined)) bad();
      // PostgreSQL gives the columns a VALUES list leaves out their default, NULL here: so the 10-05
      // insert meets the 10-06 table
      const row = Object.fromEntries(cols.map((c, i) => [c, i < values.length ? values[i] : null]));
      if (m[3] && m[3] !== KEY[name].join(',')) bad();
      const old = table(name).find((r) => KEY[name].every((k) => r[k] === row[k]));
      if (!old) { table(name).push(row); return { rows: [], rowCount: 1 }; }
      if (!m[4]) throw Error('store: duplicate key in ' + name);
      if (m[4] === 'NOTHING') return { rows: [], rowCount: 0 };
      for (const set of m[5].split(',')) { const s = set.match(/^(\w+)=excluded\.(\w+)$/); if (!s || s[1] !== s[2]) bad(); old[col(name, s[1])] = row[s[1]]; }
      return { rows: [], rowCount: 1 };
    }
    if ((m = sql.match(/^UPDATE (\w+) SET (.+?) WHERE (.+?)(?: RETURNING (\*|\w+))?$/))) {
      // `clock_timestamp()` is the clock this script sets; node-postgres hands a timestamptz back as a Date
      const name = m[1], sets = m[2].split(',').map((s) => { const x = s.match(/^(\w+)=(\$\d+|'[^']*'|clock_timestamp\(\))$/); return x ? [col(name, x[1]), x[2] === 'clock_timestamp()' ? new Date(W.clock) : term(x[2])] : bad(); });
      const hit = table(name).filter(where(name, m[3]));
      for (const r of hit) for (const [k, v] of sets) r[k] = v;
      const cols = !m[4] ? [] : m[4] === '*' ? COLS[name] : [col(name, m[4])];
      return { rows: m[4] ? hit.map((r) => Object.fromEntries(cols.map((c) => [c, NUMERIC.has(c) ? String(r[c]) : r[c]]))) : [], rowCount: hit.length };
    }
    return bad();
  }
}

// ── v2 with its five outside names supplied ─────────────────────────────────
const NO_SUCH_BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
const W = { store: new Store(), clock: 0, ids: 0, events: [], branches: {}, rateSaves: [], rateSaveFails: null };
// ids look like v2's (lower-case hex) and differ in their first eight characters, which is what the queue number shows
const idAt = (n) => ((0xab54c0de + n * 0x01010101) >>> 0).toString(16).padStart(8, '0') + '-7e57-4000-8000-' + n.toString(16).padStart(12, '0');
const modelOf = (branchId) => {
  const b = W.branches[branchId || 'MAIN'];
  if (!b) throw Error(NO_SUCH_BRANCH);
  return {
    branches: Object.entries(W.branches).map(([id, x]) => ({ id, name: x.name })),
    stock: { shifts: b.open ? [{ id: 'S1', status: 'OPEN' }] : [{ id: 'S0', status: 'CLOSED' }] },
    rates: structuredClone(b.rates),
  };
};
const POOL = { query: async (sql, args) => W.store.query(sql, args) };
const OUTSIDE = [
  POOL,
  async (name, p, actor) => {
    if (name === 'adminSnapshot') return modelOf(p.branchId);
    if (name === 'adminSaveRate') {
      W.rateSaves.push({ branch: p.branchId, code: p.code, buy: p.buy, sell: p.sell, previousUpdated: p.previousUpdated, partnerName: p.partnerName, partnerCost: p.partnerCost, requestId: p.requestId, actor: actor.email });
      if (W.rateSaveFails) throw Error(W.rateSaveFails);
      const b = W.branches[p.branchId], row = b.rates.find((r) => r.code === p.code);
      if (row) { row.buy = p.buy; row.sell = p.sell; } else b.rates.push({ code: p.code, buy: p.buy, sell: p.sell });
      return { saved: true };
    }
    throw Error('fixture: unexpected engine call ' + name);
  },
  async (branch, actor, type, subject, details) => { W.events.push({ branch, actor, type, subject, details }); },
  () => idAt(++W.ids),
  { now: () => W.clock },
  { env: {} },
];
const v2 = new Function('pool', 'execute', 'logEvent', 'randomUUID9', 'Date', 'process', src + '\nreturn { action2, beforeSave, afterSave, currencies_default };')(...OUTSIDE);
// the 10-06 module, over the same store and with the same five names, plus its three new ones
const notPlayed = (name) => { throw Error('fixture: the 10-06 ' + name + ' is not played by this script'); };
const v2q = new Function('pool', 'execute', 'logEvent', 'randomUUID9', 'Date', 'process', 'queueWork', 'paymentSettings', 'paymentAllowed', src6 + '\nreturn { action2 };')(
  ...OUTSIDE,
  createQueueWork(POOL),
  new Proxy({}, { get: () => notPlayed('paymentSettings') }),
  () => notPlayed('paymentAllowed'),
);

// ── figures: the fixture writes money and rates as text, v2 holds doubles ───
const minor = (text) => { const [a, b = ''] = String(text).split('.'); return BigInt(a + (b + '00').slice(0, 2)); };
const tenK = (text) => { const [a, b = ''] = String(text).split('.'); return BigInt(a + (b + '0000').slice(0, 4)); };
const money = (m) => { const s = m.toString().padStart(3, '0'); return s.slice(0, -2) + '.' + s.slice(-2); };
const rateText = (n) => { const s = Number(n).toFixed(4); if (Math.abs(Number(s) - Number(n)) > 1e-12) throw Error('a rate with more than four decimals: ' + n); return s; };
const cents = (n) => { const c = Math.round(Number(n) * 100); if (Math.abs(Number(n) * 100 - c) > 1e-6) throw Error('not whole hundredths: ' + n); return BigInt(c); };

// ── people ──────────────────────────────────────────────────────────────────
// A staff member works in one branch; v2 puts every call of theirs on it. The fixture says which.
const staff = (id, username, role, branchId) => ({ kind: 'staff', id, email: username, username, name: username, role, employeeId: id, branchId });
const PEOPLE = {
  customer: null,
  owner: { kind: 'owner', id: 'owner', email: 'owner', username: 'owner', name: 'owner', role: 'OWNER' },
  F1: staff('E1', 'front1', 'FRONT', 'MAIN'), F2: staff('E2', 'front2', 'FRONT', 'MAIN'),
  BE1: staff('E3', 'backend1', 'BACKEND', 'MAIN'), MGR1: staff('E4', 'manager1', 'MANAGER', 'MAIN'),
  RT1: staff('E5', 'rates1', 'RATES', 'MAIN'), FIN1: staff('E6', 'finance1', 'FINANCE', 'MAIN'),
  G1: staff('E8', 'front9', 'FRONT', 'B2'), GR1: staff('E9', 'rates9', 'RATES', 'B2'),
  // the same user name in the other branch: only a name, never the same person
  G1same: staff('E10', 'front1', 'FRONT', 'B2'),
};
const people = Object.fromEntries(Object.entries(PEOPLE).map(([k, u]) => [k, u && { username: u.username, role: u.role, branch: u.branchId ?? null }]));

// ── the branches as v2 sees them ────────────────────────────────────────────
const T0 = Date.parse('2026-10-06T05:00:00.000Z');
const RATES = () => [
  { code: 'USD', buy: 32.1, sell: 32.6 }, { code: 'EUR', buy: 37.5, sell: 38.2 }, { code: 'JPY', buy: 0.211, sell: 0.2195 },
  { code: 'CHF', buy: 36.4, sell: 37.1 },   // has a rate, is not among the kiosk's currencies until switched on
  { code: 'AUD', buy: 21.3333, sell: 21.9 },
];
function fresh() {
  W.store = new Store(); W.clock = T0; W.ids = 0; W.events = []; W.rateSaves = []; W.rateSaveFails = null;
  W.branches = { MAIN: { name: 'XC-B', open: true, rates: RATES() }, B2: { name: 'Second', open: true, rates: [{ code: 'USD', buy: 31.9, sell: 32.8 }] } };
}

const SENTENCES = [
  'Counter is closed. Please contact our staff.', 'Select an exchange direction.', 'Please enter a valid amount and currency.', 'Amount is too small.',
  'Select cash or bank transfer to receive THB.', 'Invalid payment method.', 'Request not found. Please start again.', 'Quote expired. Please check the rate again.',
  'Please review your quote for 3 seconds.', 'Rate changed or counter closed. Please start again.', 'Request not found.', 'Staff access required.',
  'Owner or Rates access required.', 'The assigned employee must confirm the passport check.', 'Assigned staff required.',
  'Accept this request using your own account first.', 'This request is already assigned or unavailable.',
  'คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว', 'คิวต้องบันทึกเป็นรายการจริง', 'สกุลเงินและจำนวนต้องตรงกับคิว', 'ช่องทางต้องตรงกับที่ลูกค้าเลือก',
  'Invalid receipt image.', 'Invalid currency settings.', 'Set valid counter buy and sell rates first.',
  NO_SUCH_BRANCH, 'เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน',
  // the 10-06 `accept` (queue-work.mjs): nothing to claim; and, below, the request has a holder
  'คิวนี้ไม่พร้อมรับเรื่อง กรุณาโหลดล่าสุด',
];
const HELD_BY = /^ออเดอร์นี้มี (.+) กำลังทำรายการแล้ว$/;
const seen = new Set();
/** A timestamptz as the fixture writes it: milliseconds after `t0`. */
const msOf = (d) => { if (d == null) return null; if (!(d instanceof Date)) throw Error('v2 answered a time that is not a timestamptz: ' + d); return d.getTime() - T0; };

// ── what is stored, written the way the Rust side holds it ──────────────────
const quoteOf = (d) => ({ type: d.type, currency: d.currency, rate: rateText(d.rate), foreign: money(cents(d.foreign)), thb: money(cents(d.thb)), inputAmount: money(cents(d.inputAmount)), paymentMethod: d.paymentMethod });
function rows() {
  return W.store.t.kiosk_requests.map((r) => {
    if (!['QUOTED', 'WAITING', 'ACCEPTED', 'ID_CHECKED', 'COMPLETED'].includes(r.status)) throw Error('v2 stored status ' + r.status);
    const link = W.store.t.kiosk_transactions.find((x) => x.request_id === r.id);
    return { branch: r.branch_id, id: r.id, status: r.status, staff: r.staff === '' ? null : r.staff, documentRef: link?.transaction_ref ?? null, submissionId: link?.submission_id ?? null };
  }).sort((a, b) => (a.branch + a.id < b.branch + b.id ? -1 : 1));
}
const switches = () => W.store.t.kiosk_currencies.map((r) => ({ branch: r.branch_id, code: r.code, enabled: r.enabled })).sort((a, b) => (a.branch + a.code < b.branch + b.code ? -1 : 1));
const identity = (o) => {
  const out = { id: o.id, queue: o.queue, orderNo: o.orderNo, transactionNo: o.transactionNo, status: o.status };
  if (Object.values(out).some((v) => v === undefined)) throw Error('v2 identity lacks a field: ' + JSON.stringify(o));
  return out;
};
/** `{png: n}` is a PNG data URL of exactly n characters; `{text}` is sent as it is. */
const PREFIX = 'data:image/png;base64,';
const image = (spec) => spec.text !== undefined ? spec.text : PREFIX + 'A'.repeat(spec.png - PREFIX.length);
const eventOut = (e) => {
  const out = { branch: e.branch, actor: e.actor, kind: e.type, subject: e.subject };
  if (e.type === 'KIOSK_REQUEST') {
    if (e.details.financialTransaction !== false) throw Error('v2 audit line of a request lacks financialTransaction:false');
    out.quote = quoteOf(e.details);
  } else if (e.type === 'KIOSK_ACCEPTED') {
    // the 10-06 claim copies its start into the audit line; v3 keeps the start once, on the request
    // (design §5.5.3), so the line is written without it
    const { startedAt, ...rest } = e.details;
    if (Object.keys(rest).length || !(startedAt instanceof Date)) throw Error('v2 audit line of a claim carries other details: ' + JSON.stringify(e));
  } else if (e.type === 'KIOSK_COMPLETED') out.documentRef = e.details.transactionNo;
  else if (e.type === 'KIOSK_CURRENCY_CONFIGURED') Object.assign(out, { enabled: e.details.enabled, buy: String(e.details.buy), sell: String(e.details.sell) });
  else if (Object.keys(e.details).length) throw Error('v2 audit line carries details this script does not know: ' + JSON.stringify(e));
  return out;
};

/** Plays one step against v2 and returns its record. `step.v3` holds what v3 answers where it differs on purpose. */
async function play(step) {
  const { at = 0, who = 'customer', op, branch = 'MAIN', v3, note, ...args } = step;
  const user = PEOPLE[who]; if (user === undefined) throw Error('fixture: nobody called ' + who);
  if (user?.kind === 'staff' && user.branchId !== branch) throw Error(`fixture: ${who} works in ${user.branchId}, the step says ${branch}`);
  W.clock = T0 + at;
  const rec = { at, who, op, branch, ...args };
  if (note) rec.note = note;
  if (op === 'counter') { // not a call: the counter of a branch changes behind the kiosk's back
    const b = W.branches[branch];
    if (args.open !== undefined) b.open = args.open;
    for (const r of args.rates ?? []) { const row = b.rates.find((x) => x.code === r.code); if (row) Object.assign(row, { buy: Number(r.buy), sell: Number(r.sell) }); else b.rates.push({ code: r.code, buy: Number(r.buy), sell: Number(r.sell) }); }
    return rec;
  }
  const before = JSON.stringify(W.store.t), eventsBefore = W.events.length, savesBefore = W.rateSaves.length;
  let out, error = null;
  try {
    const p = { branchId: branch };
    if (op === 'quote') {
      for (const k of ['type', 'currency', 'amount', 'paymentMethod']) if (args[k] != null) p[k] = args[k];
      rec.newId = idAt(W.ids + 1);
      out = await v2.action2('quote', p, user);
    } else if (op === 'checkSave' || op === 'complete') {
      const q = { kioskRequestId: args.id, submissionId: args.submissionId, mode: args.live === false ? 'DEMO' : 'LIVE', currency: args.currency, type: args.type, amount: Number(args.amount), method: args.method };
      if (op === 'checkSave') await v2.beforeSave(q, user, branch);
      else await v2.afterSave(q, { saved: true, documentRef: args.documentRef }, user, branch);
    } else if (op === 'configure') {
      W.rateSaveFails = args.rateSaveFails ?? null;
      Object.assign(p, { code: args.code, enabled: args.enabled, buy: Number(args.buy), sell: Number(args.sell) });
      for (const k of ['previousUpdated', 'partnerName', 'partnerCost']) if (args[k] != null) p[k] = args[k];
      try { out = await v2.action2('configure', p, user); } finally { W.rateSaveFails = null; }
    } else {
      if (args.id !== undefined) p.id = args.id;
      if (op === 'identityChecked') p.confirmed = args.confirmed;
      if (op === 'publishReceipt') p.image = image(args.image);
      out = await (op === 'accept' ? v2q : v2).action2(op, p, user);
    }
  } catch (e) {
    const held = e.message.match(HELD_BY);
    if (!SENTENCES.includes(e.message) && !held) throw Error(`v2 crashed on ${JSON.stringify(step)}: ${e.message}`);
    // the name in the sentence is the holder's, as stored
    if (held && W.store.t.kiosk_requests.find((r) => r.id === args.id && r.branch_id === branch)?.staff !== held[1]) throw Error('v2 named someone who does not hold the request: ' + e.message);
    error = e.message; seen.add(error);
  }
  const events = W.events.slice(eventsBefore);
  if (error) {
    rec.error = error;
    if (events.length || before !== JSON.stringify(W.store.t)) throw Error(`v2 refused and still wrote: ${JSON.stringify(step)}`);
  } else if (op === 'rates') {
    rec.answer = { branchId: out.branchId, branchName: out.branchName, open: out.open, rates: out.rates.map((r) => ({ code: r.code, buy: rateText(r.buy), sell: rateText(r.sell) })) };
  } else if (op === 'quote') {
    const row = W.store.t.kiosk_requests.find((r) => r.id === out.id), d = JSON.parse(row.data), q = quoteOf(d);
    const a = minor(args.amount), r = tenK(q.rate), foreign = args.type === 'BUY' ? a : a * 10000n / r, thb = (foreign * r + 5000n) / 10000n;
    if (q.foreign !== money(foreign) || q.thb !== money(thb)) throw Error('fixture: v2 and exact arithmetic differ on ' + JSON.stringify(step) + '; pick another amount (gen_kiosk.mjs records those cases)');
    if (out.id !== rec.newId || row.status !== 'QUOTED' || row.created !== W.clock || row.staff !== '' || row.branch_id !== branch) throw Error('v2 stored an unexpected quote');
    rec.answer = { ...identity(out), quote: q, created: at };
  } else if (['confirm', 'status', 'identityChecked'].includes(op)) rec.answer = identity(out);
  else if (op === 'customerReceipt') rec.answer = { ...identity(out), image: out.image === null ? null : { length: out.image.length } };
  else if (op === 'notifications') rec.answer = { branchName: out.branchName, count: out.count, latest: out.latest };
  else if (op === 'list') {
    const listed = (requests) => requests.map((r) => {
      if (r.branch !== branch || r.data !== undefined) throw Error('v2 listed something unexpected');
      return { ...identity(r), staff: r.staff === '' ? null : r.staff, created: Number(r.created) - T0, quote: quoteOf(r) };
    });
    rec.answer = listed(out.requests);
    // the same step on the 10-06 queue (it writes nothing): its first page of all four statuses
    const again = await v2q.action2('list', { branchId: branch }, user);
    if (again.page !== 0 || again.hasMore || JSON.stringify(listed(again.requests)) !== JSON.stringify(rec.answer)) throw Error('the 10-06 queue lists this step otherwise: ' + JSON.stringify(step));
  } else if (op === 'accept') {
    if (out.saved !== true) throw Error('v2 did not say saved');
    // what `queue-work.mjs` answers a claim (`claimed: true`) and the holder again (no `claimed`)
    rec.answer = { claimed: out.claimed === true, startedAt: msOf(out.startedAt), completedAt: msOf(out.completedAt), durationSeconds: out.durationSeconds };
  } else if (op === 'get') {
    if (out.id !== args.id || out.queue !== 'K-' + args.id.slice(0, 8).toUpperCase()) throw Error('v2 get answered another request');
    rec.answer = { id: out.id, queue: out.queue, quote: quoteOf(out) };
  } else if (op === 'manage') {
    if (out.currencies.map((c) => c.code).join() !== v2.currencies_default.map((c) => c.code).join()) throw Error('v2 manage lists other currencies than its list');
    rec.answer = { enabled: out.currencies.filter((c) => c.enabled).map((c) => c.code) };
  } else if (['publishReceipt', 'configure'].includes(op)) { if (out.saved !== true) throw Error('v2 did not say saved'); }
  else if (op !== 'checkSave' && op !== 'complete') throw Error('fixture: unknown op ' + op);
  if (op === 'configure') rec.rateSaves = W.rateSaves.slice(savesBefore).map((s) => ({ branch: s.branch, code: s.code, buy: String(s.buy), sell: String(s.sell), previousUpdated: s.previousUpdated, partnerName: s.partnerName, partnerCost: s.partnerCost == null ? null : String(s.partnerCost), actor: s.actor }));
  rec.events = events.map(eventOut);
  rec.rows = rows();
  rec.switches = switches();
  if (v3) rec.v3 = v3;
  return rec;
}

const scenarios = [];
async function scenario(name, steps) {
  fresh();
  const out = [], labels = {};
  for (const { as, ...s } of steps) {
    // `as: 'x'` names the id a quote draws; `id: '@x'` uses it (configure draws ids too, so they cannot be counted)
    if (typeof s.id === 'string' && s.id.startsWith('@')) { s.id = labels[s.id.slice(1)]; if (!s.id) throw Error('fixture: unknown label in ' + name); }
    const rec = await play(s);
    if (as) { if (rec.error) throw Error('fixture: the labelled quote was refused'); labels[as] = rec.newId; }
    out.push(rec);
  }
  scenarios.push({ name, steps: out });
}
const ID = idAt; // the n-th id drawn in a scenario
const BUY100 = { op: 'quote', type: 'BUY', currency: 'USD', amount: '100' };
const SAVE = (id, over = {}) => ({ id, submissionId: 'SUB-0000000000000001', type: 'BUY', currency: 'USD', amount: '100.00', method: 'Cash', ...over });
const PNG = { png: 64 };
const MIN = 60_000;

// ═══ 1. one request from the quote to the saved transaction ═════════════════
await scenario('a request goes from quote to completed', [
  { op: 'rates' },
  { op: 'notifications', who: 'F1' },
  { op: 'list', who: 'F1' },
  { at: 1000, op: 'quote', type: 'BUY', currency: 'USD', amount: '250.50', paymentMethod: 'Bank Transfer' },
  { at: 1500, op: 'status', id: ID(1) },
  { at: 1500, op: 'list', who: 'F1', note: 'a quote is not in the queue' },
  { at: 1500, op: 'notifications', who: 'F1' },
  { at: 2000, op: 'customerReceipt', id: ID(1) },
  { at: 3999, op: 'confirm', id: ID(1), note: '2,999 ms after the quote' },
  { at: 4000, op: 'confirm', id: ID(1) },
  { at: 4100, op: 'confirm', id: ID(1), note: 'again: answered, nothing written' },
  { at: 5000, op: 'notifications', who: 'F1' },
  { at: 5000, op: 'notifications', who: 'RT1', note: 'any signed-in role' },
  { at: 5000, op: 'list', who: 'F1' },
  { at: 5000, op: 'get', who: 'F1', id: ID(1), note: 'not accepted yet' },
  { at: 6000, op: 'accept', who: 'F1', id: ID(1) },
  { at: 6100, op: 'accept', who: 'F1', id: ID(1), note: 'twice' },
  { at: 6200, op: 'accept', who: 'F2', id: ID(1) },
  { at: 6300, op: 'get', who: 'F1', id: ID(1) },
  { at: 6300, op: 'get', who: 'F2', id: ID(1) },
  { at: 6400, op: 'notifications', who: 'F1' },
  { at: 6500, op: 'publishReceipt', who: 'F1', id: ID(1), image: PNG, note: 'before the passport check' },
  { at: 7000, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: false },
  { at: 7000, op: 'identityChecked', who: 'F2', id: ID(1), confirmed: true },
  { at: 7100, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: true },
  { at: 7200, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: true, note: 'again: answered, nothing written' },
  { at: 7300, op: 'get', who: 'F1', id: ID(1) },
  { at: 8000, op: 'checkSave', who: 'F1', ...SAVE(ID(1), { amount: '250.50', method: 'Bank Transfer' }) },
  { at: 8000, op: 'checkSave', who: 'F2', ...SAVE(ID(1), { amount: '250.50', method: 'Bank Transfer' }) },
  { at: 8100, op: 'complete', who: 'F1', ...SAVE(ID(1), { amount: '250.50', method: 'Bank Transfer' }), documentRef: 'XC-20261006-000001' },
  { at: 8200, op: 'complete', who: 'F1', ...SAVE(ID(1), { amount: '250.50', method: 'Bank Transfer', submissionId: 'SUB-0000000000000002' }), documentRef: 'XC-20261006-000002', note: 'again: the first link stays, nothing written' },
  { at: 8300, op: 'status', id: ID(1) },
  { at: 8300, op: 'get', who: 'F1', id: ID(1), note: 'a completed request is no longer opened' },
  { at: 8300, op: 'checkSave', who: 'F1', ...SAVE(ID(1), { amount: '250.50', method: 'Bank Transfer' }), note: 'the same save again' },
  { at: 8300, op: 'checkSave', who: 'F1', ...SAVE(ID(1), { amount: '250.50', method: 'Bank Transfer', submissionId: 'SUB-0000000000000002' }) },
  { at: 8400, op: 'publishReceipt', who: 'F1', id: ID(1), image: PNG },
  { at: 8500, op: 'customerReceipt', id: ID(1) },
  { at: 8600, op: 'list', who: 'MGR1' },
  { at: 8600, op: 'notifications', who: 'F1' },
]);

// ═══ 2. a request belongs to its branch ═════════════════════════════════════
await scenario('a request is only found in its own branch', [
  { at: 0, ...BUY100 },
  { at: 0, ...BUY100, branch: 'B2' },
  { at: 4000, op: 'status', id: ID(1), branch: 'B2' },
  { at: 4000, op: 'customerReceipt', id: ID(1), branch: 'B2' },
  { at: 4000, op: 'confirm', id: ID(1), branch: 'B2' },
  { at: 4000, op: 'confirm', id: ID(1), who: 'owner', branch: 'B2', note: 'the owner works on the branch named' },
  { at: 4000, op: 'confirm', id: ID(1) },
  { at: 4000, op: 'confirm', id: ID(2), branch: 'B2' },
  { at: 5000, op: 'notifications', who: 'G1', branch: 'B2' },
  { at: 5000, op: 'list', who: 'G1', branch: 'B2' },
  { at: 5000, op: 'list', who: 'owner', branch: 'MAIN' },
  { at: 5000, op: 'accept', who: 'G1', id: ID(1), branch: 'B2' },
  { at: 5000, op: 'accept', who: 'G1same', id: ID(1), branch: 'B2' },
  { at: 5100, op: 'accept', who: 'F1', id: ID(1) },
  { at: 5200, op: 'get', who: 'G1same', id: ID(1), branch: 'B2', note: 'the same user name in the other branch' },
  { at: 5200, op: 'identityChecked', who: 'G1same', id: ID(1), branch: 'B2', confirmed: true },
  { at: 5300, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: true },
  { at: 5400, op: 'publishReceipt', who: 'G1same', id: ID(1), branch: 'B2', image: PNG },
  { at: 5400, op: 'checkSave', who: 'G1same', branch: 'B2', ...SAVE(ID(1)) },
  { at: 5400, op: 'checkSave', who: 'F1', ...SAVE(ID(1)) },
  { at: 5500, op: 'accept', who: 'owner', id: ID(2), branch: 'B2', note: 'the owner accepts in the branch named' },
  { at: 5600, op: 'get', who: 'owner', id: ID(2), branch: 'B2' },
  { at: 5600, op: 'get', who: 'owner', id: ID(2), branch: 'MAIN' },
]);

// ═══ 3. quotes nobody confirmed are swept away by the next quote ═════════════
await scenario('an old quote is deleted when the next quote arrives', [
  { at: 0, ...BUY100 },
  { at: 1000, ...BUY100 },
  { at: 5000, op: 'confirm', id: ID(2), note: 'this one is confirmed and stays for ever' },
  { at: 10 * MIN - 1, ...BUY100, note: 'the first quote is 599,999 ms old' },
  { at: 10 * MIN, ...BUY100, note: 'exactly 600,000 ms: still there' },
  { at: 10 * MIN, op: 'status', id: ID(1), note: 'shown as expired, not gone' },
  { at: 10 * MIN, op: 'confirm', id: ID(1) },
  { at: 10 * MIN + 1, op: 'status', id: ID(1), note: 'time alone deletes nothing' },
  { at: 10 * MIN + 1, ...BUY100, amount: '0', note: 'a refused quote sweeps nothing' },
  { at: 10 * MIN + 1, ...BUY100, note: '600,001 ms: the first quote goes' },
  { at: 10 * MIN + 1, op: 'status', id: ID(1) },
  { at: 10 * MIN + 1, op: 'confirm', id: ID(1) },
  { at: 10 * MIN + 1, op: 'customerReceipt', id: ID(1) },
  { at: 30 * MIN, ...BUY100, note: 'every quote older than ten minutes goes; the confirmed request stays' },
  { at: 30 * MIN, op: 'status', id: ID(2) },
]);

const row = (branch, id) => ({ branch, id, status: 'QUOTED', staff: null, documentRef: null, submissionId: null });
await scenario('the sweep and the other branch', [
  { at: 0, ...BUY100, branch: 'B2' },
  { at: 0, ...BUY100 },
  { at: 11 * MIN, ...BUY100, note: 'v2 sweeps every branch at once; v3 sweeps the branch of the quote, under that branch\'s lock',
    v3: { rows: [row('B2', ID(1)), row('MAIN', ID(3))] } },
  { at: 11 * MIN, op: 'status', id: ID(1), branch: 'B2', v3: { answer: { id: ID(1), queue: 'K-' + ID(1).slice(0, 8).toUpperCase(), orderNo: 'ORD-' + ID(1).toUpperCase(), transactionNo: null, status: 'EXPIRED' }, rows: [row('B2', ID(1)), row('MAIN', ID(3))] },
    note: 'so in v3 the other branch\'s quote is still there, expired, until that branch quotes again' },
]);

// ═══ 4. the queue the desk sees ═════════════════════════════════════════════
await scenario('the queue lists what was confirmed, newest first', [
  { at: 0, ...BUY100 },
  { at: 1000, op: 'quote', type: 'SELL', currency: 'USD', amount: '1000', paymentMethod: 'Alipay' },
  { at: 2000, op: 'quote', type: 'BUY', currency: 'JPY', amount: '10000' },
  { at: 3000, op: 'quote', type: 'SELL', currency: 'EUR', amount: '764' },
  { at: 4000, ...BUY100, branch: 'B2' },
  { at: 7000, op: 'confirm', id: ID(3) },
  { at: 7100, op: 'confirm', id: ID(1) },
  { at: 7200, op: 'confirm', id: ID(2) },
  { at: 7300, op: 'confirm', id: ID(5), branch: 'B2' },
  { at: 8000, op: 'notifications', who: 'F1', note: 'latest is the newest quote among those waiting, not the one confirmed last' },
  { at: 8000, op: 'list', who: 'F1' },
  { at: 8000, op: 'list', who: 'BE1' },
  { at: 8000, op: 'list', who: 'owner' },
  { at: 8000, op: 'list', who: 'RT1' },
  { at: 8000, op: 'list', who: 'FIN1' },
  { at: 8000, op: 'list' },
  { at: 8000, op: 'notifications' },
  { at: 8000, op: 'notifications', who: 'FIN1' },
  { at: 8000, op: 'notifications', who: 'owner' },
  { at: 8000, op: 'notifications', who: 'owner', branch: 'B2' },
  { at: 9000, op: 'accept', who: 'F1', id: ID(3) },
  { at: 9100, op: 'accept', who: 'F2', id: ID(2) },
  { at: 9200, op: 'identityChecked', who: 'F2', id: ID(2), confirmed: true },
  { at: 9300, op: 'notifications', who: 'F1' },
  { at: 9300, op: 'list', who: 'F1' },
  { at: 6 * MIN, op: 'list', who: 'F1', note: 'the quote nobody confirmed has expired; it was never in the queue' },
  { at: 6 * MIN, op: 'notifications', who: 'F1', note: 'a waiting request does not expire' },
  { at: 26 * 60 * MIN, op: 'accept', who: 'BE1', id: ID(1), note: 'a day later' },
  { at: 26 * 60 * MIN, op: 'list', who: 'G1', branch: 'B2' },
]);

// ═══ 5. the receipt on the customer's screen ════════════════════════════════
const ready = [
  { at: 0, ...BUY100 }, { at: 4000, op: 'confirm', id: ID(1) }, { at: 5000, op: 'accept', who: 'F1', id: ID(1) },
];
await scenario('a published receipt is shown for just under ten minutes', [
  ...ready,
  { at: 6000, op: 'publishReceipt', who: 'F1', id: ID(1), image: PNG, note: 'accepted, passport not checked yet' },
  { at: 7000, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: true },
  { at: 8000, op: 'customerReceipt', id: ID(1), note: 'nothing published yet' },
  { at: 9000, op: 'publishReceipt', who: 'F2', id: ID(1), image: PNG },
  { at: 9000, op: 'publishReceipt', who: 'RT1', id: ID(1), image: PNG },
  { at: 9000, op: 'publishReceipt', id: ID(1), image: PNG },
  { at: 9000, op: 'publishReceipt', who: 'F2', id: ID(1), image: { text: 'not an image' }, note: 'who is asked before what' },
  { at: 10000, op: 'publishReceipt', who: 'F1', id: ID(1), image: { png: 64 } },
  { at: 10001, op: 'customerReceipt', id: ID(1) },
  { at: 10000 + 10 * MIN - 1, op: 'customerReceipt', id: ID(1), note: '599,999 ms after publishing' },
  { at: 10000 + 10 * MIN, op: 'customerReceipt', id: ID(1), note: '600,000 ms: no longer shown' },
  { at: 20 * MIN, op: 'publishReceipt', who: 'F1', id: ID(1), image: { png: 80 }, note: 'published again: the image is replaced and the ten minutes start again' },
  { at: 20 * MIN + 1, op: 'customerReceipt', id: ID(1) },
  { at: 30 * MIN - 1, op: 'customerReceipt', id: ID(1) },
  { at: 30 * MIN, op: 'customerReceipt', id: ID(1) },
]);
await scenario('what counts as a receipt image', [
  ...ready,
  { at: 7000, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: true },
  ...[
    [{ text: '' }, 'empty'], [{ text: PREFIX }, 'no data after the comma'], [{ text: 'data:image/jpeg;base64,AAAA' }, 'not a PNG'],
    [{ text: 'data:image/png;base64,AAAA\n' }, 'a line break at the end'], [{ text: 'data:image/png;base64,AA AA' }, 'a space'],
    [{ text: ' data:image/png;base64,AAAA' }, 'a leading space'], [{ text: 'DATA:image/png;base64,AAAA' }, 'capitals'],
    [{ text: 'data:image/png;base64,AAAA-_' }, 'the url-safe alphabet'], [{ text: 'data:image/png;base64,ก' }, 'not ASCII'],
    [{ png: 1_800_001 }, 'one character too long'],
  ].map(([image, note], i) => ({ at: 8000 + i, op: 'publishReceipt', who: 'F1', id: ID(1), image, note })),
  { at: 8500, op: 'customerReceipt', id: ID(1), note: 'none of them was stored' },
  { at: 9000, op: 'publishReceipt', who: 'F1', id: ID(1), image: { text: 'data:image/png;base64,+/=09azAZ' }, note: 'every character of the alphabet; it is not decoded' },
  { at: 9100, op: 'publishReceipt', who: 'F1', id: ID(1), image: { text: 'data:image/png;base64,=' }, note: 'one character is enough' },
  { at: 9200, op: 'publishReceipt', who: 'F1', id: ID(1), image: { png: 1_800_000 }, note: 'exactly 1,800,000 characters' },
  { at: 9300, op: 'customerReceipt', id: ID(1) },
]);

// ═══ 6. which currencies the kiosk offers ═══════════════════════════════════
await scenario('the kiosk offers six currencies until the branch says otherwise', [
  { op: 'rates', note: 'CHF has a rate and is not offered; GBP and CNY are offered and have no rate' },
  { op: 'rates', branch: 'B2' },
  { op: 'manage', who: 'owner' },
  { op: 'manage', who: 'RT1' },
  { op: 'manage', who: 'F1' },
  { op: 'manage', who: 'MGR1' },
  { op: 'manage' },
  { op: 'quote', type: 'BUY', currency: 'CHF', amount: '100', note: 'not offered' },
  { op: 'quote', type: 'BUY', currency: 'GBP', amount: '100', note: 'offered, no rate' },
  { at: 1000, op: 'configure', who: 'F1', code: 'CHF', enabled: true, buy: '36.4', sell: '37.1' },
  { at: 1000, op: 'configure', code: 'CHF', enabled: true, buy: '36.4', sell: '37.1' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'THB', enabled: true, buy: '1', sell: '1', note: 'baht is not a currency of the list' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'XXX', enabled: true, buy: '1', sell: '1' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'chf', enabled: true, buy: '36.4', sell: '37.1' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'XXX', enabled: true, buy: '0', sell: '0', note: 'the currency is judged before the rates' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'CHF', enabled: true, buy: '0', sell: '37.1' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'CHF', enabled: true, buy: '37.2', sell: '37.1', note: 'sell below buy' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'CHF', enabled: true, buy: '-1', sell: '37.1' },
  { at: 1000, op: 'configure', who: 'RT1', code: 'CHF', enabled: true, buy: '36.4', sell: '37.1', rateSaveFails: 'เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน', note: 'the rate cannot be saved: the switch is not stored either' },
  { at: 1000, op: 'rates' },
  { at: 2000, op: 'configure', who: 'RT1', code: 'CHF', enabled: true, buy: '36.5', sell: '36.5', previousUpdated: '2026-10-06T04:00:00.000Z', partnerName: 'Partner', partnerCost: '36.1', note: 'sell equal to buy is allowed' },
  { at: 2000, op: 'rates' },
  { at: 2000, op: 'manage', who: 'RT1' },
  { at: 2000, op: 'rates', branch: 'B2', note: 'the other branch is untouched' },
  { at: 2000, op: 'manage', who: 'GR1', branch: 'B2' },
  { at: 3000, op: 'quote', type: 'SELL', currency: 'CHF', amount: '365', as: 'chf' },
  { at: 3000, op: 'quote', type: 'BUY', currency: 'USD', amount: '100', as: 'usd' },
  { at: 4000, op: 'configure', who: 'owner', code: 'USD', enabled: false, buy: '32.1', sell: '32.6' },
  { at: 4000, op: 'rates' },
  { at: 4000, op: 'quote', type: 'BUY', currency: 'USD', amount: '100', note: 'switched off' },
  { at: 7000, op: 'confirm', id: '@usd', note: 'quoted while offered, switched off since' },
  { at: 7000, op: 'confirm', id: '@chf' },
  { at: 8000, op: 'configure', who: 'owner', code: 'USD', enabled: true, buy: '32.1', sell: '32.6' },
  { at: 8000, op: 'confirm', id: '@usd', note: 'offered again at the same rate' },
  { at: 9000, op: 'configure', who: 'owner', code: 'CHF', enabled: false, buy: '36.5', sell: '36.5', note: 'a switch is one row per branch and currency, replaced' },
  { at: 9000, op: 'configure', who: 'GR1', branch: 'B2', code: 'SGD', enabled: true, buy: '24.05', sell: '24.6' },
  { at: 9000, op: 'manage', who: 'owner' },
  { at: 9000, op: 'manage', who: 'owner', branch: 'B2' },
  { at: 9000, op: 'rates', branch: 'B2' },
]);

// ═══ 7. the counter at the moment of the call ═══════════════════════════════
await scenario('the quote and the confirm look at the counter as it is now', [
  { op: 'counter', open: false },
  { op: 'rates' },
  { op: 'quote', type: 'BUY', currency: 'USD', amount: '100' },
  { op: 'quote', type: 'BUY', currency: 'USD', amount: '100', branch: 'B2', note: 'the other branch is open' },
  { op: 'counter', open: true },
  { at: 1000, op: 'quote', type: 'BUY', currency: 'USD', amount: '100' },
  { at: 1000, op: 'quote', type: 'SELL', currency: 'USD', amount: '1000' },
  { at: 2000, op: 'counter', rates: [{ code: 'USD', buy: '32.1', sell: '32.7' }], note: 'only the selling rate moves' },
  { at: 5000, op: 'confirm', id: ID(3), note: 'sold at 32.6, now 32.7' },
  { at: 5000, op: 'confirm', id: ID(2), note: 'bought at 32.1, still 32.1' },
  { at: 6000, op: 'counter', rates: [{ code: 'USD', buy: '32.1', sell: '32.6' }] },
  { at: 6000, op: 'counter', open: false },
  { at: 6000, op: 'confirm', id: ID(3), note: 'the rate is back, the counter is closed' },
  { at: 6000, op: 'confirm', id: ID(2), note: 'already waiting: answered whatever the counter does' },
  { at: 6000, op: 'accept', who: 'F1', id: ID(2), note: 'the desk does not look at the counter' },
  { at: 6100, op: 'identityChecked', who: 'F1', id: ID(2), confirmed: true },
  { at: 7000, op: 'counter', open: true },
  { at: 7000, op: 'confirm', id: ID(3) },
  { at: 5 * MIN + 1000, op: 'confirm', id: ID(3), note: 'again after five minutes: it is waiting, so it is answered' },
]);

// ═══ 8. the save of a transaction made for a kiosk request ══════════════════
const checked = [
  { at: 0, op: 'quote', type: 'SELL', currency: 'USD', amount: '1000', paymentMethod: 'PromptPay' },
  { at: 4000, op: 'confirm', id: ID(1) },
];
const S = (over = {}) => SAVE(ID(1), { type: 'SELL', amount: '30.67', method: 'PromptPay', ...over });
await scenario('a transaction is saved for the request its employee holds', [
  ...checked,
  { at: 4500, op: 'checkSave', who: 'F1', ...S(), note: 'waiting' },
  { at: 5000, op: 'accept', who: 'F1', id: ID(1) },
  { at: 5500, op: 'checkSave', who: 'F1', ...S(), note: 'accepted, passport not checked' },
  { at: 5600, op: 'complete', who: 'F1', ...S(), documentRef: 'XC-20261006-000009', note: 'v2 writes the link and leaves the status; v3 refuses (#67)',
    v3: { error: 'คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว', rows: [{ branch: 'MAIN', id: ID(1), status: 'ACCEPTED', staff: 'front1', documentRef: null, submissionId: null }] } },
]);
await scenario('the save must match the queue', [
  ...checked,
  { at: 5000, op: 'accept', who: 'F1', id: ID(1) },
  { at: 6000, op: 'identityChecked', who: 'F1', id: ID(1), confirmed: true },
  { at: 7000, op: 'checkSave', who: 'F2', ...S() },
  { at: 7000, op: 'checkSave', who: 'FIN1', ...S() },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ id: ID(7) }), note: 'no such request' },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ live: false }) },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ live: false, amount: '30.66' }), note: 'practice is said before the figures' },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ currency: 'EUR' }) },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ type: 'BUY' }) },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ amount: '30.66' }) },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ amount: '999.84' }), note: 'the baht figure in place of the dollars' },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ method: 'Cash' }) },
  { at: 7000, op: 'checkSave', who: 'F1', ...S({ method: 'Cash', amount: '30.66' }), note: 'the figures are said before the method' },
  { at: 7000, op: 'checkSave', who: 'F1', ...S() },
  { at: 7100, op: 'complete', who: 'F1', ...S(), documentRef: 'XC-20261006-000003' },
  { at: 7200, op: 'checkSave', who: 'F1', ...S(), note: 'the same save again' },
  { at: 7200, op: 'checkSave', who: 'F1', ...S({ live: false }), note: 'the same save again is held to the same rules' },
  { at: 7200, op: 'checkSave', who: 'F1', ...S({ submissionId: 'SUB-0000000000000002' }) },
  { at: 7200, op: 'checkSave', who: 'F2', ...S() },
  { at: 7300, op: 'complete', who: 'F1', ...S({ submissionId: 'SUB-0000000000000002' }), documentRef: 'XC-20261006-000004' },
  { at: 7400, op: 'status', id: ID(1) },
  { at: 7400, op: 'list', who: 'F2' },
  { at: 7400, op: 'customerReceipt', id: ID(1) },
]);

// ═══ measured limits ═════════════════════════════════════════════════════════
// The queue shows pages of this many requests (the 10-06 bundle's `queue-work.mjs`); the waiting
// count is not cut.
fresh();
const many = 103;
for (let i = 0; i < many; i++) { W.clock = T0 + i * 10; await v2.action2('quote', { branchId: 'MAIN', type: 'BUY', currency: 'USD', amount: '100' }, null); }
for (let i = 0; i < many; i++) { W.clock = T0 + 5000 + i; await v2.action2('confirm', { branchId: 'MAIN', id: idAt(i + 1) }, null); }
W.clock = T0 + 9000;
const pages = [];
for (let page = 0; page < 3; page++) pages.push(await v2q.action2('list', { branchId: 'MAIN', page }, PEOPLE.F1));
const noted = await v2.action2('notifications', { branchId: 'MAIN' }, PEOPLE.F1);
const ids = pages.flatMap((p) => p.requests.map((r) => r.id));
if (ids.join() !== Array.from({ length: many }, (_, i) => idAt(many - i)).join() || noted.latest !== idAt(many) || noted.count !== many) throw Error('fixture: the queue is not newest first, page after page');
if (pages.some((p, n) => p.page !== n)) throw Error('fixture: a page answered another number');
fresh();
const defaults = (await v2.action2('manage', { branchId: 'MAIN' }, PEOPLE.owner)).currencies.filter((c) => c.enabled).map((c) => c.code);
const limits = {
  pageSize: pages[0].requests.length,
  pagesOf103: pages.map((p) => p.requests.length),
  hasMoreOf103: pages.map((p) => p.hasMore),
  firstPageOldestOf103: (many - pages[0].requests.length) + 1, // the n-th quote made is the oldest one on page 0
  waitingCountOf103: noted.count,
  defaults,
  currencies: v2.currencies_default.map((c) => c.code),
  receiptImageMaxLength: 1_800_000,
  receiptReadableBelowMs: 600_000,
  quoteSweptAboveMs: 600_000,
};
if (limits.pageSize !== 50) throw Error('fixture: v2 pages by ' + limits.pageSize);

const ACCESS = ['Staff access required.', 'Owner or Rates access required.'];
const DESK = ['OWNER', 'FRONT', 'BACKEND', 'MANAGER'];
const ROLE_ONLY = ['list', 'notifications', 'manage', 'configure'];
const roleStep = (s) => ACCESS.includes(s.error) && (ROLE_ONLY.includes(s.op) || (PEOPLE[s.who] != null && !DESK.includes(PEOPLE[s.who].role)));
for (const sc of scenarios) sc.steps = sc.steps.filter((s) => !roleStep(s));
const count = scenarios.reduce((n, s) => n + s.steps.length, 0);
process.stdout.write(JSON.stringify({
  source: 'xchange-online@3ffc5a5 — the kiosk module of the deployed server.mjs; accept and the page size from the 10-06 bundle (queue-work.mjs); run by api/tools/v2-fixtures/gen_kiosk_service.mjs',
  t0: T0,
  people,
  // every scenario starts from this: the branches, whether a shift is open, and the counter rates
  world: (() => { fresh(); return Object.fromEntries(Object.entries(W.branches).map(([id, b]) => [id, { name: b.name, open: b.open, rates: b.rates.map((r) => ({ code: r.code, buy: rateText(r.buy), sell: rateText(r.sell) })) }])); })(),
  noSuchBranch: NO_SUCH_BRANCH,
  sentences: [...seen].sort(),
  limits,
  counts: { scenarios: scenarios.length, steps: count },
  scenarios,
}, null, 1) + '\n');
