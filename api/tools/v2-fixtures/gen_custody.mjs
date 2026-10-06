// Drives the custody module of the v2 engine (`runOperations`, extracted unchanged from
// the deployed bundle; server.mjs:5969-6251) and records what it answered.
//   node gen_custody.mjs > ../../crates/xc-shift/tests/fixtures/v2_custody_cases.json
//
// v2 draws ids with randomUUID and hashes a proposal with sha256; the module is therefore
// evaluated from the text of engine.mjs with a counting id source in place of randomUUID,
// so that the output is the same on every run. Nothing else is replaced.
import fs from 'node:fs';
import { createHash } from 'node:crypto';

const src = fs.readFileSync(new URL('./engine.mjs', import.meta.url), 'utf8');
const slice = (from, to) => { const a = src.indexOf(from), b = src.indexOf(to, a); if (a < 0 || b < 0) throw Error(from + ' not found in engine.mjs'); return src.slice(a, b); };
let idCounter = 0;
const idAt = (n) => '00000000-0000-4000-8000-' + n.toString(16).padStart(12, '0');
const nextId = () => idAt(++idCounter);
const { runOperations, initialOperations } = new Function('randomUUID2', 'createHash',
  slice('var zero = () =>', '// src/online/lib/engine.mjs') + '\nreturn { runOperations, initialOperations };')(nextId, createHash);
const XCStock = new Function(slice('var XCStock = /* @__PURE__ */ (() => {', 'const ADMIN_SCHEMA') + '\nreturn XCStock;')();
const fingerprint = (p) => createHash('sha256').update(JSON.stringify(p)).digest('hex');

// ── figures: the fixture writes money as text, v2 takes numbers ─────────────
const cents = (n) => Math.round(Number(n) * 100);
const fmt = (c) => { const neg = c < 0; const s = String(Math.abs(c)).padStart(3, '0'); return (neg ? '-' : '') + s.slice(0, -2) + '.' + s.slice(-2); };
const shown = (o) => o == null ? null : Object.fromEntries(XCStock.currencies.filter((c) => cents(o[c] ?? 0) !== 0).map((c) => [c, fmt(cents(o[c]))]));
const numbers = (o) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, Number(v)]));
const SEVEN = ['THB', 'USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD'];
/** A count whose rows add up to the totals: hundreds, then the remainder as one piece. */
const sheet = (totals, rows) => ({
  totals,
  denominations: rows ?? Object.entries(totals).filter(([c]) => SEVEN.includes(c)).flatMap(([currency, v]) => {
    const c = cents(v), hundreds = Math.floor(c / 10000), rest = c % 10000, out = [];
    if (hundreds) out.push({ currency, denomination: '100', count: hundreds });
    if (rest) out.push({ currency, denomination: fmt(rest), count: 1 });
    return out;
  }),
});

// ── people ──────────────────────────────────────────────────────────────────
const P = {
  owner: { id: 'owner-1', employeeId: null, role: 'OWNER', name: 'Owner' },
  F1: { id: 'F1', employeeId: 'F1', role: 'FRONT', name: 'Front One' },
  F2: { id: 'F2', employeeId: 'F2', role: 'FRONT', name: 'Front Two' },
  F3: { id: 'F3', employeeId: 'F3', role: 'FRONT', name: 'Front Three' }, // never clocks in
  FIN1: { id: 'FIN1', employeeId: 'FIN1', role: 'FINANCE', name: 'Finance One' },
  FIN2: { id: 'FIN2', employeeId: 'FIN2', role: 'FINANCE', name: 'Finance Two' },
  MON1: { id: 'MON1', employeeId: 'MON1', role: 'MONITOR', name: 'Monitor One' },
  MGR1: { id: 'MGR1', employeeId: 'MGR1', role: 'MANAGER', name: 'Manager One' },
  BE1: { id: 'BE1', employeeId: 'BE1', role: 'BACKEND', name: 'Backend One' },
  BE2: { id: 'BE2', employeeId: 'BE2', role: 'BACKEND', name: 'Backend Two' },
  RT1: { id: 'RT1', employeeId: 'RT1', role: 'RATES', name: 'Rates One' },
};
const v2Actor = (a) => ({ id: a.id, employeeId: a.employeeId ?? undefined, role: a.role === 'OWNER' ? undefined : a.role, name: a.name });
const NOON = Date.parse('2026-10-06T05:00:00.000Z'); // 12:00 in Bangkok
const bangkok = (hhmm) => { const [h, m] = hhmm.split(':').map(Number); return Date.parse('2026-10-06T00:00:00.000Z') + ((h - 7) * 60 + m) * 60000; };
const EV = (n) => idAt(0xe0000 + n); // evidence file keys
const RQ = (n) => idAt(0xa0000 + n); // client request ids

