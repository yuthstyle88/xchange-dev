// OR-4 (#550, docs/design/blueprint/org-roles-branches.md §4.3): the Blueprint policy fixture is the
// output of api/tools/policy-map.mjs over v2's fixture, byte for byte. Written by the ticket author.
// Do not edit, delete or weaken.
//
// Run from the repository root:   node --test api/tests/js/*.test.mjs
//
// Which outcomes the mapping may change is asserted on the Rust side, independently of this script
// (`api/crates/xc-auth/tests/policy_test.rs` `the_mapping_changes_exactly_c1_to_c9`); here the
// committed file is held to the script, so neither can drift from the other.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const TOOL = new URL('../../tools/policy-map.mjs', import.meta.url);
const FIXTURES = new URL('../../crates/xc-auth/tests/fixtures/', import.meta.url);
const read = (name) => fs.readFileSync(fileURLToPath(new URL(name, FIXTURES)), 'utf8');

test('the committed Blueprint fixture is the output of policy-map.mjs', async () => {
  const { render } = await import(TOOL);
  const v2 = JSON.parse(read('v2_policy_cases.json'));
  assert.equal(read('blueprint_policy_cases.json'), render(v2));
});

test('the mapping keeps every case, in v2 order', async () => {
  const { build } = await import(TOOL);
  const v2 = JSON.parse(read('v2_policy_cases.json'));
  const m = build(v2);
  assert.equal(m.cases.length, 1425);
  assert.equal(m.branchCases.length, 285);
  m.cases.forEach((c, i) => {
    const v = v2.cases[i];
    assert.deepEqual([c.path, c.route, c.actor, c.action], [v.path, v.route, v.actor, v.action], `case ${i}`);
  });
});

test('running the script twice gives the same bytes', async () => {
  const { render } = await import(TOOL);
  const v2 = JSON.parse(read('v2_policy_cases.json'));
  assert.equal(render(v2), render(JSON.parse(JSON.stringify(v2))));
});
