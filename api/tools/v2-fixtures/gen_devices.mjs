// Drives the devices module of the v2 server and records what it answered and stored.
//   node gen_devices.mjs path/to/server.mjs > ../../tests/fixtures/devices/v2_device_cases.json
//
// It needs the deployed bundle (`server.mjs`, `xchange-online@3ffc5a5`, 8301 lines). The bundle is
// not in this repository, so this cannot run in CI; its output is checked in and is what the
// tests read.
//
// How v2 is run. The text from `var DEVICES = process.env.DEVICES_ENABLED === "1";` up to, not
// including, `var KINDS = {` (server.mjs:7562-7766: the role lists, `action`, `api`, the receipt
// image archive, `deviceAfterSave`, the two hooks) is evaluated unchanged. Every device action
// below is an HTTP request through v2's own `api(request, actor)`; the lines written after a save
// come from v2's own `deviceAfterSave`, the line written after an upload from v2's own
// `hookUpload`. What is supplied from here, and nothing else:
//   Date          → a clock this script sets (v2 reads `new Date()`)
//   randomUUID8   → a counter
//   createHash6   → node's own
//   pool          → an in-memory store that answers the module's eleven statements, each one known
//                   by its whole shape (below). It is not PostgreSQL; it throws on any other statement.
//                   BEGIN / COMMIT / ROLLBACK keep and restore a copy, so a failed transaction
//                   leaves nothing.
//   execute       → answers `adminSnapshot` with the branch this script describes (its
//                   transactions), and for a branch this script does not know with the sentence
//                   v2's engine has for it (recorded for ticket T5a, `v2_rates_cases.json`)
//   env, console  → never reached; they throw
// Nothing inside the module is replaced.
//
// The script fails, and writes nothing, when v2 answers with a sentence that is not in the list
// below, when a sentence of the list was never heard, or when a statement was never run.
import fs from 'node:fs';
import crypto from 'node:crypto';

