// Drives `saveTransaction` of the v2 engine (extracted unchanged from the deployed
// bundle) with one form after another and records, for each, the sentence v2 refused
// it with — or, when v2 saved it, the three baht figures and the receipt HTML it wrote.
//   node gen_save_form.mjs > ../../crates/xc-exchange/tests/fixtures/v2_save_form_cases.json
//
// Every form is sent to a branch prepared so that nothing but the form itself can be
// refused: a counter rate equal to the form's rate, an open shift holding plenty of
// every currency used, the customer reserved, both files uploaded. Numbers travel as
// text in the fixture; v2 is given the JSON number the screens send.
import { runEngine } from './engine.mjs';
import fs from 'node:fs';

// The receipt is 25 KB of fixed markup around five places that change. The fixed part is
// `receipt_template.html` (beside the fixture); only the five changing parts of each
// receipt v2 wrote are recorded, and this script proves on every run that template +
// parts gives back v2's bytes exactly.
const TEMPLATE = fs.readFileSync(new URL(process.env.RECEIPT_TEMPLATE ?? '../../crates/xc-exchange/tests/fixtures/receipt_template.html', import.meta.url), 'utf8');
const between = (text, a, b) => { const i = text.indexOf(a); if (i < 0) throw Error('not found: ' + a); const j = text.indexOf(b, i + a.length); if (j < 0) throw Error('not found: ' + b); return text.slice(i + a.length, j); };
function receiptParts(html, name) {
  const parts = {
    title: between(html, '<title>', '</title>'),
    demoBanner: between(html, '</head><body>', '<main class="receipt"'),
    currency: between(html, '<span class="route-code">', '</span>'),
    rows: between(html, '<dl>', '</dl>'),
    total: between(html, '<dt>THB Total</dt><dd>', ' THB</dd>'),
  };
  const back = TEMPLATE.replace('{{TITLE}}', () => parts.title).replace('{{DEMO_BANNER}}', () => parts.demoBanner).replace('{{CURRENCY}}', () => parts.currency).replace('{{ROWS}}', () => parts.rows).replace('{{TOTAL}}', () => parts.total);
  if (back !== html) throw Error(name + ': the template and the recorded parts do not give back the receipt v2 wrote');
  return parts;
}

const actor = { id: 'owner', email: 'owner' };
const isPlain = (s) => typeof s === 'string' && /^\d+(\.\d+)?$/.test(s);
const num = (s) => (isPlain(s) ? Number(s) : s);
const id = (n) => 'req-' + String(n).padStart(4, '0') + '-aaaaaaaaaaaa';
const PASSPORT = '11111111-1111-4111-8111-111111111111', SLIP = '22222222-2222-4222-8222-222222222222';
const files = {
  [PASSPORT]: { url: '/files/' + PASSPORT, hash: 'a'.repeat(64), type: 'image/jpeg', ext: 'jpg' },
  [SLIP]: { url: '/files/' + SLIP, hash: 'b'.repeat(64), type: 'image/png', ext: 'png' },
};
const base = {
  name: 'Somchai Jaidee', passportNo: 'AA1234567', nationality: 'THA', birthDate: '1990-01-31', passportExpiry: '2031-12-31', mode: 'LIVE',
  submissionId: 'sub-0001-aaaaaaaaaaaa', type: 'BUY', currency: 'USD', amount: '100', rate: '32.1', fee: '0', bankFee: '0',
  method: 'Cash', ref: '', account: 'Cash drawer 1', staff: 'EMP-001', booking: 'WALK-IN', notes: '',
  valueDate: '2026-10-06', paidAt: '2026-10-06T05:30:00.000Z', paymentConfirmed: true, identityConfirmed: true,
  passport: PASSPORT, slip: null,
};
const NUMERIC = ['amount', 'rate', 'fee', 'bankFee', 'referenceRate'];