// ── one branch as v2 sees it ────────────────────────────────────────────────
class World {
  constructor() {
    this.ops = initialOperations();
    this.model = { stock: { shifts: [], movements: [] }, transactions: [], rates: [{ code: 'USD', buy: 32.1, sell: 32.6 }], customers: [{ id: 'CX-00000001' }], updated: '2026-10-06T03:00:00.000Z' };
    this.clock = NOON; this.shiftNo = 0; this.steps = []; this.requestNo = 0;
  }
  clone() { const w = new World(); Object.assign(w, structuredClone({ ops: this.ops, model: this.model, clock: this.clock, shiftNo: this.shiftNo, requestNo: this.requestNo })); w.steps = this.steps.slice(); return w; }
  active() { return this.model.stock.shifts.find((s) => s.status === 'OPEN') || null; }
  facts(cmd) {
    const s = this.active();
    return {
      openShift: s ? { id: s.id, expected: shown(XCStock.expected(s, this.model.transactions, this.model.stock.movements)) } : null,
      hasLiveRate: this.model.rates.some((r) => r.buy > 0 && r.sell > 0),
      ratesUpdated: this.model.updated,
      pendingTransactions: this.model.transactions.filter((t) => t.status === 'PENDING').map((t) => t.id),
      lastTransaction: s ? this.model.transactions.filter((t) => t.shiftId === s.id).at(-1)?.id || null : null,
      customerKnown: cmd.action === 'opsHold' ? this.model.customers.some((c) => c.id === cmd.customerId) : false,
    };
  }
  /** The payload v2 receives for a command written the way the fixture writes it. */
  payload(cmd) {
    const p = { ...cmd }; delete p.action; delete p.payloadHash;
    if (cmd.action === 'opsBeginTransaction') p.amount = Number(cmd.amount);
    if (cmd.action === 'opsTarget') p.amounts = numbers(cmd.amounts);
    if (cmd.action === 'opsPropose') {
      const rows = (s) => s.denominations.length || !s.omitWhenEmpty ? s.denominations.map((r) => ({ ...r, denomination: Number(r.denomination) })) : undefined;
      delete p.counted; delete p.safeCount; delete p.receiver;
      p.receiverId = cmd.receiver ?? undefined;
      p.counted = numbers(cmd.counted.totals); p.denominations = rows(cmd.counted);
      p.safeCount = numbers(cmd.safeCount.totals); p.safeDenominations = rows(cmd.safeCount);
      if (cmd.amount == null) delete p.amount; else p.amount = Number(cmd.amount);
      if (cmd.currency == null) delete p.currency;
    }
    return p;
  }
  /** What the Rust state must equal, written with text figures. */
  project() {
    const o = this.ops, delta = (d) => d == null ? null : shown(d);
    return {
      enabled: o.enabled, phase: o.phase, custodian: o.custodian, safe: shown(o.safe), target: o.target ? { amounts: shown(o.target.amounts), by: o.target.by } : null,
      entity: o.entity ? { company: o.entity.company, site: o.entity.site, confirmedBy: o.entity.confirmedBy } : null,
      clockedIn: o.attendance.filter((a) => !a.out).map((a) => a.employeeId), attendance: o.attendance.map((a) => ({ id: a.id, employeeId: a.employeeId, role: a.role, out: a.out != null })),
      cutoff: o.cutoff ? { by: o.cutoff.by, shiftId: o.cutoff.shiftId, lastRef: o.cutoff.lastRef, pending: o.cutoff.pending } : null,
      serviceOpened: o.serviceOpenedAt != null,
      requests: o.requests.map((r) => ({
        id: r.id, requestId: r.requestId, payloadHash: r.hash, kind: r.kind, status: r.status, stage: r.stage ?? null, sender: r.sender, receiver: r.receiver, note: r.note, shiftId: r.shiftId,
        evidenceKeys: r.evidenceKeys, signatures: r.signatures.map((s) => [s.employeeId, s.role]),
        counted: r.counted ? shown(r.counted) : null, targetDifference: delta(r.targetDifference), expected: r.expected ? shown(r.expected) : null, variance: delta(r.variance),
        safeCount: r.safeCount ? shown(r.safeCount) : null, safeExpected: r.safeExpected ? shown(r.safeExpected) : null, safeVariance: delta(r.safeVariance),
        cutoffShift: r.cutoff?.shiftId ?? null, currency: r.currency ?? null, amount: r.amount == null ? null : fmt(cents(r.amount)), dispatchedBy: r.dispatchedBy ?? null,
        rejected: r.rejected ? [r.rejected.by, r.rejected.reason] : null, completed: r.completedAt != null, reconciliation: r.reconciliation ?? null, reconciled: r.reconciled ? [r.reconciled.by, r.reconciled.note] : null,
      })),
      holds: (o.holds || []).map((h) => ({ id: h.id, customerId: h.customerId, reason: h.reason, by: h.by, status: h.status, reviewedBy: h.reviewedBy ?? null, note: h.note ?? null })),
      drafts: (o.drafts || []).map((d) => ({ id: d.id, shiftId: d.shiftId, employeeId: d.employeeId, type: d.type, currency: d.currency, amount: fmt(cents(d.amount)), status: d.status })),
      exceptions: o.exceptions.map((x) => ({ id: x.id, requestId: x.requestId, status: x.status, targetDifference: delta(x.targetDifference), cashVariance: delta(x.cashVariance), safeVariance: delta(x.safeVariance), note: x.note, reviewedBy: x.reviewedBy ?? null })),
    };
  }
  /** Runs one command against v2 and returns the fixture record. `keep` = false leaves the world as it was. */
  run(who, cmd, { at, keep = true, expect, note } = {}) {
    const actor = P[who], now = at ?? (keep ? (this.clock += 60000) : this.clock + 60000);
    cmd = this.hashed(cmd);
    const facts = this.facts(cmd), before = structuredClone({ ops: this.ops, model: this.model, shiftNo: this.shiftNo }), firstId = idCounter + 1, was = this.project();
    const effects = [], requestIds = new Set(this.ops.requests.map((r) => r.requestId));
    const applyShift = (name, p) => {
      const own = requestIds.has(p.requestId) ? p.requestId : null;
      if (name === 'adminOpenShift') {
        if (this.active()) throw Error('fixture: a shift is already open');
        this.model.stock.shifts.push({ id: 'S' + ++this.shiftNo, status: 'OPEN', opening: p.opening });
        effects.push({ effect: 'OpenShift', requestId: own, opening: shown(p.opening) });
      } else if (name === 'adminCloseShift') {
        this.active().status = 'CLOSED';
        effects.push({ effect: 'CloseShift', requestId: own, shiftId: p.shiftId, expected: shown(p.expected), counted: shown(p.counted), note: p.note });
      } else if (name === 'adminMoveStock') {
        this.model.stock.movements.push({ shiftId: p.shiftId, currency: p.currency, direction: p.direction, amount: p.amount });
        effects.push({ effect: 'MoveStock', requestId: own, shiftId: p.shiftId, currency: p.currency, direction: p.direction, amount: fmt(cents(p.amount)), reason: p.reason });
      } else throw Error('fixture: unexpected ' + name);
    };
    const record = { actor: who, command: cmd, now, facts };
    if (note) record.note = note;
    let out;
    try {
      out = runOperations(cmd.action, this.payload(cmd), v2Actor(actor), this.ops, this.model, XCStock, applyShift, new Date(now));
    } catch (e) {
      Object.assign(this, structuredClone(before));
      record.facts.newId = idAt(firstId);
      record.v2Error = e.message;
      record.error = expect ?? e.message; // `expect` only where v2 crashed instead of refusing
      if (expect) record.v2DiffersOnPurpose = true;
      else if (!/[฀-๿]/.test(e.message)) throw Error('v2 crashed on ' + JSON.stringify(cmd) + ': ' + e.message);
      return record;
    }
    const old = before.ops, created = [];
    for (const list of ['attendance', 'requests', 'holds', 'drafts', 'exceptions']) for (const x of out.state[list] || []) if (!(old[list] || []).some((y) => y.id === x.id)) created.push(x.id);
    if (created.length > 1) throw Error('v2 created two records in one command: ' + JSON.stringify(cmd));
    record.facts.newId = created[0] ?? idAt(firstId);
    const subject = (e) => e.type === 'HOLD_CUSTOMER' ? e.details.customerId : ['PROPOSE', 'REJECT', 'SIGN', 'DISPATCH', 'COMMIT', 'RECONCILE', 'RELEASE_CUSTOMER'].includes(e.type) ? e.details.id : null;
    record.events = out.state.events.slice(old.events.length).map((e) => ({ kind: e.type, subject: subject(e) }));
    record.effects = effects;
    record.id = out.result.id ?? null;
    // the server keeps the state as JSON text between commands (server.mjs:6860, 6882); so does this,
    // or two fields that v2 assigns the same object (`safe = r.counted`, 6207) would stay one object
    this.ops = JSON.parse(JSON.stringify(out.state));
    // only the parts of the state that this command changed; the rest must stay as it was
    const is = this.project();
    record.changed = Object.fromEntries(Object.entries(is).filter(([k, v]) => JSON.stringify(v) !== JSON.stringify(was[k])));
    if (!keep) Object.assign(this, structuredClone(before));
    else this.steps.push(record);
    return record;
  }
  ok(who, cmd, opt) { const r = this.run(who, cmd, opt); if (r.error) { this.steps.push(r); throw Error(`expected v2 to accept ${cmd.action} by ${who}: ${r.error}`); } return r; }
  no(who, cmd, opt) { const r = this.run(who, cmd, opt); if (!r.error) throw Error(`expected v2 to refuse ${cmd.action} by ${who}`); this.steps.push(r); return r; }
  hashed(cmd) { return cmd.action === 'opsPropose' && cmd.payloadHash == null ? { ...cmd, payloadHash: fingerprint(this.payload(cmd)) } : cmd; }
  rq() { return RQ(++this.requestNo); }
  lastRequest() { return this.ops.requests.at(-1)?.id; }
  // things that happen outside custody
  sale(amount = 100) { const s = this.active(); this.model.transactions.push({ id: 'XC-' + (this.model.transactions.length + 1), shiftId: s.id, status: 'COMPLETED', type: 'BUY', currency: 'USD', amount, rate: 32.1, fee: 0, method: 'Cash' }); }
}