const file = process.argv[2];
if (!file) throw Error('usage: node gen_devices.mjs path/to/server.mjs');
const src = fs.readFileSync(file, 'utf8');
const FROM = 'var DEVICES = process.env.DEVICES_ENABLED === "1";', TO = 'var KINDS = {';
const a = src.indexOf(FROM), b = src.indexOf(TO, a);
if (a < 0 || b < 0) throw Error('the devices module was not found in ' + file);
const body = src.slice(a, b);
for (const needle of ['var PERMITS = {', 'async function action(name, p, actor) {', 'async function api(req, actor) {', 'async function deviceAfterSave(', 'async function hookRpc(', 'async function hookUpload(']) if (!body.includes(needle)) throw Error('devices module: ' + needle + ' not found');
if (/^import /m.test(body) || /setTimeout|Math\.random|fetch\(|Date\.now/.test(body)) throw Error('the devices module reads something this script does not supply');

// ── the store ────────────────────────────────────────────────────────────────
const JOB = ['id', 'request_id', 'request_hash', 'branch_id', 'ref', 'kind', 'status', 'actor', 'created', 'updated'];
const text = (v) => v === null || v === undefined ? null : String(v);
const fresh = () => ({ jobs: [], events: [], settings: [], seq: 0 });
const W = { t: fresh(), saved: null, clock: 0, ids: 0, drawn: [], branches: {}, statements: new Set() };
const copy = (r) => ({ ...r });
const STATEMENTS = {
  writeLine: [/^INSERT INTO device_events\(.*ON CONFLICT \(dedupe\) DO NOTHING RETURNING seq$/, ([dedupe, branch_id, actor, kind, ref, detail, at]) => {
    // a NULL never conflicts with another NULL
    if (dedupe != null && W.t.events.some((e) => e.dedupe === dedupe)) return [];
    const seq = ++W.t.seq;
    W.t.events.push({ seq, dedupe: dedupe ?? null, branch_id, actor, kind, ref, detail, at });
    return [{ seq: String(seq) }];
  }],
  insertJob: [/^INSERT INTO device_jobs VALUES\(.*'QUEUED'.*ON CONFLICT \(request_id\) DO NOTHING RETURNING \*$/, ([id, request_id, request_hash, branch_id, ref, kind, actor, at]) => {
    if (W.t.jobs.some((j) => j.request_id === request_id)) return [];
    if (W.t.jobs.some((j) => j.id === id)) throw Error('store: duplicate job id');
    const row = { id, request_id, request_hash, branch_id, ref, kind, status: 'QUEUED', actor, created: at, updated: at };
    W.t.jobs.push(row);
    return [copy(row)];
  }],
  printerSwitch: [/^SELECT value FROM device_settings WHERE branch_id=\$1 AND key='printerOnline'$/, ([branch]) => W.t.settings.filter((s) => s.branch_id === branch && s.key === 'printerOnline').map((s) => ({ value: s.value }))],
  newestJobs: [/^SELECT \* FROM device_jobs WHERE branch_id=\$1 ORDER BY created DESC LIMIT 100$/, ([branch]) => W.t.jobs.filter((j) => j.branch_id === branch).sort((x, y) => (x.created < y.created ? 1 : x.created > y.created ? -1 : 0)).slice(0, 100).map(copy)],
  newestLines: [/^SELECT \* FROM device_events WHERE branch_id=\$1 ORDER BY seq DESC LIMIT 200$/, ([branch]) => W.t.events.filter((e) => e.branch_id === branch).sort((x, y) => y.seq - x.seq).slice(0, 200).map((e) => ({ ...e, seq: String(e.seq) }))],
  jobOfRequest: [/^SELECT \* FROM device_jobs WHERE request_id=\$1$/, ([request]) => W.t.jobs.filter((j) => j.request_id === request).map(copy)],
  jobForProcess: [/^SELECT \* FROM device_jobs WHERE id=\$1 AND branch_id=\$2 FOR UPDATE$/, ([id, branch]) => W.t.jobs.filter((j) => j.id === text(id) && j.branch_id === branch).map(copy)],
  setStatus: [/^UPDATE device_jobs SET status=\$1,updated=\$2 WHERE id=\$3 RETURNING \*$/, ([status, at, id]) => W.t.jobs.filter((j) => j.id === id).map((j) => { j.status = status; j.updated = at; return copy(j); })],
  failedJob: [/^SELECT \* FROM device_jobs WHERE id=\$1 AND branch_id=\$2 AND status='FAILED' FOR UPDATE$/, ([id, branch]) => W.t.jobs.filter((j) => j.id === text(id) && j.branch_id === branch && j.status === 'FAILED').map(copy)],
  queueAgain: [/^UPDATE device_jobs SET status='QUEUED',updated=\$1 WHERE id=\$2$/, ([at, id]) => { for (const j of W.t.jobs) if (j.id === id) { j.status = 'QUEUED'; j.updated = at; } return []; }],
  setSwitch: [/^INSERT INTO device_settings VALUES\(\$1,'printerOnline',\$2\) ON CONFLICT \(branch_id,key\) DO UPDATE SET value=excluded\.value$/, ([branch, value]) => {
    const old = W.t.settings.find((s) => s.branch_id === branch && s.key === 'printerOnline');
    if (old) old.value = value; else W.t.settings.push({ branch_id: branch, key: 'printerOnline', value });
    return [];
  }],
};
function query(sql, args = []) {
  if (sql === 'BEGIN') { if (W.saved) throw Error('store: nested transaction'); W.saved = structuredClone(W.t); return { rows: [] }; }
  if (sql === 'COMMIT') { W.saved = null; return { rows: [] }; }
  if (sql === 'ROLLBACK') { if (W.saved) W.t = W.saved; W.saved = null; return { rows: [] }; }
  const known = Object.entries(STATEMENTS).filter(([, [shape]]) => shape.test(sql));
  if (known.length !== 1) throw Error('store: unknown statement: ' + sql);
  const [name, [, run]] = known[0];
  if (args.some((x) => x === undefined)) throw Error('store: undefined parameter in: ' + sql);
  W.statements.add(name);
  const rows = run(args);
  return { rows, rowCount: rows.length };
}

// ── v2 with its outside names supplied ──────────────────────────────────────
class Clock extends Date { constructor(...x) { if (x.length) super(...x); else super(W.clock); } }
const never = (what) => () => { throw Error('fixture: v2 reached ' + what); };
const NO_SUCH_BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
const scope = {
  process: { env: { DEVICES_ENABLED: '1' } },
  pool: { query: async (sql, args) => query(sql, args), connect: async () => ({ query: async (sql, args) => query(sql, args), release() {} }) },
  execute: async (name, payload) => {
    if (name !== 'adminSnapshot') throw Error('fixture: unexpected engine call ' + name);
    const branch = W.branches[payload.branchId];
    if (!branch) throw Error(NO_SUCH_BRANCH);
    return structuredClone({ branches: Object.entries(W.branches).map(([id, x]) => ({ id, name: x.name })), transactions: branch.transactions });
  },
  env: { BUCKET: { get: never('the file bucket'), put: never('the file bucket') } },
  console: { error: never('console.error'), log: never('console.log') },
  createHash6: crypto.createHash,
  randomUUID8: () => { const id = idAt(++W.ids); W.drawn.push(id); return id; },
  Date: Clock,
};
for (const n of Object.keys(scope)) if (new RegExp('^(?:var|async function|function) ' + n + '\\b', 'm').test(body)) throw Error(n + ' is declared by v2 and must not be replaced');
const v2 = new Function(...Object.keys(scope), body + '\nreturn { api, deviceAfterSave, hookUpload, hookRpc, PERMITS };')(...Object.values(scope));
// lower-case hex, 36 long, different in every part that could be compared
function idAt(n) { return (0x7e570000 + n).toString(16).padStart(8, '0') + '-de71-4ce5-8000-' + n.toString(16).padStart(12, '0'); }
const requestAt = (n) => 'aaaaaaaa-0000-4000-8000-' + n.toString(16).padStart(12, '0');

// ── sentences v2 may answer with; anything else stops the script ────────────
const SENTENCES = {
  badBranch: 'สาขาไม่ถูกต้อง', noSuchBranch: NO_SUCH_BRANCH,
  badKind: 'ประเภทอุปกรณ์ไม่ถูกต้อง', badRequestId: 'รหัสคำขอไม่ถูกต้อง', printNeedsRef: 'เลือกรายการก่อนพิมพ์',
  notCompletedHere: 'ไม่พบรายการสำเร็จในสาขานี้', reasonTooLong: 'เหตุผลยาวเกินกำหนด', drawerNeedsReason: 'ระบุเหตุผลเปิดลิ้นชัก',
  requestChanged: 'คำขอเดิมมีข้อมูลเปลี่ยน', noSuchJob: 'ไม่พบงาน', retryOnlyFailed: 'ลองใหม่ได้เฉพาะงานที่ล้มเหลว',
  notHere: 'ไม่พบรายการในสาขานี้', badState: 'สถานะไม่ถูกต้อง',
};
const heard = new Set();

// ── people and branches ─────────────────────────────────────────────────────
const OWNER = { kind: 'owner', id: 'owner', email: 'owner', role: 'OWNER' };
const staff = (username, role, branchId) => ({ kind: 'staff', id: 'E-' + username, email: username, role, branchId });
const PEOPLE = { owner: OWNER, front1: staff('front1', 'FRONT', 'MAIN'), front2: staff('front2', 'FRONT', 'MAIN'), backend1: staff('backend1', 'BACKEND', 'MAIN'), front9: staff('front9', 'FRONT', 'B9') };
const TX = (id, status) => ({ id, status });
const BRANCHES = () => ({
  MAIN: { name: 'XC-B', transactions: [TX('XC-20261006-000001', 'COMPLETED'), TX('XC-20261006-000002', 'COMPLETED'), TX('XC-20261006-000003', 'PENDING'), TX('XC-20261006-000004', 'CANCELLED')] },
  B9: { name: 'Nine', transactions: [TX('XC-20261006-000009', 'COMPLETED')] },
});
const T0 = Date.parse('2026-10-06T05:00:00.123Z');
const ORIGIN = 'http://desk.test';

const state = () => ({
  jobs: W.t.jobs.map(({ request_hash, ...j }) => j),
  events: W.t.events.slice().sort((x, y) => x.seq - y.seq).map(({ seq, ...e }) => ({ ...e, detail: JSON.parse(e.detail) })),
  settings: W.t.settings.map(copy),
});
const scenarios = [];
let current = null;
function scenario(name, note) {
  W.t = fresh(); W.saved = null; W.clock = T0; W.ids = 0; W.branches = BRANCHES();
  current = { name, note, exchanges: Object.fromEntries(Object.entries(W.branches).map(([id, x]) => [id, x.transactions])), steps: [] };
  scenarios.push(current);
}
/** One call. `who` is a name of PEOPLE; the branch is the one v2 itself works on for that caller. */
async function step(who, call, args, { branch, note, v3 } = {}) {
  const actor = PEOPLE[who]; if (!actor) throw Error('fixture: nobody called ' + who);
  W.clock += 1000; W.drawn = [];
  const before = JSON.stringify(state());
  const worksOn = actor.kind === 'staff' ? actor.branchId : branch || 'MAIN';
  const rec = { call, actor: actor.email, branch: worksOn, now: new Date(W.clock).toISOString(), args };
  if (note) rec.note = note;
  if (v3) rec.v3 = v3;
  if (call === 'afterSave') {
    const p = { method: args.cash ? 'Cash' : 'Bank Transfer', submissionId: args.submissionId, currency: args.currency, amount: Number(args.amount), rate: Number(args.rate), passport: args.passport ? { uploadKey: args.passport } : undefined, slip: args.slip ? { uploadKey: args.slip } : undefined };
    await v2.deviceAfterSave(p, { documentRef: args.documentRef }, worksOn, actor);
    rec.result = null;
  } else if (call === 'scanImported') {
    const req = new Request(ORIGIN + '/api/upload', { method: 'POST', headers: { 'content-type': args.mime }, body: 'x' });
    const res = await v2.hookUpload(req, async () => Response.json({ uploadKey: args.uploadKey }), { branch: worksOn, actor });
    if (!res.ok) throw Error('fixture: the upload hook did not pass the answer on');
    rec.result = null;
  } else {
    const name = { queue: 'queue', process: 'process', retry: 'retry', printerOnline: 'printerOnline', drawerObservation: 'drawerObservation', overview: 'snapshot' }[call];
    if (!name) throw Error('fixture: no call ' + call);
    const payload = { ...args }; if (branch) payload.branchId = branch;
    const res = await v2.api(new Request(ORIGIN + '/api/devices', { method: 'POST', headers: { origin: ORIGIN, 'x-xchange-request': '1', 'content-type': 'application/json' }, body: JSON.stringify({ action: name, payload }) }), actor);
    if (res.headers.get('cache-control') !== 'no-store') throw Error('fixture: an answer without Cache-Control: no-store');
    const answer = await res.json();
    if (res.status === 400) {
      const key = Object.keys(SENTENCES).find((k) => SENTENCES[k] === answer.error);
      if (!key) throw Error(`${call} as ${who}: unexpected sentence ${JSON.stringify(answer.error)}`);
      heard.add(key); rec.error = answer.error;
      if (JSON.stringify(state()) !== before) throw Error(`${call} as ${who}: v2 refused and still stored something`);
    } else if (res.status !== 200) throw Error(`${call}: status ${res.status}`);
    else if (call === 'overview') {
      if (answer.mode !== 'SIMULATION' || answer.drawerPhysicalDetection !== false) throw Error('fixture: the snapshot is not the simulated one');
      rec.result = { jobs: answer.jobs.map((j) => j.id), events: answer.events.map((e) => ({ kind: e.kind, ref: e.ref, at: e.at })), printerOnline: answer.printerOnline };
    } else if (call === 'queue' || call === 'process') { const { request_hash, ...job } = answer; rec.result = job; }
    else { if (JSON.stringify(answer) !== '{"saved":true}') throw Error(`${call}: unexpected answer ${JSON.stringify(answer)}`); rec.result = null; }
  }
  rec.ids = W.drawn.slice();
  // what the call stored: the lines it added, and every job and switch that is new or different
  const was = JSON.parse(before), after = state(), same = (x, y) => JSON.stringify(x) === JSON.stringify(y);
  if (!same(after.events.slice(0, was.events.length), was.events)) throw Error('fixture: v2 changed a line already written');
  if (was.jobs.some((j) => !after.jobs.some((k) => k.id === j.id))) throw Error('fixture: v2 removed a job');
  rec.stored = {
    events: after.events.slice(was.events.length),
    jobs: after.jobs.filter((j) => !was.jobs.some((k) => same(j, k))),
    settings: after.settings.filter((x) => !was.settings.some((k) => same(x, k))),
  };
  rec.changed = !same(after, was);
  if (call === 'afterSave' && args.cash) {
    // v2 multiplies two doubles; v3 multiplies decimals. Both are recorded.
    const line = rec.stored.events.find((e) => e.kind === 'COUNT_CONFIRMED');
    if (line) { line.detail.thbExact = exactProduct(args.amount, args.rate); if (Math.abs(line.detail.thb - Number(line.detail.thbExact)) > 1e-6) throw Error('fixture: the product'); }
  }
  current.steps.push(rec);
  return rec;
}
/** The product of two plain decimals, exact, without trailing zeros. */
function exactProduct(x, y) {
  const parts = (t) => { const [i, f = ''] = String(t).split('.'); return [BigInt(i + f), f.length]; };
  const [p, dp] = parts(x), [r, dr] = parts(y);
  let digits = (p * r).toString().padStart(dp + dr + 1, '0');
  if (dp + dr === 0) return digits;
  return (digits.slice(0, -(dp + dr)) + '.' + digits.slice(-(dp + dr))).replace(/0+$/, '').replace(/\.$/, '');
}
const REF1 = 'XC-20261006-000001', REF2 = 'XC-20261006-000002', PENDING = 'XC-20261006-000003', CANCELLED = 'XC-20261006-000004', REF9 = 'XC-20261006-000009';
const expectError = (rec, key) => { if (rec.error !== SENTENCES[key]) throw Error(`fixture: expected ${key}, v2 said ${JSON.stringify(rec.error ?? rec.result)}`); };
const expectOk = (rec) => { if (rec.error) throw Error('fixture: expected success, v2 said ' + rec.error); return rec; };

// ── A. queue: the checks and their order ─────────────────────────────────────
scenario('queue_checks', 'Every refusal of queue, alone and two at a time, to pin the order of the checks.');
let n = 0;
const q = (over) => ({ kind: 'PRINT_RECEIPT', ref: REF1, requestId: requestAt(++n), reason: '', ...over });
expectError(await step('owner', 'queue', q({ kind: 'SCAN' })), 'badKind');
expectError(await step('owner', 'queue', q({ kind: 'print_receipt' })), 'badKind');
expectError(await step('owner', 'queue', q({ kind: '' })), 'badKind');
expectError(await step('owner', 'queue', q({ kind: 'NOPE', requestId: 'x' }), { note: 'kind before request id' }), 'badKind');
expectError(await step('owner', 'queue', q({ requestId: '' })), 'badRequestId');
expectError(await step('owner', 'queue', q({ requestId: 'AAAAAAAA-0000-4000-8000-000000000001' }), { note: 'capitals' }), 'badRequestId');
expectError(await step('owner', 'queue', q({ requestId: 'aaaaaaaa-0000-4000-8000-00000000001' }), { note: '35 characters' }), 'badRequestId');
expectError(await step('owner', 'queue', q({ requestId: 'aaaaaaaa-0000-4000-8000-0000000000012' }), { note: '37 characters' }), 'badRequestId');
expectError(await step('owner', 'queue', q({ requestId: 'gaaaaaaa-0000-4000-8000-000000000001' }), { note: 'a letter after f' }), 'badRequestId');
expectError(await step('owner', 'queue', q({ requestId: 'aaaaaaaa-0000-4000-8000-00000000000\n' }), { note: 'a line feed as the 36th character' }), 'badRequestId');
expectError(await step('owner', 'queue', q({ requestId: 'x', ref: '' }), { note: 'request id before the missing ref' }), 'badRequestId');
expectError(await step('owner', 'queue', q({ ref: '' })), 'printNeedsRef');
expectError(await step('owner', 'queue', q({ ref: '', reason: 'x'.repeat(201) }), { note: 'missing ref before the long reason' }), 'printNeedsRef');
expectError(await step('owner', 'queue', q({ ref: 'XC-20261006-999999' })), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ ref: PENDING }), { note: 'an exchange that is not completed' }), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ ref: CANCELLED })), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ ref: REF9 }), { note: 'completed, in another branch' }), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ ref: 'xc-20261006-000001' }), { note: 'the reference in lower case' }), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ ref: REF1 + ' ' }), { note: 'a blank after the reference' }), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: PENDING, reason: 'x' }), { note: 'a drawer request names an exchange only if it is completed' }), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ ref: PENDING, reason: 'x'.repeat(201) }), { note: 'the exchange before the long reason' }), 'notCompletedHere');
expectError(await step('owner', 'queue', q({ reason: 'x'.repeat(201) })), 'reasonTooLong');
expectError(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: '', reason: ' '.repeat(201) }), { note: 'length before blankness' }), 'reasonTooLong');
expectError(await step('owner', 'queue', q({ reason: '😀'.repeat(101) }), { note: '101 characters that JavaScript counts as 202' }), 'reasonTooLong');
expectError(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: '', reason: '' })), 'drawerNeedsReason');
expectError(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: '', reason: ' \t\n ' })), 'drawerNeedsReason');
expectError(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: REF1, reason: '﻿ 　' }), { note: 'a byte-order mark, a no-break space and an ideographic space are blank to JavaScript' }), 'drawerNeedsReason');
expectOk(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: '', reason: '\u0085' }), { note: 'U+0085 is not blank to JavaScript' }));
expectOk(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: '', reason: '​' }), { note: 'a zero-width space is not blank' }));
expectOk(await step('owner', 'queue', q({ reason: 'x'.repeat(200) }), { note: 'exactly 200' }));
expectOk(await step('owner', 'queue', q({ reason: 'ก'.repeat(200) }), { note: '200 Thai letters' }));
expectOk(await step('owner', 'queue', q({ reason: '😀'.repeat(100) }), { note: '100 characters that JavaScript counts as 200' }));
expectOk(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: '', reason: ' change for a customer ' }), { note: 'the reason is kept as typed' }));
expectOk(await step('owner', 'queue', q({ kind: 'DRAWER_REQUEST', ref: REF2, reason: 'recount' })));
expectOk(await step('owner', 'queue', q({ requestId: '-'.repeat(36) }), { note: '36 dashes pass the request-id rule' }));
expectOk(await step('front1', 'queue', q({ ref: REF2 }), { note: 'an employee: the job carries the user name' }));
expectOk(await step('front9', 'queue', q({ ref: REF9 }), { note: 'an employee of another branch works on that branch' }));
expectError(await step('front9', 'queue', q({ ref: REF1 }), { note: 'an exchange of MAIN, asked from B9' }), 'notCompletedHere');

