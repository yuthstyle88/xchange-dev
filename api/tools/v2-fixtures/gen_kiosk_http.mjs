// Records what v2 answers on `POST /api/kiosk` — the one route of the customer kiosk, the
// desk's kiosk queue and the kiosk-currency page — for ticket T12c, re-recorded from the
// 10-06 bundle for row K-HTTP (`docs/design/xchange-v3.md` §5.5.3).
//   node gen_kiosk_http.mjs <v2 bundle dir> <postgres url of a scratch database> [out dir]
//   out dir defaults to ../../tests/fixtures; it gets kiosk/*.json
//
// Like gen_compat.mjs (whose loader, clock, id counter and file writer are repeated here
// unchanged, because that script runs on import) this runs the WHOLE bundle against a real
// PostgreSQL: every answer went through v2's own `handle(request)`. The bundle is the 10-06
// one (`XC-Desk-v2-Passport-Drawer-20261006.zip`, sha256 6514b4c8…; `server.mjs` sha256
// 99ddcd10…), with `schema.sql` and the three add-on modules `queue-work.mjs`,
// `payment-settings.mjs`, `passport-drawer.mjs` beside it. The bundle is not in this
// repository and nothing of it is copied here.
//
// The database: the schema `xc_kiosk_http_fixture` of the database named, made from v2's
// `schema.sql`, dropped for every scenario and at the end.
//
// Replaced, and nothing else: the listening statement is cut off; `Date` is a settable clock,
// in `server.mjs` and in the three add-on modules alike; PostgreSQL's `clock_timestamp()` (the
// time queue-work.mjs stamps a claim and a completion with) is that same clock: a function of
// that name in the fixture schema, found before `pg_catalog`'s because the search path names
// `pg_catalog` after the schema; `randomUUID` / `randomBytes` count; `fetch` answers the two
// Bank of Thailand addresses; v2's `console` is silent. Written beside v2's own statements: the `owners` row, the session
// rows, and `must_change` cleared on the employees v2 created.
//
// Beside the three scenario files it writes kiosk/behind.json — per file and step number, what
// happened behind an answer: `audit`, the rows v2 added to `device_events` while it answered
// (branch, actor, kind, ref, detail: the kiosk's audit lines), and for an accepted `configure`
// `bank`, the Bank of Thailand reference v2 kept with the rate it saved — and
// kiosk/currency_table.json, v2's table of the currencies a kiosk can offer (code, Thai and
// English name, countries), read off the first `manage` answer.
//
// The script fails, and writes nothing, when v2 answers a sentence a step did not expect,
// when a step expected a refusal and got none, when a sentence of SENTENCES is never said,
// or when a quote's figures are not what exact decimal arithmetic gives (v3 computes
// exactly; a step whose v2 figure is a floating-point accident must not be recorded —
// design §3, #67).
//   XC_KIOSK_DISCOVER=1 prints the sentences heard instead of writing files.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { pathToFileURL } from 'node:url';

const [bundleArg, databaseUrl, outArg] = process.argv.slice(2);
if (!bundleArg || !databaseUrl) { console.error('usage: node gen_kiosk_http.mjs <v2 bundle dir> <postgres url> [out dir]'); process.exit(2); }
const bundleDir = fs.statSync(bundleArg).isDirectory() ? bundleArg : path.dirname(bundleArg);
const outDir = path.resolve(outArg || new URL('../../tests/fixtures', import.meta.url).pathname);
const SCHEMA = 'xc_kiosk_http_fixture';

// ── clock and ids ───────────────────────────────────────────────────────────
const RealDate = Date;
const T0 = RealDate.parse('2026-10-06T05:00:00.000Z'); // 12:00 in Bangkok
let clock = T0;
class FakeDate extends RealDate {
  constructor(...a) { if (a.length) super(...a); else super(clock); }
  static now() { return clock; }
  // v2 passes its rows through structuredClone, which gives back plain dates; they must still count as dates
  static [Symbol.hasInstance](x) { return Object.prototype.toString.call(x) === '[object Date]'; }
}
let idCounter = 0, byteCounter = 0;
const idAt = (n) => '00000000-0000-4000-8000-' + n.toString(16).padStart(12, '0');
const sha = (x) => crypto.createHash('sha256').update(x).digest('hex');
const fakeCrypto = {
  ...crypto,
  randomUUID: () => idAt(++idCounter),
  randomBytes: (n) => { const out = []; while (out.length < n) out.push(...crypto.createHash('sha256').update('bytes' + ++byteCounter).digest()); return Buffer.from(out.slice(0, n)); },
};

// ── the Bank of Thailand, as v2 asks for it (server.mjs:5925-5957) ───────────
const BOT = { USD: ['32.0512', '32.1498', '32.4876'], EUR: ['37.2034', '37.3120', '37.9981'], GBP: ['42.6001', '42.7334', '43.5120'], JPY: ['21.3312', '21.4105', '21.9874'], CNY: ['4.3801', '4.4012', '4.5230'], AUD: ['20.6102', '20.7015', '21.3340'] };
let botDown = false;
const fakeFetch = async (url) => {
  if (botDown) return { ok: false, json: async () => ({}) };
  const json = (v) => ({ ok: true, json: async () => v });
  if (String(url).endsWith('statisticstable2.labels.json')) return json({ level1StartTime: '2026-10-06T00:00:00.000Z', level1EndTime: '2026-10-07T00:00:00.000Z', level2StartTime: '2026-10-07T00:00:00.000Z', level2EndTime: '2026-10-08T00:00:00.000Z' });
  if (/statisticstable2\.results\.level[123]cache\.json$/.test(String(url))) return json({ noDataFoundFlag: false, responseContent: Object.entries(BOT).map(([currency_id, [buying_sight, buying_transfer, selling]]) => ({ currency_id, period: '2026-10-05', buying_sight, buying_transfer, selling })) });
  throw Error('v2 fetched an address this script does not know: ' + url);
};

