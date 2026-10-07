// The form helper's pure parts (row FV, docs/design/xchange-v3.md §5.3
// "The shared form helper"). Written before the code. Do not edit, delete or weaken.
//
// Run from the repository root:   node --test api/tests/js/*.test.mjs
//
// api/static/xc-forms.js is a classic browser script. In a browser it installs
// itself on `document`; under Node, where `module` exists, it installs nothing and
// exports the functions below through `module.exports`.
//
// No DOM here: the tests hand the functions small fake elements.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('../../static/xc-forms.js', import.meta.url));
const require = createRequire(import.meta.url);
const F = require(SCRIPT);

/** What a browser does with a `pattern` attribute (HTML: `^(?:p)$`, flag `v`). */
const browser = (p) => new RegExp(`^(?:${p})$`, 'v');
/** What browsers before the `v` flag did (flag `u`), the rule v2's author saw run. */
const older = (p) => new RegExp(`^(?:${p})$`, 'u');
const compiles = (p) => {
  try {
    browser(p);
    return true;
  } catch {
    return false;
  }
};

// The two patterns of the kept bundle (public/employees.js, public/login.html).
const EMPLOYEE = '[a-z0-9._-]{3,40}';
const LOGIN = '[a-zA-Z0-9._-]{3,30}';

// ------------------------------------------------------------------ 1. patterns

test('the two kept patterns do not compile in a current browser (the root cause)', () => {
  assert.equal(compiles(EMPLOYEE), false);
  assert.equal(compiles(LOGIN), false);
});

test('the employee pattern is repaired by escaping the dash that ends its class', () => {
  const fixed = F.fixPattern(EMPLOYEE);
  assert.equal(fixed, '[a-z0-9._\\-]{3,40}');
  const rule = browser(fixed);
  // the owner's case
  assert.equal(rule.test('t1'), false, 't1 is refused');
  for (const ok of ['front01', 'a.b_c-d', 'abc', 'x'.repeat(40)]) {
    assert.equal(rule.test(ok), true, ok);
  }
  for (const bad of ['ab', 'x'.repeat(41), 'Front01', 'front 01', 'ฟร้อน01', 'a/b']) {
    assert.equal(rule.test(bad), false, bad);
  }
});

test('a repaired pattern accepts exactly what the old browsers accepted', () => {
  const samples = ['t1', 'abc', 'a-b', 'a_b', 'a.b', 'A-b', 'ab-', '-ab', 'a]b', 'a\\b', 'abc def', ''];
  for (const p of ['[a-z0-9._-]{3,40}', '[a-]x[b-]', '[a\\\\-]+']) {
    const fixed = F.fixPattern(p);
    assert.notEqual(fixed, null, p);
    assert.equal(compiles(fixed), true, `${p} -> ${fixed}`);
    for (const s of samples) {
      assert.equal(browser(fixed).test(s), older(p).test(s), `${p} -> ${fixed} on ${JSON.stringify(s)}`);
    }
  }
});

test('every class in a pattern is repaired, and a dash after an escaped backslash counts', () => {
  assert.equal(F.fixPattern('[a-]x[b-]'), '[a\\-]x[b\\-]');
  // `\\` is an escaped backslash, so the dash after it is not escaped
  assert.equal(F.fixPattern('[a\\\\-]+'), '[a\\\\\\-]+');
});

test('a pattern that compiles is left alone, so repairing twice changes nothing', () => {
  for (const p of ['[a-z]{3}', '[a-z0-9._\\-]{3,40}', '\\d{4}', '.*', '']) {
    assert.equal(F.fixPattern(p), null, p);
  }
  const once = F.fixPattern(EMPLOYEE);
  assert.equal(F.fixPattern(once), null);
});

test('a pattern the dash rule cannot repair is left alone', () => {
  // broken for another reason, or a dash the rule does not cover
  for (const p of ['(', '[a-z(]', 'a-]', '[-a]', '[a-z]{3,2}']) {
    assert.equal(compiles(p), false, `${p} is broken to begin with`);
    assert.equal(F.fixPattern(p), null, p);
  }
});

test('the login pattern becomes the 3-40 the sign-in door accepts', () => {
  // login.html says 3-30 (the owner's rule), but /api/auth signs employees in too,
  // whose names are 3-40 (server.mjs:7871-7883, v3 service.rs:150-197).
  assert.equal(F.PATTERN_OVERRIDES[LOGIN], '[a-zA-Z0-9._\\-]{3,40}');
  const fixed = F.fixPattern(LOGIN);
  assert.equal(fixed, '[a-zA-Z0-9._\\-]{3,40}');
  const rule = browser(fixed);
  assert.equal(rule.test('t1'), false);
  assert.equal(rule.test('Owner.01'), true, 'capitals are allowed here: the server lower-cases');
  assert.equal(rule.test('e'.repeat(40)), true, 'an employee name of 40 signs in');
  assert.equal(rule.test('e'.repeat(41)), false);
});