// ── B. queue: the same request again ─────────────────────────────────────────
scenario('queue_again', 'A request id seen before: the same request returns the job, a changed one is refused.');
const R1 = requestAt(1001), R2 = requestAt(1002);
const first = expectOk(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: R1, reason: 'reprint' }));
const again = expectOk(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: R1, reason: 'reprint' }, { note: 'the same request: the same job, nothing stored, no id used up beyond the one drawn' }));
if (again.result.id !== first.result.id || again.changed) throw Error('fixture: the repeat was not a repeat');
expectError(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: R1, reason: 'reprint ' }, { note: 'a blank more in the reason' }), 'requestChanged');
expectError(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF2, requestId: R1, reason: 'reprint' }, { note: 'another exchange' }), 'requestChanged');
expectError(await step('owner', 'queue', { kind: 'DRAWER_REQUEST', ref: REF1, requestId: R1, reason: 'reprint' }, { note: 'another kind' }), 'requestChanged');
expectError(await step('front1', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: R1, reason: 'reprint' }, { note: 'another caller' }), 'requestChanged');
expectError(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF9, requestId: R1, reason: 'reprint' }, { branch: 'B9', v3: 'differs', note: 'v2 keeps request ids for the whole system; v3 keeps them per branch, so there this is a new job' }), 'requestChanged');
expectError(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: 'XC-20261006-999999', requestId: R1, reason: 'reprint' }, { note: 'the checks come before the look-up of the earlier request' }), 'notCompletedHere');
expectOk(await step('owner', 'process', { id: first.result.id }));
const later = expectOk(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: R1, reason: 'reprint' }, { note: 'after the job was processed: the job as it is now' }));
if (later.result.status !== 'SIMULATED_PRINTED' || later.changed) throw Error('fixture: the repeat after processing');
expectOk(await step('owner', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: R2, reason: 'reprint' }, { note: 'the same content under a new request id is a second job' }));

