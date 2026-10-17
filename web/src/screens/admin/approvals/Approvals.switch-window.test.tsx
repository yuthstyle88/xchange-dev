import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import type {
  ApprovalDecisionDto,
  ApprovalRequestItemDto,
} from '../../../shared/api/generated/approval_requests';
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
): ApprovalRequestItemDto {
  return {
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
  };
}

interface DeferredRead {
  branch: string;
  status: string;
  resolveItems: (items: ApprovalRequestItemDto[]) => void;
  resolveError: (status: number, code: string, message: string) => void;
}

describe('Admin Approvals filter-switch window (#2407)', () => {
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
          return new Promise<Response>((resolve) => {
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
            });
          });
        }

        if (
          url.pathname.startsWith('/api/approvals/') &&
          url.pathname.endsWith('/decide') &&
          method === 'POST'
        ) {
          return Promise.resolve(
            new Response(JSON.stringify({ data: { ok: true } }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
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

  function mount(initialBranch = 'BRANCH-A'): {
    root: HTMLElement;
    setBranch: Setter<string>;
    setShown: Setter<boolean>;
  } {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown, setShown] = createSignal(true);
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
    return { root, setBranch, setShown };
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

  it('row 1: Branch A->B, while B read is in flight -> no A cards remain on screen', async () => {
    const { setBranch } = mount('BRANCH-A');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveItems([makeRequest('req-a-1', 'PENDING', null, 'BRANCH-A')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-a-1"]')).not.toBeNull();
    });

    // Switch to BRANCH-B; B read is in flight
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-B');

    // While B is in flight, no BRANCH-A cards or Approve/Reject buttons remain
    expect(root.querySelector('[data-request="req-a-1"]')).toBeNull();
    expect(root.querySelectorAll('[data-request]').length).toBe(0);
    expect(root.querySelector('.approval-decision-form')).toBeNull();
  });

  it('row 2: PENDING->APPROVED, while read is in flight -> no PENDING cards or decision form', async () => {
    mount('MAIN');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveItems([makeRequest('req-pending-1', 'PENDING', null, 'MAIN')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-pending-1"]')).not.toBeNull();
    });
    expect(root.querySelector('.approval-decision-form')).not.toBeNull();

    // Switch status pill from PENDING to APPROVED; read #2 is in flight
    switchButton(screenTh.approved).click();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.status).toBe('APPROVED');

    // While APPROVED read is in flight, PENDING cards and decision form are gone
    expect(root.querySelector('[data-request="req-pending-1"]')).toBeNull();
    expect(root.querySelector('.approval-decision-form')).toBeNull();
  });

  it('row 3: Branch A read failed, switch to B -> A error is not shown while B loads', async () => {
    const { setBranch } = mount('BRANCH-A');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveError(500, 'serverError', 'BRANCH-A-ERR');
    await vi.waitFor(() => {
      expect(root.querySelector('[role="alert"]')).not.toBeNull();
    });
    expect(root.textContent ?? '').toContain(sharedTh['error.serverError']);

    // Switch to BRANCH-B; while B is in flight, A's error is cleared immediately
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-B');

    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.textContent ?? '').not.toContain(sharedTh['error.serverError']);
  });

  it('row 4: Branch A loaded, then re-read A and switch to B with A answering last -> A cards cleared in flight and B shown', async () => {
    const { setBranch, setShown } = mount('BRANCH-A');
    pendingReads[0]!.resolveItems([makeRequest('req-a-initial', 'PENDING', null, 'BRANCH-A')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-a-initial"]')).not.toBeNull();
    });

    // Trigger another read for BRANCH-A via shown false -> true, then switch to BRANCH-B
    setShown(false);
    setShown(true);
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-A');

    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(3);
    expect(pendingReads[2]!.branch).toBe('BRANCH-B');

    // Immediately cleared while B is in flight
    expect(root.querySelector('[data-request="req-a-initial"]')).toBeNull();

    // B answers first, then stale A answers last
    pendingReads[2]!.resolveItems([makeRequest('req-b-final', 'PENDING', null, 'BRANCH-B')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-b-final"]')).not.toBeNull();
    });
    pendingReads[1]!.resolveItems([makeRequest('req-a-stale', 'PENDING', null, 'BRANCH-A')]);
    await new Promise((r) => setTimeout(r, 20));

    expect(root.querySelector('[data-request="req-b-final"]')).not.toBeNull();
    expect(root.querySelector('[data-request="req-a-stale"]')).toBeNull();
  });

  it('row 5: Decision on PENDING followed by switch to APPROVED clears cards in flight, and failed new-branch read keeps old requests cleared', async () => {
    const { setBranch } = mount('BRANCH-A');
    pendingReads[0]!.resolveItems([makeRequest('req-decide-1', 'PENDING', null, 'BRANCH-A')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-decide-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-decide-1"]')!;
    const approveBtn = [...card.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => normText(b) === screenTh.approve,
    )!;
    approveBtn.click();

    await vi.waitFor(() => {
      expect(pendingReads.length).toBe(2);
    });
    // onDecided reload for same context keeps rows until answer or context change
    expect(root.querySelector('[data-request="req-decide-1"]')).not.toBeNull();

    // Switch to APPROVED while onDecided reload is in flight -> immediately clears cards
    switchButton(screenTh.approved).click();
    expect(pendingReads.length).toBe(3);
    expect(root.querySelector('[data-request="req-decide-1"]')).toBeNull();

    pendingReads[2]!.resolveItems([
      makeRequest('req-approved-1', 'APPROVED', {
        approverId: 'u-1',
        outcome: 'APPROVED',
        reason: 'ok',
        at: '2026-10-15T12:00:00Z',
      }),
    ]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    });

    // Switch to BRANCH-B -> clears cards in flight, and if B fails, old cards stay cleared
    setBranch('BRANCH-B');
    expect(root.querySelector('[data-request="req-approved-1"]')).toBeNull();
    pendingReads[3]!.resolveError(500, 'serverError', 'BRANCH-B-FAIL');
    await vi.waitFor(() => {
      expect(root.querySelector('[role="alert"]')).not.toBeNull();
    });
    expect(root.querySelector('[data-request="req-approved-1"]')).toBeNull();
  });

  it('edge cases: shown false -> true with the same context keeps the rows, while switching context clears rows and shows empty state during load', async () => {
    const { setBranch, setShown } = mount('BRANCH-A');
    pendingReads[0]!.resolveItems([makeRequest('req-keep-1', 'PENDING', null, 'BRANCH-A')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-keep-1"]')).not.toBeNull();
    });

    // Re-show with the exact same branch + status keeps the rows while the reload is in flight
    setShown(false);
    setShown(true);
    expect(pendingReads.length).toBe(2);
    expect(root.querySelector('[data-request="req-keep-1"]')).not.toBeNull();

    // Switching branch clears the rows immediately and shows the empty-state text during the load
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(3);
    expect(root.querySelector('[data-request="req-keep-1"]')).toBeNull();
    expect(root.textContent ?? '').toContain(screenTh.empty);
  });
});
