// Drives the shift actions of the v2 engine (`adminOpenShift`, `adminMoveStock`,
// `adminCloseShift`, `adminAbandonRequest`, extracted unchanged from the deployed
// bundle) through whole shifts and records, step by step, what v2 answered and
// what the branch held afterwards. Every figure is checked against the same sum
// in exact decimal arithmetic; the script stops if v2's float disagrees.
//   node gen_shift.mjs > ../../crates/xc-shift/tests/fixtures/v2_shift_cases.json
import fs from 'node:fs';
import { runEngine } from './engine.mjs';

const src = fs.readFileSync(new URL('./engine.mjs', import.meta.url), 'utf8');
const start = src.indexOf('var XCStock = /* @__PURE__ */ (() => {');
const end = src.indexOf('const ADMIN_SCHEMA', start);
if (start < 0 || end < 0) throw Error('XCStock not found in engine.mjs');
const XCStock = new Function(src.slice(start, end) + '\nreturn XCStock;')();

const actor = { id: 'owner', email: 'owner' };
const toMinor = (s, dp = 2) => { const [a, b = ''] = String(s).split('.'); return BigInt(a + (b + '0'.repeat(dp)).slice(0, dp)); };
const fmt = (x) => { const neg = x < 0n; const s = (neg ? -x : x).toString().padStart(3, '0'); return (neg ? '-' : '') + s.slice(0, -2) + '.' + s.slice(-2); };
const exactPrincipal = (amount, rate) => (toMinor(amount, 2) * toMinor(rate, 4) + 5000n) / 10000n;
const v2Minor = (n) => BigInt(Math.round(Number(n) * 100));
// non-zero entries, in the register's order (THB first), two decimals
const shown = (obj) => Object.fromEntries(XCStock.currencies.filter((c) => obj && v2Minor(obj[c] ?? 0) !== 0n).map((c) => [c, fmt(v2Minor(obj[c]))]));
const shownExact = (map) => Object.fromEntries(XCStock.currencies.filter((c) => (map[c] ?? 0n) !== 0n).map((c) => [c, fmt(map[c])]));

let idCounter = 0;
// request ids are fixed text so the file is the same on every run
const rid = (label) => `req-${label}-${String(++idCounter).padStart(4, '0')}-0000000000`;

