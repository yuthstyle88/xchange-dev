// Drives the WHOLE v2 engine (`runEngine`, extracted unchanged from the deployed bundle)
// through days of cash custody: every `ops*` action arrives as the payload a screen sends,
// the shift operations it triggers really run on v2's tables, and after every accepted
// step the script records what the branch holds — custody state, its event log, the
// shifts and the stock movements.
//   node gen_custody_service.mjs > ../../crates/xc-shift/tests/fixtures/v2_custody_service_cases.json
//
// v2 reads the wall clock and draws random ids. The module text is therefore evaluated
// with a settable clock in place of `Date` and a counting id source in place of
// `randomUUID`, so the file is the same on every run. Nothing else is replaced.
import fs from 'node:fs';
import { createHash } from 'node:crypto';

const RealDate = Date;
let clock = RealDate.parse('2026-10-06T05:00:00.000Z'); // 12:00 in Bangkok
class FakeDate extends RealDate {
  constructor(...a) { if (a.length) super(...a); else super(clock); }
  static now() { return clock; }
}
let idCounter = 0;
const idAt = (n) => '00000000-0000-4000-8000-' + n.toString(16).padStart(12, '0');
const nextId = () => idAt(++idCounter);

let src = fs.readFileSync(new URL('./engine.mjs', import.meta.url), 'utf8');
src = src.replace(/^import \{[^}]*\} from "node:crypto";$/gm, '').replace(/^export \{[^}]*\};$/m, '');
if (/^import |^export /m.test(src)) throw Error('engine.mjs: an import or export was not removed');
const { runEngine } = new Function('Date', 'randomUUID2', 'randomUUID3', 'createHash', 'createHash2',
  src + '\nreturn { runEngine };')(FakeDate, nextId, nextId, createHash, createHash);
const start = src.indexOf('var XCStock = /* @__PURE__ */ (() => {'), end = src.indexOf('const ADMIN_SCHEMA', start);
const XCStock = new Function(src.slice(start, end) + '\nreturn XCStock;')();

// ── figures ─────────────────────────────────────────────────────────────────
const cents = (n) => Math.round(Number(n) * 100);
const fmt = (c) => { const neg = c < 0; const s = String(Math.abs(c)).padStart(3, '0'); return (neg ? '-' : '') + s.slice(0, -2) + '.' + s.slice(-2); };
const shown = (o) => o == null ? null : Object.fromEntries(XCStock.currencies.filter((c) => cents(o[c] ?? 0) !== 0).map((c) => [c, fmt(cents(o[c]))]));
const SEVEN = ['THB', 'USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD'];
/** Rows that add up to the totals: hundreds, then the remainder as one piece. */
const rowsFor = (totals) => Object.entries(totals).filter(([c]) => SEVEN.includes(c)).flatMap(([currency, v]) => {
  const c = cents(v), hundreds = Math.floor(c / 10000), rest = c % 10000, out = [];
  if (hundreds) out.push({ currency, denomination: 100, count: hundreds });
  if (rest) out.push({ currency, denomination: rest / 100, count: 1 });
  return out;
});
const canonical = (v) => Array.isArray(v) ? v.map(canonical) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canonical(v[k])])) : v;
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g;
/** Every id-looking text becomes "$n" by order of first appearance (walking keys in sorted order). */
const normalise = (v, map) => {
  if (typeof v === 'string') return v.replace(UUID, (id) => { if (!map.has(id)) map.set(id, '$' + (map.size + 1)); return map.get(id); });
  if (Array.isArray(v)) return v.map((x) => normalise(x, map));
  if (v && typeof v === 'object') return Object.fromEntries(Object.keys(v).sort().map((k) => [k, normalise(v[k], map)]));
  return v;
};

/** One key per scalar part and per list element, so a step records only what it touched. */
const flatten = (doc) => {
  const out = {};
  const put = (key, v) => {
    if (Array.isArray(v)) { out[key + '#len'] = v.length; v.forEach((x, i) => { out[key + '#' + i] = x; }); } else out[key] = v;
  };
  for (const [k, v] of Object.entries(doc)) {
    if (k === 'ops' || k === 'times') for (const [sub, x] of Object.entries(v)) put(k + '.' + sub, x);
    else put(k, v);
  }
  return out;
};