test('the override table holds the login pattern only', () => {
  assert.deepEqual(Object.keys(F.PATTERN_OVERRIDES), [LOGIN]);
  // a name that is not a key, though every object has it
  assert.equal(F.fixPattern('constructor'), null);
  assert.equal(F.fixPattern('toString'), null);
});

// --------------------------------------------------------- 2. the first invalid

const field = (valid, name) => ({ name, checkValidity: () => valid, willValidate: true });

test('firstInvalid answers the first field that fails, in document order', () => {
  const a = field(true, 'a');
  const b = field(false, 'b');
  const c = field(false, 'c');
  assert.equal(F.firstInvalid([a, b, c]), b);
  assert.equal(F.firstInvalid([a]), null);
  assert.equal(F.firstInvalid([]), null);
  // a button or fieldset that takes no part in validation is skipped
  const button = { name: 'btn', checkValidity: () => false, willValidate: false };
  assert.equal(F.firstInvalid([button, c]), c);
});

// -------------------------------------------------------------- 3. red marking

class FakeElement {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.attributes = new Map();
    this.listeners = new Map();
    this.children = [];
    this.style = {};
    this.dataset = {};
    const classes = new Set();
    this.classList = {
      add: (...c) => c.forEach((x) => classes.add(x)),
      remove: (...c) => c.forEach((x) => classes.delete(x)),
      contains: (c) => classes.has(c),
      toggle: (c, on) => {
        const want = on === undefined ? !classes.has(c) : on;
        if (want) classes.add(c);
        else classes.delete(c);
        return want;
      },
    };
  }
  // a reflected attribute, as in a real DOM
  get type() {
    return this.attributes.get('type') ?? '';
  }
  set type(v) {
    this.attributes.set('type', String(v));
  }
  setAttribute(k, v) {
    this.attributes.set(k, String(v));
  }
  getAttribute(k) {
    return this.attributes.has(k) ? this.attributes.get(k) : null;
  }
  hasAttribute(k) {
    return this.attributes.has(k);
  }
  removeAttribute(k) {
    this.attributes.delete(k);
  }
  addEventListener(type, fn) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(fn);
  }
  removeEventListener() {}
  appendChild(child) {
    this.children.push(child);
    return child;
  }
  append(...c) {
    this.children.push(...c);
  }
  /** What a click on it does: every click listener, with an event that records preventDefault. */
  click() {
    const event = { type: 'click', target: this, currentTarget: this, defaultPrevented: false };
    event.preventDefault = () => {
      event.defaultPrevented = true;
    };
    event.stopPropagation = () => {};
    for (const fn of this.listeners.get('click') ?? []) fn.call(this, event);
    return event;
  }
}

const fakeDocument = () => ({
  createElement: (tag) => new FakeElement(tag),
  createElementNS: (_ns, tag) => new FakeElement(tag),
  createTextNode: (text) => ({ nodeType: 3, textContent: text }),
});

test('markInvalid marks a field red and says so to assistive technology; clearMark undoes it', () => {
  const el = new FakeElement('input');
  F.markInvalid(el);
  assert.equal(el.classList.contains('xc-invalid'), true);
  assert.equal(el.getAttribute('aria-invalid'), 'true');
  F.clearMark(el);
  assert.equal(el.classList.contains('xc-invalid'), false);
  assert.equal(el.getAttribute('aria-invalid'), null);
});

test('the style sheet colours the marked field and the field the browser finds invalid', () => {
  assert.equal(typeof F.CSS, 'string');
  assert.match(F.CSS, /\.xc-invalid/);
  assert.match(F.CSS, /:user-invalid/);
  assert.match(F.CSS, /\.xc-reveal/);
});

// ------------------------------------------------------ 4. server refusals → field

const BRANCH = 'ไม่พบสาขาที่เลือก กรุณารีเฟรช';
const PASSWORD = 'รหัสผ่านต้องมี 12–128 ตัวอักษร'; // U+2013 between the numbers
const EMPLOYEE_DATA = 'ข้อมูลพนักงานไม่ครบหรือไม่ถูกต้อง';
const WRONG = 'ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง';

test('the refusal table holds exactly the sentences that name one field', () => {
  assert.deepEqual(F.REFUSALS, {
    'employee-error': {
      [BRANCH]: 'employee-branch',
      [PASSWORD]: 'employee-password',
    },
    'reset-error': {
      [PASSWORD]: 'reset-employee-password',
    },
  });
});

test('fieldFor points a listed sentence at its field', () => {
  assert.equal(F.fieldFor('employee-error', BRANCH), 'employee-branch');
  assert.equal(F.fieldFor('employee-error', PASSWORD), 'employee-password');
  assert.equal(F.fieldFor('reset-error', PASSWORD), 'reset-employee-password');
  // the screen may print the sentence with spaces around it
  assert.equal(F.fieldFor('employee-error', `  ${BRANCH}\n`), 'employee-branch');
});