// ── load v2 ─────────────────────────────────────────────────────────────────
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'xc-kiosk-http-'));
const uploads = path.join(work, 'uploads');
let v2, sql;
async function load() {
  let src = fs.readFileSync(path.join(bundleDir, 'server.mjs'), 'utf8');
  const app = src.indexOf('\n// src/server.mjs\n'), tail = src.lastIndexOf('\nhttp.createServer(');
  if (app < 0 || tail < app) throw Error('server.mjs: the markers `// src/server.mjs` and `http.createServer(` were not found');
  // v2's own modules (after the bundled driver): their crypto imports become the counting ones.
  // The upload shim's bare `randomUUID` (temporary file names) is left alone.
  const head = src.slice(0, app), body = src.slice(app, tail).replace(/^import \{([^}]*)\} from "node:crypto";$/gm, (line, names) => /\bas\b|createHash|scrypt|randomBytes/.test(names) ? 'const {' + names.replace(/ as /g, ': ') + '} = globalThis.__XC_COMPAT.crypto;' : line);
  if ((body.match(/globalThis\.__XC_COMPAT\.crypto/g) || []).length !== 9) throw Error('server.mjs: expected nine crypto imports in v2\'s own modules');
  // the add-on modules, beside it, on the same clock
  for (const m of ['queue-work.mjs', 'payment-settings.mjs', 'passport-drawer.mjs']) {
    const text = fs.readFileSync(path.join(bundleDir, m), 'utf8');
    if (/\bDate\s*=/.test(text)) throw Error(m + ': assigns Date');
    fs.writeFileSync(path.join(work, m), 'const Date = globalThis.__XC_COMPAT.Date;\n' + text);
  }
  src = 'const Date = globalThis.__XC_COMPAT.Date, fetch = globalThis.__XC_COMPAT.fetch, console = { log() {}, error() {}, warn() {} };\n' + head + body + '\nexport { handle, pool };\n';
  fs.writeFileSync(path.join(work, 'server.mjs'), src);
  const url = new URL(databaseUrl);
  url.searchParams.set('options', '-c search_path=' + SCHEMA + ',pg_catalog');
  Object.assign(process.env, { DATABASE_URL: url.toString(), UPLOAD_DIR: uploads, BOOKING_BRIDGE_SECRET: 'fixture-bridge-secret-0123456789abcdef', AUTH_ENABLED: '1', DEVICES_ENABLED: '1', CUSTOMER_HOST: 'customer.test', PORT: '0' });
  for (const k of ['OWNER_ID', 'OWNER_EMAIL', 'DESK_HOST', 'ADMIN_ORIGIN']) delete process.env[k];
  globalThis.__XC_COMPAT = { Date: FakeDate, fetch: fakeFetch, crypto: fakeCrypto };
  v2 = await import(pathToFileURL(path.join(work, 'server.mjs')).href);
  sql = (text, args = []) => v2.pool.query(text, args);
}
async function freshDatabase() {
  await sql('DROP SCHEMA IF EXISTS ' + SCHEMA + ' CASCADE');
  await sql('CREATE SCHEMA ' + SCHEMA);
  // the database's clock is the script's: see the head of this file
  await sql('CREATE TABLE ' + SCHEMA + '.xc_fixture_clock (at timestamptz NOT NULL)');
  await sql('INSERT INTO ' + SCHEMA + '.xc_fixture_clock VALUES ($1)', [new RealDate(T0).toISOString()]);
  await sql('CREATE FUNCTION ' + SCHEMA + '.clock_timestamp() RETURNS timestamptz LANGUAGE sql STABLE AS $$ SELECT at FROM ' + SCHEMA + '.xc_fixture_clock $$');
  await sql(fs.readFileSync(path.join(bundleDir, 'schema.sql'), 'utf8'));
  fs.rmSync(uploads, { recursive: true, force: true });
  fs.mkdirSync(uploads, { recursive: true });
  clock = T0; idCounter = 0; byteCounter = 0;
}


// ── who calls ───────────────────────────────────────────────────────────────
const ORIGIN = 'http://desk.test';
const FOREVER = 4102444800000;
const DEFAULT_HEADERS = { 'cache-control': 'no-store', 'content-type': 'application/json' };
const RQ = (n) => idAt(0xa0000 + n);
const KIOSK = '/api/kiosk';
const DISCOVER = !!process.env.XC_KIOSK_DISCOVER;

// ── every sentence a refusing step may be answered with ─────────────────────
const SENTENCES = new Set(DISCOVER ? [] : [
  "Accept this request using your own account first.",
  "Amount is too small.",
  "Assigned staff required.",
  "Cannot destructure property 'action' of 'JSON.parse(...)' as it is null.",
  "Counter is closed. Please contact our staff.",
  "Expected ',' or '}' after property value in JSON at position 30 (line 1 column 31)",
  "Invalid branch.",
  "Invalid currency settings.",
  "Invalid payment method.",
  "Invalid receipt image.",
  "Invalid request.",
  "Not found",
  "Owner or Rates access required.",
  "Please enter a valid amount and currency.",
  "Please review your quote for 3 seconds.",
  "Quote expired. Please check the rate again.",
  "Rate changed or counter closed. Please start again.",
  "Request not found.",
  "Request not found. Please start again.",
  "Request rejected.",
  "Request too large.",
  "Select an exchange direction.",
  "Select cash or bank transfer to receive THB.",
  "Set valid counter buy and sell rates first.",
  "Staff access required.",
  "The assigned employee must confirm the passport check.",
  "Unexpected token 'o', \"not json\" is not valid JSON",
  "Unknown kiosk action.",
  "กรอกชื่อคู่ค้าและเรทต้นทุนให้ครบคู่",
  "เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน",
  "ไม่พบสาขาที่เลือก กรุณารีเฟรช",
  "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท",
  // the 10-06 bundle's queue work (queue-work.mjs)
  "คิวนี้ไม่พร้อมรับเรื่อง กรุณาโหลดล่าสุด",
  "ตัวกรองไม่ถูกต้อง",
  "หน้าข้อมูลไม่ถูกต้อง",
  "ออเดอร์นี้มี fr1 กำลังทำรายการแล้ว",
  "ออเดอร์นี้มี fr2 กำลังทำรายการแล้ว",
  "ออเดอร์นี้มี owner กำลังทำรายการแล้ว",
]);
const seen = new Map();

// ── exact decimal arithmetic, to tell a figure from a floating-point accident ─
const SCALE = 10n ** 10n;
const dec = (x) => { const s = String(x); if (!/^\d+(\.\d+)?$/.test(s)) throw Error('not a plain decimal: ' + s); const [i, f = ''] = s.split('.'); if (f.length > 10) throw Error('too many places: ' + s); return BigInt(i) * SCALE + BigInt(f.padEnd(10, '0')); };
const cent = SCALE / 100n;
const halfUp2 = (v) => (v + cent / 2n) / cent;      // → hundredths
const floor2 = (v) => v / cent;
function exactQuote(type, amount, rate) {
  const a = dec(amount), r = dec(rate);
  const foreign = type === 'BUY' ? halfUp2(a) : floor2(a * SCALE / r);
  const thb = halfUp2(foreign * cent * r / SCALE);
  return { foreign: Number(foreign) / 100, thb: Number(thb) / 100 };
}

