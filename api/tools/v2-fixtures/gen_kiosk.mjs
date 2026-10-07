// Drives the kiosk module of the v2 server (`action2`, `beforeSave`, `afterSave`; server.mjs:7894-8076
// of the deployed bundle, `xchange-online@3ffc5a5`) and records what it answered.
//   node gen_kiosk.mjs path/to/10-06-bundle-dir > ../../crates/xc-kiosk/tests/fixtures/v2_kiosk_cases.json
//
// It needs `kiosk.mjs` beside it: lines 7894-8076 of the bundle's `server.mjs`, unchanged (from the
// line `// src/currencies.mjs` up to, not including, `// src/staff-routes.mjs`). The bundle is not in
// this repository, so this cannot run in CI; its output is checked in and is what the tests read.
//
// The module reads the wall clock (`Date.now()`), draws ids (`randomUUID`), talks to PostgreSQL
// (`pool.query`), asks the engine for the branch (`execute("adminSnapshot")`) and writes an audit line
// (`logEvent`). It is evaluated from its text with those five names supplied from here, so that the
// output is the same on every run:
//   Date         → a clock this script sets
//   randomUUID9  → a counter
//   pool         → an in-memory store that interprets the module's own SQL texts (below); it is not
//                  PostgreSQL, and it throws on any statement shape it does not know
//   execute      → answers `adminSnapshot` with the branch this script describes
//   logEvent     → collects the audit lines
// Nothing inside the module is replaced.
//
// The role table is not written here any more (row POL, design `docs/design/xchange-v3.md` §5.5.7): who
// may call which kiosk action is recorded once, by `gen_policy.mjs`, as the path `Kiosk` of
// `crates/xc-auth/tests/fixtures/v2_policy_cases.json`, and `Policy` is its one home. Everything below is
// still played against v2 and checked, so a sentence v2 did not say still stops the script; only the
// output leaves out (a) the `access` list and (b) every case where v2 refused an employee of a role
// outside the desk (not OWNER, FRONT, BACKEND, MANAGER) with one of its two access sentences. The
// refusals of a caller nobody signed in as stay: a step that needs a holder still refuses a caller
// without a name (`KioskActor::Customer`), and that check is the machine's.
//
// Row Q1 (design §5.5.3, ticket #328): the holder's repeat `accept` is recorded from the 10-06 bundle
// (`XC-Desk-v2-Passport-Drawer-20261006.zip`, design §5.5), whose kiosk module hands `accept` to its
// `queue-work.mjs`. The directory of that bundle is the one argument:
//   node gen_kiosk.mjs path/to/10-06-bundle-dir > ../../crates/xc-kiosk/tests/fixtures/v2_kiosk_cases.json
// The same slice of its `server.mjs` is evaluated unchanged, with `queueWork` made by its own
// `queue-work.mjs` over the same store. Every `accept` is played on both modules from the same state:
// where both claim, the record is the claim; where the 10-05 module refuses with "This request is
// already assigned or unavailable." the 10-06 one either answers the holder again (recorded: answered,
// nothing written) or refuses with one of its two new sentences, which `AcceptOutcome` leaves to the
// route (`HeldBy`, `Unavailable`) — the machine keeps the 10-05 sentence, and so does the record.
// Any other pair stops the script.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';