// ── commands as the fixture writes them ─────────────────────────────────────
const none = sheet({}, []);
const propose = (kind, o = {}) => ({ action: 'opsPropose', requestId: o.requestId, kind, receiver: o.receiver ?? null, note: o.note ?? '', evidenceKeys: o.evidenceKeys ?? [EV(1)], counted: o.counted ?? none, safeCount: o.safeCount ?? none, currency: o.currency ?? null, amount: o.amount ?? null });
const C = {
  entity: (o = {}) => ({ action: 'opsSetEntity', company: 'XCHANGE Co., Ltd.', site: 'Main', confirmSeparated: true, ...o }),
  enable: () => ({ action: 'opsEnable' }),
  clock: (direction) => ({ action: 'opsClock', direction }),
  begin: (o = {}) => ({ action: 'opsBeginTransaction', type: 'BUY', currency: 'USD', amount: '100', ...o }),
  target: (amounts = { THB: '50000', USD: '1000' }) => ({ action: 'opsTarget', amounts }),
  start: (w, o = {}) => ({ action: 'opsStartService', equipmentReady: true, ratesVerified: true, rateUpdated: w.model.updated, ...o }),
  cutoff: () => ({ action: 'opsCutoff' }),
  hold: (o = {}) => ({ action: 'opsHold', customerId: 'CX-00000001', reason: 'passport photo unclear', ...o }),
  release: (id, note = 'checked the original') => ({ action: 'opsRelease', id, note }),
  reject: (id, reason = 'count again') => ({ action: 'opsReject', id, reason }),
  sign: (id) => ({ action: 'opsSign', id }),
  dispatch: (id) => ({ action: 'opsDispatch', id }),
  commit: (id) => ({ action: 'opsCommit', id }),
  reconcile: (id, note = 'matched the bank slip') => ({ action: 'opsReconcile', id, note }),
};
const DRAWER = { THB: '50000', USD: '1000' }, SAFE = { THB: '200000', USD: '5000', EUR: '300' };

