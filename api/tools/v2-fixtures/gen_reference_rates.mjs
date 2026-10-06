// Records what v2 answers on `/api/reference-rates`, and what its Bank of Thailand
// fetch makes of what the bank publishes.
//   node gen_reference_rates.mjs route  [path/to/server.mjs] > ../../tests/fixtures/v2_reference_rates_cases.json
//   node gen_reference_rates.mjs source [path/to/server.mjs] > ../../crates/xc-rate/tests/fixtures/v2_bot_source_cases.json
//
// Like `gen_policy.mjs` this script needs the whole deployed bundle (`server.mjs`,
// `xchange-online@3ffc5a5`, 8301 lines), not `engine.mjs`: the route, its gate and the
// fetch all live outside the engine. The bundle is not in this repository; the default
// path is `./server.mjs`.
//
// How v2 is run. Four pieces of the bundle are evaluated unchanged: `identity` /
// `authorizeWrite`, the reference-rate module (`normalizeBOT`, `fetchBOT`), the custody
// module, and everything from the documents route to the end of the file — the handlers,
// the route table, `handle`, `toRequest`, `send` and the `http.createServer` callback.
// Every request of the `route` mode goes through that callback, exactly as a socket
// would hand it over; every case of the `source` mode calls v2's own `fetchBOT`.
// What is replaced, and nothing else:
//   - `import … from "node:crypto"` lines are dropped and the names are passed in;
//     `import.meta.url` is a fixed string and v2's `console` is silent;
//   - `http.createServer` hands its callback to this script instead of listening;
//   - `fetch` answers from the case's script instead of the network and records every
//     call; `AbortSignal.timeout` records the milliseconds it was given; `Date` reads the
//     case's clock;
//   - the two database handles answer the two session look-ups (that is how v2 learns
//     who is calling) and stop the script on anything else: this route touches nothing
//     else in the database.
//
// The script fails, and writes nothing, when v2 says a sentence it was not expected to
// say, when a database statement other than the two look-ups is reached, or when a
// number v2 computed is further than 1e-12 from the decimal it stands for.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { Readable } from 'node:stream';
import { fileURLToPath } from 'node:url';

const mode = process.argv[2];
if (mode !== 'route' && mode !== 'source') throw Error('usage: node gen_reference_rates.mjs route|source [server.mjs]');
const file = process.argv[3] || new URL('./server.mjs', import.meta.url);
const src = fs.readFileSync(file, 'utf8');
const slice = (from, to) => { const a = src.indexOf(from), b = to === null ? src.length : src.indexOf(to, a); if (a < 0 || b < 0) throw Error(from + ' not found in server.mjs'); return src.slice(a, b); };

// ── the world around v2 ─────────────────────────────────────────────────────
const world = { actor: null, dbDown: false, now: 0, upstream: null, requests: [] };
const OWNER_ROW = { id: 'owner', username: 'owner', name: 'Owner', version: 1, active: true };
const employeeRow = (a) => ({ id: 'E-' + a.role, username: a.role.toLowerCase(), name: a.role, role: a.role, branch_id: 'B2', active: 1, must_change: 0, version: 1 });
const unexpected = (sql) => { throw Error('v2 reached a database statement this route should not need: ' + sql); };
const statement = (sql) => ({
  bind: () => statement(sql),
  first: async () => /JOIN employee_sessions/.test(sql) ? (world.actor?.kind === 'staff' ? employeeRow(world.actor) : null) : unexpected(sql),
  all: async () => unexpected(sql),
  run: async () => unexpected(sql),
});
class Clock extends Date {
  constructor(...a) { if (a.length) super(...a); else super(world.now); }
  static now() { return world.now; }
}
let onRequest = null;
const scope = {
  process: { env: { AUTH_ENABLED: '1', DEVICES_ENABLED: '1', OWNER_ID: 'owner', CUSTOMER_HOST: 'customer.test' } },
  env: { DB: { prepare: (sql) => statement(sql) }, BUCKET: {}, BOOKING_BRIDGE_SECRET: 'b'.repeat(32) },
  pool: { query: async (sql) => {
    if (!/JOIN owner_sessions/.test(sql)) unexpected(sql);
    if (world.dbDown) throw Error('connection refused');
    return { rows: world.actor?.kind === 'owner' ? [OWNER_ROW] : [] };
  } },
  execute: async () => unexpected('the engine'),
  index_html_default: '<html><head></head><body></body></html>',
  path2: path, fs2: fs, fileURLToPath, Readable2: Readable,
  GET: () => unexpected('a page'), GET2: () => unexpected('a page'), GET3: () => unexpected('a page'), getFile: async () => unexpected('a file'),
  htmlResponse: () => unexpected('a page'),
  http: { createServer: (cb) => { onRequest = cb; return { listen() {} }; } },
  fetch: async (url, options) => {
    world.requests.push({ url, cache: options?.cache ?? null, timeoutMs: options?.signal?.timeoutMs ?? null });
    const answer = world.upstream(url);
    if (answer.fail) throw new TypeError('fetch failed');
    return new Response(answer.body, { status: answer.status });
  },
  AbortSignal: { timeout: (ms) => ({ timeoutMs: ms }) },
  Date: Clock,
  console: { error() {}, log() {} },
};
for (const n of ['createHash', 'createHash3', 'createHash4', 'createHash5', 'createHash6', 'createHash7']) scope[n] = crypto.createHash;
for (const n of ['randomUUID2', 'randomUUID5', 'randomUUID6', 'randomUUID7', 'randomUUID8', 'randomUUID9']) scope[n] = () => '00000000-0000-4000-8000-000000000000';
for (const n of ['timingSafeEqual', 'timingSafeEqual2', 'timingSafeEqual3']) scope[n] = crypto.timingSafeEqual;
for (const n of ['randomBytes', 'randomBytes2']) scope[n] = crypto.randomBytes;
for (const n of ['scrypt', 'scrypt2']) scope[n] = crypto.scrypt;

