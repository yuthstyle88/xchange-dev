// Records what v2 answers on `/api/rpc` and `/api/staff/rpc` — the contract the kept admin,
// desk and staff screens are written against — for tickets T11a (dispatch), T11b (snapshot)
// and T11c (desk config).
//   node gen_compat.mjs <v2 bundle dir> <postgres url of a scratch database> [out dir]
//   out dir defaults to ../../tests/fixtures; it gets snapshot/*.json and rpc/*.json
//
// Unlike its neighbours this script runs the WHOLE deployed bundle (`server.mjs`,
// `xchange-online@3ffc5a5`, with `schema.sql` beside it) against a real PostgreSQL, so every
// answer below went through v2's own `handle(request)`: the route table, the session
// look-up, the staff bridge, the save hooks, the ledger (`execute`), v2's own SQL through
// its own driver, and the engine. The bundle is not in this repository and nothing of it is
// copied here; this file holds slice markers, payloads and the sentences v2 is expected to say.
//
// The database: the script creates the schema `xc_compat_fixture` in the database named,
// applies v2's `schema.sql` to it, drops it again for every scenario and at the end. Nothing
// outside that schema is read or written.
//
// What is replaced, and nothing else:
//   - the last statement (`http.createServer(…).listen`) is cut off; requests are handed to
//     `handle` directly as `Request` objects, so no port is opened;
//   - `Date` is a settable clock and `randomUUID` / `randomBytes` count, so the files are
//     the same on every run (v2 reads the wall clock and draws random ids);
//   - `fetch` answers the two Bank of Thailand addresses v2 calls when a rate is saved,
//     with made-up figures in the real shape; any other address fails the script;
//   - v2's `console` is silent.
// What is written to the database beside v2's own statements, because v2 has no way to do it
// that this contract needs: the `owners` row and the session rows (a sign-in would only add
// a password hash to the story), `must_change` cleared on the employees v2 itself created,
// and — in the one scenario that says so — a `members` row (the bookings API is decision D5).
//
// The script fails, and writes nothing, when v2 answers with a sentence a step did not
// expect, when a step expected a refusal and got none, or when one of the claims the
// contract leans on (listed in `claims` below) does not hold on the recorded answers.
// Three claims are checked on every answer that can show them: a write answers with the
// snapshot of the same instant; adminDeskConfig is five values of adminSnapshot plus the same
// `operations`; the staff path answers with the /api/rpc snapshot rewritten (`bridged` below).
//
// When a scenario is changed: XC_COMPAT_DISCOVER=1 prints the sentences heard instead of
// writing files (paste them into REFUSALS), XC_COMPAT_TRACE=1 prints every refusal as it comes.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { pathToFileURL } from 'node:url';

const [bundleArg, databaseUrl, outArg] = process.argv.slice(2);
if (!bundleArg || !databaseUrl) { console.error('usage: node gen_compat.mjs <v2 bundle dir> <postgres url> [out dir]'); process.exit(2); }
const bundleDir = fs.statSync(bundleArg).isDirectory() ? bundleArg : path.dirname(bundleArg);
const outDir = path.resolve(outArg || new URL('../../tests/fixtures', import.meta.url).pathname);
const SCHEMA = 'xc_compat_fixture';

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
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'xc-compat-'));
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
  src = 'const Date = globalThis.__XC_COMPAT.Date, fetch = globalThis.__XC_COMPAT.fetch, console = { log() {}, error() {}, warn() {} };\n' + head + body + '\nexport { handle, pool };\n';
  fs.writeFileSync(path.join(work, 'server.mjs'), src);
  const url = new URL(databaseUrl);
  url.searchParams.set('options', '-c search_path=' + SCHEMA);
  Object.assign(process.env, { DATABASE_URL: url.toString(), UPLOAD_DIR: uploads, BOOKING_BRIDGE_SECRET: 'fixture-bridge-secret-0123456789abcdef', AUTH_ENABLED: '1', DEVICES_ENABLED: '1', CUSTOMER_HOST: 'customer.test', PORT: '0' });
  for (const k of ['OWNER_ID', 'OWNER_EMAIL', 'DESK_HOST', 'ADMIN_ORIGIN']) delete process.env[k];
  globalThis.__XC_COMPAT = { Date: FakeDate, fetch: fakeFetch, crypto: fakeCrypto };
  v2 = await import(pathToFileURL(path.join(work, 'server.mjs')).href);
  sql = (text, args = []) => v2.pool.query(text, args);
}
async function freshDatabase() {
  await sql('DROP SCHEMA IF EXISTS ' + SCHEMA + ' CASCADE');
  await sql('CREATE SCHEMA ' + SCHEMA);
  await sql(fs.readFileSync(path.join(bundleDir, 'schema.sql'), 'utf8'));
  fs.rmSync(uploads, { recursive: true, force: true });
  fs.mkdirSync(uploads, { recursive: true });
  clock = T0; idCounter = 0; byteCounter = 0;
}


// ── who calls ───────────────────────────────────────────────────────────────
const ORIGIN = 'http://desk.test';
const FOREVER = 4102444800000; // the sessions written below never run out while a scenario plays
const DEFAULT_HEADERS = { 'cache-control': 'no-store', 'content-type': 'application/json' };
const RQ = (n) => idAt(0xa0000 + n); // request ids the screen would make
const PNG = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82]);
const PDF = Buffer.from('%PDF-1.4\n%fixture\n');
const SEVEN = ['THB', 'USD', 'EUR', 'GBP', 'JPY', 'CNY', 'AUD'];
const cents = (n) => Math.round((Number(n ?? 0) + Number.EPSILON) * 100);
/** Count rows that add up to the totals: hundreds, then the remainder as one piece. */
const rowsFor = (totals) => Object.entries(totals).filter(([c]) => SEVEN.includes(c)).flatMap(([currency, v]) => {
  const c = cents(v), hundreds = Math.floor(c / 10000), rest = c % 10000, out = [];
  if (hundreds) out.push({ currency, denomination: 100, count: hundreds });
  if (rest) out.push({ currency, denomination: rest / 100, count: 1 });
  return out;
});
const count = (totals) => ({ counted: totals, denominations: rowsFor(totals) });
const safeCount = (totals) => ({ safeCount: totals, safeDenominations: rowsFor(totals) });
const nonZero = (o) => Object.fromEntries(Object.entries(o).filter(([, v]) => v !== 0));
const ROLES = ['FRONT', 'BACKEND', 'MANAGER', 'RATES', 'MONITOR', 'FINANCE', 'SYSTEM', 'DOCUMENTS', 'DOCUMENT_REVIEW']; // server.mjs:7290

/** What a shift is expected to hold, worked out the way the admin screen has to before it closes one. v2 refuses the close when this is wrong (server.mjs:6694). */
function expectedOf(model) {
  const shift = model.stock.shifts.find((s) => s.status === 'OPEN');
  const bal = Object.fromEntries(Object.entries(shift.opening).map(([c, v]) => [c, cents(v)]));
  for (const t of model.transactions.filter((x) => x.shiftId === shift.id && x.status === 'COMPLETED')) {
    const principal = cents(t.amount * t.rate), fee = cents(t.fee);
    bal[t.currency] += (t.type === 'BUY' ? 1 : -1) * cents(t.amount);
    if (t.method === 'Cash') bal.THB += t.type === 'SELL' ? principal + fee : -(principal - fee);
  }
  for (const m of model.stock.movements.filter((x) => x.shiftId === shift.id)) bal[m.currency] += (m.direction === 'IN' ? 1 : -1) * cents(m.amount);
  return Object.fromEntries(Object.entries(bal).map(([c, v]) => [c, v / 100]));
}

// ── sentences a refusing step may be answered with; anything else stops the script ──
const REFUSALS = new Set((process.env.XC_COMPAT_DISCOVER ? [] : [
  "Backend ใช้ปุ่มตรวจและบันทึกงาน",
  "Cannot destructure property 'action' of 'JSON.parse(...)' as it is null.",
  "Cannot read properties of null (reading 'payload')",
  "EMPLOYEE_AUTH_REQUIRED",
  "Expected ',' or '}' after property value in JSON at position 31 (line 1 column 32)",
  "Not found",
  "Unexpected end of JSON input",
  "Unexpected token 'o', \"not json\" is not valid JSON",
  "กรอกชื่อคู่ค้าและเรทต้นทุนให้ครบคู่",
  "กรุณากรอกจำนวนเงิน",
  "กรุณาระบุเหตุผลยอดต่างก่อนปิดกะ",
  "กรุณาเข้าสู่ระบบ",
  "กรุณาเปลี่ยนรหัสผ่านก่อนเริ่มงาน",
  "กรุณาเปิดกะจากหน้าแอดมินก่อน",
  "กรุณาเลือกประเภทรายการ",
  "กะเปลี่ยนหรือปิดแล้ว กรุณารีเฟรช",
  "กำหนดกิจการก่อนเปิดใช้และก่อนเปิดกะ",
  "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด",
  "ข้อมูลคำขอใหญ่เกินกำหนด",
  "ข้อมูลมากเกินไป",
  "ข้อมูลรายการไม่ครบ",
  "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด",
  "คำขอเดิมมีข้อมูลเปลี่ยน",
  "คำขอไม่ถูกต้อง",
  "ค่าบริการสูงกว่ายอดซื้อ",
  "งานยังไม่เสร็จ",
  "จำนวนเงินต้องไม่ติดลบ ไม่เกิน 100,000,000 และมีทศนิยมไม่เกิน 2 ตำแหน่ง",
  "จำนวนเงินหรือเรทไม่ถูกต้อง",
  "ชื่อสาขานี้มีอยู่แล้ว",
  "ต้องระบุเลขอ้างอิงการโอน",
  "ต้องเป็น Front ผู้ถือครองเงินในกะที่เปิด",
  "ต้องเป็น Front ผู้รับผิดชอบลิ้นชักที่ลงเวลาเข้าแล้ว",
  "ต้องแนบหลักฐานการนับหรือรับ–ส่งเงิน",
  "ทศนิยมเกินกำหนด",
  "บทบาทนี้อัปโหลดหลักฐานไม่ได้",
  "บทบาทนี้ไม่มีสิทธิ์ทำรายการ",
  "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้",
  "บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า",
  "ประเภทงานไม่ถูกต้อง",
  "ผู้ดูแลเรทต้องตั้งเรทจริงก่อนเปิดกะ",
  "ผู้รับหรือบทบาทไม่ถูกต้องสำหรับสาขา",
  "มีกะเปิดอยู่แล้ว กรุณารีเฟรช",
  "มีรายการรับ–ส่งที่ยังไม่เสร็จ กรุณาตรวจรายการเดิมก่อน",
  "ยอด EUR ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน",
  "ยอด THB ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน",
  "ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน",
  "ยอดเปลี่ยนระหว่างตรวจนับ กรุณารีเฟรชและตรวจสอบอีกครั้ง",
  "ยังไม่มีกะเปิด",
  "ยังไม่ได้ลงเวลาเข้า",
  "ยืนยันการนับเงินก่อนบันทึกรายการเงินสด",
  "ยืนยันกิจการและการแยกบัญชีให้ครบ",
  "ยืนยันสาขาและกิจการก่อนเปิดใช้",
  "ยืนยันอุปกรณ์และเรทที่แสดงให้ตรงกับชุดเรทล่าสุด",
  "รหัสคำขอไม่ถูกต้อง",
  "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่",
  "รอ Backend บันทึกส่งเงินก่อนยืนยันรับ",
  "รอ Finance ยืนยันการรับ–ส่ง",
  "รอ Monitor ตรวจและลงชื่อ",
  "รองรับสูงสุด 20 สาขา",
  "ระบุผลตรวจรายการที่พัก",
  "ระบุผลตรวจและหลักฐานอ้างอิง",
  "ระบุเหตุผลส่งกลับ",
  "รายการนี้สิ้นสุดแล้ว",
  "รายการรับ–จ่ายไม่ถูกต้อง",
  "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่",
  "รายการไม่ถูกต้อง",
  "รูปแบบวันที่ไม่ถูกต้อง",
  "ลงเวลาเข้าก่อนทำรายการ",
  "ลูกค้านี้อยู่ระหว่างพักตรวจ ให้ Monitor ยืนยันผลก่อนทำรายการ",
  "วันที่ไม่ถูกต้อง",
  "สร้างบัญชีให้ครบ Front, Backend, Manager, Rates, Monitor และ Finance ในสาขาก่อน",
  "สาขาไม่ถูกต้อง",
  "ส่งมอบเงินและงานที่ค้างให้เสร็จก่อนลงเวลาออก",
  "หลักฐานไม่ใช่ของผู้ส่งหรือสาขานี้",
  "หัวตารางเปลี่ยน กรุณาคืนลำดับคอลัมน์ก่อนบันทึก: Transactions",
  "อยู่ระหว่างตรวจนับ",
  "เงินอยู่ระหว่างนำส่ง ต้องยืนยันการรับหรือจัดการคืนเงินตามหลักฐานก่อน",
  "เจ้าของต้องเปิดใช้ขั้นตอนรับ–ส่งเงินก่อน",
  "เจ้าของเป็นผู้เปิดใช้ขั้นตอน",
  "เฉพาะเจ้าของระบบเท่านั้น",
  "เปิดบริการได้หลังเตรียมกะเสร็จ ระหว่าง 11:00–22:50",
  "เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน",
  "เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ",
  "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ",
  "เลือกลูกค้าและระบุเหตุพักตรวจ",
  "แนบหลักฐาน 1–5 ไฟล์",
  "แนบเอกสารไม่ครบ",
  "ใช้ขั้นตอนรับ–ส่งเงินและการลงชื่อในเมนูปฏิบัติงาน",
  "ใช้ได้เฉพาะ Front",
  "ไฟล์แนบไม่ถูกต้อง",
  "ไม่พบรายการรับ–ส่ง",
  "ไม่พบสาขาที่เลือก กรุณารีเฟรช",
  "ไม่พบเอกสารรายการ",
  "ไม่พบไฟล์แนบของสาขานี้",
  "ไม่มีสิทธิ์ลงชื่อ",
  "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท",
]));
const seenRefusals = new Map();

