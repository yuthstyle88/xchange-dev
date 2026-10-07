// VM tests for xc-desk.js in the context of v2 desk declarations (issue #332)
// Verifies top-level `let` globals (cloudOnline, recordMode, step, s, staffDraftId)
// are read and assigned lexically, never creating or preferring win.* copies.
//
// Run from repository root: node --test api/tests/js/*.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const SCRIPT_PATH = fileURLToPath(new URL('../../static/xc-desk.js', import.meta.url));
const SCRIPT_CODE = fs.readFileSync(SCRIPT_PATH, 'utf8');

class MockElement {
  constructor(tagName = 'div', id = '') {
    this.tagName = tagName.toUpperCase();
    this.id = id;
    this.textContent = '';
    this.value = '';
    this.readOnly = false;
    this.title = '';
    this.disabled = false;
    this.children = [];
    this.parentNode = null;
    this.listeners = {};
    this.onclick = null;
    this.onsubmit = null;
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
    if (idx === -1) {
      return this.appendChild(newChild);
    }
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
    if (this.parentNode) {
      this.parentNode.removeChild(this);
    }
  }

  get nextSibling() {
    if (!this.parentNode) return null;
    const idx = this.parentNode.children.indexOf(this);
    return this.parentNode.children[idx + 1] || null;
  }

  addEventListener(event, fn) {
    if (!this.listeners[event]) this.listeners[event] = [];
    this.listeners[event].push(fn);
  }

  dispatchEvent(event) {
    const list = this.listeners[event.type] || [];
    for (const fn of list) {
      fn.call(this, event);
    }
    if (event.type === 'click' && typeof this.onclick === 'function') {
      this.onclick.call(this, event);
    }
  }

  click() {
    this.dispatchEvent({ type: 'click', target: this });
  }

  querySelector(selector) {
    if (selector === 'form') {
      for (const child of this.children) {
        if (child.tagName === 'FORM') return child;
        const found = child.querySelector(selector);
        if (found) return found;
      }
    }
    if (selector.startsWith('#')) {
      const targetId = selector.slice(1);
      if (this.id === targetId) return this;
      for (const child of this.children) {
        if (child.id === targetId) return child;
        const found = child.querySelector(selector);
        if (found) return found;
      }
    }
    return null;
  }
}

class MockDocument {
  constructor() {
    this.body = this.createElement('body');
    this.body.id = 'body';
    this.listeners = {};
    this.defaultView = null;
  }

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
    return find(this.body);
  }

  createElement(tag) {
    const el = new MockElement(tag);
    el.ownerDocument = this;
    return el;
  }

  addEventListener(event, fn) {
    if (!this.listeners[event]) this.listeners[event] = [];
    this.listeners[event].push(fn);
  }

  dispatchEvent(event) {
    const list = this.listeners[event.type] || [];
    for (const fn of list) {
      fn.call(this, event);
    }
  }
}

function createDeskContext({
  cloudOnline = true,
  recordMode = 'LIVE',
  step = 0,
  s = {},
  pathname = '/staff/desk',
  fetchHandler = null,
} = {}) {
  const doc = new MockDocument();
  const panel = doc.createElement('div');
  panel.id = 'panel';
  doc.body.appendChild(panel);

  const customerError = doc.createElement('p');
  customerError.id = 'customerError';
  customerError.textContent = 'ไม่สามารถสร้างรหัสลูกค้าได้ (browser counter)';
  panel.appendChild(customerError);

  const form = doc.createElement('form');
  panel.appendChild(form);

  const retry = doc.createElement('button');
  retry.id = 'retry';
  panel.appendChild(retry);

  const newBtn = doc.createElement('button');
  newBtn.id = 'new';
  panel.appendChild(newBtn);

  const printBtn = doc.createElement('button');
  printBtn.id = 'print';
  panel.appendChild(printBtn);

  const staffInput = doc.createElement('input');
  staffInput.id = 'staff';
  panel.appendChild(staffInput);

  class MockMutationObserver {
    constructor(cb) {
      this.cb = cb;
    }
    observe() {}
    disconnect() {}
  }

  const win = {
    location: { pathname },
    MutationObserver: MockMutationObserver,
    fetch: async (input, init) => {
      if (fetchHandler) {
        return fetchHandler(input, init);
      }
      return {
        ok: true,
        status: 200,
        json: async () => ({}),
        text: async () => '{}',
        clone() {
          return this;
        },
      };
    },
    gasCall: null,
    accountingResult: null,
    renderPassport: null,
    render: null,
  };
  doc.defaultView = win;

  const sandbox = {
    window: win,
    document: doc,
    URL,
    JSON,
    Boolean,
    console,
    queueMicrotask: (fn) => queueMicrotask(fn),
    setTimeout,
    clearTimeout,
  };

  const context = vm.createContext(sandbox);

  // Declare top-level lexical bindings via `let` as desk.html does
  vm.runInContext(`
    let cloudOnline = ${JSON.stringify(cloudOnline)};
    let recordMode = ${JSON.stringify(recordMode)};
    let step = ${JSON.stringify(step)};
    let s = ${JSON.stringify(s)};
    let staffDraftId = 'draft-init';
    let cloudSaving = false;
    function gasCall(name, payload) {
      if (window.gasCallImpl) return window.gasCallImpl(name, payload);
      return Promise.resolve({ saved: true });
    }
    function accountingResult() {
      if (window.accountingResultImpl) return window.accountingResultImpl();
    }
    function renderPassport() {
      if (window.renderPassportImpl) return window.renderPassportImpl();
    }
    function render() {
      if (window.renderImpl) return window.renderImpl();
    }
  `, context);

  win.gasCall = vm.runInContext('gasCall', context);
  win.accountingResult = vm.runInContext('accountingResult', context);
  win.renderPassport = vm.runInContext('renderPassport', context);
  win.render = vm.runInContext('render', context);

  return { context, win, doc, panel, form, customerError, retry, newBtn, printBtn, staffInput };
}

