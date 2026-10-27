// RF-2: v2's desk page still reads an RPC refusal that names a field
// (docs/design/rpc-refusal-field.md §4 rows R19–R21, decision RF-D1).
// Written before the implementation by the sub-controller (RF-0). Do not edit, delete or weaken.
// These hold api/static/xc-desk.js, which RF-2 does not change: they pass before and after it.
//
// Run from the repository root:   node --test api/tests/js/*.test.mjs

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const SCRIPT = fileURLToPath(new URL('../../static/xc-desk.js', import.meta.url));
const require = createRequire(import.meta.url);
let loaded;
const D = () => (loaded ??= require(SCRIPT));

const BAD_TEXT = 'ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด';
const NAMED = JSON.stringify({ error: BAD_TEXT, field: 'passportNo' });

// R19
test('read_answer_keeps_the_sentence', () => {
  assert.deepEqual(D().readAnswer(400, NAMED), { status: 400, json: true, error: BAD_TEXT });
});

// R20
test('customer_error_text_is_the_sentence', () => {
  const observed = D().readAnswer(400, NAMED);
  assert.equal(D().customerErrorText(true, observed), BAD_TEXT);
  assert.notEqual(D().customerErrorText(true, observed), D().CUSTOMER_UNREACHED);
});

// R21
test('save_outcome_is_refused', () => {
  assert.equal(D().saveOutcome(D().readAnswer(400, NAMED)), 'refused');
});