class World {
  static async open(file, about) {
    await freshDatabase();
    const w = new World();
    Object.assign(w, { file, about, steps: [], people: [], tokens: new Map(), requestNo: 0, auditSeen: 0, behind: {} });
    await sql("INSERT INTO owners(id,username,name,password_hash,created_at) VALUES('owner-1','owner','Owner','-',$1)", [new FakeDate().toISOString()]);
    await w.session('owner', 'owner_sessions', 'owner-1');
    w.people.push({ key: 'owner', kind: 'owner', id: 'owner', username: 'owner', name: 'Owner', role: 'OWNER', branchId: null });
    return w;
  }
  async session(key, table, id) {
    const token = sha('session:' + key);
    await sql('INSERT INTO ' + table + ' VALUES($1,$2,1,$3)', [sha(token), id, FOREVER]);
    this.tokens.set(key, (table === 'owner_sessions' ? '__Host-xc_owner=' : '__Host-xc_staff=') + token);
  }
  /** An employee, created by v2's own `/api/employees` (not a recorded step); then signed in by a session row. */
  async employee(key, role, branchId = 'MAIN', opts = {}) {
    const username = key.toLowerCase(), name = key + ' ' + role[0] + role.slice(1).toLowerCase();
    const res = await this.send('owner', '/api/employees', JSON.stringify({ action: 'create', payload: { username, name, role, branchId, password: 'fixture-password-' + key } }));
    if (res.status !== 200) throw Error('employee ' + key + ': ' + res.text);
    const row = (await sql('SELECT id FROM employees WHERE username=$1', [username])).rows[0];
    if (!opts.mustChange) await sql('UPDATE employees SET must_change=0 WHERE id=$1', [row.id]);
    await this.session(key, 'employee_sessions', row.id);
    this.people.push({ key, kind: 'staff', id: row.id, username, name, role, branchId, ...(opts.mustChange ? { mustChange: true } : {}) });
  }
  async staff(list, branchId = 'MAIN') { for (const [key, role] of list) await this.employee(key, role, branchId); }
  rq() { return RQ(++this.requestNo); }
  async send(as, route, body, { method = 'POST', headers = {} } = {}) {
    const h = { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json', ...(as && this.tokens.get(as) ? { cookie: this.tokens.get(as) } : {}), ...headers };
    for (const k of Object.keys(h)) if (h[k] === null) delete h[k];
    const init = { method, headers: h };
    if (method !== 'GET' && method !== 'HEAD' && body !== undefined) init.body = body;
    await sql('UPDATE ' + SCHEMA + '.xc_fixture_clock SET at = $1', [new RealDate(clock).toISOString()]);
    const res = await v2.handle(new Request(ORIGIN + route, init));
    const text = await res.text(), got = Object.fromEntries([...res.headers].filter(([k]) => k !== 'set-cookie'));
    return { status: res.status, headers: got, text };
  }
  async audit() {
    const rows = (await sql('SELECT seq,branch_id,actor,kind,ref,detail FROM device_events WHERE seq>$1 ORDER BY seq', [this.auditSeen])).rows;
    if (rows.length) this.auditSeen = Number(rows[rows.length - 1].seq);
    return rows.filter((r) => r.kind.startsWith('KIOSK_')).map((r) => ({ branch: r.branch_id, actor: r.actor, kind: r.kind, ref: r.ref, detail: JSON.parse(r.detail) }));
  }
  async record(step, res, expect) {
    step = { n: this.steps.length + 1, at: new FakeDate().toISOString(), ...step, status: res.status };
    if (JSON.stringify(res.headers) !== JSON.stringify(DEFAULT_HEADERS)) step.headers = res.headers;
    let json;
    try { json = JSON.parse(res.text); } catch { json = undefined; }
    if (json !== undefined && JSON.stringify(json) === res.text) step.body = json; else step.text = res.text;
    const refused = res.status >= 400;
    const where = `${this.file} step ${step.n} (${step.as} ${step.route} ${step.action ?? step.raw ?? ''})`;
    if (expect === 'ok' && refused) throw Error(`${where}: expected v2 to accept, got ${res.status} ${res.text.slice(0, 300)}`);
    if (expect === 'no' || (expect === null && refused)) {
      if (!refused) throw Error(`${where}: expected v2 to refuse, got ${res.status} ${res.text.slice(0, 200)}`);
      const sentence = json && typeof json.error === 'string' ? json.error : res.text;
      if (!DISCOVER && !SENTENCES.has(sentence)) throw Error(`${where}: unexpected sentence ${JSON.stringify(sentence)} (${res.status})`);
      seen.set(sentence, (seen.get(sentence) || 0) + 1);
      if (process.env.XC_KIOSK_TRACE) console.error(`${this.file}#${step.n}\t${step.as}\t${step.action ?? (step.raw ?? '').slice(0, 40)}\t${res.status}\t${sentence}\t${step.note ?? ''}`);
    }
    const audit = await this.audit();
    if (refused && audit.length && step.route === KIOSK) throw Error(`${where}: a refused kiosk step wrote an audit line`);
    if (audit.length) (this.behind[step.n] ??= {}).audit = audit;
    if (!refused && step.route === KIOSK && step.action === 'configure') {
      // what the Bank of Thailand answered while the rate was saved: kept with the rate
      const branch = this.people.find((p) => p.key === step.as)?.branchId ?? step.payload.branchId;
      const rate = (await this.model(branch ? { branchId: branch } : {})).rates.find((r) => r.code === step.payload.code);
      (this.behind[step.n] ??= {}).bank = rate.botReference;
    }
    this.steps.push(step);
    return json;
  }
  /** A step on `/api/rpc`: what sets the counter up, or shows what a kiosk step left. Moves the clock a minute. */
  async rpc(as, action, payload = {}, note) {
    clock += 60000;
    const res = await this.send(as, '/api/rpc', JSON.stringify({ action, payload }));
    return this.record({ as, method: 'POST', route: '/api/rpc', action, payload, ...(note ? { note } : {}) }, res, 'ok');
  }
  model(payload = {}) { return this.send('owner', '/api/rpc', JSON.stringify({ action: 'adminSnapshot', payload })).then((r) => JSON.parse(r.text)); }
  async rate(code, buy, sell, branchId) {
    const m = await this.model(branchId ? { branchId } : {});
    return this.rpc('owner', 'adminSaveRate', { requestId: this.rq(), code, buy, sell, previousUpdated: m.rates.find((r) => r.code === code)?.updated || '', ...(branchId ? { branchId } : {}) });
  }
  async updated(code, branchId) { const m = await this.model(branchId ? { branchId } : {}); return m.rates.find((r) => r.code === code)?.updated || ''; }
  /** One kiosk action. `after` is how far the clock moves before it (default one second). */
  async k(expect, as, action, payload = {}, note, after = 1000) {
    clock += after;
    const res = await this.send(as, KIOSK, JSON.stringify({ action, payload }));
    const json = await this.record({ as: as ?? 'nobody', method: 'POST', route: KIOSK, action, payload, ...(note ? { note } : {}) }, res, expect);
    if (expect === 'ok' && action === 'quote') {
      const want = exactQuote(json.type, json.inputAmount, json.rate);
      if (want.foreign !== json.foreign || want.thb !== json.thb) throw Error(`${this.file} step ${this.steps.length}: v2's figures (${json.foreign}, ${json.thb}) are not the exact ones (${want.foreign}, ${want.thb}); choose another amount`);
    }
    return json;
  }
  ok(as, action, payload, note, after) { return this.k('ok', as, action, payload, note, after); }
  no(as, action, payload, note, after) { return this.k('no', as, action, payload, note, after); }
  /** A request written out by hand. The clock moves a second. */
  async raw(expect, as, body, opts = {}, note, route = KIOSK) {
    clock += 1000;
    const res = await this.send(as, route, body, opts);
    const step = { as: as ?? 'nobody', method: opts.method || 'POST', route, ...(body === undefined ? {} : body.length > 400 ? { rawLength: body.length, rawStart: body.slice(0, 60) } : { raw: body }), ...(opts.headers ? { requestHeaders: opts.headers } : {}), ...(note ? { note } : {}) };
    return this.record(step, res, expect);
  }
}

// ── writing: one step per line; values that repeat inside a file are written once ──
function fileText(w) {
  const text = new Map(), seen = new Map();
  const ser = (v) => { if (v === null || typeof v !== 'object') return JSON.stringify(v); let t = text.get(v); if (t === undefined) { t = Array.isArray(v) ? '[' + v.map(ser).join(',') + ']' : '{' + Object.entries(v).map(([k, x]) => JSON.stringify(k) + ':' + ser(x)).join(',') + '}'; text.set(v, t); seen.set(t, (seen.get(t) || 0) + 1); } return t; };
  for (const s of w.steps) { if (s.body !== undefined) ser(s.body); }
  const shared = new Map(); // text → key
  const put = (v) => {
    if (v === null || typeof v !== 'object') return JSON.stringify(v);
    const t = text.get(v);
    if (t.length >= 90 && seen.get(t) > 1) {
      if (!shared.has(t)) { const key = 's' + (shared.size + 1); shared.set(t, { key, body: null }); shared.get(t).body = inner(v); }
      return '{"$shared":"' + shared.get(t).key + '"}';
    }
    return inner(v);
  };
  const inner = (v) => Array.isArray(v) ? '[' + v.map(put).join(',') + ']' : '{' + Object.entries(v).map(([k, x]) => { if (k === '$shared') throw Error('v2 used the key $shared'); return JSON.stringify(k) + ':' + put(x); }).join(',') + '}';
  const lines = w.steps.map((s) => { const { body, ...rest } = s; const head = JSON.stringify(rest); return '  ' + (body === undefined ? head : head.slice(0, -1) + ',"body":' + put(body) + '}'); });
  const out = '{\n "source": ' + JSON.stringify('v2 10-06 bundle (server.mjs sha256 99ddcd10…, with queue-work.mjs) on PostgreSQL, every step through handle(); api/tools/v2-fixtures/gen_kiosk_http.mjs') + ',\n'
    + ' "about": ' + JSON.stringify(w.about) + ',\n'
    + ' "reading": ' + JSON.stringify('A step is one request. `at` is the server clock when it arrived; a step with `view` is a read taken at the clock of the step before it. `body` is the JSON v2 answered; wherever {"$shared":"sN"} stands, put `shared.sN` there (recursively) and serialise without spaces, keys in the order written: those are v2\'s bytes. `headers` is written only where it differs from `defaultHeaders`. `text` replaces `body` when the answer was not JSON.') + ',\n'
    + ' "origin": ' + JSON.stringify(ORIGIN) + ', "defaultHeaders": ' + JSON.stringify(DEFAULT_HEADERS) + ',\n'
    + ' "people": [\n' + w.people.map((p) => '  ' + JSON.stringify(p)).join(',\n') + '\n ],\n'
    + ' "shared": {\n' + [...shared.values()].map((x) => '  ' + JSON.stringify(x.key) + ': ' + x.body).join(',\n') + '\n },\n'
    + ' "steps": [\n' + lines.join(',\n') + '\n ]\n}\n';
  // the file must give back v2's bytes
  const parsed = JSON.parse(out), expand = (v) => Array.isArray(v) ? v.map(expand) : v && typeof v === 'object' ? (typeof v.$shared === 'string' ? expand(parsed.shared[v.$shared]) : Object.fromEntries(Object.entries(v).map(([k, x]) => [k, expand(x)]))) : v;
  parsed.steps.forEach((s, i) => { if (s.body !== undefined && JSON.stringify(expand(s.body)) !== JSON.stringify(w.steps[i].body)) throw Error(w.file + ' step ' + (i + 1) + ': the shared form does not give back the answer'); });
  return out;
}

// ════ scenarios ═════════════════════════════════════════════════════════════
const PNG_URL = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';
const padded = (n, ch = 'x') => { const head = '{"action":"rates","payload":{"pad":"', tail = '"}}'; return head + ch.repeat(n - head.length - tail.length) + tail; };

async function transport() {
  const w = await World.open('kiosk/transport.json', 'What POST /api/kiosk does with a request before any action runs: the two headers of the same-origin rule, the body, the size limit, the branch a request is for, and names it does not know. An empty shop: no rate, no shift.');
  await w.staff([['FR1', 'FRONT'], ['RT1', 'RATES']]);
  await w.employee('NEW', 'FRONT', 'MAIN', { mustChange: true });
  const rates = '{"action":"rates","payload":{}}';
  await w.raw('ok', null, rates, {}, 'the request the kiosk sends; nobody is signed in');
  await w.raw('ok', 'FR1', rates);
  await w.raw('ok', 'owner', rates);
  for (const who of [null, 'FR1', 'owner']) {
    await w.raw('no', who, rates, { headers: { origin: null } }, 'no Origin');
    await w.raw('no', who, rates, { headers: { origin: 'http://elsewhere.test' } }, 'another Origin');
    await w.raw('no', who, rates, { headers: { origin: 'http://desk.test/' } }, 'the Origin with a slash after it');
    await w.raw('no', who, rates, { headers: { 'x-xchange-request': null } }, 'no X-Xchange-Request');
    await w.raw('no', who, rates, { headers: { 'x-xchange-request': 'true' } }, 'X-Xchange-Request that is not 1');
  }
  await w.raw('ok', null, rates, { headers: { 'content-type': 'text/plain' } }, 'the content type is not looked at');
  await w.raw('ok', null, rates, { headers: { 'content-type': null } });
  await w.raw('ok', null, rates, { headers: { cookie: '__Host-xc_staff=' + 'f'.repeat(64) } }, 'a session that does not exist is nobody');
  await w.raw('no', null, '{"action":"notifications","payload":{}}', { headers: { cookie: '__Host-xc_staff=' + 'f'.repeat(64) } });
  await w.raw('no', null, rates, { headers: { origin: null, cookie: '__Host-xc_staff=short' } }, 'rejected before anything else');
  // the body
  await w.raw('no', null, '', {}, 'an empty body is {} — no action');
  await w.raw('no', 'FR1', '', {}, 'the same for an employee of the desk: another sentence');
  await w.raw('no', 'RT1', '');
  await w.raw('no', null, 'not json');
  await w.raw('no', null, 'null');
  await w.raw('no', null, '[]', {}, 'an array has no action');
  await w.raw('no', 'FR1', '"rates"');
  await w.raw('no', null, '{"action":"rates","payload":{}');
  await w.raw('ok', null, '{"action":"rates"}', {}, 'no payload is an empty payload');
  await w.raw('no', null, '{"action":"rates","payload":null}');
  await w.raw('no', null, '{"action":"rates","payload":[]}');
  await w.raw('no', null, '{"action":"rates","payload":"x"}');
  await w.raw('no', null, '{"action":"rates","payload":0}');
  await w.raw('no', null, '{"action":"rates","payload":1}');
  await w.raw('no', null, '{"action":"rates","payload":true}');
  await w.raw('ok', null, '{"payload":{},"action":"rates","more":true}', {}, 'other keys and another order');
  await w.raw('ok', null, ' \n{"action":"rates","payload":{}}\n ', {}, 'white space around');
  await w.raw('ok', null, '﻿' + rates, {}, 'a byte-order mark');
  await w.raw('ok', null, padded(2000000), {}, 'two million characters: the limit');
  await w.raw('no', null, padded(2000001), {}, 'one more');
  await w.raw('no', 'owner', padded(2000001));
  await w.raw('ok', null, padded(700000, 'ก'), {}, 'counted in characters, not bytes: 2.1 million bytes');
  // the method
  await w.raw('no', null, undefined, { method: 'GET' });
  await w.raw('no', 'owner', rates, { method: 'PUT' });
  // the branch
  await w.no(null, 'rates', { branchId: 'not a branch!' });
  await w.no(null, 'rates', { branchId: 'B-nowhere' }, 'a branch that does not exist');
  await w.no(null, 'rates', { branchId: 'x'.repeat(71) });
  await w.ok(null, 'rates', { branchId: '' }, 'an empty branch is MAIN');
  await w.ok(null, 'rates', { branchId: null });
  await w.ok(null, 'rates', { branchId: 0 });
  await w.no(null, 'rates', { branchId: {} });
  await w.no(null, 'rates', { branchId: ['MAIN', 'B'] });
  await w.ok('FR1', 'rates', { branchId: 'B-nowhere' }, 'an employee is answered for their own branch whatever they name');
  await w.ok('FR1', 'rates', { branchId: 'not a branch!' });
  await w.no('owner', 'rates', { branchId: 'B-nowhere' }, 'the owner is answered for the branch named');
  // names
  for (const who of [null, 'FR1', 'RT1', 'owner']) {
    await w.no(who, 'nothing', {}, 'a name the route does not know');
    await w.no(who, 'adminSnapshot', {});
  }
  await w.raw('no', 'FR1', '{"action":7,"payload":{}}');
  await w.raw('no', 'FR1', '{"action":["rates"],"payload":{}}', {}, 'a name must be the text itself');
  await w.raw('no', null, '{"action":"Rates","payload":{}}');
  // an employee who has not changed the first password
  await w.ok('NEW', 'rates', {}, 'an employee whose first password is unchanged');
  await w.k(null, 'NEW', 'notifications', {});
  await w.k(null, 'NEW', 'list', {});
  await w.k(null, 'NEW', 'nothing', {});
  await w.k(null, 'NEW', 'rates', { branchId: 'B-nowhere' }, 'whose branch they are answered for');
  return w;
}

async function currencies() {
  const w = await World.open('kiosk/currencies.json', 'The kiosk-currency page: manage and configure, by the owner and the rates role, and what the customer kiosk is offered afterwards. Two branches.');
  await w.staff([['FR1', 'FRONT'], ['RT1', 'RATES'], ['MG1', 'MANAGER'], ['MO1', 'MONITOR']]);
  await w.ok('owner', 'manage', {}, 'nothing set yet: the six defaults are on');
  await w.ok('RT1', 'manage', {});
  for (const who of [null, 'FR1', 'MG1', 'MO1']) { await w.no(who, 'manage', {}); await w.no(who, 'configure', { code: 'USD', enabled: true, buy: 32.1, sell: 32.6 }); }
  const cfg = (o) => ({ code: 'USD', enabled: true, buy: 32.1, sell: 32.6, previousUpdated: '', ...o });
  await w.no('owner', 'configure', cfg({ code: 'XXX' }));
  await w.no('owner', 'configure', cfg({ code: 'THB' }), 'the baht is not a kiosk currency');
  await w.no('owner', 'configure', cfg({ code: 'usd' }));
  await w.no('owner', 'configure', cfg({ code: undefined }));
  await w.no('owner', 'configure', cfg({ enabled: 'true' }), 'enabled must be true or false itself');
  await w.no('owner', 'configure', cfg({ enabled: 1 }));
  await w.no('owner', 'configure', cfg({ enabled: undefined }));
  await w.no('owner', 'configure', cfg({ buy: 0 }));
  await w.no('owner', 'configure', cfg({ buy: 33, sell: 32.6 }), 'sell below buy');
  await w.no('owner', 'configure', cfg({ buy: 'abc' }));
  await w.no('owner', 'configure', cfg({ buy: undefined }));
  await w.no('owner', 'configure', cfg({ sell: null }));
  await w.no('owner', 'configure', cfg({ buy: -1, sell: 5 }));
  await w.ok('owner', 'configure', cfg({}), 'the dollar: rate saved, switch stored, one audit line', 60000);
  await w.ok(null, 'rates', {}, 'the kiosk is offered the dollar; no shift is open');
  await w.ok('owner', 'configure', cfg({ buy: 32.6, sell: 32.6, previousUpdated: await w.updated('USD') }), 'sell equal to buy', 60000);
  await w.no('owner', 'configure', cfg({ buy: 32.2, sell: 32.7 }), 'the rate was changed since this page read it', 60000);
  await w.ok('RT1', 'configure', { code: 'EUR', enabled: true, buy: '37.5', sell: '38.25', previousUpdated: '', partnerName: 'Partner FX', partnerCost: 37.4 }, 'the rates role, figures as text, with a partner cost', 60000);
  await w.no('RT1', 'configure', { code: 'GBP', enabled: true, buy: 42.5, sell: 43.75, partnerName: 'Partner FX' }, 'a partner name without its cost: the rate rule refuses, nothing is stored', 60000);
  await w.ok('owner', 'configure', { code: 'KRW', enabled: true, buy: 0.0231, sell: 0.0259 }, 'a currency that is not one of the six defaults, switched on', 60000);
  await w.ok('owner', 'configure', { code: 'JPY', enabled: false, buy: 0.2105, sell: 0.2199 }, 'a default switched off: its rate is saved all the same', 60000);
  await w.ok('owner', 'configure', { code: 'SGD', enabled: false, buy: 24.1, sell: 24.9 }, 'a rate for a currency that stays off', 60000);
  await w.ok(null, 'rates', {}, 'dollar, euro and won; not the yen, not the Singapore dollar');
  await w.rpc('owner', 'adminSnapshot', {}, 'the desk sees the rates the kiosk page saved, each under the name of who saved it');
  await w.ok('owner', 'manage', {});
  await w.ok('RT1', 'manage', { branchId: 'B-nowhere' }, 'an employee manages their own branch');
  await w.ok('owner', 'configure', { code: 'JPY', enabled: true, buy: 0.2105, sell: 0.2199, previousUpdated: await w.updated('JPY') }, 'switched back on', 60000);
  await w.ok('owner', 'configure', { code: 'KRW', enabled: false, buy: 0.0231, sell: 0.0259, previousUpdated: await w.updated('KRW') }, undefined, 60000);
  await w.ok(null, 'rates', {});
  botDown = true;
  await w.no('owner', 'configure', { code: 'GBP', enabled: true, buy: 42.5, sell: 43.75 }, 'the Bank of Thailand does not answer: the rate is not saved and the switch is not stored', 60000);
  botDown = false;
  await w.ok('owner', 'manage', {}, 'the pound is as it was');
  // a second branch
  const airport = (await w.rpc('owner', 'adminCreateBranch', { requestId: w.rq(), name: 'Airport' })).branchId;
  await w.staff([['RT2', 'RATES']], airport);
  await w.ok(null, 'rates', { branchId: airport }, 'a new branch: nothing offered');
  await w.ok('owner', 'manage', { branchId: airport }, 'its switches are its own: the six defaults');
  await w.ok('RT2', 'configure', { code: 'USD', enabled: true, buy: 31.9, sell: 32.9, branchId: 'MAIN' }, 'the rates role of the airport names MAIN: the airport is configured', 60000);
  await w.ok('owner', 'configure', { code: 'AUD', enabled: true, buy: 20.5, sell: 21.5, branchId: airport }, undefined, 60000);
  await w.ok(null, 'rates', { branchId: airport });
  await w.ok(null, 'rates', {}, 'MAIN is as it was');
  await w.rpc('owner', 'adminSnapshot', { branchId: airport });
  await w.no('owner', 'configure', { code: 'USD', enabled: true, buy: 1, sell: 2, branchId: 'B-nowhere' });
  await w.no('owner', 'manage', { branchId: 'B-nowhere' });
  // the role is judged before any field
  await w.no('FR1', 'configure', { code: 'XXX', enabled: 'yes', buy: 0, sell: 0 }, 'not allowed, and nothing right in it: the role is said first');
  await w.no(null, 'configure', {});
  await w.no('MG1', 'configure', { code: 'USD', enabled: true });
  return w;
}

async function queue() {
  const w = await World.open('kiosk/queue.json', 'A day at the kiosk: quotes, the three-second and five-minute rules, the queue the desk sees, accepting, the passport check, the receipt picture — and who may ask for what.');
  await w.staff([['FR1', 'FRONT'], ['FR2', 'FRONT'], ['BE1', 'BACKEND'], ['MG1', 'MANAGER'], ['RT1', 'RATES'], ['MO1', 'MONITOR'], ['FI1', 'FINANCE'], ['SY1', 'SYSTEM'], ['DO1', 'DOCUMENTS'], ['RV1', 'DOCUMENT_REVIEW']]);
  await w.rate('USD', 32.1, 32.6);
  await w.rate('EUR', 37.5, 38.25);
  await w.rate('JPY', 0.2105, 0.2199);
  await w.rate('SGD', 24.1, 24.9);
  const buy = (o) => ({ type: 'BUY', currency: 'USD', amount: 100, ...o });
  await w.no(null, 'quote', buy(), 'no shift is open');
  await w.rpc('owner', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 100000, USD: 2000, EUR: 500 } });
  await w.ok(null, 'rates', {}, 'open, three of the four rates: the Singapore dollar is not a default');
  // what a quote must be
  await w.no(null, 'quote', {});
  await w.no(null, 'quote', buy({ type: 'buy' }));
  await w.no(null, 'quote', buy({ type: undefined }));
  await w.no(null, 'quote', buy({ currency: 'SGD' }), 'a rate exists but the kiosk does not offer it');
  await w.no(null, 'quote', buy({ currency: 'XXX' }));
  await w.no(null, 'quote', buy({ currency: undefined }));
  await w.no(null, 'quote', buy({ amount: 0 }));
  await w.no(null, 'quote', buy({ amount: -5 }));
  await w.no(null, 'quote', buy({ amount: 'abc' }));
  await w.no(null, 'quote', buy({ amount: undefined }));
  await w.no(null, 'quote', buy({ amount: 10000000.01 }));
  await w.ok(null, 'quote', buy({ amount: 10000000 }), 'ten million: the limit');
  await w.no(null, 'quote', { type: 'SELL', currency: 'USD', amount: 0.3 }, 'thirty satang buys less than a cent');
  await w.no(null, 'quote', buy({ paymentMethod: 'Credit Card' }), 'the customer selling currency is paid in cash or by transfer');
  await w.no(null, 'quote', buy({ paymentMethod: 'PromptPay' }));
  await w.no(null, 'quote', { type: 'SELL', currency: 'USD', amount: 1000, paymentMethod: 'Bitcoin' });
  await w.ok(null, 'quote', buy({ paymentMethod: 'Bank Transfer' }));
  for (const m of ['Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer']) await w.ok(null, 'quote', { type: 'SELL', currency: 'EUR', amount: 765, paymentMethod: m });
  await w.ok(null, 'quote', { type: 'SELL', currency: 'JPY', amount: '2199' }, 'an amount as text');
  await w.ok('FR1', 'quote', buy({ amount: 12.5, branchId: 'B-nowhere' }), 'an employee may ask as the kiosk does, for their own branch');
  // one request, start to end
  const q1 = await w.ok(null, 'quote', buy({ amount: 250 }), 'the request this file follows');
  await w.ok(null, 'status', { id: q1.id });
  await w.no(null, 'confirm', { id: q1.id }, 'two seconds after the quote', 1000);
  await w.no(null, 'confirm', { id: 'no-such-request' });
  await w.no(null, 'confirm', {});
  await w.no(null, 'status', { id: 'no-such-request' });
  await w.no(null, 'status', {});
  await w.no(null, 'customerReceipt', { id: 'no-such-request' });
  for (const who of ['FR1', 'owner']) { await w.no(who, 'accept', { id: q1.id }, 'not in the queue yet'); await w.no(who, 'get', { id: q1.id }); }
  await w.ok('FR1', 'list', {}, 'a quote nobody confirmed is not in the queue');
  await w.ok(null, 'confirm', { id: q1.id }, 'after three seconds: in the queue, one audit line');
  await w.ok(null, 'confirm', { id: q1.id }, 'again: the same answer, no second line');
  await w.ok(null, 'status', { id: q1.id });
  await w.ok(null, 'customerReceipt', { id: q1.id }, 'no picture yet');
  // who may ask
  const everyone = [null, 'owner', 'FR1', 'FR2', 'BE1', 'MG1', 'RT1', 'MO1', 'FI1', 'SY1', 'DO1', 'RV1'];
  const desk = ['owner', 'FR1', 'FR2', 'BE1', 'MG1'];
  for (const who of everyone) await w.k(who ? 'ok' : 'no', who, 'notifications', {});
  for (const who of everyone) await w.k(who ? 'ok' : 'no', who, 'list', {}); // the 10-06 bundle: anyone signed in (server.mjs:8026)
  for (const who of everyone.filter((x) => !desk.includes(x))) for (const action of ['accept', 'get', 'identityChecked', 'publishReceipt']) await w.no(who, action, { id: q1.id, confirmed: true, image: PNG_URL });
  // the desk
  await w.no('FR1', 'identityChecked', { id: q1.id, confirmed: true }, 'nobody accepted it');
  await w.no('FR1', 'publishReceipt', { id: q1.id, image: PNG_URL });
  await w.ok('FR1', 'accept', { id: q1.id });
  await w.no('FR2', 'accept', { id: q1.id }, 'somebody else has it');
  await w.ok('FR1', 'accept', { id: q1.id }, 'accepting twice: the holder\'s repeat (queue-work.mjs:11)');
  await w.ok('BE1', 'list', {}, 'the queue names who has it');
  await w.ok('BE1', 'notifications', {}, 'nothing waits now');
  await w.ok(null, 'status', { id: q1.id });
  await w.ok(null, 'confirm', { id: q1.id }, 'the customer presses again: nothing changes');
  await w.no('FR2', 'get', { id: q1.id });
  await w.no('owner', 'get', { id: q1.id }, 'not even the owner');
  await w.ok('FR1', 'get', { id: q1.id });
  await w.no('FR1', 'publishReceipt', { id: q1.id, image: PNG_URL }, 'the passport is not checked yet');
  await w.no('FR1', 'identityChecked', { id: q1.id });
  await w.no('FR1', 'identityChecked', { id: q1.id, confirmed: false });
  await w.no('FR1', 'identityChecked', { id: q1.id, confirmed: 'true' });
  await w.no('FR2', 'identityChecked', { id: q1.id, confirmed: true });
  await w.ok('FR1', 'identityChecked', { id: q1.id, confirmed: true });
  await w.ok('FR1', 'identityChecked', { id: q1.id, confirmed: true }, 'again: the same answer, no second line');
  await w.ok('FR1', 'get', { id: q1.id });
  await w.no('FR2', 'publishReceipt', { id: q1.id, image: PNG_URL });
  await w.no('FR1', 'publishReceipt', { id: q1.id });
  await w.no('FR1', 'publishReceipt', { id: q1.id, image: 'data:image/jpeg;base64,AAAA' });
  await w.no('FR1', 'publishReceipt', { id: q1.id, image: 'data:image/png;base64,' });
  await w.no('FR1', 'publishReceipt', { id: q1.id, image: 'data:image/png;base64,AA AA' });
  await w.no('FR1', 'publishReceipt', { id: q1.id, image: 7 });
  await w.ok('FR1', 'publishReceipt', { id: q1.id, image: PNG_URL });
  await w.ok(null, 'customerReceipt', { id: q1.id }, 'the customer sees the picture');
  await w.ok('FR1', 'publishReceipt', { id: q1.id, image: 'data:image/png;base64,QUJD' }, 'a second picture replaces the first');
  await w.ok(null, 'customerReceipt', { id: q1.id }, undefined, 599000);
  await w.ok(null, 'customerReceipt', { id: q1.id }, 'ten minutes after it was sent the picture is gone', 2000);
  await w.ok('MG1', 'list', {});
  // the owner at the desk, and the rules of time
  const q2 = await w.ok(null, 'quote', { type: 'SELL', currency: 'EUR', amount: 1530, paymentMethod: 'Alipay' });
  await w.ok(null, 'confirm', { id: q2.id }, undefined, 4000);
  const q3 = await w.ok(null, 'quote', buy({ amount: 40 }));
  await w.ok(null, 'confirm', { id: q3.id }, 'five minutes to the millisecond', 300000);
  await w.ok('RT1', 'notifications', {}, 'two wait; the latest is named');
  await w.ok('owner', 'accept', { id: q2.id });
  await w.ok('owner', 'get', { id: q2.id });
  await w.no('FR1', 'get', { id: q2.id });
  await w.ok('owner', 'identityChecked', { id: q2.id, confirmed: true });
  await w.ok('owner', 'publishReceipt', { id: q2.id, image: PNG_URL });
  await w.ok('FR2', 'accept', { id: q3.id });
  await w.ok('owner', 'list', {});
  const q4 = await w.ok(null, 'quote', buy({ amount: 5 }));
  await w.ok(null, 'status', { id: q4.id }, 'still a quote at five minutes', 300000);
  await w.ok(null, 'status', { id: q4.id }, 'expired one millisecond later', 1);
  await w.no(null, 'confirm', { id: q4.id });
  await w.ok(null, 'customerReceipt', { id: q4.id });
  const q5 = await w.ok(null, 'quote', buy({ amount: 6 }));
  await w.rate('USD', 32.2, 32.7);
  await w.no(null, 'confirm', { id: q5.id }, 'the rate moved after the quote');
  const q6 = await w.ok(null, 'quote', { type: 'SELL', currency: 'JPY', amount: 4398 });
  await w.ok(null, 'status', { id: q4.id }, 'an old quote is still there until somebody quotes ten minutes after it', 240000);
  const q7 = await w.ok(null, 'quote', buy({ amount: 7 }), 'this quote clears the quotes older than ten minutes', 240000);
  await w.no(null, 'status', { id: q4.id }, 'gone');
  await w.ok(null, 'status', { id: q6.id }, 'a younger one stays');
  // another branch
  const airport = (await w.rpc('owner', 'adminCreateBranch', { requestId: w.rq(), name: 'Airport' })).branchId;
  await w.staff([['FR9', 'FRONT']], airport);
  await w.no(null, 'status', { id: q1.id, branchId: airport }, 'a request belongs to its branch');
  await w.no(null, 'confirm', { id: q7.id, branchId: airport }, undefined, 3000);
  await w.ok('FR9', 'list', {}, 'the airport has no queue');
  await w.ok('FR9', 'notifications', { branchId: 'MAIN' });
  await w.no('FR9', 'accept', { id: q3.id });
  await w.ok('owner', 'list', { branchId: airport });
  await w.ok('owner', 'notifications', { branchId: airport });
  await w.no(null, 'quote', buy({ branchId: airport }), 'the airport is closed');
  // the counter closes
  const m = await w.model();
  const shift = m.stock.shifts.find((s) => s.status === 'OPEN');
  await w.rpc('owner', 'adminCloseShift', { requestId: w.rq(), shiftId: shift.id, expected: shift.opening, counted: shift.opening, note: '' });
  await w.no(null, 'confirm', { id: q7.id }, 'the counter closed after the quote');
  await w.no(null, 'quote', buy());
  await w.ok(null, 'rates', {}, 'closed, the rates still shown');
  await w.ok('FR1', 'list', {}, 'the queue is still there');
  return w;
}

