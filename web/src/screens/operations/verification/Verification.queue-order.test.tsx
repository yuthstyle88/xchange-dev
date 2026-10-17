import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const QUEUE_POLL_MS = 30_000;

const CASE_A_ID = '11111111-1111-1111-1111-111111111111';
const CASE_C_ID = '33333333-3333-3333-3333-333333333333';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const ITEM_A = {
  id: CASE_A_ID,
  status: 'SUBMITTED',
  submittedAt: '2026-10-14T06:00:00Z',
  customerDisplayName: 'Somchai Jaidee',
  identityLast4: '1234',
  branchId: 'MAIN',
};

const ITEM_C = {
  id: CASE_C_ID,
  status: 'UNDER_REVIEW',
  submittedAt: '2026-10-14T05:00:00Z',
  customerDisplayName: 'Alice Smith',
  identityLast4: '5678',
  branchId: null,
};

const CASE_C_DATA = {
  case: {
    id: CASE_C_ID,
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

function queueAnswer(version: number, items: unknown[]): Response {
  return jsonResponse({ version, changed: true, data: items });
}

interface RecordedRequest {
  method: string;
  pathname: string;
  search: string;
  body: unknown;
}

describe('VerificationScreen queue read ordering (#2454)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: RecordedRequest[] = [];
  // Queue reads take the next planned answer in order; when the plan is empty
  // they answer with the default queue.
  let queuePlan: Array<Promise<Response> | (() => Response)> = [];
  let branchReviewAllowed = false;

  async function settle(): Promise<void> {
    for (let i = 0; i < 15; i++) {
      await vi.advanceTimersByTimeAsync(0);
    }
  }

  function mountScreen(): void {
    dispose = render(() => <VerificationScreen />, host);
  }

  function queueRequests(): RecordedRequest[] {
    return sentRequests.filter((r) => r.pathname === '/api/v3/views/verification_queue');
  }

  function caseRow(id: string): HTMLElement | null {
    return host.querySelector<HTMLElement>(`[data-case-id="${id}"]`);
  }

  function refreshButton(): HTMLButtonElement {
    const btn = host.querySelector<HTMLButtonElement>('[data-action="refresh"]');
    expect(btn).not.toBeNull();
    return btn!;
  }

  async function startPoll(): Promise<void> {
    await vi.advanceTimersByTimeAsync(QUEUE_POLL_MS);
    await settle();
  }

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];
    queuePlan = [];
    branchReviewAllowed = false;

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
      sentRequests.push({ method, pathname: parsed.pathname, search: parsed.search, body });

      if (parsed.pathname === '/api/v3/views/me' || parsed.pathname === '/api/v3/views/staff_me') {
        return jsonResponse({ version: 1, changed: true, data: REVIEWER_SESSION });
      }

      if (parsed.pathname === '/api/v3/views/verification_policy') {
        return jsonResponse({ version: 1, changed: true, data: { branchReviewAllowed } });
      }

      if (parsed.pathname === '/api/v3/views/verification_queue') {
        const next = queuePlan.shift();
        if (next === undefined) return queueAnswer(10, [ITEM_A, ITEM_C]);
        return typeof next === 'function' ? next() : next;
      }

      if (parsed.pathname === '/api/v3/views/verification_case') {
        return jsonResponse({ version: 3, changed: true, data: CASE_C_DATA });
      }

      if (parsed.pathname.endsWith('/approve')) {
        const reqBody = body as { expectedVersion?: number; expiresAt?: string } | null;
        return jsonResponse({
          ...CASE_C_DATA.case,
          status: 'VERIFIED',
          version: (reqBody?.expectedVersion ?? 3) + 1,
          expiresAt: reqBody?.expiresAt ?? '2030-12-31',
        });
      }

      if (parsed.pathname === '/api/operations/verification/branch-cases') {
        return jsonResponse({ id: '44444444-4444-4444-4444-444444444444', status: 'SUBMITTED', version: 1 });
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

  it('row 1: poll in flight, decide C, full reload answers without C, then the old poll answers with C -> C is not in the queue; lastQueueVersion is the reload\'s', async () => {
    branchReviewAllowed = true;
    mountScreen();
    await settle();
    expect(caseRow(CASE_C_ID)).not.toBeNull();

    // The poll is captured before the decision and stays in flight.
    const oldPoll = createDeferred<Response>();
    queuePlan.push(oldPoll.promise);
    await startPoll();

    // Decide case C.
    caseRow(CASE_C_ID)!.click();
    await settle();
    const expiresInput = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(expiresInput).not.toBeNull();
    expiresInput!.value = '2030-12-31';
    expiresInput!.dispatchEvent(new Event('input', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-action="approve"]')!.click();
    await settle();
    expect(sentRequests.filter((r) => r.pathname.endsWith('/approve'))).toHaveLength(1);

    // Full reload (the at-branch submit reloads the queue without `since`) answers without C.
    queuePlan.push(() => queueAnswer(12, [ITEM_A]));
    const custInput = host.querySelector<HTMLInputElement>('input[name="customerId"]');
    expect(custInput).not.toBeNull();
    custInput!.value = 'CUST-1';
    custInput!.dispatchEvent(new Event('input', { bubbles: true }));
    const fileInput = host.querySelector<HTMLInputElement>('input[name="file"]')!;
    const file = new File(['abc'], 'passport.png', { type: 'image/png' });
    Object.defineProperty(fileInput, 'files', { value: [file], configurable: true });
    fileInput.dispatchEvent(new Event('change', { bubbles: true }));
    host.querySelector<HTMLButtonElement>('[data-action="submit-branch-case"]')!.click();
    await settle();
    expect(caseRow(CASE_C_ID)).toBeNull();

    // The old poll answers last, still carrying C.
    oldPoll.resolve(queueAnswer(11, [ITEM_A, ITEM_C]));
    await settle();

    expect(caseRow(CASE_C_ID)).toBeNull();
    expect(caseRow(CASE_A_ID)).not.toBeNull();

    // lastQueueVersion is the reload's: the next poll asks since=12.
    await startPoll();
    const last = queueRequests().at(-1)!;
    expect(new URLSearchParams(last.search).get('since')).toBe('12');
  });

  it('row 2: old poll fails after a newer read succeeded -> no error / retry shown', async () => {
    mountScreen();
    await settle();

    const oldPoll = createDeferred<Response>();
    queuePlan.push(oldPoll.promise);
    await startPoll();

    queuePlan.push(() => queueAnswer(12, [ITEM_A]));
    refreshButton().click();
    await settle();
    expect(caseRow(CASE_C_ID)).toBeNull();

    oldPoll.resolve(errorResponse('serverError', 'Old poll failed', 503));
    await settle();

    expect(host.querySelector('[data-error]')).toBeNull();
    expect(host.querySelector('[data-action="retry"]')).toBeNull();
    expect(caseRow(CASE_A_ID)).not.toBeNull();
  });

  it('row 3: manual refresh while a poll is in flight -> the later-started read wins', async () => {
    mountScreen();
    await settle();

    const oldPoll = createDeferred<Response>();
    queuePlan.push(oldPoll.promise);
    await startPoll();

    const refresh = createDeferred<Response>();
    queuePlan.push(refresh.promise);
    refreshButton().click();
    await settle();

    refresh.resolve(queueAnswer(12, [ITEM_C]));
    await settle();
    oldPoll.resolve(queueAnswer(11, [ITEM_A, ITEM_C]));
    await settle();

    expect(caseRow(CASE_C_ID)).not.toBeNull();
    expect(caseRow(CASE_A_ID)).toBeNull();

    await startPoll();
    const last = queueRequests().at(-1)!;
    expect(new URLSearchParams(last.search).get('since')).toBe('12');
  });

  it('edge: a 403 refusal on the latest read still sets the refused state', async () => {
    mountScreen();
    await settle();

    const oldPoll = createDeferred<Response>();
    queuePlan.push(oldPoll.promise);
    await startPoll();

    queuePlan.push(() => errorResponse('roleNotAllowed', 'Not allowed', 403));
    refreshButton().click();
    await settle();
    expect(host.querySelector('[data-refused]')).not.toBeNull();

    oldPoll.resolve(queueAnswer(11, [ITEM_A, ITEM_C]));
    await settle();

    expect(host.querySelector('[data-refused]')).not.toBeNull();
    expect(caseRow(CASE_A_ID)).toBeNull();
    expect(caseRow(CASE_C_ID)).toBeNull();
  });

  it('edge: polling stops when unauthorized, even if an older poll answers OK afterwards', async () => {
    mountScreen();
    await settle();

    const oldPoll = createDeferred<Response>();
    queuePlan.push(oldPoll.promise);
    await startPoll();

    queuePlan.push(() => errorResponse('roleNotAllowed', 'Not allowed', 403));
    refreshButton().click();
    await settle();

    oldPoll.resolve(queueAnswer(11, [ITEM_A, ITEM_C]));
    await settle();

    const before = queueRequests().length;
    await startPoll();
    await startPoll();
    expect(queueRequests().length).toBe(before);
  });
});