const body = [
  slice('function identity(request) {', 'function htmlResponse('),
  slice('// src/online/lib/bot-rates.ts', '// src/online/lib/ledger.ts'),
  slice('var zero = () =>', '// src/online/lib/engine.mjs'),
  slice('// src/online/app/documents/route.ts', null),
].join('\n').replace(/^import \{[^}]*\} from "node:crypto";$/gm, '').replace('import.meta.url', '"file:///v2/server.mjs"');
if (/^import /m.test(body) || body.includes('import.meta')) throw Error('an import survived; the stand-in list is out of date');
// A name the bundle declares must not also be handed in, or the hand-in would win silently.
for (const n of Object.keys(scope)) if (new RegExp('^(?:var|async function|function|class) ' + n + '\\b', 'm').test(body)) throw Error(n + ' is declared by v2 and must not be replaced');
const v2 = new Function(...Object.keys(scope), body + '\nreturn { fetchBOT, BOT_SOURCE, base, roles };')(...Object.values(scope));
if (typeof onRequest !== 'function') throw Error('v2 did not hand over its request callback');

// ── what the bank publishes ─────────────────────────────────────────────────
const NOW = Date.parse('2026-10-06T11:20:00.000Z');
const iso = (ms) => new Date(ms).toISOString().replace('.000Z', 'Z');
// The bank's label document carries 29 keys; the four below are the ones v2 reads.
const labelsDoc = (over = {}) => ({
  tab1Label: 'อัตราแลกเปลี่ยนถัวเฉลี่ยธนาคารพาณิชย์', currentTime: '06:20 PM',
  level1StartTime: iso(NOW - 19 * 60e3), level1EndTime: iso(NOW - 5 * 60e3),
  level2StartTime: iso(NOW - 5 * 60e3), level2EndTime: iso(NOW + 10 * 60e3), ...over,
});
const bankRow = (currency_id, buying_sight, buying_transfer, selling, over = {}) => ({
  period: '2026-10-06', currency_id, countryName: currency_id, currency_name_th: currency_id, currency_name_eng: currency_id,
  buying_sight, buying_transfer, selling, flagPath: '/content/dam/bot/currencyflags/' + currency_id + '.png', sortOrder: 1, ...over,
});
// The bank's own order, with currencies v2 does not keep in between. The yen is quoted per 100.
const standardRows = () => [
  bankRow('USD', '33.4218', '33.5008', '33.8278'),
  bankRow('GBP', '44.0195', '44.1299', '44.9090'),
  bankRow('EUR', '37.3552', '37.4413', '38.0844'),
  bankRow('JPY', '20.9500', '21.0100', '21.6300'),
  bankRow('HKD', '4.2610', '4.2729', '4.3508'),
  bankRow('SGD', '25.6000', '25.7000', '26.3000'),
  bankRow('AUD', '23.0133', '23.0712', '23.7839'),
  bankRow('MMK', '', '', '0.0170'),
  bankRow('CNY', '4.9541', '4.9767', '5.0667'),
];
const resultsDoc = (rows = standardRows(), over = {}) => ({
  tab1Heading: 'อัตราแลกเปลี่ยนเฉลี่ยของธนาคารพาณิชย์ในกรุงเทพมหานคร', lastUpdated: 'ประจำวันที่ 06 ตุลาคม 2569',
  responseContent: rows, futureDateFlag: false, noDataFoundMessage: 'ไม่มีข้อมูล หรือวันที่เลือกเป็นวันหยุดธนาคาร', noDataFoundFlag: false, ...over,
});
const text = (v) => (typeof v === 'string' ? v : JSON.stringify(v));
const ok = (v) => ({ status: 200, body: text(v) });
const LABELS = '.labels.json';
const LEVEL = (n) => '.results.level' + n + 'cache.json';
/** `script` maps a path suffix to `{status, body}` or `{fail: true}`; a path it lacks answers 404. */
const upstreamFrom = (script) => (url) => {
  if (!url.startsWith(v2.base)) throw Error('v2 fetched outside its base: ' + url);
  return script[url.slice(v2.base.length)] ?? { status: 404, body: 'Not Found' };
};
const everyLevel = (answer) => ({ [LEVEL(1)]: answer, [LEVEL(2)]: answer, [LEVEL(3)]: answer });
const standardScript = () => ({ [LABELS]: ok(labelsDoc()), ...everyLevel(ok(resultsDoc())) });