// ------------------------------------------------------------------ Rows 1 & 2
test('row 1: server refusal on passport step replaces customer error using lexical cloudOnline / recordMode', async () => {
  const refusal = 'เจ้าของต้องเปิดใช้ขั้นตอนรับ–ส่งเงินก่อน';
  const env = createDeskContext({
    cloudOnline: true,
    recordMode: 'LIVE',
    fetchHandler: async (url, init) => {
      return {
        ok: false,
        status: 400,
        json: async () => ({ error: refusal }),
        text: async () => JSON.stringify({ error: refusal }),
        clone() {
          return this;
        },
      };
    },
  });

  // Verify that window.* does NOT hold the state
  assert.equal(typeof env.win.cloudOnline, 'undefined');
  assert.equal(typeof env.win.recordMode, 'undefined');

  // Form submit in desk.html makes the RPC call
  env.form.onsubmit = async () => {
    await env.win.fetch('/api/staff/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'reserveCustomer' }),
    });
  };

  // Run xc-desk.js in the VM context
  vm.runInContext(SCRIPT_CODE, env.context);

  // Trigger passport form submit
  await env.form.onsubmit();

  // The error element must show the server's refusal sentence
  assert.equal(env.customerError.textContent, refusal);
});

test('row 2: unreached server on passport step replaces customer error with CUSTOMER_UNREACHED', async () => {
  const env = createDeskContext({
    cloudOnline: true,
    recordMode: 'LIVE',
    fetchHandler: async (url, init) => {
      return {
        ok: false,
        status: 502,
        json: async () => {
          throw new Error('not json');
        },
        text: async () => '<html>502 Bad Gateway</html>',
        clone() {
          return this;
        },
      };
    },
  });

  env.form.onsubmit = async () => {
    await env.win.fetch('/api/staff/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'reserveCustomer' }),
    });
  };

  vm.runInContext(SCRIPT_CODE, env.context);

  await env.form.onsubmit();

  assert.equal(
    env.customerError.textContent,
    'สร้างรหัสลูกค้าไม่ได้ เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่',
  );
});

test('row 3: offline desk keeps v2 browser counter sentence on customer error', async () => {
  const initialText = 'ไม่สามารถสร้างรหัสลูกค้าได้ (browser counter)';
  const env = createDeskContext({
    cloudOnline: false,
    recordMode: 'DEMO',
    fetchHandler: async () => ({
      ok: false,
      status: 400,
      json: async () => ({ error: 'refused' }),
      text: async () => JSON.stringify({ error: 'refused' }),
      clone() {
        return this;
      },
    }),
  });

  env.form.onsubmit = async () => {
    await env.win.fetch('/api/staff/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'reserveCustomer' }),
    });
  };

  vm.runInContext(SCRIPT_CODE, env.context);

  await env.form.onsubmit();

  // Must keep the browser counter sentence
  assert.equal(env.customerError.textContent, initialText);
});