const src = fs.readFileSync(new URL('./kiosk.mjs', import.meta.url), 'utf8');
for (const needle of ['async function action2(', 'async function beforeSave(', 'async function afterSave(', 'var currencies_default = [']) if (!src.includes(needle)) throw Error('kiosk.mjs: ' + needle + ' not found');
if (/^import /m.test(src)) throw Error('kiosk.mjs must not contain an import line');
if (/new Date|setTimeout|Math\.random|fetch\(/.test(src)) throw Error('kiosk.mjs reads something this script does not replace');

// ── the 10-06 bundle, told apart by its own hashes (design §5.5) ────────────
const newDir = process.argv[2];
if (!newDir) throw Error('usage: node gen_kiosk.mjs path/to/10-06-bundle-dir');
const sha256 = (f) => createHash('sha256').update(fs.readFileSync(f)).digest('hex');
const NEW_SERVER = path.join(newDir, 'server.mjs'), NEW_QUEUE_WORK = path.join(newDir, 'queue-work.mjs');
if (sha256(NEW_SERVER) !== '99ddcd107bd4107226ce3065efcefea11a8b2a58d86a45b9a5872e571a8b1b0b') throw Error(NEW_SERVER + ' is not the server.mjs of the 10-06 bundle');
if (sha256(NEW_QUEUE_WORK) !== 'e03bc3499d599cf397d61a4d9138c7ae99269cf390ee076d60d10d3c348bfb30') throw Error(NEW_QUEUE_WORK + ' is not the queue-work.mjs of the 10-06 bundle');
const bundle6 = fs.readFileSync(NEW_SERVER, 'utf8');
const from6 = bundle6.indexOf('// src/currencies.mjs'), to6 = bundle6.indexOf('// src/staff-routes.mjs');
if (from6 < 0 || to6 < from6) throw Error('the kiosk module was not found in ' + NEW_SERVER);
const src6 = bundle6.slice(from6, to6);
for (const needle of ['async function action2(', 'queueWork.accept(']) if (!src6.includes(needle)) throw Error('10-06 kiosk module: ' + needle + ' not found');
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
const statements = new Set();
class Store {
  constructor() { this.t = Object.fromEntries(Object.keys(COLS).map((k) => [k, []])); }
  clone() { const s = new Store(); s.t = structuredClone(this.t); return s; }
  query(sql, args = []) {
    statements.add(sql);
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
const W = { store: new Store(), clock: 0, ids: 0, events: [], model: null, configuring: false };
// ids look like v2's (lower-case hex) and differ in their first eight characters, which is what the queue number shows
const idAt = (n) => ((0xab54c0de + n * 0x01010101) >>> 0).toString(16).padStart(8, '0') + '-7e57-4000-8000-' + n.toString(16).padStart(12, '0');
const POOL = { query: async (sql, args) => W.store.query(sql, args) };
const OUTSIDE = [
  POOL,
  async (name) => {
    if (name === 'adminSnapshot') return structuredClone(W.model);
    // `configure` saves a rate through the engine before it stores the switch; section A lets that one call through
    if (name === 'adminSaveRate' && W.configuring) return { saved: true };
    throw Error('fixture: unexpected engine call ' + name);
  },
  async (branch, actor, type, subject, details) => { W.events.push({ branch, actor, type, subject, details }); },
  () => idAt(++W.ids),
  { now: () => W.clock },
  { env: {} },
];
const v2 = new Function('pool', 'execute', 'logEvent', 'randomUUID9', 'Date', 'process', src + '\nreturn { action2, beforeSave, afterSave };')(...OUTSIDE);
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
const rateText = (n) => Number(n).toFixed(4);
const cents = (n) => BigInt(Math.round(Number(n) * 100));

// ── people ──────────────────────────────────────────────────────────────────
const staff = (id, username, role) => ({ kind: 'staff', id, email: username, username, name: username, role, employeeId: id, branchId: 'MAIN' });
const PEOPLE = {
  customer: null,
  owner: { kind: 'owner', id: 'owner', email: 'owner', username: 'owner', name: 'owner', role: 'OWNER' },
  F1: staff('E1', 'front1', 'FRONT'), F2: staff('E2', 'front2', 'FRONT'),
  BE1: staff('E3', 'backend1', 'BACKEND'), MGR1: staff('E4', 'manager1', 'MANAGER'),
  RT1: staff('E5', 'rates1', 'RATES'), FIN1: staff('E6', 'finance1', 'FINANCE'), MON1: staff('E7', 'monitor1', 'MONITOR'),
};
const people = Object.fromEntries(Object.entries(PEOPLE).map(([k, u]) => [k, u && { username: u.username, role: u.role }]));
const EVERYONE = Object.keys(PEOPLE);

// ── the branch as v2 sees it ────────────────────────────────────────────────
const T0 = Date.parse('2026-10-06T05:00:00.000Z');
const RATES = [
  { code: 'USD', buy: 32.1, sell: 32.6 }, { code: 'EUR', buy: 37.5055, sell: 38.2 }, { code: 'JPY', buy: 0.211, sell: 0.2195 },
  { code: 'GBP', buy: 43.9, sell: 0 },      // one side only: offered, but nothing can be sold
  { code: 'CNY', buy: 0, sell: 0 },         // no rate at all: not offered
  { code: 'CHF', buy: 36.4, sell: 37.1 },   // has a rate, is not among the kiosk's currencies
  { code: 'SGD', buy: 24.05, sell: 24.6 },  // switched on for the kiosk below
  { code: 'AUD', buy: 21.3333, sell: 21.9 },// switched off for the kiosk below
];
function fresh({ open = true, rates = RATES } = {}) {
  W.store = new Store(); W.clock = T0; W.ids = 0; W.events = [];
  W.model = { branches: [{ id: 'MAIN', name: 'Main' }], stock: { shifts: open ? [{ id: 'S1', status: 'OPEN' }] : [{ id: 'S0', status: 'CLOSED' }] }, rates: structuredClone(rates) };
  W.store.t.kiosk_currencies.push({ branch_id: 'MAIN', code: 'SGD', enabled: true }, { branch_id: 'MAIN', code: 'AUD', enabled: false });
}
const snap = () => ({ store: W.store.clone(), clock: W.clock, ids: W.ids, events: W.events.slice(), model: structuredClone(W.model) });
const restore = (s) => { W.store = s.store.clone(); W.clock = s.clock; W.ids = s.ids; W.events = s.events.slice(); W.model = structuredClone(s.model); };

// ── `accept` on both bundles (see the head of this file) ─────────────────────
const ALREADY = 'This request is already assigned or unavailable.';
const UNAVAILABLE = 'คิวนี้ไม่พร้อมรับเรื่อง กรุณาโหลดล่าสุด', HELD_BY = /^ออเดอร์นี้มี (.+) กำลังทำรายการแล้ว$/;
const sameRequests = (a, b) => JSON.stringify(a.kiosk_requests.map(({ started_at, completed_at, ...r }) => r)) === JSON.stringify(b.kiosk_requests.map(({ started_at, completed_at, ...r }) => r));
async function acceptOnBoth(p, user) {
  const start = snap();
  let old = null, oldError = null;
  try { old = await v2.action2('accept', p, user); } catch (e) { oldError = e.message; }
  const oldEnd = snap();
  restore(start);
  let now = null, nowError = null;
  try { now = await v2q.action2('accept', p, user); } catch (e) { nowError = e.message; }
  const nowEvents = W.events.slice(start.events.length), oldEvents = oldEnd.events.slice(start.events.length);
  const pair = `${user?.username ?? 'customer'} accept ${p.id}: 10-05 ${oldError ?? 'claims'}, 10-06 ${nowError ?? JSON.stringify(now)}`;
  if (!oldError) {
    // both claim: the same request, the same line (the 10-06 line also carries the start, which v3 keeps on the request)
    if (nowError || now.claimed !== true || !sameRequests(oldEnd.store.t, W.store.t) || nowEvents.length !== 1 || oldEvents.length !== 1
      || ['branch', 'actor', 'type', 'subject'].some((k) => nowEvents[0][k] !== oldEvents[0][k])) throw Error('the bundles claim otherwise: ' + pair);
    return old;
  }
  if (oldError === ALREADY && !nowError) {
    // the holder again: answered, nothing written
    if (now.claimed || nowEvents.length || JSON.stringify(W.store.t) !== JSON.stringify(start.store.t)) throw Error('the 10-06 repeat wrote something: ' + pair);
    const held = start.store.t.kiosk_requests.find((r) => r.id === p.id);
    if (held?.staff !== user.username || !['ACCEPTED', 'ID_CHECKED'].includes(held.status)) throw Error('the 10-06 repeat answered someone who does not hold it: ' + pair);
    return now;
  }
  restore(start);
  const held = nowError?.match(HELD_BY);
  if (oldError === ALREADY && (nowError === UNAVAILABLE || (held && start.store.t.kiosk_requests.find((r) => r.id === p.id)?.staff === held[1]))) throw Error(ALREADY);
  if (oldError === nowError) throw Error(oldError);
  throw Error('the bundles refuse otherwise: ' + pair);
}
const DEFAULTS = ['USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD']; // the kiosk's currencies when nothing is configured
/** What the caller of the machine will look up: is the counter open, and the counter rate for this currency and side. */
function facts(currency, type) {
  const on = new Set(DEFAULTS);
  for (const r of W.store.t.kiosk_currencies.filter((r) => r.branch_id === 'MAIN')) r.enabled ? on.add(r.code) : on.delete(r.code);
  const row = W.model.rates.find((r) => r.code === currency && on.has(r.code) && (r.buy > 0 || r.sell > 0));
  const n = row && (type === 'BUY' ? row.buy : type === 'SELL' ? row.sell : undefined);
  return { open: W.model.stock.shifts.some((s) => s.status === 'OPEN'), rate: n > 0 ? rateText(n) : null };
}

// ── what a stored request is, written the way the Rust type holds it ────────
function project(id) {
  const r = W.store.t.kiosk_requests.find((x) => x.id === id);
  if (!r) return null;
  const d = JSON.parse(r.data), link = W.store.t.kiosk_transactions.find((x) => x.request_id === id);
  if (!['QUOTED', 'WAITING', 'ACCEPTED', 'ID_CHECKED', 'COMPLETED'].includes(r.status)) throw Error('v2 stored status ' + r.status);
  if (r.branch_id !== 'MAIN') throw Error('fixture: one branch only');
  return {
    id: r.id, status: r.status, created: r.created, staff: r.staff === '' ? null : r.staff,
    quote: { type: d.type, currency: d.currency, rate: rateText(d.rate), foreign: money(cents(d.foreign)), thb: money(cents(d.thb)), inputAmount: money(cents(d.inputAmount)), paymentMethod: d.paymentMethod },
    transaction: link ? { submissionId: link.submission_id, documentRef: link.transaction_ref } : null,
  };
}

// ── the sentences v2 may answer with; anything else stops the script ────────
const SENTENCES = [
  'Counter is closed. Please contact our staff.', 'Select an exchange direction.', 'Please enter a valid amount and currency.', 'Amount is too small.',
  'Select cash or bank transfer to receive THB.', 'Invalid payment method.', 'Request not found. Please start again.', 'Quote expired. Please check the rate again.',
  'Please review your quote for 3 seconds.', 'Rate changed or counter closed. Please start again.', 'Request not found.', 'Staff access required.',
  'Owner or Rates access required.', 'The assigned employee must confirm the passport check.', 'Assigned staff required.',
  'Accept this request using your own account first.', 'This request is already assigned or unavailable.',
  'คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว', 'คิวต้องบันทึกเป็นรายการจริง', 'สกุลเงินและจำนวนต้องตรงกับคิว', 'ช่องทางต้องตรงกับที่ลูกค้าเลือก',
];
const seen = new Set();
const NOT_READY = 'คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว';
const PNG = 'data:image/png;base64,iVBORw0KGgo=';
const EVENT_OF = { confirm: 'KIOSK_REQUEST', accept: 'KIOSK_ACCEPTED', identityChecked: 'KIOSK_ID_CHECKED', complete: 'KIOSK_COMPLETED' };

/**
 * Runs one thing against v2 and returns the fixture record.
 * `step.action`: quote · confirm · accept · identityChecked · status · get · publishReceipt · checkSave · complete.
 */
async function run(who, step, { at, keep = true, note, mayDiffer = false } = {}) {
  const user = PEOPLE[who]; if (user === undefined) throw Error('fixture: nobody called ' + who);
  const before = snap();
  W.clock = at ?? W.clock;
  const now = W.clock, id = step.id ?? null, was = id ? project(id) : null;
  const rec = { actor: who, action: step.action, now };
  if (note) rec.note = note;
  const eventsBefore = W.events.length, rowsBefore = W.store.t.kiosk_requests.length;
  let out, error = null;
  try {
    if (step.action === 'quote') {
      rec.input = { type: step.type ?? null, currency: step.currency ?? null, amount: step.amount ?? null, paymentMethod: step.paymentMethod ?? null };
      rec.facts = facts(step.currency, step.type); rec.newId = idAt(W.ids + 1);
      const p = {}; for (const k of ['type', 'currency', 'amount', 'paymentMethod']) if (step[k] != null) p[k] = step[k];
      out = await v2.action2('quote', p, user);
    } else if (step.action === 'checkSave' || step.action === 'complete') {
      rec.before = was;
      const q = was?.quote, save = { live: true, type: q?.type ?? 'BUY', currency: q?.currency ?? 'USD', amount: q?.foreign ?? '100.00', method: q?.paymentMethod ?? 'Cash', submissionId: 'SUB-1', ...step.save };
      const p = { kioskRequestId: id, submissionId: save.submissionId, mode: save.live ? 'LIVE' : 'DEMO', currency: save.currency, type: save.type, amount: Number(save.amount), method: save.method };
      if (!user) throw Error('fixture: a save has a signed-in actor');
      if (step.action === 'checkSave') { rec.save = save; await v2.beforeSave(p, user, 'MAIN'); }
      else { rec.submissionId = save.submissionId; rec.documentRef = step.documentRef ?? 'XC-20261006-000001'; await v2.afterSave(p, { saved: true, documentRef: rec.documentRef }, user, 'MAIN'); }
    } else {
      rec.before = was;
      if (step.action === 'confirm') rec.facts = was ? facts(was.quote.currency, was.quote.type) : { open: W.model.stock.shifts.some((s) => s.status === 'OPEN'), rate: null };
      const p = { id };
      if (step.action === 'identityChecked') { rec.confirmed = step.confirmed; if (step.confirmed !== undefined) p.confirmed = step.confirmed; }
      if (step.action === 'publishReceipt') p.image = PNG;
      out = step.action === 'accept' ? await acceptOnBoth(p, user) : await v2.action2(step.action, p, user);
    }
  } catch (e) {
    if (!SENTENCES.includes(e.message)) throw Error(`v2 crashed on ${who} ${JSON.stringify(step)}: ${e.message}`);
    error = e.message; seen.add(error);
  }
  const events = W.events.slice(eventsBefore), is = id ? project(id) : null;
  if (events.length > 1) throw Error('v2 wrote two audit lines in one call');
  rec.error = error;
  if (error) {
    if (events.length || JSON.stringify(before.store.t) !== JSON.stringify(W.store.t)) throw Error(`v2 refused and still wrote: ${who} ${JSON.stringify(step)}`);
  } else if (step.action === 'quote') {
    if (W.store.t.kiosk_requests.length > rowsBefore + 1) throw Error('v2 stored two requests for one quote');
    const made = project(out.id);
    if (out.id !== rec.newId || made.status !== 'QUOTED' || made.created !== now || made.staff !== null || made.transaction) throw Error('v2 stored an unexpected quote: ' + JSON.stringify(made));
    const d = JSON.parse(W.store.t.kiosk_requests.find((r) => r.id === out.id).data);
    // what exact decimal arithmetic gives for the same input
    const a = minor(step.amount), r = tenK(rec.facts.rate);
    const foreign = step.type === 'BUY' ? a : a * 10000n / r, thb = (foreign * r + 5000n) / 10000n;
    const exact = { foreign: money(foreign), thb: money(thb) }, got = { foreign: money(cents(d.foreign)), thb: money(cents(d.thb)) };
    if (d.type !== step.type || d.currency !== step.currency || rateText(d.rate) !== rec.facts.rate || d.paymentMethod !== (step.paymentMethod || 'Cash') || cents(d.inputAmount) !== a) throw Error('v2 stored other data than it was sent: ' + JSON.stringify(d));
    if (Math.abs(d.foreign * 100 - Math.round(d.foreign * 100)) > 1e-6 || Math.abs(d.thb * 100 - Math.round(d.thb * 100)) > 1e-6) throw Error('v2 stored a figure that is not whole hundredths');
    for (const k of ['paymentMethod', 'type', 'currency', 'rate', 'foreign', 'thb', 'inputAmount']) if (out[k] !== d[k]) throw Error('v2 answered other data than it stored: ' + k);
    rec.v2 = got; rec.v2DiffersFromExact = got.foreign !== exact.foreign || got.thb !== exact.thb;
    // a request that later steps build on must be one where v2 and exact arithmetic agree
    if (rec.v2DiffersFromExact && !mayDiffer) throw Error('fixture: v2 and exact arithmetic differ on a quote that a story continues from: ' + JSON.stringify(rec.input));
    rec.request = { ...made, quote: { ...made.quote, ...exact } };
    rec.identity = identity(out);
  } else {
    rec.after = is;
    rec.event = events[0]?.type ?? null;
    if (rec.event && (rec.event !== EVENT_OF[step.action] || events[0].subject !== id)) throw Error('v2 wrote an unexpected audit line: ' + JSON.stringify(events[0]));
    if (rec.event && events[0].actor !== (step.action === 'confirm' ? 'CUSTOMER-KIOSK' : user.username)) throw Error('v2 named an unexpected actor in the audit line');
    if (rec.event) rec.eventActor = events[0].actor;
    if (!rec.event && JSON.stringify(was) !== JSON.stringify(is)) {
      // afterSave on a request that is not at the passport-checked step: v2 writes the link row and leaves the status
      if (step.action !== 'complete' || was.status === 'COMPLETED' || was.status === 'ID_CHECKED') throw Error(`v2 changed a request without an audit line: ${who} ${JSON.stringify(step)}`);
    }
    if (step.action === 'confirm' || step.action === 'status' || step.action === 'identityChecked') rec.identity = identity(out);
    if (step.action === 'get' && (out.id !== id || out.queue !== 'K-' + id.slice(0, 8).toUpperCase())) throw Error('v2 get answered another request');
    if (['accept', 'publishReceipt'].includes(step.action) && out.saved !== true) throw Error('v2 did not say saved');
    if (step.action === 'complete' && !['ID_CHECKED', 'COMPLETED'].includes(was.status)) {
      // Deliberate difference: the Rust machine refuses instead of writing a link to a request that was never checked.
      if (!is.transaction || is.status !== was.status) throw Error('fixture: v2 did something else on an unchecked request');
      rec.v2 = 'wrote the link to the saved document and left the status'; delete rec.after; rec.error = NOT_READY; rec.v2DiffersOnPurpose = true;
    }
  }
  if (!keep) restore(before);
  return rec;
}
const identity = (o) => {
  const out = { id: o.id, queue: o.queue, orderNo: o.orderNo, transactionNo: o.transactionNo, status: o.status };
  if (Object.values(out).some((v) => v === undefined)) throw Error('v2 identity lacks a field: ' + JSON.stringify(o));
  return out;
};

// ═══ A. who may call which action ═══════════════════════════════════════════
// Every action is called with a payload that passes once the caller is let in; the record keeps only
// whether v2 answered one of its two "access" sentences.
const ACCESS = ['Staff access required.', 'Owner or Rates access required.'];
async function accessCases() {
  const out = [];
  const ACTIONS = ['rates', 'quote', 'confirm', 'status', 'customerReceipt', 'notifications', 'list', 'accept', 'get', 'identityChecked', 'publishReceipt', 'manage', 'configure'];
  for (const action of ACTIONS) for (const who of EVERYONE) {
    fresh();
    const q = await v2.action2('quote', { type: 'BUY', currency: 'USD', amount: '100' }, null), id = q.id;
    const row = W.store.t.kiosk_requests[0];
    if (['get', 'identityChecked'].includes(action)) { row.status = 'ACCEPTED'; row.staff = PEOPLE[who]?.username ?? 'front1'; }
    if (action === 'publishReceipt') { row.status = 'ID_CHECKED'; row.staff = PEOPLE[who]?.username ?? 'front1'; }
    if (action === 'accept') row.status = 'WAITING';
    W.clock += 10000;
    const p = { rates: {}, quote: { type: 'BUY', currency: 'USD', amount: '100' }, confirm: { id }, status: { id }, customerReceipt: { id }, notifications: {}, list: {}, accept: { id }, get: { id },
      identityChecked: { id, confirmed: true }, publishReceipt: { id, image: PNG }, manage: {}, configure: { code: 'USD', enabled: true, buy: 32.1, sell: 32.6 } }[action];
    let error = null;
    W.configuring = action === 'configure';
    try { await v2.action2(action, p, PEOPLE[who]); } catch (e) { error = e.message; } finally { W.configuring = false; }
    if (error && !ACCESS.includes(error)) throw Error(`access: ${action} by ${who} failed for another reason: ${error}`);
    if (error) seen.add(error);
    out.push({ action, actor: who, error });
  }
  return out;
}

// ═══ B. quotes ══════════════════════════════════════════════════════════════
async function quoteCases() {
  const out = [];
  const one = async (step, opt = {}) => { fresh(opt.world); const r = await run(opt.who ?? 'customer', { action: 'quote', ...step }, { note: opt.note, mayDiffer: true }); out.push(r); return r; };
  // arithmetic: every amount at every rate, both ways
  const amounts = ['0.01', '0.30', '1', '2.05', '5.05', '41.73', '67.41', '99.99', '100', '100.50', '321', '1234.56', '3210', '19.95', '250000', '9999999.99', '10000000'];
  for (const [currency, type] of [['USD', 'BUY'], ['USD', 'SELL'], ['EUR', 'BUY'], ['EUR', 'SELL'], ['JPY', 'BUY'], ['JPY', 'SELL'], ['SGD', 'BUY'], ['SGD', 'SELL']])
    for (const amount of amounts) await one({ type, currency, amount });
  // where v2's doubles and exact decimals part: v2 hands over one hundredth less when the baht buy a whole number of
  // hundredths (the division lands just below it), and rounds a half hundredth of a baht down when the product does
  for (const [type, currency, amount] of [['SELL', 'USD', '14.67'], ['SELL', 'USD', '35.86'], ['SELL', 'EUR', '24.83'], ['SELL', 'JPY', '223.89'], ['SELL', 'SGD', '36.90'],
    ['BUY', 'EUR', '10'], ['BUY', 'EUR', '110'], ['BUY', 'JPY', '45'], ['BUY', 'SGD', '3.30'], ['BUY', 'SGD', '4.10']]) {
    const r = await one({ type, currency, amount }, { note: 'v2 and exact arithmetic differ here' });
    if (!r.v2DiffersFromExact) throw Error('fixture: expected v2 to differ from exact on ' + JSON.stringify(r.input));
  }
  // payment methods
  for (const type of ['BUY', 'SELL']) for (const paymentMethod of [null, '', 'Cash', 'Bank Transfer', 'Credit Card', 'Alipay', 'PromptPay', 'cash', 'SCB Transfer', 'Bitcoin'])
    await one({ type, currency: 'USD', amount: '100', paymentMethod });
  // the amount
  for (const type of ['BUY', 'SELL']) for (const amount of ['0', '0.00', '10000000.01', '100000000', '-5', '', 'abc', null]) await one({ type, currency: 'USD', amount });
  await one({ type: 'SELL', currency: 'USD', amount: '0.32' }, { note: 'buys less than one hundredth of a dollar' });
  await one({ type: 'SELL', currency: 'USD', amount: '0.33' }, { note: 'buys exactly one hundredth' });
  await one({ type: 'SELL', currency: 'JPY', amount: '0.01' }, { note: '0.01 baht buys 0.04 yen' });
  // the currency
  for (const type of ['BUY', 'SELL']) for (const currency of ['GBP', 'CNY', 'CHF', 'AUD', 'SGD', 'THB', 'XXX', 'usd', '', null]) await one({ type, currency, amount: '100' });
  // the direction
  for (const type of [null, '', 'buy', 'SWAP']) await one({ type, currency: 'USD', amount: '100' });
  // the counter
  await one({ type: 'BUY', currency: 'USD', amount: '100' }, { world: { open: false } });
  // which refusal comes first
  await one({ type: 'SWAP', currency: 'XXX', amount: '0', paymentMethod: 'Bitcoin' }, { world: { open: false }, note: 'everything wrong: the closed counter is named' });
  await one({ type: 'SWAP', currency: 'XXX', amount: '0', paymentMethod: 'Bitcoin' }, { note: 'counter open: the direction is named' });
  await one({ type: 'BUY', currency: 'XXX', amount: '100', paymentMethod: 'Bitcoin' }, { note: 'the currency is named before the payment method' });
  await one({ type: 'BUY', currency: 'USD', amount: '0', paymentMethod: 'Bitcoin' }, { note: 'the amount is named before the payment method' });
  await one({ type: 'SELL', currency: 'USD', amount: '0.32', paymentMethod: 'Bitcoin' }, { note: 'too small is named before the payment method' });
  await one({ type: 'BUY', currency: 'USD', amount: '100', paymentMethod: 'Bitcoin' }, { note: 'buying: an unknown method gets the cash-or-transfer sentence' });
  // a quote is made the same way whoever is signed in
  for (const who of ['owner', 'F1', 'FIN1']) await one({ type: 'SELL', currency: 'EUR', amount: '5000', paymentMethod: 'PromptPay' }, { who });
  return out;
}

// ═══ C. every action by everyone from every state ═══════════════════════════
const SEC = 1000, MIN = 60000;
const BUILD = {};
const customer = (step, opt) => run('customer', step, opt);
async function must(p) { const r = await p; if (r.error) throw Error('fixture: expected v2 to accept: ' + JSON.stringify(r)); return r; }
BUILD.QUOTED = async () => { fresh(); const r = await must(customer({ action: 'quote', type: 'BUY', currency: 'USD', amount: '100' })); return r.request.id; };
BUILD.WAITING = async () => { const id = await BUILD.QUOTED(); await must(customer({ action: 'confirm', id }, { at: T0 + 10 * SEC })); return id; };
BUILD.ACCEPTED = async () => { const id = await BUILD.WAITING(); await must(run('F1', { action: 'accept', id }, { at: T0 + 20 * SEC })); return id; };
BUILD.ID_CHECKED = async () => { const id = await BUILD.ACCEPTED(); await must(run('F1', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 30 * SEC })); return id; };
BUILD.COMPLETED = async () => { const id = await BUILD.ID_CHECKED(); await must(run('F1', { action: 'complete', id }, { at: T0 + 40 * SEC })); return id; };
const STATES = [
  // [name, builder, the moment the action arrives, what the state is]
  ['MISSING', async () => { fresh(); return idAt(0xdead); }, T0 + 10 * SEC, 'no request has this id'],
  ['QUOTED_1S', BUILD.QUOTED, T0 + 1 * SEC, 'quoted one second ago'],
  ['QUOTED_10S', BUILD.QUOTED, T0 + 10 * SEC, 'quoted ten seconds ago'],
  ['QUOTED_5MIN_1MS', BUILD.QUOTED, T0 + 5 * MIN + 1, 'quoted five minutes and one millisecond ago: reads EXPIRED'],
  ['QUOTED_11MIN', BUILD.QUOTED, T0 + 11 * MIN, 'quoted eleven minutes ago and not yet swept away'],
  ['WAITING', BUILD.WAITING, T0 + 1 * MIN, 'confirmed by the customer'],
  ['WAITING_1H', BUILD.WAITING, T0 + 60 * MIN, 'confirmed and left waiting for an hour'],
  ['ACCEPTED', BUILD.ACCEPTED, T0 + 1 * MIN, 'accepted by F1'],
  ['ID_CHECKED', BUILD.ID_CHECKED, T0 + 1 * MIN, 'passport checked by F1'],
  ['COMPLETED', BUILD.COMPLETED, T0 + 1 * MIN, 'saved by F1 under SUB-1'],
];
const EVENTS = [
  { action: 'confirm' }, { action: 'status' }, { action: 'accept' },
  { action: 'identityChecked', confirmed: true }, { action: 'identityChecked', confirmed: false }, { action: 'identityChecked' },
  { action: 'get' }, { action: 'publishReceipt' }, { action: 'checkSave' }, { action: 'complete' },
];
async function matrix() {
  const out = [];
  for (const [name, build, at, what] of STATES) {
    const id = await build(); W.clock = at;
    const here = snap(), cases = [];
    for (const ev of EVENTS) for (const who of EVERYONE) {
      if ((ev.action === 'checkSave' || ev.action === 'complete') && who === 'customer') continue; // a save has a signed-in actor
      if (ev.action === 'complete' && name === 'MISSING') continue;                               // the machine completes a request it was handed
      restore(here);
      const r = await run(who, { ...ev, id }, { keep: false });
      // the state's request and moment are written once, on the state
      if (r.now !== at || JSON.stringify(r.before ?? null) !== JSON.stringify(project(id))) throw Error('fixture: a matrix case left its state');
      delete r.now; delete r.before;
      if (r.after !== undefined && JSON.stringify(r.after) === JSON.stringify(project(id))) { delete r.after; r.unchanged = true; }
      cases.push(r);
    }
    restore(here);
    out.push({ state: name, what, now: at, request: project(id), cases });
  }
  return out;
}

// ═══ D. the counter and the rate at confirm ═════════════════════════════════
async function confirmFacts() {
  const out = [];
  const at = T0 + 10 * SEC;
  const variant = async (note, change, state = 'QUOTED') => {
    const id = await BUILD[state](); change(); out.push(await run('customer', { action: 'confirm', id }, { at, note }));
  };
  const rate = (code) => W.model.rates.find((r) => r.code === code);
  await variant('nothing changed', () => {});
  await variant('the counter closed', () => { W.model.stock.shifts[0].status = 'CLOSED'; });
  await variant('the buy rate moved by 0.0001', () => { rate('USD').buy = 32.1001; });
  await variant('only the sell rate moved; this quote is a buy', () => { rate('USD').sell = 33; });
  await variant('the buy rate was set to zero while the sell rate stays', () => { rate('USD').buy = 0; });
  await variant('the currency was switched off for the kiosk', () => { W.store.t.kiosk_currencies.push({ branch_id: 'MAIN', code: 'USD', enabled: false }); });
  await variant('the rate row is gone', () => { W.model.rates = W.model.rates.filter((r) => r.code !== 'USD'); });
  await variant('counter closed and rate moved', () => { W.model.stock.shifts[0].status = 'CLOSED'; rate('USD').buy = 30; });
  for (const state of ['WAITING', 'ACCEPTED', 'ID_CHECKED', 'COMPLETED'])
    await variant('already confirmed: neither the counter nor the rate is asked again', () => { W.model.stock.shifts[0].status = 'CLOSED'; rate('USD').buy = 30; }, state);
  return out;
}

// ═══ E. a save that names a kiosk request ═══════════════════════════════════
async function saveCases() {
  const out = [];
  const on = async (state, who, save, note) => { const id = await BUILD[state](); W.clock = T0 + 1 * MIN; out.push(await run(who, { action: 'checkSave', id, save }, { note })); };
  await on('ID_CHECKED', 'F1', {}, 'the save matches the request');
  await on('ID_CHECKED', 'F1', { live: false }, 'a practice save');
  await on('ID_CHECKED', 'F1', { currency: 'EUR' }, 'another currency');
  await on('ID_CHECKED', 'F1', { type: 'SELL' }, 'the other direction');
  await on('ID_CHECKED', 'F1', { amount: '100.01' }, 'one hundredth more');
  await on('ID_CHECKED', 'F1', { amount: '99.99' }, 'one hundredth less');
  await on('ID_CHECKED', 'F1', { method: 'Bank Transfer' }, 'another payment method');
  await on('ID_CHECKED', 'F1', { method: 'cash' }, 'the method is compared letter for letter');
  await on('ID_CHECKED', 'F1', { method: 'SCB Transfer' }, 'the old name of a bank transfer is not Cash');
  await on('ID_CHECKED', 'F1', { live: false, currency: 'EUR', method: 'Alipay' }, 'practice is named before the figures');
  await on('ID_CHECKED', 'F1', { currency: 'EUR', method: 'Alipay' }, 'the figures are named before the method');
  await on('ID_CHECKED', 'F2', { live: false, currency: 'EUR', method: 'Alipay' }, 'another employee: that is named first');
  await on('ACCEPTED', 'F1', { live: false }, 'not yet checked: that is named first');
  await on('ID_CHECKED', 'F1', { submissionId: 'SUB-9' }, 'any submission id may save a checked request');
  await on('COMPLETED', 'F1', { submissionId: 'SUB-1' }, 'the save that completed it, sent again');
  await on('COMPLETED', 'F1', { submissionId: 'SUB-2' }, 'a second save for a completed request');
  await on('COMPLETED', 'F1', { submissionId: 'SUB-1', live: false }, 'the retry is still held to the other rules');
  await on('COMPLETED', 'F1', { submissionId: 'SUB-1', amount: '50.00' }, 'the retry is still held to the figures');
  await on('COMPLETED', 'F2', { submissionId: 'SUB-1' }, 'the right submission id from another employee');
  // a bank transfer request
  fresh(); const q = await must(customer({ action: 'quote', type: 'SELL', currency: 'EUR', amount: '5000', paymentMethod: 'Bank Transfer' }));
  const id = q.request.id; await must(customer({ action: 'confirm', id }, { at: T0 + 5 * SEC })); await must(run('owner', { action: 'accept', id }, { at: T0 + 6 * SEC })); await must(run('owner', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 7 * SEC }));
  out.push(await run('owner', { action: 'checkSave', id }, { note: 'the owner accepted, checked and saves' }));
  out.push(await run('owner', { action: 'checkSave', id, save: { method: 'SCB Transfer' } }, { note: 'the request says Bank Transfer; the save says SCB Transfer' }));
  out.push(await run('owner', { action: 'checkSave', id, save: { method: 'Cash' } }));
  return out;
}

// ═══ F. stories ═════════════════════════════════════════════════════════════
async function stories() {
  const out = [];
  const story = async (name, what, play) => { fresh(); const steps = []; const s = async (who, step, opt) => { const r = await run(who, step, opt); steps.push(r); return r; }; await play(s); out.push({ name, what, steps }); };

  await story('buy-dollars-for-cash', 'a customer sells 250.50 dollars, front1 serves, the request ends completed', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'BUY', currency: 'USD', amount: '250.50' })).request.id;
    await s('customer', { action: 'status', id }, { at: T0 + 1 });
    await s('customer', { action: 'confirm', id }, { at: T0 + 2999, note: 'one millisecond too early' });
    await s('customer', { action: 'confirm', id }, { at: T0 + 3000, note: 'exactly three seconds' });
    await s('customer', { action: 'confirm', id }, { at: T0 + 3500, note: 'the screen sends it again' });
    await s('customer', { action: 'status', id }, { at: T0 + 8000 });
    await s('F1', { action: 'get', id }, { at: T0 + 9000, note: 'not accepted yet' });
    await s('F1', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 9500, note: 'not accepted yet' });
    await s('FIN1', { action: 'accept', id }, { at: T0 + 10000 });
    await s('F1', { action: 'accept', id }, { at: T0 + 11000 });
    await s('F2', { action: 'accept', id }, { at: T0 + 12000, note: 'a second employee is too late' });
    await s('F1', { action: 'accept', id }, { at: T0 + 12500, note: 'the one who holds it, again: answered, nothing written (10-06 bundle)' });
    await s('F2', { action: 'get', id }, { at: T0 + 13000 });
    await s('F1', { action: 'get', id }, { at: T0 + 13500 });
    await s('F1', { action: 'publishReceipt', id }, { at: T0 + 14000, note: 'no receipt before the passport check' });
    await s('F1', { action: 'checkSave', id }, { at: T0 + 14500, note: 'no save before the passport check' });
    await s('F1', { action: 'identityChecked', id, confirmed: false }, { at: T0 + 15000 });
    await s('F2', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 16000 });
    await s('F1', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 17000 });
    await s('F1', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 18000, note: 'sent again' });
    await s('customer', { action: 'status', id }, { at: T0 + 19000 });
    await s('F1', { action: 'get', id }, { at: T0 + 19500 });
    await s('F2', { action: 'checkSave', id }, { at: T0 + 20000 });
    await s('F1', { action: 'checkSave', id, save: { submissionId: 'SUB-A' } }, { at: T0 + 21000 });
    await s('F1', { action: 'complete', id, save: { submissionId: 'SUB-A' }, documentRef: 'XC-20261006-000042' }, { at: T0 + 22000 });
    await s('customer', { action: 'status', id }, { at: T0 + 23000, note: 'the customer now sees the document number' });
    await s('F1', { action: 'checkSave', id, save: { submissionId: 'SUB-A' } }, { at: T0 + 24000, note: 'the same save again' });
    await s('F1', { action: 'complete', id, save: { submissionId: 'SUB-A' }, documentRef: 'XC-20261006-000042' }, { at: T0 + 25000, note: 'and its completion again: nothing changes' });
    await s('F1', { action: 'checkSave', id, save: { submissionId: 'SUB-B' } }, { at: T0 + 26000, note: 'a different save' });
    await s('F1', { action: 'complete', id, save: { submissionId: 'SUB-B' }, documentRef: 'XC-20261006-000043' }, { at: T0 + 27000, note: 'cannot replace the document number' });
    await s('F1', { action: 'get', id }, { at: T0 + 28000, note: 'a completed request is no longer opened at the desk' });
    await s('F1', { action: 'publishReceipt', id }, { at: T0 + 29000 });
    await s('F1', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 30000, note: 'too late to check again' });
    await s('customer', { action: 'confirm', id }, { at: T0 + 20 * MIN, note: 'a confirm long afterwards is still answered' });
  });

  await story('sell-euros-by-promptpay', 'a customer buys euros for 5,000 baht, the owner serves', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'SELL', currency: 'EUR', amount: '5000', paymentMethod: 'PromptPay' })).request.id;
    await s('customer', { action: 'confirm', id }, { at: T0 + 4 * MIN + 59 * SEC });
    await s('owner', { action: 'accept', id }, { at: T0 + 6 * MIN, note: 'older than five minutes, but it was confirmed in time' });
    await s('F1', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 7 * MIN });
    await s('owner', { action: 'identityChecked', id, confirmed: true }, { at: T0 + 8 * MIN });
    await s('owner', { action: 'checkSave', id, save: { method: 'Cash' } }, { at: T0 + 9 * MIN });
    await s('owner', { action: 'checkSave', id }, { at: T0 + 9 * MIN + 1 });
    await s('owner', { action: 'complete', id, documentRef: 'XC-20261006-000007' }, { at: T0 + 10 * MIN });
    await s('customer', { action: 'status', id }, { at: T0 + 11 * MIN });
  });

  await story('a-quote-runs-out', 'nobody confirms; the quote reads expired after five minutes and can no longer be confirmed', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'SELL', currency: 'USD', amount: '1000' })).request.id;
    await s('customer', { action: 'status', id }, { at: T0 + 5 * MIN - 1 });
    await s('customer', { action: 'status', id }, { at: T0 + 5 * MIN, note: 'exactly five minutes: still a quote' });
    await s('customer', { action: 'status', id }, { at: T0 + 5 * MIN + 1 });
    await s('customer', { action: 'confirm', id }, { at: T0 + 5 * MIN + 1 });
    await s('F1', { action: 'accept', id }, { at: T0 + 6 * MIN });
    await s('customer', { action: 'status', id }, { at: T0 + 10 * MIN });
    await s('customer', { action: 'confirm', id }, { at: T0 + 10 * MIN });
    await s('customer', { action: 'status', id }, { at: T0 + 3 * 60 * MIN, note: 'nobody asked for a new quote, so it is still there' });
  });

  await story('confirmed-on-the-last-millisecond', 'a quote confirmed exactly five minutes after it was made', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'BUY', currency: 'JPY', amount: '10000' })).request.id;
    await s('customer', { action: 'confirm', id }, { at: T0 + 5 * MIN });
    await s('customer', { action: 'status', id }, { at: T0 + 5 * MIN + 1, note: 'a waiting request never reads expired' });
    await s('MGR1', { action: 'accept', id }, { at: T0 + 2 * 60 * MIN });
  });

  await story('the-rate-moves-and-comes-back', 'the counter rate changes between the quote and the confirm', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'BUY', currency: 'USD', amount: '100' })).request.id;
    W.model.rates.find((r) => r.code === 'USD').buy = 32.15;
    await s('customer', { action: 'confirm', id }, { at: T0 + 10 * SEC });
    await s('customer', { action: 'status', id }, { at: T0 + 11 * SEC, note: 'refused, not cancelled: the quote is still a quote' });
    W.model.rates.find((r) => r.code === 'USD').buy = 32.1;
    await s('customer', { action: 'confirm', id }, { at: T0 + 20 * SEC, note: 'the rate is back where it was' });
  });

  await story('the-counter-closes', 'the shift is closed between the quote and the confirm', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'BUY', currency: 'USD', amount: '100' })).request.id;
    W.model.stock.shifts[0].status = 'CLOSED';
    await s('customer', { action: 'confirm', id }, { at: T0 + 10 * SEC });
    await s('customer', { action: 'quote', type: 'BUY', currency: 'USD', amount: '100' }, { at: T0 + 11 * SEC });
    W.model.stock.shifts.push({ id: 'S2', status: 'OPEN' });
    await s('customer', { action: 'confirm', id }, { at: T0 + 30 * SEC, note: 'another shift is open now' });
  });

  await story('too-early-and-too-late-at-once', 'what is said when two refusals apply', async (s) => {
    const id = (await s('customer', { action: 'quote', type: 'BUY', currency: 'USD', amount: '100' })).request.id;
    W.model.stock.shifts[0].status = 'CLOSED';
    await s('customer', { action: 'confirm', id }, { at: T0 + 1000, note: 'too early and the counter is closed: too early is said' });
    await s('customer', { action: 'confirm', id }, { at: T0 + 5 * MIN + 1, note: 'expired and the counter is closed: expired is said' });
    await s('customer', { action: 'confirm', id }, { at: T0 - 1000, note: 'a clock that went backwards reads as too early' });
  });
  return out;
}