// ── decimals as text ────────────────────────────────────────────────────────
/** "20.9500" over 100 → "0.2095"; "33.0000" over 1 → "33". Text in, text out: no float. */
function exact(raw, unit) {
  const s = String(raw);
  if (!/^\d+(\.\d+)?$/.test(s)) throw Error('not a plain decimal: ' + JSON.stringify(raw));
  let [int, frac = ''] = s.split('.');
  if (unit === 100) { int = int.padStart(3, '0'); frac = int.slice(-2) + frac; int = int.slice(0, -2); } else if (unit !== 1) throw Error('unit ' + unit);
  int = int.replace(/^0+(?=\d)/, ''); frac = frac.replace(/0+$/, '');
  return frac ? int + '.' + frac : int;
}
/** What v3 must answer for a result v2 gave: the same document with every computed rate as its exact decimal text. */
function expected(result) {
  let differs = false;
  const rows = result.rows.map((r) => {
    const out = { code: r.code, date: r.date, unit: r.unit };
    for (const k of ['buyingSight', 'buyingTransfer', 'selling']) {
      out[k] = exact(r.original[k], r.unit);
      if (Math.abs(Number(out[k]) - r[k]) > 1e-12) throw Error(`${r.code}.${k}: v2 computed ${r[k]}, the decimal is ${out[k]}`);
      if (JSON.stringify(r[k]) !== out[k]) { differs = true; if (r.unit === 1) throw Error(`${r.code}.${k}: v2 printed ${JSON.stringify(r[k])} for ${out[k]} without dividing`); }
    }
    out.original = r.original;
    return out;
  });
  return { answer: { source: result.source, fetchedAt: result.fetchedAt, kind: result.kind, rows }, differs };
}

// ═══ mode: source ═══════════════════════════════════════════════════════════
const SOURCE_SENTENCES = ['ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท'];

// The same document is served by several paths and in many cases: each is written once.
const bodies = [];
const bodyIndex = (text) => { let i = bodies.indexOf(text); if (i < 0) i = bodies.push(text) - 1; return i; };

async function sourceCase(name, script, now = NOW) {
  world.now = now; world.requests = []; world.upstream = upstreamFrom(script);
  let result = null, error = null;
  try { result = await v2.fetchBOT(); } catch (e) { error = e.message; }
  if (error !== null && !SOURCE_SENTENCES.includes(error)) throw Error(`${name}: unexpected sentence ${JSON.stringify(error)}`);
  for (const r of world.requests) if (r.cache !== 'no-store' || r.timeoutMs !== 10000) throw Error(`${name}: v2 fetched with ${JSON.stringify(r)}`);
  const upstream = {};
  for (const [suffix, a] of Object.entries(script)) upstream[suffix] = a.fail ? { fail: true } : { status: a.status, body: bodyIndex(a.body) };
  const out = { name, now: new Date(now).toISOString(), upstream, requested: world.requests.map((r) => r.url.slice(v2.base.length)), v2Error: error };
  if (result) {
    const e = expected(result);
    out.v2Body = JSON.stringify(result); out.differsFromV2 = e.differs; out.answer = e.answer;
  }
  return out;
}