// ── people ──────────────────────────────────────────────────────────────────
const P = {
  owner: { id: 'owner-1', employeeId: null, role: 'OWNER', name: 'Owner' },
  F1: { id: 'F1', employeeId: 'F1', role: 'FRONT', name: 'Front One' },
  F2: { id: 'F2', employeeId: 'F2', role: 'FRONT', name: 'Front Two' },
  FIN1: { id: 'FIN1', employeeId: 'FIN1', role: 'FINANCE', name: 'Finance One' },
  FIN2: { id: 'FIN2', employeeId: 'FIN2', role: 'FINANCE', name: 'Finance Two' },
  MON1: { id: 'MON1', employeeId: 'MON1', role: 'MONITOR', name: 'Monitor One' },
  MGR1: { id: 'MGR1', employeeId: 'MGR1', role: 'MANAGER', name: 'Manager One' },
  BE1: { id: 'BE1', employeeId: 'BE1', role: 'BACKEND', name: 'Backend One' },
  BE2: { id: 'BE2', employeeId: 'BE2', role: 'BACKEND', name: 'Backend Two' },
};
const v2Actor = (a) => ({ id: a.id, email: 'owner', employeeId: a.employeeId ?? undefined, role: a.role === 'OWNER' ? undefined : a.role, name: a.name });
const EV = (n) => idAt(0xe0000 + n); // evidence file keys
const RQ = (n) => idAt(0xa0000 + n); // client request ids
const KNOWN_ROLES = ['OWNER', 'FRONT', 'FINANCE', 'MONITOR', 'MANAGER', 'BACKEND'];

