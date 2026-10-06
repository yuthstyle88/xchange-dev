// Drives the branch and rate actions of the v2 engine (`adminSaveRate`,
// `adminCreateBranch`, `adminAbandonRequest`, `adminSnapshot`, extracted unchanged
// from the deployed bundle) through scripted scenarios and records what v2 answered
// after every step.
//   node gen_rates.mjs > ../../crates/xc-rate/tests/fixtures/v2_rate_cases.json
//
// Numbers travel as text in the fixture ("32.1"); v2 is given the JSON number the
// screens send. A step's `previousUpdated` is symbolic, because v2 stamps rows with
// the wall clock: "@current" = the `updated` of the code's newest row in that branch
// ("" when it has none), "@stale" = a time that is never the newest row's.
import { runEngine } from './engine.mjs';

const actor = { id: 'owner', email: 'owner' };
const STALE = '2001-01-01T00:00:00.000Z';
const isPlain = (s) => typeof s === 'string' && /^\d+(\.\d+)?$/.test(s);
const num = (s) => (isPlain(s) ? Number(s) : s);
const f4 = (n) => (n == null ? null : Number(n).toFixed(4));
const id = (n) => 'req-' + String(n).padStart(4, '0') + '-aaaaaaaaaaaa'; // 21 characters, matches v2's request id pattern

function play(name, steps) {
  let tables = {}, props = {};
  const call = (action, payload) => {
    const r = runEngine(action, payload, tables, props, actor, {}, null);
    tables = r.tables; props = r.properties;
    return r;
  };
  const out = [];
  for (const s of steps) {
    const branch = s.branch ?? 'MAIN';
    if (s.op === 'saveRate') {
      const before = call('adminSnapshot', { branchId: branch });
      const current = before.error ? '' : (before.result.rates.find((r) => r.code === String(s.code).trim())?.updated ?? '');
      const previousUpdated = s.previousUpdated === '@current' ? current : s.previousUpdated === '@stale' ? STALE : s.previousUpdated;
      const payload = { branchId: branch, requestId: s.requestId, code: s.code, buy: num(s.buy), sell: num(s.sell), previousUpdated };
      if (s.partnerName !== undefined) payload.partnerName = s.partnerName;
      if (s.partnerCost !== undefined) payload.partnerCost = s.partnerCost === null ? null : num(s.partnerCost);
      // what the host adds before the engine runs: the reference rows of this code
      payload.botReference = { source: 'fixture', fetchedAt: STALE, kind: 'FIXTURE', rows: [{ code: s.code }].filter((r) => ['USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD'].includes(r.code)) };
      const r = call('adminSaveRate', payload);
      const after = call('adminSnapshot', { branchId: branch });
      const row = after.error ? null : after.result.rates.find((x) => x.code === String(s.code).trim()) ?? null;
      out.push({ ...s, branch, v2Error: r.error ?? null,
        after: after.error ? null : {
          rate: row && { buy: f4(row.buy), sell: f4(row.sell), requestId: row.requestId, partnerName: row.partnerName, partnerCost: f4(row.partnerCost), hasUpdated: row.updated !== '' },
          history: after.result.rateHistory.map((h) => ({ code: h.code, buy: f4(h.buy), sell: f4(h.sell), actor: h.actor, partnerName: h.partnerName, partnerCost: f4(h.partnerCost) })),
        } });
    } else if (s.op === 'abandon') {
      const r = call('adminAbandonRequest', { branchId: branch, requestId: s.requestId });
      out.push({ ...s, branch, v2Error: r.error ?? null, resolution: r.error ? null : r.result.requestResolution });
    } else if (s.op === 'createBranch') {
      const r = call('adminCreateBranch', { branchId: branch, requestId: s.requestId, name: s.name });
      const after = call('adminSnapshot', {});
      out.push({ ...s, branch, v2Error: r.error ?? null, createdId: r.error ? null : r.result.branchId,
        branchesAfter: after.result.branches.map((b) => ({ id: b.id, name: b.name, requestId: b.requestId ?? null })) });
    } else throw Error('unknown op ' + s.op);
  }
  return { name, steps: out };
}