async function sourceMode() {
  const cases = [];
  const add = async (...a) => cases.push(await sourceCase(...a));
  const withRows = (rows, over) => ({ [LABELS]: ok(labelsDoc()), ...everyLevel(ok(resultsDoc(rows, over))) });
  const change = (code, over) => standardRows().map((r) => (r.currency_id === code ? { ...r, ...over } : r));
  const without = (code) => standardRows().filter((r) => r.currency_id !== code);
  // each level answers with a different dollar rate, so the fixture shows which one was read
  const levels = () => ({
    [LEVEL(1)]: ok(resultsDoc(change('USD', { selling: '33.0001' }))),
    [LEVEL(2)]: ok(resultsDoc(change('USD', { selling: '33.0002' }))),
    [LEVEL(3)]: ok(resultsDoc(change('USD', { selling: '33.0003' }))),
  });
  const at = (labels) => ({ [LABELS]: ok(labels), ...levels() });
  const m = 60e3;

  // ── the plain path ──
  await add('the six currencies, in the bank\'s order, the rest dropped', standardScript());
  await add('the answer is stamped with the time of the call', standardScript(), Date.parse('2026-12-31T16:59:59.987Z'));
  await add('rates that end in zeros, and whole numbers', withRows(change('USD', { buying_sight: '33.4200', buying_transfer: '33.0000', selling: '34.1000' })));
  await add('the yen is per 100: a rate v2 gets right', withRows(change('JPY', { buying_sight: '21.0000', buying_transfer: '21.5000', selling: '21.6300' })));
  await add('the yen is per 100: rates v2\'s arithmetic bends', withRows(change('JPY', { buying_sight: '20.9382', buying_transfer: '21.0107', selling: '21.6311' })));
  await add('the yen below one baht per 100', withRows(change('JPY', { buying_sight: '0.9382', buying_transfer: '0.9500', selling: '1.0007' })));
  await add('rates the bank sends as numbers', withRows(change('EUR', { buying_sight: 37.3552, buying_transfer: 37.4413, selling: 38 })));
  await add('the six currencies and nothing else', withRows(standardRows().filter((r) => ['USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD'].includes(r.currency_id))));
  await add('another order from the bank is kept', withRows(standardRows().reverse()));
  await add('a date that is no real day still has the shape', withRows(change('GBP', { period: '2026-13-45' })));
  await add('each row carries its own date', withRows(change('AUD', { period: '2026-10-05' })));

  // ── which of the bank's three documents is read ──
  await add('inside the first window: level 1', at(labelsDoc({ level1StartTime: iso(NOW - m), level1EndTime: iso(NOW + m), level2StartTime: iso(NOW + m), level2EndTime: iso(NOW + 2 * m) })));
  await add('at the first window\'s start: level 1', at(labelsDoc({ level1StartTime: iso(NOW), level1EndTime: iso(NOW + m), level2StartTime: iso(NOW + m), level2EndTime: iso(NOW + 2 * m) })));
  await add('at the first window\'s end, which is the second one\'s start: level 2', at(labelsDoc({ level1StartTime: iso(NOW - m), level1EndTime: iso(NOW), level2StartTime: iso(NOW), level2EndTime: iso(NOW + m) })));
  await add('inside the second window: level 2', at(labelsDoc({ level1StartTime: iso(NOW - 3 * m), level1EndTime: iso(NOW - 2 * m), level2StartTime: iso(NOW - 2 * m), level2EndTime: iso(NOW + m) })));
  await add('at the second window\'s end: level 3', at(labelsDoc({ level1StartTime: iso(NOW - 3 * m), level1EndTime: iso(NOW - 2 * m), level2StartTime: iso(NOW - 2 * m), level2EndTime: iso(NOW) })));
  await add('before both windows: level 3', at(labelsDoc({ level1StartTime: iso(NOW + m), level1EndTime: iso(NOW + 2 * m), level2StartTime: iso(NOW + 2 * m), level2EndTime: iso(NOW + 3 * m) })));
  await add('after both windows: level 3', at(labelsDoc({ level1StartTime: iso(NOW - 4 * m), level1EndTime: iso(NOW - 3 * m), level2StartTime: iso(NOW - 3 * m), level2EndTime: iso(NOW - m) })));
  await add('inside both windows: level 1', at(labelsDoc({ level1StartTime: iso(NOW - m), level1EndTime: iso(NOW + m), level2StartTime: iso(NOW - m), level2EndTime: iso(NOW + m) })));
  await add('a first window that cannot be read, inside the second: level 2', at(labelsDoc({ level1StartTime: 'soon', level1EndTime: iso(NOW + m), level2StartTime: iso(NOW - m), level2EndTime: iso(NOW + m) })));
  await add('windows that cannot be read: level 3', at(labelsDoc({ level1StartTime: 'x', level1EndTime: 'x', level2StartTime: '', level2EndTime: null })));
  await add('labels without the window keys: level 3', at({ tab1Label: 'x' }));
  await add('times with milliseconds and an offset are times', at(labelsDoc({ level1StartTime: '2026-10-06T18:19:59.999+07:00', level1EndTime: '2026-10-06T18:20:00.001+07:00', level2StartTime: iso(NOW + m), level2EndTime: iso(NOW + 2 * m) })));

  // ── the bank does not answer ──
  await add('labels: 500', { [LABELS]: { status: 500, body: 'oops' }, ...levels() });
  await add('labels: 404', { ...levels() });
  await add('labels: not JSON', { [LABELS]: ok('<html>maintenance</html>'), ...levels() });
  await add('labels: empty body', { [LABELS]: ok(''), ...levels() });
  await add('labels: no connection', { [LABELS]: { fail: true }, ...levels() });
  await add('rates: 500', { [LABELS]: ok(labelsDoc()), ...everyLevel({ status: 500, body: text(resultsDoc()) }) });
  await add('rates: 404', { [LABELS]: ok(labelsDoc()) });
  await add('rates: not JSON', { [LABELS]: ok(labelsDoc()), ...everyLevel(ok('<html>maintenance</html>')) });
  await add('rates: no connection', { [LABELS]: ok(labelsDoc()), ...everyLevel({ fail: true }) });
  await add('only the level asked for is read: level 2 down, the others up', { [LABELS]: ok(labelsDoc()), [LEVEL(1)]: ok(resultsDoc()), [LEVEL(2)]: { status: 503, body: '' }, [LEVEL(3)]: ok(resultsDoc()) });

  // ── the bank answers something that is not a full table ──
  await add('the bank says it has no data', withRows(standardRows(), { noDataFoundFlag: true }));
  await add('no data and no rows', withRows([], { noDataFoundFlag: true }));
  await add('no rows', withRows([]));
  await add('rows are not a list', withRows({ USD: 1 }));
  await add('rows are missing', { [LABELS]: ok(labelsDoc()), ...everyLevel(ok({ lastUpdated: 'x', noDataFoundFlag: false })) });
  for (const code of ['USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD']) await add('no ' + code, withRows(without(code)));
  await add('the dollar twice', withRows([...standardRows(), bankRow('USD', '33.4218', '33.5008', '33.8278')]));
  await add('the dollar twice and no yuan: six rows, five currencies', withRows([...without('CNY'), bankRow('USD', '33.4218', '33.5008', '33.8278')]));
  const bad = { 'empty': '', 'zero': '0', 'zero with decimals': '0.0000', 'negative': '-33.1', 'a word': 'n/a', 'null': null, 'a comma for the point': '33,42' };
  for (const [what, value] of Object.entries(bad)) for (const field of ['buying_sight', 'buying_transfer', 'selling']) await add(`${field} of the dollar: ${what}`, withRows(change('USD', { [field]: value })));
  await add('buying_sight of the dollar: missing', withRows(standardRows().map((r) => { if (r.currency_id !== 'USD') return r; const { buying_sight, ...rest } = r; return rest; })));
  await add('selling of the yuan, the last row: zero', withRows(change('CNY', { selling: '0' })));
  await add('a bad rate in a currency v2 does not keep is not looked at', withRows(change('HKD', { buying_sight: 'n/a', selling: '-1' })));
  for (const [what, value] of Object.entries({ 'day first': '06/10/2026', 'one-digit day': '2026-10-6', 'with a time': '2026-10-06T00:00:00Z', 'empty': '', 'a Buddhist-era sentence': '06 ตุลาคม 2569' })) await add('date of the euro: ' + what, withRows(change('EUR', { period: value })));
  await add('a bad date in a currency v2 does not keep is not looked at', withRows(change('SGD', { period: 'yesterday' })));

  if (cases.filter((c) => c.differsFromV2).length < 2) throw Error('no case shows v2\'s float arithmetic; the yen cases are not doing their job');
  return {
    source: 'v2 reference-rate fetch extracted unchanged from the deployed bundle (xchange-online@3ffc5a5): normalizeBOT, fetchBOT (server.mjs:5925-5956), driven by gen_reference_rates.mjs source',
    base: v2.base, labelsSuffix: LABELS, levelSuffixes: [LEVEL(1), LEVEL(2), LEVEL(3)],
    sourcePage: v2.BOT_SOURCE, kind: 'DAILY_COMMERCIAL_BANK_AVERAGE', timeoutMs: 10000, sentence: SOURCE_SENTENCES[0],
    note: 'upstream[suffix].body is an index into `bodies`; a suffix the case lacks answers 404; fail = the connection ends without an answer. answer = what v3 must give: v2\'s document with each computed rate as exact decimal text (the JSON number v3 writes prints as that text). v2Body = the bytes v2 gave; differsFromV2 marks the cases where v2\'s float division shows.',
    bodies, cases,
  };
}

