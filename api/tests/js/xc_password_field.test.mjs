// The password field component of the form helper (issue #378, row FV-TS of
// docs/design/xchange-v3.md). Written before the code.
//
// Run from the repository root:   node --test api/tests/js/*.test.mjs
//
// The owner, on the kept "สร้างบัญชีพนักงาน" dialog, 2026-10-08: the eye rendered under the
// field ("ไม่สวย เลย"); wanted it inside the field on the right with one border
// ("เอาแบบนี้ ดิ"), as one component reused by every password field.
//
// No DOM here: a small fake tree with just the calls the component makes.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('../../static/xc-forms.js', import.meta.url));
const require = createRequire(import.meta.url);
const F = require(SCRIPT);

class Node {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.attributes = new Map();
    this.childNodes = [];
    this.parentNode = null;
    this.listeners = new Map();
    const classes = new Set();
    this.classList = {
      add: (...c) => c.forEach((x) => classes.add(x)),
      remove: (...c) => c.forEach((x) => classes.delete(x)),
      contains: (c) => classes.has(c),
    };
  }
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
  detach() {
    if (this.parentNode) {
      const siblings = this.parentNode.childNodes;
      siblings.splice(siblings.indexOf(this), 1);
      this.parentNode = null;
    }
  }
  appendChild(child) {
    child.detach();
    child.parentNode = this;
    this.childNodes.push(child);
    return child;
  }
  append(...children) {
    for (const c of children) this.appendChild(c);
  }
  insertBefore(child, ref) {
    child.detach();
    child.parentNode = this;
    const i = ref ? this.childNodes.indexOf(ref) : -1;
    if (i < 0) this.childNodes.push(child);
    else this.childNodes.splice(i, 0, child);
    return child;
  }
  get nextElementSibling() {
    if (!this.parentNode) return null;
    const s = this.parentNode.childNodes;
    return s[s.indexOf(this) + 1] ?? null;
  }
  get previousElementSibling() {
    if (!this.parentNode) return null;
    const s = this.parentNode.childNodes;
    return s[s.indexOf(this) - 1] ?? null;
  }
}

const fakeDocument = () => ({
  createElement: (tag) => new Node(tag),
  createElementNS: (_ns, tag) => new Node(tag),
});

/** `<label>รหัสผ่านชั่วคราว<input type="password"></label>` followed by a hint, as v2 renders it. */
function dialog() {
  const label = new Node('label');
  const before = new Node('span');
  const input = new Node('input');
  input.type = 'password';
  input.setAttribute('id', 'employee-password');
  Object.defineProperty(input, 'value', {
    get() {
      throw new Error('the component read the password');
    },
  });
  const after = new Node('p');
  label.append(before, input, after);
  return { label, before, input, after };
}

test('the component wraps the field in place: the wrapper takes the field\'s slot', () => {
  const { label, before, input, after } = dialog();
  const wrapper = F.wrapPasswordField(fakeDocument(), input);

  assert.equal(wrapper.tagName, 'SPAN');
  assert.equal(wrapper.classList.contains('xc-password'), true);
  assert.deepEqual(label.childNodes, [before, wrapper, after], 'nothing else in the label moved');
  assert.equal(input.parentNode, wrapper, 'the field now lives in the wrapper');
});

test('the eye sits inside the wrapper, right after the field', () => {
  const { input } = dialog();
  const wrapper = F.wrapPasswordField(fakeDocument(), input);

  assert.equal(wrapper.childNodes.length, 2);
  assert.equal(wrapper.childNodes[0], input);
  const eye = wrapper.childNodes[1];
  assert.equal(eye.tagName, 'BUTTON');
  assert.equal(eye.classList.contains('xc-reveal'), true);
  assert.equal(eye.getAttribute('type'), 'button', 'never submits');
  // the submit handler and the orphan sweep find the eye by sibling, so that link must hold
  assert.equal(input.nextElementSibling, eye);
  assert.equal(eye.previousElementSibling, input);
  assert.equal(input.getAttribute('data-xc-revealable'), 'true');
  assert.equal(eye.getAttribute('data-xc-reveal'), 'true');
});

test('wrapping twice changes nothing: one wrapper, one eye', () => {
  const { label, input } = dialog();
  const doc = fakeDocument();
  const first = F.wrapPasswordField(doc, input);
  const second = F.wrapPasswordField(doc, input);

  assert.equal(second, first);
  assert.equal(label.childNodes.filter((n) => n.classList.contains('xc-password')).length, 1);
  assert.equal(first.childNodes.length, 2);
});

test('the eye reveals and hides the field it was made for', () => {
  const { input } = dialog();
  const wrapper = F.wrapPasswordField(fakeDocument(), input);
  const eye = wrapper.childNodes[1];
  const click = () => {
    const event = { preventDefault() {} };
    for (const fn of eye.listeners.get('click') ?? []) fn.call(eye, event);
  };
  input.focus = () => {};
  click();
  assert.equal(input.type, 'text');
  click();
  assert.equal(input.type, 'password');
});

test('a field with no parent is left alone', () => {
  const input = new Node('input');
  input.type = 'password';
  assert.equal(F.wrapPasswordField(fakeDocument(), input), null);
  assert.equal(input.parentNode, null);
});

test('the style puts the eye inside the field and keeps one border', () => {
  const css = F.CSS;
  // the wrapper is the positioning box, the eye is pinned to its right edge
  assert.match(css, /\.xc-password\s*\{[^}]*position:\s*relative/);
  assert.match(css, /\.xc-password\s+\.xc-reveal\s*\{[^}]*position:\s*absolute/);
  assert.match(css, /\.xc-password\s+\.xc-reveal\s*\{[^}]*right:\s*0/);
  // the field leaves room for the eye
  assert.match(css, /\.xc-password\s*>\s*input\s*\{[^}]*padding-right/);
  // a focused field shows one border, not the page's outer ring
  assert.match(css, /\.xc-password\s*>\s*input:focus\s*\{[^}]*outline:\s*none/);
  // an invalid field is marked by its border only — the pink fill is gone
  assert.doesNotMatch(css, /#fdecea/i);
  assert.doesNotMatch(css, /background-color/);
});