const save = (n, code, buy, sell, previousUpdated = '@current', extra = {}) => ({ op: 'saveRate', requestId: id(n), code, buy, sell, previousUpdated, ...extra });
const scenarios = [];
const sc = (name, steps) => scenarios.push(play(name, steps));

// ── rates: the plain path ───────────────────────────────────────────────────
sc('first rate of a code, then a newer one', [save(1, 'USD', '32.1', '32.6'), save(2, 'USD', '32.15', '32.65'), save(3, 'EUR', '37.5055', '37.9')]);
sc('sell equal to buy is allowed', [save(1, 'JPY', '0.2195', '0.2195')]);
sc('four decimals, smallest and largest rate', [save(1, 'USD', '0.0001', '0.0001'), save(2, 'EUR', '100000000', '100000000'), save(3, 'GBP', '43.9000', '44.10')]);
sc('the last code of the register', [save(1, 'ZWG', '1', '2')]);

// ── rates: refused by the numbers ───────────────────────────────────────────
sc('sell below buy', [save(1, 'USD', '32.6', '32.1')]);
sc('sell below buy by one ten-thousandth', [save(1, 'USD', '32.1001', '32.1')]);
sc('buy zero', [save(1, 'USD', '0', '32.1')]);
sc('sell zero', [save(1, 'USD', '32.1', '0')]);
sc('buy with five decimals', [save(1, 'USD', '32.12345', '33')]);
sc('sell with five decimals', [save(1, 'USD', '32', '33.00001')]);
sc('buy above the limit', [save(1, 'USD', '100000000.0001', '200000000')]);
sc('buy empty', [save(1, 'USD', '', '32.1')]);
sc('sell empty', [save(1, 'USD', '32.1', '')]);
sc('buy not a number', [save(1, 'USD', 'abc', '32.1')]);
sc('buy negative', [save(1, 'USD', '-1', '32.1')]);
sc('both wrong: buy is judged first', [save(1, 'USD', '32.12345', '0')]);

// ── rates: refused by the code ──────────────────────────────────────────────
sc('THB has no counter rate', [save(1, 'THB', '1', '1')]);
sc('a code outside the register', [save(1, 'XXX', '1', '2')]);
sc('lower case code', [save(1, 'usd', '1', '2')]);
sc('code empty', [save(1, '', '1', '2')]);
sc('code of four letters', [save(1, 'USDT', '1', '2')]);
sc('code with spaces around it is trimmed', [save(1, ' USD ', '1', '2')]);
sc('bad code and bad number: the number is judged first', [save(1, 'XXX', '0', '2')]);
sc('bad code and sell below buy give the same sentence', [save(1, 'XXX', '3', '2')]);

// ── rates: the request id ───────────────────────────────────────────────────
const rid = (requestId) => ({ op: 'saveRate', requestId, code: 'USD', buy: '32.1', sell: '32.6', previousUpdated: '@current' });
sc('request id of 15 characters', [rid('a'.repeat(15))]);
sc('request id of 16 characters', [rid('a'.repeat(16))]);
sc('request id of 60 characters', [rid('b'.repeat(60))]);
sc('request id of 61 characters', [rid('b'.repeat(61))]);
sc('request id empty', [rid('')]);
sc('request id with an underscore', [rid('aaaaaaaa_aaaaaaaa')]);
sc('request id with a space inside', [rid('aaaaaaaa aaaaaaaa')]);
sc('request id with spaces around it is trimmed', [rid('  cccccccccccccccc  '), { ...rid('cccccccccccccccc') }]);
sc('bad request id and bad number: the request id is judged first', [{ ...rid('short'), buy: '0' }]);