class Branch {
  constructor() {
    this.tables = {}; this.props = {};
    this.shiftIds = [];           // v2 ids, in the order the shifts were opened
    this.exact = null;            // exact ledger of the open shift, minor units
    const r = this.run('adminSnapshot', {});
    if (r.error) throw Error('init: ' + r.error);
  }
  run(action, payload) {
    const r = runEngine(action, payload, this.tables, this.props, actor, {}, null);
    this.tables = r.tables; this.props = r.properties;
    return r;
  }
  model() { const r = this.run('adminSnapshot', {}); if (r.error) throw Error('snapshot: ' + r.error); return r.result; }
  // "$SHIFT:n" in a payload stands for the id of the n-th shift opened in the scenario
  resolve(payload) {
    const p = structuredClone(payload);
    if (typeof p.shiftId === 'string' && p.shiftId.startsWith('$SHIFT:')) {
      const id = this.shiftIds[Number(p.shiftId.slice(7))];
      if (!id) throw Error('no such shift for ' + p.shiftId);
      p.shiftId = id;
    }
    return p;
  }
  state() {
    const m = this.model(), shifts = m.stock.shifts, active = XCStock.active(shifts);
    const index = (id) => this.shiftIds.indexOf(id);
    const out = {
      shifts: shifts.map((s) => ({
        status: s.status, opening: shown(s.opening),
        expected: s.status === 'OPEN' ? shown(XCStock.expected(s, m.transactions, m.stock.movements)) : shown(s.expected),
        counted: s.counted ? shown(s.counted) : null, difference: s.difference ? shown(s.difference) : null, note: s.note || '',
      })),
      movements: m.stock.movements.map((x) => ({ shift: index(x.shiftId), currency: x.currency, direction: x.direction, amount: fmt(v2Minor(x.amount)), reason: x.reason })),
    };
    if (active) {
      const want = shownExact(this.exact);
      if (JSON.stringify(out.shifts.at(-1).expected) !== JSON.stringify(want)) throw Error('v2 float differs from exact: ' + JSON.stringify([out.shifts.at(-1).expected, want]));
    }
    return out;
  }
  step(s) {
    const before = this.model();
    let error = null, extra = {};
    if (s.do === 'open' || s.do === 'move' || s.do === 'close') {
      const action = { open: 'adminOpenShift', move: 'adminMoveStock', close: 'adminCloseShift' }[s.do];
      const payload = this.resolve(s.payload);
      const r = this.run(action, payload);
      error = r.error ?? null;
      if (!error) {
        const after = r.result.stock;
        if (s.do === 'open' && after.shifts.length > before.stock.shifts.length) {
          this.shiftIds.push(after.shifts.at(-1).id);
          this.exact = Object.fromEntries(XCStock.currencies.map((c) => [c, toMinor(String(payload.opening?.[c] ?? 0).trim() || '0')]));
        }
        if (s.do === 'move' && after.movements.length > before.stock.movements.length) {
          const sign = payload.direction === 'IN' ? 1n : -1n;
          this.exact[payload.currency] += sign * toMinor(String(payload.amount).trim());
        }
        if (s.do === 'close' && !XCStock.active(after.shifts)) this.exact = null;
      }
    } else if (s.do === 'tx') {
      // what saveTransaction's stock check does before it writes a completed row
      const t = s.tx, shift = XCStock.active(before.stock.shifts);
      try {
        if (!shift) throw Error('กรุณาเปิดกะจากหน้าแอดมินก่อน');
        const balance = XCStock.expected(shift, before.transactions, before.stock.movements);
        XCStock.ensureStock(balance, XCStock.delta({ type: t.type, currency: t.currency, amount: Number(t.amount), rate: Number(t.rate), fee: Number(t.fee), method: t.method, status: 'COMPLETED' }));
        const row = Array(34).fill('');
        Object.assign(row, { 0: 'XC-TEST-' + (++idCounter), 1: 'SAVED', 2: new Date(), 3: new Date(), 4: new Date('2026-10-06T00:00:00+07:00'), 6: t.type, 7: t.currency, 8: Number(t.amount), 9: Number(t.rate), 10: 0, 11: Number(t.fee), 12: 0, 15: t.method });
        this.tables.Transactions.push(row);
        const p = exactPrincipal(t.amount, t.rate), f = toMinor(t.fee), a = toMinor(t.amount);
        this.exact[t.currency] += t.type === 'BUY' ? a : -a;
        if (t.method === 'Cash') this.exact.THB += t.type === 'SELL' ? p + f : -(p - f);
      } catch (e) { error = e.message; }
    } else if (s.do === 'abandon') {
      const r = this.run('adminAbandonRequest', { requestId: s.requestId });
      if (r.error) throw Error('abandon: ' + r.error);
      extra.resolution = r.result.requestResolution;
    } else throw Error('unknown step ' + s.do);
    return { ...s, ...extra, v2Error: error, after: this.state() };
  }
}

function scenario(name, steps) {
  const b = new Branch();
  return { name, steps: steps.map((s) => b.step(s)) };
}

const O = (requestId, opening) => ({ do: 'open', payload: { requestId, opening } });
const M = (requestId, shift, currency, direction, amount, reason = 'top up') => ({ do: 'move', payload: { requestId, shiftId: typeof shift === 'number' ? '$SHIFT:' + shift : shift, currency, direction, amount, reason } });
const C = (requestId, shift, counted, expected, note) => ({ do: 'close', payload: { requestId, shiftId: typeof shift === 'number' ? '$SHIFT:' + shift : shift, counted, expected, ...(note === undefined ? {} : { note }) } });
const T = (type, currency, amount, rate, fee, method) => ({ do: 'tx', tx: { type, currency, amount, rate, fee, method } });
const A = (requestId) => ({ do: 'abandon', requestId });
const NOBODY = 'SH-00000000-0000-4000-8000-000000000000';