// ── builders: each returns a world standing in one state ────────────────────
const B = {};
B.disabled = () => new World();
B.entitySet = () => { const w = B.disabled(); w.ok('owner', C.entity()); return w; };
B.enabled = () => { const w = B.entitySet(); w.ok('owner', C.enable()); for (const who of ['F1', 'F2', 'FIN1', 'FIN2', 'MON1', 'MGR1', 'BE1', 'BE2', 'RT1']) w.ok(who, C.clock('IN')); return w; };
B.safeProposed = () => { const w = B.enabled(); w.ok('FIN1', propose('SAFE_OPEN', { requestId: w.rq(), counted: sheet(SAFE) })); return w; };
B.safeOpened = () => { const w = B.safeProposed(), id = w.lastRequest(); w.ok('MON1', C.sign(id)); w.ok('BE1', C.commit(id)); w.ok('FIN1', C.target()); return w; };
B.openProposed = () => { const w = B.safeOpened(); w.ok('F1', propose('OPEN', { requestId: w.rq(), counted: sheet(DRAWER) })); return w; };
B.ready = () => { const w = B.openProposed(); w.ok('BE1', C.commit(w.lastRequest())); return w; };
B.open = () => { const w = B.ready(); w.ok('BE1', C.start(w)); return w; };
B.cutOff = () => { const w = B.open(); w.ok('BE1', C.cutoff()); return w; };
B.closeProposed = () => { const w = B.cutOff(); w.ok('F1', propose('CLOSE', { requestId: w.rq(), counted: sheet(DRAWER), safeCount: sheet(SAFE) })); return w; };
B.handoverProposed = () => { const w = B.cutOff(); w.ok('F1', propose('HANDOVER', { requestId: w.rq(), receiver: 'F2', counted: sheet(DRAWER), safeCount: sheet(SAFE) })); return w; };
B.transferProposed = (kind = 'SAFE_IN') => { const w = B.open(); w.ok('F1', propose(kind, { requestId: w.rq(), receiver: 'FIN1', currency: 'USD', amount: '200' })); return w; };
B.inTransit = (kind = 'SAFE_IN') => { const w = B.transferProposed(kind); w.ok('BE1', C.dispatch(w.lastRequest())); return w; };
B.closed = () => { const w = B.closeProposed(), id = w.lastRequest(); w.ok('MGR1', C.sign(id)); w.ok('MON1', C.sign(id)); w.ok('BE1', C.commit(id)); return w; };

// ── A. every command by every role from every state ─────────────────────────
const bogus = idAt(0xdead);
const probeCommands = (w) => {
  const id = w.lastRequest() ?? bogus, hold = w.ops.holds?.at(-1)?.id ?? bogus;
  return [
    C.entity(), C.enable(), C.clock('IN'), C.clock('OUT'), C.begin(), C.target(), C.start(w), C.cutoff(),
    propose('OPEN', { requestId: RQ(900), counted: sheet(DRAWER) }),
    propose('HANDOVER', { requestId: RQ(901), receiver: 'F2', counted: sheet(DRAWER), safeCount: sheet(SAFE) }),
    propose('CLOSE', { requestId: RQ(902), counted: sheet(DRAWER), safeCount: sheet(SAFE) }),
    propose('SAFE_OPEN', { requestId: RQ(903), counted: sheet(SAFE) }),
    propose('SAFE_IN', { requestId: RQ(904), receiver: 'FIN1', currency: 'USD', amount: '200' }),
    propose('SAFE_OUT', { requestId: RQ(905), receiver: 'FIN1', currency: 'USD', amount: '200' }),
    propose('EXTERNAL_IN', { requestId: RQ(906), receiver: 'MGR1', currency: 'THB', amount: '1000' }),
    propose('EXTERNAL_OUT', { requestId: RQ(907), receiver: 'MGR1', currency: 'THB', amount: '1000' }),
    C.hold(), C.release(hold), C.reject(id), C.sign(id), C.dispatch(id), C.commit(id), C.reconcile(id),
  ];
};
const probeActors = ['owner', 'F1', 'F2', 'F3', 'FIN1', 'FIN2', 'MON1', 'MGR1', 'BE1', 'RT1'];
const matrix = [];
for (const [state, build] of Object.entries({
  disabled: B.disabled, entitySet: B.entitySet, enabled: B.enabled, safeProposed: B.safeProposed, safeOpened: B.safeOpened, openProposed: B.openProposed,
  ready: B.ready, open: B.open, cutOff: B.cutOff, closeProposed: B.closeProposed, handoverProposed: B.handoverProposed,
  safeInProposed: () => B.transferProposed('SAFE_IN'), safeInTransit: () => B.inTransit('SAFE_IN'), safeOutTransit: () => B.inTransit('SAFE_OUT'), closed: B.closed,
})) {
  const w = build(); w.ops.holds?.length || (w.ops.enabled && w.ok('MON1', C.hold()));
  // the facts and the commands are the same for every probe of a state, so they are written once
  const commands = probeCommands(w).map((c) => w.hashed(c)), probes = [];
  const facts = { ...w.facts(commands[0]), customerKnown: true };
  for (const who of probeActors) commands.forEach((cmd, n) => {
    const r = w.run(who, cmd, { keep: false }), { newId, ...rest } = r.facts;
    if (JSON.stringify({ ...rest, customerKnown: true }) !== JSON.stringify(facts)) throw Error('probe facts differ');
    probes.push(r.error ? { actor: who, command: n, newId, error: r.error } : { actor: who, command: n, newId, events: r.events, effects: r.effects, id: r.id, changed: r.changed });
  });
  matrix.push({ state, phase: w.ops.phase, now: w.clock + 60000, facts, commands, steps: w.steps, probes });
}

// ── B. stories: the paths and refusals a matrix of single commands cannot reach ─
const stories = [];
const story = (name, f) => { const w = f(); stories.push({ name, steps: w.steps }); };