async function queueWork() {
  const w = await World.open('kiosk/queue_work.json', 'The 10-06 bundle\'s queue work (queue-work.mjs): list with filter and page for anyone signed in, the claim with its start time, the holder\'s repeat claim, the two Thai refusals of a claim, notifications with who is working since when, fifty to a page, and a second branch.');
  await w.staff([['FR1', 'FRONT'], ['FR2', 'FRONT'], ['BE1', 'BACKEND'], ['MG1', 'MANAGER'], ['RT1', 'RATES'], ['MO1', 'MONITOR'], ['FI1', 'FINANCE']]);
  await w.rate('USD', 32.1, 32.6);
  await w.rpc('owner', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 100000, USD: 2000 } });
  const everyone = [null, 'owner', 'FR1', 'FR2', 'BE1', 'MG1', 'RT1', 'MO1', 'FI1'];
  // an empty queue, to everyone signed in
  for (const who of everyone) await w.k(who ? 'ok' : 'no', who, 'list', {});
  for (const who of everyone) await w.k(who ? 'ok' : 'no', who, 'notifications', {});
  // the page and the filter
  for (const page of [-1, 1.5, 100001, 'x', '2px', {}, -0.5]) await w.no('FR1', 'list', { page }, 'not a page');
  for (const page of [0, null, '', false, 2, '2', ' 3 ', '1e3', true, 100000]) await w.ok('FR1', 'list', { page }, 'a page');
  for (const filter of ['Waiting', 'x', 1, true, 'WAITING']) await w.no('FR1', 'list', { filter }, 'not a filter');
  for (const filter of ['waiting', 'working', 'completed', 'all', null, '', false, 0]) await w.ok('FR1', 'list', { filter }, 'a filter');
  await w.no('FR1', 'list', { page: -1, filter: 'x' }, 'the page is judged before the filter');
  // three requests wait
  const buy = (amount) => ({ type: 'BUY', currency: 'USD', amount });
  const waiting = async (amount, note) => { const q = await w.ok(null, 'quote', buy(amount), note, 4000); await w.ok(null, 'confirm', { id: q.id }, undefined, 4000); return q; };
  const q1 = await waiting(101, 'the first request');
  const q2 = await waiting(102);
  const q3 = await waiting(103);
  const q0 = await w.ok(null, 'quote', buy(99), 'a quote nobody confirmed', 4000);
  await w.ok('FR1', 'notifications', {}, 'three wait, nobody works');
  await w.ok('RT1', 'list', {}, 'three waiting; no button for a role that cannot claim');
  // the claim
  await w.no('FR1', 'accept', { id: q0.id }, 'not confirmed: not ready');
  await w.no('FR1', 'accept', { id: 'no-such-request' }, 'no such request: not ready');
  await w.no('FR1', 'accept', {}, 'no id: not ready');
  await w.no('RT1', 'accept', { id: q1.id }, 'a role that cannot claim');
  await w.no(null, 'accept', { id: q1.id });
  await w.ok('FR1', 'accept', { id: q1.id }, 'the claim: its start time');
  await w.ok('FR1', 'accept', { id: q1.id }, 'the holder again: the same start, no second line', 5000);
  await w.no('FR2', 'accept', { id: q1.id }, 'somebody else holds it: named');
  await w.no('owner', 'accept', { id: q1.id }, 'not even the owner');
  await w.ok('FR2', 'accept', { id: q2.id }, undefined, 2000);
  await w.no('BE1', 'accept', { id: q2.id }, 'FR2 holds it');
  await w.ok('owner', 'accept', { id: q3.id }, undefined, 2000);
  await w.no('FR1', 'accept', { id: q3.id }, 'the owner holds it');
  for (const who of ['owner', 'FR1', 'FR2', 'RT1']) await w.ok(who, 'list', {}, 'who works on what since when; the holder alone may continue');
  for (const filter of ['waiting', 'working', 'completed']) await w.ok('FR1', 'list', { filter });
  await w.ok('MG1', 'notifications', {}, 'nobody waits; three work, in the order they started');
  await w.ok('FR1', 'identityChecked', { id: q1.id, confirmed: true });
  await w.ok('FR1', 'accept', { id: q1.id }, 'the holder after the passport check: the same start');
  await w.no('FR2', 'accept', { id: q1.id }, 'still held');
  await w.ok('FR2', 'list', { filter: 'working' });
  // more than fifty
  const many = [];
  for (let i = 0; i < 52; i++) many.push(await waiting(110 + i, i === 0 ? 'fifty-two more wait' : undefined));
  await w.ok('FR1', 'notifications', {}, 'fifty-two wait');
  await w.ok('FR1', 'list', {}, 'the first page: fifty, and there is more');
  await w.ok('FR1', 'list', { page: 1 }, 'the second page: the rest');
  await w.ok('FR1', 'list', { page: 2 }, 'nothing on the third');
  await w.ok('FR1', 'list', { filter: 'waiting', page: 1 }, 'two waiting on the second page of the waiting');
  // more than ten at work
  for (let i = 0; i < 9; i++) await w.ok(['BE1', 'MG1', 'FR2'][i % 3], 'accept', { id: many[i].id }, i === 0 ? 'nine more claimed: twelve at work' : undefined, 1000);
  await w.ok('FR1', 'notifications', {}, 'ten of the twelve at work are listed, all twelve counted');
  await w.ok('owner', 'list', { filter: 'working' });
  // another branch
  const airport = (await w.rpc('owner', 'adminCreateBranch', { requestId: w.rq(), name: 'Airport' })).branchId;
  await w.staff([['FR9', 'FRONT']], airport);
  await w.ok('FR9', 'list', {}, 'the airport has no queue');
  await w.ok('FR9', 'list', { branchId: 'MAIN' }, 'an employee is answered for their own branch');
  await w.ok('FR9', 'notifications', {});
  await w.no('FR9', 'accept', { id: many[20].id }, 'a request of another branch: not ready');
  await w.ok('owner', 'list', { branchId: airport });
  await w.ok('owner', 'list', { branchId: 'MAIN', filter: 'working' });
  return w;
}

