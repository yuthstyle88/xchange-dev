import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_ID_1 = '11111111-1111-1111-1111-111111111111';
const CASE_ID_2 = '22222222-2222-2222-2222-222222222222';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const QUEUE_ITEMS = [
  {
    id: CASE_ID_1,
    status: 'SUBMITTED',
    submittedAt: '2026-10-14T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
  {
    id: CASE_ID_2,
    status: 'UNDER_REVIEW',
    submittedAt: '2026-10-14T05:00:00Z',
    customerDisplayName: 'Alice Smith',
    identityLast4: '5678',
    branchId: null,
  },
];

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

describe('VerificationScreen #2742: Start review queue re-read', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let queueRequests: string[] = [];
  let queueData = [...QUEUE_ITEMS];
  let startReviewStatus = 200;
  let approveStatus = 200;

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
    queueRequests = [];
    queueData = QUEUE_ITEMS.map((item) => ({ ...item }));
    startReviewStatus = 200;
    approveStatus = 200;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const parsed = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

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
        queueRequests.push(`${method} ${parsed.pathname}${parsed.search}`);
        return jsonResponse({
          version: 10,
          changed: true,
          data: queueData,
        });
      }

      if (parsed.pathname === '/api/v3/views/verification_case') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            case: {
              id: CASE_ID_1,
              status: 'SUBMITTED',
              version: 1,
              submittedAt: '2026-10-14T06:00:00Z',
              customerDisplayName: 'Somchai Jaidee',
              identityLast4: '1234',
              branchId: 'MAIN',
            },
            documents: [],
            audit: [],
          },
        });
      }

      if (parsed.pathname.endsWith('/start-review')) {
        if (startReviewStatus !== 200) {
          return errorResponse('staleVersion', 'Start review failed', startReviewStatus);
        }
        // Update queue data so subsequent fetchQueue gets UNDER_REVIEW
        queueData = queueData.map((item) =>
          item.id === CASE_ID_1 ? { ...item, status: 'UNDER_REVIEW' } : item,
        );
        return jsonResponse({
          id: CASE_ID_1,
          status: 'UNDER_REVIEW',
          version: 2,
          submittedAt: '2026-10-14T06:00:00Z',
          customerDisplayName: 'Somchai Jaidee',
          identityLast4: '1234',
          branchId: 'MAIN',
        });
      }

      if (parsed.pathname.endsWith('/approve')) {
        if (approveStatus !== 200) {
          return errorResponse('staleVersion', 'Approve failed', approveStatus);
        }
        queueData = queueData.map((item) =>
          item.id === CASE_ID_1 ? { ...item, status: 'VERIFIED' } : item,
        );
        return jsonResponse({
          id: CASE_ID_1,
          status: 'VERIFIED',
          version: 3,
          submittedAt: '2026-10-14T06:00:00Z',
          customerDisplayName: 'Somchai Jaidee',
          identityLast4: '1234',
          branchId: 'MAIN',
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

  it('1. Open a SUBMITTED case and press Start review; it answers 200 UNDER_REVIEW -> A full verification_queue read (no since) is sent right after, and the queue row shows the new status', async () => {
    mountScreen();
    await settle();

    // Initial load: 1 queue request
    expect(queueRequests).toHaveLength(1);

    // Click case 1
    const caseRow = host.querySelector<HTMLElement>(`[data-case-row][data-case-id="${CASE_ID_1}"]`);
    expect(caseRow).not.toBeNull();
    caseRow!.click();
    await settle();

    const startReviewBtn = host.querySelector<HTMLButtonElement>('[data-action="start-review"]');
    expect(startReviewBtn).not.toBeNull();

    // Press Start review
    startReviewBtn!.click();
    await settle();

    // Must have sent a second verification_queue read without 'since'
    expect(queueRequests.length).toBeGreaterThanOrEqual(2);
    const lastQueueReq = queueRequests[queueRequests.length - 1]!;
    expect(lastQueueReq).toBe('GET /api/v3/views/verification_queue');
    expect(lastQueueReq).not.toContain('since=');

    // The queue row now shows the new status (UNDER_REVIEW / รอตรวจสอบ)
    const updatedRow = host.querySelector<HTMLElement>(`[data-case-row][data-case-id="${CASE_ID_1}"]`);
    expect(updatedRow).not.toBeNull();
    expect(updatedRow!.textContent).toContain('UNDER_REVIEW');
  });

  it('2. Start review fails (409 staleVersion or 500) -> No extra queue read; the error behaviour is unchanged (kept)', async () => {
    startReviewStatus = 409;
    mountScreen();
    await settle();

    expect(queueRequests).toHaveLength(1);

    const caseRow = host.querySelector<HTMLElement>(`[data-case-row][data-case-id="${CASE_ID_1}"]`);
    caseRow!.click();
    await settle();

    const startReviewBtn = host.querySelector<HTMLButtonElement>('[data-action="start-review"]');
    startReviewBtn!.click();
    await settle();

    // No extra queue read sent on failure
    expect(queueRequests).toHaveLength(1);
    expect(host.querySelector('[data-error]')).not.toBeNull();
  });

  it('3. Approve -> Still re-reads the queue (kept)', async () => {
    mountScreen();
    await settle();

    expect(queueRequests).toHaveLength(1);

    const caseRow = host.querySelector<HTMLElement>(`[data-case-row][data-case-id="${CASE_ID_1}"]`);
    caseRow!.click();
    await settle();

    // Set expiration date for approve
    const expiresInput = host.querySelector<HTMLInputElement>('[data-testid="expires-at-input"]') ??
      host.querySelector<HTMLInputElement>('input[type="date"]');
    expect(expiresInput).not.toBeNull();
    expiresInput!.value = '2026-12-31';
    expiresInput!.dispatchEvent(new Event('input', { bubbles: true }));
    expiresInput!.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    const approveBtn = host.querySelector<HTMLButtonElement>('[data-action="approve"]');
    expect(approveBtn).not.toBeNull();
    approveBtn!.click();
    await settle();

    // Queue read was sent on approve
    expect(queueRequests.length).toBeGreaterThanOrEqual(2);
    const lastQueueReq = queueRequests[queueRequests.length - 1]!;
    expect(lastQueueReq).toBe('GET /api/v3/views/verification_queue');
  });
});