story('a whole day: open, serve, hand over with a shortage, close, reconcile', () => {
  const w = B.open();
  w.ok('F1', C.begin({ type: 'SELL', currency: 'EUR', amount: '50.5' }));
  w.sale(100); // 100 USD bought for cash: expected is now THB 46790, USD 1100
  w.ok('BE1', C.cutoff());
  const short = { THB: '46780', USD: '1100' };
  w.no('F1', propose('HANDOVER', { requestId: RQ(11), receiver: 'F2', counted: sheet(short), safeCount: sheet(SAFE) })); // a shortage with no note
  w.ok('F1', propose('HANDOVER', { requestId: RQ(11), receiver: 'F2', note: '  ten baht short, recounted twice ', counted: sheet(short), safeCount: sheet(SAFE) }));
  const h = w.lastRequest();
  w.no('BE1', C.commit(h)); // no manager yet
  w.ok('MGR1', C.sign(h));
  w.no('BE1', C.commit(h)); // no monitor yet
  w.ok('MON1', C.sign(h));
  w.no('BE1', C.commit(h)); // the receiver has not signed
  w.ok('F1', C.sign(h), { note: 'the sender signed when proposing: nothing happens, no event' });
  w.ok('F2', C.sign(h));
  w.ok('F2', C.sign(h)); // twice: nothing
  w.ok('BE1', C.commit(h));
  w.ok('BE1', C.commit(h)); // again: nothing
  w.no('MON1', C.reject(h)); // finished
  w.no('F1', C.begin()); // no longer the custodian
  w.ok('F1', C.clock('OUT'));
  w.ok('F2', C.begin());
  w.no('FIN2', C.reconcile(h, '   '));
  w.ok('FIN2', C.reconcile(h));
  w.ok('FIN2', C.reconcile(h, 'second look')); // allowed again, a second event
  w.ok('BE1', C.cutoff());
  w.ok('F2', propose('CLOSE', { requestId: RQ(12), counted: sheet(short), safeCount: sheet(SAFE) })); // S2 opened with the short count: no variance now
  const c = w.lastRequest();
  w.ok('MGR1', C.sign(c)); w.ok('MON1', C.sign(c));
  w.ok('BE2', C.commit(c));
  w.no('BE1', C.start(w)); // closed
  return w;
});

story('the four transfers', () => {
  const w = B.open();
  for (const [kind, receiver, currency, amount, note] of [['SAFE_IN', 'FIN1', 'USD', '200', ''], ['SAFE_OUT', 'FIN1', 'EUR', '300', 'float for the afternoon'], ['EXTERNAL_IN', 'MGR1', 'THB', '10000', ''], ['EXTERNAL_OUT', 'F2', 'THB', '5000.25', 'to the bank']]) {
    w.ok('F1', propose(kind, { requestId: w.rq(), receiver, currency, amount, note }));
    const id = w.lastRequest();
    w.no(receiver, C.sign(id)); // the receiver waits for the dispatch
    w.no('BE1', C.commit(id)); // nobody has signed
    w.ok('FIN2', C.sign(id));
    w.no('BE1', C.commit(id)); // the receiver has not signed (or, for FIN1, not yet)
    w.ok('BE1', C.dispatch(id));
    w.ok('BE2', C.dispatch(id)); // again: nothing
    w.no('MON1', C.reject(id)); // money is on its way
    w.no('F1', C.clock('OUT'));
    w.no(receiver, C.clock('OUT')); // party to a pending request
    w.ok(receiver, C.sign(id));
    w.ok('BE1', C.commit(id));
    w.no('BE1', C.dispatch(id)); // finished
    w.ok('FIN2', C.reconcile(id));
  }
  return w;
});

story('transfers that are refused', () => {
  const w = B.open();
  const t = (o) => propose('SAFE_OUT', { requestId: RQ(31), receiver: 'FIN1', currency: 'USD', amount: '200', ...o });
  w.no('F1', t({ amount: null }));
  w.no('F1', t({ amount: '0' }));
  w.no('F1', t({ currency: 'CHF' })); // registered, but not one of the seven counted currencies
  w.no('F1', t({ currency: null }));
  w.no('F1', t({ receiver: null }));
  w.no('F1', t({ receiver: 'F1' }));
  w.no('F2', t()); // a Front who does not hold the drawer
  w.no('FIN1', t()); // Finance proposes only SAFE_OPEN
  w.ok('F1', t({ amount: '5000.01' })); // more than the safe holds: accepted now …
  const id = w.lastRequest();
  w.no('BE1', C.dispatch(id)); // … refused at the dispatch
  w.no('F1', t({ requestId: RQ(32) })); // one pending request at a time
  w.ok('BE1', C.reject(id, '  the safe has 5000  '));
  w.no('BE1', C.reject(id)); w.no('MON1', C.sign(id)); w.no('BE1', C.dispatch(id)); w.no('BE1', C.commit(id)); w.no('FIN2', C.reconcile(id));
  w.ok('F1', t({ requestId: RQ(32), amount: '5000' })); // exactly what the safe holds
  const all = w.lastRequest();
  w.ok('BE1', C.dispatch(all));
  w.ok('FIN1', C.sign(all));
  w.ok('BE1', C.commit(all));
  // the shift changes under a pending transfer
  w.ok('F1', propose('EXTERNAL_IN', { requestId: RQ(33), receiver: 'MGR1', currency: 'THB', amount: '100' }));
  const late = w.lastRequest();
  w.active().status = 'CLOSED'; w.model.stock.shifts.push({ id: 'S9', status: 'OPEN', opening: {} });
  w.no('BE1', C.dispatch(late), { note: 'another shift is open now' });
  w.active().status = 'CLOSED';
  w.no('BE1', C.dispatch(late), { note: 'no shift is open now' });
  return w;
});