const scenarios = [];
{
  const [o, m1, m2, c] = [rid('open'), rid('move'), rid('move'), rid('close')];
  scenarios.push(scenario('a whole shift: open, stock in and out, a purchase, a sale, count, close', [
    O(o, { THB: 500000, USD: 1000 }),
    M(m1, 0, 'USD', 'IN', 250.5, 'from the safe'),
    M(m2, 0, 'THB', 'OUT', 1000, 'petty cash'),
    T('BUY', 'USD', '100', '32.1', '0', 'Cash'),
    T('SELL', 'USD', '50', '32.6', '10', 'Bank Transfer'),
    T('SELL', 'USD', '19.95', '32.6', '25.5', 'Cash'),
    C(c, 0, { THB: 496465.87, USD: 1280.55 }, { THB: 496465.87, USD: 1280.55 }, ''),
  ]));
}
{
  const [o, o2, m, c] = [rid('open'), rid('open'), rid('move'), rid('close')];
  scenarios.push(scenario('the same request sent again is answered from what was saved; changed, it is refused', [
    O(o, { THB: 1000, USD: 500 }),
    O(o, { THB: 1000, USD: 500 }),
    O(o, { USD: 500, THB: '1000.00' }),
    O(o, { THB: 1000, USD: 500.01 }),
    O(o, { THB: 1000 }),
    M(m, 0, 'USD', 'OUT', 120.25, 'to the safe'),
    M(m, 0, 'USD', 'OUT', 120.25, 'to the safe'),
    M(m, 0, 'USD', 'OUT', '120.25', '  to the safe  '),
    M(m, 0, 'USD', 'OUT', 120.26, 'to the safe'),
    M(m, 0, 'USD', 'IN', 120.25, 'to the safe'),
    M(m, 0, 'EUR', 'OUT', 120.25, 'to the safe'),
    M(m, 0, 'USD', 'OUT', 120.25, 'to the vault'),
    M(m, NOBODY, 'USD', 'OUT', 120.25, 'to the safe'),
    C(c, 0, { THB: 1000, USD: 379.75 }, { THB: 1000, USD: 379.75 }, 'all there'),
    C(c, 0, { THB: 1000, USD: 379.75 }, { THB: 1000, USD: 379.75 }, 'all there'),
    C(c, 0, { THB: 1000, USD: 379.75 }, {}, ' all there '),
    C(c, 0, { THB: 1000, USD: 379.74 }, { THB: 1000, USD: 379.75 }, 'all there'),
    C(c, 0, { THB: 1000, USD: 379.75 }, { THB: 1000, USD: 379.75 }, 'all there!'),
    C(c, NOBODY, { THB: 1000, USD: 379.75 }, { THB: 1000, USD: 379.75 }, 'all there'),
    // the shift is closed; its earlier requests are still recognised
    O(o, { THB: 1000, USD: 500 }),
    M(m, 0, 'USD', 'OUT', 120.25, 'to the safe'),
    M(m, 0, 'USD', 'OUT', 1, 'to the safe'),
    // a new request naming the closed shift
    M(rid('move'), 0, 'USD', 'IN', 1),
    C(rid('close'), 0, { THB: 1000, USD: 379.75 }, { THB: 1000, USD: 379.75 }, 'again'),
    O(o2, { THB: 7 }),
    M(rid('move'), 0, 'THB', 'IN', 1),
    C(rid('close'), 0, { THB: 7 }, { THB: 7 }),
    M(rid('move'), 1, 'THB', 'IN', 1),
    M(m, 0, 'USD', 'OUT', 120.25, 'to the safe'),
    C(c, 0, { THB: 1000, USD: 379.75 }, { THB: 1000, USD: 379.75 }, 'all there'),
  ]));
}
{
  scenarios.push(scenario('what a branch refuses: a second shift, a stale shift id, stock it does not hold', [
    M(rid('move'), NOBODY, 'USD', 'IN', 5),
    C(rid('close'), NOBODY, {}, {}),
    T('BUY', 'USD', '1', '32.1', '0', 'Cash'),
    O(rid('open'), { THB: 3210, USD: 100, JPY: 0 }),
    O(rid('open'), { THB: 1 }),
    M(rid('move'), NOBODY, 'USD', 'IN', 5),
    M(rid('move'), 0, 'USD', 'OUT', 100.01),
    M(rid('move'), 0, 'EUR', 'OUT', 0.01),
    M(rid('move'), 0, 'THB', 'OUT', 3210.01),
    M(rid('move'), 0, 'USD', 'OUT', 100),
    M(rid('move'), 0, 'USD', 'OUT', 0.01),
    M(rid('move'), 0, 'USD', 'IN', 0.01),
    M(rid('move'), 0, 'USD', 'OUT', 0.01),
    M(rid('move'), 0, 'THB', 'OUT', 3210),
    M(rid('move'), 0, 'THB', 'IN', 100000000),
    M(rid('move'), 0, 'THB', 'IN', 100000000, 'a second full load'),
    // more than a person may type: a count is capped at 100,000,000 although a balance is not
    C(rid('close'), 0, { THB: 200000000 }, { THB: 200000000 }),
    C(rid('close'), 0, { THB: 100000000 }, { THB: 200000000 }),
    C(rid('close'), 0, { THB: 100000000 }, { THB: 200000000 }, 'the rest is in the vault'),
  ]));
}
{
  scenarios.push(scenario('closing: the count is checked against what the server expects now', [
    O(rid('open'), { THB: 10000, USD: 300, EUR: 40 }),
    M(rid('move'), 0, 'USD', 'OUT', 50),
    // counted against the figures from before the movement
    C(rid('close'), 0, { THB: 10000, USD: 300, EUR: 40 }, { THB: 10000, USD: 300, EUR: 40 }, 'ok'),
    C(rid('close'), 0, { THB: 10000, USD: 250, EUR: 40 }, {}, 'ok'),
    C(rid('close'), 0, { THB: 10000, USD: 250, EUR: 40 }, { THB: 10000, USD: 250 }, 'ok'),
    C(rid('close'), 0, { THB: 10000, USD: 250, EUR: 40 }, { THB: 10000, USD: 250, EUR: 40, JPY: 1 }, 'ok'),
    C(rid('close'), 0, { THB: 10000, USD: 250, EUR: 40 }, { THB: 10000, USD: 250, EUR: 40.01 }, 'ok'),
    // right figures, a difference, no reason given
    C(rid('close'), 0, { THB: 10000, USD: 249.5, EUR: 40 }, { THB: 10000, USD: 250, EUR: 40 }),
    C(rid('close'), 0, { THB: 10000, USD: 249.5, EUR: 40 }, { THB: 10000, USD: 250, EUR: 40 }, ''),
    C(rid('close'), 0, { THB: 10000, USD: 249.5, EUR: 40 }, { THB: 10000, USD: 250, EUR: 40 }, '   '),
    C(rid('close'), 0, { THB: 10000.5, USD: 250, EUR: 40, GBP: 3 }, { THB: 10000, USD: 250, EUR: 40 }, null),
    // expected sent as text and with explicit zeroes is the same expectation
    C(rid('close'), 0, { THB: 10000.5, USD: 249.5, GBP: 3 }, { THB: '10000.00', USD: '250', EUR: 40, JPY: 0, GBP: '0.00' }, '  short half a dollar, a stray note  '),
    C(rid('close'), 0, { THB: 10000 }, { THB: 10000 }, 'x'),
    O(rid('open'), { USD: 12.34 }),
    T('BUY', 'USD', '1', '32.1', '0', 'Bank Transfer'),
    C(rid('close'), 1, { USD: 13.34 }, { USD: 13.34 }),
    O(rid('open'), {}),
    C(rid('close'), 2, {}, {}),
    C(rid('close'), 2, {}, {}),
  ]));
}
{
  const [dead1, dead2, dead3, used] = [rid('dead'), rid('dead'), rid('dead'), rid('open')];
  scenarios.push(scenario('a request the screen gave up on can never be applied', [
    A(dead1),
    O(dead1, { THB: 100 }),
    O(used, { THB: 100, USD: 10 }),
    // already open, and the id is a cancelled one: the cancellation is what is reported
    O(dead1, { THB: 100 }),
    A(dead2),
    M(dead2, 0, 'USD', 'IN', 1),
    M(dead2, NOBODY, 'USD', 'OUT', 5000),
    A(dead3),
    C(dead3, 0, { THB: 100, USD: 10 }, { THB: 100, USD: 10 }),
    C(dead3, NOBODY, {}, {}),
    // giving up on a request that was applied changes nothing
    A(used),
    O(used, { THB: 100, USD: 10 }),
    M(rid('move'), 0, 'USD', 'IN', 1),
  ]));
}
{
  scenarios.push(scenario('transactions move the same balances the movements do', [
    O(rid('open'), { THB: 3210, USD: 100 }),
    T('BUY', 'USD', '100', '32.1', '0', 'Cash'),          // pays out exactly all the baht
    T('BUY', 'USD', '0.01', '32.1', '0', 'Cash'),         // 0.32 baht short
    T('BUY', 'USD', '0.01', '32.1', '0', 'PromptPay'),    // not cash: no baht leaves
    T('SELL', 'USD', '200.02', '32.6', '0', 'Cash'),      // one cent more than held
    T('SELL', 'USD', '200.01', '32.6', '12.5', 'Cash'),
    T('SELL', 'EUR', '1', '37.5055', '0', 'Cash'),
    T('BUY', 'JPY', '10000', '0.2195', '2195', 'Cash'),   // the fee eats the whole principal
    T('BUY', 'JPY', '10000', '0.2195', '5', 'Cash'),
    M(rid('move'), 0, 'JPY', 'OUT', 20000.01),
    M(rid('move'), 0, 'JPY', 'OUT', 20000),
    C(rid('close'), 0, { THB: 4342.83 }, { THB: 4342.83 }),
    T('SELL', 'USD', '1', '32.6', '0', 'Cash'),
    O(rid('open'), { USD: 5 }),
    T('SELL', 'USD', '5', '32.6', '0', 'Credit Card'),
    T('SELL', 'USD', '0.01', '32.6', '0', 'Credit Card'),
  ]));
}

