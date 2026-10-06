// Drives the stock module of the v2 engine (`XCStock`, extracted unchanged from the
// deployed bundle) and records what it computed, beside the same figures in exact
// decimal arithmetic.
//   node gen_stock.mjs > ../../crates/xc-shift/tests/fixtures/v2_stock_cases.json
import fs from 'node:fs';

const src = fs.readFileSync(new URL('./engine.mjs', import.meta.url), 'utf8');
const start = src.indexOf('var XCStock = /* @__PURE__ */ (() => {');
const end = src.indexOf('const ADMIN_SCHEMA', start);
if (start < 0 || end < 0) throw Error('XCStock not found in engine.mjs');
const XCStock = new Function(src.slice(start, end) + '\nreturn XCStock;')();

const toMinor = (s, dp = 2) => { const [a, b = ''] = String(s).split('.'); return BigInt(a + (b + '0'.repeat(dp)).slice(0, dp)); };
const fmt = (x) => { const neg = x < 0n; const s = (neg ? -x : x).toString().padStart(3, '0'); return (neg ? '-' : '') + s.slice(0, -2) + '.' + s.slice(-2); };
const exactPrincipal = (amount, rate) => (toMinor(amount, 2) * toMinor(rate, 4) + 5000n) / 10000n;
const v2Minor = (n) => BigInt(Math.round(n * 100)); // v2 holds cents as a float; integers here
const nonZero = (obj, f) => Object.fromEntries(Object.entries(obj).filter(([, v]) => v !== 0).map(([k, v]) => [k, f(v)]));
// both sides are compared in the register's order (THB first), as v2 lists them
const ordered = (o) => Object.fromEntries(XCStock.currencies.filter((c) => c in o).map((c) => [c, o[c]]));
let differs = 0;

// ── A. the delta of one completed transaction ───────────────────────────────
const amounts = ['0.01', '1', '100', '100.5', '99.99', '1234.56', '0.07', '250000', '99999999.99', '100000000', '3.33', '7.77', '1000.01', '19.95'];
const rates = ['32.1', '32.6', '0.211', '0.2195', '43.9', '4.4567', '0.0001', '1', '37.5055', '21.3333'];
const deltas = [];
const pushDelta = (type, currency, amount, rate, fee, method) => {
  const v2 = XCStock.delta({ type, currency, amount: Number(amount), rate: Number(rate), fee: Number(fee), method, status: 'COMPLETED' });
  const p = exactPrincipal(amount, rate), f = toMinor(fee), a = toMinor(amount);
  const exact = { [currency]: fmt(type === 'BUY' ? a : -a) };
  if (method === 'Cash') exact.THB = fmt(type === 'SELL' ? p + f : -(p - f));
  if (exact.THB === '0.00') delete exact.THB;
  const v2s = nonZero(v2, (c) => fmt(BigInt(c))), exo = ordered(exact);
  const d = JSON.stringify(v2s) !== JSON.stringify(exo); if (d) differs++;
  deltas.push({ type, currency, amount, rate, fee, method, v2: v2s, exact: exo, v2DiffersFromExact: d });
};
for (const type of ['BUY', 'SELL']) for (const amount of amounts) for (const rate of rates) for (const method of ['Cash', 'Bank Transfer']) pushDelta(type, 'USD', amount, rate, '0', method);
for (const type of ['BUY', 'SELL']) for (const fee of ['0.01', '10', '25.5', '3210']) for (const method of ['Cash', 'Bank Transfer', 'Credit Card', 'Alipay', 'PromptPay']) pushDelta(type, 'JPY', '100', '32.1', fee, method);
for (const currency of ['EUR', 'GBP', 'CNY', 'ZWG']) pushDelta('BUY', currency, '250', '37.5055', '12.34', 'Cash');
const deltaRefusals = [];
for (const currency of ['THB', 'XXX', 'usd', '']) {
  let err = null; try { XCStock.delta({ type: 'BUY', currency, amount: 1, rate: 1, fee: 0, method: 'Cash', status: 'COMPLETED' }); } catch (e) { err = e.message; }
  deltaRefusals.push({ currency, v2Error: err, error: 'NOT_A_STOCK_CURRENCY' });
  if (!err) throw Error('v2 accepted ' + currency);
}

