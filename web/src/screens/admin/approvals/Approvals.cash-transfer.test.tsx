// #5314: CASH_TRANSFER approval card shows transfer id, from_branch, and limit flag
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from './i18n';
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

describe('#5314 Approvals: CASH_TRANSFER detail display (§8, §9)', () => {
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

  // Row 1: {transfer_id:"74f7ecfe-cc4a-…", from_branch:"MAIN", within_limit:false}
  it('row 1: shows #74f7ecfe (title = full id) · MAIN · เกินวงเงิน — ผู้บริหารตัดสิน', async () => {
    const fullId = '74f7ecfe-cc4a-4b94-aac2-7b32f08ff33e';
    currentRequests = [
      createTransferRequest('req-1', {
        transfer_id: fullId,
        from_branch: 'MAIN',
        within_limit: false,
      }),
    ];

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-1"]');
    const shortIdEl = card!.querySelector(`[title="${fullId}"]`);
    expect(shortIdEl, 'element has full transfer_id in title').not.toBeNull();
    expect(shortIdEl?.textContent).toBe('#74f7ecfe');

    expect(card!.textContent).toContain('#74f7ecfe · MAIN · เกินวงเงิน — ผู้บริหารตัดสิน');

    // Test English localization
    setLang('en');
    await vi.waitFor(() => {
      expect(card!.textContent).toContain('#74f7ecfe · MAIN · Above the limit — an executive decides');
    });
  });

  // Row 2: same with within_limit:true -> within-limit text
  it('row 2: same with within_limit:true shows within-limit text', async () => {
    const fullId = '74f7ecfe-cc4a-4b94-aac2-7b32f08ff33e';
    currentRequests = [
      createTransferRequest('req-2', {
        transfer_id: fullId,
        from_branch: 'MAIN',
        within_limit: true,
      }),
    ];

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-2"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-2"]');
    expect(card!.textContent).toContain('#74f7ecfe · MAIN · อยู่ในวงเงิน');

    // Test English localization
    setLang('en');
    await vi.waitFor(() => {
      expect(card!.textContent).toContain('#74f7ecfe · MAIN · Within the limit');
    });
  });

  // Row 3: {} or null -> no extra lines, card still renders
  it('row 3: proposedPayload {} or null renders card with no extra detail lines', async () => {
    currentRequests = [
      createTransferRequest('req-empty', {}),
      createTransferRequest('req-null', null),
    ];

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-empty"]')).not.toBeNull();
      expect(root.querySelector('[data-request="req-null"]')).not.toBeNull();
    });

    const cardEmpty = root.querySelector('[data-request="req-empty"]');
    expect(cardEmpty!.textContent).not.toContain('undefined');
    expect(cardEmpty!.textContent).not.toContain('·');

    const cardNull = root.querySelector('[data-request="req-null"]');
    expect(cardNull!.textContent).not.toContain('undefined');
    expect(cardNull!.textContent).not.toContain('·');
  });

  // Row 4: a non-CASH_TRANSFER card -> unchanged
  it('row 4: non-CASH_TRANSFER card is unchanged', async () => {
    currentRequests = [
      createTransferRequest(
        'req-rate',
        {
          draft_id: 'draft-1',
          policy_version_id: 'pol-1',
          reference_line_ids: ['line-1'],
          is_in_bound: false,
          out_of_bound: [{ currency: 'USD', side: 'SELL' }],
        },
        'RATE_PUBLICATION',
      ),
    ];

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-rate"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-rate"]');
    expect(card!.textContent).toContain('USD');
    expect(card!.textContent).toContain(th.sideSELL);
    expect(card!.textContent).not.toContain('อยู่ในวงเงิน');
    expect(card!.textContent).not.toContain('เกินวงเงิน');
  });

  // Section 9: Edge cases: transfer_id is not a string: no transfer line
  it('edge case: transfer_id is not a string: no transfer line', async () => {
    currentRequests = [
      createTransferRequest('req-edge', {
        transfer_id: 12345678,
        from_branch: 'MAIN',
        within_limit: false,
      }),
    ];

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-edge"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-edge"]');
    expect(card!.textContent).not.toContain('#12345678');
    expect(card!.textContent).not.toContain('#');
    expect(card!.textContent).toContain('MAIN · เกินวงเงิน — ผู้บริหารตัดสิน');
  });
});