// ── C. process and retry ─────────────────────────────────────────────────────
scenario('process_and_retry', 'A queued job is processed once; a failed one can be queued again.');
const print = expectOk(await step('front1', 'queue', { kind: 'PRINT_RECEIPT', ref: REF1, requestId: requestAt(2001), reason: '' })).result.id;
const drawer = expectOk(await step('front1', 'queue', { kind: 'DRAWER_REQUEST', ref: '', requestId: requestAt(2002), reason: 'change' })).result.id;
const other = expectOk(await step('front9', 'queue', { kind: 'PRINT_RECEIPT', ref: REF9, requestId: requestAt(2003), reason: '' })).result.id;
expectError(await step('front1', 'process', { id: idAt(999) }), 'noSuchJob');
expectError(await step('front1', 'process', { id: '' }), 'noSuchJob');
expectError(await step('front1', 'process', { id: other }, { note: 'a job of another branch' }), 'noSuchJob');
expectError(await step('front1', 'process', { id: print.toUpperCase() }), 'noSuchJob');
expectError(await step('front1', 'retry', { id: print }, { note: 'a queued job is not retried' }), 'retryOnlyFailed');
expectError(await step('front1', 'retry', { id: idAt(999) }, { note: 'no such job: the same sentence' }), 'retryOnlyFailed');
expectOk(await step('backend1', 'process', { id: print }, { note: 'processed by somebody else than queued it: the job keeps its first actor, the event has the second' }));
const twice = expectOk(await step('front1', 'process', { id: print }, { note: 'again: the job as it is, nothing stored' }));
if (twice.changed) throw Error('fixture: processing twice stored something');
expectError(await step('front1', 'retry', { id: print }, { note: 'a printed job is not retried' }), 'retryOnlyFailed');
expectOk(await step('owner', 'process', { id: drawer }));
expectOk(await step('backend1', 'printerOnline', { online: false }));
const p2 = expectOk(await step('front1', 'queue', { kind: 'PRINT_RECEIPT', ref: REF2, requestId: requestAt(2004), reason: '' })).result.id;
const d2 = expectOk(await step('front1', 'queue', { kind: 'DRAWER_REQUEST', ref: '', requestId: requestAt(2005), reason: 'change' })).result.id;
if (expectOk(await step('front1', 'process', { id: p2 }, { note: 'the printer is offline' })).result.status !== 'FAILED') throw Error('fixture: offline print');
if (expectOk(await step('front1', 'process', { id: d2 }, { note: 'the printer is offline: a drawer request fails too' })).result.status !== 'FAILED') throw Error('fixture: offline drawer');
if (expectOk(await step('front9', 'process', { id: other }, { note: 'the printer of B9 was never switched off' })).result.status !== 'SIMULATED_PRINTED') throw Error('fixture: the other branch');
expectError(await step('front9', 'retry', { id: p2 }, { note: 'a failed job of another branch' }), 'retryOnlyFailed');
expectOk(await step('front1', 'process', { id: p2 }, { note: 'a failed job is not processed again' }));
expectOk(await step('front1', 'retry', { id: p2 }));
expectError(await step('front1', 'retry', { id: p2 }, { note: 'queued again: not failed any more' }), 'retryOnlyFailed');
if (expectOk(await step('front1', 'process', { id: p2 }, { note: 'still offline: fails again' })).result.status !== 'FAILED') throw Error('fixture: still offline');
expectOk(await step('front1', 'retry', { id: p2 }));
expectOk(await step('backend1', 'printerOnline', { online: true }));
if (expectOk(await step('front1', 'process', { id: p2 })).result.status !== 'SIMULATED_PRINTED') throw Error('fixture: back online');
expectOk(await step('front1', 'retry', { id: d2 }));
if (expectOk(await step('front1', 'process', { id: d2 })).result.status !== 'COMMAND_SIMULATED') throw Error('fixture: drawer back online');