function run(name, over, { branchName = null } = {}) {
  const form = { ...base, ...over };
  let tables = {}, props = {};
  const call = (action, payload, f = {}) => {
    const r = runEngine(action, payload, tables, props, actor, f, null);
    tables = r.tables; props = r.properties;
    return r;
  };
  const must = (r, what) => { if (r.error) throw Error(`${name}: setup ${what}: ${r.error}`); return r; };
  let branchId = 'MAIN';
  if (branchName) branchId = must(call('adminCreateBranch', { requestId: id(50), name: branchName }), 'branch').result.branchId;
  // the counter rate v2 compares the form's rate with; 1 when the form's own rate is not storable
  const n = Number(form.rate), storable = isPlain(String(form.rate)) && n > 0 && n <= 1e8 && Math.abs(n * 1e4 - Math.round(n * 1e4)) <= 1e-5;
  const code = /^[A-Z]{3}$/.test(String(form.currency).trim()) && String(form.currency).trim() !== 'THB' ? String(form.currency).trim() : 'USD';
  const counter = storable ? n : 1;
  const rateSaved = call('adminSaveRate', { branchId, code, buy: counter, sell: counter, previousUpdated: '', requestId: id(1) });
  const opening = { THB: 1e8 }; opening[code] = 1e8;
  if (!rateSaved.error) must(call('adminOpenShift', { branchId, requestId: id(2), opening }), 'shift');
  const cust = call('reserveCustomer', { branchId, name: 'Somchai Jaidee', passportNo: 'AA1234567', nationality: 'THA', mode: form.mode === 'DEMO' ? 'DEMO' : 'LIVE' });
  must(cust, 'customer');
  if (form.customerId === undefined) form.customerId = cust.result.customerId;

  const payload = { ...form, branchId };
  for (const k of NUMERIC) if (payload[k] !== undefined) payload[k] = num(payload[k]);
  payload.passport = form.passport ? { uploadKey: form.passport } : null;
  payload.slip = form.slip ? { uploadKey: form.slip } : null;
  const r = call('saveTransaction', payload, files);

  const out = { name, form, v2Error: r.error ?? null };
  if (!r.error) {
    const sheet = tables[(branchId === 'MAIN' ? '' : branchId + '_') + (form.mode === 'LIVE' ? 'Transactions' : 'DemoTransactions')];
    const row = sheet[sheet.length - 1];
    const c2 = (x) => Number(x).toFixed(2);
    const html = Buffer.from(r.pendingFiles[0].bytes).toString('utf8');
    out.saved = {
      documentRef: r.result.documentRef, branchName: branchName ?? 'XC-B',
      principal: c2(row[10]), received: c2(row[13]), paid: c2(row[14]),
      // what v2 stored of the form after trimming
      stored: { type: row[6], currency: row[7], amount: c2(row[8]), rate: Number(row[9]).toFixed(4), fee: c2(row[11]), bankFee: c2(row[12]), method: row[15], ref: String(row[16]).replace(/^'/, ''), account: String(row[17]).replace(/^'/, ''), staff: String(row[21]).replace(/^'/, ''), booking: String(row[22]).replace(/^'/, ''), notes: String(row[28]).replace(/^'/, ''), month: row[5] },
      receipt: receiptParts(html, name),
    };
  }
  return out;
}

const cases = [];
const ok = (name, over = {}, opt) => { const c = run(name, over, opt); if (c.v2Error) throw Error(`${name}: expected v2 to save, it said: ${c.v2Error}`); cases.push(c); };
const no = (name, over = {}, opt) => { const c = run(name, over, opt); if (!c.v2Error) throw Error(`${name}: expected v2 to refuse`); cases.push(c); };
const bank = { method: 'Bank Transfer', ref: 'TRX-778899', slip: SLIP };

// ── saved ───────────────────────────────────────────────────────────────────
ok('buy for cash');
ok('sell for cash', { type: 'SELL' });
ok('buy with a fee', { fee: '25.5' });
ok('sell with a fee', { type: 'SELL', fee: '25.5' });
ok('buy whose fee equals the principal', { fee: '3210' });
ok('half a satang rounds up', { amount: '0.07', rate: '0.2195' });
ok('the largest amount at a large rate', { type: 'SELL', amount: '100000000', rate: '43.9' });
ok('the smallest amount at the smallest rate', { amount: '0.01', rate: '0.0001', type: 'SELL' });
ok('a bank fee is recorded and changes no figure', { bankFee: '15' });
ok('sell by bank transfer with a slip', { type: 'SELL', ...bank });
for (const method of ['SCB Transfer', 'Credit Card', 'Alipay', 'PromptPay']) ok('paid by ' + method, { ...bank, method });
ok('cash with a payment reference and a slip anyway', { ref: 'R-1', slip: SLIP });
ok('a reference rate', { referenceRate: '32.25' });
ok('an empty reference rate is none', { referenceRate: '' });
ok('a demo exchange', { mode: 'DEMO' });
ok('a demo sell by transfer', { mode: 'DEMO', type: 'SELL', ...bank });
ok('another currency', { currency: 'JPY', amount: '10000', rate: '0.2195' });
ok('in another branch', {}, { branchName: 'Silom & Sons <2>' });
ok('texts that need escaping', { name: `O'Brien & <b>"Sons"</b>`, booking: 'B&B <1>', ref: `q' "r" <&>`, staff: 'E<1>' });
ok('a Thai name and notes', { name: 'สมชาย ใจดี', notes: 'ลูกค้าประจำ', account: 'ลิ้นชัก 1' });
ok('every text at its longest', { name: 'N'.repeat(100), ref: 'R'.repeat(80), account: 'A'.repeat(100), staff: 'S'.repeat(40), booking: 'B'.repeat(60), notes: 'x'.repeat(1000), customerId: undefined });
ok('texts are trimmed', { type: ' BUY ', currency: ' USD ', method: ' Cash ', account: '  Cash drawer 1 ', staff: ' EMP-001 ', booking: ' WALK-IN ', notes: '  note ', submissionId: '  sub-0001-aaaaaaaaaaaa ', valueDate: ' 2026-10-06 ', paidAt: ' 2026-10-06T05:30:00.000Z ' });
ok('paid just before Bangkok midnight', { paidAt: '2026-10-06T16:59:59.999Z' });
ok('paid at Bangkok midnight', { paidAt: '2026-10-06T17:00:00.000Z' });
ok('paid with an offset instead of Z', { paidAt: '2026-10-06T12:30:00+07:00' });
ok('a value date in another month than the payment', { valueDate: '2026-11-01' });
ok('amount and rate written with trailing zeros', { amount: '100.00', rate: '32.1000', fee: '0.00' });
ok('a payment time of 30 characters', { paidAt: '2026-10-06T05:30:00.0000+07:00' });
ok('an account of 50 emoji is 100 UTF-16 units', { account: '😀'.repeat(50) });
ok('the earliest value date v2 accepts', { valueDate: '1920-04-02' });
ok('a submission id of 16 characters', { submissionId: 'a'.repeat(16) });
ok('a submission id of 60 characters', { submissionId: 'b'.repeat(60) });

// ── refused: the customer's six fields come first ───────────────────────────
no('no name', { name: '' });
no('no mode', { mode: '' });
no('a bad birth date', { birthDate: '1990-02-30' });
no('no name and no submission id: the name is judged first', { name: '', submissionId: '' });
no('a bad mode and no submission id: the mode is judged first', { mode: 'X', submissionId: '' });

// ── refused: one text each, in v2's order ───────────────────────────────────
no('no submission id', { submissionId: '' });
no('a submission id of 61 characters', { submissionId: 'b'.repeat(61) });
no('no customer id', { customerId: '' });
no('a customer id of 41 characters', { customerId: 'C'.repeat(41) });
no('no type', { type: '' });
no('a type of five letters', { type: 'BUYER' });
no('no currency', { currency: '' });
no('a currency of four letters', { currency: 'USDT' });
no('no method', { method: '' });
no('a method of 31 characters', { method: 'M'.repeat(31) });
no('a payment reference of 81 characters', { ref: 'R'.repeat(81) });
no('no account', { account: '' });
no('an account of 101 characters', { account: 'A'.repeat(101) });
no('no staff', { staff: '' });
no('a staff of 41 characters', { staff: 'S'.repeat(41) });
no('no booking', { booking: '' });
no('a booking of 61 characters', { booking: 'B'.repeat(61) });
no('an account of 51 emoji is 102 UTF-16 units', { account: '😀'.repeat(51) });
no('notes of 1001 characters', { notes: 'x'.repeat(1001) });
no('no value date', { valueDate: '' });
no('a value date with slashes', { valueDate: '06/10/2026' });
no('a value date that is no day', { valueDate: '2026-02-30' });
no('a value date one day before the earliest', { valueDate: '1920-04-01' });
no('no payment time', { paidAt: '' });
no('a payment time of 31 characters', { paidAt: '2026-10-06T05:30:00.00000+07:00' });

// ── refused: the numbers ────────────────────────────────────────────────────
no('no amount', { amount: '' });
no('amount zero', { amount: '0' });
no('amount with three decimals', { amount: '100.001' });
no('amount above the limit', { amount: '100000000.01' });
no('amount not a number', { amount: 'abc' });
no('amount negative', { amount: '-5' });
no('no rate', { rate: '' });
no('rate zero', { rate: '0' });
no('rate with twelve decimals', { rate: '32.100000000001' });
no('no fee', { fee: '' });
no('fee with three decimals', { fee: '1.001' });
no('fee negative', { fee: '-1' });
no('no bank fee', { bankFee: '' });
no('bank fee with three decimals', { bankFee: '1.001' });

// ── refused: order between fields ───────────────────────────────────────────
no('no submission id and no amount: the submission id is judged first', { submissionId: '', amount: '' });
no('no customer id and a bad type: the customer id is judged first', { customerId: '', type: 'XXXXX' });
no('a bad amount and a bad rate: the amount is judged first', { amount: '1.001', rate: '0' });
no('a bad rate and a bad fee: the rate is judged first', { rate: '1.000000000001', fee: '' });
no('a bad fee and no method: the fee is judged first', { fee: '1.001', method: '' });
no('a fee with three decimals and no bank fee: the fee is judged first', { fee: '1.001', bankFee: '' });
no('a bad bank fee and no method: the bank fee is judged first', { bankFee: '', method: '' });
no('no method and no account: the method is judged first', { method: 'M'.repeat(31), account: '' });
no('no value date and no payment time: the value date is judged first', { valueDate: '06/10/2026', paidAt: '' });
no('a bad amount and an unknown type: the amount is judged first', { amount: '0', type: 'SWAP' });

// ── refused: "the form is incomplete" ───────────────────────────────────────
no('a submission id of 15 characters', { submissionId: 'a'.repeat(15) });
no('a submission id with an underscore', { submissionId: 'aaaaaaaa_aaaaaaaa' });
no('a type that is neither BUY nor SELL', { type: 'SWAP' });
no('a type in lower case', { type: 'buy' });
no('a currency in lower case', { currency: 'usd' });
no('a currency with a digit', { currency: 'US1' });
no('a currency of two letters', { currency: 'US' });
no('a method that is not on the list', { method: 'Bitcoin' });
no('a method in another case', { method: 'cash' });
no('a payment time that is no time', { paidAt: 'yesterday' });
no('payment not confirmed', { paymentConfirmed: false });
no('identity not confirmed', { identityConfirmed: false });
no('incomplete and no payment reference: incomplete is judged first', { ...bank, ref: '', paymentConfirmed: false });

// ── refused: after the form is complete ─────────────────────────────────────
no('a transfer without a payment reference', { ...bank, ref: '' });
no('a transfer whose payment reference is spaces', { ...bank, ref: '   ' });
no('a buy whose fee is one satang above the principal', { fee: '3210.01' });
no('a fee above the principal and no passport file: the fee is judged first', { fee: '3210.01', passport: null });
no('no payment reference and a fee above the principal: the reference is judged first', { ...bank, ref: '', fee: '99999' });
no('no passport file', { passport: null });
no('a transfer without a slip', { ...bank, slip: null });
no('no passport file and no slip: the passport is judged first', { ...bank, passport: null, slip: null });
no('a missing file and a bad reference rate: the file is judged first', { passport: null, referenceRate: '0' });
// v2 holds satang in a float: a principal above 90,071,992,547,409.91 baht is beyond what it can count
ok('the largest principal v2 can count', { type: 'SELL', amount: '100000000', rate: '900719.9254' });
ok('a principal 6,417.11 baht below the limit', { type: 'SELL', amount: '99999999.99', rate: '900719.9255' });
no('a principal 1,597.29 baht above the limit', { type: 'SELL', amount: '90071992.54', rate: '1000000.0001' });
no('a principal beyond what v2 can count', { type: 'SELL', amount: '100000000', rate: '900719.9255' });
no('the largest amount at the largest rate', { type: 'SELL', amount: '100000000', rate: '100000000' });
no('reference rate zero', { referenceRate: '0' });
no('reference rate with twelve decimals', { referenceRate: '32.000000000001' });
no('reference rate not a number', { referenceRate: 'abc' });

const sentences = [...new Set(cases.map((c) => c.v2Error).filter(Boolean))].sort();
console.error(`cases=${cases.length} saved=${cases.filter((c) => !c.v2Error).length} refused=${cases.filter((c) => c.v2Error).length} sentences=${sentences.length}`);
console.log(JSON.stringify({
  source: 'v2 engine extracted unchanged from the deployed bundle (xchange-online@3ffc5a5): saveTransaction, receiptHtml_ (server.mjs:6417-6506), driven by gen_save_form.mjs',
  passportKey: PASSPORT, slipKey: SLIP, sentences, cases,
}, null, 1));
