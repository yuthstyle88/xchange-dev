// WE-4 (#1753): screens/admin/approvals/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: loadQueue, handleApprove, handleReject)
// and §4.3 (stale data cleared on branch switch before failed read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import type { ApprovalRequestItemDto } from '../../../shared/api/generated/approval_requests';
import { Approvals } from './Approvals';
import { Toast, toast } from '../parts/toast';

function fakeRequest(id: string, branch = 'MAIN'): ApprovalRequestItemDto {
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
    requestedBy: 'operator-1',
    requestedAt: '2026-10-14T10:00:00+00:00',
    status: 'PENDING',
  };
}

describe('Approvals worded error handling (#1753)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let queueResponse: { status: number; body: unknown } | 'network' | null = null;
  let decideResponse: { status: number; body: unknown } | 'network' | null = null;

  beforeEach(() => {
    setLang('th');
    queueResponse = null;
    decideResponse = null;
    root = document.createElement('div');
    document.body.appendChild(root);
    toast('');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = String(input);
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.includes('/api/v3/views/approval_requests') && method === 'GET') {
          if (queueResponse === 'network') {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          if (queueResponse) {
            return Promise.resolve(
              new Response(JSON.stringify(queueResponse.body), {
                status: queueResponse.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { requests: [fakeRequest('req-1')] },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.includes('/api/approvals/') && url.includes('/decide') && method === 'POST') {
          if (decideResponse === 'network') {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          if (decideResponse) {
            return Promise.resolve(
              new Response(JSON.stringify(decideResponse.body), {
                status: decideResponse.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify({ status: 'APPROVED' }), {
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
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): { root: HTMLElement; setBranch: (b: string) => void } {
    const [branch, setBranch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
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

  // ── Catch 1: loadQueue (Approvals.tsx:178) ──────────────────────────────────

  it('catch 1 (loadQueue): (1) network TypeError shows error.network in TH and EN', async () => {
    queueResponse = 'network';
    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.network']);
    });

    setLang('en');
    // Re-mount under English
    dispose?.();
    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedEn['error.network']);
    });
  });

  it('catch 1 (loadQueue): (2) 500 refusal shows error.serverError', async () => {
    queueResponse = {
      status: 500,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Internal server error 500',
        },
      },
    };
    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
      expect(alert!.textContent).not.toContain('Internal server error 500');
    });
  });

  it('catch 1 (loadQueue): (3) refusal with unmapped code and message shows server message', async () => {
    queueResponse = {
      status: 400,
      body: {
        error: {
          code: 'unmappedApprovalsQueueCode',
          message: 'Server unmapped queue error message',
        },
      },
    };
    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain('Server unmapped queue error message');
      expect(alert!.textContent).not.toContain(sharedTh['error.serverError']);
      expect(alert!.textContent).not.toContain(sharedTh['error.network']);
    });
  });

  // ── Catch 2: handleApprove (Approvals.tsx:56) ───────────────────────────────

  it('catch 2 (handleApprove): (1) network TypeError shows error.network in TH and EN', async () => {
    decideResponse = 'network';
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const approveBtn = root.querySelector('button.primary') as HTMLButtonElement;
    expect(approveBtn).not.toBeNull();
    approveBtn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain(sharedTh['error.network']);
    });

    setLang('en');
    dispose?.();
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const approveBtnEn = root.querySelector('button.primary') as HTMLButtonElement;
    expect(approveBtnEn).not.toBeNull();
    approveBtnEn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain(sharedEn['error.network']);
    });
  });

  it('catch 2 (handleApprove): (2) 500 refusal shows error.serverError', async () => {
    decideResponse = {
      status: 500,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Internal decide error 500',
        },
      },
    };
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const approveBtn = root.querySelector('button.primary') as HTMLButtonElement;
    expect(approveBtn).not.toBeNull();
    approveBtn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain(sharedTh['error.serverError']);
      expect(bodyText).not.toContain('Internal decide error 500');
    });
  });

  it('catch 2 (handleApprove): (3) refusal with unmapped code and message shows server message', async () => {
    decideResponse = {
      status: 400,
      body: {
        error: {
          code: 'unmappedApproveCode',
          message: 'Server unmapped approve error message',
        },
      },
    };
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const approveBtn = root.querySelector('button.primary') as HTMLButtonElement;
    expect(approveBtn).not.toBeNull();
    approveBtn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain('Server unmapped approve error message');
      expect(bodyText).not.toContain(sharedTh['error.serverError']);
      expect(bodyText).not.toContain(sharedTh['error.network']);
    });
  });

  // ── Catch 3: handleReject (Approvals.tsx:88) ────────────────────────────────

  it('catch 3 (handleReject): (1) network TypeError shows error.network in TH and EN', async () => {
    decideResponse = 'network';
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const reasonInput = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'Reason for rejection';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtn = root.querySelector('button.danger') as HTMLButtonElement;
    expect(rejectBtn).not.toBeNull();
    rejectBtn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain(sharedTh['error.network']);
    });

    setLang('en');
    dispose?.();
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const reasonInputEn = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(reasonInputEn).not.toBeNull();
    reasonInputEn.value = 'Reason for rejection';
    reasonInputEn.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtnEn = root.querySelector('button.danger') as HTMLButtonElement;
    expect(rejectBtnEn).not.toBeNull();
    rejectBtnEn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain(sharedEn['error.network']);
    });
  });

  it('catch 3 (handleReject): (2) 500 refusal shows error.serverError', async () => {
    decideResponse = {
      status: 500,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Internal reject error 500',
        },
      },
    };
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const reasonInput = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'Reason for rejection';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtn = root.querySelector('button.danger') as HTMLButtonElement;
    expect(rejectBtn).not.toBeNull();
    rejectBtn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain(sharedTh['error.serverError']);
      expect(bodyText).not.toContain('Internal reject error 500');
    });
  });

  it('catch 3 (handleReject): (3) refusal with unmapped code and message shows server message', async () => {
    decideResponse = {
      status: 400,
      body: {
        error: {
          code: 'unmappedRejectCode',
          message: 'Server unmapped reject error message',
        },
      },
    };
    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const reasonInput = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'Reason for rejection';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtn = root.querySelector('button.danger') as HTMLButtonElement;
    expect(rejectBtn).not.toBeNull();
    rejectBtn.click();

    await vi.waitFor(() => {
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain('Server unmapped reject error message');
      expect(bodyText).not.toContain(sharedTh['error.serverError']);
      expect(bodyText).not.toContain(sharedTh['error.network']);
    });
  });

  // ── §4.3: Stale data cleared on branch switch before failed read ───────────

  it('stale data from earlier successful read is cleared when read on new branch fails', async () => {
    const { setBranch } = mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    queueResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'Branch 2 queue failure',
        },
      },
    };

    setBranch('BRANCH_2');

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });

    // Stale request from previous branch must be cleared
    expect(root.querySelector('[data-request="req-1"]')).toBeNull();
  });
});