// ═══ mode: route ════════════════════════════════════════════════════════════
const TOKEN = 'a'.repeat(64);
const ROUTE_SENTENCES = new Map(Object.entries({ 'กรุณาเข้าสู่ระบบ': 401, 'เฉพาะเจ้าของระบบเท่านั้น': 403, 'ORIGIN_REJECTED': 400, [SOURCE_SENTENCES[0]]: 400, 'เซิร์ฟเวอร์ขัดข้อง': 500, 'Not found': 404 }));

/** One request through v2's `http.createServer` callback. */
async function send(c) {
  const actor = c.caller === 'owner' ? { kind: 'owner' } : c.caller === 'staff' ? { kind: 'staff', role: c.role } : c.caller === 'lookup fails' ? { kind: 'owner' } : null;
  world.actor = actor; world.dbDown = c.caller === 'lookup fails'; world.now = NOW; world.requests = [];
  world.upstream = upstreamFrom(c.upstream === 'fails' ? { [LABELS]: { status: 503, body: '' } } : standardScript());
  const headers = {};
  for (const [k, v] of c.headers) headers[k.toLowerCase()] = v;
  if (actor) headers.cookie = (actor.kind === 'owner' ? '__Host-xc_owner=' : '__Host-xc_staff=') + TOKEN;
  const req = Readable.from(c.body === null ? [] : [Buffer.from(c.body)]);
  Object.assign(req, { method: c.method, url: c.path, headers });
  const got = { status: null, headers: {}, chunks: [] };
  await new Promise((resolve, reject) => {
    const res = {
      headersSent: false,
      writeHead(status, h) { got.status = status; got.headers = Object.fromEntries(Object.entries(h || {}).map(([k, v]) => [k.toLowerCase(), v])); this.headersSent = true; },
      write(chunk) { got.chunks.push(Buffer.from(chunk)); },
      end(chunk) { if (chunk) got.chunks.push(Buffer.from(chunk)); resolve(); },
    };
    onRequest(req, res).catch(reject);
  });
  const bodyText = Buffer.concat(got.chunks).toString('utf8');
  let sentence = bodyText;
  try { const j = JSON.parse(bodyText); if (j && typeof j.error === 'string') sentence = j.error; } catch { /* plain text */ }
  if (got.status !== 200 && ROUTE_SENTENCES.get(sentence) !== got.status) throw Error(`${c.name}: unexpected answer ${got.status} ${JSON.stringify(bodyText.slice(0, 120))}`);
  return { status: got.status, contentType: got.headers['content-type'] ?? null, cacheControl: got.headers['cache-control'] ?? null, body: bodyText, upstreamRequests: world.requests.length };
}

