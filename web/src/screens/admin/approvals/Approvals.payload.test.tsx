// FE-T3a #5853: the approval card shows what is being approved, and never prints a bare id as text.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/fe-triage-1101.md` W13 (PR #5848): TRANSACTION_REVERSAL shows the amount (THB)
// and the reason from the payload; COUNT_DIFFERENCE shows the net difference (THB); ids move to `title`.
// Payload shapes on main `a174fb95`:
// - TRANSACTION_REVERSAL: `api/src/application/workflows/exchange/reversal_request.rs:147-151`
//   `{ "originalTransactionId": "<uuid>", "reason": "<text>", "amountThb": "<decimal string>" }`.
// - COUNT_DIFFERENCE: `api/crates/xc-cash/src/application/ledger_approval/count_difference.rs:236-239`
//   `{ "count_id": "<uuid>", "net_difference_thb": "<decimal string>" }`.
// Money is shown as `formatMoney(value) + " THB"` (`web/src/shared/ui/MoneyText.tsx`).
// One test per row of the ticket's behaviour table; the row id is in the test name.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { Approvals } from './Approvals';
import { Toast } from '../parts/toast';
import { makeApprovalDecisionDto, makeApprovalRequestItemDto } from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';

// The ticket's wording (§7 of #5853). Each is a new key of `./i18n.ts`.
const LABELS = {
  th: {
    originalTransaction: 'รายการเดิม: ',
    reversalAmount: 'ยอดเงิน: ',
    requestReason: 'เหตุผลที่ขอ: ',
    netDifference: 'ผลต่างสุทธิ: ',
  },
  en: {
    originalTransaction: 'Original transaction: ',
    reversalAmount: 'Amount: ',
    requestReason: 'Request reason: ',
    netDifference: 'Net difference: ',
  },
} as const;
type LabelKey = keyof typeof LABELS.th;

