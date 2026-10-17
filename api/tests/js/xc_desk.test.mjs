// The desk script's pure parts (row DK-1, #332; docs/design/xchange-v3.md §5.3
// "The kept desk's dead ends"). Written before the code. Do not edit, delete or weaken.
//
// Run from the repository root:   node --test api/tests/js/*.test.mjs
//
// api/static/xc-desk.js is a classic browser script, like api/static/xc-forms.js:
// in a browser it installs itself on the kept desk page; under Node, where `module`
// exists, it installs nothing and exports the functions below through
// `module.exports`. Each test loads it on its own, so a missing script fails every
// test by name.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('../../static/xc-desk.js', import.meta.url));
const require = createRequire(import.meta.url);
let loaded;
const D = () => (loaded ??= require(SCRIPT));

const CUSTODY_OFF = 'เจ้าของต้องเปิดใช้ขั้นตอนรับ–ส่งเงินก่อน'; // U+2013, CustodyError::NotEnabled
const NO_USD = 'ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน';
const post = (action) => ({
  method: 'POST',
  headers: { 'Content-Type': 'application/json', 'X-Xchange-Request': '1' },
  body: JSON.stringify({ action, payload: { branchId: 'XC-B01' } }),
});
const json = (status, value) => D().readAnswer(status, JSON.stringify(value));

// ------------------------------------------------------------------ sentences

test('the four sentences of the design', () => {
  assert.equal(D().BACK_LABEL, '← กลับไปแก้ไขรายการ');
  assert.equal(
    D().UNKNOWN_NOTE,
    'ยังไม่ทราบผลการบันทึก กด "ลองบันทึกซ้ำ" ได้ ระบบไม่บันทึกรายการเดียวกันซ้ำ',
  );
  assert.equal(D().CUSTOMER_UNREACHED, 'สร้างรหัสลูกค้าไม่ได้ เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่');
  assert.equal(D().STAFF_TITLE, 'รหัสจากบัญชีที่เข้าสู่ระบบ');
});

test('under Node the script installs nothing', () => {
  const fetchBefore = globalThis.fetch;
  D();
  assert.equal(globalThis.fetch, fetchBefore, 'fetch was replaced');
  assert.equal(typeof globalThis.gasCall, 'undefined');
});

// ------------------------------------------------------------------ rpcAction

test('rpcAction names the action of a POST to either RPC route', () => {
  assert.equal(D().rpcAction('/api/rpc', post('saveTransaction')), 'saveTransaction');
  assert.equal(D().rpcAction('/api/staff/rpc', post('saveTransaction')), 'saveTransaction');
  assert.equal(D().rpcAction('/api/staff/rpc', post('reserveCustomer')), 'reserveCustomer');
  assert.equal(D().rpcAction('/api/staff/rpc', post('opsBeginTransaction')), 'opsBeginTransaction');
  assert.equal(D().rpcAction('http://xc.test/api/rpc', post('saveTransaction')), 'saveTransaction');
  assert.equal(D().rpcAction('/api/rpc', { ...post('saveTransaction'), method: 'post' }), 'saveTransaction');
});

test('rpcAction is null for anything else', () => {
  assert.equal(D().rpcAction('/api/rpc', undefined), null, 'no init: a GET');
  assert.equal(D().rpcAction('/api/rpc', { method: 'GET' }), null);
  assert.equal(D().rpcAction('/api/upload?branch=XC-B01', post('saveTransaction')), null);
  assert.equal(D().rpcAction('/api/auth', post('status')), null);
  assert.equal(D().rpcAction('/api/kiosk', post('notifications')), null);
  assert.equal(D().rpcAction('/api/rpcx', post('saveTransaction')), null);
  assert.equal(D().rpcAction('/api/rpc', { method: 'POST', body: 'not json' }), null);
  assert.equal(D().rpcAction('/api/rpc', { method: 'POST', body: new Uint8Array([1, 2]) }), null);
  assert.equal(D().rpcAction('/api/rpc', { method: 'POST', body: '{"payload":{}}' }), null);
  assert.equal(D().rpcAction('/api/rpc', { method: 'POST', body: '{"action":7}' }), null);
});