// ── one branch as v2 keeps it ───────────────────────────────────────────────
class World {
  constructor() {
    this.tables = {}; this.props = {}; this.steps = []; this.ids = new Map(); this.requestNo = 0; this.txNo = 0;
    const r = this.engine('owner', 'adminSnapshot', {});
    if (r.error) throw Error('init: ' + r.error);
    this.was = this.project({}).doc;
  }
  engine(who, action, payload) {
    const r = runEngine(action, payload, this.tables, this.props, v2Actor(P[who]), {}, null);
    return r;
  }
  keep(r) { this.tables = r.tables; this.props = r.properties; }
  /** The payload as the fixture keeps it: an `id` the server made is written as its "$n". */
  written(payload) { return typeof payload.id === 'string' && this.ids.has(payload.id) ? { ...payload, id: this.ids.get(payload.id) } : payload; }
  model() { const r = this.engine('owner', 'adminSnapshot', {}); if (r.error) throw Error('snapshot: ' + r.error); return r.result; }
  /** What the ports of the Rust service must answer before the next step. */
  facts() {
    const m = this.model(), s = XCStock.active(m.stock.shifts);
    return {
      hasLiveRate: m.rates.some((r) => r.buy > 0 && r.sell > 0),
      ratesUpdated: m.updated,
      pendingTransactions: m.transactions.filter((t) => t.status === 'PENDING').map((t) => t.id),
      lastTransaction: s ? m.transactions.filter((t) => t.shiftId === s.id).at(-1)?.id || null : null,
      customers: m.customers.map((c) => c.id),
    };
  }
  /** Everything the branch holds, written with text figures and ISO times. */
  project(result) {
    const m = this.model(), o = m.operations, delta = (d) => d == null ? null : shown(d);
    const shiftIds = m.stock.shifts.map((s) => s.id);
    const role = (r) => KNOWN_ROLES.includes(r) ? r : 'OTHER';
    const details = (e) => {
      const d = e.details ?? {};
      switch (e.type) {
        case 'CASH_TARGET': return { amounts: shown(d.amounts), by: d.by, at: d.at };
        case 'DISPATCH': return { id: d.id, kind: d.kind, currency: d.currency, amount: fmt(cents(d.amount)) };
        case 'CUT_OFF': return { at: d.at, by: d.by, shiftId: d.shiftId, lastRef: d.lastRef, pending: d.pending };
        default: return d;
      }
    };
    const ops = {
      enabled: o.enabled, phase: o.phase, custodian: o.custodian, safe: shown(o.safe), target: o.target ? { amounts: shown(o.target.amounts), by: o.target.by } : null,
      entity: o.entity ? { company: o.entity.company, site: o.entity.site, confirmedBy: o.entity.confirmedBy } : null,
      clockedIn: o.attendance.filter((a) => !a.out).map((a) => a.employeeId), attendance: o.attendance.map((a) => ({ id: a.id, employeeId: a.employeeId, role: role(a.role), out: a.out != null })),
      cutoff: o.cutoff ? { by: o.cutoff.by, shiftId: o.cutoff.shiftId, lastRef: o.cutoff.lastRef, pending: o.cutoff.pending } : null,
      serviceOpened: o.serviceOpenedAt != null,
      requests: o.requests.map((r) => ({
        id: r.id, requestId: r.requestId, kind: r.kind, status: r.status, stage: r.stage ?? null, sender: r.sender, receiver: r.receiver, note: r.note, shiftId: r.shiftId,
        evidenceKeys: r.evidenceKeys, signatures: r.signatures.map((s) => [s.employeeId, role(s.role)]),
        counted: r.counted ? shown(r.counted) : null, targetDifference: delta(r.targetDifference), expected: r.expected ? shown(r.expected) : null, variance: delta(r.variance),
        safeCount: r.safeCount ? shown(r.safeCount) : null, safeExpected: r.safeExpected ? shown(r.safeExpected) : null, safeVariance: delta(r.safeVariance),
        cutoffShift: r.cutoff?.shiftId ?? null, currency: r.currency ?? null, amount: r.amount == null ? null : fmt(cents(r.amount)), dispatchedBy: r.dispatchedBy ?? null,
        rejected: r.rejected ? [r.rejected.by, r.rejected.reason] : null, completed: r.completedAt != null, reconciliation: r.reconciliation ?? null, reconciled: r.reconciled ? [r.reconciled.by, r.reconciled.note] : null,
      })),
      holds: (o.holds || []).map((h) => ({ id: h.id, customerId: h.customerId, reason: h.reason, by: h.by, status: h.status, reviewedBy: h.reviewedBy ?? null, note: h.note ?? null })),
      drafts: (o.drafts || []).map((d) => ({ id: d.id, shiftId: d.shiftId, employeeId: d.employeeId, type: d.type, currency: d.currency, amount: fmt(cents(d.amount)), status: d.status })),
      exceptions: o.exceptions.map((x) => ({ id: x.id, requestId: x.requestId, status: x.status, targetDifference: delta(x.targetDifference), cashVariance: delta(x.cashVariance), safeVariance: delta(x.safeVariance), note: x.note, reviewedBy: x.reviewedBy ?? null })),
    };
    const times = {
      entity: o.entity?.at ?? null, target: o.target?.at ?? null, cutoff: o.cutoff?.at ?? null, serviceOpened: o.serviceOpenedAt ?? null,
      attendance: o.attendance.map((a) => [a.in, a.out ?? null]),
      requests: o.requests.map((r) => ({
        at: r.at, signatures: r.signatures.map((s) => s.at), dispatched: r.dispatchedAt ?? null, rejected: r.rejected?.at ?? null, completed: r.completedAt ?? null, reconciled: r.reconciled?.at ?? null,
        // the copies a request keeps of what it was counted against, and the count sheets row by row
        target: r.target ? { amounts: shown(r.target.amounts), by: r.target.by, at: r.target.at } : null,
        cutoff: r.cutoff ? { at: r.cutoff.at, by: r.cutoff.by, shiftId: r.cutoff.shiftId, lastRef: r.cutoff.lastRef, pending: r.cutoff.pending } : null,
        rows: r.denominations ? r.denominations.map((d) => [d.currency, fmt(cents(d.denomination)), d.count]) : null,
        safeRows: r.safeDenominations ? r.safeDenominations.map((d) => [d.currency, fmt(cents(d.denomination)), d.count]) : null,
      })),
      holds: (o.holds || []).map((h) => [h.at, h.reviewedAt ?? null]),
      drafts: (o.drafts || []).map((d) => d.at),
      exceptions: o.exceptions.map((x) => [x.at, x.reviewedAt ?? null]),
    };
    const doc = {
      events: o.events.map((e) => ({ type: e.type, at: e.at, actor: e.actor, role: role(e.role), details: details(e) })),
      movements: m.stock.movements.map((x) => ({ shift: shiftIds.indexOf(x.shiftId), currency: x.currency, direction: x.direction, amount: fmt(cents(x.amount)), reason: x.reason, requestId: x.requestId })),
      ops, result,
      shifts: m.stock.shifts.map((s) => ({ id: s.id, status: s.status, opening: shown(s.opening), expected: s.status === 'OPEN' ? shown(XCStock.expected(s, m.transactions, m.stock.movements)) : shown(s.expected), counted: s.counted ? shown(s.counted) : null, difference: s.difference ? shown(s.difference) : null, note: s.note || '', openRequest: s.openRequest, closeRequest: s.closeRequest || null })),
      times,
    };
    return { doc: flatten(normalise(canonical(doc), this.ids)) };
  }
  record(step, r) {
    if (r.error) { step.v2Error = r.error; return step; }
    const is = this.project(r.result ?? null).doc;
    // only the top-level parts this step changed; the Rust side holds the rest to "unchanged"
    step.changed = Object.fromEntries(Object.entries(is).filter(([k, v]) => k === 'result' || JSON.stringify(v) !== JSON.stringify(this.was[k])));
    this.was = is;
    return step;
  }
  /** A custody command that v2 must accept; it is kept. */
  ok(who, action, payload = {}) {
    clock += 60000;
    const step = { actor: who, action, payload: this.written(payload), now: clock, facts: this.facts() };
    const r = this.engine(who, action, payload);
    if (r.error) throw Error(`expected v2 to accept ${action} by ${who}: ${r.error}`);
    this.keep(r);
    this.steps.push(this.record(step, r));
    return r.result;
  }
  /** A custody command that v2 must refuse; nothing of it is kept. */
  no(who, action, payload = {}, why) {
    const step = { actor: who, action, payload: this.written(payload), now: clock + 60000, facts: this.facts() };
    const saved = clock; clock += 60000;
    const r = this.engine(who, action, payload);
    clock = saved;
    if (!r.error) throw Error(`expected v2 to refuse ${action} by ${who}: ${JSON.stringify(payload)}`);
    if (!/[฀-๿]/.test(r.error)) throw Error(`v2 crashed on ${action}: ${r.error}`);
    if (why) step.why = why;
    step.v2Error = r.error;
    this.steps.push(step);
    return r.error;
  }
  // ── things that happen outside custody ────────────────────────────────────
  rate(code, buy, sell) {
    clock += 60000;
    const prev = this.model().rates.find((r) => r.code === code)?.updated || '';
    const r = this.engine('owner', 'adminSaveRate', { requestId: 'rate-' + idAt(++idCounter), code, buy, sell, previousUpdated: prev });
    if (r.error) throw Error('rate: ' + r.error);
    this.keep(r);
    this.was = this.project(null).doc;
    this.steps.push({ outside: 'rate', code, buy: String(buy), sell: String(sell), facts: this.facts() });
  }
  customer(passportNo) {
    const r = this.engine('owner', 'reserveCustomer', { name: 'TEST CUSTOMER', passportNo, nationality: 'TH', mode: 'LIVE' });
    if (r.error) throw Error('customer: ' + r.error);
    this.keep(r);
    this.steps.push({ outside: 'customer', customerId: r.result.customerId, facts: this.facts() });
    return r.result.customerId;
  }
  /** A completed transaction of the open shift (what T8 will save): its stock change lands on the till. */
  sale(type, currency, amount, rate, fee, method) {
    const m = this.model(), shift = XCStock.active(m.stock.shifts);
    const balance = XCStock.expected(shift, m.transactions, m.stock.movements);
    XCStock.ensureStock(balance, XCStock.delta({ type, currency, amount: Number(amount), rate: Number(rate), fee: Number(fee), method, status: 'COMPLETED' }));
    const row = Array(34).fill(''), ref = 'XC-TEST-' + String(++this.txNo).padStart(4, '0');
    Object.assign(row, { 0: ref, 1: 'SAVED', 2: new FakeDate(), 3: new FakeDate(), 4: new RealDate('2026-10-06T00:00:00+07:00'), 6: type, 7: currency, 8: Number(amount), 9: Number(rate), 10: 0, 11: Number(fee), 12: 0, 15: method });
    this.tables.Transactions.push(row);
    this.was = this.project(null).doc;
    this.steps.push({ outside: 'tx', ref, tx: { type, currency, amount, rate, fee, method }, facts: this.facts() });
    return ref;
  }
  /** A transaction whose documents are not finished: it only shows up in the facts. */
  pending(on) {
    if (on) { const row = Array(34).fill(''); Object.assign(row, { 0: 'XC-PENDING-1', 1: 'PENDING_FILES', 2: new FakeDate(), 3: new FakeDate(), 4: new RealDate('2026-10-06T00:00:00+07:00'), 6: 'BUY', 7: 'USD', 8: 1, 9: 32.1, 10: 0, 11: 0, 12: 0, 15: 'Cash' }); this.tables.Transactions.push(row); }
    else this.tables.Transactions = this.tables.Transactions.filter((r) => r[0] !== 'XC-PENDING-1');
    this.steps.push({ outside: 'pending', on, facts: this.facts() });
  }
  rq() { return RQ(++this.requestNo); }
  lastRequest() { return this.model().operations.requests.at(-1)?.id; }
  expected() { const m = this.model(), s = XCStock.active(m.stock.shifts); return Object.fromEntries(Object.entries(XCStock.expected(s, m.transactions, m.stock.movements)).filter(([, v]) => v !== 0)); }
  safe() { return Object.fromEntries(Object.entries(this.model().operations.safe).filter(([, v]) => v !== 0)); }
}