// ── one shop, one database ──────────────────────────────────────────────────
class World {
  static async open(file, about) {
    await freshDatabase();
    const w = new World();
    Object.assign(w, { file, about, steps: [], people: [], tokens: new Map(), requestNo: 0, submissionNo: 0 });
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
  /** An employee, created by v2's own `/api/employees`; then signed in by a session row. */
  async employee(key, role, branchId = 'MAIN', opts = {}) {
    const username = key.toLowerCase(), name = opts.name || key + ' ' + role[0] + role.slice(1).toLowerCase();
    const res = await this.send('owner', '/api/employees', JSON.stringify({ action: 'create', payload: { username, name, role, branchId, password: 'fixture-password-' + key } }));
    if (res.status !== 200) throw Error('employee ' + key + ': ' + res.text);
    const row = (await sql('SELECT id FROM employees WHERE username=$1', [username])).rows[0];
    if (!opts.mustChange) await sql('UPDATE employees SET must_change=0 WHERE id=$1', [row.id]);
    await this.session(key, 'employee_sessions', row.id);
    this.people.push({ key, kind: 'staff', id: row.id, username, name, role, branchId, ...(opts.mustChange ? { mustChange: true } : {}) });
    return row.id;
  }
  async staff(list, branchId = 'MAIN') { for (const [key, role] of list) await this.employee(key, role, branchId); }
  id(key) { return this.people.find((p) => p.key === key).id; }
  rq() { return RQ(++this.requestNo); }
  /** One request through v2's `handle`. `headers: {name: null}` leaves a default header out. */
  async send(as, route, body, { method = 'POST', headers = {} } = {}) {
    const h = { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json', ...(as && this.tokens.get(as) ? { cookie: this.tokens.get(as) } : {}), ...headers };
    for (const k of Object.keys(h)) if (h[k] === null) delete h[k];
    const init = { method, headers: h };
    if (method !== 'GET' && method !== 'HEAD' && body !== undefined) init.body = body;
    const res = await v2.handle(new Request(ORIGIN + route, init));
    const text = await res.text(), got = Object.fromEntries([...res.headers].filter(([k]) => k !== 'set-cookie'));
    return { status: res.status, headers: got, text };
  }
  record(step, res, expect) {
    step = { n: this.steps.length + 1, at: new FakeDate().toISOString(), ...step, status: res.status };
    if (JSON.stringify(res.headers) !== JSON.stringify(DEFAULT_HEADERS)) step.headers = res.headers;
    let json;
    try { json = JSON.parse(res.text); } catch { json = undefined; }
    if (json !== undefined && JSON.stringify(json) === res.text) step.body = json; else step.text = res.text;
    const refused = res.status >= 400;
    const where = `${this.file} step ${step.n} (${step.as} ${step.route} ${step.action ?? step.raw ?? ''})`;
    if (expect === 'ok' && refused) throw Error(`${where}: expected v2 to accept, got ${res.status} ${res.text.slice(0, 300)}`);
    if (expect === 'no' || (expect === undefined && refused)) {
      if (!refused) throw Error(`${where}: expected v2 to refuse, got ${res.status} ${res.text.slice(0, 200)}`);
      const sentence = json && typeof json.error === 'string' ? json.error : res.text;
      if (!process.env.XC_COMPAT_DISCOVER && !REFUSALS.has(sentence)) throw Error(`${where}: unexpected sentence ${JSON.stringify(sentence)} (${res.status})`);
      seenRefusals.set(sentence, (seenRefusals.get(sentence) || 0) + 1);
      if (process.env.XC_COMPAT_TRACE) console.error(`${this.file}#${step.n}\t${step.as}\t${step.route.replace('/api/', '')}\t${step.action ?? (step.raw ?? '').slice(0, 50)}\t${res.status}\t${sentence}\t${step.note ?? ''}`);
    }
    this.steps.push(step);
    return json;
  }
  /** An RPC call. `route` is 'rpc' (/api/rpc) or 'staff' (/api/staff/rpc). */
  async rpc(expect, as, route, action, payload = {}, note, still = false) {
    if (!still) clock += 60000;
    const path2 = route === 'rpc' ? '/api/rpc' : route === 'staff' ? '/api/staff/rpc' : route;
    const res = await this.send(as, path2, JSON.stringify({ action, payload }));
    return this.record({ as: as ?? 'nobody', method: 'POST', route: path2, action, payload, ...(note ? { note } : {}) }, res, expect);
  }
  ok(as, route, action, payload, note) { return this.rpc('ok', as, route, action, payload, note); }
  no(as, route, action, payload, note) { return this.rpc('no', as, route, action, payload, note); }
  /** A read at the clock of the step before it, so that it shows the state that step left. */
  read(as, route, action, payload = {}, note) { return this.rpc('ok', as, route, action, payload, note, true); }
  /** A request written out by hand: the transport cases. */
  async raw(expect, as, route, body, opts = {}, note) {
    const res = await this.send(as, route, body, opts);
    const step = { as: as ?? 'nobody', method: opts.method || 'POST', route, ...(body === undefined ? {} : body.length > 400 ? { rawLength: body.length, rawStart: body.slice(0, 60) } : { raw: body }), ...(opts.headers ? { requestHeaders: opts.headers } : {}), ...(note ? { note } : {}) };
    return this.record(step, res, expect);
  }
  /** A file sent the way transport.js sends one. */
  async upload(as, route, branch, kind = 'png', expect = 'ok', note) {
    clock += 60000;
    const path2 = (route === 'rpc' ? '/api/upload' : '/api/staff/upload') + (branch ? '?branch=' + branch : '');
    const res = await this.send(as, path2, kind === 'pdf' ? PDF : PNG, { headers: { 'content-type': kind === 'pdf' ? 'application/pdf' : 'image/png' } });
    const json = this.record({ as, method: 'POST', route: path2, upload: kind, ...(note ? { note } : {}) }, res, expect);
    return json?.uploadKey;
  }
  /** adminSnapshot and adminDeskConfig as each of `viewers` sees them, on both paths. */
  async views(label, viewers, routes = ['rpc', 'staff']) {
    for (const who of viewers) for (const route of who === 'owner' ? ['rpc'] : routes) for (const action of ['adminSnapshot', 'adminDeskConfig']) {
      const res = await this.send(who, route === 'rpc' ? '/api/rpc' : '/api/staff/rpc', JSON.stringify({ action, payload: {} }));
      this.record({ as: who, method: 'POST', route: route === 'rpc' ? '/api/rpc' : '/api/staff/rpc', action, payload: {}, view: label }, res);
    }
  }
  model(as = 'owner', payload = {}) { return this.send(as, '/api/rpc', JSON.stringify({ action: 'adminSnapshot', payload })).then((r) => JSON.parse(r.text)); }
  // ── things most scenarios need ────────────────────────────────────────────
  async rate(as, route, code, buy, sell, extra = {}, branchId) {
    const m = await this.model('owner', branchId ? { branchId } : {});
    return this.ok(as, route, 'adminSaveRate', { requestId: this.rq(), code, buy, sell, previousUpdated: m.rates.find((r) => r.code === code)?.updated || '', ...extra, ...(branchId ? { branchId } : {}) });
  }
  customerPayload(n, mode = 'LIVE') { return { name: 'Customer ' + n, passportNo: 'P' + String(1000000 + n), nationality: ['JP', 'DE', 'US', 'CN'][n % 4], birthDate: '1990-01-0' + (1 + n % 9), passportExpiry: '2031-06-1' + (n % 9), mode }; }
  /** The payload the desk screen sends for a save (the screen's own field names; `staff` is overwritten by the server). */
  txPayload(n, customerId, tx, files, mode = 'LIVE') {
    return { ...this.customerPayload(n, mode), submissionId: idAt(0xb0000 + ++this.submissionNo), customerId, type: tx.type, currency: tx.currency, amount: tx.amount, rate: tx.rate, fee: tx.fee ?? 0, bankFee: tx.bankFee ?? 0, method: tx.method ?? 'Cash', ref: tx.ref ?? '', account: tx.account ?? 'Cash Drawer', staff: 'desk', booking: tx.booking ?? 'WALK-IN', notes: tx.notes ?? '', valueDate: '2026-10-06', paidAt: new RealDate(clock).toISOString(), paymentConfirmed: true, identityConfirmed: true, countConfirmed: true, passport: { uploadKey: files.passport }, ...(files.slip ? { slip: { uploadKey: files.slip } } : {}), ...(tx.referenceRate !== undefined ? { referenceRate: tx.referenceRate } : {}), ...(tx.extra ?? {}) };
  }
}

// ── claims the contract document leans on, checked on every recorded answer ──
const claims = { writeAnswersAreTheSnapshot: 0, deskConfigIsPartOfTheSnapshot: 0, staffPathIsTheSnapshotRewritten: 0 };
const MODEL_ACTIONS = new Set(['adminOpenShift', 'adminCloseShift', 'adminMoveStock', 'adminSaveRate', 'adminCreateBranch', 'adminAbandonRequest']);
const same = (a, b, what) => { if (JSON.stringify(a) !== JSON.stringify(b)) throw Error('claim does not hold: ' + what + '\n' + JSON.stringify(a).slice(0, 400) + '\n' + JSON.stringify(b).slice(0, 400)); };
/** Claim 1: a write answers with the snapshot of the same instant, without `operations` and without the two member fields. */
async function checkWrite(w, as, action, payload, json) {
  const snap = await w.model(as, action === 'adminCreateBranch' ? { branchId: json.branchId } : { branchId: payload.branchId });
  const { operations, ...rest } = snap;
  rest.customers = rest.customers.map(({ member, memberVerifiedAt, ...c }) => c);
  const { requestResolution, ...answer } = json;
  if ((action === 'adminAbandonRequest') !== (requestResolution !== undefined)) throw Error('requestResolution on ' + action);
  same(answer, rest, action + ' answers with the snapshot');
  claims.writeAnswersAreTheSnapshot++;
}
/** Claim 2: the desk config is five values taken from the snapshot, plus the same `operations`. */
function checkDesk(snap, desk) {
  same(desk, { branchId: snap.branchId, branchName: snap.branches.find((b) => b.id === snap.branchId).name, rates: snap.rates, activeShift: snap.stock.shifts.find((s) => s.status === 'OPEN')?.id || null, adminUrl: '/?branch=' + encodeURIComponent(snap.branchId), operations: snap.operations }, 'adminDeskConfig is part of adminSnapshot');
  claims.deskConfigIsPartOfTheSnapshot++;
}
/** Claim 3: what the staff bridge does to the answer of `/api/rpc` (server.mjs:7444-7459). */
function bridged(snap, person, colleagues) {
  const out = structuredClone(snap);
  out.employees = colleagues;
  out.employee = { id: person.id, username: person.username, name: person.name, role: person.role, branchId: person.branchId, active: true, mustChange: false };
  if (['RATES', 'SYSTEM'].includes(person.role)) {
    out.transactions = []; out.customers = [];
    out.operations = { ...out.operations, attendance: out.operations.attendance.filter((a) => a.employeeId === person.id), requests: [], events: [], exceptions: [], holds: [], cutoff: null };
  }
  if (person.role === 'SYSTEM') {
    out.stock = { shifts: [], movements: [] }; out.rates = []; out.rateHistory = [];
    out.operations = { enabled: false, phase: 'CLOSED', entity: out.operations.entity };
  }
  return rewrite(out);
}
const rewrite = (x) => typeof x === 'string' && x.startsWith('/files/') ? x.replace('/files/', '/api/staff/file/') : typeof x === 'string' && x.startsWith('/documents?') ? x.replace('/documents?', '/staff/documents?') : Array.isArray(x) ? x.map(rewrite) : x && typeof x === 'object' ? Object.fromEntries(Object.entries(x).map(([k, v]) => [k, ['adminUrl', 'sheetUrl'].includes(k) ? '/staff' : k === 'deskUrl' ? '/staff/desk' : rewrite(v)])) : x;
function checkViews(w) {
  const last = new Map();
  for (const s of w.steps) {
    if (!s.view || s.status !== 200) continue;
    const key = s.view + '|' + s.as;
    if (s.action === 'adminSnapshot' && s.route === '/api/rpc') last.set(key, s.body);
    if (s.action === 'adminDeskConfig' && s.route === '/api/rpc' && last.has(key)) checkDesk(last.get(key), s.body);
    if (s.action === 'adminSnapshot' && s.route === '/api/staff/rpc' && last.has(key)) {
      const person = w.people.find((p) => p.key === s.as);
      // the bridge lists the branch's active employees in the order the database returns them, which is the order they were created in
      const colleagues = w.people.filter((p) => p.kind === 'staff' && p.branchId === person.branchId && !p.disabled).map((p) => ({ id: p.id, name: p.name, role: p.role }));
      same(s.body, bridged(last.get(key), person, colleagues), 'the staff path answers with the /api/rpc snapshot rewritten (' + key + ')');
      claims.staffPathIsTheSnapshotRewritten++;
    }
  }
}
const rpc0 = World.prototype.rpc;
World.prototype.rpc = async function (expect, as, route, action, payload = {}, note, still) {
  const json = await rpc0.call(this, expect, as, route, action, payload, note, still);
  if (expect === 'ok' && route === 'rpc' && MODEL_ACTIONS.has(action)) await checkWrite(this, as, action, payload, json);
  return json;
};

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
  const out = '{\n "source": ' + JSON.stringify('v2 server.mjs (xchange-online@3ffc5a5) on PostgreSQL, every step through handle(); api/tools/v2-fixtures/gen_compat.mjs') + ',\n'
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

// ════ snapshot scenarios ════════════════════════════════════════════════════
const NINE = [['FR1', 'FRONT'], ['BE1', 'BACKEND'], ['MG1', 'MANAGER'], ['RT1', 'RATES'], ['MO1', 'MONITOR'], ['FI1', 'FINANCE'], ['SY1', 'SYSTEM'], ['DO1', 'DOCUMENTS'], ['RV1', 'DOCUMENT_REVIEW']];
const keys = (list) => list.map(([k]) => k);

async function empty() {
  const w = await World.open('snapshot/empty.json', 'A shop nobody has used yet: no rate, no shift, no customer, custody off. First the owner alone, then every role on both paths.');
  await w.views('nobody employed yet', ['owner']);
  await w.staff(NINE);
  await w.views('empty', ['owner', ...keys(NINE)]);
  await w.read('owner', 'rpc', 'getConnectionStatus');
  await w.read('FR1', 'rpc', 'getConnectionStatus');
  await w.read('FR1', 'staff', 'getConnectionStatus');
  await w.no('owner', 'rpc', 'adminSnapshot', { branchId: 'B-nowhere' }, 'a branch that does not exist');
  await w.no('owner', 'rpc', 'adminDeskConfig', { branchId: 'B-nowhere' });
  await w.no('owner', 'rpc', 'adminSnapshot', { branchId: 'not a branch!' }, 'not a branch id at all');
  await w.read('FR1', 'rpc', 'adminSnapshot', { branchId: 'B-nowhere' }, 'an employee who names another branch is answered for their own');
  await w.read('FR1', 'staff', 'adminSnapshot', { branchId: 'B-nowhere' });
  return w;
}

async function rates() {
  const w = await World.open('snapshot/rates.json', 'One branch whose counter rates were set, one of them twice and one with a partner cost; saved by the owner and by the rates role on both paths.');
  await w.staff([['FR1', 'FRONT'], ['RT1', 'RATES'], ['SY1', 'SYSTEM']]);
  await w.rate('owner', 'rpc', 'USD', 32.1, 32.6);
  await w.rate('owner', 'rpc', 'EUR', '37.5', '38.25', { partnerName: 'Partner FX', partnerCost: 37.4 });
  await w.rate('owner', 'rpc', 'JPY', 0.2105, 0.2199);
  await w.views('three rates', ['owner']);
  await w.rate('RT1', 'rpc', 'GBP', 42.5, 43.75);
  await w.rate('RT1', 'staff', 'USD', 32.15, 32.65, { partnerName: '=Partner starting with an equals sign', partnerCost: '31.9' });
  await w.views('four rates, the dollar set twice', ['owner', 'FR1', 'RT1', 'SY1']);
  return w;
}

async function shift() {
  const w = await World.open('snapshot/shift.json', 'Custody off. A shift is opened, money is moved in and out, the shift is closed with a difference, and the next one is opened: by the owner and by the roles /api/rpc lets do it.');
  await w.staff([['FR1', 'FRONT'], ['BE1', 'BACKEND'], ['MG1', 'MANAGER'], ['FI1', 'FINANCE'], ['RT1', 'RATES'], ['SY1', 'SYSTEM']]);
  await w.rate('owner', 'rpc', 'USD', 32.1, 32.6);
  let m = await w.ok('owner', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 50000, USD: '1000.50' } });
  const s1 = m.stock.shifts.at(-1).id;
  await w.views('a shift just opened', ['owner', 'FR1']);
  await w.ok('FI1', 'rpc', 'adminMoveStock', { requestId: w.rq(), shiftId: s1, currency: 'USD', direction: 'IN', amount: 500, reason: 'top up from the safe' });
  await w.ok('owner', 'rpc', 'adminMoveStock', { requestId: w.rq(), shiftId: s1, currency: 'THB', direction: 'OUT', amount: '1234.50', reason: '-deposit at the bank' });
  await w.views('an open shift with two movements', ['owner', 'FR1', 'BE1', 'MG1', 'FI1', 'RT1', 'SY1']);
  const expected = expectedOf(await w.model());
  await w.ok('MG1', 'rpc', 'adminCloseShift', { requestId: w.rq(), shiftId: s1, expected: nonZero(expected), counted: { ...nonZero(expected), THB: expected.THB - 0.5 }, note: 'half a baht short' });
  await w.views('one closed shift', ['owner', 'FR1']);
  m = await w.ok('BE1', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 48765, USD: 1500.5 } });
  const s2 = m.stock.shifts.at(-1).id;
  await w.ok('owner', 'rpc', 'adminMoveStock', { requestId: w.rq(), shiftId: s2, currency: 'EUR', direction: 'IN', amount: 0.01, reason: 'one cent found' });
  await w.views('a closed shift and an open one', ['owner', 'FR1', 'BE1', 'SY1']);
  return w;
}

