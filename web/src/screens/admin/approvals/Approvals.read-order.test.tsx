import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import {
  makeApprovalDecisionDto,
  makeApprovalRequestItemDto,
  type ApprovalDecisionDto,
  type ApprovalRequestItemDto,
} from '../../../shared/testing/fixtures';
import { Approvals } from './Approvals';
import { th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

beforeAll(() => warmModules(() => import('./Approvals')), 60_000);

const REQUESTS_PATH = '/api/v3/views/approval_requests';

function makeRequest(
  id: string,
  status: 'PENDING' | 'APPROVED' | 'REJECTED',
  decision: ApprovalDecisionDto | null = null,
  branch = 'MAIN',
) {
  return makeApprovalRequestItemDto({
    id,
    type: 'RATE_PUBLICATION',
    branchId: branch,
    objectId: `draft-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-1',
    proposedPayload: {
      draft_id: `draft-${id}`,
      policy_version_id: 'pol-1',
      reference_line_ids: ['line-1'],
      is_in_bound: false,
      out_of_bound: [{ currency: 'USD', side: 'SELL' }],
    },
    requestedBy: 'proposer-1',
    requestedAt: '2026-10-15T09:00:00Z',
    status,
    decision,
  });
}

interface DeferredRead {
  branch: string;
  status: string;
  resolveItems: (items: ApprovalRequestItemDto[]) => void;
  resolveError: (status: number, code: string, message: string) => void;
  rejectNetwork: (err?: unknown) => void;
}

describe('Admin Approvals read-order guard (#2271)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredRead[];

  beforeEach(() => {
    setLang('th');
    pendingReads = [];
    root = document.createElement('div');
    document.body.appendChild(root);
    toast('');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          const branch = url.searchParams.get('branch') ?? '';
          const status = url.searchParams.get('status') ?? '';
          return new Promise<Response>((resolve, reject) => {
            pendingReads.push({
              branch,
              status,
              resolveItems: (items) => {
                resolve(
                  new Response(
                    JSON.stringify({
                      version: 1,
                      changed: true,
                      data: { requests: structuredClone(items) },
                    }),
                    {
                      status: 200,
                      headers: { 'content-type': 'application/json' },
                    },
                  ),
                );
              },
              resolveError: (httpStatus, code, message) => {
                resolve(
                  new Response(
                    JSON.stringify({
                      error: { code, message },
                    }),
                    {
                      status: httpStatus,
                      headers: { 'content-type': 'application/json' },
                    },
                  ),
                );
              },
              rejectNetwork: (err = new TypeError('Failed to fetch')) => {
                reject(err);
              },
            });
          });
        }

        if (url.pathname.startsWith('/api/approvals/') && url.pathname.endsWith('/decide') && method === 'POST') {
          return Promise.resolve(
            new Response(
              JSON.stringify({ data: { ok: true } }),
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

  function mount(initialBranch = 'MAIN'): {
    root: HTMLElement;
    setBranch: Setter<string>;
  } {
    const [branch, setBranch] = createSignal(initialBranch);
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
    return { root, setBranch };
  }

  function normText(el: Element | null | undefined): string {
    return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  }

  function switchButton(label: string): HTMLButtonElement {
    const switchGroup = root.querySelector('[data-status-switch]') ?? root;
    const found = [...switchGroup.querySelectorAll<HTMLButtonElement>('button')].filter(
      (b) => normText(b) === label,
    );
    expect(found.length, `status switch button ${JSON.stringify(label)}`).toBe(1);
    return found[0]!;
  }

  it('row 1: PENDING then APPROVED, the PENDING answer comes last -> the APPROVED items stay', async () => {
    mount('MAIN');
    expect(pendingReads.length).toBe(1);
    expect(pendingReads[0]!.status).toBe('PENDING');

    // Switch to APPROVED while PENDING read is still in flight
    switchButton(screenTh.approved).click();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.status).toBe('APPROVED');

    // Newer APPROVED read answers first
    pendingReads[1]!.resolveItems([
      makeRequest(
        'req-approved-1',
        'APPROVED',
        makeApprovalDecisionDto({
          approverId: 'u-9',
          outcome: 'APPROVED',
          reason: 'ok',
          at: '2026-10-15T10:00:00Z',
        }),
      ),
    ]);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    });

    // Older PENDING read answers last
    pendingReads[0]!.resolveItems([makeRequest('req-pending-stale', 'PENDING', null)]);
    await new Promise((r) => setTimeout(r, 20));

    expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    expect(root.querySelector('[data-request="req-pending-stale"]')).toBeNull();
  });

  it('row 2: Branch A then B, A answers last -> B requests are shown', async () => {
    const { setBranch } = mount('BRANCH-A');
    expect(pendingReads.length).toBe(1);
    expect(pendingReads[0]!.branch).toBe('BRANCH-A');

    // Switch to BRANCH-B while BRANCH-A read is still in flight
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-B');

    // Newer BRANCH-B read answers first
    pendingReads[1]!.resolveItems([makeRequest('req-branch-b', 'PENDING', null, 'BRANCH-B')]);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-branch-b"]')).not.toBeNull();
    });

    // Older BRANCH-A read answers last
    pendingReads[0]!.resolveItems([makeRequest('req-branch-a-stale', 'PENDING', null, 'BRANCH-A')]);
    await new Promise((r) => setTimeout(r, 20));

    expect(root.querySelector('[data-request="req-branch-b"]')).not.toBeNull();
    expect(root.querySelector('[data-request="req-branch-a-stale"]')).toBeNull();
  });

  it('row 3: An older read fails after a newer one succeeded -> no error; B items stay', async () => {
    const { setBranch } = mount('BRANCH-A');
    expect(pendingReads.length).toBe(1);
    expect(pendingReads[0]!.branch).toBe('BRANCH-A');

    // Switch to BRANCH-B while BRANCH-A read is still in flight
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-B');

    // Newer BRANCH-B read succeeds first
    pendingReads[1]!.resolveItems([makeRequest('req-branch-b-ok', 'PENDING', null, 'BRANCH-B')]);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-branch-b-ok"]')).not.toBeNull();
    });

    // Older BRANCH-A read fails last with 500 serverError
    pendingReads[0]!.resolveError(500, 'serverError', 'STALE-BRANCH-A-FAIL');
    await new Promise((r) => setTimeout(r, 20));

    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent ?? '').not.toContain(sharedTh['error.serverError']);
    expect(root.querySelector('[data-request="req-branch-b-ok"]')).not.toBeNull();
  });

  it('edge case: A decision followed by a filter switch while the reload is in flight -> the switch answer wins', async () => {
    mount('MAIN');
    expect(pendingReads.length).toBe(1);

    // Initial PENDING read resolves with a pending item
    pendingReads[0]!.resolveItems([makeRequest('req-decide-1', 'PENDING', null, 'MAIN')]);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-decide-1"]')).not.toBeNull();
    });

    // Click Approve on the pending card; POST /decide resolves and triggers onDecided -> loadQueue (read #2)
    const card = root.querySelector('[data-request="req-decide-1"]')!;
    const approveBtn = [...card.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => normText(b) === screenTh.approve,
    )!;
    expect(approveBtn).toBeDefined();
    approveBtn.click();

    await vi.waitFor(() => {
      expect(pendingReads.length).toBe(2);
    });
    expect(pendingReads[1]!.status).toBe('PENDING');

    // While the post-decision reload (read #2, PENDING) is still in flight, switch filter to APPROVED (read #3)
    switchButton(screenTh.approved).click();
    expect(pendingReads.length).toBe(3);
    expect(pendingReads[2]!.status).toBe('APPROVED');

    // Read #3 (APPROVED) answers first
    pendingReads[2]!.resolveItems([
      makeRequest(
        'req-approved-after-switch',
        'APPROVED',
        makeApprovalDecisionDto({
          approverId: 'u-1',
          outcome: 'APPROVED',
          reason: 'ok',
          at: '2026-10-15T12:00:00Z',
        }),
      ),
    ]);

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-after-switch"]')).not.toBeNull();
    });

    // In-flight post-decision reload (read #2, PENDING) answers last
    pendingReads[1]!.resolveItems([makeRequest('req-pending-from-reload', 'PENDING', null, 'MAIN')]);
    await new Promise((r) => setTimeout(r, 20));

    expect(root.querySelector('[data-request="req-approved-after-switch"]')).not.toBeNull();
    expect(root.querySelector('[data-request="req-pending-from-reload"]')).toBeNull();
  });
});
