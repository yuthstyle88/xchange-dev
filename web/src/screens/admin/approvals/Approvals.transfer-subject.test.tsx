// F48 + F54 (docs/design/backdated-keying.md §16.2): a CASH_TRANSFER approval card reads the request's `subject`
// (`kind: 'cashTransfer'`) and shows each currency's total, the transfer's note and the branch's name, so the approver
// does not decide on THB 150,000 blind. Without a subject the card is as #5314 left it (Approvals.cash-transfer.test.tsx).
// Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own. The harness is copied from Approvals.cash-transfer.test.tsx.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Approvals } from './Approvals';
import { Toast } from '../parts/toast';
import {
  makeApprovalRequestItemDto,
} from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';

function createTransferRequest(
  id: string,
  proposedPayload: any,
  type = 'CASH_TRANSFER',
  status = 'PENDING',
) {
  return makeApprovalRequestItemDto({
    id,
    type,
    branchId: 'MAIN',
    objectId: `obj-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-1',
    proposedPayload,
    requestedBy: '43839561-1111-4000-8000-000000000001',
    requestedAt: '2026-10-28T03:00:00Z',
    status,
    decision: null,
  });
}

describe('F48 + F54: the cash-transfer card shows the figure, the note and the branch name', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let currentRequests: ReturnType<typeof makeApprovalRequestItemDto>[];

  beforeEach(() => {
    setLang('th');
    currentRequests = [];

    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { requests: structuredClone(currentRequests) },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
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
    const [period] = createSignal({ from: '2026-10-28', to: '2026-10-28' });
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

  const FULL_ID = '74f7ecfe-cc4a-4b94-aac2-7b32f08ff33e';
  const PAYLOAD = { transfer_id: FULL_ID, from_branch: 'MAIN', within_limit: false };

  function subject(over: Record<string, unknown> = {}) {
    return {
      kind: 'cashTransfer',
      transferId: FULL_ID,
      fromBranchId: 'MAIN',
      fromBranchName: 'XC-B',
      toBranchName: null,
      note: 'ยอดสมมติ — เติมเงินต้นวัน',
      lines: [
        { currency: 'THB', amount: '150000.00' },
        { currency: 'USD', amount: '520.00' },
      ],
      ...over,
    };
  }

  async function card(id: string): Promise<Element> {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector(`[data-request="${id}"]`)).not.toBeNull();
    });
    return root.querySelector(`[data-request="${id}"]`)!;
  }

  it('shows the branch name from the subject, not the branch id', async () => {
    currentRequests = [{ ...createTransferRequest('req-s1', PAYLOAD), subject: subject() as never }];
    const c = await card('req-s1');
    expect(c.textContent).toContain('#74f7ecfe · XC-B · เกินวงเงิน — ผู้บริหารตัดสิน');
    expect(c.textContent).not.toContain('· MAIN ·');
  });

  it('shows every currency total, grouped, in the order the server sent', async () => {
    currentRequests = [{ ...createTransferRequest('req-s2', PAYLOAD), subject: subject() as never }];
    const c = await card('req-s2');
    const amounts = c.querySelector('[data-testid="transfer-amounts"]');
    expect(amounts, 'the totals line').not.toBeNull();
    expect((amounts!.textContent ?? '').replace(/\s+/g, ' ').trim()).toBe('THB 150,000.00 · USD 520.00');
  });

  it("shows the transfer's note", async () => {
    currentRequests = [{ ...createTransferRequest('req-s3', PAYLOAD), subject: subject() as never }];
    const c = await card('req-s3');
    expect(c.querySelector('[data-testid="transfer-note"]')?.textContent).toContain('ยอดสมมติ — เติมเงินต้นวัน');
  });

  it('a subject with no note shows no note element', async () => {
    currentRequests = [{ ...createTransferRequest('req-s4', PAYLOAD), subject: subject({ note: null }) as never }];
    const c = await card('req-s4');
    expect(c.querySelector('[data-testid="transfer-amounts"]')).not.toBeNull();
    expect(c.querySelector('[data-testid="transfer-note"]')).toBeNull();
  });

  it('a subject with no branch name falls back to the branch id', async () => {
    currentRequests = [
      { ...createTransferRequest('req-s5', PAYLOAD), subject: subject({ fromBranchName: null }) as never },
    ];
    const c = await card('req-s5');
    expect(c.textContent).toContain('#74f7ecfe · MAIN · เกินวงเงิน — ผู้บริหารตัดสิน');
  });

  it('without a subject the card is unchanged: no totals, no note', async () => {
    currentRequests = [createTransferRequest('req-s6', PAYLOAD)];
    const c = await card('req-s6');
    expect(c.textContent).toContain('#74f7ecfe · MAIN · เกินวงเงิน — ผู้บริหารตัดสิน');
    expect(c.querySelector('[data-testid="transfer-amounts"]')).toBeNull();
    expect(c.querySelector('[data-testid="transfer-note"]')).toBeNull();
  });
});