// ── rates: partner ──────────────────────────────────────────────────────────
sc('partner name and cost together', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'SuperRich', partnerCost: '32.05' })]);
sc('partner name without cost', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'SuperRich' })]);
sc('partner name with null cost', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'SuperRich', partnerCost: null })]);
sc('partner name with empty cost', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'SuperRich', partnerCost: '' })]);
sc('partner cost without name', [save(1, 'USD', '32.1', '32.6', '@current', { partnerCost: '32.05' })]);
sc('partner name of spaces counts as none', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: '   ', partnerCost: '32.05' })]);
sc('partner name of spaces and no cost is no partner', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: '   ' })]);
sc('partner name is trimmed', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: '  Vasu  ', partnerCost: '32' })]);
sc('partner name of 100 characters', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'ก'.repeat(100), partnerCost: '32' })]);
sc('partner name of 101 characters', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'ก'.repeat(101), partnerCost: '32' })]);
sc('partner name of 50 emoji is 100 UTF-16 units', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: '😀'.repeat(50), partnerCost: '32' })]);
sc('partner name of 51 emoji is 102 UTF-16 units', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: '😀'.repeat(51), partnerCost: '32' })]);
sc('partner cost zero', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '0' })]);
sc('partner cost with five decimals', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '32.00001' })]);
sc('partner cost may exceed the sell rate', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '99' })]);
sc('a newer row without partner clears it', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '32' }), save(2, 'USD', '32.2', '32.7')]);

