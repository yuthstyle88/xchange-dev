import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { locale, setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { Approvals } from './Approvals';
import { Toast } from '../parts/toast';
import {
  makeApprovalRequestItemDto,
  makeCountDifferenceSubject,
  makeReversalSubject,
} from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';
const PROPOSER = '43839561-1111-4000-8000-000000000001';
const ORIGINAL = 'c0ffee12-3456-4789-8abc-def012345678';
const SHIFT_ID = '98765432-1111-4000-8000-000000000099';
const SHIFT_OPENED_AT = '2026-11-01T02:00:00Z';

type Subject = ReturnType<typeof makeApprovalRequestItemDto>['subject'];

function formatBangkok(iso: string, loc: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(loc, { timeZone: 'Asia/Bangkok' });
}

function makeRequest(
  id: string,
  type: string,
  proposedPayload: unknown,
  extra: { requestedByName?: string | null; subject?: Subject } = {},
) {
  return makeApprovalRequestItemDto({
    id,
    type,
    branchId: 'MAIN',
    objectId: `obj-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-1',
    proposedPayload,
    requestedBy: PROPOSER,
    requestedAt: '2026-11-01T03:00:00Z',
    status: 'PENDING',
    decision: null,
    ...extra,
  });
}

const reversalPayload = {
  originalTransactionId: ORIGINAL,
  reason: 'ลูกค้าเปลี่ยนใจ',
  amountThb: '12500.00',
};

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('FE-T3c: approval card shows the shift of the reversed sale', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let byStatus: Record<string, ReturnType<typeof makeApprovalRequestItemDto>[]>;

  beforeEach(() => {
    setLang('th');
    byStatus = { PENDING: [], APPROVED: [], REJECTED: [] };
    root = document.createElement('div');
    document.body.appendChild(root);
    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          const status = url.searchParams.get('status') ?? 'PENDING';
          return Promise.resolve(
            new Response(
              JSON.stringify({ version: 1, changed: true, data: { requests: structuredClone(byStatus[status] ?? []) } }),
              { status: 200, headers: { 'content-type': 'application/json' } },
            ),
          );
        }
        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-11-01', to: '2026-11-01' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <Approvals branch={branch} period={period} shown={shown} />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
    return root;
  }

  async function cardOf(id: string): Promise<HTMLElement> {
    await vi.waitFor(() => {
      expect(root.querySelector(`[data-request="${id}"]`), `card ${id}`).not.toBeNull();
    });
    return root.querySelector(`[data-request="${id}"]`) as HTMLElement;
  }

  it('i18n: shift key exists in th and en with the ticket wording', () => {
    expect((table.th as Record<string, string>).shift).toBe('กะที่เปิด: ');
    expect((table.en as Record<string, string>).shift).toBe('Shift opened: ');
  });

  it('B3: TRANSACTION_REVERSAL approval card with shiftOpenedAt renders shift row after customer row', async () => {
    const subject: Subject = {
      kind: 'reversal',
      ...makeReversalSubject({
        reversalId: 'aaaa1111-0000-4000-8000-000000000001',
        originalTransactionId: ORIGINAL,
        receiptNumber: 'R261101-0007',
        amountThb: '3500.50',
        reason: 'ทอนเงินผิด',
        customerName: 'สมหญิง ใจดี',
        customerNo: 'CX-00000040',
        requestedByName: 'Somchai Teller',
        shiftId: SHIFT_ID,
        shiftOpenedAt: SHIFT_OPENED_AT,
      }),
    };
    byStatus.PENDING = [makeRequest('rev-shift', 'TRANSACTION_REVERSAL', reversalPayload, { subject })];
    mount();

    const card = await cardOf('rev-shift');
    const proposers = Array.from(card.querySelectorAll('.approval-proposer'));
    const customerIdx = proposers.findIndex((el) => text(el).includes('ลูกค้า:'));
    expect(customerIdx, 'customer row exists').toBeGreaterThan(-1);

    const shiftRow = proposers[customerIdx + 1];
    expect(shiftRow, 'shift row directly follows customer row').toBeDefined();

    const spans = shiftRow.querySelectorAll('span');
    expect(spans.length).toBe(2);
    expect(text(spans[0])).toBe('กะที่เปิด:');
    expect(spans[1].getAttribute('title')).toBe(SHIFT_ID);
    expect(text(spans[1])).toBe(formatBangkok(SHIFT_OPENED_AT, locale()));

    // Test with en
    dispose?.();
    setLang('en');
    mount();
    const cardEn = await cardOf('rev-shift');
    const proposersEn = Array.from(cardEn.querySelectorAll('.approval-proposer'));
    const customerIdxEn = proposersEn.findIndex((el) => text(el).includes('Customer:'));
    expect(customerIdxEn, 'customer row in en').toBeGreaterThan(-1);
    const shiftRowEn = proposersEn[customerIdxEn + 1];
    expect(shiftRowEn, 'shift row directly follows customer row in en').toBeDefined();
    const spansEn = shiftRowEn.querySelectorAll('span');
    expect(text(spansEn[0])).toBe('Shift opened:');
    expect(spansEn[1].getAttribute('title')).toBe(SHIFT_ID);
    expect(text(spansEn[1])).toBe(formatBangkok(SHIFT_OPENED_AT, locale()));
  });

  it('B4: subject with shiftOpenedAt: null renders no shift row', async () => {
    const subject: Subject = {
      kind: 'reversal',
      ...makeReversalSubject({
        receiptNumber: 'R261101-0008',
        amountThb: '100.00',
        reason: 'x',
        shiftOpenedAt: null,
        shiftId: null,
      }),
    };
    byStatus.PENDING = [makeRequest('rev-no-shift', 'TRANSACTION_REVERSAL', reversalPayload, { subject })];
    mount();

    const card = await cardOf('rev-no-shift');
    const t = text(card);
    expect(t).not.toContain('กะที่เปิด:');
    expect(t).not.toContain('Shift opened:');
    for (const el of Array.from(card.querySelectorAll('.approval-proposer'))) {
      expect(text(el)).not.toContain('กะที่เปิด:');
    }
  });

  it('B5: legacy payload with subject: null renders no shift row', async () => {
    byStatus.PENDING = [makeRequest('rev-legacy', 'TRANSACTION_REVERSAL', reversalPayload, { subject: null })];
    mount();

    const card = await cardOf('rev-legacy');
    const t = text(card);
    expect(t).not.toContain('กะที่เปิด:');
    expect(t).not.toContain('Shift opened:');
    for (const el of Array.from(card.querySelectorAll('.approval-proposer'))) {
      expect(text(el)).not.toContain('กะที่เปิด:');
    }
  });

  it('edge case: shiftOpenedAt set and shiftId null shows formatted time with title unset', async () => {
    const subject: Subject = {
      kind: 'reversal',
      ...makeReversalSubject({
        reversalId: 'aaaa1111-0000-4000-8000-000000000002',
        originalTransactionId: ORIGINAL,
        receiptNumber: 'R261101-0009',
        amountThb: '500.00',
        reason: 'ทอนเงินผิด',
        shiftOpenedAt: SHIFT_OPENED_AT,
        shiftId: null,
      }),
    };
    byStatus.PENDING = [makeRequest('rev-null-shift-id', 'TRANSACTION_REVERSAL', reversalPayload, { subject })];
    mount();

    const card = await cardOf('rev-null-shift-id');
    const proposers = Array.from(card.querySelectorAll('.approval-proposer'));
    const shiftRow = proposers.find((el) => text(el).includes('กะที่เปิด:'));
    expect(shiftRow, 'shift row found').toBeDefined();
    const spans = shiftRow!.querySelectorAll('span');
    expect(text(spans[0])).toBe('กะที่เปิด:');
    expect(spans[1].hasAttribute('title')).toBe(false);
    expect(spans[1].getAttribute('title')).toBeNull();
    expect(text(spans[1])).toBe(formatBangkok(SHIFT_OPENED_AT, locale()));
  });

  it('B6: COUNT_DIFFERENCE card shows the shift row between counted-by and net difference; null shows none', async () => {
    const count = (shiftId: string | null, shiftOpenedAt: string | null): Subject => ({ kind: 'countDifference', ...makeCountDifferenceSubject({ countedByName: 'สมหญิง', shiftId, shiftOpenedAt }) });
    byStatus.PENDING = [
      makeRequest('count-shift', 'COUNT_DIFFERENCE', {}, { subject: count(SHIFT_ID, SHIFT_OPENED_AT) }),
      makeRequest('count-no-shift', 'COUNT_DIFFERENCE', {}, { subject: count(null, null) }),
    ];
    mount();
    const rows = Array.from((await cardOf('count-shift')).querySelectorAll('.approval-proposer'));
    const at = (p: string) => rows.findIndex((el) => text(el).startsWith(p));
    expect([at('ผู้นับ:') + 1, at('กะที่เปิด:') + 1]).toEqual([at('กะที่เปิด:'), at('ผลต่างสุทธิ:')]);
    const value = rows[at('กะที่เปิด:')]!.querySelectorAll('span')[1]!;
    expect([value.getAttribute('title'), text(value)]).toEqual([SHIFT_ID, formatBangkok(SHIFT_OPENED_AT, locale())]);
    expect(text(await cardOf('count-no-shift'))).not.toContain('กะที่เปิด:');
  });
});