// ── D. the printer switch ────────────────────────────────────────────────────
scenario('printer_switch', 'One switch per branch; every use of it is written down, changed or not.');
if (expectOk(await step('owner', 'overview', {})).result.printerOnline !== true) throw Error('fixture: a printer nobody touched is online');
expectOk(await step('owner', 'printerOnline', { online: false }));
expectOk(await step('owner', 'printerOnline', { online: false }, { note: 'the same value again: a second line' }));
if (expectOk(await step('owner', 'overview', {})).result.printerOnline !== false) throw Error('fixture: offline');
if (expectOk(await step('owner', 'overview', {}, { branch: 'B9' })).result.printerOnline !== true) throw Error('fixture: the other branch');
expectOk(await step('owner', 'printerOnline', { online: false }, { branch: 'B9' }));
expectOk(await step('backend1', 'printerOnline', { online: true }));
if (expectOk(await step('owner', 'overview', {})).result.printerOnline !== true) throw Error('fixture: online again');
if (expectOk(await step('owner', 'overview', {}, { branch: 'B9' })).result.printerOnline !== false) throw Error('fixture: the other branch stays off');

// ── E. what an employee saw of the drawer ────────────────────────────────────
scenario('drawer_observation', 'An observation names an exchange of the branch, in any state, or none.');
expectOk(await step('front1', 'drawerObservation', { ref: '', state: 'OPEN' }));
expectOk(await step('front1', 'drawerObservation', { ref: REF1, state: 'CLOSED' }));
expectOk(await step('owner', 'drawerObservation', { ref: PENDING, state: 'OPEN' }, { note: 'an exchange that is not completed is accepted here' }));
expectOk(await step('owner', 'drawerObservation', { ref: CANCELLED, state: 'CLOSED' }));
expectError(await step('front1', 'drawerObservation', { ref: REF9, state: 'OPEN' }, { note: 'an exchange of another branch' }), 'notHere');
expectError(await step('front1', 'drawerObservation', { ref: 'nope', state: 'nope' }, { note: 'the exchange before the state' }), 'notHere');
expectError(await step('front1', 'drawerObservation', { ref: '', state: 'open' }), 'badState');
expectError(await step('front1', 'drawerObservation', { ref: REF1, state: '' }), 'badState');
expectError(await step('front1', 'drawerObservation', { ref: REF1, state: 'OPEN ' }), 'badState');

