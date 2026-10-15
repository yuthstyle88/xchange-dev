// RE-5b (#1984): A06 status switch (Pending / Approved / Rejected) and decision block on decided cards.
// One test per §9 behaviour table row.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import type {
  ApprovalDecisionDto,
  ApprovalRequestItemDto,
} from '../../../shared/api/generated/approval_requests';
import { Approvals } from './Approvals';
import { en as screenEn, th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

beforeAll(() => warmModules(() => import('./Approvals')), 60_000);

const THAI = /[\u0E00-\u0E7F]/;
const REQUESTS_PATH = '/api/v3/views/approval_requests';

function makeRequest(
  id: string,
  status: 'PENDING' | 'APPROVED' | 'REJECTED',
  decision: ApprovalDecisionDto | null,
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

interface RecordedRead {
  path: string;
  params: URLSearchParams;
}

describe('RE-5b (#1984): A06 status switch and decision block (§9)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let reads: RecordedRead[];
  let itemsByStatus: Record<string, ApprovalRequestItemDto[]>;
  let errorForStatus: Record<string, { status: number; body: unknown }>;

  beforeEach(() => {
    setLang('th');
    reads = [];
    errorForStatus = {};
    itemsByStatus = {
      PENDING: [makeRequest('req-pending-1', 'PENDING', null)],
      APPROVED: [
        makeRequest('req-approved-1', 'APPROVED', {
          approverId: 'u-9',
          outcome: 'APPROVED',
          reason: 'ok',
          at: '2026-10-15T10:00:00Z',
        }),
      ],
      REJECTED: [
        makeRequest('req-rejected-1', 'REJECTED', {
          approverId: 'u-7',
          outcome: 'REJECTED',
          reason: 'spread exceeds branch limit',
          at: '2026-10-15T11:30:00Z',
        }),
      ],
    };

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
          const status = url.searchParams.get('status') ?? '';
          const forcedErr = errorForStatus[status];
          if (forcedErr) {
            return Promise.resolve(
              new Response(JSON.stringify(forcedErr.body), {
                status: forcedErr.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          const list = status
            ? (itemsByStatus[status] ?? [])
            : Object.values(itemsByStatus).flat();
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { requests: structuredClone(list) },
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

  it('1. first load: sends one read with filter.status = "PENDING"', async () => {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-pending-1"]')).not.toBeNull();
    });
    expect(reads.length).toBe(1);
    expect(reads[0]!.params.get('branch')).toBe('MAIN');
    expect(reads[0]!.params.get('status')).toBe('PENDING');
  });

  it('2. switch to Approved: sends one read with status "APPROVED" and renders its items', async () => {
    mount();
    await vi.waitFor(() => {
      expect(reads.length).toBe(1);
    });

    switchButton(screenTh.approved).click();

    await vi.waitFor(() => {
      expect(reads.length).toBe(2);
      expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    });
    expect(reads[1]!.params.get('branch')).toBe('MAIN');
    expect(reads[1]!.params.get('status')).toBe('APPROVED');
    expect(root.querySelector('[data-request="req-pending-1"]')).toBeNull();
  });

  it('3. an APPROVED item with decision shows approverId, approved label, reason and time, with no approve/reject buttons', async () => {
    mount();
    await vi.waitFor(() => {
      expect(reads.length).toBe(1);
    });

    switchButton(screenTh.approved).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-approved-1"]') as HTMLElement;
    const decisionBlock = card.querySelector('[data-decision="req-approved-1"]');
    expect(decisionBlock).not.toBeNull();

    const blockText = normText(decisionBlock);
    expect(blockText).toContain('u-9');
    expect(blockText).toContain(screenTh.approved);
    expect(blockText).toContain('ok');
    expect(blockText).toContain('2026-10-15T10:00:00Z');

    expect(card.querySelectorAll('button').length).toBe(0);
    expect(card.querySelector('input[type="text"]')).toBeNull();
  });

  it('4. a REJECTED item with a reason shows the rejected label and the reason', async () => {
    mount();
    await vi.waitFor(() => {
      expect(reads.length).toBe(1);
    });

    switchButton(screenTh.rejected).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-rejected-1"]')).not.toBeNull();
    });
    expect(reads.at(-1)!.params.get('status')).toBe('REJECTED');

    const card = root.querySelector('[data-request="req-rejected-1"]') as HTMLElement;
    const decisionBlock = card.querySelector('[data-decision="req-rejected-1"]');
    expect(decisionBlock).not.toBeNull();

    const blockText = normText(decisionBlock);
    expect(blockText).toContain(screenTh.rejected);
    expect(blockText).toContain('spread exceeds branch limit');
    expect(blockText).toContain('u-7');
    expect(blockText).toContain('2026-10-15T11:30:00Z');
    expect(card.querySelectorAll('button').length).toBe(0);
  });

  it('5. a PENDING item (decision: null) shows the decision form as today and no decision block', async () => {
    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-pending-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-pending-1"]') as HTMLElement;
    expect(card.querySelector('.approval-decision-form')).not.toBeNull();
    expect(card.querySelector('input[type="text"]')).not.toBeNull();
    expect(card.querySelectorAll('button').length).toBe(2);
    expect(card.querySelector('[data-decision]')).toBeNull();
    expect(normText(card)).not.toContain(screenTh.decidedBy.trim());
    expect(normText(card)).not.toContain(screenTh.noDecisionRecord);
  });

  it('6. an item with status APPROVED but decision: null does not crash and shows the worded no decision record line', async () => {
    itemsByStatus.APPROVED = [makeRequest('req-approved-null', 'APPROVED', null)];
    mount();
    await vi.waitFor(() => {
      expect(reads.length).toBe(1);
    });

    switchButton(screenTh.approved).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-null"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-approved-null"]') as HTMLElement;
    const decisionBlock = card.querySelector('[data-decision="req-approved-null"]');
    expect(decisionBlock).not.toBeNull();
    expect(normText(decisionBlock)).toContain(screenTh.noDecisionRecord);
    expect(card.querySelectorAll('button').length).toBe(0);
  });

  it('7. the switched read failing with 500 {code: "roleNotAllowedHere"} shows th["error.serverError"] via wordedError', async () => {
    errorForStatus.APPROVED = {
      status: 500,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Keyed 500 failure on switched read',
        },
      },
    };

    mount();
    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-pending-1"]')).not.toBeNull();
    });

    switchButton(screenTh.approved).click();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });

    const alertText = root.querySelector('[role="alert"]')!.textContent ?? '';
    expect(alertText).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(alertText).not.toContain('Keyed 500 failure on switched read');
    expect(root.querySelector('[data-request="req-pending-1"]')).toBeNull();
  });

  it('8. language en: renders the status switch and the decision block labels in English with no Thai', async () => {
    setLang('en');
    mount();

    await vi.waitFor(() => {
      expect(reads.length).toBe(1);
    });

    expect(switchButton(screenEn.pending)).toBeDefined();
    expect(switchButton(screenEn.approved)).toBeDefined();
    expect(switchButton(screenEn.rejected)).toBeDefined();

    switchButton(screenEn.approved).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-approved-1"]')).not.toBeNull();
    });

    const card = root.querySelector('[data-request="req-approved-1"]') as HTMLElement;
    const decisionBlock = card.querySelector('[data-decision="req-approved-1"]');
    expect(decisionBlock).not.toBeNull();

    const blockText = normText(decisionBlock);
    expect(blockText).toContain(screenEn.decidedBy.trim());
    expect(blockText).toContain(screenEn.decisionOutcome.trim());
    expect(blockText).toContain(screenEn.approved);
    expect(blockText).toContain(screenEn.decisionReason);
    expect(blockText).toContain(screenEn.decidedAt.trim());
    expect(blockText).toContain('u-9');
    expect(blockText).toContain('ok');
    expect(blockText).toContain('2026-10-15T10:00:00Z');
    expect(normText(root)).not.toMatch(THAI);
  });
});