async function routeMode() {
  const HOST = 'desk.test', ORIGIN = 'http://desk.test', PATH = '/api/reference-rates';
  const cases = [];
  const add = async (name, over = {}) => {
    const c = { name, caller: 'owner', role: null, method: 'POST', path: PATH, host: HOST, origin: ORIGIN, xreq: '1', proto: undefined, body: '{}', contentType: 'application/json', upstream: 'answers', extra: [], ...over };
    const headers = [];
    if (c.host !== null) headers.push(['Host', c.host]);
    if (c.origin !== null) headers.push(['Origin', c.origin]);
    if (c.xreq !== null) headers.push(['X-Xchange-Request', c.xreq]);
    if (c.proto !== undefined) headers.push(['X-Forwarded-Proto', c.proto]);
    if (c.body !== null && c.contentType !== null) headers.push(['Content-Type', c.contentType]);
    headers.push(...c.extra);
    const request = { name: c.name, caller: c.caller, role: c.role, method: c.method, path: c.path, headers, body: c.body, upstream: c.upstream };
    cases.push({ ...request, v2: await send(request) });
  };

  // ── who may call ──
  await add('the owner');
  await add('nobody signed in', { caller: 'anonymous' });
  for (const role of v2.roles) await add('staff: ' + role, { caller: 'staff', role });
  // ── the bank does not answer ──
  await add('the owner, the bank down', { upstream: 'fails' });
  await add('staff, the bank down: refused before the bank is asked', { caller: 'staff', role: 'RATES', upstream: 'fails' });
  await add('nobody signed in, the bank down', { caller: 'anonymous', upstream: 'fails' });
  // ── the same-origin rule ──
  await add('no Origin', { origin: null });
  await add('Origin of another site', { origin: 'http://evil.test' });
  await add('Origin "null"', { origin: 'null' });
  await add('Origin with a trailing slash', { origin: ORIGIN + '/' });
  await add('Origin with a path', { origin: ORIGIN + PATH });
  await add('https Origin on a plain-http request', { origin: 'https://desk.test' });
  await add('Origin with another port', { origin: 'http://desk.test:8093' });
  await add('Origin in capitals', { origin: 'http://DESK.test' });
  await add('Origin with the scheme in capitals', { origin: 'HTTP://desk.test' });
  await add('X-Forwarded-Proto https and an https Origin', { proto: 'https', origin: 'https://desk.test' });
  await add('X-Forwarded-Proto https and an http Origin', { proto: 'https' });
  await add('X-Forwarded-Proto http', { proto: 'http' });
  await add('X-Forwarded-Proto HTTPS is not https', { proto: 'HTTPS', origin: 'https://desk.test' });
  await add('X-Forwarded-Proto HTTPS leaves the request plain http', { proto: 'HTTPS' });
  await add('a Host with a port and the same Origin', { host: 'desk.test:8093', origin: 'http://desk.test:8093' });
  await add('a Host with a port and an Origin without it', { host: 'desk.test:8093' });
  await add('a Host in capitals and the Origin a browser sends for it', { host: 'DESK.Test' });
  await add('a Host naming the default port and the Origin a browser sends for it', { host: 'desk.test:80' });
  await add('https, a Host naming port 443', { host: 'desk.test:443', proto: 'https', origin: 'https://desk.test' });
  await add('X-Forwarded-Host of another site, with that site as Origin', { origin: 'http://evil.test', extra: [['X-Forwarded-Host', 'evil.test']] });
  await add('X-Forwarded-Host of another site, with the real Origin', { extra: [['X-Forwarded-Host', 'evil.test']] });
  await add('Forwarded naming another host and https, with that as Origin', { origin: 'https://evil.test', extra: [['Forwarded', 'for=10.0.0.1;host=evil.test;proto=https']] });
  await add('Forwarded naming https, with an https Origin', { origin: 'https://desk.test', extra: [['Forwarded', 'proto=https']] });
  await add('no X-Xchange-Request', { xreq: null });
  await add('X-Xchange-Request: 0', { xreq: '0' });
  await add('X-Xchange-Request: true', { xreq: 'true' });
  await add('X-Xchange-Request: 11', { xreq: '11' });
  await add('neither Origin nor X-Xchange-Request', { origin: null, xreq: null });
  await add('the bank down and no Origin: the bank is not asked', { origin: null, upstream: 'fails' });
  // ── the caller is judged before the request ──
  await add('staff without Origin', { caller: 'staff', role: 'MANAGER', origin: null });
  await add('staff without X-Xchange-Request', { caller: 'staff', role: 'FRONT', xreq: null });
  await add('nobody signed in, without Origin', { caller: 'anonymous', origin: null });
  await add('nobody signed in, without either header', { caller: 'anonymous', origin: null, xreq: null });
  // ── the body is never read ──
  await add('no body', { body: null });
  await add('an empty body', { body: '' });
  await add('a body that is not JSON', { body: '{' });
  await add('a JSON array', { body: '[1,2,3]' });
  await add('an action nobody defined', { body: '{"action":"nope","payload":{"branchId":"B2"}}' });
  await add('plain text', { body: 'hello', contentType: 'text/plain' });
  await add('a body without Content-Type', { body: '{}', contentType: null });
  await add('a long body', { body: JSON.stringify({ pad: 'x'.repeat(20000) }) });
  await add('nobody signed in, a body that is not JSON', { caller: 'anonymous', body: '{' });
  // ── the path and the method ──
  await add('a query string', { path: PATH + '?branch=B2&x=1' });
  await add('a trailing slash', { path: PATH + '/' });
  await add('something under the path', { path: PATH + '/latest' });
  await add('the path in capitals', { path: '/API/REFERENCE-RATES' });
  await add('a near miss of the path', { path: '/api/reference-rate' });
  for (const method of ['GET', 'HEAD', 'PUT', 'PATCH', 'DELETE', 'OPTIONS']) await add(method + ' by the owner', { method, body: null });
  await add('GET by nobody: not a redirect to the sign-in page', { caller: 'anonymous', method: 'GET', body: null });
  await add('GET by staff', { caller: 'staff', role: 'RATES', method: 'GET', body: null });
  await add('PUT by nobody', { caller: 'anonymous', method: 'PUT', body: null });
  await add('DELETE by staff', { caller: 'staff', role: 'SYSTEM', method: 'DELETE', body: null });
  // ── the caller cannot be looked up ──
  await add('the session look-up fails', { caller: 'lookup fails' });

  const okCase = cases[0];
  if (okCase.v2.status !== 200 || okCase.v2.upstreamRequests !== 2) throw Error('the plain case did not reach the bank');
  const e = expected(JSON.parse(okCase.v2.body));
  if (e.differs) throw Error('the standard table must be one v2 computes exactly, so its 200 body can be compared as it is');
  for (const c of cases) if (c.v2.status === 200 && c.v2.body !== okCase.v2.body) throw Error(c.name + ': a 200 with another body');
  return {
    source: 'v2 route POST /api/reference-rates run unchanged from the deployed bundle (xchange-online@3ffc5a5) through its own http.createServer callback: toRequest, handle, the route gate, POST4, authorizeWrite, fetchBOT, send (server.mjs:5543-5547, 5925-5956, 7275-7282, 8159, 8235-8300), driven by gen_reference_rates.mjs route',
    path: PATH,
    sentences: { signIn: 'กรุณาเข้าสู่ระบบ', ownerOnly: 'เฉพาะเจ้าของระบบเท่านั้น', origin: 'ORIGIN_REJECTED', bankDown: SOURCE_SENTENCES[0], serverError: 'เซิร์ฟเวอร์ขัดข้อง' },
    note: 'caller "lookup fails" = v2 could not read the session (its database refused). upstream "answers" = the bank gives the standard table, "fails" = it answers 503. v2.upstreamRequests counts v2\'s requests to the bank (two per fetch: labels, then one level). answer = the reference rates the bank\'s standard table makes, which is also the 200 body.',
    answer: e.answer,
    cases,
  };
}

const out = mode === 'route' ? await routeMode() : await sourceMode();
// one case, and one upstream document, per line
const { cases, bodies: documents, ...head } = out;
const lines = (list) => '[\n' + list.map((x) => '  ' + JSON.stringify(x)).join(',\n') + '\n ]';
let json = JSON.stringify(head, null, 1).replace(/\n}$/, '');
if (documents) json += ',\n "bodies": ' + lines(documents);
json += ',\n "cases": ' + lines(cases) + '\n}\n';
if (JSON.stringify(JSON.parse(json)) !== JSON.stringify({ ...head, ...(documents ? { bodies: documents } : {}), cases })) throw Error('the writer changed the data');
process.stdout.write(json);
