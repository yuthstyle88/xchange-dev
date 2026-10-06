// Plays whole exchanges against the v2 engine (extracted unchanged from the deployed
// bundle; see README.md) and records what v2 answered and what it left behind:
// the document number, the three links, the open shift's expected balances, the
// customer's row. `api/tests/save_transaction_test.rs` replays the same steps.
//
//   node gen_save.mjs > ../../tests/fixtures/save/v2_save_cases.json
//
// The clock is set by each step (`at`), so document numbers do not depend on the
// day this script runs. Numbers travel as text; v2 is given the JSON number the
// screens send. A file is named by an alias; `files` says what its bytes are, and
// two aliases may hold the same bytes under different keys.
import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { runEngine } from './engine.mjs';

const src = fs.readFileSync(new URL('./engine.mjs', import.meta.url), 'utf8');
const start = src.indexOf('var XCStock = /* @__PURE__ */ (() => {'), end = src.indexOf('\n  })();', start) + '\n  })();'.length;
if (start < 0) throw Error('XCStock not found in engine.mjs');
const XCStock = new Function(src.slice(start, end) + '\nreturn XCStock;')();

// ---- the clock
const RealDate = Date;
let clock = RealDate.parse('2026-10-06T05:00:00.000Z');
globalThis.Date = class extends RealDate {
  constructor(...a) { if (a.length === 0) super(clock); else super(...a); }
  static now() { return clock; }
};

// ---- files: alias -> key, type, bytes (hex head + zero padding)
const FILES = {
  P1: { key: '11111111-1111-4111-8111-111111111111', mime: 'image/jpeg', head: 'ffd8ffe000104a46494601', pad: 40 },
  P1_AGAIN: { key: '11111111-1111-4111-8111-1111111111aa', mime: 'image/jpeg', head: 'ffd8ffe000104a46494601', pad: 40 },
  P2: { key: '33333333-3333-4333-8333-333333333333', mime: 'image/png', head: '89504e470d0a1a0a02', pad: 40 },
  S1: { key: '22222222-2222-4222-8222-222222222222', mime: 'application/pdf', head: Buffer.from('%PDF-1.7 slip one').toString('hex'), pad: 10 },
  S2: { key: '44444444-4444-4444-8444-444444444444', mime: 'image/webp', head: Buffer.from('RIFF').toString('hex') + '24000000' + Buffer.from('WEBPVP8 ').toString('hex'), pad: 10 },
};
const EXT = { 'image/jpeg': 'jpg', 'image/png': 'png', 'image/webp': 'webp', 'application/pdf': 'pdf' };
const descriptors = {};
for (const f of Object.values(FILES)) {
  const bytes = Buffer.concat([Buffer.from(f.head, 'hex'), Buffer.alloc(f.pad)]);
  descriptors[f.key] = { url: '/files/' + f.key, hash: createHash('sha256').update(bytes).digest('hex'), type: f.mime, ext: EXT[f.mime] };
}
const aliasOf = (url) => { if (!url) return null; const hit = Object.entries(FILES).find(([, f]) => '/files/' + f.key === url); if (!hit) throw Error('unknown file url ' + url); return hit[0]; };

const actor = { id: 'owner', email: 'owner' };
const isPlain = (s) => typeof s === 'string' && /^\d+(\.\d+)?$/.test(s);
const num = (s) => (isPlain(s) ? Number(s) : s);
const money = (n) => (Math.round(n * 100) / 100).toFixed(2);
let rid = 0;
const req = () => 'req-' + String(++rid).padStart(6, '0') + '-aaaaaaaaaa';