// ── F. the lines a saved exchange leaves ─────────────────────────────────────
scenario('after_save', 'What is written when an exchange has been saved, and that writing it twice adds nothing.');
const save = (over) => ({ cash: true, submissionId: 'sub-0001-aaaaaaaaaaaa', documentRef: REF1, currency: 'USD', amount: '250', rate: '32.5', passport: '11111111-1111-4111-8111-111111111111', slip: null, ...over });
expectOk(await step('front1', 'afterSave', save({}), { note: 'cash: the count line, the link line, a print job and its line' }));
if ((await step('front1', 'afterSave', save({}), { note: 'the same save again: nothing' })).changed) throw Error('fixture: a second after-save stored something');
expectOk(await step('front1', 'afterSave', save({ cash: false, submissionId: 'sub-0002-aaaaaaaaaaaa', documentRef: REF2, currency: 'JPY', amount: '10000', rate: '0.2195', slip: '22222222-2222-4222-8222-222222222222' }), { note: 'not cash: no count line' }));
expectOk(await step('front9', 'afterSave', save({ submissionId: 'sub-0001-aaaaaaaaaaaa', documentRef: REF9, passport: null, amount: '100', rate: '32.1' }), { note: 'the same submission id in another branch is another save' }));
expectOk(await step('front1', 'afterSave', save({ submissionId: 'sub-0003-aaaaaaaaaaaa', documentRef: REF1 }), { note: 'a document already linked: only the count line of the new submission' }));
const auto = W.t.jobs.find((j) => j.branch_id === 'MAIN' && j.ref === REF1);
expectOk(await step('front1', 'process', { id: auto.id }, { note: 'the job queued by the save is processed like any other' }));
expectOk(await step('owner', 'overview', {}));

