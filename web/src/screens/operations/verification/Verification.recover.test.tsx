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

describe('VerificationScreen queue recovery and error clearing (#2158)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let queueStatus = 200;
  let queueCode = 'serverError';
  let queueVersion = 10;
  let queueData = [...QUEUE_ITEMS];
  let caseStatus = 200;
  let startReviewStatus = 200;
  let queueRequests: string[] = [];

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
    queueStatus = 200;
    queueCode = 'serverError';
    queueVersion = 10;
    queueData = [...QUEUE_ITEMS];
    caseStatus = 200;
    startReviewStatus = 200;
    queueRequests = [];

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
        if (queueStatus !== 200) {
          return errorResponse(queueCode, 'Queue read failed', queueStatus);
        }
        return jsonResponse({
          version: queueVersion,
          changed: true,
          data: queueData,
        });
      }

      if (parsed.pathname === '/api/v3/views/verification_case') {
        if (caseStatus !== 200) {
          return errorResponse('serverError', 'Case load failed', caseStatus);
        }
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
          return errorResponse('serverError', 'Start review failed', startReviewStatus);
        }
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

  it('row 1: first verification_queue read fails 503 -> error is shown with Retry and queue is empty', async () => {
    queueStatus = 503;
    mountScreen();
    await settle();

    const errorEl = host.querySelector('[data-error]');
    const retryBtn = host.querySelector('[data-action="retry"]');
    const rows = host.querySelectorAll('[data-case-row]');

    expect(errorEl).not.toBeNull();
    expect(retryBtn).not.toBeNull();
    expect(rows.length).toBe(0);
  });

  it('row 2: then Retry, and the read succeeds -> queue is shown and error is gone', async () => {
    queueStatus = 503;
    mountScreen();
    await settle();

    const retryBtn = host.querySelector('[data-action="retry"]') as HTMLButtonElement | null;
    expect(retryBtn).not.toBeNull();

    queueStatus = 200;
    retryBtn!.click();
    await settle();

    expect(host.querySelector('[data-error]')).toBeNull();
    const rows = host.querySelectorAll('[data-case-row]');
    expect(rows.length).toBe(2);
    expect(host.textContent).toContain('Somchai Jaidee');
  });

  it('row 3: first read fails, and the next poll tick succeeds without Retry -> queue is shown and error is gone', async () => {
    queueStatus = 503;
    mountScreen();
    await settle();

    expect(host.querySelector('[data-error]')).not.toBeNull();
    expect(host.querySelectorAll('[data-case-row]').length).toBe(0);

    queueStatus = 200;
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();

    expect(host.querySelector('[data-error]')).toBeNull();
    expect(host.querySelectorAll('[data-case-row]').length).toBe(2);
    expect(host.textContent).toContain('Somchai Jaidee');
  });

  it('row 4: a poll fails after a good load -> error is shown and queue keeps its last rows', async () => {
    queueStatus = 200;
    mountScreen();
    await settle();

    expect(host.querySelectorAll('[data-case-row]').length).toBe(2);
    expect(host.querySelector('[data-error]')).toBeNull();

    queueStatus = 503;
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();

    expect(host.querySelector('[data-error]')).not.toBeNull();
    expect(host.querySelector('[data-action="retry"]')).not.toBeNull();
    expect(host.querySelectorAll('[data-case-row]').length).toBe(2);
    expect(host.textContent).toContain('Somchai Jaidee');
  });

  it('row 5: then a poll succeeds -> that error is gone', async () => {
    queueStatus = 200;
    mountScreen();
    await settle();

    queueStatus = 503;
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();
    expect(host.querySelector('[data-error]')).not.toBeNull();

    queueStatus = 200;
    queueVersion = 11;
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();

    expect(host.querySelector('[data-error]')).toBeNull();
    expect(host.querySelectorAll('[data-case-row]').length).toBe(2);
  });

  it('row 6: a case action failed (its error shown), then a poll succeeds -> the action error stays', async () => {
    queueStatus = 200;
    mountScreen();
    await settle();

    const firstRow = host.querySelector(`[data-case-id="${CASE_ID_1}"]`) as HTMLElement | null;
    expect(firstRow).not.toBeNull();
    firstRow!.click();
    await settle();

    startReviewStatus = 500;
    const startReviewBtn = host.querySelector(
      '[data-action="start-review"]',
    ) as HTMLButtonElement | null;
    expect(startReviewBtn).not.toBeNull();
    startReviewBtn!.click();
    await settle();

    const errorBeforePoll = host.querySelector('[data-error]')?.textContent;
    expect(errorBeforePoll).toBeTruthy();

    queueStatus = 200;
    queueVersion = 12;
    await vi.advanceTimersByTimeAsync(30_000);
    await settle();

    const errorAfterPoll = host.querySelector('[data-error]')?.textContent;
    expect(errorAfterPoll).toBe(errorBeforePoll);
  });

  it('row 7: first read refused 403 roleNotAllowed -> refusal as today; no poll; no Retry', async () => {
    queueStatus = 403;
    queueCode = 'roleNotAllowed';
    mountScreen();
    await settle();

    expect(host.querySelector('[data-refused]')).not.toBeNull();
    expect(host.querySelector('[data-action="retry"]')).toBeNull();
    expect(queueRequests.length).toBe(1);

    await vi.advanceTimersByTimeAsync(60_000);
    await settle();

    expect(queueRequests.length).toBe(1);
    expect(host.querySelector('[data-action="retry"]')).toBeNull();
  });
});
