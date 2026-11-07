// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { warmModules } from '../../../../shared/testing/warm-modules';
import { makeReversalPendingRow, makeReversalSubject, type ReversalPendingRow } from '../../../../shared/testing/fixtures';
import { ReversalsPart } from './ReversalsPart';

beforeAll(() => warmModules(() => import('./ReversalsPart')), 60_000);

const REQUEST_ID = '5e7a9c31-1111-4000-8000-000000000001';

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('FE-T3c: reversal row shows the shift of the reversed sale', () => {
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

  it('B1: shiftOpenedAt set shows shift stamp right before createdAt stamp in th and en', () => {
    const shiftOpenedAt = '2026-11-01T02:00:00Z';
    const createdAt = '2026-11-01T05:44:25Z';
    mount([
      makeReversalPendingRow({
        requestId: REQUEST_ID,
        createdAt,
        subject: makeReversalSubject({
          receiptNumber: 'R261101-0007',
          amountThb: '3500.50',
          reason: 'ทอนเงินผิด',
          requestedByName: 'Somchai Teller',
          customerName: 'สมหญิง ใจดี',
          shiftOpenedAt,
        }),
      }),
    ]);

    const s = summary();
    expect(s, 'summary element').not.toBeNull();
    const tTh = text(s);
    // F55: pinned (was rebuilt with the screen's own stamp()); 02:00Z = 09:00, 05:44:25Z = 12:44 in Bangkok.
    const expectedShiftTh = 'กะ 01/11/2026 09:00';
    const expectedCreatedTh = '01/11/2026 12:44';
    expect(tTh).toContain(`${expectedShiftTh} · ${expectedCreatedTh}`);
    expect(tTh.endsWith(expectedCreatedTh)).toBe(true);

    // Test en
    dispose?.();
    setLang('en');
    mount([
      makeReversalPendingRow({
        requestId: REQUEST_ID,
        createdAt,
        subject: makeReversalSubject({
          receiptNumber: 'R261101-0007',
          amountThb: '3500.50',
          reason: 'ทอนเงินผิด',
          requestedByName: 'Somchai Teller',
          customerName: 'สมหญิง ใจดี',
          shiftOpenedAt,
        }),
      }),
    ]);
    const tEn = text(summary());
    const expectedShiftEn = 'Shift 01/11/2026 09:00';
    const expectedCreatedEn = '01/11/2026 12:44';
    expect(tEn).toContain(`${expectedShiftEn} · ${expectedCreatedEn}`);
    expect(tEn.endsWith(expectedCreatedEn)).toBe(true);
  });

  it('B2: shiftOpenedAt null shows no shift segment, no null, no empty separator', () => {
    mount([
      makeReversalPendingRow({
        requestId: REQUEST_ID,
        createdAt: '2026-11-01T05:44:25Z',
        subject: makeReversalSubject({
          receiptNumber: 'R261101-0008',
          amountThb: '250.00',
          reason: 'ลูกค้าเปลี่ยนใจ',
          shiftOpenedAt: null,
        }),
      }),
    ]);
    const t = text(summary());
    expect(t).not.toContain('กะ ');
    expect(t).not.toContain('Shift ');
    expect(t).not.toContain('null');
    expect(t).not.toMatch(/·\s*·/);
  });
});