async function transactions() {
  const w = await World.open('snapshot/transactions.json', 'Custody off. Customers and saved transactions, LIVE and DEMO, cash and transfer, by the owner and by a Front on both paths; a member; names that start with a formula sign.');
  await w.staff([['FR1', 'FRONT'], ['BE1', 'BACKEND'], ['RT1', 'RATES'], ['MO1', 'MONITOR'], ['SY1', 'SYSTEM'], ['DO1', 'DOCUMENTS']]);
  await w.rate('owner', 'rpc', 'USD', 32.1, 32.6);
  await w.rate('owner', 'rpc', 'EUR', 37.5, 38.25);
  await w.ok('owner', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 100000, USD: 2000, EUR: 500 } });
  const c1 = (await w.ok('owner', 'rpc', 'reserveCustomer', w.customerPayload(1))).customerId;
  await w.ok('owner', 'rpc', 'reserveCustomer', w.customerPayload(1), 'the same passport again: the same customer');
  const c2 = (await w.ok('FR1', 'rpc', 'reserveCustomer', w.customerPayload(2))).customerId;
  const c3 = (await w.ok('FR1', 'staff', 'reserveCustomer', { ...w.customerPayload(3), name: '=Customer whose name starts with an equals sign', birthDate: '', passportExpiry: '' })).customerId;
  const d1 = (await w.ok('owner', 'rpc', 'reserveCustomer', w.customerPayload(1, 'DEMO'), 'the same passport in DEMO: another customer, TEST- numbered')).customerId;
  let passport = await w.upload('owner', 'rpc', 'MAIN');
  const first = w.txPayload(1, c1, { type: 'BUY', currency: 'USD', amount: 100, rate: 32.1 }, { passport });
  await w.ok('owner', 'rpc', 'saveTransaction', first, 'the shop buys 100 dollars for cash');
  await w.ok('owner', 'rpc', 'saveTransaction', first, 'the same submission again: the same answer');
  await w.views('one transaction', ['owner']);
  passport = await w.upload('FR1', 'rpc');
  await w.ok('FR1', 'rpc', 'saveTransaction', w.txPayload(2, c2, { type: 'SELL', currency: 'USD', amount: '50.50', rate: '32.6', fee: 10, notes: '+a note that starts with a plus sign' }, { passport }), 'the shop sells 50.50 dollars for cash, with a fee');
  passport = await w.upload('FR1', 'staff');
  let slip = await w.upload('FR1', 'staff', null, 'pdf');
  await w.ok('FR1', 'staff', 'saveTransaction', w.txPayload(3, c3, { type: 'SELL', currency: 'EUR', amount: 200, rate: 38.25, fee: 25.5, bankFee: 5, method: 'Bank Transfer', ref: 'TRF-0001', account: 'Bank 000-0-00000-0', referenceRate: 37.9981 }, { passport, slip }), 'a transfer, with a slip and a reference rate, on the staff path');
  passport = await w.upload('owner', 'rpc', 'MAIN');
  slip = await w.upload('owner', 'rpc', 'MAIN', 'pdf');
  await w.ok('owner', 'rpc', 'saveTransaction', w.txPayload(1, c1, { type: 'BUY', currency: 'EUR', amount: 10, rate: 37.5, method: 'SCB Transfer', ref: 'TRF-0002', account: 'Bank 000-0-00000-0', referenceRate: '37.2034' }, { passport, slip }), 'the old method name: read back as Bank Transfer');
  passport = await w.upload('owner', 'rpc', 'MAIN');
  await w.ok('owner', 'rpc', 'saveTransaction', w.txPayload(1, d1, { type: 'SELL', currency: 'USD', amount: 5, rate: 99 }, { passport }, 'DEMO'), 'a DEMO save: no shift and no rate is asked of it, and no snapshot shows it');
  // a member: the bookings screens write this row (decision D5); it is written here directly
  await sql("INSERT INTO members(id,branch_id,customer_id,name,phone,token_hash,verified_at,verified_by) VALUES('M-1','MAIN',$1,'Customer 2','0800000000','-','2026-10-06T04:00:00.000Z','owner')", [c2]);
  w.steps.push({ n: w.steps.length + 1, at: new FakeDate().toISOString(), as: 'generator', sql: 'a members row for customer ' + c2 + ', verified_at 2026-10-06T04:00:00.000Z', note: 'not an RPC: the bookings API is decision D5' });
  await w.views('five saves, four of them LIVE, and a member', ['owner', 'FR1', 'BE1', 'RT1', 'MO1', 'SY1', 'DO1']);
  const ref = (await w.model()).transactions[0].id;
  await w.ok('FR1', 'staff', 'documents', { ref }, 'the documents of one transaction, as the staff path answers');
  await w.ok('DO1', 'staff', 'documents', { ref: (await w.model()).transactions[2].id });
  await w.no('RT1', 'staff', 'documents', { ref });
  await w.no('FR1', 'staff', 'documents', { ref: 'XC-00000000-000000' }, 'no such transaction');
  await w.no('owner', 'rpc', 'documents', { ref }, 'on /api/rpc `documents` is not an action for anybody');
  await w.raw('ok', 'owner', '/documents?branch=MAIN&ref=' + ref, undefined, { method: 'GET' }, 'the page the owner opens instead');
  return w;
}

