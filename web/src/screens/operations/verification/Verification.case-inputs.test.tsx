import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_A_ID = '11111111-1111-1111-1111-111111111111';
const CASE_B_ID = '22222222-2222-2222-2222-222222222222';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const QUEUE_ITEMS = [
  {
    id: CASE_A_ID,
    status: 'UNDER_REVIEW',
    submittedAt: '2026-10-14T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
  {
    id: CASE_B_ID,
    status: 'UNDER_REVIEW',
    submittedAt: '2026-10-14T05:00:00Z',
    customerDisplayName: 'Alice Smith',
    identityLast4: '5678',
    branchId: null,
  },
];

const CASE_A_DATA = {
  case: {
    id: CASE_A_ID,
    status: 'UNDER_REVIEW',
    version: 1,
    submittedAt: '2026-10-14T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
  documents: [],
  audit: [],
};

const CASE_B_DATA = {
  case: {
    id: CASE_B_ID,
    status: 'UNDER_REVIEW',
    version: 3,
    submittedAt: '2026-10-14T05:00:00Z',
    customerDisplayName: 'Alice Smith',
    identityLast4: '5678',
    branchId: null,
  },
  documents: [],
  audit: [],
};

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function errorResponse(code: string, message: string, status: number): Response {
  return new Response(JSON.stringify({ error: { code, message } }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

interface RecordedRequest {
  method: string;
  pathname: string;
  search: string;
  body: unknown;
}

describe('VerificationScreen case decision input isolation (#2457)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: RecordedRequest[] = [];

  async function settle(): Promise<void> {
    for (let i = 0; i < 15; i++) {
      await vi.advanceTimersByTimeAsync(0);
    }
  }

  function mountScreen(): void {
    dispose = render(() => <VerificationScreen />, host);
  }

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const parsed = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }
      sentRequests.push({
        method,
        pathname: parsed.pathname,
        search: parsed.search,
        body,
      });

      if (parsed.pathname === '/api/v3/views/me' || parsed.pathname === '/api/v3/views/staff_me') {
        return jsonResponse({ version: 1, changed: true, data: REVIEWER_SESSION });
      }

      if (parsed.pathname === '/api/v3/views/verification_policy') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: { branchReviewAllowed: false },
        });
      }

      if (parsed.pathname === '/api/v3/views/verification_queue') {
        return jsonResponse({
          version: 10,
          changed: true,
          data: QUEUE_ITEMS,
        });
      }

      if (parsed.pathname === '/api/v3/views/verification_case') {
        const filterStr = parsed.searchParams.get('filter');
        let id = '';
        if (filterStr) {
          try {
            id = (JSON.parse(filterStr) as { id?: string }).id ?? '';
          } catch {
            id = '';
          }
        }
        if (!id) {
          id = parsed.searchParams.get('id') ?? '';
        }
        const payload = id === CASE_B_ID ? CASE_B_DATA : CASE_A_DATA;
        return jsonResponse({
          version: payload.case.version,
          changed: true,
          data: payload,
        });
      }

      if (parsed.pathname.endsWith('/approve')) {
        const reqBody = body as { id?: string; expectedVersion?: number; expiresAt?: string } | null;
        const targetId = reqBody?.id ?? CASE_B_ID;
        const baseCase = targetId === CASE_B_ID ? CASE_B_DATA.case : CASE_A_DATA.case;
        return jsonResponse({
          ...baseCase,
          status: 'VERIFIED',
          version: (reqBody?.expectedVersion ?? baseCase.version) + 1,
          expiresAt: reqBody?.expiresAt ?? '2030-12-31',
        });
      }

      if (parsed.pathname.endsWith('/reject')) {
        const reqBody = body as {
          id?: string;
          expectedVersion?: number;
          reasonCode?: string;
        } | null;
        const targetId = reqBody?.id ?? CASE_B_ID;
        const baseCase = targetId === CASE_B_ID ? CASE_B_DATA.case : CASE_A_DATA.case;
        return jsonResponse({
          ...baseCase,
          status: 'REJECTED',
          version: (reqBody?.expectedVersion ?? baseCase.version) + 1,
          decisionReasonCode: reqBody?.reasonCode ?? 'DOCUMENT_UNREADABLE',
        });
      }

      return errorResponse('notFound', `Unhandled ${parsed.pathname}`, 404);
    });
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = null;
    }
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("row 1: Type a date on A, open B, Approve -> The POST for B carries no date from A (empty or B's own input)", async () => {
    mountScreen();
    await settle();

    // Open Case A and type an expiry date
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    const inputA = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(inputA).not.toBeNull();
    inputA!.value = '2030-12-31';
    inputA!.dispatchEvent(new Event('input', { bubbles: true }));

    // Switch to Case B
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    const inputB = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(inputB).not.toBeNull();
    expect(inputB!.value).toBe('');

    // Clicking Approve on B without entering B's date must not send A's date
    host.querySelector<HTMLButtonElement>('[data-action="approve"]')!.click();
    await settle();

    const prematureApproves = sentRequests.filter((r) => r.pathname.endsWith('/approve'));
    expect(prematureApproves).toEqual([]);

    // Typing B's own date and clicking Approve sends B's date
    inputB!.value = '2031-06-15';
    inputB!.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-action="approve"]')!.click();
    await settle();

    const approveReqs = sentRequests.filter((r) => r.pathname.endsWith('/approve'));
    expect(approveReqs).toHaveLength(1);
    expect(approveReqs[0].pathname).toBe(
      `/api/operations/verification/cases/${CASE_B_ID}/approve`,
    );
    expect(approveReqs[0].body).toEqual({
      id: CASE_B_ID,
      expectedVersion: 3,
      expiresAt: '2031-06-15',
    });
  });

  it('row 2: Pick a reason on A, open B -> The reason select shows the default', async () => {
    mountScreen();
    await settle();

    // Open Case A and select a non-default reason code
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    host.querySelector<HTMLButtonElement>('[data-action="reject"]')!.click();
    await settle();

    const selectA = host.querySelector<HTMLSelectElement>('select[name="reasonCode"]');
    expect(selectA).not.toBeNull();
    selectA!.value = 'SUSPECTED_FRAUD';
    selectA!.dispatchEvent(new Event('change', { bubbles: true }));
    expect(selectA!.value).toBe('SUSPECTED_FRAUD');

    // Open Case B and open the reason panel
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    host.querySelector<HTMLButtonElement>('[data-action="reject"]')!.click();
    await settle();

    const selectB = host.querySelector<HTMLSelectElement>('select[name="reasonCode"]');
    expect(selectB).not.toBeNull();
    expect(selectB!.value).toBe('DOCUMENT_UNREADABLE');

    host.querySelector<HTMLButtonElement>('[data-action="confirm-reject"]')!.click();
    await settle();

    const rejectReqs = sentRequests.filter((r) => r.pathname.endsWith('/reject'));
    expect(rejectReqs).toHaveLength(1);
    expect(rejectReqs[0].body).toEqual({
      id: CASE_B_ID,
      expectedVersion: 3,
      reasonCode: 'DOCUMENT_UNREADABLE',
    });
  });

  it('row 3: Approve A ok, open B -> Empty expiry input', async () => {
    mountScreen();
    await settle();

    // Open Case A and approve it with an expiry date
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    const inputA = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(inputA).not.toBeNull();
    inputA!.value = '2030-12-31';
    inputA!.dispatchEvent(new Event('input', { bubbles: true }));

    host.querySelector<HTMLButtonElement>('[data-action="approve"]')!.click();
    await settle();

    expect(host.querySelector('[data-testid="case-status"]')?.textContent).toBe('VERIFIED');

    // Open Case B: its expiry input must be empty
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    const inputB = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(inputB).not.toBeNull();
    expect(inputB!.value).toBe('');
  });

  it('edge case: Re-opening a case after switching does not leak previous inputs across cases', async () => {
    mountScreen();
    await settle();

    // Open Case A, type a date and pick a reason
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    const inputA = host.querySelector<HTMLInputElement>('input[name="expiresAt"]')!;
    inputA.value = '2030-12-31';
    inputA.dispatchEvent(new Event('input', { bubbles: true }));

    host.querySelector<HTMLButtonElement>('[data-action="reject"]')!.click();
    await settle();
    const selectA = host.querySelector<HTMLSelectElement>('select[name="reasonCode"]')!;
    selectA.value = 'DETAILS_MISMATCH';
    selectA.dispatchEvent(new Event('change', { bubbles: true }));

    // Switch to Case B and then re-open Case A
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    const reopenedInputA = host.querySelector<HTMLInputElement>('input[name="expiresAt"]')!;
    expect(reopenedInputA.value).toBe('');

    host.querySelector<HTMLButtonElement>('[data-action="reject"]')!.click();
    await settle();
    const reopenedSelectA = host.querySelector<HTMLSelectElement>('select[name="reasonCode"]')!;
    expect(reopenedSelectA.value).toBe('DOCUMENT_UNREADABLE');
  });
});
