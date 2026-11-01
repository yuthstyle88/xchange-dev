// FE-T3b (fe-triage-1101 T3, design W13): the approval card names the requester (`requestedByName`) and, when the
// server sends a `subject` (#5865), reads the receipt, amount, reason, customer and counter from it. Ids appear only
// in `title`. A request with `subject: null` keeps today's payload rendering (pinned by Approvals.payload.test.tsx).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { Approvals } from './Approvals';
import { Toast } from '../parts/toast';
import {
  makeApprovalRequestItemDto,
  makeCountDifferenceSubject,
  makeReversalSubject,
} from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';

// The ticket's wording (§7 of FE-T3b). Each is a new key of `./i18n.ts`.
const LABELS = {
  th: {
    receiptNumber: 'ใบเสร็จ: ',
    customer: 'ลูกค้า: ',
    countedBy: 'ผู้นับ: ',
  },
  en: {
    receiptNumber: 'Receipt: ',
    customer: 'Customer: ',
    countedBy: 'Counted by: ',
  },
} as const;
type LabelKey = keyof typeof LABELS.th;

const PROPOSER = '43839561-1111-4000-8000-000000000001';
const ORIGINAL = 'c0ffee12-3456-4789-8abc-def012345678';
const COUNT_ID = 'abcd1234-5555-4000-8000-000000000003';
type Subject = ReturnType<typeof makeApprovalRequestItemDto>['subject'];

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

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

describe('FE-T3b: the approval card reads the subject and names people (design W13)', () => {
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

  function label(key: LabelKey, lang: 'th' | 'en' = 'th'): string {
    const value = (table[lang] as Record<string, string>)[key];
    expect(value, `admin.approvals i18n ${lang}.${key}`).toBe(LABELS[lang][key]);
    return value;
  }

  it('L1: the three new keys exist in th and en with the ticket wording', () => {
    for (const lang of ['th', 'en'] as const) {
      for (const key of Object.keys(LABELS.th) as LabelKey[]) label(key, lang);
    }
  });

  it('S1: the requester shows requestedByName, with requestedBy only in its title', async () => {
    byStatus.PENDING = [makeRequest('cash-1', 'CASH_TRANSFER', {}, { requestedByName: 'Ploy Manager' })];
    mount();
    const card = await cardOf('cash-1');
    const who = card.querySelector(`strong[title="${PROPOSER}"]`);
    expect(who, 'requester element').not.toBeNull();
    expect(text(who)).toBe('Ploy Manager');
    expect(text(card)).not.toMatch(UUID);
  });

  it('S2: no requestedByName keeps "—", with requestedBy in its title', async () => {
    byStatus.PENDING = [makeRequest('cash-2', 'CASH_TRANSFER', {}, { requestedByName: null })];
    mount();
    const card = await cardOf('cash-2');
    expect(text(card.querySelector(`strong[title="${PROPOSER}"]`))).toBe('—');
  });

  it('S3: a reversal subject shows receipt, amount, reason and customer from the subject; ids only in titles', async () => {
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
      }),
    };
    // The payload disagrees on purpose: the card must read the subject, not the payload.
    byStatus.PENDING = [
      makeRequest('rev-s', 'TRANSACTION_REVERSAL', reversalPayload, { requestedByName: 'Somchai Teller', subject }),
    ];
    mount();
    const card = await cardOf('rev-s');
    const t = text(card);
    expect(t).toContain(`${label('receiptNumber').trim()} R261101-0007`);
    expect(t).toContain(`${(table.th as Record<string, string>).reversalAmount!.trim()} 3,500.50 THB`);
    expect(t).toContain(`${(table.th as Record<string, string>).requestReason!.trim()} ทอนเงินผิด`);
    expect(t).toContain(`${label('customer').trim()} สมหญิง ใจดี (CX-00000040)`);
    expect(t).not.toContain('12,500.00');
    expect(t).not.toMatch(UUID);
    expect(t).not.toContain('#c0ffee12');
    expect(card.querySelector(`[title="${ORIGINAL}"]`), 'original transaction id in a title').not.toBeNull();
  });

  it('S4: a reversal subject with no customer shows no customer line', async () => {
    const subject: Subject = {
      kind: 'reversal',
      ...makeReversalSubject({ receiptNumber: 'R261101-0008', amountThb: '100.00', reason: 'x', customerName: null, customerNo: null }),
    };
    byStatus.PENDING = [makeRequest('rev-n', 'TRANSACTION_REVERSAL', reversalPayload, { subject })];
    mount();
    const t = text(await cardOf('rev-n'));
    expect(t).toContain(`${label('receiptNumber').trim()} R261101-0008`);
    expect(t).not.toContain(label('customer').trim());
  });

  it('S5: a count-difference subject names the counter and reads the net difference from the subject', async () => {
    const subject: Subject = {
      kind: 'countDifference',
      ...makeCountDifferenceSubject({
        countId: COUNT_ID,
        countKind: 'CLOSE',
        countedBy: 'bbbb2222-0000-4000-8000-000000000002',
        countedByName: 'Somchai Teller',
        netDifferenceThb: '-20.00',
      }),
    };
    byStatus.PENDING = [
      makeRequest('cnt-s', 'COUNT_DIFFERENCE', { count_id: COUNT_ID, net_difference_thb: '-999.00' }, { subject }),
    ];
    mount();
    const card = await cardOf('cnt-s');
    const t = text(card);
    expect(t).toContain(`${label('countedBy').trim()} Somchai Teller`);
    expect(t).toContain(`${(table.th as Record<string, string>).netDifference!.trim()} -20.00 THB`);
    expect(t).not.toContain('999');
    expect(t).not.toMatch(UUID);
    expect(card.querySelector('[title="bbbb2222-0000-4000-8000-000000000002"]'), 'counter id in a title').not.toBeNull();
  });

  it('S6: a count-difference subject with no counter name shows "—" for the counter', async () => {
    const subject: Subject = {
      kind: 'countDifference',
      ...makeCountDifferenceSubject({ countId: COUNT_ID, countedByName: null, netDifferenceThb: '5.00' }),
    };
    byStatus.PENDING = [makeRequest('cnt-n', 'COUNT_DIFFERENCE', { count_id: COUNT_ID, net_difference_thb: '5.00' }, { subject })];
    mount();
    expect(text(await cardOf('cnt-n'))).toContain(`${label('countedBy').trim()} —`);
  });
});