async function branches() {
  const w = await World.open('snapshot/branches.json', 'Three branches. Each has its own rates, shifts and customers; an employee is always answered for their own branch; a request id that was given up.');
  await w.staff([['FR1', 'FRONT']]);
  await w.rate('owner', 'rpc', 'USD', 32.1, 32.6);
  const r1 = w.rq();
  let m = await w.ok('owner', 'rpc', 'adminCreateBranch', { requestId: r1, name: '  Airport  ' });
  const airport = m.branchId;
  await w.no('owner', 'rpc', 'adminCreateBranch', { requestId: r1, name: 'Airport' }, 'the same request again, sent from MAIN: v2 has not loaded the new branch\'s tables and says so in a sentence about table headers');
  await w.ok('owner', 'rpc', 'adminCreateBranch', { requestId: r1, name: 'Airport', branchId: airport }, 'the same request again, sent from the new branch: the new branch again');
  const town = (await w.ok('owner', 'rpc', 'adminCreateBranch', { requestId: w.rq(), name: 'Old Town', branchId: airport }, 'created while looking at another branch')).branchId;
  await w.staff([['FR2', 'FRONT'], ['BE2', 'BACKEND'], ['SY2', 'SYSTEM']], airport);
  await w.rate('owner', 'rpc', 'USD', 31.9, 32.9, {}, airport);
  await w.rate('owner', 'rpc', 'EUR', 37, 39, {}, airport);
  await w.ok('BE2', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 20000 }, branchId: 'MAIN' }, 'a Backend of the airport names MAIN: the shift opens at the airport');
  await w.ok('owner', 'rpc', 'reserveCustomer', w.customerPayload(1));
  await w.ok('FR2', 'rpc', 'reserveCustomer', w.customerPayload(1), 'the same passport at the airport: another customer, and the numbering goes on');
  for (const [label, branchId] of [['MAIN', undefined], ['the airport', airport], ['Old Town', town]]) for (const action of ['adminSnapshot', 'adminDeskConfig']) {
    const res = await w.send('owner', '/api/rpc', JSON.stringify({ action, payload: branchId ? { branchId } : {} }));
    w.record({ as: 'owner', method: 'POST', route: '/api/rpc', action, payload: branchId ? { branchId } : {}, view: 'three branches: ' + label }, res, 'ok');
  }
  await w.views('three branches: staff of MAIN', ['FR1']);
  await w.views('three branches: staff of the airport', ['FR2', 'BE2', 'SY2']);
  await w.read('FR2', 'rpc', 'adminSnapshot', { branchId: town }, 'naming Old Town changes nothing');
  await w.read('FR2', 'staff', 'adminDeskConfig', { branchId: town });
  // a request id that is given up
  const dead = w.rq();
  await w.ok('owner', 'rpc', 'adminAbandonRequest', { requestId: dead }, 'never applied: cancelled for good');
  await w.ok('owner', 'rpc', 'adminAbandonRequest', { requestId: dead }, 'again: the same answer');
  await w.no('owner', 'rpc', 'adminOpenShift', { requestId: dead, opening: { THB: 1 } }, 'a request that was given up can never be applied');
  await w.ok('owner', 'rpc', 'adminAbandonRequest', { requestId: dead, branchId: airport }, 'the same id at the airport: given up there too, separately');
  await w.ok('owner', 'rpc', 'adminAbandonRequest', { requestId: r1 }, 'the request that created the airport: it was applied');
  return w;
}

const TWELVE = [['FR1', 'FRONT'], ['FR2', 'FRONT'], ['FI1', 'FINANCE'], ['FI2', 'FINANCE'], ['MO1', 'MONITOR'], ['MG1', 'MANAGER'], ['BE1', 'BACKEND'], ['BE2', 'BACKEND'], ['RT1', 'RATES'], ['SY1', 'SYSTEM'], ['DO1', 'DOCUMENTS'], ['RV1', 'DOCUMENT_REVIEW']];
/** A custody request as the staff screen proposes it: its own evidence file first. */
async function propose(w, expect, who, kind, extra = {}, note) {
  const evidence = await w.upload(who, 'staff');
  const out = await w.rpc(expect, who, 'staff', 'opsPropose', { requestId: w.rq(), kind, evidenceKeys: [evidence], note: '', ...extra }, note);
  return out?.id;
}
async function custodyOn(w, staff = TWELVE) {
  await w.staff(staff);
  await w.ok('owner', '/api/employees', 'entity', { company: '  XC Exchange Co., Ltd.  ', site: ' Riverside B ', confirmSeparated: true }, 'opsSetEntity: the owner reaches it only through /api/employees');
  await w.ok('owner', '/api/employees', 'enable', { branchId: 'MAIN' }, 'opsEnable: likewise');
}