const PERSON = { name: 'Somchai Jaidee', passportNo: 'AA1234567', nationality: 'THA', birthDate: '', passportExpiry: '' };
const FORM = {
  ...PERSON, mode: 'LIVE', type: 'BUY', currency: 'USD', amount: '100', rate: '32.1', fee: '0', bankFee: '0',
  method: 'Cash', ref: '', account: 'Cash drawer 1', staff: 'EMP-001', booking: 'WALK-IN', notes: '',
  valueDate: '2026-10-06', paidAt: '2026-10-06T05:30:00.000Z', paymentConfirmed: true, identityConfirmed: true,
};
const BANK = { method: 'Bank Transfer', ref: 'TRX-778899' };

class World {
  constructor(name) { this.name = name; this.tables = {}; this.props = {}; this.steps = []; this.branches = { MAIN: 'MAIN' }; this.customers = {}; this.sub = 0; }
  call(action, payload) { const r = runEngine(action, payload, this.tables, this.props, actor, descriptors, null); this.tables = r.tables; this.props = r.properties; return r; }
  must(r, what) { if (r.error) throw Error(`${this.name}: ${what}: ${r.error}`); return r; }
  b(label) { const id = this.branches[label]; if (!id) throw Error(this.name + ': no branch ' + label); return id; }
  at(iso) { clock = RealDate.parse(iso); return this; }
  branch(label, name) {
    const r = this.must(this.call('adminCreateBranch', { requestId: req(), name }), 'branch');
    this.branches[label] = r.result.branchId;
    this.steps.push({ op: 'createBranch', branch: label, id: r.result.branchId, name });
    return this;
  }
  rate(label, code, buy, sell) {
    const m = this.model(label), prev = m.rates.find((x) => x.code === code)?.updated ?? '';
    this.must(this.call('adminSaveRate', { branchId: this.b(label), code, buy: Number(buy), sell: Number(sell), previousUpdated: prev, requestId: req() }), 'rate');
    this.steps.push({ op: 'rate', branch: label, code, buy, sell });
    return this;
  }
  open(label, opening) {
    this.must(this.call('adminOpenShift', { branchId: this.b(label), requestId: req(), opening: Object.fromEntries(Object.entries(opening).map(([k, v]) => [k, Number(v)])) }), 'open shift');
    this.steps.push({ op: 'openShift', branch: label, opening });
    return this;
  }
  close(label) {
    const m = this.model(label), shift = XCStock.active(m.stock.shifts), expected = XCStock.expected(shift, m.transactions, m.stock.movements);
    this.must(this.call('adminCloseShift', { branchId: this.b(label), requestId: req(), shiftId: shift.id, counted: expected, expected, note: '' }), 'close shift');
    this.steps.push({ op: 'closeShift', branch: label });
    return this;
  }
  reserve(label, who = {}, as = 'c') {
    const p = { ...PERSON, mode: 'LIVE', ...who };
    const r = this.must(this.call('reserveCustomer', { branchId: this.b(label), ...p }), 'reserve');
    this.customers[as] = r.result.customerId;
    this.steps.push({ op: 'reserve', branch: label, as, ...p, customerId: r.result.customerId });
    return this;
  }
  model(label) { return this.must(runEngine('adminSnapshot', { branchId: this.b(label) }, this.tables, this.props, actor, {}, null), 'snapshot').result; }
  /** `over.customer` names a reserved customer (default `c`); `passport` / `slip` are file aliases. */
  save(label, expectError, over = {}, note = '') {
    const { customer = 'c', passport = 'P1', slip = null, submission, ...fields } = over;
    const form = { ...FORM, ...fields };
    const submissionId = submission ?? 'sub-' + String(++this.sub).padStart(4, '0') + '-aaaaaaaaaaaa';
    const customerId = fields.customerId ?? this.customers[customer];
    const payload = { ...form, submissionId, customerId, branchId: this.b(label) };
    for (const k of ['amount', 'rate', 'fee', 'bankFee', 'referenceRate']) if (payload[k] !== undefined) payload[k] = num(payload[k]);
    payload.passport = passport ? { uploadKey: FILES[passport].key } : null;
    payload.slip = slip ? { uploadKey: FILES[slip].key } : null;
    const before = this.state();
    const r = this.call('saveTransaction', payload);
    const step = { op: 'save', note, at: new RealDate(clock).toISOString(), branch: label, form: { ...form, submissionId, customerId }, passport, slip, v2Error: r.error ?? null };
    if (expectError !== !!r.error) throw Error(`${this.name}: save '${note}' ${r.error ? 'was refused: ' + r.error : 'was saved'}`);
    if (r.error) {
      // a refusal changes nothing in v2
      if (this.state() !== before) throw Error(`${this.name}: '${note}' refused but v2 changed something`);
    } else {
      const res = r.result;
      if (res.receiptNo !== res.documentRef || res.saved !== true) throw Error('result shape');
      const receipt = r.pendingFiles[0];
      step.saved = {
        mode: res.mode, documentRef: res.documentRef, passport: aliasOf(res.passportUrl), slip: aliasOf(res.slipUrl), receiptUrl: res.receiptUrl,
        // null when this call wrote nothing (the exchange was already saved)
        receiptSha256: receipt ? receipt.hash : null,
        wroteNothing: this.state() === before,
      };
      if (receipt && (res.receiptUrl !== '/files/receipt-' + receipt.hash || receipt.mime !== 'text/html' || createHash('sha256').update(Buffer.from(receipt.bytes)).digest('hex') !== receipt.hash)) throw Error('receipt shape');
    }
    // nothing to look at in a branch that does not exist
    step.after = label === 'GONE' ? null : this.seen(label);
    this.steps.push(step);
    return this;
  }
  /** What v2 holds for a branch after a step. */
  seen(label) {
    const m = this.model(label), shift = XCStock.active(m.stock.shifts);
    const expected = shift ? XCStock.expected(shift, m.transactions, m.stock.movements) : null;
    const id = this.b(label), pre = id === 'MAIN' ? '' : id + '_';
    const rows = (t) => (this.tables[pre + t] || []).slice(1);
    return {
      balances: expected && Object.fromEntries(Object.entries(expected).filter(([, v]) => v !== 0).map(([k, v]) => [k, money(v)])),
      live: rows('Transactions').map((r) => r[0]), demo: rows('DemoTransactions').map((r) => r[0]),
      shiftOf: Object.fromEntries(m.transactions.map((t) => [t.id, t.shiftId ?? null])),
      customers: rows('Customers').map((r) => ({ id: r[0], name: String(r[1]).replace(/^'/, ''), birthDate: r[4] ? new RealDate(new RealDate(r[4]).getTime() + 7 * 3600e3).toISOString().slice(0, 10) : null, passportExpiry: r[5] ? new RealDate(new RealDate(r[5]).getTime() + 7 * 3600e3).toISOString().slice(0, 10) : null, lastDocumentRef: r[7] || null, mode: r[8] })),
      counters: Object.fromEntries(Object.entries(this.props).filter(([k]) => k.startsWith('document-'))),
    };
  }
  /** Everything v2 holds, without the header rows the sheets that hold none yet and the custody sheet it starts empty (v2 adds a `Reference Rate` heading even to a save it then refuses). */
  state() { return JSON.stringify([Object.fromEntries(Object.entries(this.tables).map(([k, rows]) => [k, rows.slice(1)]).filter(([k, rows]) => rows.length && !k.endsWith('OpsState')).sort()), this.props]); }
  done() { return { name: this.name, steps: this.steps }; }
}

const scenarios = [];
const scenario = (name, build) => { const w = new World(name); build(w); scenarios.push(w.done()); };
const ready = (w, opening = { THB: '100000', USD: '1000' }) => w.rate('MAIN', 'USD', '32.1', '33').open('MAIN', opening).reserve('MAIN');

scenario('a buy for cash takes baht out and puts the currency in', (w) => ready(w).save('MAIN', false, {}, 'buy 100 USD'));
scenario('a sell for cash does the opposite', (w) => ready(w).save('MAIN', false, { type: 'SELL', rate: '33', amount: '250.5' }, 'sell 250.5 USD'));
scenario('a fee moves with the baht', (w) => ready(w).save('MAIN', false, { fee: '25.5' }, 'buy, fee').save('MAIN', false, { type: 'SELL', rate: '33', fee: '10' }, 'sell, fee'));
scenario('paid by transfer: only the currency moves', (w) => ready(w)
  .save('MAIN', false, { ...BANK, slip: 'S1' }, 'buy by transfer')
  .save('MAIN', false, { ...BANK, method: 'PromptPay', type: 'SELL', rate: '33', slip: 'S2', fee: '5' }, 'sell by PromptPay'));
scenario('numbers follow one another through a day, live and demo apart', (w) => ready(w)
  .reserve('MAIN', { mode: 'DEMO' }, 'd')
  .save('MAIN', false, {}, 'first live')
  .save('MAIN', false, { mode: 'DEMO', customer: 'd' }, 'first demo')
  .save('MAIN', false, { amount: '1' }, 'second live')
  .save('MAIN', false, { mode: 'DEMO', customer: 'd', amount: '2' }, 'second demo'));
scenario('a demo exchange needs no shift, no rate and no stock, and moves none', (w) => w
  .reserve('MAIN', { mode: 'DEMO' }, 'd')
  .save('MAIN', false, { mode: 'DEMO', customer: 'd', currency: 'JPY', rate: '0.2195', amount: '1000000' }, 'demo with nothing prepared')
  .rate('MAIN', 'USD', '32.1', '33').open('MAIN', { THB: '100', USD: '1' })
  .save('MAIN', false, { mode: 'DEMO', customer: 'd', type: 'SELL', rate: '99', amount: '5000' }, 'demo against an open shift: its balances stay'));
scenario('the number is counted for the whole shop, not per branch', (w) => {
  ready(w).branch('B', 'Silom').rate('B', 'USD', '32.1', '33').open('B', { THB: '100000', USD: '1000' }).reserve('B', {}, 'cb');
  w.save('MAIN', false, {}, 'MAIN first').save('B', false, { customer: 'cb' }, 'the other branch gets the next number').save('MAIN', false, { amount: '2' }, 'MAIN again');
});
scenario('the same submission in two branches is two exchanges', (w) => {
  ready(w).branch('B', 'Silom').rate('B', 'USD', '32.1', '33').open('B', { THB: '100000', USD: '1000' }).reserve('B', {}, 'cb');
  w.save('MAIN', false, { submission: 'same-submission-0001' }, 'in MAIN').save('B', false, { submission: 'same-submission-0001', customer: 'cb' }, 'in the other branch');
});
scenario('the same submission live and demo is two exchanges', (w) => ready(w).reserve('MAIN', { mode: 'DEMO' }, 'd')
  .save('MAIN', false, { submission: 'same-submission-0002' }, 'live')
  .save('MAIN', false, { submission: 'same-submission-0002', mode: 'DEMO', customer: 'd' }, 'demo'));
scenario('the day turns at Bangkok midnight', (w) => ready(w)
  .at('2026-10-06T16:59:59.999Z').save('MAIN', false, {}, 'the last instant of the 6th')
  .at('2026-10-06T17:00:00.000Z').save('MAIN', false, { amount: '1' }, 'the first instant of the 7th starts at 1')
  .at('2026-10-06T17:00:01.000Z').save('MAIN', false, { amount: '2' }, 'and goes on'));
scenario('sending a saved exchange again changes nothing and answers the same', (w) => ready(w)
  .save('MAIN', false, { submission: 'again-0001-aaaaaaaaaa', fee: '1.25', notes: 'n' }, 'first time')
  .save('MAIN', false, { submission: 'again-0001-aaaaaaaaaa', fee: '1.25', notes: 'n' }, 'again')
  .save('MAIN', false, { submission: 'again-0001-aaaaaaaaaa', fee: '1.250', notes: ' n ', amount: '100.00', rate: '32.1000' }, 'again, written differently')
  .save('MAIN', false, { submission: 'again-0001-aaaaaaaaaa', fee: '1.25', notes: 'n', passport: 'P1_AGAIN' }, 'again, the same passport bytes uploaded anew: the first link is answered'));
scenario('sending it again still works when nothing would allow it now', (w) => ready(w, { THB: '3210', USD: '0' })
  .save('MAIN', false, { submission: 'again-0002-aaaaaaaaaa' }, 'takes all the baht')
  .rate('MAIN', 'USD', '30', '31')
  .save('MAIN', false, { submission: 'again-0002-aaaaaaaaaa' }, 'again: the rate moved and the baht is gone')
  .close('MAIN')
  .save('MAIN', false, { submission: 'again-0002-aaaaaaaaaa' }, 'again: the shift is closed'));
const CHANGES = [
  ['another amount', { amount: '101' }], ['another fee', { fee: '2' }], ['another bank fee', { bankFee: '1' }], ['the other side', { type: 'SELL' }],
  ['another method', { ...BANK, slip: 'S1' }], ['another account', { account: 'Cash drawer 2' }], ['another staff', { staff: 'EMP-002' }],
  ['another booking', { booking: 'B-2' }], ['other notes', { notes: 'changed' }], ['another value date', { valueDate: '2026-10-07' }],
  ['another customer name', { name: 'Somchai J.' }], ['a birth date added', { birthDate: '1990-01-31' }], ['a reference rate added', { referenceRate: '32.2' }],
  ['another passport file', { passport: 'P2' }], ['a slip added', { slip: 'S1' }], ['a payment reference added', { ref: 'R-1' }],
];
scenario('a submission sent again with anything changed is refused', (w) => {
  ready(w).save('MAIN', false, { submission: 'changed-0001-aaaaaaaa' }, 'first time');
  for (const [what, over] of CHANGES) w.save('MAIN', true, { submission: 'changed-0001-aaaaaaaa', ...over }, what);
  w.save('MAIN', false, { submission: 'changed-0001-aaaaaaaa' }, 'unchanged: still answered');
});
scenario('sent again: the method alone changed, and the customer is still judged first', (w) => ready(w).reserve('MAIN', { passportNo: 'BB7654321', name: 'Other Person' }, 'o')
  .save('MAIN', false, { submission: 'changed-0003-aaaaaaaa', ...BANK, slip: 'S1' }, 'first time, by bank transfer')
  .save('MAIN', true, { submission: 'changed-0003-aaaaaaaa', ...BANK, method: 'PromptPay', slip: 'S1' }, 'the same reference and slip, another method')
  .save('MAIN', true, { submission: 'changed-0003-aaaaaaaa', ...BANK, slip: 'S1', customer: 'o' }, "sent again under another customer's id: that is told, not the change")
  .save('MAIN', false, { submission: 'changed-0003-aaaaaaaa', ...BANK, slip: 'S1' }, 'unchanged: still answered'));
scenario('a changed submission is refused before the shift, the rate and the stock are looked at', (w) => ready(w, { THB: '3210', USD: '0' })
  .save('MAIN', false, { submission: 'changed-0002-aaaaaaaa' }, 'first time')
  .close('MAIN')
  .save('MAIN', true, { submission: 'changed-0002-aaaaaaaa', amount: '5' }, 'changed, no shift open'));
scenario('no shift is open', (w) => w.rate('MAIN', 'USD', '32.1', '33').reserve('MAIN')
  .save('MAIN', true, {}, 'never opened')
  .open('MAIN', { THB: '100000', USD: '1000' }).save('MAIN', false, {}, 'opened').close('MAIN')
  .save('MAIN', true, { amount: '1' }, 'closed again'));
scenario('the rate must be the counter rate of that side', (w) => ready(w)
  .save('MAIN', true, { rate: '32.2' }, 'buy above the buy rate')
  .save('MAIN', true, { rate: '32.0999' }, 'buy a hair below')
  .save('MAIN', true, { rate: '33' }, 'buy at the sell rate')
  .save('MAIN', true, { type: 'SELL', rate: '32.1' }, 'sell at the buy rate')
  .save('MAIN', false, { type: 'SELL', rate: '33' }, 'sell at the sell rate')
  .save('MAIN', false, { rate: '32.1' }, 'buy at the buy rate')
  .rate('MAIN', 'USD', '32.15', '33.05')
  .save('MAIN', true, { rate: '32.1' }, 'the rate moved: the old one is refused')
  .save('MAIN', false, { rate: '32.15' }, 'the new one')
  .save('MAIN', true, { currency: 'EUR', rate: '35' }, 'a currency with no counter rate')
  .save('MAIN', true, { currency: 'THB', rate: '1' }, 'baht has no counter rate')
  .save('MAIN', true, { currency: 'ZZZ', rate: '1' }, 'a code that is no currency'));
scenario('a rate of another branch does not count', (w) => {
  ready(w).branch('B', 'Silom').rate('B', 'USD', '31', '32').open('B', { THB: '100000', USD: '1000' }).reserve('B', {}, 'cb');
  w.save('B', true, { customer: 'cb', rate: '32.1' }, "MAIN's rate in the other branch").save('B', false, { customer: 'cb', rate: '31' }, 'its own rate');
});
scenario('the shift must hold what goes out', (w) => ready(w, { THB: '3210', USD: '100' })
  .save('MAIN', true, { amount: '100.01' }, 'buy: one satang of baht short')
  .save('MAIN', true, { type: 'SELL', rate: '33', amount: '100.01' }, 'sell: one cent short')
  .save('MAIN', false, { amount: '100' }, 'buy for exactly all the baht')
  .save('MAIN', false, { type: 'SELL', rate: '33', amount: '200' }, 'sell exactly all the dollars')
  .save('MAIN', true, { type: 'SELL', rate: '33', amount: '0.01' }, 'nothing left to sell')
  .save('MAIN', false, { amount: '1', ...BANK, slip: 'S1' }, 'buy by transfer needs no baht in the drawer'));
scenario('a fee lowers what a buy takes out', (w) => ready(w, { THB: '3200', USD: '0' })
  .save('MAIN', true, {}, 'buy without a fee: ten baht short')
  .save('MAIN', false, { fee: '10' }, 'buy with a fee of ten: exactly enough'));
scenario('the customer id must be that passport in that branch and mode', (w) => {
  ready(w).reserve('MAIN', { passportNo: 'BB7654321', name: 'Other Person' }, 'o').reserve('MAIN', { mode: 'DEMO' }, 'd')
    .branch('B', 'Silom').rate('B', 'USD', '32.1', '33').open('B', { THB: '100000', USD: '1000' }).reserve('B', { passportNo: 'CC0000001' }, 'cb');
  w.save('MAIN', true, { customer: 'o' }, "another customer's id")
    .save('MAIN', true, { customerId: 'CX-99999999' }, 'an id that does not exist')
    .save('MAIN', true, { customer: 'd' }, 'the demo id on a live exchange')
    .save('MAIN', true, { mode: 'DEMO' }, 'the live id on a demo exchange')
    .save('MAIN', true, { nationality: 'USA' }, 'another nationality')
    .save('B', true, {}, "MAIN's customer in the other branch")
    .save('MAIN', true, { customer: 'o', amount: '999999', rate: '1' }, 'the customer is judged before the rate and the stock')
    .save('MAIN', false, { passportNo: ' aa1234567 ', nationality: 'tha' }, 'passport and nationality are compared trimmed and upper-cased');
});
scenario('a save writes the customer as the form has it', (w) => ready(w)
  .save('MAIN', false, { name: 'Somchai J. Jaidee', birthDate: '1990-01-31', passportExpiry: '2031-12-31' }, 'new name and dates')
  .save('MAIN', false, { amount: '1' }, 'the dates are emptied again, the name goes back')
  .save('MAIN', true, { amount: '999999', name: 'Never Written' }, 'a refused save leaves the customer alone'));
scenario('the form is judged before anything is looked up', (w) => w
  .save('MAIN', true, { customerId: 'CX-00000001', name: '' }, 'no name, nothing prepared')
  .save('MAIN', true, { customerId: 'CX-00000001', amount: '0' }, 'no amount')
  .save('MAIN', true, { customerId: 'CX-00000001', passport: null }, 'no passport file')
  .save('MAIN', true, { customerId: 'CX-00000001', ...BANK }, 'a transfer without a slip')
  .save('MAIN', true, { customerId: 'CX-00000001' }, 'a good form, no such customer'));
scenario('a branch that does not exist', (w) => { w.branches.GONE = 'XC-B99'; w.save('GONE', true, { customerId: 'CX-00000001' }, 'a good form for no branch').save('GONE', true, { customerId: 'CX-00000001', name: '' }, 'a bad form for no branch: the branch is judged first'); });
scenario('a refused exchange uses no number', (w) => ready(w, { THB: '3210', USD: '0' })
  .save('MAIN', true, { amount: '500' }, 'refused: not enough baht')
  .save('MAIN', true, { rate: '1' }, 'refused: wrong rate')
  .save('MAIN', false, {}, 'the first one saved is number 1'));
scenario('a reference rate is kept and changes nothing else', (w) => ready(w)
  .save('MAIN', false, { referenceRate: '32.25' }, 'with a reference rate')
  .save('MAIN', false, { amount: '1', referenceRate: '' }, 'an empty one is none'));
scenario('an exchange belongs to the shift that was open', (w) => ready(w)
  .save('MAIN', false, {}, 'in the first shift').close('MAIN')
  .open('MAIN', { THB: '50000', USD: '10' })
  .save('MAIN', false, { amount: '2' }, 'in the second shift: balances start from its opening'));
scenario('several currencies in one shift', (w) => w.rate('MAIN', 'USD', '32.1', '33').rate('MAIN', 'JPY', '0.2195', '0.2295').rate('MAIN', 'EUR', '35.5', '36.25')
  .open('MAIN', { THB: '50000', USD: '100', JPY: '100000', EUR: '0' }).reserve('MAIN')
  .save('MAIN', false, { currency: 'JPY', rate: '0.2195', amount: '10000' }, 'buy yen')
  .save('MAIN', false, { currency: 'JPY', type: 'SELL', rate: '0.2295', amount: '110000' }, 'sell every yen')
  .save('MAIN', true, { currency: 'EUR', type: 'SELL', rate: '36.25', amount: '1' }, 'no euro to sell')
  .save('MAIN', false, { currency: 'EUR', rate: '35.5', amount: '0.07' }, 'buy seven euro cents: 2.485 baht rounds to 2.49')
  .save('MAIN', false, { currency: 'EUR', type: 'SELL', rate: '36.25', amount: '0.07', fee: '0.01' }, 'sell them back'));

const sentences = [...new Set(scenarios.flatMap((s) => s.steps).filter((s) => s.v2Error).map((s) => s.v2Error))].sort();
const saves = scenarios.flatMap((s) => s.steps).filter((s) => s.op === 'save');
process.stdout.write(JSON.stringify({
  source: 'xchange-online@3ffc5a5 engine (server.mjs:5961-6952)', actor: actor.id,
  files: Object.fromEntries(Object.entries(FILES).map(([a, f]) => [a, { mime: f.mime, head: f.head, pad: f.pad }])),
  counts: { scenarios: scenarios.length, saves: saves.length, saved: saves.filter((s) => !s.v2Error).length, refused: saves.filter((s) => s.v2Error).length },
  sentences, scenarios,
}, null, 1) + '\n');