test('fieldFor answers null where the sentence does not name one field', () => {
  // four fields share it (service.rs:391-412)
  assert.equal(F.fieldFor('employee-error', EMPLOYEE_DATA), null);
  // the sign-in sentence must not say which half was wrong
  assert.equal(F.fieldFor('message', WRONG), null);
  // a sentence of one form under another form's error element
  assert.equal(F.fieldFor('reset-error', BRANCH), null);
  assert.equal(F.fieldFor('no-such-error', BRANCH), null);
  assert.equal(F.fieldFor('employee-error', ''), null);
  assert.equal(F.fieldFor('employee-error', 'ไม่พบสาขาที่เลือก'), null, 'part of a sentence');
  assert.equal(F.fieldFor('employee-error', PASSWORD.replace('–', '-')), null, 'a hyphen is not the en dash');
  // names every object has are not table entries
  assert.equal(F.fieldFor('constructor', BRANCH), null);
  assert.equal(F.fieldFor('employee-error', 'constructor'), null);
  assert.equal(F.fieldFor('employee-error', '__proto__'), null);
});

// ------------------------------------------------------- 5. show / hide a password

/** A password field whose value must never be read. */
function passwordField() {
  const form = { submitted: 0 };
  form.submit = () => {
    form.submitted += 1;
  };
  form.requestSubmit = () => {
    form.submitted += 1;
  };
  const input = new FakeElement('input');
  input.type = 'password';
  input.form = form;
  input.selectionStart = 3;
  input.selectionEnd = 5;
  input.focused = 0;
  input.selection = null;
  input.focus = () => {
    input.focused += 1;
  };
  input.setSelectionRange = (a, b) => {
    input.selection = [a, b];
    input.selectionStart = a;
    input.selectionEnd = b;
  };
  Object.defineProperty(input, 'value', {
    get() {
      throw new Error('the helper read the password');
    },
    set() {
      throw new Error('the helper wrote the password');
    },
  });
  return input;
}

test('revealLabel says what the button will do, in Thai', () => {
  assert.equal(F.revealLabel(false), 'แสดงรหัสผ่าน');
  assert.equal(F.revealLabel(true), 'ซ่อนรหัสผ่าน');
});

test('flipReveal turns the field to text and back, keeping focus and the caret', () => {
  const input = passwordField();
  assert.equal(F.flipReveal(input), true, 'now shown');
  assert.equal(input.type, 'text');
  assert.deepEqual(input.selection, [3, 5]);
  assert.ok(input.focused >= 1, 'focus is back in the field');

  input.selectionStart = 7;
  input.selectionEnd = 7;
  assert.equal(F.flipReveal(input), false, 'now hidden');
  assert.equal(input.type, 'password');
  assert.deepEqual(input.selection, [7, 7]);
  assert.equal(input.form.submitted, 0);
});

test('the button is a plain button: it reveals, it does not submit', () => {
  const input = passwordField();
  const button = F.makeRevealButton(fakeDocument(), input);
  assert.equal(button.tagName, 'BUTTON');
  assert.equal(button.getAttribute('type'), 'button', 'type="button" never submits a form');
  assert.equal(button.getAttribute('aria-label'), 'แสดงรหัสผ่าน');
  assert.equal(button.getAttribute('aria-pressed'), 'false');
  assert.equal(button.classList.contains('xc-reveal'), true);
  assert.notEqual(button.getAttribute('tabindex'), '-1', 'reachable from the keyboard');
  assert.equal(button.disabled === true, false);

  button.click();
  assert.equal(input.type, 'text');
  assert.equal(button.getAttribute('aria-label'), 'ซ่อนรหัสผ่าน');
  assert.equal(button.getAttribute('aria-pressed'), 'true');

  button.click();
  assert.equal(input.type, 'password');
  assert.equal(button.getAttribute('aria-label'), 'แสดงรหัสผ่าน');
  assert.equal(button.getAttribute('aria-pressed'), 'false');

  assert.equal(input.form.submitted, 0, 'the form was not submitted');
});

test('concealAll turns every revealed field back to a password field', () => {
  const a = passwordField();
  const b = passwordField();
  const c = passwordField();
  F.flipReveal(a);
  F.flipReveal(c);
  F.concealAll([a, b, c]);
  assert.deepEqual([a.type, b.type, c.type], ['password', 'password', 'password']);
  F.concealAll([]);
});

// ------------------------------------------------------------ what it never does

test('the script reads no field value and sends, stores or logs nothing', () => {
  const text = readFileSync(SCRIPT, 'utf8');
  for (const banned of [
    /\.value\b/,
    /console\./,
    /localStorage/,
    /sessionStorage/,
    /indexedDB/,
    /document\.cookie/,
    /clipboard/,
    /\bfetch\s*\(/,
    /XMLHttpRequest/,
    /sendBeacon/,
    /WebSocket/,
    /\beval\s*\(/,
    /new\s+Function\b/,
  ]) {
    assert.doesNotMatch(text, banned);
  }
});

test('under Node the script installs nothing and touches no global', () => {
  assert.equal(typeof globalThis.document, 'undefined');
  for (const name of ['fixPattern', 'firstInvalid', 'markInvalid', 'clearMark', 'fieldFor', 'flipReveal', 'revealLabel', 'makeRevealButton', 'concealAll']) {
    assert.equal(typeof F[name], 'function', name);
  }
});