async function custody() {
  const w = await World.open('snapshot/custody.json', 'Custody on. One day at one branch through /api/staff/rpc: the safe and the drawer are counted in, service, drafts, saves, money to and from the safe and the bank, a refused request, a customer on hold, a handover, the close with a difference, the reconciliation. Requests are seen PENDING, in transit, REJECTED and COMPLETED.');
  await custodyOn(w);
  await w.views('custody on, nobody clocked in', ['owner', 'FR1']);
  for (const who of ['FR1', 'FR2', 'FI1', 'FI2', 'MO1', 'MG1', 'BE1', 'BE2', 'RT1', 'SY1']) await w.ok(who, 'staff', 'opsClock', { direction: 'IN' });
  await w.rate('RT1', 'staff', 'USD', 32.1, 32.6);
  await w.rate('RT1', 'staff', 'EUR', 37.5, 38.25);
  await w.ok('FI1', 'staff', 'opsTarget', { amounts: { THB: 100000, USD: '2000.00' } });
  let id = await propose(w, 'ok', 'FI1', 'SAFE_OPEN', count({ THB: 500000, USD: 10000, EUR: 2000 }));
  await w.views('the safe count waits for a signature', ['owner', 'FR1', 'FI1', 'MO1', 'RT1', 'SY1']);
  await w.ok('MO1', 'staff', 'opsSign', { id });
  await w.ok('BE1', 'staff', 'opsCommit', { id });
  id = await propose(w, 'ok', 'FR1', 'OPEN', { ...count({ THB: 100000, USD: 1990 }), note: '  ten dollars short  ' });
  await w.ok('BE1', 'staff', 'opsCommit', { id }, 'the shift opens; a cash exception is recorded');
  await w.views('the shift is open, service has not started', ['owner', 'FR1']);
  await w.ok('BE1', 'staff', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: (await w.model()).updated });
  // serving: the /staff/desk page begins a draft, reserves the customer, saves with the draft's id
  let draft = (await w.ok('FR1', 'staff', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 250 })).id;
  const c1 = (await w.ok('FR1', 'staff', 'reserveCustomer', w.customerPayload(1))).customerId;
  let passport = await w.upload('FR1', 'staff');
  await w.ok('FR1', 'staff', 'saveTransaction', { ...w.txPayload(1, c1, { type: 'BUY', currency: 'USD', amount: 250, rate: 32.1 }, { passport }), staffDraftId: draft }, 'saved with its draft: the draft is completed');
  await w.ok('FR1', 'staff', 'opsBeginTransaction', { type: 'SELL', currency: 'USD', amount: '100.00' }, 'a draft nobody finishes');
  const c2 = (await w.ok('FR1', 'staff', 'reserveCustomer', w.customerPayload(2))).customerId;
  passport = await w.upload('FR1', 'staff');
  const slip = await w.upload('FR1', 'staff', null, 'pdf');
  await w.ok('FR1', 'staff', 'saveTransaction', w.txPayload(2, c2, { type: 'SELL', currency: 'USD', amount: 100, rate: 32.6, fee: 25.5, method: 'Bank Transfer', ref: 'TRF-0101', account: 'Bank 000-0-00000-0' }, { passport, slip }), 'saved without a draft: allowed while service is open');
  // from the safe to the drawer
  id = await propose(w, 'ok', 'FR1', 'SAFE_OUT', { receiverId: w.id('FI1'), currency: 'USD', amount: 500, note: 'top up the drawer' });
  await w.ok('BE1', 'staff', 'opsDispatch', { id });
  await w.views('money from the safe is in transit', ['owner', 'FR1', 'BE1', 'FI1']);
  await w.ok('FI1', 'staff', 'opsSign', { id });
  await w.ok('BE1', 'staff', 'opsCommit', { id });
  // more than the drawer holds cannot leave it; Monitor sends the request back
  id = await propose(w, 'ok', 'FR1', 'SAFE_IN', { receiverId: w.id('FI1'), currency: 'USD', amount: 99999 });
  await w.no('BE1', 'staff', 'opsDispatch', { id }, 'the stock movement fails, so the whole dispatch does');
  await w.ok('MO1', 'staff', 'opsReject', { id, reason: 'the drawer never held that much' });
  id = await propose(w, 'ok', 'FR1', 'SAFE_IN', { receiverId: w.id('FI1'), currency: 'USD', amount: 300.25 });
  await w.ok('BE1', 'staff', 'opsDispatch', { id });
  await w.ok('FI1', 'staff', 'opsSign', { id });
  await w.ok('BE1', 'staff', 'opsCommit', { id });
  // to the bank and from the bank
  id = await propose(w, 'ok', 'FR1', 'EXTERNAL_OUT', { receiverId: w.id('MG1'), currency: 'THB', amount: 20000, note: 'to the bank' });
  await w.ok('BE1', 'staff', 'opsDispatch', { id });
  await w.ok('MG1', 'staff', 'opsSign', { id });
  await w.ok('FI1', 'staff', 'opsSign', { id });
  await w.ok('BE1', 'staff', 'opsCommit', { id });
  id = await propose(w, 'ok', 'FR1', 'EXTERNAL_IN', { receiverId: w.id('FR2'), currency: 'EUR', amount: '750.50' });
  await w.ok('BE1', 'staff', 'opsDispatch', { id });
  await w.ok('FR2', 'staff', 'opsSign', { id });
  await w.ok('FI1', 'staff', 'opsSign', { id });
  await w.ok('BE2', 'staff', 'opsCommit', { id });
  // a customer is held, and released; another stays held
  await w.ok('FR1', 'staff', 'opsHold', { customerId: c1, reason: 'passport looks altered' });
  await w.ok('MO1', 'staff', 'opsHold', { customerId: c2, reason: 'asked to wait for a second check' });
  await w.views('two customers on hold', ['owner', 'MO1', 'RT1']);
  passport = await w.upload('FR1', 'staff');
  await w.no('FR1', 'staff', 'saveTransaction', w.txPayload(1, c1, { type: 'BUY', currency: 'USD', amount: 1, rate: 32.1 }, { passport }), 'a customer on hold cannot be served');
  await w.ok('MO1', 'staff', 'opsRelease', { id: (await w.model()).operations.holds[0].id, note: 'checked with immigration, fine' });
  // the drawer is handed to the next Front
  await w.ok('BE1', 'staff', 'opsCutoff', {});
  let m = await w.model();
  id = await propose(w, 'ok', 'FR1', 'HANDOVER', { receiverId: w.id('FR2'), ...count(nonZero(expectedOf(m))), ...safeCount(nonZero(m.operations.safe)) });
  await w.views('counting for a handover', ['owner', ...keys(TWELVE)]);
  await w.ok('MG1', 'staff', 'opsSign', { id });
  await w.ok('MO1', 'staff', 'opsSign', { id });
  await w.ok('FR2', 'staff', 'opsSign', { id });
  await w.ok('BE1', 'staff', 'opsCommit', { id }, 'one shift closes, the next opens with what was counted; the receiver holds the drawer');
  await w.views('after the handover', ['owner', 'FR1', 'FR2']);
  const c3 = (await w.ok('FR2', 'staff', 'reserveCustomer', w.customerPayload(3))).customerId;
  passport = await w.upload('FR2', 'staff');
  await w.ok('FR2', 'staff', 'saveTransaction', w.txPayload(3, c3, { type: 'BUY', currency: 'USD', amount: 10, rate: 32.1 }, { passport }));
  // the close: a hundred baht short in the drawer and one euro over in the safe
  await w.ok('BE1', 'staff', 'opsCutoff', {});
  m = await w.model();
  const exp = nonZero(expectedOf(m)), safe = nonZero(m.operations.safe);
  id = await propose(w, 'ok', 'FR2', 'CLOSE', { ...count({ ...exp, THB: Math.round((exp.THB - 100) * 100) / 100 }), ...safeCount({ ...safe, EUR: Math.round((safe.EUR + 1) * 100) / 100 }), note: 'a hundred baht short; one euro over in the safe' });
  await w.ok('MG1', 'staff', 'opsSign', { id });
  await w.ok('MO1', 'staff', 'opsSign', { id });
  await w.ok('BE2', 'staff', 'opsCommit', { id }, 'the shift closes; nobody holds the drawer; an exception is recorded');
  await w.views('closed, before the reconciliation', ['owner', 'FI1']);
  await w.ok('FI1', 'staff', 'opsReconcile', { id, note: 'counted again with the manager, written off' });
  await w.ok('FR2', 'staff', 'opsClock', { direction: 'OUT' });
  await w.views('closed for the night', ['owner', ...keys(TWELVE)]);
  return w;
}

// ════ rpc scenarios ═════════════════════════════════════════════════════════
async function transport() {
  const w = await World.open('rpc/transport.json', 'What both RPC routes do with a request before any action runs: the two headers of the same-origin rule, who is signed in, the method, a body that is not what the screens send, and the size limit.');
  await w.staff([['FR1', 'FRONT'], ['RT1', 'RATES']]);
  await w.employee('NEW', 'FRONT', 'MAIN', { mustChange: true });
  const snap = JSON.stringify({ action: 'adminSnapshot', payload: {} });
  const status = JSON.stringify({ action: 'getConnectionStatus', payload: {} });
  for (const route of ['/api/rpc', '/api/staff/rpc']) {
    const insider = 'FR1';
    await w.raw('ok', insider, route, status, {}, 'the request the screens send');
    // the same-origin rule
    await w.raw('no', insider, route, status, { headers: { origin: null } }, 'no Origin');
    await w.raw('no', insider, route, status, { headers: { origin: 'http://elsewhere.test' } }, 'another Origin');
    await w.raw('no', insider, route, status, { headers: { origin: ORIGIN + '/' } }, 'the Origin with a slash after it');
    await w.raw('no', insider, route, status, { headers: { 'x-xchange-request': null } }, 'no X-Xchange-Request');
    await w.raw('no', insider, route, status, { headers: { 'x-xchange-request': 'true' } }, 'X-Xchange-Request that is not 1');
    await w.raw('ok', insider, route, status, { headers: { 'content-type': 'text/plain' } }, 'the content type is not looked at');
    await w.raw('ok', insider, route, status, { headers: { 'content-type': null } });
    // who is calling
    await w.raw('no', null, route, status, {}, 'nobody signed in');
    await w.raw('no', null, route, status, { headers: { origin: null } }, 'nobody signed in, and no Origin: which is said first');
    await w.raw('no', null, route, status, { headers: { cookie: '__Host-xc_staff=' + 'f'.repeat(64) } }, 'a session that does not exist');
    await w.raw('no', null, route, status, { headers: { cookie: '__Host-xc_staff=short' } }, 'a cookie that is not a session token');
    await w.raw(route === '/api/rpc' ? 'ok' : 'no', 'owner', route, status, {}, 'the owner');
    await w.raw('no', 'NEW', route, status, {}, 'an employee who has not changed the first password');
    // the method
    for (const method of ['GET', 'PUT', 'DELETE']) await w.raw('no', insider, route, method === 'GET' ? undefined : status, { method }, 'method ' + method);
    // the body
    for (const [body, note] of [['', 'an empty body'], ['not json', 'not JSON'], ['{"action":"getConnectionStatus"', 'JSON cut short'], ['null', 'JSON null'], ['[]', 'an array'], ['"getConnectionStatus"', 'a string'], ['{}', 'no action'],
      ['{"action":"getConnectionStatus"}', 'no payload: read as {}'], ['{"action":"getConnectionStatus","payload":null}', 'payload null'], ['{"action":"getConnectionStatus","payload":[]}', 'payload an array'],
      ['{"action":"getConnectionStatus","payload":"x"}', 'payload a string'], ['{"action":"getConnectionStatus","payload":7}', 'payload a number'], ['{"action":"getConnectionStatus","payload":{},"more":1}', 'a key the server does not know'],
      ['{"action":7,"payload":{}}', 'action a number'], ['{"action":["getConnectionStatus"],"payload":{}}', 'action an array of one name'], ['{"action":"nope","payload":{}}', 'an action that does not exist'],
      ['{"action":"adminsnapshot","payload":{}}', 'the right name in the wrong case'], ['{"action":"getConnectionStatus ","payload":{}}', 'a space after the name'], ['{"action":"opsNope","payload":{}}', 'an unknown name starting with ops'],
      ['{"action":"__proto__","payload":{}}', 'a name every object has'], ['{"action":"toString","payload":{}}', 'another one'],
      ['{"action":"getConnectionStatus","payload":{"branchId":"no such branch!"}}', 'a branch id that is not one'], ['{"action":"getConnectionStatus","payload":{"branchId":"B-nowhere"}}', 'a branch that does not exist']]) {
      await w.raw(undefined, insider, route, body, {}, note);
    }
    for (const [body, note] of [['{"action":"nope","payload":{}}', 'an action that does not exist, asked by the owner'], ['{"action":"getConnectionStatus","payload":[]}', 'payload an array, asked by the owner'], ['not json', 'not JSON, asked by the owner'], ['{"action":"getConnectionStatus","payload":{"branchId":"no such branch!"}}', 'a branch id that is not one, asked by the owner']]) {
      if (route === '/api/rpc') await w.raw(undefined, 'owner', route, body, {}, note);
    }
    // the size limit: 120,000 characters (UTF-16 units), not bytes
    const padded = (n, ch = 'x') => { const head = '{"action":"getConnectionStatus","payload":{"pad":"', tail = '"}}'; return head + ch.repeat(n - head.length - tail.length) + tail; };
    await w.raw('ok', insider, route, padded(120000), {}, 'exactly 120,000 characters');
    await w.raw('no', insider, route, padded(120001), {}, '120,001 characters');
    await w.raw('ok', insider, route, padded(120000, 'ก'), {}, '120,000 characters of Thai: about 360,000 bytes, accepted');
    if (route === '/api/rpc') await w.raw('no', 'owner', route, padded(120001), {}, '120,001 characters, asked by the owner');
  }
  await w.raw('no', 'FR1', '/api/staff/rpc/extra', snap, {}, 'a longer path');
  await w.raw('no', 'FR1', '/api/staff/nope', snap, {}, 'a staff route that does not exist');
  const res = await v2.handle(new Request('http://customer.test/api/rpc', { method: 'POST', headers: { origin: 'http://customer.test', 'x-xchange-request': '1', cookie: w.tokens.get('owner') }, body: snap }));
  w.record({ as: 'owner', method: 'POST', route: 'http://customer.test/api/rpc', raw: snap, note: 'the customer host serves the kiosk only' }, { status: res.status, headers: Object.fromEntries(res.headers), text: await res.text() }, 'no');
  return w;
}

