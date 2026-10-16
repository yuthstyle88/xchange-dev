import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_A_ID = '11111111-1111-1111-1111-111111111111';
const CASE_B_ID = '22222222-2222-2222-2222-222222222222';
const CASE_C_ID = '33333333-3333-3333-3333-333333333333';
const DOC_A_ID = 'dddddddd-1111-1111-1111-111111111111';
const DOC_B_ID = 'dddddddd-2222-2222-2222-222222222222';
const DOC_C_ID = 'dddddddd-3333-3333-3333-333333333333';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

interface QueueItem {
  id: string;
  status: string;
  submittedAt: string;
  customerDisplayName: string;
  identityLast4: string;
  branchId: string | null;
}

interface RecordedRequest {
  method: string;
  pathname: string;
  search: string;
  body: unknown;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

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

describe('#2559 VerificationScreen decision queue refresh', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: RecordedRequest[] = [];
  let queueItems: QueueItem[] = [];
  let queueVersion = 10;
  let approveHandler: ((body: unknown) => Promise<Response>) | null = null;

  async function settle(): Promise<void> {
    for (let i = 0; i < 15; i++) {
      await vi.advanceTimersByTimeAsync(0);
    }
  }

  function mountScreen(): void {
    dispose = render(() => <VerificationScreen />, host);
  }

  function queueCalls(): RecordedRequest[] {
    return sentRequests.filter(
      (r) => r.method === 'GET' && r.pathname === '/api/v3/views/verification_queue',
    );
  }

  function rowStatus(caseId: string): string {
    const row = host.querySelector<HTMLElement>(`[data-case-id="${caseId}"]`);
    expect(row, `row for ${caseId}`).not.toBeNull();
    return row!.children[2]?.textContent?.trim() ?? '';
  }

  function setDate(value: string): void {
    const input = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(input, 'expiresAt input').not.toBeNull();
    input!.value = value;
    input!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function pickReason(code: string): void {
    const select = host.querySelector<HTMLSelectElement>('select[name="reasonCode"]');
    expect(select, 'reasonCode select').not.toBeNull();
    select!.value = code;
    select!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];
    approveHandler = null;
    queueVersion = 10;
    queueItems = [
      {
        id: CASE_A_ID,
        status: 'SUBMITTED',
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
      {
        id: CASE_C_ID,
        status: 'VERIFIED',
        submittedAt: '2026-10-14T04:00:00Z',
        customerDisplayName: 'Bob Stone',
        identityLast4: '9999',
        branchId: 'MAIN',
      },
    ];

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
          version: ++queueVersion,
          changed: true,
          data: queueItems,
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
        if (!id) id = parsed.searchParams.get('id') ?? '';
        const item = queueItems.find((q) => q.id === id) ?? queueItems[0]!;
        const docId = id === CASE_B_ID ? DOC_B_ID : id === CASE_C_ID ? DOC_C_ID : DOC_A_ID;
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            case: {
              id: item.id,
              status: item.status,
              version: 1,
              submittedAt: item.submittedAt,
              customerDisplayName: item.customerDisplayName,
              identityLast4: item.identityLast4,
              branchId: item.branchId,
            },
            documents: [
              {
                id: docId,
                documentType: 'PASSPORT',
                mime: 'image/png',
                sizeBytes: 1024,
                uploadedAt: item.submittedAt,
              },
            ],
            audit: [],
          },
        });
      }

      if (parsed.pathname.includes('/documents/') && parsed.pathname.endsWith('/link')) {
        const docId = parsed.pathname.split('/documents/')[1]?.split('/link')[0] ?? '';
        return jsonResponse({
          url: `/api/verification/content/mock-token-${docId}`,
          expiresAt: '2030-01-01T00:00:00Z',
        });
      }

      if (parsed.pathname.startsWith('/api/verification/content/')) {
        return new Response(new Uint8Array([1, 2, 3, 4]), {
          status: 200,
          headers: { 'Content-Type': 'image/png' },
        });
      }

      if (parsed.pathname.endsWith('/approve')) {
        if (approveHandler) {
          return approveHandler(body);
        }
        const b = body as { id?: string; expiresAt?: string; expectedVersion?: number } | null;
        const id = b?.id ?? CASE_A_ID;
        const itm = queueItems.find((q) => q.id === id);
        if (itm) itm.status = 'VERIFIED';
        return jsonResponse({
          id,
          status: 'VERIFIED',
          version: (b?.expectedVersion ?? 1) + 1,
          expiresAt: b?.expiresAt ?? '2030-12-31',
        });
      }

      if (parsed.pathname.endsWith('/reject')) {
        const b = body as { id?: string; reasonCode?: string; expectedVersion?: number } | null;
        const id = b?.id ?? CASE_B_ID;
        const itm = queueItems.find((q) => q.id === id);
        if (itm) itm.status = 'REJECTED';
        return jsonResponse({
          id,
          status: 'REJECTED',
          version: (b?.expectedVersion ?? 1) + 1,
          reasonCode: b?.reasonCode,
        });
      }

      if (parsed.pathname.endsWith('/request-update')) {
        const b = body as { id?: string; reasonCode?: string; expectedVersion?: number } | null;
        const id = b?.id ?? CASE_B_ID;
        const itm = queueItems.find((q) => q.id === id);
        if (itm) itm.status = 'UPDATE_REQUESTED';
        return jsonResponse({
          id,
          status: 'UPDATE_REQUESTED',
          version: (b?.expectedVersion ?? 1) + 1,
          reasonCode: b?.reasonCode,
        });
      }

      if (parsed.pathname.endsWith('/revoke')) {
        const b = body as { id?: string; reasonCode?: string; expectedVersion?: number } | null;
        const id = b?.id ?? CASE_C_ID;
        const itm = queueItems.find((q) => q.id === id);
        if (itm) itm.status = 'REVOKED';
        return jsonResponse({
          id,
          status: 'REVOKED',
          version: (b?.expectedVersion ?? 1) + 1,
          reasonCode: b?.reasonCode,
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

  it('row 1: Approve succeeds -> A queue read is sent right after the decision; the row shows the new status', async () => {
    mountScreen();
    await settle();

    // Select Case A
    const rowA = host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`);
    rowA!.click();
    await settle();

    const beforeReads = queueCalls().length;
    expect(rowStatus(CASE_A_ID)).toBe('SUBMITTED');

    // Fill date and click approve
    setDate('2030-01-01');
    const approveBtn = host.querySelector<HTMLElement>('[data-action="approve"]');
    expect(approveBtn, 'approve button').not.toBeNull();
    approveBtn!.click();
    await settle();

    // Must have sent an extra queue read right after approve
    expect(queueCalls().length).toBe(beforeReads + 1);
    expect(rowStatus(CASE_A_ID)).toBe('VERIFIED');
  });

  it('row 2a: Reject succeeds -> queue read is sent and row shows REJECTED', async () => {
    mountScreen();
    await settle();

    // Select Case B
    const rowB = host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`);
    rowB!.click();
    await settle();

    const beforeReads = queueCalls().length;
    expect(rowStatus(CASE_B_ID)).toBe('UNDER_REVIEW');

    // Click reject trigger, choose reason, confirm
    host.querySelector<HTMLElement>('[data-action="reject"]')!.click();
    await settle();
    pickReason('DOCUMENT_EXPIRED');
    host.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
    await settle();

    expect(queueCalls().length).toBe(beforeReads + 1);
    expect(rowStatus(CASE_B_ID)).toBe('REJECTED');
  });

  it('row 2b: Request update succeeds -> queue read is sent and row shows UPDATE_REQUESTED', async () => {
    mountScreen();
    await settle();

    // Select Case B
    const rowB = host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`);
    rowB!.click();
    await settle();

    const beforeReads = queueCalls().length;
    expect(rowStatus(CASE_B_ID)).toBe('UNDER_REVIEW');

    host.querySelector<HTMLElement>('[data-action="request-update"]')!.click();
    await settle();
    pickReason('DOCUMENT_UNREADABLE');
    host.querySelector<HTMLElement>('[data-action="confirm-request-update"]')!.click();
    await settle();

    expect(queueCalls().length).toBe(beforeReads + 1);
    expect(rowStatus(CASE_B_ID)).toBe('UPDATE_REQUESTED');
  });

  it('row 2c: Revoke succeeds -> queue read is sent and row shows REVOKED', async () => {
    mountScreen();
    await settle();

    // Select Case C (already VERIFIED)
    const rowC = host.querySelector<HTMLElement>(`[data-case-id="${CASE_C_ID}"]`);
    rowC!.click();
    await settle();

    const beforeReads = queueCalls().length;
    expect(rowStatus(CASE_C_ID)).toBe('VERIFIED');

    host.querySelector<HTMLElement>('[data-action="revoke"]')!.click();
    await settle();
    pickReason('SUSPECTED_FRAUD');
    host.querySelector<HTMLElement>('[data-action="confirm-revoke"]')!.click();
    await settle();

    expect(queueCalls().length).toBe(beforeReads + 1);
    expect(rowStatus(CASE_C_ID)).toBe('REVOKED');
  });

  it('row 3: Decision fails -> No extra queue read (unchanged); retry succeeds -> queue read is sent', async () => {
    let failApprove = true;
    approveHandler = async () => {
      if (failApprove) {
        return errorResponse('internalError', 'Server failed', 500);
      }
      const itm = queueItems.find((q) => q.id === CASE_A_ID);
      if (itm) itm.status = 'VERIFIED';
      return jsonResponse({
        id: CASE_A_ID,
        status: 'VERIFIED',
        version: 2,
        expiresAt: '2030-01-01',
      });
    };

    mountScreen();
    await settle();

    const rowA = host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`);
    rowA!.click();
    await settle();

    setDate('2030-01-01');
    const beforeReads = queueCalls().length;

    // First attempt fails with 500
    host.querySelector<HTMLElement>('[data-action="approve"]')!.click();
    await settle();

    expect(host.querySelector('[data-error]')).not.toBeNull();
    // No extra queue read on failure
    expect(queueCalls().length).toBe(beforeReads);
    expect(rowStatus(CASE_A_ID)).toBe('SUBMITTED');

    // Retry succeeds
    failApprove = false;
    const retryBtn = host.querySelector<HTMLElement>('[data-action="retry"]');
    if (retryBtn) {
      retryBtn.click();
    } else {
      host.querySelector<HTMLElement>('[data-action="approve"]')!.click();
    }
    await settle();

    // Queue read must be sent once retry succeeds
    expect(queueCalls().length).toBe(beforeReads + 1);
    expect(rowStatus(CASE_A_ID)).toBe('VERIFIED');
  });

  it('row 4: Decision answer arrives after the reviewer switched case -> No extra queue read', async () => {
    const approveDeferred = createDeferred<Response>();
    approveHandler = () => approveDeferred.promise;

    mountScreen();
    await settle();

    // Open Case A
    const rowA = host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`);
    rowA!.click();
    await settle();

    setDate('2030-01-01');

    // Click approve for Case A (now in flight)
    host.querySelector<HTMLElement>('[data-action="approve"]')!.click();
    await settle();

    // Switch to Case B before Case A resolves
    const rowB = host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`);
    rowB!.click();
    await settle();

    const readsAfterSwitch = queueCalls().length;

    // Resolve Case A's approve now that reviewer is on Case B
    approveDeferred.resolve(
      jsonResponse({
        id: CASE_A_ID,
        status: 'VERIFIED',
        version: 2,
        expiresAt: '2030-01-01',
      }),
    );
    await settle();

    // The guard returns early; no queue read was sent for Case A
    expect(queueCalls().length).toBe(readsAfterSwitch);

    // Now decide active Case B: reject
    approveHandler = null;
    host.querySelector<HTMLElement>('[data-action="reject"]')!.click();
    await settle();
    pickReason('DOCUMENT_EXPIRED');
    host.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
    await settle();

    // Queue read sent for Case B
    expect(queueCalls().length).toBe(readsAfterSwitch + 1);
    expect(rowStatus(CASE_B_ID)).toBe('REJECTED');
  });
});