// ════ run ═══════════════════════════════════════════════════════════════════
async function main() {
  await load();
  const worlds = [];
  try {
    for (const scenario of [transport, currencies, queue, queueWork]) worlds.push(await scenario());
  } finally {
    await sql('DROP SCHEMA IF EXISTS ' + SCHEMA + ' CASCADE').catch(() => {});
    await v2.pool.end();
    fs.rmSync(work, { recursive: true, force: true });
  }
  if (DISCOVER) { for (const s of [...seen.keys()].sort()) console.log('  ' + JSON.stringify(s) + ','); return; }
  for (const s of SENTENCES) if (!seen.has(s)) throw Error('a sentence this script expects was never said: ' + s);
  fs.mkdirSync(path.join(outDir, 'kiosk'), { recursive: true });
  for (const w of worlds) { const text = fileText(w); fs.writeFileSync(path.join(outDir, w.file), text); console.error(w.file.padEnd(28) + String(w.steps.length).padStart(5) + ' steps ' + String(Buffer.byteLength(text)).padStart(9) + ' bytes'); }
  const behind = '{\n "source": "v2 10-06 bundle (server.mjs sha256 99ddcd10…, with queue-work.mjs) on PostgreSQL; api/tools/v2-fixtures/gen_kiosk_http.mjs",\n "about": "Per file and step number: the audit lines v2 wrote while it answered that step, and the Bank of Thailand reference kept with a rate that an accepted configure saved. A step that is not listed wrote no line.",\n'
    + worlds.map((w) => ' ' + JSON.stringify(w.file.replace('kiosk/', '')) + ': {\n' + Object.entries(w.behind).map(([n, v]) => '  ' + JSON.stringify(n) + ': ' + JSON.stringify(v)).join(',\n') + '\n }').join(',\n') + '\n}\n';
  fs.writeFileSync(path.join(outDir, 'kiosk/behind.json'), behind);
  const first = worlds.find((w) => w.file === 'kiosk/currencies.json').steps[0].body.currencies;
  const table = '{\n "source": "v2 10-06 bundle (server.mjs sha256 99ddcd10…), the first manage answer of kiosk/currencies.json; api/tools/v2-fixtures/gen_kiosk_http.mjs",\n "currencies": [\n' + first.map(({ code, name, nameEn, countries }) => '  ' + JSON.stringify({ code, name, nameEn, countries })).join(',\n') + '\n ]\n}\n';
  fs.writeFileSync(path.join(outDir, 'kiosk/currency_table.json'), table);
  console.error('sentences: ' + [...seen].map(([s, n]) => n + '× ' + s).join(' | '));
}
await main();