async function engineActions() {
  const w = await World.open('rpc/engine_actions.json', 'The eleven actions of /api/rpc and `documents`, one after another: the payload each accepts, what it answers, and every refusal this script could reach, with custody off. Refusals that depend on who is calling are in the policy fixture (#65); a few are repeated here to show the envelope.');
  await w.staff([['FR1', 'FRONT'], ['BE1', 'BACKEND'], ['MG1', 'MANAGER'], ['RT1', 'RATES'], ['FI1', 'FINANCE'], ['MO1', 'MONITOR']]);
  const o = (action, payload, note) => w.ok('owner', 'rpc', action, payload, note), x = (action, payload, note) => w.no('owner', 'rpc', action, payload, note);
  // ── getConnectionStatus
  await o('getConnectionStatus', {});
  await o('getConnectionStatus', { anything: 'is ignored' });
  await x('getConnectionStatus', { branchId: 'B-nowhere' });
  // ── adminSaveRate
  await x('adminSaveRate', { code: 'USD', buy: 32.1, sell: 32.6, previousUpdated: '' }, 'no request id');
  await x('adminSaveRate', { requestId: 'short', code: 'USD', buy: 32.1, sell: 32.6, previousUpdated: '' }, 'a request id under 16 characters');
  await x('adminSaveRate', { requestId: 'has spaces in it and more', code: 'USD', buy: 32.1, sell: 32.6 }, 'a request id with other characters');
  await x('adminSaveRate', { requestId: w.rq(), buy: 32.1, sell: 32.6 }, 'no currency');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', sell: 32.6 }, 'no buy rate');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 0, sell: 32.6 }, 'a rate of zero');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: -1, sell: 32.6 });
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 'abc', sell: 32.6 });
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.12345, sell: 32.6 }, 'five decimals');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 100000001, sell: 100000002 }, 'over 100,000,000');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.6, sell: 32.1 }, 'sells for less than it buys');
  await x('adminSaveRate', { requestId: w.rq(), code: 'THB', buy: 1, sell: 1 }, 'the baht itself');
  await x('adminSaveRate', { requestId: w.rq(), code: 'XXX', buy: 1, sell: 1 }, 'a currency that is not in the register');
  await x('adminSaveRate', { requestId: w.rq(), code: 'usd', buy: 1, sell: 1 }, 'lower case');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.1, sell: 32.6, partnerName: 'Partner FX' }, 'a partner without a cost');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.1, sell: 32.6, partnerCost: 31.9 }, 'a cost without a partner');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.1, sell: 32.6, partnerName: 'n'.repeat(101), partnerCost: 31.9 }, 'a partner name of 101 characters');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.1, sell: 32.6, previousUpdated: '2026-01-01T00:00:00.000Z' }, 'the screen believes a rate exists; none does');
  botDown = true;
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.1, sell: 32.6, previousUpdated: '' }, 'the Bank of Thailand does not answer');
  botDown = false;
  const rateRq = w.rq(), rate = { requestId: rateRq, code: 'USD', buy: 32.1, sell: 32.6, previousUpdated: '' };
  let m = await o('adminSaveRate', rate, 'accepted: answers with the whole snapshot');
  await o('adminSaveRate', rate, 'the same request again: the snapshot again, nothing new');
  await o('adminSaveRate', { ...rate, previousUpdated: 'whatever' }, 'the same request with another previousUpdated: still the same request');
  await x('adminSaveRate', { ...rate, sell: 32.7 }, 'the same request id with another rate');
  await x('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: 32.2, sell: 32.7, previousUpdated: '' }, 'the screen has not seen the rate that is there');
  await o('adminSaveRate', { requestId: w.rq(), code: 'USD', buy: '32.2', sell: '32.7', previousUpdated: m.updated, partnerName: ' Partner FX ', partnerCost: '31.95' }, 'figures as text, a partner');
  await w.ok('RT1', 'rpc', 'adminSaveRate', { requestId: w.rq(), code: 'EUR', buy: 37.5, sell: 38.25, previousUpdated: '' });
  await w.no('FR1', 'rpc', 'adminSaveRate', { requestId: w.rq(), code: 'GBP', buy: 42, sell: 43, previousUpdated: '' }, 'not the rates role, on /api/rpc');
  await w.no('FR1', 'staff', 'adminSaveRate', { requestId: w.rq(), code: 'GBP', buy: 42, sell: 43, previousUpdated: '' }, 'not the rates role, on the staff path');
  // ── adminOpenShift
  await x('adminOpenShift', { opening: { THB: 1000 } }, 'no request id');
  await x('adminOpenShift', { requestId: w.rq(), opening: { THB: -1 } });
  await x('adminOpenShift', { requestId: w.rq(), opening: { THB: '' } }, 'an empty amount');
  await x('adminOpenShift', { requestId: w.rq(), opening: { THB: 1.005 } }, 'three decimals');
  await x('adminOpenShift', { requestId: w.rq(), opening: { THB: 100000001 } });
  await x('adminOpenShift', { requestId: w.rq(), opening: { THB: 'abc' } });
  await x('adminMoveStock', { requestId: w.rq(), shiftId: 'SH-none', currency: 'USD', direction: 'IN', amount: 1, reason: 'no shift is open' });
  await x('adminCloseShift', { requestId: w.rq(), shiftId: 'SH-none', counted: {}, expected: {} }, 'no shift is open');
  const openRq = w.rq(), opening = { requestId: openRq, opening: { THB: 50000, USD: 1000, XXX: 5 } };
  m = await o('adminOpenShift', opening, 'accepted; a currency that is not in the register is ignored');
  const shiftId = m.stock.shifts.at(-1).id;
  await o('adminOpenShift', opening, 'the same request again');
  await x('adminOpenShift', { ...opening, opening: { THB: 50001 } }, 'the same request id with other figures');
  await x('adminOpenShift', { requestId: w.rq(), opening: {} }, 'a shift is already open');
  await x('adminOpenShift', { requestId: openRq }, 'the same request id with no opening at all: read as all zero, which is not what was sent before');
  await w.no('FR1', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: {} }, 'a Front, on /api/rpc');
  await w.no('BE1', 'staff', 'adminOpenShift', { requestId: w.rq(), opening: {} }, 'a Backend, on the staff path: nobody opens a shift there');
  // ── adminMoveStock
  const move = (extra) => ({ requestId: w.rq(), shiftId, currency: 'USD', direction: 'IN', amount: 100, reason: 'a reason', ...extra });
  await x('adminMoveStock', move({ requestId: undefined }), 'no request id');
  await x('adminMoveStock', move({ currency: undefined }));
  await x('adminMoveStock', move({ currency: 'XXX' }));
  await x('adminMoveStock', move({ currency: 'USDT' }), 'four letters');
  await x('adminMoveStock', move({ direction: 'UP' }));
  await x('adminMoveStock', move({ direction: 'INWARD' }));
  await x('adminMoveStock', move({ amount: 0 }));
  await x('adminMoveStock', move({ amount: '' }));
  await x('adminMoveStock', move({ amount: -5 }));
  await x('adminMoveStock', move({ amount: 1.005 }));
  await x('adminMoveStock', move({ reason: '' }), 'no reason');
  await x('adminMoveStock', move({ reason: 'r'.repeat(201) }), 'a reason of 201 characters');
  await x('adminMoveStock', move({ shiftId: undefined }));
  await x('adminMoveStock', move({ shiftId: 'SH-' + idAt(0xdead) }), 'another shift');
  await x('adminMoveStock', move({ direction: 'OUT', amount: 1000.01 }), 'more than the drawer holds');
  await x('adminMoveStock', move({ direction: 'OUT', currency: 'EUR', amount: 0.01 }), 'a currency the drawer does not hold');
  const moved = move({ direction: 'OUT', amount: '250.25', reason: '  to the safe  ' });
  await o('adminMoveStock', moved, 'accepted');
  await o('adminMoveStock', moved, 'the same request again');
  await x('adminMoveStock', { ...moved, amount: 250.26 }, 'the same request id with another amount');
  await w.ok('FI1', 'rpc', 'adminMoveStock', move({ currency: 'THB', amount: 0.01 }), 'Finance, on /api/rpc');
  await w.no('FI1', 'staff', 'adminMoveStock', move({}), 'Finance, on the staff path');
  // ── reserveCustomer
  const cust = w.customerPayload(1);
  await x('reserveCustomer', { ...cust, name: '' });
  await x('reserveCustomer', { ...cust, name: 'n'.repeat(101) });
  await x('reserveCustomer', { ...cust, passportNo: 'P'.repeat(31) });
  await x('reserveCustomer', { ...cust, nationality: undefined });
  await x('reserveCustomer', { ...cust, mode: 'REAL' }, 'neither LIVE nor DEMO');
  await x('reserveCustomer', { ...cust, mode: undefined });
  await x('reserveCustomer', { ...cust, birthDate: '01/02/1990' }, 'a date in another format');
  await x('reserveCustomer', { ...cust, birthDate: '1990-02-30' }, 'a day that does not exist');
  await x('reserveCustomer', { ...cust, passportExpiry: '2031-13-01' });
  const c1 = (await o('reserveCustomer', { ...cust, name: '  Customer One  ', passportNo: ' p1000001 ', nationality: 'de' }, 'accepted: trimmed, passport and nationality in capitals')).customerId;
  await o('reserveCustomer', { ...cust, name: 'Another Name' }, 'the same passport: the same customer, the name is not changed here');
  await o('reserveCustomer', { name: 'Customer Without Dates', passportNo: 'P2000002', nationality: 'DE', mode: 'LIVE' }, 'no dates');
  await w.ok('FR1', 'rpc', 'reserveCustomer', w.customerPayload(3));
  await w.no('BE1', 'rpc', 'reserveCustomer', w.customerPayload(4), 'not a Front');
  // ── saveTransaction
  const passport = await w.upload('owner', 'rpc', 'MAIN'), slip = await w.upload('owner', 'rpc', 'MAIN', 'pdf');
  const tx = (over = {}, files = { passport }) => ({ ...w.txPayload(1, c1, { type: 'BUY', currency: 'USD', amount: 100, rate: 32.2 }, files), ...over });
  await x('saveTransaction', tx({ submissionId: 'short' }));
  await x('saveTransaction', tx({ submissionId: undefined }));
  await x('saveTransaction', tx({ type: 'SWAP' }));
  await x('saveTransaction', tx({ currency: 'usd' }));
  await x('saveTransaction', tx({ amount: 0 }));
  await x('saveTransaction', tx({ amount: 1.005 }), 'three decimals');
  await x('saveTransaction', tx({ amount: 100000001 }));
  await x('saveTransaction', tx({ rate: 32.20001 }), 'five decimals');
  await x('saveTransaction', tx({ fee: -1 }));
  await x('saveTransaction', tx({ method: 'Cheque' }));
  await x('saveTransaction', tx({ paidAt: 'yesterday' }));
  await x('saveTransaction', tx({ valueDate: '06/10/2026' }));
  await x('saveTransaction', tx({ valueDate: undefined }));
  await x('saveTransaction', tx({ account: '' }), 'no account');
  await x('saveTransaction', tx({ booking: '' }), 'no booking number');
  await x('saveTransaction', tx({ paymentConfirmed: 'yes' }), 'the confirmation must be the boolean true');
  await x('saveTransaction', tx({ identityConfirmed: false }));
  await x('saveTransaction', tx({ countConfirmed: undefined }), 'cash that nobody said was counted');
  await x('saveTransaction', tx({ method: 'Bank Transfer' }), 'a transfer without a reference');
  await x('saveTransaction', tx({ method: 'Bank Transfer', ref: 'TRF-1' }), 'a transfer without a slip');
  await x('saveTransaction', tx({ fee: 3220.01 }), 'a fee above what the shop pays');
  await x('saveTransaction', tx({ passport: undefined }), 'no passport file');
  await x('saveTransaction', tx({ passport: { uploadKey: 'not-a-key' } }));
  await x('saveTransaction', tx({ passport: { uploadKey: idAt(0xdead) } }), 'a file nobody uploaded');
  await x('saveTransaction', tx({ passport: { base64: 'AAAA', type: 'image/png' } }), 'the file itself instead of an upload key');
  const fronts = await w.upload('FR1', 'rpc');
  await x('saveTransaction', tx({ passport: { uploadKey: fronts } }), 'a file somebody else uploaded');
  await x('saveTransaction', tx({ customerId: 'CX-99999999' }), 'a customer number that is not this passport');
  await x('saveTransaction', tx({ passportNo: 'P9999999' }));
  await x('saveTransaction', tx({ rate: 32.1 }), 'not the counter rate');
  await x('saveTransaction', tx({ type: 'SELL', rate: 32.7, amount: 750 }), 'more dollars than the drawer holds');
  await x('saveTransaction', tx({ amount: 2000 }), 'more baht than the drawer holds');
  await x('saveTransaction', tx({ currency: 'GBP', rate: 42 }), 'a currency without a counter rate');
  await x('saveTransaction', tx({ mode: 'REAL' }));
  await x('saveTransaction', tx({ referenceRate: -1 }));
  await x('saveTransaction', tx({ notes: 'n'.repeat(1001) }));
  const good = tx({ notes: 'first', referenceRate: 32.4876 });
  const saved = await o('saveTransaction', good, 'accepted');
  await o('saveTransaction', good, 'the same submission again: the same answer');
  await x('saveTransaction', { ...good, notes: 'second' }, 'the same submission id with other content');
  await o('saveTransaction', tx({ type: 'SELL', rate: 32.7, amount: 20, method: 'PromptPay', ref: 'PP-1', account: 'PromptPay 000' }, { passport, slip }), 'a transfer with its slip');
  await o('saveTransaction', tx({ mode: 'DEMO', customerId: (await o('reserveCustomer', w.customerPayload(1, 'DEMO'))).customerId, rate: 1 }), 'DEMO: any rate, no shift needed');
  await w.no('BE1', 'rpc', 'saveTransaction', tx(), 'not a Front');
  // ── documents
  await w.ok('FR1', 'staff', 'documents', { ref: saved.documentRef });
  await w.no('FR1', 'staff', 'documents', {}, 'no reference');
  await w.no('RT1', 'staff', 'documents', { ref: saved.documentRef });
  await w.no('owner', 'rpc', 'documents', { ref: saved.documentRef });
  await w.no('FR1', 'rpc', 'documents', { ref: saved.documentRef });
  // ── adminCloseShift
  m = await w.model();
  const expected = nonZero(expectedOf(m)), close = (extra) => ({ requestId: w.rq(), shiftId, expected, counted: expected, note: '', ...extra });
  await x('adminCloseShift', close({ requestId: undefined }));
  await x('adminCloseShift', close({ shiftId: undefined }));
  await x('adminCloseShift', close({ shiftId: 'SH-' + idAt(0xdead) }), 'another shift');
  await x('adminCloseShift', close({ counted: { THB: -1 } }));
  await x('adminCloseShift', close({ expected: { ...expected, THB: expected.THB + 1 } }), 'the screen counted against figures that are no longer true');
  await x('adminCloseShift', close({ expected: undefined }), 'no expected figures');
  await x('adminCloseShift', close({ counted: { ...expected, THB: expected.THB - 1 } }), 'a difference without a note');
  await x('adminCloseShift', close({ note: 'n'.repeat(301) }));
  await w.no('FR1', 'rpc', 'adminCloseShift', close({}), 'a Front');
  const closing = close({ counted: { ...expected, USD: expected.USD + 0.01 }, note: ' one cent over ' });
  await w.ok('MG1', 'rpc', 'adminCloseShift', closing, 'accepted, by a Manager');
  await o('adminCloseShift', closing, 'the same request again');
  await x('adminCloseShift', { ...closing, note: 'another note' }, 'the same request id with another note');
  await x('saveTransaction', tx(), 'a LIVE save with no shift open');
  // ── adminAbandonRequest
  await x('adminAbandonRequest', {});
  await x('adminAbandonRequest', { requestId: 'short' });
  const dead = w.rq();
  await o('adminAbandonRequest', { requestId: dead }, 'never applied: CANCELLED');
  await x('adminSaveRate', { requestId: dead, code: 'GBP', buy: 42, sell: 43, previousUpdated: '' }, 'a request that was given up');
  await x('adminOpenShift', { requestId: dead, opening: {} });
  await o('adminAbandonRequest', { requestId: rateRq }, 'a rate request that was applied: APPLIED');
  await o('adminAbandonRequest', { requestId: openRq }, 'a shift request that was applied');
  await w.no('FR1', 'rpc', 'adminAbandonRequest', { requestId: w.rq() }, 'no employee may give a request up');
  // ── adminCreateBranch
  await x('adminCreateBranch', { name: 'Airport' }, 'no request id');
  await x('adminCreateBranch', { requestId: w.rq() }, 'no name');
  await x('adminCreateBranch', { requestId: w.rq(), name: 'n'.repeat(61) });
  await x('adminCreateBranch', { requestId: w.rq(), name: 'xc-b' }, 'the name of MAIN in another case');
  await x('adminCreateBranch', { requestId: dead, name: 'Airport' }, 'a request that was given up');
  const branchRq = w.rq();
  const b = await o('adminCreateBranch', { requestId: branchRq, name: 'Airport' }, 'accepted: answers with the snapshot of the new branch');
  await x('adminCreateBranch', { requestId: branchRq, name: 'Airport' }, 'the same request again, sent from MAIN (see snapshot/branches.json)');
  await o('adminCreateBranch', { requestId: branchRq, name: 'Airport', branchId: b.branchId }, 'the same request again, sent from the new branch');
  await x('adminCreateBranch', { requestId: branchRq, name: 'Airport 2' }, 'the same request id with another name');
  await x('adminCreateBranch', { requestId: w.rq(), name: 'AIRPORT' });
  await w.no('MG1', 'rpc', 'adminCreateBranch', { requestId: w.rq(), name: 'Harbour' }, 'no employee may');
  await o('adminSnapshot', { branchId: b.branchId }, 'the new branch');
  await o('adminDeskConfig', { branchId: b.branchId });
  for (let i = 3; i <= 20; i++) await o('adminCreateBranch', { requestId: w.rq(), name: 'Branch ' + i });
  await x('adminCreateBranch', { requestId: w.rq(), name: 'Branch 21' }, 'the twenty-first branch');
  return w;
}