story('the same request sent again, and a changed one', () => {
  const w = B.safeOpened();
  const first = propose('OPEN', { requestId: RQ(41), counted: sheet(DRAWER) });
  w.ok('F1', first);
  w.ok('F1', first); // the same again: the same id, no event
  w.ok('F2', { ...first, payloadHash: fingerprint(w.payload(first)) }, { note: 'v2 answers a repeat to any Front or Finance, not only to the sender' });
  w.no('F1', propose('OPEN', { requestId: RQ(41), counted: sheet(DRAWER), note: 'changed' }));
  w.no('MON1', first); // the role is checked before the repeat is recognised
  w.no('F1', propose('OPEN', { requestId: 'not-a-request-id', counted: sheet(DRAWER) }));
  w.no('F1', propose('OPEN', { requestId: RQ(42).toUpperCase(), counted: sheet(DRAWER) }));
  w.no('F1', propose('OPEN', { requestId: RQ(42), counted: sheet(DRAWER), evidenceKeys: [] }));
  w.no('F1', propose('OPEN', { requestId: RQ(42), counted: sheet(DRAWER), evidenceKeys: [EV(1), EV(2), EV(3), EV(4), EV(5), EV(6)] }));
  w.no('F1', propose('OPEN', { requestId: RQ(42), counted: sheet(DRAWER), evidenceKeys: [EV(1), 'slip.jpg'] }));
  w.ok('BE1', C.reject(w.lastRequest()));
  w.ok('F1', first, { note: 'a rejected request still answers its request id' });
  w.ok('F1', propose('OPEN', { requestId: RQ(42), counted: sheet(DRAWER), evidenceKeys: [EV(1), EV(2), EV(3), EV(4), EV(5)] }));
  return w;
});

story('counts that are refused', () => {
  const w = B.safeOpened();
  const open = (o) => propose('OPEN', { requestId: RQ(51), counted: sheet(DRAWER), ...o });
  w.no('F1', open({ counted: sheet(DRAWER, []) }));
  w.no('F1', open({ counted: sheet(DRAWER, Array.from({ length: 201 }, () => ({ currency: 'THB', denomination: '1', count: 1 }))) }));
  w.no('F1', open({ counted: sheet(DRAWER, [{ currency: 'CHF', denomination: '100', count: 1 }]) }));
  w.no('F1', open({ counted: sheet(DRAWER, [{ currency: 'THB', denomination: '0', count: 1 }]) }));
  w.no('F1', open({ counted: sheet(DRAWER, [{ currency: 'THB', denomination: '100000.01', count: 1 }]) }));
  w.no('F1', open({ counted: sheet(DRAWER, [{ currency: 'THB', denomination: '1', count: 1000001 }]) }));
  w.no('F1', open({ counted: sheet(DRAWER, [{ currency: 'THB', denomination: '1000', count: 50 }]) })); // USD rows missing
  w.no('F1', open({ counted: sheet(DRAWER, [{ currency: 'USD', denomination: '100', count: 10 }, { currency: 'THB', denomination: '1000', count: 49 }]) })); // THB named first
  w.no('F1', open({ counted: sheet({ THB: '50000', USD: '1000', CHF: '70' }), note: '' }), { note: 'CHF is not one of the seven: no rows are asked for it, and it counts as a difference from the target' });
  w.no('F1', open({ counted: sheet({ THB: '50000', USD: '999.99' }) })); // differs from the target, no note
  w.no('F1', open({ note: 'x'.repeat(501) }));
  w.no('F1', open({ counted: sheet({ THB: '50000', USD: '999.99' }), note: ' '.repeat(5) })); // a blank note is no note
  w.model.rates = [{ code: 'USD', buy: 32.1, sell: 0 }];
  w.no('F1', open(), { note: 'no currency has both rates' });
  w.model.rates = [{ code: 'USD', buy: 32.1, sell: 32.6 }];
  w.ok('F1', open({ counted: sheet({ THB: '50000', USD: '1000', CHF: '70', JPY: '0' }, [{ currency: 'THB', denomination: '1000', count: 50 }, { currency: 'USD', denomination: '100', count: 10 }, { currency: 'JPY', denomination: '1000', count: 0 }]), note: ' x'.repeat(250) }));
  const id = w.lastRequest();
  w.ok('BE1', C.commit(id), { note: 'an OPEN request needs no signature but the Backend\'s; the target difference becomes an exception' });
  w.no('F1', propose('OPEN', { requestId: RQ(52), counted: sheet(DRAWER) })); // a shift is open
  w.no('FIN1', propose('SAFE_OPEN', { requestId: RQ(53), counted: sheet(SAFE) })); // the safe has its opening count
  w.ok('FIN1', C.reconcile(id));
  return w;
});

story('opening the safe', () => {
  const w = B.enabled();
  w.no('F1', propose('SAFE_OPEN', { requestId: RQ(61), counted: sheet(SAFE) }));
  w.no('F1', propose('SAFE_IN', { requestId: RQ(61), receiver: 'FIN1', currency: 'USD', amount: '1' })); // not the custodian: nobody is
  w.no('F1', propose('OPEN', { requestId: RQ(61), counted: sheet(DRAWER) })); // no target yet
  w.ok('FIN1', propose('SAFE_OPEN', { requestId: RQ(61), counted: sheet(SAFE) }));
  const id = w.lastRequest();
  w.no('BE1', C.commit(id)); // waits for Monitor
  w.no('FIN2', C.sign(id)); w.no('F2', C.sign(id)); w.no('BE1', C.sign(id)); w.no('RT1', C.sign(id));
  w.ok('MGR1', C.sign(id)); // a manager may sign anything; it does not replace Monitor
  w.no('BE1', C.commit(id));
  w.ok('MON1', C.sign(id));
  w.no('FIN1', C.clock('OUT')); // the sender of a pending request
  w.ok('BE1', C.commit(id));
  w.ok('FIN1', C.clock('OUT'));
  w.no('FIN1', C.target()); // clocked out
  w.ok('FIN1', C.clock('IN'));
  w.ok('FIN1', C.clock('IN')); // already in: nothing
  w.no('FIN1', C.reconcile(id)); // the sender
  w.ok('FIN2', C.target({ THB: '100000000', ZWG: '0.01' }));
  return w;
});