// ------------------------------------------------------------------ The Way Back (Row 7)
test('the way back assigns lexical s and step, never window.s or window.step, allowing subsequent page updates', async () => {
  const initialS = {
    type: 'SELL',
    currency: 'USD',
    amount: '5000.00',
    rate: '33.2500',
    booking: 'BK-1234',
    bookedAt: '2026-10-07T08:00:00Z',
    submissionId: 'sub-789',
    paidAt: '2026-10-07T08:01:00Z',
    ref: 'REF-1234',
    staff: 'pwtest-front',
  };

  const env = createDeskContext({
    cloudOnline: true,
    recordMode: 'LIVE',
    step: 4,
    s: initialS,
    fetchHandler: async (url, init) => {
      return {
        ok: false,
        status: 400,
        json: async () => ({ error: 'ยอด USD ไม่เพียงพอ' }),
        text: async () => JSON.stringify({ error: 'ยอด USD ไม่เพียงพอ' }),
        clone() {
          return this;
        },
      };
    },
  });

  let renderCalled = false;
  env.win.renderImpl = () => {
    renderCalled = true;
  };

  vm.runInContext(SCRIPT_CODE, env.context);

  // Perform a refused saveTransaction
  env.win.gasCallImpl = async () => {
    await env.win.fetch('/api/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'saveTransaction' }),
    });
    return { saved: false };
  };

  await env.win.gasCall('saveTransaction', {});

  // accountingResult renders step-4 outcome
  env.win.accountingResult();

  // The way back button should be present
  const backBtn = env.doc.getElementById('xc-back');
  assert.ok(backBtn, 'back button should exist');

  // Click the way back button
  backBtn.click();
  assert.ok(renderCalled, 'render() should have been called');

  // Verify lexical variables in VM context were updated
  const currentStep = vm.runInContext('step', env.context);
  assert.equal(currentStep, 0, 'lexical step must be reset to 0');

  const currentS = vm.runInContext('s', env.context);
  assert.equal(currentS.ref, '', 'ref in s must be cleared');
  assert.equal('booking' in currentS, false, 'booking in s must be dropped');
  assert.equal('submissionId' in currentS, false, 'submissionId in s must be dropped');
  assert.equal('paidAt' in currentS, false, 'paidAt in s must be dropped');
  assert.equal(currentS.amount, '5000.00', 'figures in s must be preserved');

  const draftId = vm.runInContext('staffDraftId', env.context);
  assert.equal(draftId, '', 'staffDraftId must be cleared');

  // CRITICAL: window.s and window.step must NEVER have been defined!
  assert.equal(typeof env.win.s, 'undefined', 'window.s must remain undefined');
  assert.equal(typeof env.win.step, 'undefined', 'window.step must remain undefined');
  assert.equal(typeof env.win.ref, 'undefined', 'window.ref must remain undefined');

  // Verify subsequent page updates are observed by xc-desk.js:
  // Simulate user editing figures and proceeding to step 2:
  vm.runInContext(`
    s = { type: 'BUY', currency: 'EUR', amount: '200.00', staff: 'pwtest-front' };
    step = 2;
  `, env.context);

  // Trigger applyStaff or wrapNew
  env.newBtn.click();

  // Verify the script reads the updated lexical s and step, not the old snapshot
  const stepAfter = vm.runInContext('step', env.context);
  assert.equal(stepAfter, 2, 'lexical step remains 2');
  const sAfter = vm.runInContext('s', env.context);
  assert.equal(sAfter.currency, 'EUR', 'lexical s is the updated currency');
  assert.equal(typeof env.win.s, 'undefined', 'window.s is still undefined');
});

// ------------------------------------------------------------------ Follow-up 1 (per-attempt reset)
test('gasCall clears saveTransaction observation when a new attempt starts', async () => {
  let attempt = 0;
  const env = createDeskContext({
    cloudOnline: true,
    recordMode: 'LIVE',
    step: 4,
    s: { type: 'SELL', currency: 'USD', amount: '100' },
    fetchHandler: async (url, init) => {
      if (url === '/api/auth') {
        return {
          ok: true,
          status: 200,
          json: async () => ({ user: { username: 'pwtest-front' } }),
          text: async () => JSON.stringify({ user: { username: 'pwtest-front' } }),
          clone() {
            return this;
          },
        };
      }
      attempt++;
      if (attempt === 1) {
        return {
          ok: false,
          status: 400,
          json: async () => ({ error: 'ยอด USD ไม่เพียงพอ' }),
          text: async () => JSON.stringify({ error: 'ยอด USD ไม่เพียงพอ' }),
          clone() {
            return this;
          },
        };
      }
      return {
        ok: true,
        status: 200,
        json: async () => ({ saved: true }),
        text: async () => JSON.stringify({ saved: true }),
        clone() {
          return this;
        },
      };
    },
  });

  // Attempt 1: fails
  env.win.gasCallImpl = async () => {
    await env.win.fetch('/api/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'saveTransaction' }),
    });
    return { saved: false };
  };

  vm.runInContext(SCRIPT_CODE, env.context);

  await env.win.gasCall('saveTransaction', {});
  env.win.accountingResult();
  assert.ok(env.doc.getElementById('xc-back'), 'back button present after refused attempt');

  // Attempt 2: succeeds
  env.win.gasCallImpl = async () => {
    await env.win.fetch('/api/rpc', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ action: 'saveTransaction' }),
    });
    return { saved: true };
  };

  await env.win.gasCall('saveTransaction', {});
  env.win.accountingResult();
  assert.equal(env.doc.getElementById('xc-back'), null, 'back button removed after successful attempt');
});
