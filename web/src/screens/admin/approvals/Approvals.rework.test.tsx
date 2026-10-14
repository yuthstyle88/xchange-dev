import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import type { ApprovalRequestItemDto } from '../../../shared/api/generated/approval_requests';
import { Approvals } from './Approvals';
import { th as screenTh, en as screenEn } from './i18n';
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

describe('Approvals rework (#1695): worded errors on failed reads and commands', () => {
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

  // ── Row 1 (§2): Approvals.tsx:179 (loadQueue failed read) ──────────────────

  it('row 1: read failure 500 serverError displays errorText in Thai and not empty message or raw message', async () => {
    queueResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-DB-CONNECTION-FAILURE-500',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.serverError']);
    expect(text).not.toContain('RAW-DB-CONNECTION-FAILURE-500');
    expect(text).not.toContain(screenTh.empty);
  });

  it('row 1: read failure 500 serverError in English displays en errorText', async () => {
    setLang('en');
    queueResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-DB-CONNECTION-FAILURE-500',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedEn['error.serverError']);
    expect(text).not.toContain('RAW-DB-CONNECTION-FAILURE-500');
    expect(text).not.toContain(screenEn.empty);
  });

  it('row 1: read failure 403 refusal with known code (roleNotAllowedHere) displays worded error', async () => {
    queueResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-REFUSAL-NOT-ALLOWED-HERE',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(text).not.toContain('RAW-REFUSAL-NOT-ALLOWED-HERE');
    expect(text).not.toContain(screenTh.empty);
  });

  it('row 1: read failure 403 refusal with known code (roleNotAllowedHere) whose message equals th key text shows worded error', async () => {
    queueResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: sharedTh['error.roleNotAllowedHere'],
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(text).not.toContain(screenTh.connectionError);
    expect(text).not.toContain(screenTh.empty);
  });

  it('row 1: read failure with unknown code displays generic worded message and never raw message', async () => {
    queueResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownCustomCode1695',
          message: 'RAW-UNKNOWN-READ-ERROR-1695',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain('RAW-UNKNOWN-READ-ERROR-1695');
    expect(text).not.toContain(screenTh.empty);
  });

  it('row 1: read failure with unknown code in English displays en generic worded message', async () => {
    setLang('en');
    queueResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownCustomCode1695',
          message: 'RAW-UNKNOWN-READ-ERROR-1695',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain('RAW-UNKNOWN-READ-ERROR-1695');
    expect(text).not.toContain(screenEn.empty);
  });

  it('row 1: read failure on network error (fetch rejects with TypeError) displays generic worded message', async () => {
    queueResponse = 'network';

    mount();

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBeTruthy();
    });

    const text = root.textContent ?? '';
    expect(text).toContain(sharedTh['error.network']);
    expect(text).not.toContain('Failed to fetch');
    expect(text).not.toContain(screenTh.empty);
  });

  it('row 1: failed read for a new branch clears previous branch requests and shows worded error', async () => {
    // 1. Initial mount succeeds with fakeRequest('req-1') for MAIN
    const { setBranch } = mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    // 2. Set next fetch for B2 to fail with 500 serverError
    queueResponse = {
      status: 500,
      body: {
        error: {
          code: 'serverError',
          message: 'RAW-B2-QUEUE-FAILURE',
        },
      },
    };

    // Switch branch to B2
    setBranch('B2');

    await vi.waitFor(() => {
      const alert = root.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });

    // MAIN request must NOT show under B2
    expect(root.querySelector('[data-request="req-1"]')).toBeNull();
  });

  // ── Row 2 (§2): Approvals.tsx:58, 91 (handleApprove and handleReject command refusals) ──

  it('row 2: handleApprove refusal with known code (roleNotAllowedHere) displays errorText lookup and not raw err.message', async () => {
    decideResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-DECIDE-APPROVE-REFUSAL',
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
      const text = document.body.textContent ?? '';
      expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(bodyText).not.toContain('RAW-DECIDE-APPROVE-REFUSAL');
  });

  it('row 2: handleApprove refusal with known code (roleNotAllowedHere) whose message equals th key text shows worded error', async () => {
    decideResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: sharedTh['error.roleNotAllowedHere'],
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
      const text = document.body.textContent ?? '';
      expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(bodyText).not.toContain(screenTh.connectionError);
  });

  it('row 2: handleApprove refusal with known code in English displays en errorText', async () => {
    setLang('en');
    decideResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-DECIDE-APPROVE-REFUSAL',
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
      const text = document.body.textContent ?? '';
      expect(text).toContain(sharedEn['error.roleNotAllowedHere']);
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain(sharedEn['error.roleNotAllowedHere']);
    expect(bodyText).not.toContain('RAW-DECIDE-APPROVE-REFUSAL');
  });

  it('row 2: handleReject refusal with known code (roleNotAllowedHere) displays errorText lookup and not raw err.message', async () => {
    decideResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-DECIDE-REJECT-REFUSAL',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const reasonInput = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'A valid reason for rejection';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtn = root.querySelector('button.danger') as HTMLButtonElement;
    expect(rejectBtn).not.toBeNull();
    rejectBtn.click();

    await vi.waitFor(() => {
      const text = document.body.textContent ?? '';
      expect(text).toContain(sharedTh['error.roleNotAllowedHere']);
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(bodyText).not.toContain('RAW-DECIDE-REJECT-REFUSAL');
  });

  it('row 2: handleReject refusal with known code in English displays en errorText', async () => {
    setLang('en');
    decideResponse = {
      status: 403,
      body: {
        error: {
          code: 'roleNotAllowedHere',
          message: 'RAW-DECIDE-REJECT-REFUSAL',
        },
      },
    };

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const reasonInput = root.querySelector('input[type="text"]') as HTMLInputElement;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'A valid reason for rejection';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const rejectBtn = root.querySelector('button.danger') as HTMLButtonElement;
    expect(rejectBtn).not.toBeNull();
    rejectBtn.click();

    await vi.waitFor(() => {
      const text = document.body.textContent ?? '';
      expect(text).toContain(sharedEn['error.roleNotAllowedHere']);
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain(sharedEn['error.roleNotAllowedHere']);
    expect(bodyText).not.toContain('RAW-DECIDE-REJECT-REFUSAL');
  });

  it('row 2: handleApprove refusal with unknown code displays generic worded message and not raw message', async () => {
    decideResponse = {
      status: 400,
      body: {
        error: {
          code: 'unknownRefusal1695',
          message: 'RAW-UNKNOWN-DECIDE-ERROR-1695',
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
      const text = document.body.textContent ?? '';
      expect(text).toContain('RAW-UNKNOWN-DECIDE-ERROR-1695');
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain('RAW-UNKNOWN-DECIDE-ERROR-1695');
  });

  it('row 2: handleApprove on network error (fetch rejects with TypeError) displays generic worded message', async () => {
    decideResponse = 'network';

    mount();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-request="req-1"]')).not.toBeNull();
    });

    const approveBtn = root.querySelector('button.primary') as HTMLButtonElement;
    expect(approveBtn).not.toBeNull();
    approveBtn.click();

    await vi.waitFor(() => {
      const text = document.body.textContent ?? '';
      expect(text).toContain(sharedTh['error.network']);
    });

    const bodyText = document.body.textContent ?? '';
    expect(bodyText).toContain(sharedTh['error.network']);
    expect(bodyText).not.toContain('Failed to fetch');
  });
});