// ── rates: the same request again ───────────────────────────────────────────
sc('same request, same data: nothing new is written', [save(1, 'USD', '32.1', '32.6'), save(1, 'USD', '32.1', '32.6', '')]);
sc('same request after a newer row, with a stale previousUpdated: still the saved answer', [save(1, 'USD', '32.1', '32.6'), save(2, 'USD', '32.2', '32.7'), save(1, 'USD', '32.1', '32.6', '@stale')]);
sc('same request, the number written another way', [save(1, 'USD', '32.1', '32.6'), save(1, 'USD', '32.1000', '32.60', '')]);
sc('same request, other buy', [save(1, 'USD', '32.1', '32.6'), save(1, 'USD', '32.2', '32.6', '@current')]);
sc('same request, other sell', [save(1, 'USD', '32.1', '32.6'), save(1, 'USD', '32.1', '32.7', '@current')]);
sc('same request, other code', [save(1, 'USD', '32.1', '32.6'), save(1, 'EUR', '32.1', '32.6', '@current')]);
sc('same request, partner added', [save(1, 'USD', '32.1', '32.6'), save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '32' })]);
sc('same request, partner cost changed', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '32' }), save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '32.01' })]);
sc('same request, partner name changed', [save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P', partnerCost: '32' }), save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'Q', partnerCost: '32' })]);
sc('same request with a refused partner pair: the pair is judged before the replay', [save(1, 'USD', '32.1', '32.6'), save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P' })]);

// ── rates: edited from another screen ───────────────────────────────────────
sc('first row with a previousUpdated that names a row', [save(1, 'USD', '32.1', '32.6', '@stale')]);
sc('newer row with an empty previousUpdated', [save(1, 'USD', '32.1', '32.6'), save(2, 'USD', '32.2', '32.7', '')]);
sc('newer row with a stale previousUpdated', [save(1, 'USD', '32.1', '32.6'), save(2, 'USD', '32.2', '32.7', '@stale')]);
sc('previousUpdated is per code: another code having rows does not matter', [save(1, 'USD', '32.1', '32.6'), save(2, 'EUR', '37', '38', '')]);

// ── rates: an abandoned request ─────────────────────────────────────────────
sc('abandoned before use', [{ op: 'abandon', requestId: id(1) }, save(1, 'USD', '32.1', '32.6')]);
sc('abandon after it was applied changes nothing', [save(1, 'USD', '32.1', '32.6'), { op: 'abandon', requestId: id(1) }, save(1, 'USD', '32.1', '32.6', '')]);
sc('abandoned request and bad number: the number is judged first', [{ op: 'abandon', requestId: id(1) }, save(1, 'USD', '0', '32.6')]);
sc('abandoned request and a refused partner pair: abandoned is judged first', [{ op: 'abandon', requestId: id(1) }, save(1, 'USD', '32.1', '32.6', '@current', { partnerName: 'P' })]);

// ── rates and branches ──────────────────────────────────────────────────────
const B = 'B' + id(50);
sc('a branch that does not exist', [{ ...save(1, 'USD', '32.1', '32.6'), branch: 'BNOPE' }]);
sc('a branch that does not exist and sell below buy: the branch is judged first', [{ ...save(1, 'USD', '32.6', '32.1'), branch: 'BNOPE' }]);
sc('rates are kept per branch', [
  { op: 'createBranch', requestId: id(50), name: 'Silom' },
  save(1, 'USD', '32.1', '32.6'),
  { ...save(2, 'USD', '31', '33', ''), branch: B },
  { ...save(3, 'USD', '31.5', '33.5'), branch: B },
  save(4, 'USD', '32.2', '32.7'),
]);
sc('a request abandoned in one branch is refused only there', [
  { op: 'createBranch', requestId: id(50), name: 'Silom' },
  { op: 'abandon', requestId: id(1) },
  { ...save(1, 'USD', '31', '33'), branch: B },
]);

// ── branches ────────────────────────────────────────────────────────────────
const mk = (n, name, branch = 'MAIN') => ({ op: 'createBranch', requestId: id(n), name, branch });
sc('create a branch', [mk(1, 'Silom')]);
sc('create two branches', [mk(1, 'Silom'), mk(2, 'Asok')]);
sc('branch name is trimmed', [mk(1, '  Silom  ')]);
sc('same request, same name', [mk(1, 'Silom'), mk(1, 'Silom')]);
sc('same request, other name', [mk(1, 'Silom'), mk(1, 'Asok')]);
sc('same request, the name in another case', [mk(1, 'Silom'), mk(1, 'SILOM')]);
sc('name taken', [mk(1, 'Silom'), mk(2, 'Silom')]);
sc('name taken in another case', [mk(1, 'Silom'), mk(2, 'sILOM')]);
sc('the name of the main branch is taken', [mk(1, 'xc-b')]);
sc('branch name empty', [mk(1, '')]);
sc('branch name of spaces', [mk(1, '   ')]);
sc('branch name of 60 characters', [mk(1, 'ส'.repeat(60))]);
sc('branch name of 61 characters', [mk(1, 'ส'.repeat(61))]);
sc('branch request id too short', [{ op: 'createBranch', requestId: 'short', name: 'Silom' }]);
sc('bad request id and empty name: the request id is judged first', [{ op: 'createBranch', requestId: 'short', name: '' }]);
sc('created from a branch that does not exist, with a bad request id: the branch is judged first', [{ op: 'createBranch', requestId: 'short', name: 'Silom', branch: 'BNOPE' }]);
sc('created from a branch that does not exist', [mk(1, 'Silom', 'BNOPE')]);
sc('created from another branch', [mk(50, 'Silom'), mk(2, 'Asok', B)]);
sc('abandoned request', [{ op: 'abandon', requestId: id(1) }, mk(1, 'Silom')]);
sc('abandoned request and a taken name: abandoned is judged first', [{ op: 'abandon', requestId: id(1) }, mk(1, 'XC-B')]);
sc('abandon after the branch was created changes nothing', [mk(1, 'Silom'), { op: 'abandon', requestId: id(1) }, mk(1, 'Silom')]);
sc('twenty branches at most, the main one included', [...Array.from({ length: 19 }, (_, i) => mk(i + 1, 'Branch ' + (i + 1))), mk(20, 'One too many'), mk(19, 'Branch 19'), mk(21, 'Branch 1')]);

// ── the register: code, Thai name and symbol of every currency with a rate row ─
const snap = runEngine('adminSnapshot', {}, {}, {}, actor, {}, null).result;
const currencies = snap.rates.map((r) => ({ code: r.code, name: r.name, symbol: r.symbol ?? null }));

const sentences = [...new Set(scenarios.flatMap((s) => s.steps.map((x) => x.v2Error)).filter(Boolean))].sort();
const steps = scenarios.flatMap((s) => s.steps);
console.error(`scenarios=${scenarios.length} steps=${steps.length} refused=${steps.filter((s) => s.v2Error).length} sentences=${sentences.length} currencies=${currencies.length} with_symbol=${currencies.filter((c) => c.symbol).length}`);
console.log(JSON.stringify({
  source: 'v2 engine extracted unchanged from the deployed bundle (xchange-online@3ffc5a5): adminSaveRate, adminCreateBranch, adminAbandonRequest, adminSnapshot (server.mjs:6614-6824), driven by gen_rates.mjs',
  mainBranch: snap.branches[0], sentences, currencies, scenarios,
}, null, 1));