const count = (totals) => ({ counted: totals, denominations: rowsFor(totals) });
const safeCount = (totals) => ({ safeCount: totals, safeDenominations: rowsFor(totals) });
const propose = (w, kind, extra = {}) => ({ requestId: w.rq(), kind, evidenceKeys: [EV(w.requestNo)], note: '', ...extra });

// ════ story 1: a whole day ══════════════════════════════════════════════════
function day() {
  const w = new World();
  // before anything is set up
  w.no('F1', 'opsClock', { direction: 'IN' }, 'custody is not enabled yet');
  w.no('owner', 'opsEnable', {}, 'no entity yet');
  w.no('owner', 'opsSetEntity', { company: 'XC Exchange Co., Ltd.', site: 'Suvarnabhumi B', confirmSeparated: 'yes' }, 'confirmSeparated must be the boolean true');
  w.ok('owner', 'opsSetEntity', { company: '  XC Exchange Co., Ltd.  ', site: ' Suvarnabhumi B ', confirmSeparated: true });
  w.ok('owner', 'opsEnable', {});
  w.no('F1', 'opsClock', { direction: 'UP' }, 'neither IN nor OUT');
  w.no('F1', 'opsClock', {}, 'no direction');
  for (const who of ['F1', 'F2', 'FIN1', 'FIN2', 'MON1', 'MGR1', 'BE1', 'BE2']) w.ok(who, 'opsClock', { direction: 'IN' });
  w.ok('F1', 'opsClock', { direction: 'IN' }); // again: accepted, nothing new

  // Finance sets the target; malformed amounts are refused with the stock sentences
  w.no('FIN1', 'opsTarget', { amounts: { USD: -1 } });
  w.no('FIN1', 'opsTarget', { amounts: { USD: '' } });
  w.no('FIN1', 'opsTarget', { amounts: { AED: -1, USD: '' } }, 'register order: USD is read before AED');
  w.ok('FIN1', 'opsTarget', { amounts: { THB: 50000, USD: '1000.00', XXX: 'ignored' } });

  // the safe is counted in
  w.no('FIN1', 'opsPropose', { ...propose(w, 'GIFT'), ...count({ THB: 1 }) }, 'a kind that does not exist');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000, USD: -5 }, denominations: rowsFor({ THB: 200000 }) }, 'a negative count total');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000 }, denominations: [{ currency: 'THB', denomination: '1000', count: 200 }] }, 'a denomination sent as text');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000 }, denominations: [{ currency: 'THB', denomination: 1000, count: 199.5 }, { currency: 'THB', denomination: 500, count: 1 }] }, 'half a banknote');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000 }, denominations: [{ currency: 'THB', denomination: 1000, count: -1 }] }, 'a negative number of notes');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000 }, denominations: [{ currency: 'XXX', denomination: 1000, count: 200 }] }, 'a currency that is not in the register');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000 }, denominations: [{ currency: 'THB', denomination: 1000.005, count: 200 }] }, 'a denomination with three decimals');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), counted: { THB: 200000 } }, 'no denomination rows at all');
  w.no('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), ...count({ THB: 200000 }), evidenceKeys: [] }, 'no evidence');
  const safeOpen = { ...propose(w, 'SAFE_OPEN'), ...count({ THB: 200000, USD: 5000 }) };
  w.ok('FIN1', 'opsPropose', safeOpen);
  w.ok('FIN1', 'opsPropose', safeOpen); // the same request again: answered with the same id
  w.no('FIN1', 'opsPropose', { ...safeOpen, note: 'changed' }, 'the same request id with other content');
  const safeId = w.lastRequest();
  w.no('BE1', 'opsCommit', { id: safeId }, 'Monitor has not signed');
  w.ok('MON1', 'opsSign', { id: safeId });
  w.no('BE1', 'opsCommit', {}, 'no id');
  w.ok('BE1', 'opsCommit', { id: safeId });
  w.ok('BE1', 'opsCommit', { id: safeId }); // again: accepted, nothing new

  // the drawer is counted in: ten dollars short of the target, explained
  w.no('F1', 'opsPropose', { ...propose(w, 'OPEN'), ...count({ THB: 50000, USD: 990 }), note: 'ten dollars short' }, 'no live rate yet');
  w.rate('USD', 32.1, 32.6);
  w.rate('EUR', 37.5, 38.25);
  w.ok('F1', 'opsPropose', { ...propose(w, 'OPEN'), ...count({ THB: 50000, USD: 990 }), note: '  ten dollars short  ' });
  const openId = w.lastRequest();
  w.ok('BE1', 'opsCommit', { id: openId }); // the shift opens; a cash exception is recorded
  w.no('BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: '2026-01-01T00:00:00.000Z' }, 'a stale rate set');
  w.no('BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true }, 'no rate set named');
  w.no('BE1', 'opsStartService', { equipmentReady: 'true', ratesVerified: true, rateUpdated: w.model().updated }, 'equipmentReady must be the boolean true');
  w.ok('BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: w.model().updated });

  // serving
  const cx = w.customer('P1234567');
  w.no('F1', 'opsBeginTransaction', { type: 'SWAP', currency: 'USD', amount: 100 });
  w.no('F1', 'opsBeginTransaction', { type: 'BUY', currency: 'XXX', amount: 100 });
  w.no('F1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: -1 });
  w.no('F1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 1.005 });
  w.no('F1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: '' });
  w.no('F1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD' });
  w.ok('F1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 100 });
  w.sale('BUY', 'USD', '100', '32.1', '0', 'Cash');
  w.ok('F1', 'opsBeginTransaction', { type: 'SELL', currency: 'USD', amount: '50.50' });
  w.sale('SELL', 'USD', '50.50', '32.6', '10', 'Cash');

  // money from the safe to the drawer
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), receiverId: 'FIN1', currency: 'USD', amount: -5 });
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), receiverId: 'FIN1', currency: 'USD', amount: 1.005 });
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), receiverId: 'FIN1', currency: 'USD', amount: 'abc' });
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), receiverId: 'FIN1', currency: 'USD' });
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), receiverId: 'FIN1', currency: 'XXX', amount: 5 });
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), currency: 'USD', amount: 5 }, 'no receiver');
  w.ok('F1', 'opsPropose', { ...propose(w, 'SAFE_OUT'), receiverId: 'FIN1', currency: 'USD', amount: 500, note: 'top up the drawer' });
  const outId = w.lastRequest();
  w.no('F1', 'opsPropose', { ...propose(w, 'SAFE_IN'), receiverId: 'FIN1', currency: 'USD', amount: 1 }, 'one request at a time');
  w.no('FIN1', 'opsSign', { id: outId }, 'not dispatched yet');
  w.ok('BE1', 'opsDispatch', { id: outId }); // the safe gives the money up; nothing reaches the drawer yet
  w.ok('BE1', 'opsDispatch', { id: outId }); // again: nothing new
  w.ok('FIN1', 'opsSign', { id: outId });
  w.ok('BE1', 'opsCommit', { id: outId }); // the drawer receives it: a stock movement IN

  // money from the drawer to the safe: more than the drawer holds cannot leave it
  w.ok('F1', 'opsPropose', { ...propose(w, 'SAFE_IN'), receiverId: 'FIN1', currency: 'USD', amount: 99999 });
  const tooMuch = w.lastRequest();
  w.no('BE1', 'opsDispatch', { id: tooMuch }, 'the stock movement fails, so the whole dispatch does');
  w.ok('MON1', 'opsReject', { id: tooMuch, reason: 'the drawer never held that much' });
  w.ok('F1', 'opsPropose', { ...propose(w, 'SAFE_IN'), receiverId: 'FIN1', currency: 'USD', amount: 300.25 });
  const inId = w.lastRequest();
  w.ok('BE1', 'opsDispatch', { id: inId }); // a stock movement OUT, under the request's own id
  w.no('MON1', 'opsReject', { id: inId, reason: 'x' }, 'money in transit');
  w.ok('FIN1', 'opsSign', { id: inId });
  w.ok('BE1', 'opsCommit', { id: inId }); // the safe receives it

  // money in from outside, with a note too long to become a movement reason
  const long = 'n'.repeat(195);
  w.ok('F1', 'opsPropose', { ...propose(w, 'EXTERNAL_IN'), receiverId: 'MGR1', currency: 'EUR', amount: 1000, note: long });
  const extLong = w.lastRequest();
  w.ok('BE1', 'opsDispatch', { id: extLong });
  w.ok('MGR1', 'opsSign', { id: extLong });
  w.ok('FIN2', 'opsSign', { id: extLong });
  w.no('BE1', 'opsCommit', { id: extLong }, 'the movement reason would exceed 200 characters: the commit is undone');
  // money in transit cannot be rejected, so that request stays pending for good in v2; a new branch tells the rest
  return { name: 'a day: set-up, safe, opening, service, transfers', steps: w.steps };
}