// ── B. what a shift should hold: opening + transactions + movements ────────
let seed = 20261006;
const rnd = (n) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return (seed >>> 12) % n; }; // high bits: the low ones of this generator cycle
const pick = (a) => a[rnd(a.length)];
const codes = ['USD', 'EUR', 'JPY', 'GBP', 'CNY'];
const scenarios = [];
for (let s = 0; s < 40; s++) {
  const opening = { THB: pick(['0', '500000', '1000000.50', '100000000']) };
  for (const c of codes) if (rnd(3)) opening[c] = pick(['0', '1000', '2500.75', '100000000']);
  const shift = { id: 'S1', opening: Object.fromEntries(Object.entries(opening).map(([k, v]) => [k, Number(v)])) };
  const txs = [], moves = [], v2txs = [], v2moves = [];
  const n = rnd(9);
  for (let i = 0; i < n; i++) {
    if (rnd(4) === 0) {
      const m = { currency: pick(['THB', ...codes]), direction: pick(['IN', 'OUT']), amount: pick(['0.01', '100', '999.99', '50000']) };
      moves.push(m); v2moves.push({ shiftId: 'S1', ...m, amount: Number(m.amount) });
    } else {
      const t = { type: pick(['BUY', 'SELL']), currency: pick(codes), amount: pick(amounts.slice(0, 8)), rate: pick(rates), fee: pick(['0', '0', '10', '25.5']), method: pick(['Cash', 'Cash', 'Bank Transfer']) };
      // v2 refuses a purchase whose fee exceeds its principal before it reaches the stock module
      if (toMinor(t.fee) > exactPrincipal(t.amount, t.rate)) t.fee = '0';
      txs.push(t); v2txs.push({ shiftId: 'S1', status: 'COMPLETED', ...t, amount: Number(t.amount), rate: Number(t.rate), fee: Number(t.fee) });
    }
  }
  // a row of another shift and a row that is not completed: v2 leaves both out
  v2txs.push({ shiftId: 'OTHER', status: 'COMPLETED', type: 'BUY', currency: 'USD', amount: 777, rate: 1, fee: 0, method: 'Cash' });
  v2txs.push({ shiftId: 'S1', status: 'CANCELLED', type: 'BUY', currency: 'USD', amount: 888, rate: 1, fee: 0, method: 'Cash' });
  const v2 = XCStock.expected(shift, v2txs, v2moves);
  const ex = Object.fromEntries(XCStock.currencies.map((c) => [c, toMinor(opening[c] ?? '0')]));
  for (const t of txs) {
    const a = toMinor(t.amount), p = exactPrincipal(t.amount, t.rate), f = toMinor(t.fee);
    ex[t.currency] += t.type === 'BUY' ? a : -a;
    if (t.method === 'Cash') ex.THB += t.type === 'SELL' ? p + f : -(p - f);
  }
  for (const m of moves) ex[m.currency] += m.direction === 'IN' ? toMinor(m.amount) : -toMinor(m.amount);
  const exS = Object.fromEntries(Object.entries(ex).filter(([, v]) => v !== 0n).map(([k, v]) => [k, fmt(v)]));
  const v2S = nonZero(v2, (x) => fmt(v2Minor(x)));
  const d = JSON.stringify(v2S) !== JSON.stringify(exS); if (d) differs++;
  const firstNegative = XCStock.currencies.find((c) => ex[c] < 0n) ?? null;
  scenarios.push({ opening, transactions: txs, movements: moves, v2: v2S, exact: exS, firstNegative, v2DiffersFromExact: d });
}

// ── C. the guard before a change is written ────────────────────────────────
const guards = [];
const guard = (balance, change) => {
  const b = XCStock.balances(Object.fromEntries(Object.entries(balance).map(([k, v]) => [k, Number(v)])));
  const ch = Object.fromEntries(Object.entries(change).map(([k, v]) => [k, Number(toMinor(v.replace('-', '')) * (v.startsWith('-') ? -1n : 1n))]));
  let err = null; try { XCStock.ensureStock(b, ch); } catch (e) { err = e.message; }
  const short = err ? (err.match(/ยอด ([A-Z]{3}) /) || [])[1] : null;
  if (err && !short) throw Error('unreadable v2 error: ' + err);
  guards.push({ balance, change, v2Error: err, insufficient: short });
};
guard({ USD: '100' }, { USD: '-100' });
guard({ USD: '100' }, { USD: '-100.01' });
guard({ THB: '3210', USD: '0' }, { USD: '100', THB: '-3210' });
guard({ THB: '3209.99', USD: '0' }, { USD: '100', THB: '-3210' });
guard({}, { USD: '-0.01' });
guard({}, { USD: '0.01' });
guard({ THB: '0', USD: '0' }, { THB: '-1', USD: '-1' });      // two short: v2 names the first in register order
guard({ ZWG: '0', EUR: '0' }, { ZWG: '-1', EUR: '-1' });
guard({ USD: '100000000' }, { USD: '100000000' });            // a balance may pass the entry cap
guard({ JPY: '5' }, {});

// ── D. counted against expected at close ───────────────────────────────────
const differences = [];
const diff = (expected, counted) => {
  const n = (o) => Object.fromEntries(XCStock.currencies.map((c) => [c, Number(o[c] ?? 0)]));
  const v2 = XCStock.difference(n(expected), n(counted));
  const ex = {};
  for (const c of XCStock.currencies) { const d = toMinor(counted[c] ?? '0') - toMinor(expected[c] ?? '0'); if (d !== 0n) ex[c] = fmt(d); }
  const v2S = nonZero(v2, (x) => fmt(v2Minor(x)));
  const d = JSON.stringify(v2S) !== JSON.stringify(ex); if (d) differs++;
  differences.push({ expected, counted, v2: v2S, exact: ex, v2DiffersFromExact: d });
};
diff({ THB: '500000', USD: '1000' }, { THB: '500000', USD: '1000' });
diff({ THB: '500000', USD: '1000' }, { THB: '499999.50', USD: '1000.25' });
diff({ THB: '0.10' }, { THB: '0.30' });
diff({ USD: '100' }, {});
diff({}, { EUR: '0.01' });
diff({ THB: '3209999999.68' }, { THB: '3209999999.69' });

console.error(`deltas=${deltas.length} refusals=${deltaRefusals.length} scenarios=${scenarios.length} negative=${scenarios.filter((s) => s.firstNegative).length} guards=${guards.length} refused=${guards.filter((g) => g.insufficient).length} differences=${differences.length} v2_differs_from_exact=${differs} currencies=${XCStock.currencies.length}`);
console.log(JSON.stringify({
  source: 'v2 engine extracted unchanged from the deployed bundle (xchange-online@3ffc5a5): XCStock.delta, expected, ensureStock, difference (server.mjs:6511-6580), driven by gen_stock.mjs',
  deltas, deltaRefusals, scenarios, guards, differences,
}, null, 1));
