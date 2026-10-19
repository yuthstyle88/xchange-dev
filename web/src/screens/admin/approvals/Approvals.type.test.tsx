// #3332: Approvals approval request type display and type filter switch
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th, en } from './i18n';
import { Approvals } from './Approvals';
import { Toast, toast } from '../parts/toast';
import {
  makeApprovalRequestItemDto,
  type ApprovalDecisionDto,
  type ApprovalRequestItemDto,
} from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';

function makeRequest(
  id: string,
  type: string = 'RATE_PUBLICATION',
  status: string = 'PENDING',
  decision: ApprovalDecisionDto | null = null,
) {
  return makeApprovalRequestItemDto({
    id,
    type,
    branchId: 'MAIN',
    objectId: `draft-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-1',
    proposedPayload: {
      draft_id: `draft-${id}`,
      policy_version_id: 'pol-1',
      reference_line_ids: ['line-1'],
      is_in_bound: false,
      out_of_bound: [],
    },
    requestedBy: 'proposer-1',
    requestedAt: '2026-10-15T09:00:00Z',
    status,
    decision,
  });
}

interface RecordedRead {
  path: string;
  params: URLSearchParams;
}

const TYPE_CASES = [
  { type: 'RATE_PUBLICATION', th: th.ratePublication, en: en.ratePublication },
  { type: 'RATE_FEED_PUBLICATION', th: th.rateFeedPublication, en: en.rateFeedPublication },
  { type: 'CAMPAIGN_VERSION', th: th.campaignVersion, en: en.campaignVersion },
  { type: 'LOYALTY_RULE_VERSION', th: th.loyaltyRuleVersion, en: en.loyaltyRuleVersion },
  { type: 'EXCHANGE_TRANSACTION', th: th.exchangeTransaction, en: en.exchangeTransaction },
  { type: 'TRANSACTION_REVERSAL', th: th.transactionReversal, en: en.transactionReversal },
  { type: 'COUNT_DIFFERENCE', th: th.countDifference, en: en.countDifference },
  { type: 'NEGATIVE_STOCK', th: th.negativeStock, en: en.negativeStock },
  { type: 'CASH_TRANSFER', th: th.cashTransfer, en: en.cashTransfer },
] as const;

describe('#3332 Approvals: type display and type filter switch (§8)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let reads: RecordedRead[];
  let currentRequests: ApprovalRequestItemDto[];
  let customFetchHandler: ((url: URL, method: string) => Promise<Response> | null) | null;

  beforeEach(() => {
    setLang('th');
    reads = [];
    customFetchHandler = null;
    currentRequests = [makeRequest('req-1', 'RATE_PUBLICATION', 'PENDING')];

    root = document.createElement('div');
    document.body.appendChild(root);
    toast('');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          reads.push({ path: url.pathname, params: url.searchParams });

          if (customFetchHandler) {
            const custom = customFetchHandler(url, method);
            if (custom) return custom;
          }

          const typeFilter = url.searchParams.get('type');
          const statusFilter = url.searchParams.get('status') ?? 'PENDING';
          const filtered = currentRequests.filter((r) => {
            if (r.status !== statusFilter) return false;
            if (typeFilter && r.type !== typeFilter) return false;
            return true;
          });

          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { requests: structuredClone(filtered) },
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
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
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

  function typeButton(type: string): HTMLButtonElement {
    const btn = root.querySelector<HTMLButtonElement>(`button[data-type-filter="${type}"]`);
    expect(btn, `type filter button for "${type}"`).not.toBeNull();
    return btn!;
  }

  // Row 1: a TRANSACTION_REVERSAL request
  it('row 1: a TRANSACTION_REVERSAL request shows header กลับรายการ, data-request-type="TRANSACTION_REVERSAL", never เผยแพร่เรท', async () => {
    currentRequests = [makeRequest('req-rev-1', 'TRANSACTION_REVERSAL', 'PENDING')];
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-rev-1"]')).not.toBeNull();
    });

    const header = root.querySelector('[data-request="req-rev-1"] .approval-type');
    expect(header).not.toBeNull();
    expect(header?.getAttribute('data-request-type')).toBe('TRANSACTION_REVERSAL');
    expect(header?.textContent?.trim()).toBe('กลับรายการ');
    expect(header?.textContent).not.toContain('เผยแพร่เรท');
  });

  // Row 2: each of the nine types (parametrised test)
  describe('row 2: each of the nine types has its own label in TH and EN', () => {
    for (const item of TYPE_CASES) {
      it(`renders label for type ${item.type}`, async () => {
        // Test in TH
        setLang('th');
        currentRequests = [makeRequest(`req-${item.type}`, item.type, 'PENDING')];
        mount();

        await vi.waitFor(() => {
          expect(root.querySelector(`[data-request="req-${item.type}"]`)).not.toBeNull();
        });

        let header = root.querySelector(`[data-request="req-${item.type}"] .approval-type`);
        expect(header?.getAttribute('data-request-type')).toBe(item.type);
        expect(header?.textContent?.trim()).toBe(item.th);

        // Test in EN
        dispose?.();
        root.innerHTML = '';
        setLang('en');
        mount();

        await vi.waitFor(() => {
          expect(root.querySelector(`[data-request="req-${item.type}"]`)).not.toBeNull();
        });

        header = root.querySelector(`[data-request="req-${item.type}"] .approval-type`);
        expect(header?.getAttribute('data-request-type')).toBe(item.type);
        expect(header?.textContent?.trim()).toBe(item.en);
      });
    }
  });

  // Row 3: an unknown type FUTURE_X
  it('row 3: an unknown type FUTURE_X shows header FUTURE_X', async () => {
    currentRequests = [makeRequest('req-future-1', 'FUTURE_X', 'PENDING')];
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-future-1"]')).not.toBeNull();
    });

    const header = root.querySelector('[data-request="req-future-1"] .approval-type');
    expect(header?.getAttribute('data-request-type')).toBe('FUTURE_X');
    expect(header?.textContent?.trim()).toBe('FUTURE_X');
  });

  // Row 4: pick type COUNT_DIFFERENCE
  it('row 4: pick type COUNT_DIFFERENCE sends read with type=COUNT_DIFFERENCE and current status', async () => {
    mount();

    await vi.waitFor(() => expect(reads.length).toBeGreaterThanOrEqual(1));
    const btn = typeButton('COUNT_DIFFERENCE');
    btn.click();

    await vi.waitFor(() => {
      const lastRead = reads[reads.length - 1]!;
      expect(lastRead.params.get('type')).toBe('COUNT_DIFFERENCE');
      expect(lastRead.params.get('status')).toBe('PENDING');
    });
  });

  // Row 5: pick "all"
  it('row 5: pick "all" sends read with no type filter', async () => {
    mount();
    await vi.waitFor(() => expect(reads.length).toBeGreaterThanOrEqual(1));

    // First pick a type
    typeButton('COUNT_DIFFERENCE').click();
    await vi.waitFor(() => {
      expect(reads[reads.length - 1]?.params.get('type')).toBe('COUNT_DIFFERENCE');
    });

    // Then pick all ("")
    typeButton('').click();
    await vi.waitFor(() => {
      const lastRead = reads[reads.length - 1]!;
      expect(lastRead.params.get('type')).toBeNull();
      expect(lastRead.params.get('status')).toBe('PENDING');
    });
  });

  // Row 6: switch type while a read is in flight
  it('row 6: switch type while a read is in flight drops the late answer for the old type', async () => {
    let resolveOldRead: ((res: Response) => void) | null = null;
    customFetchHandler = (url) => {
      const type = url.searchParams.get('type');
      if (type === 'COUNT_DIFFERENCE') {
        return new Promise<Response>((resolve) => {
          resolveOldRead = resolve;
        });
      }
      return null;
    };

    mount();
    await vi.waitFor(() => expect(reads.length).toBe(1));

    // 1. Pick COUNT_DIFFERENCE (read remains in flight)
    typeButton('COUNT_DIFFERENCE').click();
    await vi.waitFor(() => {
      expect(reads.some((r) => r.params.get('type'))).toBe(true);
    });
    expect(resolveOldRead).not.toBeNull();

    // 2. Quickly switch to NEGATIVE_STOCK
    currentRequests = [makeRequest('req-neg-1', 'NEGATIVE_STOCK', 'PENDING')];
    typeButton('NEGATIVE_STOCK').click();

    // 3. NEGATIVE_STOCK resolves and renders
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-neg-1"]')).not.toBeNull();
    });

    // 4. Old in-flight COUNT_DIFFERENCE read now resolves with stale data
    resolveOldRead!(
      new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: { requests: [makeRequest('req-count-diff-old', 'COUNT_DIFFERENCE', 'PENDING')] },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );

    // Let any microtasks settle
    await new Promise((r) => setTimeout(r, 20));

    // Stale COUNT_DIFFERENCE card was not shown; only NEGATIVE_STOCK is present
    expect(root.querySelector('[data-request="req-count-diff-old"]')).toBeNull();
    expect(root.querySelector('[data-request="req-neg-1"]')).not.toBeNull();
  });

  // Row 7: a RATE_PUBLICATION request
  it('row 7: a RATE_PUBLICATION request shows เผยแพร่เรท as today', async () => {
    currentRequests = [makeRequest('req-rate-1', 'RATE_PUBLICATION', 'PENDING')];
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-rate-1"]')).not.toBeNull();
    });

    const header = root.querySelector('[data-request="req-rate-1"] .approval-type');
    expect(header?.getAttribute('data-request-type')).toBe('RATE_PUBLICATION');
    expect(header?.textContent?.trim()).toBe('เผยแพร่เรท');
  });
});