// ── seeded walks: whatever comes next, v2 and the exact ledger must agree ───────
let seed = 20261007;
const rnd = (n) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return (seed >>> 12) % n; };
const pick = (a) => a[rnd(a.length)];
const codes = ['USD', 'EUR', 'JPY'];
for (let w = 0; w < 24; w++) {
  const b = new Branch(), steps = [];
  let open = false;
  for (let i = 0; i < 14; i++) {
    const k = rnd(10);
    let s;
    if (!open || k === 0) {
      const opening = { THB: pick([0, 5000, 250000.5]) };
      for (const c of codes) if (rnd(2)) opening[c] = pick([0, 100, 2500.75]);
      s = O(rid('open'), opening);
    } else if (k <= 4) {
      s = M(rid('move'), rnd(8) ? b.shiftIds.length - 1 : NOBODY, pick(['THB', ...codes]), pick(['IN', 'OUT']), pick([0.01, 100, 999.99, 5000]), pick(['count', 'safe', 'bank run']));
    } else if (k <= 7) {
      s = T(pick(['BUY', 'SELL']), pick(codes), pick(['1', '100', '99.99', '1234.56']), pick(['32.1', '0.2195', '37.5055']), pick(['0', '0', '10']), pick(['Cash', 'Cash', 'Alipay']));
      if (toMinor(s.tx.fee) > exactPrincipal(s.tx.amount, s.tx.rate)) s.tx.fee = '0';
    } else {
      const m = b.model(), shift = XCStock.active(m.stock.shifts);
      const expected = shift ? XCStock.expected(shift, m.transactions, m.stock.movements) : {};
      const exp = Object.fromEntries(Object.entries(expected).filter(([, v]) => v !== 0));
      const counted = { ...exp };
      const off = rnd(3) === 0;
      if (off) counted.THB = Math.round(((counted.THB ?? 0) + 1) * 100) / 100;
      s = C(rid('close'), b.shiftIds.length - 1, counted, rnd(5) ? exp : {}, off && rnd(2) ? '' : 'walk');
    }
    const done = b.step(s);
    steps.push(done);
    open = done.after.shifts.some((x) => x.status === 'OPEN');
  }
  scenarios.push({ name: 'seeded walk ' + (w + 1), steps });
}

