import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

// #2412: a second click on a decision button (or the at-branch Submit) while the
// first send is still in flight must not send again. A second concurrent send
// with the same body reuses the idempotency key; the server answers 409
// idempotencyConflict for a key that is still PENDING and keyedWrite then drops
// the key, so the user sees an error although the write worked.

const CASE_ID_1 = '11111111-1111-4111-8111-111111111111';
const CASE_ID_2 = '22222222-2222-4222-8222-222222222222';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'DOCUMENTS',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

interface RecordedWrite {
  pathname: string;
  key: string | null;
  rawBody: string;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (v: T) => void;
  reject: (e: unknown) => void;
}

function createDeferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

type WriteStep =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown }
  | { kind: 'deferred'; deferred: Deferred<Response> };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('#2412: VerificationScreen in-flight guard on decision and at-branch writes', () => {
  let writes: RecordedWrite[] = [];
  let writePlan: WriteStep[] = [];
  let pendingKeys = new Set<string>();
  let caseStatus: 'SUBMITTED' | 'UNDER_REVIEW' | 'VERIFIED' = 'UNDER_REVIEW';
  let branchReviewAllowed = false;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    writes = [];
    writePlan = [];
    pendingKeys = new Set<string>();
    caseStatus = 'UNDER_REVIEW';
    branchReviewAllowed = false;
    document.body.innerHTML = '';

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);

      if (url.pathname === '/api/v3/views/me' || url.pathname === '/api/v3/views/staff_me') {
        return jsonResponse({ version: 1, changed: true, data: REVIEWER_SESSION });
      }

      if (url.pathname === '/api/v3/views/verification_policy') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            policyVersion: 1,
            branchReviewAllowed,
            acceptedDocumentTypes: ['PASSPORT', 'NATIONAL_ID'],
            documentsRequired: 1,
          },
        });
      }

      if (url.pathname === '/api/v3/views/verification_queue') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: [
            {
              id: CASE_ID_1,
              status: caseStatus,
              submittedAt: '2026-10-14T06:00:00Z',
              customerDisplayName: 'Somchai Jaidee',
              identityLast4: '1234',
              branchId: 'MAIN',
            },
            {
              id: CASE_ID_2,
              status: caseStatus,
              submittedAt: '2026-10-14T05:00:00Z',
              customerDisplayName: 'Alice Smith',
              identityLast4: '5678',
              branchId: 'MAIN',
            },
          ],
        });
      }

      if (url.pathname === '/api/v3/views/verification_case') {
        const requestedId = url.searchParams.get('id') ?? CASE_ID_1;
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            case: {
              id: requestedId,
              status: caseStatus,
              version: 2,
              submittedAt: '2026-10-14T06:00:00Z',
              customerDisplayName: requestedId === CASE_ID_2 ? 'Alice Smith' : 'Somchai Jaidee',
              identityLast4: requestedId === CASE_ID_2 ? '5678' : '1234',
              branchId: 'MAIN',
            },
            documents: [],
            audit: [],
          },
        });
      }

      if (method === 'POST' && url.pathname.startsWith('/api/operations/verification/')) {
        const key = headers.get('idempotency-key');
        writes.push({
          pathname: url.pathname,
          key,
          rawBody: typeof init?.body === 'string' ? init.body : '',
        });
        // Server fact (pg_store.rs): a key still PENDING answers 409 idempotencyConflict.
        if (key && pendingKeys.has(key)) {
          return jsonResponse(
            { error: { code: 'idempotencyConflict', message: 'key still pending' } },
            409,
          );
        }
        const next = writePlan.shift();
        if (!next) {
          return jsonResponse({
            id: CASE_ID_1,
            status: caseStatus,
            version: 2,
            submittedAt: '2026-10-14T06:00:00Z',
          });
        }
        if (next.kind === 'network') throw new TypeError('Failed to fetch');
        if (next.kind === 'deferred') {
          if (key) pendingKeys.add(key);
          try {
            return await next.deferred.promise;
          } finally {
            if (key) pendingKeys.delete(key);
          }
        }
        return jsonResponse(next.body, next.status);
      }

      return jsonResponse({ error: { code: 'notFound', message: 'Not found' } }, 404);
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 15; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mount(): Promise<void> {
    dispose?.();
    document.body.innerHTML = '';
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <VerificationScreen />, host);
    await flush();
  }

  async function selectCase(caseId: string): Promise<void> {
    const row = document.querySelector<HTMLElement>(`[data-case-row][data-case-id="${caseId}"]`);
    expect(row, `case row ${caseId} must exist`).not.toBeNull();
    row!.click();
    await flush();
  }

  async function mountAndOpenCase(caseId = CASE_ID_1): Promise<void> {
    await mount();
    await selectCase(caseId);
  }

  function setExpiresDate(date: string): void {
    const dateInput = document.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(dateInput, 'expiresAt input must exist').not.toBeNull();
    dateInput!.value = date;
    dateInput!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  async function openRejectPanel(): Promise<void> {
    document.querySelector<HTMLElement>('[data-action="reject"]')!.click();
    await flush();
    expect(document.querySelector('[data-testid="reason-decision-panel"]')).not.toBeNull();
  }

  function btn(action: string): HTMLButtonElement {
    const el = document.querySelector<HTMLButtonElement>(`[data-action="${action}"]`);
    expect(el, `${action} button must exist`).not.toBeNull();
    return el!;
  }

  function approveWrites(): RecordedWrite[] {
    return writes.filter((w) => w.pathname.endsWith('/approve'));
  }

  // Behaviour table row 1
  it('Double click Approve -> one approve POST', async () => {
    const first = createDeferred<Response>();
    writePlan = [{ kind: 'deferred', deferred: first }];
    await mountAndOpenCase();
    setExpiresDate('2030-01-01');

    btn('approve').click();
    btn('approve').click();
    await flush();

    expect(approveWrites()).toHaveLength(1);
    expect(btn('approve').disabled).toBe(true);

    first.resolve(
      jsonResponse({ id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' }),
    );
    await flush();

    expect(approveWrites()).toHaveLength(1);
    expect(document.querySelector('[data-error]')).toBeNull();
    expect(document.querySelector('[data-testid="case-status"]')?.textContent).toBe('VERIFIED');
  });

  // Behaviour table row 2
  it('Click Reject while a reject is in flight -> no error', async () => {
    const first = createDeferred<Response>();
    writePlan = [{ kind: 'deferred', deferred: first }];
    await mountAndOpenCase();
    await openRejectPanel();

    btn('confirm-reject').click();
    await flush();
    expect(btn('confirm-reject').disabled).toBe(true);
    btn('confirm-reject').click();
    await flush();

    first.resolve(
      jsonResponse({ id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' }),
    );
    await flush();

    expect(writes.filter((w) => w.pathname.endsWith('/reject'))).toHaveLength(1);
    expect(document.querySelector('[data-error]')).toBeNull();
    expect(document.querySelector('[data-testid="case-status"]')?.textContent).toBe('REJECTED');
  });

  // Behaviour table row 3
  it('Network error on a decision, then retry -> same key (a click during the retry does not drop it)', async () => {
    const retrying = createDeferred<Response>();
    writePlan = [
      { kind: 'network' },
      { kind: 'deferred', deferred: retrying },
      {
        kind: 'http',
        status: 200,
        body: { id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
      },
    ];
    await mountAndOpenCase();
    await openRejectPanel();

    btn('confirm-reject').click();
    await flush();
    btn('retry').click();
    await flush();
    // The reason panel stays on screen while the retry is in flight.
    btn('confirm-reject').click();
    await flush();

    retrying.reject(new TypeError('Failed to fetch'));
    await flush();
    btn('retry').click();
    await flush();

    const rejects = writes.filter((w) => w.pathname.endsWith('/reject'));
    expect(rejects).toHaveLength(3);
    expect(rejects[0]!.key).toBeTruthy();
    for (const w of rejects) {
      expect(w.key).toBe(rejects[0]!.key);
      expect(w.rawBody).toBe(rejects[0]!.rawBody);
    }
    expect(document.querySelector('[data-error]')).toBeNull();
  });

  it('Double click Start review -> one start-review POST, no error', async () => {
    caseStatus = 'SUBMITTED';
    const first = createDeferred<Response>();
    writePlan = [{ kind: 'deferred', deferred: first }];
    await mountAndOpenCase();

    btn('start-review').click();
    btn('start-review').click();
    await flush();
    first.resolve(
      jsonResponse({ id: CASE_ID_1, status: 'UNDER_REVIEW', version: 3, submittedAt: '2026-10-14T06:00:00Z' }),
    );
    await flush();

    expect(writes.filter((w) => w.pathname.endsWith('/start-review'))).toHaveLength(1);
    expect(document.querySelector('[data-error]')).toBeNull();
  });

  it('Double click the at-branch Submit -> one POST, no error', async () => {
    branchReviewAllowed = true;
    const first = createDeferred<Response>();
    writePlan = [{ kind: 'deferred', deferred: first }];
    await mount();

    const custInput = document.querySelector<HTMLInputElement>('input[name="customerId"]');
    expect(custInput, 'branch customerId input must exist').not.toBeNull();
    custInput!.value = 'CUST-1';
    custInput!.dispatchEvent(new Event('input', { bubbles: true }));
    const fileInput = document.querySelector<HTMLInputElement>('input[name="file"]')!;
    const file = new File(['abc'], 'passport.png', { type: 'image/png' });
    Object.defineProperty(fileInput, 'files', { value: [file], configurable: true });
    fileInput.dispatchEvent(new Event('change', { bubbles: true }));

    btn('submit-branch-case').click();
    btn('submit-branch-case').click();
    await flush();
    expect(btn('submit-branch-case').disabled).toBe(true);

    first.resolve(jsonResponse({ id: CASE_ID_1, status: 'SUBMITTED', version: 1 }));
    await flush();

    expect(writes.filter((w) => w.pathname.endsWith('/branch-cases'))).toHaveLength(1);
    expect(document.querySelector('[data-error]')).toBeNull();
    expect(document.querySelector('[data-testid="branch-case-success"]')).not.toBeNull();
  });

  // Edge case: deciding cleared after a refusal
  it('deciding is cleared after a refusal: Approve works again with a new key', async () => {
    const first = createDeferred<Response>();
    writePlan = [{ kind: 'deferred', deferred: first }];
    await mountAndOpenCase();
    setExpiresDate('2030-01-01');

    btn('approve').click();
    btn('approve').click();
    await flush();
    first.resolve(jsonResponse({ error: { code: 'outOfScope', message: 'out of scope' } }, 403));
    await flush();

    expect(document.querySelector('[data-error]')).not.toBeNull();
    expect(btn('approve').disabled).toBe(false);

    btn('approve').click();
    await flush();

    const sent = approveWrites();
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBeTruthy();
    expect(sent[1]!.key).not.toBe(sent[0]!.key);
    expect(document.querySelector('[data-error]')).toBeNull();
  });

  // Edge case: a decision in flight then a case switch
  it('a decision in flight then a case switch: the switch happens and the old case answer is dropped', async () => {
    const first = createDeferred<Response>();
    writePlan = [{ kind: 'deferred', deferred: first }];
    await mountAndOpenCase(CASE_ID_1);
    setExpiresDate('2030-01-01');

    btn('approve').click();
    btn('approve').click();
    await flush();

    await selectCase(CASE_ID_2);
    const detail = document.querySelector('[data-testid="case-detail-view"]');
    expect(detail?.textContent).toContain(CASE_ID_2);

    first.resolve(
      jsonResponse({ id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' }),
    );
    await flush();

    expect(approveWrites()).toHaveLength(1);
    const after = document.querySelector('[data-testid="case-detail-view"]');
    expect(after?.textContent).toContain(CASE_ID_2);
    expect(after?.textContent).not.toContain(CASE_ID_1);
    expect(document.querySelector('[data-testid="case-status"]')?.textContent).toBe('UNDER_REVIEW');
    expect(document.querySelector('[data-error]')).toBeNull();

    // Once A's write settled, B's decisions are usable and carry B's id.
    setExpiresDate('2030-01-01');
    btn('approve').click();
    await flush();
    const sent = approveWrites();
    expect(sent).toHaveLength(2);
    expect(sent[1]!.pathname).toBe(`/api/operations/verification/cases/${CASE_ID_2}/approve`);
  });
});