// ═══ G. sweeping old quotes away, and how long a receipt can be read ════════
// v2 deletes old quotes when the next quote is made; a published receipt is handed out for ten minutes.
async function windows() {
  const sweep = [], receipt = [];
  for (const [state, age] of [['QUOTED', 10 * MIN - 1], ['QUOTED', 10 * MIN], ['QUOTED', 10 * MIN + 1], ['QUOTED', 5 * MIN + 1], ['WAITING', 10 * MIN + 1], ['WAITING', 24 * 60 * MIN], ['ACCEPTED', 24 * 60 * MIN], ['ID_CHECKED', 24 * 60 * MIN], ['COMPLETED', 24 * 60 * MIN]]) {
    const id = await BUILD[state](); const request = project(id);
    W.clock = T0 + age;
    await v2.action2('quote', { type: 'SELL', currency: 'EUR', amount: '100' }, null);
    sweep.push({ request, now: T0 + age, swept: project(id) === null });
  }
  for (const age of [0, 1, 10 * MIN - 1, 10 * MIN, 10 * MIN + 1, 60 * MIN]) {
    const id = await BUILD.ID_CHECKED(); const published = T0 + 2 * MIN;
    W.clock = published; await v2.action2('publishReceipt', { id, image: PNG }, PEOPLE.F1);
    W.clock = published + age; const got = await v2.action2('customerReceipt', { id }, null);
    if (got.image !== null && got.image !== PNG) throw Error('v2 handed out another image');
    receipt.push({ published, now: published + age, readable: got.image !== null });
  }
  return { sweep, receipt };
}