async function opsActions() {
  const w = await World.open('rpc/ops_actions.json', 'The fifteen custody actions as they travel: thirteen through /api/staff/rpc, and opsSetEntity and opsEnable through /api/employees, the only door v2 gives them. Each is refused and accepted at least once. What the custody rules themselves refuse is recorded in full in #72\'s fixture; here it is the envelope, the checks the routes add, and the shift actions custody closes.');
  await w.staff(TWELVE);
  const st = (expect, who, action, payload, note) => w.rpc(expect, who, 'staff', action, payload, note);
  // before the owner turns custody on
  await st('no', 'FR1', 'opsClock', { direction: 'IN' }, 'custody is off');
  await st('no', 'FR1', 'opsNope', {}, 'an unknown name starting with ops reaches the engine');
  await st('no', 'FR1', 'opsEnable', {}, 'no employee turns custody on');
  await st('no', 'FR1', 'opsSetEntity', { company: 'C', site: 'S', confirmSeparated: true });
  await w.no('owner', 'rpc', 'opsEnable', {}, '/api/rpc knows no ops action, for the owner');
  await w.no('owner', 'rpc', 'opsClock', { direction: 'IN' });
  await w.no('FR1', 'rpc', 'opsClock', { direction: 'IN' }, '/api/rpc knows no ops action, for an employee');
  await w.no('owner', 'staff', 'opsEnable', {}, 'the owner has no session on the staff path');
  await w.no('owner', '/api/employees', 'enable', { branchId: 'MAIN' }, 'no entity yet');
  await w.no('owner', '/api/employees', 'entity', { company: 'XC Exchange Co., Ltd.', site: 'Riverside B', confirmSeparated: 'yes' }, 'the confirmation must be the boolean true');
  await w.no('owner', '/api/employees', 'entity', { company: '', site: 'Riverside B', confirmSeparated: true });
  await w.no('FR1', '/api/employees', 'entity', { company: 'C', site: 'S', confirmSeparated: true }, 'not the owner');
  await w.ok('owner', '/api/employees', 'entity', { company: '  XC Exchange Co., Ltd.  ', site: ' Riverside B ', confirmSeparated: true });
  await w.no('owner', '/api/employees', 'enable', {}, 'no branch named: no employee matches');
  await w.ok('owner', '/api/employees', 'enable', { branchId: 'MAIN' });
  await w.ok('owner', '/api/employees', 'enable', { branchId: 'MAIN' }, 'again');
  await w.no('owner', '/api/employees', 'entity', { company: 'Other', site: 'Other', confirmSeparated: true }, 'custody is already on');
  // the shift actions are closed while custody is on
  await w.no('owner', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 1 } }, 'custody is on: shifts open and close through custody only');
  await w.no('owner', 'rpc', 'adminMoveStock', { requestId: w.rq(), shiftId: 'x', currency: 'THB', direction: 'IN', amount: 1, reason: 'r' });
  await w.no('owner', 'rpc', 'adminCloseShift', { requestId: w.rq(), shiftId: 'x', counted: {}, expected: {} });
  await w.no('BE1', 'rpc', 'adminOpenShift', { requestId: w.rq(), opening: { THB: 1 } });
  // opsClock
  await st('no', 'FR1', 'opsTarget', { amounts: {} }, 'not clocked in');
  await st('no', 'FR1', 'opsNope', {}, 'unknown, not clocked in');
  await st('no', 'FR1', 'opsClock', {}, 'no direction');
  await st('no', 'FR1', 'opsClock', { direction: 'UP' });
  await st('no', 'FR1', 'opsClock', { direction: 'OUT' }, 'not clocked in yet');
  for (const who of ['FR1', 'FR2', 'FI1', 'FI2', 'MO1', 'MG1', 'BE1', 'BE2', 'RT1', 'SY1', 'DO1', 'RV1']) await st('ok', who, 'opsClock', { direction: 'IN' });
  await st('ok', 'FR1', 'opsClock', { direction: 'IN' }, 'again: accepted, nothing new');
  await st('no', 'FR1', 'opsNope', {}, 'unknown, clocked in, no id');
  // opsTarget
  await st('no', 'FR1', 'opsTarget', { amounts: { THB: 1 } }, 'not Finance');
  await st('no', 'FI1', 'opsTarget', { amounts: { USD: -1 } });
  await st('ok', 'FI1', 'opsTarget', { amounts: { THB: 100000, USD: '2000.00', XXX: 'ignored' } });
  // opsPropose: what the routes add
  await st('no', 'FI1', 'opsPropose', { requestId: w.rq(), kind: 'SAFE_OPEN', ...count({ THB: 1000 }) }, 'no evidence list');
  await st('no', 'FI1', 'opsPropose', { requestId: w.rq(), kind: 'SAFE_OPEN', evidenceKeys: [1, 2, 3, 4, 5, 6], ...count({ THB: 1000 }) }, 'six files');
  await st('no', 'FI1', 'opsPropose', { requestId: w.rq(), kind: 'SAFE_OPEN', evidenceKeys: [idAt(0xdead)], ...count({ THB: 1000 }) }, 'a file nobody uploaded');
  const frontsFile = await w.upload('FR1', 'staff');
  await st('no', 'FI1', 'opsPropose', { requestId: w.rq(), kind: 'SAFE_OPEN', evidenceKeys: [frontsFile], ...count({ THB: 1000 }) }, 'a file somebody else uploaded');
  await st('no', 'FI1', 'opsPropose', { requestId: w.rq(), kind: 'SAFE_OPEN', evidenceKeys: [], ...count({ THB: 1000 }) }, 'an empty evidence list passes the route and is refused by the custody rules');
  await w.upload('MO1', 'staff', null, 'png', 'no', 'only Front and Finance upload');
  await propose(w, 'no', 'FI1', 'GIFT', count({ THB: 1 }), 'a kind that does not exist');
  let id = await propose(w, 'ok', 'FI1', 'SAFE_OPEN', count({ THB: 500000, USD: 10000 }));
  await propose(w, 'no', 'FR1', 'OPEN', count({ THB: 100000, USD: 2000 }), 'one request at a time');
  // opsSign, opsCommit
  await st('no', 'BE1', 'opsCommit', { id }, 'Monitor has not signed');
  await st('no', 'BE1', 'opsSign', { id }, 'Backend does not sign');
  await st('no', 'RT1', 'opsSign', { id }, 'a role that signs nothing');
  await st('no', 'MO1', 'opsSign', { id: idAt(0xdead) }, 'no such request');
  await st('no', 'MO1', 'opsSign', {}, 'no id');
  await st('no', 'MO1', 'opsNope', { id }, 'unknown, with the id of a real request');
  await st('ok', 'MO1', 'opsSign', { id });
  await st('ok', 'MO1', 'opsSign', { id }, 'again');
  await st('no', 'MO1', 'opsCommit', { id }, 'not Backend');
  await st('ok', 'BE1', 'opsCommit', { id });
  await st('ok', 'BE1', 'opsCommit', { id }, 'again');
  await propose(w, 'no', 'FR1', 'OPEN', count({ THB: 100000, USD: 2000 }), 'no counter rate yet');
  await w.rate('RT1', 'staff', 'USD', 32.1, 32.6);
  await propose(w, 'no', 'FR1', 'HANDOVER', { receiverId: w.id('MO1'), ...count({ THB: 1 }), ...safeCount({ THB: 1 }) }, 'the staff path checks the receiver: a handover goes to a Front');
  await propose(w, 'no', 'FR1', 'SAFE_OUT', { receiverId: w.id('BE1'), currency: 'USD', amount: 1 }, 'money goes to a Front, a Manager or Finance');
  await propose(w, 'no', 'FR1', 'SAFE_OUT', { receiverId: idAt(0xdead), currency: 'USD', amount: 1 }, 'a receiver who does not exist');
  id = await propose(w, 'ok', 'FR1', 'OPEN', count({ THB: 100000, USD: 2000 }));
  // opsReject
  await st('no', 'FR1', 'opsReject', { id, reason: 'x' }, 'not Monitor or Backend');
  await st('no', 'MO1', 'opsReject', { id }, 'no reason');
  await st('ok', 'MO1', 'opsReject', { id, reason: 'count it again' });
  await st('no', 'MO1', 'opsReject', { id, reason: 'again' }, 'already finished');
  id = await propose(w, 'ok', 'FR1', 'OPEN', count({ THB: 100000, USD: 2000 }));
  // opsStartService, opsBeginTransaction, opsCutoff
  await st('no', 'BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: (await w.model()).updated }, 'no shift yet');
  await st('no', 'BE1', 'opsCutoff', {}, 'no shift yet');
  await st('ok', 'BE1', 'opsCommit', { id }, 'the shift opens');
  await st('no', 'FR1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 100 }, 'service has not started');
  await st('no', 'FR1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: (await w.model()).updated }, 'not Backend');
  await st('no', 'BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: '2026-01-01T00:00:00.000Z' }, 'a stale rate set');
  await st('ok', 'BE1', 'opsStartService', { equipmentReady: true, ratesVerified: true, rateUpdated: (await w.model()).updated });
  await st('no', 'FR2', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 100 }, 'a Front who does not hold the drawer');
  await st('no', 'FR1', 'opsBeginTransaction', { type: 'SWAP', currency: 'USD', amount: 100 });
  await st('no', 'FR1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 1.005 });
  const draft = (await st('ok', 'FR1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 100 })).id;
  // a save while custody is on
  const c1 = (await st('ok', 'FR1', 'reserveCustomer', w.customerPayload(1))).customerId;
  const passport = await w.upload('FR1', 'staff');
  const tx = w.txPayload(1, c1, { type: 'BUY', currency: 'USD', amount: 100, rate: 32.1 }, { passport });
  const passport2 = await w.upload('FR2', 'staff');
  await st('no', 'FR2', 'saveTransaction', { ...w.txPayload(1, c1, { type: 'BUY', currency: 'USD', amount: 100, rate: 32.1 }, { passport: passport2 }) }, 'a Front who does not hold the drawer');
  await st('ok', 'FR1', 'saveTransaction', { ...tx, staffDraftId: draft });
  await st('ok', 'FR1', 'saveTransaction', { ...tx, staffDraftId: draft }, 'the same submission again');
  // opsHold, opsRelease
  await st('no', 'BE1', 'opsHold', { customerId: c1, reason: 'x' }, 'not Front or Monitor');
  await st('no', 'FR1', 'opsHold', { customerId: 'CX-99999999', reason: 'x' }, 'no such customer');
  await st('no', 'FR1', 'opsHold', { customerId: c1 }, 'no reason');
  await st('ok', 'FR1', 'opsHold', { customerId: c1, reason: 'passport looks altered' });
  const hold = (await w.model()).operations.holds[0].id;
  await st('no', 'FR1', 'opsRelease', { id: hold, note: 'fine' }, 'not Monitor');
  await st('no', 'MO1', 'opsRelease', { id: hold }, 'no note');
  await st('ok', 'MO1', 'opsRelease', { id: hold, note: 'checked, fine' });
  await st('no', 'MO1', 'opsRelease', { id: hold, note: 'again' }, 'already released');
  // opsDispatch
  id = await propose(w, 'ok', 'FR1', 'SAFE_OUT', { receiverId: w.id('FI1'), currency: 'USD', amount: 500, note: 'top up' });
  await st('no', 'FI1', 'opsSign', { id }, 'not dispatched yet');
  await st('no', 'FR1', 'opsDispatch', { id }, 'not Backend');
  await st('ok', 'BE1', 'opsDispatch', { id });
  await st('ok', 'BE1', 'opsDispatch', { id }, 'again');
  await st('no', 'MO1', 'opsReject', { id, reason: 'x' }, 'money in transit cannot be sent back');
  await st('no', 'BE1', 'opsCommit', { id }, 'Finance has not confirmed');
  await st('ok', 'FI1', 'opsSign', { id });
  await st('ok', 'BE1', 'opsCommit', { id });
  await st('no', 'FR1', 'opsClock', { direction: 'OUT' }, 'whoever holds the drawer cannot clock out');
  // opsCutoff, the close, opsReconcile
  await st('no', 'FR1', 'opsCutoff', {}, 'not Backend');
  await st('ok', 'BE1', 'opsCutoff', {});
  await st('no', 'FR1', 'opsBeginTransaction', { type: 'BUY', currency: 'USD', amount: 1 }, 'after the cut-off');
  const m = await w.model();
  id = await propose(w, 'ok', 'FR1', 'CLOSE', { ...count(nonZero(expectedOf(m))), ...safeCount(nonZero(m.operations.safe)) });
  await st('no', 'BE1', 'opsCutoff', {}, 'the count has begun');
  await st('ok', 'MG1', 'opsSign', { id });
  await st('ok', 'MO1', 'opsSign', { id });
  await st('no', 'FI1', 'opsReconcile', { id, note: 'x' }, 'not finished yet');
  await st('ok', 'BE1', 'opsCommit', { id });
  await st('no', 'MO1', 'opsReconcile', { id, note: 'x' }, 'not Finance');
  await st('no', 'FI1', 'opsReconcile', { id }, 'no note');
  await st('ok', 'FI1', 'opsReconcile', { id, note: 'counted again, agreed' });
  await st('ok', 'FR1', 'opsClock', { direction: 'OUT' });
  await w.views('after the day of this file', ['owner', 'FR1']);
  return w;
}

// ════ run ═══════════════════════════════════════════════════════════════════
const SCENARIOS = [empty, rates, shift, transactions, custody, branches, transport, engineActions, opsActions];
async function main() {
  await load();
  const worlds = [];
  try {
    for (const scenario of SCENARIOS) { const w = await scenario(); checkViews(w); worlds.push(w); }
  } finally {
    await sql('DROP SCHEMA IF EXISTS ' + SCHEMA + ' CASCADE').catch(() => {});
    await v2.pool.end();
    fs.rmSync(work, { recursive: true, force: true });
  }
  if (process.env.XC_COMPAT_DISCOVER) { for (const s of [...seenRefusals.keys()].sort()) console.log('  ' + JSON.stringify(s) + ','); return; }
  for (const s of REFUSALS) if (!seenRefusals.has(s)) throw Error('a sentence this script expects was never said: ' + s);
  for (const [name, n] of Object.entries(claims)) if (!n) throw Error('claim never checked: ' + name);

  // the index: per action and route, who was let through and every refusal seen
  const RPC_ROUTES = ['/api/rpc', '/api/staff/rpc', '/api/employees'];
  const index = {};
  for (const w of worlds) for (const s of w.steps) {
    if (!s.action || !RPC_ROUTES.includes(s.route)) continue;
    const role = w.people.find((p) => p.key === s.as)?.role ?? 'NOBODY';
    const cell = ((index[s.action] ??= {})[s.route] ??= { accepted: new Set(), refusals: new Map() });
    if (s.status < 400) cell.accepted.add(role);
    else { const key = s.status + '\t' + (s.body?.error ?? s.text); (cell.refusals.get(key) ?? cell.refusals.set(key, { roles: new Set(), first: w.file + '#' + s.n }).get(key)).roles.add(role); }
  }
  const order = ['OWNER', ...ROLES, 'NOBODY'], sorted = (set) => order.filter((r) => set.has(r));
  const indexLines = Object.keys(index).sort().map((action) => '  ' + JSON.stringify(action) + ': {' + Object.keys(index[action]).sort().map((route) => {
    const c = index[action][route];
    return JSON.stringify(route) + ': ' + JSON.stringify({ acceptedFrom: sorted(c.accepted), refusals: [...c.refusals].map(([k, v]) => ({ status: Number(k.split('\t')[0]), error: k.slice(k.indexOf('\t') + 1), from: sorted(v.roles), first: v.first })) });
  }).join(',\n    ') + '}');
  const sentences = [...seenRefusals].sort(([a], [b]) => (a < b ? -1 : 1)).map(([s, n]) => '  ' + JSON.stringify([s, n]));
  const indexText = '{\n "source": "the eight files beside this one and in ../snapshot, read by api/tools/v2-fixtures/gen_compat.mjs",\n'
    + ' "about": "Per action and route: the roles a recorded step was accepted from, and each distinct refusal with the roles it was said to and the first step that shows it. It lists what was recorded, not everything v2 can say: who may call what is #65\'s fixture, the custody rules are #72\'s.",\n'
    + ' "claims": ' + JSON.stringify(claims) + ',\n'
    + ' "actions": {\n' + indexLines.join(',\n') + '\n },\n'
    + ' "sentences": [\n' + sentences.join(',\n') + '\n ]\n}\n';

  for (const dir of ['snapshot', 'rpc']) fs.mkdirSync(path.join(outDir, dir), { recursive: true });
  const report = [];
  for (const w of worlds) { const text = fileText(w); fs.writeFileSync(path.join(outDir, w.file), text); report.push([w.file, w.steps.length, Buffer.byteLength(text)]); }
  fs.writeFileSync(path.join(outDir, 'rpc/index.json'), indexText);
  report.push(['rpc/index.json', Object.keys(index).length, Buffer.byteLength(indexText)]);
  for (const [file, n, bytes] of report) console.error(file.padEnd(32) + String(n).padStart(5) + (file.endsWith('index.json') ? ' actions' : ' steps  ') + String(bytes).padStart(9) + ' bytes');
  console.error('total'.padEnd(32) + String(report.reduce((a, r) => a + r[2], 0)).padStart(22) + ' bytes; claims ' + JSON.stringify(claims));
}
await main();