// ── what a payload must look like before the branch is even consulted ──────────
// Each case is tried on a branch with one open shift; `parseError` is v2's
// sentence when the payload itself is refused, null when the payload is well formed.
const R = 'req-parse-0000-0000000000';
const parseCases = [];
const PARSE_SENTENCES = new Set([
  '\u0E02\u0E49\u0E2D\u0E21\u0E39\u0E25\u0E02\u0E49\u0E2D\u0E04\u0E27\u0E32\u0E21\u0E44\u0E21\u0E48\u0E04\u0E23\u0E1A\u0E2B\u0E23\u0E37\u0E2D\u0E22\u0E32\u0E27\u0E40\u0E01\u0E34\u0E19\u0E01\u0E33\u0E2B\u0E19\u0E14',
  '\u0E23\u0E2B\u0E31\u0E2A\u0E04\u0E33\u0E02\u0E2D\u0E44\u0E21\u0E48\u0E16\u0E39\u0E01\u0E15\u0E49\u0E2D\u0E07',
  '\u0E01\u0E23\u0E38\u0E13\u0E32\u0E01\u0E23\u0E2D\u0E01\u0E08\u0E33\u0E19\u0E27\u0E19\u0E40\u0E07\u0E34\u0E19',
  '\u0E08\u0E33\u0E19\u0E27\u0E19\u0E40\u0E07\u0E34\u0E19\u0E15\u0E49\u0E2D\u0E07\u0E44\u0E21\u0E48\u0E15\u0E34\u0E14\u0E25\u0E1A \u0E44\u0E21\u0E48\u0E40\u0E01\u0E34\u0E19 100,000,000 \u0E41\u0E25\u0E30\u0E21\u0E35\u0E17\u0E28\u0E19\u0E34\u0E22\u0E21\u0E44\u0E21\u0E48\u0E40\u0E01\u0E34\u0E19 2 \u0E15\u0E33\u0E41\u0E2B\u0E19\u0E48\u0E07',
  '\u0E23\u0E32\u0E22\u0E01\u0E32\u0E23\u0E23\u0E31\u0E1A\u2013\u0E08\u0E48\u0E32\u0E22\u0E44\u0E21\u0E48\u0E16\u0E39\u0E01\u0E15\u0E49\u0E2D\u0E07',
]);
const tryParse = (action, payload, wellFormed) => {
  const b = new Branch();
  const o = b.step(O('req-parse-open-0000000000', { THB: 1000, USD: 1000 }));
  if (o.v2Error) throw Error(o.v2Error);
  const fn = { open: 'adminOpenShift', move: 'adminMoveStock', close: 'adminCloseShift' }[action];
  const p = structuredClone(payload);
  if (p.shiftId === '$SHIFT:0') p.shiftId = b.shiftIds[0];
  const r = b.run(fn, p);
  // a well-formed open is refused later ("a shift is already open"): that is not a payload error
  const parseError = wellFormed ? null : r.error;
  if (!wellFormed && !PARSE_SENTENCES.has(r.error)) throw Error('not refused for its form: ' + JSON.stringify(payload) + ' -> ' + r.error);
  if (wellFormed && action !== 'open' && r.error) throw Error('v2 refused ' + JSON.stringify(payload) + ': ' + r.error);
  parseCases.push({ action, payload, parseError });
};
const goodMove = { requestId: R, shiftId: '$SHIFT:0', currency: 'USD', direction: 'IN', amount: 5, reason: 'r' };
const goodClose = { requestId: R, shiftId: '$SHIFT:0', counted: { THB: 1000, USD: 1000 }, expected: { THB: 1000, USD: 1000 }, note: '' };
const without = (o, k) => { const c = { ...o }; delete c[k]; return c; };
// request id
for (const requestId of [undefined, null, '', '   ', 'short-id-15char', 'has space in the id!', 'under_score_is_not_ok', 'x'.repeat(61), 'ไอดีภาษาไทยยาวสิบหกตัว']) {
  tryParse('open', requestId === undefined ? { opening: {} } : { requestId, opening: {} }, false);
  tryParse('move', requestId === undefined ? without(goodMove, 'requestId') : { ...goodMove, requestId }, false);
  tryParse('close', requestId === undefined ? without(goodClose, 'requestId') : { ...goodClose, requestId }, false);
}
for (const requestId of ['exactly-16-chars', 'x'.repeat(60), '  padded-with-spaces-0001  ', 'ABCdef0123456789-']) {
  tryParse('open', { requestId, opening: {} }, true);
  tryParse('move', { ...goodMove, requestId }, true);
  tryParse('close', { ...goodClose, requestId }, true);
}
// amounts in an opening and in a count
const badAmounts = [-1, -0.01, 100000000.01, 1.005, 0.001, 'abc', '', '12,5', '1.2.3', '-5'];
const goodAmounts = [0, 0.01, 100000000, '100000000', '0', ' 5 ', '7.5', '7.50', 1234.56, null];
for (const v of badAmounts) {
  tryParse('open', { requestId: R, opening: { USD: v } }, false);
  tryParse('close', { ...goodClose, counted: { THB: 1000, USD: v } }, false);
}
for (const v of goodAmounts) {
  tryParse('open', { requestId: R, opening: { USD: v } }, true);
  tryParse('close', { ...goodClose, counted: { THB: 1000, USD: v }, note: 'n' }, true);
}
tryParse('open', { requestId: R }, true);
tryParse('open', { requestId: R, opening: null }, true);
tryParse('open', { requestId: R, opening: { XXX: -5, usd: 'abc', THB: 1 } }, true);      // not in the register: not read
tryParse('open', { requestId: R, opening: { ZWG: '', THB: -1 } }, false);                // THB is read first
tryParse('open', { requestId: R, opening: { ZWG: -1, THB: '' } }, false);
tryParse('close', { ...goodClose, counted: undefined, note: 'n' }, true);
tryParse('close', { ...goodClose, counted: { ZWG: -1, AED: '' } }, false);               // AED is read before ZWG
// the register's order is not the alphabet's: USD is its second entry, AED its eighth
tryParse('open', { requestId: R, opening: { AED: -1, USD: '' } }, false);
tryParse('open', { requestId: R, opening: { AED: '', USD: -1 } }, false);
tryParse('close', { ...goodClose, counted: { AED: -1, USD: '' } }, false);
tryParse('close', { ...goodClose, counted: { CNY: '', JPY: 1.005 } }, false);            // JPY before CNY
// a movement, field by field, in the order v2 reads them
for (const amount of [...badAmounts, 0, '0', '0.00', null, undefined]) tryParse('move', amount === undefined ? without(goodMove, 'amount') : { ...goodMove, amount }, false);
for (const amount of [0.01, 100000000, '100000000', ' 5 ', '7.50', 1234.56]) tryParse('move', { ...goodMove, amount }, true);
for (const currency of [undefined, null, '', 'USDX', 'usd', 'US', 'XXX', '???']) tryParse('move', currency === undefined ? without(goodMove, 'currency') : { ...goodMove, currency }, false);
for (const currency of ['THB', ' USD ', 'ZWG']) tryParse('move', { ...goodMove, currency }, true);
for (const direction of [undefined, null, '', 'in', 'OUTX', 'BUY', 'I N']) tryParse('move', direction === undefined ? without(goodMove, 'direction') : { ...goodMove, direction }, false);
for (const direction of ['IN', 'OUT', ' IN ']) tryParse('move', { ...goodMove, direction, amount: 1 }, true);
for (const reason of [undefined, null, '', '   ', 'x'.repeat(201)]) tryParse('move', reason === undefined ? without(goodMove, 'reason') : { ...goodMove, reason }, false);
for (const reason of ['x'.repeat(200), '  ' + 'x'.repeat(200) + '  ', 'เติมเงินจากตู้เซฟ', '=SUM(A1)']) tryParse('move', { ...goodMove, reason }, true);
for (const shiftId of [undefined, null, '', '  ', 'x'.repeat(61)]) {
  tryParse('move', shiftId === undefined ? without(goodMove, 'shiftId') : { ...goodMove, shiftId }, false);
  tryParse('close', shiftId === undefined ? without(goodClose, 'shiftId') : { ...goodClose, shiftId }, false);
}
tryParse('close', { ...goodClose, note: 'x'.repeat(301) }, false);
tryParse('close', { ...goodClose, note: 'x'.repeat(300) }, true);
tryParse('close', without(goodClose, 'note'), true);
tryParse('close', { ...goodClose, note: null }, true);
// two things wrong at once: the first field v2 reads decides the sentence
tryParse('move', { ...goodMove, requestId: 'bad', currency: 'USDX', amount: -1 }, false);
tryParse('move', { ...goodMove, currency: 'USDX', amount: -1 }, false);
tryParse('move', { ...goodMove, currency: 'XXX', direction: 'OUTX' }, false);
tryParse('move', { ...goodMove, currency: 'XXX', amount: -1 }, false);
tryParse('move', { ...goodMove, direction: 'UP', amount: '' }, false);
tryParse('move', { ...goodMove, amount: -1, reason: '' }, false);
tryParse('move', { ...goodMove, reason: '', shiftId: '' , currency: 'XXX' }, false);
tryParse('close', { ...goodClose, counted: { USD: -1 }, shiftId: '', note: 'x'.repeat(301) }, false);
tryParse('close', { ...goodClose, shiftId: '', note: 'x'.repeat(301) }, false);
tryParse('open', { requestId: 'bad', opening: { USD: -1 } }, false);

const sentences = {};
for (const s of scenarios) for (const st of s.steps) if (st.v2Error) sentences[st.v2Error] = (sentences[st.v2Error] ?? 0) + 1;
for (const c of parseCases) if (c.parseError) sentences[c.parseError] = (sentences[c.parseError] ?? 0) + 1;

// one case and one step per line: the file stays reviewable in a diff
const out = [
  '{',
  ' "source": ' + JSON.stringify('xchange-online@3ffc5a5 — runEngine(adminOpenShift | adminMoveStock | adminCloseShift | adminAbandonRequest), XCStock.ensureStock') + ',',
  ' "register": ' + JSON.stringify(XCStock.currencies) + ',',
  ' "sentences": ' + JSON.stringify(sentences) + ',',
  ' "parseCases": [',
  parseCases.map((c) => '  ' + JSON.stringify(c)).join(',\n'),
  ' ],',
  ' "scenarios": [',
  scenarios.map((s) => '  {"name": ' + JSON.stringify(s.name) + ', "steps": [\n' + s.steps.map((st) => '   ' + JSON.stringify(st)).join(',\n') + '\n  ]}').join(',\n'),
  ' ]',
  '}',
];
process.stdout.write(out.join('\n') + '\n');