story('service hours', () => {
  const w = B.ready();
  w.no('BE1', C.start(w), { at: bangkok('10:59') });
  w.no('BE1', C.start(w), { at: bangkok('22:50') });
  w.no('BE1', C.start(w, { equipmentReady: false }));
  w.no('BE1', C.start(w, { ratesVerified: false }));
  w.no('BE1', C.start(w, { rateUpdated: '2026-10-05T03:00:00.000Z' }));
  w.no('F1', C.begin()); // prepared, not yet serving
  w.ok('BE1', C.start(w), { at: bangkok('11:00') });
  w.no('BE1', C.start(w)); // already serving
  w.no('F1', C.begin(), { at: bangkok('10:59') });
  w.ok('F1', C.begin(), { at: bangkok('11:00') });
  w.ok('F1', C.begin({ type: 'SELL', currency: 'ZWG', amount: '0.01' }), { at: bangkok('22:49') });
  w.no('F1', C.begin(), { at: bangkok('22:50') });
  w.no('F1', C.begin(), { at: bangkok('23:59') });
  w.no('F1', C.begin(), { at: bangkok('00:00') + 86400000 });
  w.no('F1', C.begin({ currency: 'THB' }));
  w.no('F1', C.begin({ amount: '0' }));
  w.no('F2', C.begin());
  w.active().status = 'CLOSED';
  w.no('F1', C.begin(), { expect: 'ยังไม่มีกะเปิด', note: 'v2 reads the id of a shift that is not there and crashes; v3 refuses' });
  w.no('BE1', C.cutoff());
  return w;
});

story('cut-off, counting, sending back', () => {
  const w = B.ready();
  w.ok('BE1', C.cutoff(), { note: 'a cut-off is accepted before the service was ever opened' });
  w.no('BE1', C.start(w)); // no way back to serving from a cut-off
  w.model.transactions.push({ id: 'XC-9', shiftId: 'S1', status: 'PENDING', type: 'BUY', currency: 'USD', amount: 1, rate: 1, fee: 0, method: 'Cash' });
  w.ok('BE2', C.cutoff(), { note: 'cut off again: the record is replaced and lists the unfinished transaction' });
  const close = (o) => propose('CLOSE', { requestId: RQ(71), counted: sheet(DRAWER), safeCount: sheet(SAFE), ...o });
  w.no('F1', close());
  w.model.transactions.pop();
  w.no('F1', close({ safeCount: sheet(SAFE, []) }));
  w.no('F1', close({ safeCount: sheet({ ...SAFE, EUR: '299' }) })); // the safe is short, no note
  w.no('F1', propose('HANDOVER', { requestId: RQ(71), counted: sheet(DRAWER), safeCount: sheet(SAFE) }));
  w.no('F1', propose('HANDOVER', { requestId: RQ(71), receiver: 'F1', counted: sheet(DRAWER), safeCount: sheet(SAFE) }));
  w.ok('F1', close({ safeCount: sheet({ ...SAFE, EUR: '299' }), note: 'one euro missing from the safe' }));
  const id = w.lastRequest();
  w.no('BE1', C.cutoff()); // counting
  w.no('F1', propose('SAFE_IN', { requestId: RQ(72), receiver: 'FIN1', currency: 'USD', amount: '1' }));
  w.no('MON1', C.reject(id, ''));
  w.ok('MON1', C.reject(id, 'r'.repeat(600))); // back to the cut-off; the reason is kept to 500
  w.ok('F1', close({ requestId: RQ(73), safeCount: sheet({ ...SAFE, EUR: '299' }), note: 'one euro missing from the safe' }));
  const again = w.lastRequest();
  w.ok('MGR1', C.sign(again)); w.ok('MON1', C.sign(again));
  w.sale(10); // the drawer changed after the count
  w.no('BE1', C.commit(again));
  w.model.transactions.pop();
  w.active().status = 'CLOSED'; w.model.stock.shifts.push({ id: 'S9', status: 'OPEN', opening: numbers(DRAWER) });
  w.no('BE1', C.commit(again), { note: 'another shift is open now' });
  w.model.stock.shifts.pop(); w.model.stock.shifts[0].status = 'OPEN';
  w.ok('BE1', C.commit(again));
  w.ok('FIN1', C.reconcile(again));
  return w;
});

story('holding a customer', () => {
  const w = B.open();
  w.no('F1', C.hold({ customerId: 'CX-00000404' }));
  w.no('F1', C.hold({ reason: '   ' }));
  w.no('FIN1', C.hold());
  w.ok('F1', C.hold({ reason: '  ' + 'ก'.repeat(600) }));
  w.ok('MON1', C.hold()); // a second hold on the same customer is another record
  const [a, b] = w.ops.holds.map((h) => h.id);
  w.no('F1', C.release(a));
  w.no('MON1', C.release(a, ' '));
  w.no('MON1', C.release(bogus));
  w.ok('MON1', C.release(a, ' ok ' + 'n'.repeat(600)));
  w.no('MON1', C.release(a));
  w.ok('MON1', C.release(b));
  return w;
});