// ── G. an uploaded scan ──────────────────────────────────────────────────────
scenario('scan_imported', 'The line written after an evidence file was uploaded.');
expectOk(await step('front1', 'scanImported', { uploadKey: '33333333-3333-4333-8333-333333333333', mime: 'image/jpeg' }));
expectOk(await step('owner', 'scanImported', { uploadKey: '44444444-4444-4444-8444-444444444444', mime: 'application/pdf' }, { branch: 'B9' }));
expectOk(await step('front1', 'scanImported', { uploadKey: '33333333-3333-4333-8333-333333333333', mime: 'image/jpeg' }, { note: 'the same key again: a second line' }));

// ── H. a branch that is not one ──────────────────────────────────────────────
scenario('branches', 'The branch is judged before anything else about the request.');
for (const [call, args] of [['queue', { kind: 'NOPE', ref: '', requestId: 'x', reason: '' }], ['process', { id: 'x' }], ['retry', { id: 'x' }], ['printerOnline', { online: false }], ['drawerObservation', { ref: 'nope', state: 'nope' }], ['overview', {}]]) {
  expectError(await step('owner', call, args, { branch: 'no such branch' }), 'badBranch');
  expectError(await step('owner', call, args, { branch: 'B'.repeat(71) }), 'badBranch');
  expectError(await step('owner', call, args, { branch: 'B404', note: 'well formed, but no such branch (the sentence is the engine\'s, see the header)' }), 'noSuchBranch');
}

