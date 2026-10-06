// Drives `reserveCustomer` of the v2 engine (extracted unchanged from the deployed
// bundle) through scripted scenarios and records what v2 answered after every step,
// together with the customers `adminSnapshot` then lists.
//   node gen_customers.mjs > ../../crates/xc-exchange/tests/fixtures/v2_customer_cases.json
import { runEngine } from './engine.mjs';

const actor = { id: 'owner', email: 'owner' };
const id = (n) => 'req-' + String(n).padStart(4, '0') + '-aaaaaaaaaaaa';
const B = 'B' + id(50);

function play(name, steps, props0 = {}) {
  let tables = {}, props = { ...props0 };
  const call = (action, payload) => {
    const r = runEngine(action, payload, tables, props, actor, {}, null);
    tables = r.tables; props = r.properties;
    return r;
  };
  const out = [];
  for (const s of steps) {
    if (s.op === 'createBranch') {
      const r = call('adminCreateBranch', { requestId: id(50), name: 'Silom' });
      if (r.error) throw Error('setup branch: ' + r.error);
      out.push({ op: 'createBranch', branch: B, name: 'Silom' });
      continue;
    }
    const branch = s.branch ?? 'MAIN';
    const payload = { branchId: branch };
    for (const k of ['name', 'passportNo', 'nationality', 'birthDate', 'passportExpiry', 'mode']) if (s[k] !== undefined) payload[k] = s[k];
    const r = call('reserveCustomer', payload);
    const snap = call('adminSnapshot', { branchId: branch });
    const sheet = tables[branch === 'MAIN' ? 'Customers' : branch + '_Customers'] ?? [];
    const un = (v) => (typeof v === 'string' && v[0] === "'" ? v.slice(1) : v);
    const day = (v) => (v instanceof Date ? new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Bangkok' }).format(v) : null);
    out.push({ op: 'reserve', branch, ...Object.fromEntries(Object.entries(payload).filter(([k]) => k !== 'branchId')),
      v2Error: r.error ?? null, customerId: r.error ? null : r.result.customerId, existing: r.error ? null : r.result.existing,
      // every stored customer of the branch, both modes, in the order v2 wrote them
      stored: snap.error ? null : sheet.slice(1).map((c) => ({ id: c[0], name: un(c[1]), passportNo: un(c[2]), nationality: un(c[3]), birthDate: day(c[4]), passportExpiry: day(c[5]), mode: c[8] })),
      // what the admin screen lists: live customers only
      listed: snap.error ? null : snap.result.customers.map((c) => ({ id: c.id, name: c.name, nationality: c.nationality })) });
  }
  // `counters`: where v2's numbering stood before the first step (absent = never used)
  return { name, counters: props0, steps: out };
}

const base = { name: 'Somchai Jaidee', passportNo: 'AA1234567', nationality: 'THA', mode: 'LIVE' };
const rs = (over = {}) => ({ op: 'reserve', ...base, ...over });
const scenarios = [];
const sc = (name, steps, props) => scenarios.push(play(name, steps, props));

// ── the plain path ──────────────────────────────────────────────────────────
sc('a new customer gets the first number', [rs()]);
sc('three customers are numbered in order', [rs(), rs({ passportNo: 'BB1' }), rs({ passportNo: 'CC1', name: 'Third' })]);
sc('the same passport again returns the same customer and changes nothing', [rs(), rs({ name: 'Another Name', birthDate: '1990-01-31' })]);
sc('dates are kept', [rs({ birthDate: '1990-01-31', passportExpiry: '2031-12-31' })]);
sc('empty dates are no dates', [rs({ birthDate: '', passportExpiry: '' })]);
sc('passport number and nationality are stored in upper case, the name as typed', [rs({ name: 'somchai Jaidee', passportNo: 'aa1234567', nationality: 'tha' }), rs({ passportNo: 'AA1234567', nationality: 'THA' })]);
sc('all three texts are trimmed', [rs({ name: '  Somchai  ', passportNo: '  AA1  ', nationality: '  THA ' }), rs({ passportNo: 'AA1', nationality: 'THA' })]);

// ── what makes two customers ────────────────────────────────────────────────
sc('same passport number, other nationality', [rs(), rs({ nationality: 'LAO' })]);
sc('same passport, the other mode: a demo customer is a different one with its own numbering', [rs(), rs({ mode: 'DEMO' }), rs({ mode: 'DEMO', passportNo: 'ZZ9' }), rs({ passportNo: 'YY8' })]);
sc('a demo customer is not listed on the admin screen', [rs({ mode: 'DEMO' })]);
sc('the same passport in two branches is two customers, numbered from one counter', [{ op: 'createBranch' }, rs(), rs({ branch: B }), rs({ branch: B }), rs({ passportNo: 'BB1' })]);

// ── refusals ────────────────────────────────────────────────────────────────
sc('a branch that does not exist', [rs({ branch: 'BNOPE' })]);
sc('a branch that does not exist and no name: the branch is judged first', [rs({ branch: 'BNOPE', name: '' })]);
sc('name empty', [rs({ name: '' })]);
sc('name of spaces', [rs({ name: '   ' })]);
sc('name of 100 characters', [rs({ name: 'ก'.repeat(100) })]);
sc('name of 101 characters', [rs({ name: 'ก'.repeat(101) })]);
sc('name of 50 emoji is 100 UTF-16 units', [rs({ name: '😀'.repeat(50) })]);
sc('name of 51 emoji is 102 UTF-16 units', [rs({ name: '😀'.repeat(51) })]);
sc('passport number empty', [rs({ passportNo: '' })]);
sc('passport number of 30 characters', [rs({ passportNo: 'A'.repeat(30) })]);
sc('passport number of 31 characters', [rs({ passportNo: 'A'.repeat(31) })]);
sc('nationality empty', [rs({ nationality: '' })]);
sc('nationality of 60 characters', [rs({ nationality: 'N'.repeat(60) })]);
sc('nationality of 61 characters', [rs({ nationality: 'N'.repeat(61) })]);
sc('mode missing', [rs({ mode: undefined })]);
sc('mode in lower case', [rs({ mode: 'live' })]);
sc('mode unknown', [rs({ mode: 'TEST' })]);
sc('no name and no mode: the name is judged first', [rs({ name: '', mode: 'X' })]);
sc('bad birth date and no mode: the date is judged first', [rs({ birthDate: 'x', mode: 'X' })]);
sc('no passport number and a bad date: the passport number is judged first', [rs({ passportNo: '', birthDate: 'x' })]);
sc('bad birth date and bad expiry: the birth date is judged first', [rs({ birthDate: '1990-02-30', passportExpiry: 'x' })]);
for (const [what, value] of [
  ['with slashes', '31/01/1990'], ['of 11 characters', '1990-01-311'], ['without leading zeros', '1990-1-31'],
  ['of 30 February', '1990-02-30'], ['of 29 February in a common year', '2023-02-29'], ['of month 13', '1990-13-01'],
  ['of day 00', '1990-01-00'], ['of year 0000', '0000-01-01'], ['with a letter', '1990-01-3x'],
]) {
  sc('birth date ' + what, [rs({ birthDate: value })]);
  sc('passport expiry ' + what, [rs({ passportExpiry: value })]);
}
// v2 checks a date by printing it back in Asia/Bangkok, whose offset was not +07:00 before
// 1 April 1920: every earlier date fails that check. The first date it accepts is 1920-04-02.
sc('the earliest date v2 accepts', [rs({ birthDate: '1920-04-02', passportExpiry: '1920-04-02' })]);
sc('birth date one day before the earliest', [rs({ birthDate: '1920-04-01' })]);
sc('passport expiry one day before the earliest', [rs({ passportExpiry: '1920-04-01' })]);
sc('birth date in 1899', [rs({ birthDate: '1899-12-31' })]);
sc('the last date of year 9999', [rs({ birthDate: '9999-12-31', passportExpiry: '9999-12-31' })]);
sc('29 February in a leap year', [rs({ birthDate: '2024-02-29' })]);
sc('a date with spaces around it is trimmed', [rs({ birthDate: ' 1990-01-31 ' })]);
sc('a birth date in the future and an expiry in the past are accepted', [rs({ birthDate: '2999-01-01', passportExpiry: '1999-01-01' })]);

// ── the counter ─────────────────────────────────────────────────────────────
sc('the counter continues from where it stands', [rs()], { 'customer-LIVE': '41' });
sc('eight digits, then nine', [rs(), rs({ passportNo: 'BB1' })], { 'customer-LIVE': '99999998' });
sc('the last number', [rs()], { 'customer-LIVE': '999999998' });
sc('the counter is exhausted', [rs()], { 'customer-LIVE': '999999999' });

const sentences = [...new Set(scenarios.flatMap((s) => s.steps.map((x) => x.v2Error)).filter(Boolean))].sort();
const steps = scenarios.flatMap((s) => s.steps).filter((s) => s.op === 'reserve');
console.error(`scenarios=${scenarios.length} reserve_steps=${steps.length} refused=${steps.filter((s) => s.v2Error).length} sentences=${sentences.length}`);
console.log(JSON.stringify({
  source: 'v2 engine extracted unchanged from the deployed bundle (xchange-online@3ffc5a5): reserveCustomer, adminSnapshot (server.mjs:6317-6383, 6631), driven by gen_customers.mjs',
  sentences, scenarios,
}, null, 1));