// ── run everything ──────────────────────────────────────────────────────────
const access = await accessCases();
const quotes = await quoteCases();
const states = await matrix();
const confirms = await confirmFacts();
const saves = await saveCases();
const told = await stories();
const { sweep, receipt } = await windows();

// (b) above: the cases that are the role table, not the machine
const DESK = ['OWNER', 'FRONT', 'BACKEND', 'MANAGER'];
const roleCase = (c) => ACCESS.includes(c.error) && PEOPLE[c.actor] != null && !DESK.includes(PEOPLE[c.actor].role);
for (const s of states) s.cases = s.cases.filter((c) => !roleCase(c));
for (const s of told) s.steps = s.steps.filter((c) => !roleCase(c));

const all = [...quotes, ...states.flatMap((s) => s.cases), ...confirms, ...saves, ...told.flatMap((s) => s.steps)];
const never = SENTENCES.filter((s) => !seen.has(s));
if (never.length) throw Error('v2 never answered: ' + never.join(' | '));
const differs = quotes.filter((q) => q.v2DiffersFromExact).length, onPurpose = all.filter((r) => r.v2DiffersOnPurpose).length;
console.error(`access=${access.length} (played, not written) quotes=${quotes.length} (refused=${quotes.filter((q) => q.error).length}, v2_differs_from_exact=${differs}) states=${states.length} matrix_cases=${states.reduce((n, s) => n + s.cases.length, 0)} confirms=${confirms.length} saves=${saves.length} stories=${told.length} story_steps=${told.reduce((n, s) => n + s.steps.length, 0)} sweep=${sweep.length} receipt=${receipt.length} v2_differs_on_purpose=${onPurpose} sentences=${seen.size}/${SENTENCES.length} sql_statements=${statements.size}`);
console.log(JSON.stringify({
  source: 'v2 kiosk module (server.mjs:7894-8076 of the deployed bundle, xchange-online@3ffc5a5), evaluated unchanged with a set clock, counted ids and an in-memory store in place of PostgreSQL; driven by gen_kiosk.mjs; the holder\'s repeat accept from the 10-06 bundle (queue-work.mjs)',
  people, quotes, states, confirms, saves, stories: told, sweep, receipt,
}, null, 1));
