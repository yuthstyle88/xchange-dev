// Drives the v2 engine (extracted unchanged from the deployed bundle) through
// rate → shift → customer → saveTransaction and records what it computed.
import { runEngine } from './engine.mjs';
import { randomUUID } from 'node:crypto';

const actor = { id: 'owner', email: 'owner' };
function run(tables, props, action, payload, files = {}) {
  const r = runEngine(action, payload, tables, props, actor, files, null);
  return r;
}
function saveCase({ type, amount, rate, fee }) {
  let tables = {}, props = {};
  const step = (action, payload, files) => {
    const r = run(tables, props, action, payload, files);
    tables = r.tables; props = r.properties; return r;
  };
  // Counter rate equal to the submitted one on the side used; a rate v2 itself
  // would refuse to store (too many decimals, zero) is stored as 1 so that the
  // refusal under test comes from saveTransaction's own validation.
  const n = Number(rate), storable = Number.isFinite(n) && n > 0 && n <= 1e8 && Math.abs(n * 1e4 - Math.round(n * 1e4)) <= 1e-5;
  const counter = storable ? n : 1;
  let r = step('adminSaveRate', { code: 'USD', buy: counter, sell: counter, previousUpdated: '', requestId: randomUUID() });
  if (r.error) throw Error('setup rate: ' + r.error);
  r = step('adminOpenShift', { requestId: randomUUID(), opening: { THB: 1e8, USD: 1e8 } });
  if (r.error) throw Error('setup shift: ' + r.error);
  const cust = { name: 'TEST CUSTOMER', passportNo: 'P1234567', nationality: 'TH', mode: 'LIVE' };
  r = step('reserveCustomer', cust);
  if (r.error) throw Error('setup customer: ' + r.error);
  const key = randomUUID();
  const files = { [key]: { url: '/files/' + key, hash: 'h', type: 'image/png', ext: 'png' } };
  r = step('saveTransaction', {
    ...cust, customerId: r.result.customerId, submissionId: randomUUID(), type, currency: 'USD',
    amount, rate, fee, bankFee: 0, method: 'Cash', ref: '', account: 'CASH', staff: 'owner', booking: 'WALK-IN',
    notes: '', valueDate: '2026-10-06', paidAt: '2026-10-06T05:00:00.000Z', paymentConfirmed: true, identityConfirmed: true,
    passport: { uploadKey: key },
  }, files);
  if (r.error) return { error: r.error };
  const row = r.tables.Transactions.at(-1);
  return { principal: row[10], received: row[13], paid: row[14] };
}

// Exact decimal half-up on strings, to see where v2's float arithmetic departs from it.
function exact(amount, rate) {
  const toInt = (s, dp) => { const [a, b = ''] = String(s).split('.'); return BigInt(a + (b + '0'.repeat(dp)).slice(0, dp)); };
  const p = toInt(amount, 2) * toInt(rate, 4); // scale 1e6
  return (p + 5000n) / 10000n; // cents, half up
}
const cents = (n) => Math.round(n * 100);

const ERR = {
  'จำนวนเงินหรือเรทไม่ถูกต้อง': 'INVALID_NUMBER',
  'ทศนิยมเกินกำหนด': 'TOO_MANY_DECIMALS',
  'ค่าบริการสูงกว่ายอดซื้อ': 'FEE_EXCEEDS_PRINCIPAL',
};
const inputs = [];
const amounts = ['0.01', '1', '100', '100.5', '99.99', '1234.56', '0.07', '250000', '99999999.99', '100000000', '3.33', '7.77', '1000.01', '19.95'];
const rates = ['32.1', '32.6', '0.211', '0.2195', '43.9', '4.4567', '0.0001', '1', '37.5055', '21.3333'];
for (const type of ['BUY', 'SELL']) for (const amount of amounts) for (const rate of rates) inputs.push({ type, amount, rate, fee: '0' });
for (const type of ['BUY', 'SELL']) for (const fee of ['0.01', '10', '25.5', '3210', '3210.01', '99999']) inputs.push({ type, amount: '100', rate: '32.1', fee });
for (const type of ['BUY', 'SELL']) for (const [amount, rate, fee] of [['100.00', '32.1', '0'], ['100.000', '32.1', '0'], ['100', '32.10000', '0'], ['100', '32.1', '0.00'], ['0100', '32.1', '0'], ['100', '32.1', '10.50']]) inputs.push({ type, amount, rate, fee });
for (const bad of [
  { amount: '0', rate: '32.1', fee: '0' }, { amount: '-1', rate: '32.1', fee: '0' }, { amount: '100000000.01', rate: '1', fee: '0' },
  { amount: '1.001', rate: '32.1', fee: '0' }, { amount: 'abc', rate: '32.1', fee: '0' }, { amount: '', rate: '32.1', fee: '0' },
  { amount: '100', rate: '0', fee: '0' }, { amount: '100', rate: '-3', fee: '0' }, { amount: '100', rate: '32.12345', fee: '0' },
  { amount: '100', rate: '100000000.0001', fee: '0' }, { amount: '100', rate: '32.1', fee: '-1' }, { amount: '100', rate: '32.1', fee: '1.005' },
]) for (const type of ['BUY', 'SELL']) inputs.push({ type, ...bad });

const cases = []; let differs = 0, unknownErrors = new Set();
for (const input of inputs) {
  // v2 receives JSON numbers from the screens; a string that is not a number is sent as is.
  const num = (s) => (s !== '' && Number.isFinite(Number(s)) ? Number(s) : s);
  const got = saveCase({ type: input.type, amount: num(input.amount), rate: num(input.rate), fee: num(input.fee) });
  if (got.error) {
    const kind = ERR[got.error];
    if (!kind) unknownErrors.add(got.error);
    if (kind) cases.push({ ...input, v2Error: got.error, error: kind });
    continue; // a refusal that is not a number/fee rule (stock) belongs to another fixture
  }
  const ex = exact(input.amount, input.rate), v2c = BigInt(cents(got.principal));
  const fee = BigInt(Math.round(Number(input.fee) * 100));
  const c = { ...input, v2: { principal: got.principal.toFixed(2), received: got.received.toFixed(2), paid: got.paid.toFixed(2) } };
  const f = (x) => { const s = x.toString().padStart(3, '0'); return s.slice(0, -2) + '.' + s.slice(-2); };
  c.exact = { principal: f(ex), received: f(input.type === 'SELL' ? ex + fee : 0n), paid: f(input.type === 'BUY' ? ex - fee : 0n) };
  c.v2DiffersFromExact = ex !== v2c; if (c.v2DiffersFromExact) differs++;
  cases.push(c);
}
console.error(`cases=${cases.length} ok=${cases.filter(c => !c.error).length} errors=${cases.filter(c => c.error).length} v2_differs_from_exact=${differs} unknown_errors=${[...unknownErrors].join(' | ') || 'none'}`);
console.log(JSON.stringify({ source: 'v2 engine extracted unchanged from the deployed bundle (xchange-online@3ffc5a5), driven by gen_quote.mjs: adminSaveRate → adminOpenShift → reserveCustomer → saveTransaction; principal/received/paid read from the stored transaction row', cases }, null, 1));