story('setting up', () => {
  const w = B.disabled();
  w.no('owner', C.enable());
  w.no('owner', C.entity({ company: '   ' }));
  w.no('owner', C.entity({ site: '' }));
  w.no('owner', C.entity({ confirmSeparated: false }));
  w.no('owner', C.entity({ company: 'c'.repeat(151) }));
  w.no('owner', C.entity({ site: 's'.repeat(101) }));
  w.ok('owner', C.entity({ company: '  ' + 'c'.repeat(150) + ' ', site: 's'.repeat(100) }));
  w.ok('owner', C.entity()); // may be set again while custody is off
  w.model.stock.shifts.push({ id: 'S0', status: 'OPEN', opening: {} });
  w.no('owner', C.entity()); w.no('owner', C.enable());
  w.model.stock.shifts.pop();
  w.no('F1', C.clock('IN'));
  w.ok('owner', C.enable());
  w.ok('owner', C.enable()); // again: accepted, a second event
  w.no('owner', C.entity());
  w.no('owner', C.clock('IN'));
  w.no('F1', C.clock('OUT'));
  w.ok('F1', C.clock('IN'));
  w.ok('F1', C.clock('OUT'));
  w.ok('F1', C.clock('IN')); // a second attendance record
  return w;
});

story('before the safe has its opening count', () => {
  const w = B.enabled();
  w.ok('FIN1', C.target());
  w.ok('F1', propose('OPEN', { requestId: w.rq(), counted: sheet(DRAWER) }));
  const open = w.lastRequest();
  w.model.stock.shifts.push({ id: 'S0', status: 'OPEN', opening: {} });
  w.no('BE1', C.commit(open), { note: 'a shift was opened some other way since the request' });
  w.model.stock.shifts.pop();
  w.ok('BE1', C.commit(open));
  w.ok('BE1', C.start(w));
  w.no('F1', propose('SAFE_IN', { requestId: w.rq(), receiver: 'FIN1', currency: 'USD', amount: '1' }));
  w.no('F1', propose('SAFE_OUT', { requestId: w.rq(), receiver: 'FIN1', currency: 'USD', amount: '1' }));
  w.ok('F1', propose('EXTERNAL_OUT', { requestId: w.rq(), receiver: 'BE1', currency: 'USD', amount: '1' })); // needs no safe
  const out = w.lastRequest();
  w.ok('FIN1', C.sign(out));
  w.ok('BE1', C.dispatch(out), { note: 'v2 lets the receiving Backend record the dispatch' });
  w.no('BE1', C.sign(out));
  w.no('BE1', C.commit(out)); // a party to the transfer
  w.no('BE2', C.commit(out)); // and nobody else can: the receiver cannot sign
  w.ok('BE2', C.cutoff());
  w.no('F1', propose('CLOSE', { requestId: w.rq(), counted: sheet(DRAWER), safeCount: sheet(SAFE) })); // the transfer is still pending
  return w;
});

story('closing with no safe count on record', () => {
  const w = B.enabled();
  w.ok('FIN1', C.target({ THB: '50000' }));
  w.ok('F1', propose('OPEN', { requestId: w.rq(), counted: sheet({ THB: '50000' }) }));
  w.ok('BE1', C.commit(w.lastRequest()));
  w.ok('BE1', C.cutoff());
  w.no('F1', propose('CLOSE', { requestId: w.rq(), counted: sheet({ THB: '50000' }), safeCount: sheet(SAFE) }));
  w.no('F1', propose('HANDOVER', { requestId: w.rq(), receiver: 'F2', counted: sheet({ THB: '50000' }), safeCount: sheet(SAFE) }));
  return w;
});

const all = [...matrix.flatMap((m) => [...m.steps, ...m.probes]), ...stories.flatMap((s) => s.steps)];
const count = (f) => all.filter(f).length;
// every action must have been accepted somewhere and refused somewhere
for (const a of ['opsSetEntity', 'opsEnable', 'opsClock', 'opsBeginTransaction', 'opsTarget', 'opsStartService', 'opsCutoff', 'opsPropose', 'opsHold', 'opsRelease', 'opsReject', 'opsSign', 'opsDispatch', 'opsCommit', 'opsReconcile'])
  for (const refused of [false, true]) if (!stories.some((s) => s.steps.some((r) => r.command.action === a && !!r.error === refused)) && !matrix.some((m) => m.probes.some((r) => m.commands[r.command].action === a && !!r.error === refused))) throw Error(a + (refused ? ' is never refused' : ' is never accepted'));
const counts = {
  matrixStates: matrix.length, probes: matrix.reduce((n, m) => n + m.probes.length, 0), probesAccepted: matrix.reduce((n, m) => n + m.probes.filter((p) => !p.error).length, 0),
  stories: stories.length, storySteps: stories.reduce((n, s) => n + s.steps.length, 0), storyStepsRefused: stories.reduce((n, s) => n + s.steps.filter((p) => p.error).length, 0),
  distinctErrors: new Set(all.filter((r) => r.error).map((r) => r.error)).size, v2DiffersOnPurpose: count((r) => r.v2DiffersOnPurpose),
};
console.error(Object.entries(counts).map(([k, v]) => `${k}=${v}`).join(' '));
const lines = (list) => '[\n' + list.map((x) => '   ' + JSON.stringify(x)).join(',\n') + '\n  ]';
console.log('{\n "source": ' + JSON.stringify('v2 engine extracted unchanged from the deployed bundle (xchange-online@3ffc5a5): runOperations (server.mjs:5969-6251) with XCStock, driven by gen_custody.mjs; randomUUID replaced by a counter') + ',\n "counts": ' + JSON.stringify(counts)
  + ',\n "matrix": [\n' + matrix.map((m) => '  {"state": ' + JSON.stringify(m.state) + ', "phase": ' + JSON.stringify(m.phase) + ', "now": ' + m.now + ',\n  "facts": ' + JSON.stringify(m.facts) + ',\n  "commands": ' + lines(m.commands) + ',\n  "steps": ' + lines(m.steps) + ',\n  "probes": ' + lines(m.probes) + '}').join(',\n')
  + '\n ],\n "stories": [\n' + stories.map((s) => '  {"name": ' + JSON.stringify(s.name) + ',\n  "steps": ' + lines(s.steps) + '}').join(',\n') + '\n ]\n}');
