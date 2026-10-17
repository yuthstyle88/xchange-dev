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

interface DeferredDecide {
  url: string;
  body: unknown;
  resolveOk: () => void;
  resolveError: (status: number, code: string, message: string) => void;
}

describe('Admin Approvals: decide-switch behavior (#2715)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredRead[];
  let pendingDecides: DeferredDecide[];

  beforeEach(() => {
    setLang('th');
    pendingReads = [];
    pendingDecides = [];
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
          const body = init?.body ? JSON.parse(String(init.body)) : null;
          return new Promise<Response>((resolve) => {
            pendingDecides.push({
              url: url.pathname,
              body,
              resolveOk: () => {
                resolve(
                  new Response(JSON.stringify({ data: { ok: true } }), {
                    status: 200,
                    headers: { 'content-type': 'application/json' },
                  }),
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

  it('row 1: Approve on A held, switch to B, A POST fails -> no toast and no card error on B', async () => {
    const { setBranch } = mount('BRANCH-A');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveItems([makeRequest('req-a-1', 'PENDING', null, 'BRANCH-A')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-a-1"]')).not.toBeNull();
    });

    const cardA = root.querySelector('[data-request="req-a-1"]') as HTMLElement;
    const approveBtn = cardA.querySelector('button.primary') as HTMLButtonElement;
    approveBtn.click();

    await vi.waitFor(() => {
      expect(pendingDecides.length).toBe(1);
    });

    // Switch branch to BRANCH-B
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-B');

    pendingReads[1]!.resolveItems([makeRequest('req-b-1', 'PENDING', null, 'BRANCH-B')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-b-1"]')).not.toBeNull();
    });

    // A's held POST now fails with 409
    pendingDecides[0]!.resolveError(409, 'conflict', 'Stale version');
    await new Promise((r) => setTimeout(r, 20));

    const toastEl = document.body.querySelector('.toast');
    expect(toastEl?.textContent ?? '').toBe('');

    const cardB = root.querySelector('[data-request="req-b-1"]') as HTMLElement;
    expect(cardB.querySelector('.error')).toBeNull();
    expect(root.querySelector('.error')).toBeNull();
  });

  it('row 2: Reject on PENDING held, switch filter to APPROVED, POST fails -> no toast on APPROVED', async () => {
    mount('MAIN');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveItems([makeRequest('req-pending-1', 'PENDING', null, 'MAIN')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-pending-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-pending-1"]') as HTMLElement;
    const reasonInput = card.querySelector('input[type="text"]') as HTMLInputElement;
    reasonInput.value = 'invalid publication parameters';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtn = card.querySelector('button.danger') as HTMLButtonElement;
    rejectBtn.click();

    await vi.waitFor(() => {
      expect(pendingDecides.length).toBe(1);
    });

    // Switch filter to APPROVED
    switchButton(screenTh.approved).click();
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.status).toBe('APPROVED');

    pendingReads[1]!.resolveItems([
      makeRequest(
        'req-approved-1',
        'APPROVED',
        {
          approverId: 'u-1',
          outcome: 'APPROVED',
          reason: 'ok',
          at: '2026-10-15T10:00:00Z',
        },
        'MAIN',
      ),
    ]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    });

    // PENDING Reject POST fails with 500
    pendingDecides[0]!.resolveError(500, 'serverError', 'Internal error');
    await new Promise((r) => setTimeout(r, 20));

    const toastEl = document.body.querySelector('.toast');
    expect(toastEl?.textContent ?? '').toBe('');
  });

  it('row 3: Approve on A held, switch to B (B read held), A POST succeeds -> no second read for B and B read in flight not cancelled', async () => {
    const { setBranch } = mount('BRANCH-A');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveItems([makeRequest('req-a-1', 'PENDING', null, 'BRANCH-A')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-a-1"]')).not.toBeNull();
    });

    const cardA = root.querySelector('[data-request="req-a-1"]') as HTMLElement;
    const approveBtn = cardA.querySelector('button.primary') as HTMLButtonElement;
    approveBtn.click();

    await vi.waitFor(() => {
      expect(pendingDecides.length).toBe(1);
    });

    // Switch branch to BRANCH-B while A's POST is still in flight
    setBranch('BRANCH-B');
    expect(pendingReads.length).toBe(2);
    expect(pendingReads[1]!.branch).toBe('BRANCH-B');

    // A's decide POST succeeds while B's read is still held in flight
    pendingDecides[0]!.resolveOk();
    await new Promise((r) => setTimeout(r, 20));

    // Must NOT fire a second read for B
    expect(pendingReads.length).toBe(2);

    // B's read in flight resolves with B request; it must not have been cancelled by A's late loadQueue
    pendingReads[1]!.resolveItems([makeRequest('req-b-1', 'PENDING', null, 'BRANCH-B')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-b-1"]')).not.toBeNull();
    });
  });

  it('row 4: A decision fails with no switch -> card error and toast show worded error', async () => {
    mount('MAIN');
    expect(pendingReads.length).toBe(1);

    pendingReads[0]!.resolveItems([makeRequest('req-1', 'PENDING', null, 'MAIN')]);
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-1"]') as HTMLElement;
    const approveBtn = card.querySelector('button.primary') as HTMLButtonElement;
    approveBtn.click();

    await vi.waitFor(() => {
      expect(pendingDecides.length).toBe(1);
    });

    pendingDecides[0]!.resolveError(403, 'roleNotAllowedHere', 'Not allowed');

    await vi.waitFor(() => {
      const errorEl = card.querySelector('.error');
      expect(errorEl).not.toBeNull();
      expect(errorEl!.textContent).toContain(sharedTh['error.roleNotAllowedHere']);
    });

    const toastEl = document.body.querySelector('.toast');
    expect(toastEl?.textContent ?? '').toContain(sharedTh['error.roleNotAllowedHere']);
  });
});