// ------------------------------------------------------------------ readAnswer

test('readAnswer reads the status, whether the body is a JSON object, and its error', () => {
  assert.deepEqual(json(400, { error: NO_USD }), { status: 400, json: true, error: NO_USD });
  assert.deepEqual(json(200, { saved: true }), { status: 200, json: true, error: null });
  assert.deepEqual(json(400, { error: 5 }), { status: 400, json: true, error: null });
  assert.deepEqual(D().readAnswer(502, '<html>Bad Gateway</html>'), {
    status: 502,
    json: false,
    error: null,
  });
  assert.deepEqual(D().readAnswer(400, '[]'), { status: 400, json: false, error: null });
  assert.deepEqual(D().readAnswer(400, 'null'), { status: 400, json: false, error: null });
  assert.deepEqual(D().readAnswer(400, ''), { status: 400, json: false, error: null });
});

// ------------------------------------------------------------------ saveOutcome

test('a save the page refused before sending is a refusal', () => {
  // desk-devices.js throws `ยังไม่ยืนยันการนับเงิน` before any request; an upload
  // that fails also stops before the RPC.
  assert.equal(D().saveOutcome(null), 'refused');
});

test('a 4xx with a JSON error is a refusal', () => {
  assert.equal(D().saveOutcome(json(400, { error: NO_USD })), 'refused');
  assert.equal(D().saveOutcome(json(400, { error: CUSTODY_OFF })), 'refused');
  assert.equal(D().saveOutcome(json(401, { error: 'EMPLOYEE_AUTH_REQUIRED' })), 'refused');
  assert.equal(D().saveOutcome(json(403, { error: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการ' })), 'refused');
  assert.equal(D().saveOutcome(json(499, { error: 'x' })), 'refused');
});

test('everything else is an unknown outcome', () => {
  assert.equal(D().saveOutcome({ failed: true }), 'unknown', 'the network failed');
  assert.equal(D().saveOutcome(json(500, { error: 'ระบบขัดข้อง' })), 'unknown', 'a 5xx');
  assert.equal(D().saveOutcome(json(503, { error: 'x' })), 'unknown');
  assert.equal(D().saveOutcome(D().readAnswer(502, '<html>')), 'unknown', 'not JSON');
  assert.equal(D().saveOutcome(D().readAnswer(413, 'Payload Too Large')), 'unknown');
  assert.equal(D().saveOutcome(json(400, {})), 'unknown', 'no error');
  assert.equal(D().saveOutcome(json(400, { error: '' })), 'unknown', 'an empty error');
  assert.equal(D().saveOutcome(json(400, { error: 3 })), 'unknown');
  assert.equal(D().saveOutcome(json(200, { saved: false })), 'unknown', 'a 2xx not saved');
  assert.equal(D().saveOutcome(json(200, { saved: true })), 'unknown', 'not an error at all');
  assert.equal(D().saveOutcome(json(399, { error: 'x' })), 'unknown');
  assert.equal(D().saveOutcome(json(500, {})), 'unknown');
});

// ------------------------------------------------------------------ customerErrorText

test('online, the passport step shows the server sentence as it is', () => {
  assert.equal(D().customerErrorText(true, json(400, { error: CUSTODY_OFF })), CUSTODY_OFF);
  const long = 'ก'.repeat(400);
  assert.equal(D().customerErrorText(true, json(400, { error: long })), long, 'never cut');
  assert.equal(D().customerErrorText(true, json(500, { error: 'ระบบขัดข้อง' })), 'ระบบขัดข้อง');
});

test('online without a sentence, the passport step says the server was not reached', () => {
  const unreached = D().CUSTOMER_UNREACHED;
  assert.equal(D().customerErrorText(true, { failed: true }), unreached);
  assert.equal(D().customerErrorText(true, D().readAnswer(502, '<html>')), unreached);
  assert.equal(D().customerErrorText(true, json(400, { error: '' })), unreached);
  assert.equal(D().customerErrorText(true, null), unreached);
});

test('offline, the browser counter keeps v2 sentence', () => {
  assert.equal(D().customerErrorText(false, null), null);
  assert.equal(D().customerErrorText(false, json(400, { error: CUSTODY_OFF })), null);
});

// ------------------------------------------------------------------ fillsStaff

test('the staff code is filled on /staff/desk only, from a real username', () => {
  assert.equal(D().fillsStaff('/staff/desk', { username: 'pwtest-front', role: 'FRONT' }), true);
  assert.equal(D().fillsStaff('/desk', { username: 'boss', role: 'OWNER' }), false, 'account-bar.js does /desk');
  assert.equal(D().fillsStaff('/staff/desk', { username: '' }), false);
  assert.equal(D().fillsStaff('/staff/desk', {}), false);
  assert.equal(D().fillsStaff('/staff/desk', null), false);
  assert.equal(D().fillsStaff('/staff/desk', undefined), false);
  assert.equal(D().fillsStaff('/staff/desk', { username: 7 }), false);
  assert.equal(D().fillsStaff('/staff/desk/', { username: 'pwtest-front' }), false);
});

// ------------------------------------------------------------------ backToEdit

const refusedState = () => ({
  type: 'SELL',
  currency: 'USD',
  amount: '5000.00',
  rate: '33.2500',
  referenceRate: '33.10',
  name: 'JOHN SMITH',
  passportNo: 'X1234567',
  nationality: 'GBR',
  birthDate: '1980-01-02',
  passportExpiry: '2030-01-02',
  customerId: 'XC-B01-000042',
  staff: 'pwtest-front',
  valueDate: '2026-10-07',
  method: 'Cash',
  account: 'ลิ้นชัก 1',
  fee: 0,
  bankFee: 0,
  notes: '',
  drawerOrderId: '0b6c2a64-6f0e-4c55-9d2f-2d6f0f6f2a11',
  booking: 'BK-7f0a',
  bookedAt: '2026-10-07T08:00:00.000Z',
  ref: '—',
  paidAt: '2026-10-07T08:01:00.000Z',
  submissionId: '5d1c0c56-9a1f-4f0b-8a43-0e0d7c1d2b33',
  receipt: 'PENDING-5d1c0c56',
  documentRef: 'PENDING',
  cloudPayload: { submissionId: '5d1c0c56-9a1f-4f0b-8a43-0e0d7c1d2b33', amount: 5000 },
  cloudResult: undefined,
  cloudError: NO_USD,
});

test('the way back keeps the customer, the passport and the figures', () => {
  const before = refusedState();
  const after = D().backToEdit(before);
  for (const key of [
    'type', 'currency', 'amount', 'rate', 'referenceRate', 'name', 'passportNo', 'nationality',
    'birthDate', 'passportExpiry', 'customerId', 'staff', 'valueDate', 'method', 'account', 'fee',
    'bankFee', 'notes', 'drawerOrderId',
  ]) {
    assert.deepEqual(after[key], before[key], key);
  }
});

test('the way back drops the booking and everything of the refused submission', () => {
  const after = D().backToEdit(refusedState());
  for (const key of [
    'booking', 'bookedAt', 'cloudPayload', 'cloudResult', 'cloudError', 'submissionId', 'paidAt',
    'receipt', 'documentRef',
  ]) {
    assert.equal(key in after, false, `${key} is still there`);
  }
  assert.equal(after.ref, '', 'ref is emptied, as #cancel does');
});

test('the way back gives a new object and leaves the old one as it was', () => {
  const before = refusedState();
  const copy = structuredClone(before);
  const after = D().backToEdit(before);
  assert.notEqual(after, before);
  assert.deepEqual(before, copy);
});

// ------------------------------------------------------------------ buttons

const view = (over) => ({ recordMode: 'LIVE', cloudSaving: false, saved: false, outcome: null, ...over });

test('a refused LIVE save offers the way back and New, never Print', () => {
  assert.deepEqual(D().buttons(view({ outcome: 'refused' })), {
    back: true,
    newEnabled: true,
    printEnabled: false,
    unknownNote: false,
  });
});

test('an unknown LIVE outcome leaves Retry as the only exit, with the note', () => {
  assert.deepEqual(D().buttons(view({ outcome: 'unknown' })), {
    back: false,
    newEnabled: false,
    printEnabled: false,
    unknownNote: true,
  });
});

test('v2 stays as it was everywhere else', () => {
  const v2 = { back: false, unknownNote: false };
  assert.deepEqual(D().buttons(view({})), { ...v2, newEnabled: false, printEnabled: false }, 'LIVE, no answer yet');
  assert.deepEqual(D().buttons(view({ saved: true })), { ...v2, newEnabled: true, printEnabled: true }, 'saved');
  assert.deepEqual(
    D().buttons(view({ saved: true, outcome: 'refused' })),
    { ...v2, newEnabled: true, printEnabled: true },
    'saved wins over an earlier refusal',
  );
  for (const outcome of [null, 'refused', 'unknown']) {
    assert.deepEqual(
      D().buttons(view({ cloudSaving: true, outcome })),
      { ...v2, newEnabled: false, printEnabled: false },
      `saving (${outcome})`,
    );
    assert.deepEqual(
      D().buttons(view({ recordMode: 'DEMO', outcome })),
      { ...v2, newEnabled: true, printEnabled: true },
      `DEMO (${outcome})`,
    );
  }
});

// ------------------------------------------------------------------ saveTransaction attempt reset (:293)

test('the saveTransaction observation is cleared each time a save attempt starts (:293)', async () => {
  const { readFileSync } = await import('node:fs');
  const vm = await import('node:vm');
  const code = readFileSync(SCRIPT, 'utf8');

  class MockEl {
    constructor(id = '') {
      this.id = id;
      this.children = [];
      this.parentNode = null;
    }
    appendChild(child) {
      if (child.parentNode) child.parentNode.removeChild(child);
      this.children.push(child);
      child.parentNode = this;
      return child;
    }
    insertBefore(newChild, refChild) {
      if (newChild.parentNode) newChild.parentNode.removeChild(newChild);
      const idx = this.children.indexOf(refChild);
      if (idx === -1) return this.appendChild(newChild);
      this.children.splice(idx, 0, newChild);
      newChild.parentNode = this;
      return newChild;
    }
    removeChild(child) {
      const idx = this.children.indexOf(child);
      if (idx !== -1) {
        this.children.splice(idx, 1);
        child.parentNode = null;
      }
      return child;
    }
    remove() {
      if (this.parentNode) this.parentNode.removeChild(this);
    }
    get nextSibling() {
      if (!this.parentNode) return null;
      const idx = this.parentNode.children.indexOf(this);
      return this.parentNode.children[idx + 1] || null;
    }
    querySelector() {
      return null;
    }
    addEventListener() {}
    removeEventListener() {}
  }

  const doc = {
    body: new MockEl('body'),
    createElement: (tag) => new MockEl(),
    getElementById(id) {
      function find(node) {
        if (!node) return null;
        if (node.id === id) return node;
        for (const child of node.children) {
          const found = find(child);
          if (found) return found;
        }
        return null;
      }
      return find(doc.body);
    },
    addEventListener() {},
  };
  const panel = new MockEl('panel');
  doc.body.appendChild(panel);
  const retry = new MockEl('retry');
  panel.appendChild(retry);

  class MockMutationObserver {
    observe() {}
    disconnect() {}
  }

  const win = {
    MutationObserver: MockMutationObserver,
    fetch: async () => ({
      ok: false,
      status: 502,
      clone() {
        return this;
      },
      text: async () => '<html>502 Bad Gateway</html>',
    }),
    gasCall: null,
    accountingResult: null,
  };
  doc.defaultView = win;

  const sandbox = {
    window: win,
    document: doc,
    URL,
    JSON,
    Boolean,
    console,
  };
  const context = vm.createContext(sandbox);

  vm.runInContext(`
    let cloudOnline = true, recordMode = 'LIVE', step = 4, s = { type: 'SELL', currency: 'USD', amount: '100' };
    function gasCall(name, payload) {
      if (window.gasCallImpl) return window.gasCallImpl(name, payload);
      return Promise.resolve({});
    }
    function accountingResult() {
      if (window.accountingResultImpl) return window.accountingResultImpl();
    }
  `, context);

  win.gasCall = vm.runInContext('gasCall', context);
  win.accountingResult = vm.runInContext('accountingResult', context);

  vm.runInContext(code, context);

  // Attempt 1: answered 5xx from server -> unknown outcome
  win.gasCallImpl = async () => {
    await win.fetch('/api/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'saveTransaction' }),
    });
    throw new Error('502');
  };
  await assert.rejects(async () => {
    await win.gasCall('saveTransaction', {});
  });
  win.accountingResult();
  assert.equal(doc.getElementById('xc-back'), null, '5xx is unknown: no way back');
  assert.ok(doc.getElementById('xc-save-unknown'), '5xx note shown');

  // Attempt 2: refused before sending (e.g. unconfirmed cash count, failed file upload)
  win.gasCallImpl = async () => {
    throw new Error('ยังไม่ยืนยันการนับเงิน');
  };
  await assert.rejects(async () => {
    await win.gasCall('saveTransaction', {});
  });
  win.accountingResult();

  // If :293 reset cleared lastActionAnswer, attempt 2 observed nothing (null) -> refused -> way back offered
  // If :293 reset was missing, attempt 2 would inherit attempt 1's 5xx -> unknown -> xc-back absent
  assert.ok(
    doc.getElementById('xc-back'),
    'attempt refused before sending reads refused and offers the way back',
  );
  assert.equal(doc.getElementById('xc-save-unknown'), null, 'unknown note cleared');
});


// ------------------------------------------------------------------ #3008: draftBegin replaces opsBeginTransaction

const v3post = (body) => ({
  method: 'POST',
  headers: { 'content-type': 'application/json', 'x-xchange-request': '1', 'idempotency-key': 'k-1' },
  body: JSON.stringify(body),
});

test('#3008 rpcAction names a POST to /api/tx-drafts draftBegin', () => {
  const body = { branch: 'XC-B01', side: 'BUY', currency: 'USD', amount: '100' };
  assert.equal(D().rpcAction('/api/tx-drafts', v3post(body)), 'draftBegin');
  assert.equal(D().rpcAction('http://xc.test/api/tx-drafts', v3post(body)), 'draftBegin');
  assert.equal(D().rpcAction('/api/tx-drafts', { method: 'GET' }), null, 'only the POST');
});

test('#3008 readAnswer reads the message of a v3 refusal, so the desk shows the real reason', () => {
  const refusal = { error: { code: 'noOpenShift', message: 'ยังไม่ได้เปิดกะของคุณ' } };
  assert.equal(D().readAnswer(409, JSON.stringify(refusal)).error, 'ยังไม่ได้เปิดกะของคุณ');
  assert.equal(D().customerErrorText(true, D().readAnswer(409, JSON.stringify(refusal))), 'ยังไม่ได้เปิดกะของคุณ');
  assert.equal(D().readAnswer(400, JSON.stringify({ error: 'v2 sentence' })).error, 'v2 sentence', 'v2 kept');
});

test('#3008 the desk script watches draftBegin, not opsBeginTransaction', async () => {
  const { readFileSync } = await import('node:fs');
  const source = readFileSync(SCRIPT, 'utf8');
  assert.equal(source.includes('opsBeginTransaction'), false, 'no opsBeginTransaction left in xc-desk.js');
  assert.ok(source.includes("'draftBegin'"), "updateCustomerError observes lastActionAnswer['draftBegin']");
});