const PROPOSER = '43839561-1111-4000-8000-000000000001';
const APPROVER = '9a0b1c2d-2222-4000-8000-000000000002';
const ORIGINAL = 'c0ffee12-3456-4789-8abc-def012345678';
const COUNT_ID = 'abcd1234-5555-4000-8000-000000000003';
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function makeRequest(id: string, type: string, proposedPayload: unknown, status = 'PENDING', decision = null as ReturnType<typeof makeApprovalDecisionDto> | null) {
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
    status,
    decision,
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

describe('FE-T3a #5853: the approval card shows the payload (design W13)', () => {
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

  it('L1: the four new keys exist in th and en with the ticket wording', () => {
    for (const lang of ['th', 'en'] as const) {
      for (const key of Object.keys(LABELS.th) as LabelKey[]) label(key, lang);
    }
  });

  it('R1: TRANSACTION_REVERSAL shows the amount in THB and the reason (th)', async () => {
    byStatus.PENDING = [makeRequest('rev-1', 'TRANSACTION_REVERSAL', reversalPayload)];
    mount();
    const card = await cardOf('rev-1');
    const t = text(card);
    expect(t).toContain(`${label('reversalAmount').trim()} 12,500.00 THB`);
    expect(t).toContain(`${label('requestReason').trim()} ลูกค้าเปลี่ยนใจ`);
  });

  it('R2: TRANSACTION_REVERSAL shows the original transaction as #<first 8> with the full id as title', async () => {
    byStatus.PENDING = [makeRequest('rev-2', 'TRANSACTION_REVERSAL', reversalPayload)];
    mount();
    const card = await cardOf('rev-2');
    const short = card.querySelector(`[title="${ORIGINAL}"]`);
    expect(short, 'the original transaction id is a title').not.toBeNull();
    expect(short!.textContent).toBe('#c0ffee12');
    expect(text(card)).toContain(`${label('originalTransaction').trim()} #c0ffee12`);
  });

  it('R3: the reversal lines follow setLang (en)', async () => {
    byStatus.PENDING = [makeRequest('rev-3', 'TRANSACTION_REVERSAL', { ...reversalPayload, reason: 'Customer changed mind' })];
    mount();
    const card = await cardOf('rev-3');
    setLang('en');
    await vi.waitFor(() => {
      const t = text(card);
      expect(t).toContain(`${LABELS.en.reversalAmount.trim()} 12,500.00 THB`);
      expect(t).toContain(`${LABELS.en.requestReason.trim()} Customer changed mind`);
      expect(t).toContain(`${LABELS.en.originalTransaction.trim()} #c0ffee12`);
    });
  });

  it('R4: a payload sent as a JSON string is read the same way', async () => {
    byStatus.PENDING = [makeRequest('rev-4', 'TRANSACTION_REVERSAL', JSON.stringify(reversalPayload))];
    mount();
    const card = await cardOf('rev-4');
    expect(text(card)).toContain(`${label('reversalAmount').trim()} 12,500.00 THB`);
  });

  it('C1: COUNT_DIFFERENCE shows the net difference in THB, sign kept', async () => {
    byStatus.PENDING = [
      makeRequest('cnt-1', 'COUNT_DIFFERENCE', { count_id: COUNT_ID, net_difference_thb: '-1350.50' }),
    ];
    mount();
    const card = await cardOf('cnt-1');
    expect(text(card)).toContain(`${label('netDifference').trim()} -1,350.50 THB`);
    setLang('en');
    await vi.waitFor(() => {
      expect(text(card)).toContain(`${LABELS.en.netDifference.trim()} -1,350.50 THB`);
    });
  });

  it('C2: COUNT_DIFFERENCE never shows the count id as text', async () => {
    byStatus.PENDING = [
      makeRequest('cnt-2', 'COUNT_DIFFERENCE', { count_id: COUNT_ID, net_difference_thb: '200.00' }),
    ];
    mount();
    const card = await cardOf('cnt-2');
    expect(text(card)).not.toContain(COUNT_ID);
  });

  it('U1: no card prints a bare UUID as text; the proposer is the title of an element', async () => {
    byStatus.PENDING = [
      makeRequest('u-rev', 'TRANSACTION_REVERSAL', reversalPayload),
      makeRequest('u-cnt', 'COUNT_DIFFERENCE', { count_id: COUNT_ID, net_difference_thb: '200.00' }),
      makeRequest('u-cash', 'CASH_TRANSFER', {
        transfer_id: '74f7ecfe-cc4a-4b94-aac2-7b32f08ff33e',
        from_branch: 'MAIN',
        within_limit: true,
      }),
      makeRequest('u-rate', 'RATE_PUBLICATION', { out_of_bound: [{ currency: 'USD', side: 'SELL' }] }),
    ];
    mount();
    for (const id of ['u-rev', 'u-cnt', 'u-cash', 'u-rate']) {
      const card = await cardOf(id);
      expect(text(card), `card ${id} text`).not.toMatch(UUID);
      expect(card.querySelector(`[title="${PROPOSER}"]`), `card ${id}: the proposer is a title`).not.toBeNull();
    }
  });

  it('U2: a decided card keeps the approver id as a title, not as text', async () => {
    byStatus.APPROVED = [
      makeRequest(
        'dec-1',
        'TRANSACTION_REVERSAL',
        reversalPayload,
        'APPROVED',
        makeApprovalDecisionDto({ approverId: APPROVER, outcome: 'APPROVED', reason: 'ok', at: '2026-11-01T04:00:00Z' }),
      ),
    ];
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-status-filter="APPROVED"]')).not.toBeNull();
    });
    (root.querySelector('[data-status-filter="APPROVED"]') as HTMLButtonElement).click();
    const card = await cardOf('dec-1');
    const block = card.querySelector('[data-decision="dec-1"]');
    expect(block).not.toBeNull();
    expect(block!.querySelector(`[title="${APPROVER}"]`), 'the approver is a title').not.toBeNull();
    expect(text(card)).not.toMatch(UUID);
    expect(text(card)).toContain(`${label('reversalAmount').trim()} 12,500.00 THB`);
  });

  it('E1: a reversal payload without a usable amount shows no amount line and no undefined/NaN', async () => {
    byStatus.PENDING = [
      makeRequest('bad-1', 'TRANSACTION_REVERSAL', { originalTransactionId: ORIGINAL, reason: 'x', amountThb: 12 }),
      makeRequest('bad-2', 'TRANSACTION_REVERSAL', { reason: 'only a reason' }),
      makeRequest('bad-3', 'TRANSACTION_REVERSAL', null),
      makeRequest('bad-4', 'TRANSACTION_REVERSAL', 'not json {'),
    ];
    mount();
    for (const id of ['bad-1', 'bad-2', 'bad-3', 'bad-4']) {
      const t = text(await cardOf(id));
      expect(t, `card ${id}`).not.toContain(label('reversalAmount').trim());
      expect(t, `card ${id}`).not.toContain('undefined');
      expect(t, `card ${id}`).not.toContain('NaN');
      expect(t, `card ${id}`).not.toContain('null');
    }
    expect(text(await cardOf('bad-2'))).toContain(`${label('requestReason').trim()} only a reason`);
    expect(text(await cardOf('bad-2'))).not.toContain(label('originalTransaction').trim());
  });

  it('E2: a count difference payload without a usable figure shows no net-difference line', async () => {
    byStatus.PENDING = [
      makeRequest('cbad-1', 'COUNT_DIFFERENCE', { count_id: COUNT_ID }),
      makeRequest('cbad-2', 'COUNT_DIFFERENCE', { count_id: COUNT_ID, net_difference_thb: 5 }),
      makeRequest('cbad-3', 'COUNT_DIFFERENCE', {}),
    ];
    mount();
    for (const id of ['cbad-1', 'cbad-2', 'cbad-3']) {
      const t = text(await cardOf(id));
      expect(t, `card ${id}`).not.toContain(label('netDifference').trim());
      expect(t, `card ${id}`).not.toContain('undefined');
      expect(t, `card ${id}`).not.toMatch(UUID);
    }
  });

  it('E3: the reversal and count lines appear only on their own type', async () => {
    byStatus.PENDING = [
      makeRequest('x-cash', 'CASH_TRANSFER', { ...reversalPayload, net_difference_thb: '1.00' }),
      makeRequest('x-cnt', 'COUNT_DIFFERENCE', { ...reversalPayload, net_difference_thb: '1.00' }),
      makeRequest('x-rev', 'TRANSACTION_REVERSAL', { ...reversalPayload, net_difference_thb: '1.00' }),
    ];
    mount();
    const cash = text(await cardOf('x-cash'));
    expect(cash).not.toContain(label('reversalAmount').trim());
    expect(cash).not.toContain(label('netDifference').trim());
    const cnt = text(await cardOf('x-cnt'));
    expect(cnt).toContain(label('netDifference').trim());
    expect(cnt).not.toContain(label('reversalAmount').trim());
    const rev = text(await cardOf('x-rev'));
    expect(rev).toContain(label('reversalAmount').trim());
    expect(rev).not.toContain(label('netDifference').trim());
  });
});
