// FE-T3b (fe-triage-1101 T3, design W13): a pending reversal on the sheet reads its `subject` (#5865) and never prints
// the request id as text. Up-front tests by the FE sub-controller claude/01TxaRJG. Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { warmModules } from '../../../../shared/testing/warm-modules';
import { makeReversalPendingRow, makeReversalSubject, type ReversalPendingRow } from '../../../../shared/testing/fixtures';
import { ReversalsPart } from './ReversalsPart';

beforeAll(() => warmModules(() => import('./ReversalsPart')), 60_000);

const REQUEST_ID = '5e7a9c31-1111-4000-8000-000000000001';
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('FE-T3b: a pending reversal row reads its subject', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response('{}', { status: 200 }))));
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.unstubAllGlobals();
  });

  function mount(rows: ReversalPendingRow[]) {
    dispose = render(() => <ReversalsPart rows={rows} branch="MAIN" onDecided={() => Promise.resolve()} />, root);
  }

  const summary = () => root.querySelector(`[data-request="${REQUEST_ID}"] [data-summary]`);

  it('V1: the summary shows receipt, amount in THB, reason, teller and customer, joined by " · "', () => {
    mount([
      makeReversalPendingRow({
        requestId: REQUEST_ID,
        createdAt: '2026-11-01T05:44:25Z',
        subject: makeReversalSubject({
          receiptNumber: 'R261101-0007',
          amountThb: '3500.50',
          reason: 'ทอนเงินผิด',
          requestedByName: 'Somchai Teller',
          customerName: 'สมหญิง ใจดี',
        }),
      }),
    ]);
    const s = summary();
    expect(s, 'summary element').not.toBeNull();
    const t = text(s);
    expect(t.startsWith('R261101-0007 · 3,500.50 THB · ทอนเงินผิด · Somchai Teller · สมหญิง ใจดี · ')).toBe(true);
  });

  it('V2: the request id is never printed as text; it stays on data-request and in the summary title', () => {
    mount([
      makeReversalPendingRow({
        requestId: REQUEST_ID,
        subject: makeReversalSubject({ receiptNumber: 'R261101-0008', amountThb: '1.00', reason: 'x' }),
      }),
    ]);
    const row = root.querySelector(`[data-request="${REQUEST_ID}"]`);
    expect(row, 'row').not.toBeNull();
    expect(text(row)).not.toMatch(UUID);
    expect(summary()!.getAttribute('title')).toBe(REQUEST_ID);
  });

  it('V3: a missing teller or customer name leaves no empty segment and no "null"', () => {
    mount([
      makeReversalPendingRow({
        requestId: REQUEST_ID,
        subject: makeReversalSubject({
          receiptNumber: 'R261101-0009',
          amountThb: '250.00',
          reason: 'ลูกค้าเปลี่ยนใจ',
          requestedByName: null,
          customerName: null,
        }),
      }),
    ]);
    const t = text(summary());
    expect(t.startsWith('R261101-0009 · 250.00 THB · ลูกค้าเปลี่ยนใจ · ')).toBe(true);
    expect(t).not.toContain('null');
    expect(t).not.toMatch(/·\s*·/);
  });
});
