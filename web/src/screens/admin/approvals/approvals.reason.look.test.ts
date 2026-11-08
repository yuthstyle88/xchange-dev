// OL-7 admin-approvals-reason (verifier row 80, VERIFY.md: "เหตุผลการตัดสินใจ is still a single-line ~44px input for 500
// chars"). The decision reason is a 3-line textarea (admin.css already sizes every admin textarea, min-height 88px), still
// capped at 500 characters with the "n / 500" counter under it. The six screen tests that find the reason field now look
// for `textarea` (changed with this test by its author, claude/94389a6b, for the sub-controller claude/1cc51615).
// Restated by the sub-controller for OL-8 reuse-field-count: the counter class is the shared xc-field-count.
// Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let tsx = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  tsx = fs.readFileSync(here + 'Approvals.tsx', 'utf8');
});

/** The JSX of the decision form, from its opening div to the counter. */
function decisionForm(): string {
  const at = tsx.indexOf('class="approval-decision-form"');
  expect(at, 'decision form present').toBeGreaterThanOrEqual(0);
  const end = tsx.indexOf('xc-field-count', at);
  expect(end, 'counter after the field').toBeGreaterThan(at);
  return tsx.slice(at, end);
}

describe('approval decision reason field', () => {
  it('AR-1 the reason is a textarea, not a one-line text input', () => {
    const form = decisionForm();
    expect(form).toContain('<textarea');
    expect(form).not.toMatch(/<input[^>]*type="text"/);
  });

  it('AR-2 the textarea keeps the 500-character cap and shows 3 lines', () => {
    const form = decisionForm();
    const open = form.slice(form.indexOf('<textarea'), form.indexOf('/>', form.indexOf('<textarea')));
    expect(open).toMatch(/maxLength=\{500\}/);
    expect(open).toMatch(/rows=\{3\}/);
  });

  it('AR-3 unchanged: the counter still reads "{length} / 500"', () => {
    expect(tsx).toContain('<div class="xc-field-count">{reason().length} / 500</div>');
  });
});