// ════ story 2: transfers out and in, holds, handover, close, reconcile ══════
function evening() {
  const w = new World();
  w.ok('owner', 'opsSetEntity', { company: 'XC', site: 'B', confirmSeparated: true });
  w.ok('owner', 'opsEnable', {});
  for (const who of ['F1', 'F2', 'FIN1', 'FIN2', 'MON1', 'MGR1', 'BE1', 'BE2']) w.ok(who, 'opsClock', { direction: 'IN' });
  w.rate('USD', 32.1, 32.6);
  w.ok('FIN1', 'opsTarget', { amounts: { THB: 100000, USD: 2000 } });
  w.ok('FIN1', 'opsPropose', { ...propose(w, 'SAFE_OPEN'), ...count({ THB: 500000, USD: 10000, EUR: 2000 }) });
  let id = w.lastRequest();
  w.ok('MON1', 'opsSign', { id });
  w.ok('BE1', 'opsCommit', { id });
  w.ok('F1', 'opsPropose', { ...propose(w, 'OPEN'), ...count({ THB: 100000, USD: 2000 }) });
  id = w.lastRequest();
  w.ok('BE1', 'opsCommit', { id });
  w.ok('BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: w.model().updated });
  const cx = w.customer('P7654321');
  w.sale('BUY', 'USD', '250', '32.1', '0', 'Cash');
  w.sale('SELL', 'USD', '100', '32.6', '25.5', 'Bank Transfer');

  // out to the bank and in from the bank
  w.ok('F1', 'opsPropose', { ...propose(w, 'EXTERNAL_OUT'), receiverId: 'MGR1', currency: 'THB', amount: 20000, note: 'to the bank' });
  id = w.lastRequest();
  w.ok('BE1', 'opsDispatch', { id });
  w.ok('MGR1', 'opsSign', { id });
  w.ok('FIN1', 'opsSign', { id });
  w.ok('BE1', 'opsCommit', { id });
  w.ok('F1', 'opsPropose', { ...propose(w, 'EXTERNAL_IN'), receiverId: 'F2', currency: 'EUR', amount: '750.50' });
  id = w.lastRequest();
  w.ok('BE1', 'opsDispatch', { id });
  w.ok('F2', 'opsSign', { id });
  w.ok('FIN1', 'opsSign', { id });
  w.ok('BE2', 'opsCommit', { id }); // reason falls back to the request id: there is no note

  // a customer is held and released
  w.no('F1', 'opsHold', { customerId: 'CX-99999999', reason: 'unknown customer' });
  w.no('MON1', 'opsHold', { customerId: cx }, 'no reason');
  w.ok('F1', 'opsHold', { customerId: cx, reason: 'passport looks altered' });
  const holdId = w.model().operations.holds.at(-1).id;
  w.no('MON1', 'opsRelease', { id: holdId }, 'no note');
  w.ok('MON1', 'opsRelease', { id: holdId, note: 'checked with immigration, fine' });

  // hand the drawer to the next Front
  w.pending(true);
  w.ok('BE1', 'opsCutoff', {});
  w.no('F1', 'opsPropose', { ...propose(w, 'HANDOVER'), receiverId: 'F2', ...count(w.expected()), ...safeCount(w.safe()) }, 'a transaction is unfinished');
  w.pending(false);
  w.ok('BE1', 'opsCutoff', {}); // again: replaces the first cut-off
  w.no('F1', 'opsPropose', { ...propose(w, 'HANDOVER'), receiverId: 'F2', ...count(w.expected()), safeCount: { THB: -1 }, safeDenominations: [] }, 'a negative safe total');
  w.no('F1', 'opsPropose', { ...propose(w, 'HANDOVER'), receiverId: 'F2', ...count(w.expected()), safeCount: w.safe(), safeDenominations: [{ currency: 'THB', denomination: 1000, count: 1.5 }] }, 'half a banknote in the safe');
  w.ok('F1', 'opsPropose', { ...propose(w, 'HANDOVER'), receiverId: 'F2', ...count(w.expected()), ...safeCount(w.safe()) });
  id = w.lastRequest();
  w.ok('MGR1', 'opsSign', { id });
  w.ok('MON1', 'opsSign', { id });
  w.no('BE1', 'opsCommit', { id }, 'the receiver has not signed');
  w.ok('F2', 'opsSign', { id });
  w.ok('BE1', 'opsCommit', { id }); // one shift closes, the next opens with what was counted; F2 holds the drawer
  w.sale('BUY', 'USD', '10', '32.1', '0', 'Cash');

  // close for the night, one hundred baht short in the drawer and a euro over in the safe
  w.ok('BE1', 'opsCutoff', {});
  const exp = w.expected(), sf = w.safe();
  const counted = { ...exp, THB: Math.round((exp.THB - 100) * 100) / 100 }, safeCounted = { ...sf, EUR: Math.round((sf.EUR + 1) * 100) / 100 };
  w.no('F2', 'opsPropose', { ...propose(w, 'CLOSE'), ...count(counted), ...safeCount(safeCounted) }, 'a difference without a note');
  w.ok('F2', 'opsPropose', { ...propose(w, 'CLOSE'), ...count(counted), ...safeCount(safeCounted), note: 'a hundred baht short; one euro over in the safe' });
  id = w.lastRequest();
  w.ok('MGR1', 'opsSign', { id });
  w.ok('MON1', 'opsSign', { id });
  w.ok('BE2', 'opsCommit', { id }); // the shift closes; nobody holds the drawer; an exception is recorded
  w.no('F2', 'opsReconcile', { id, note: 'x' }, 'not Finance');
  w.no('FIN1', 'opsReconcile', { id }, 'no note');
  w.ok('FIN1', 'opsReconcile', { id, note: 'counted again with the manager, written off' }); // the exception is reviewed
  w.ok('F2', 'opsClock', { direction: 'OUT' });
  w.ok('F2', 'opsClock', { direction: 'IN' }); // a second attendance record
  w.no('MON1', 'opsSign', { id: idAt(0xdead) }, 'no such request');
  return { name: 'an evening: bank runs, a hold, handover, close, reconcile', steps: w.steps };
}

const stories = [day(), evening()];
const sentences = {};
for (const s of stories) for (const st of s.steps) if (st.v2Error) sentences[st.v2Error] = (sentences[st.v2Error] ?? 0) + 1;

// one step per line: the file stays reviewable in a diff
const out = [
  '{',
  ' "source": ' + JSON.stringify('xchange-online@3ffc5a5 — runEngine(ops*) with the shift operations it applies, on v2\'s own tables') + ',',
  ' "sentences": ' + JSON.stringify(sentences) + ',',
  ' "stories": [',
  stories.map((s) => '  {"name": ' + JSON.stringify(s.name) + ', "steps": [\n' + s.steps.map((st) => '   ' + JSON.stringify(st)).join(',\n') + '\n  ]}').join(',\n'),
  ' ]',
  '}',
];
process.stdout.write(out.join('\n') + '\n');