// ── I. the overview ──────────────────────────────────────────────────────────
scenario('overview', 'The newest 100 jobs and the newest 200 events of the branch, newest first.');
for (let i = 0; i < 102; i++) expectOk(await step('owner', 'queue', { kind: 'DRAWER_REQUEST', ref: '', requestId: requestAt(3000 + i), reason: 'r' + i }));
for (let i = 0; i < 100; i++) expectOk(await step('owner', 'drawerObservation', { ref: '', state: i % 2 ? 'OPEN' : 'CLOSED' }));
expectOk(await step('front9', 'queue', { kind: 'PRINT_RECEIPT', ref: REF9, requestId: requestAt(3999), reason: '' }));
const over = expectOk(await step('owner', 'overview', {}));
if (over.result.jobs.length !== 100 || over.result.events.length !== 200) throw Error('fixture: the overview is not capped');
if (expectOk(await step('owner', 'overview', {}, { branch: 'B9' })).result.jobs.length !== 1) throw Error('fixture: the overview of B9');

// ── J. cash is counted before it is saved ────────────────────────────────────
// v2's save hook (`hookRpc`) refuses a cash exchange whose payload does not carry `countConfirmed: true`,
// before the engine is asked. `reached` says whether the request went on to the engine.
const COUNT_FIRST = 'ยืนยันการนับเงินก่อนบันทึกรายการเงินสด';
const countConfirmed = [];
for (const method of ['Cash', 'Bank Transfer', 'PromptPay', 'cash']) for (const confirmed of [true, false, undefined, 'true', 1]) {
  let reached = false;
  const req = new Request(ORIGIN + '/api/rpc', { method: 'POST', body: JSON.stringify({ action: 'saveTransaction', payload: { method, countConfirmed: confirmed } }) });
  const res = await v2.hookRpc(req, async () => { reached = true; return Response.json({ saved: false }); }, { actor: OWNER, branch: () => 'MAIN' });
  const answer = await res.json();
  if (reached ? res.status !== 200 : res.status !== 400 || answer.error !== COUNT_FIRST) throw Error('fixture: the save hook answered ' + res.status + ' ' + JSON.stringify(answer));
  countConfirmed.push({ method, countConfirmed: confirmed === undefined ? null : confirmed, cash: method === 'Cash', confirmed: confirmed === true, refused: !reached });
}
if (!countConfirmed.some((c) => c.refused) || !countConfirmed.some((c) => !c.refused)) throw Error('fixture: the count check never changed its answer');

// ── checks on the recording itself ───────────────────────────────────────────
const unheard = Object.keys(SENTENCES).filter((k) => !heard.has(k));
if (unheard.length) throw Error('never heard: ' + unheard.join(', '));
const unused = Object.keys(STATEMENTS).filter((s) => !W.statements.has(s));
if (unused.length) throw Error('never run: ' + unused.join(' | '));
const all = scenarios.flatMap((s) => s.steps);
const count = (f) => all.filter(f).length;
const head = {
  source: 'xchange-online@3ffc5a5 server.mjs:7562-7766, driven by api/tools/v2-fixtures/gen_devices.mjs',
  counts: { scenarios: scenarios.length, steps: all.length, refused: count((s) => s.error), stored: count((s) => s.changed), v3Differs: count((s) => s.v3) },
  sentences: SENTENCES,
  roles: v2.PERMITS,
  countFirst: COUNT_FIRST,
  countConfirmed,
};
// one step per line, so that a change to the recording reads as changed steps
const lines = scenarios.map((s) => ` {"name":${JSON.stringify(s.name)},"note":${JSON.stringify(s.note)},"exchanges":${JSON.stringify(s.exchanges)},"steps":[\n${s.steps.map((x) => '  ' + JSON.stringify(x)).join(',\n')}\n ]}`);
process.stdout.write(JSON.stringify(head, null, 1).replace(/\n}$/, ',\n "scenarios": [\n') + lines.join(',\n') + '\n ]\n}\n');
