import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_A_ID = '11111111-1111-1111-1111-111111111111';
const CASE_B_ID = '22222222-2222-2222-2222-222222222222';
const DOC_A_ID = 'dddddddd-1111-1111-1111-111111111111';
const DOC_B_ID = 'dddddddd-2222-2222-2222-222222222222';

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
];

const CASE_A_DATA = {
  case: {
    id: CASE_A_ID,
    status: 'SUBMITTED',
    version: 1,
    submittedAt: '2026-10-14T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
  documents: [
    {
      id: DOC_A_ID,
      documentType: 'PASSPORT',
      mime: 'image/png',
      sizeBytes: 1024,
      uploadedAt: '2026-10-14T06:00:00Z',
    },
  ],
  audit: [
    {
      actorId: '00000000-0000-4000-8000-000000000091',
      role: 'CUSTOMER',
      action: 'verification_case.submit',
      objectRef: CASE_A_ID,
      beforeState: { status: 'DRAFT' },
      afterState: { status: 'SUBMITTED' },
      createdAt: '2026-10-14T06:00:00Z',
    },
  ],
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
  documents: [
    {
      id: DOC_B_ID,
      documentType: 'NATIONAL_ID',
      mime: 'image/jpeg',
      sizeBytes: 2048,
      uploadedAt: '2026-10-14T05:00:00Z',
    },
  ],
  audit: [
    {
      actorId: '00000000-0000-4000-8000-000000000092',
      role: 'VERIFICATION_REVIEWER',
      action: 'verification_case.start_review',
      objectRef: CASE_B_ID,
      beforeState: { status: 'SUBMITTED' },
      afterState: { status: 'UNDER_REVIEW' },
      createdAt: '2026-10-14T05:05:00Z',
    },
  ],
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

interface RecordedRequest {
  method: string;
  pathname: string;
  search: string;
  body: unknown;
}

describe('VerificationScreen case switch ordering (#2307)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: RecordedRequest[] = [];
  let caseViewHandler: ((caseId: string) => Promise<Response>) | null = null;
  let docLinkHandler: ((docId: string) => Promise<Response>) | null = null;
  let docContentHandler: ((pathname: string) => Promise<Response>) | null = null;

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
    caseViewHandler = null;
    docLinkHandler = null;
    docContentHandler = null;

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
        if (caseViewHandler) {
          return caseViewHandler(id);
        }
        const payload = id === CASE_B_ID ? CASE_B_DATA : CASE_A_DATA;
        return jsonResponse({
          version: payload.case.version,
          changed: true,
          data: payload,
        });
      }

      if (parsed.pathname.includes('/documents/') && parsed.pathname.endsWith('/link')) {
        const docId = parsed.pathname.split('/documents/')[1]?.split('/link')[0] ?? '';
        if (docLinkHandler) {
          return docLinkHandler(docId);
        }
        return jsonResponse({
          url: `/api/verification/content/mock-token-${docId}`,
          expiresAt: '2030-01-01T00:00:00Z',
        });
      }

      if (parsed.pathname.startsWith('/api/verification/content/')) {
        if (docContentHandler) {
          return docContentHandler(parsed.pathname);
        }
        return new Response(new Uint8Array([1, 2, 3, 4]), {
          status: 200,
          headers: { 'Content-Type': 'image/png' },
        });
      }

      if (parsed.pathname.endsWith('/start-review')) {
        const reqBody = body as { id?: string; expectedVersion?: number } | null;
        const targetId = reqBody?.id ?? CASE_B_ID;
        const baseCase = targetId === CASE_B_ID ? CASE_B_DATA.case : CASE_A_DATA.case;
        return jsonResponse({
          ...baseCase,
          status: 'UNDER_REVIEW',
          version: (reqBody?.expectedVersion ?? baseCase.version) + 1,
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

  it("row 1: Click A, then B; A answers last -> Detail, documents and audit are B's", async () => {
    const deferredA = createDeferred<Response>();
    const deferredB = createDeferred<Response>();

    caseViewHandler = (caseId) => {
      if (caseId === CASE_A_ID) return deferredA.promise;
      return deferredB.promise;
    };

    mountScreen();
    await settle();

    const rowA = host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`);
    const rowB = host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`);
    expect(rowA).not.toBeNull();
    expect(rowB).not.toBeNull();

    rowA!.click();
    rowB!.click();
    await settle();

    deferredB.resolve(
      jsonResponse({
        version: CASE_B_DATA.case.version,
        changed: true,
        data: CASE_B_DATA,
      }),
    );
    await settle();

    deferredA.resolve(
      jsonResponse({
        version: CASE_A_DATA.case.version,
        changed: true,
        data: CASE_A_DATA,
      }),
    );
    await settle();

    const detailView = host.querySelector('[data-testid="case-detail-view"]');
    expect(detailView).not.toBeNull();
    expect(detailView!.textContent).toContain(CASE_B_ID);
    expect(detailView!.textContent).toContain('Alice Smith');
    expect(detailView!.textContent).not.toContain(CASE_A_ID);
    expect(detailView!.textContent).not.toContain('Somchai Jaidee');

    expect(host.querySelector(`[data-doc-id="${DOC_B_ID}"]`)).not.toBeNull();
    expect(host.querySelector(`[data-doc-id="${DOC_A_ID}"]`)).toBeNull();

    const auditList = host.querySelector('[data-testid="case-audit-list"]');
    expect(auditList?.textContent).toContain('verification_case.start_review');
    expect(auditList?.textContent).not.toContain('verification_case.submit');
  });

  it("row 2: Open a document of A, switch to B, A's link answers last -> No preview shown in B", async () => {
    const deferredLinkA = createDeferred<Response>();
    docLinkHandler = (docId) => {
      if (docId === DOC_A_ID) return deferredLinkA.promise;
      return Promise.resolve(
        jsonResponse({
          url: `/api/verification/content/mock-token-${docId}`,
          expiresAt: '2030-01-01T00:00:00Z',
        }),
      );
    };

    mountScreen();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    const viewDocABtn = host.querySelector<HTMLElement>(`[data-doc-id="${DOC_A_ID}"]`);
    expect(viewDocABtn).not.toBeNull();
    viewDocABtn!.click();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    deferredLinkA.resolve(
      jsonResponse({
        url: `/api/verification/content/mock-token-${DOC_A_ID}`,
        expiresAt: '2030-01-01T00:00:00Z',
      }),
    );
    await settle();

    const previewImg = host.querySelector<HTMLImageElement>('.xc-verification-doc-preview img');
    expect(previewImg).toBeNull();
  });

  it("row 3: A's read fails after the switch to B -> No error shown for A", async () => {
    const deferredA = createDeferred<Response>();

    caseViewHandler = (caseId) => {
      if (caseId === CASE_A_ID) return deferredA.promise;
      return Promise.resolve(
        jsonResponse({
          version: CASE_B_DATA.case.version,
          changed: true,
          data: CASE_B_DATA,
        }),
      );
    };

    mountScreen();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    deferredA.resolve(errorResponse('serverError', 'Case A read failed', 503));
    await settle();

    expect(host.querySelector('[data-error]')).toBeNull();
    expect(host.querySelector('[data-testid="case-detail-view"]')?.textContent).toContain(
      CASE_B_ID,
    );
  });

  it("row 4: Click B, B answers -> B's detail shown", async () => {
    const deferredA = createDeferred<Response>();

    caseViewHandler = (caseId) => {
      if (caseId === CASE_A_ID) return deferredA.promise;
      return Promise.resolve(
        jsonResponse({
          version: CASE_B_DATA.case.version,
          changed: true,
          data: CASE_B_DATA,
        }),
      );
    };

    mountScreen();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    expect(host.querySelector('[data-testid="case-detail-view"]')?.textContent).toContain(
      CASE_B_ID,
    );

    deferredA.resolve(
      jsonResponse({
        version: CASE_A_DATA.case.version,
        changed: true,
        data: CASE_A_DATA,
      }),
    );
    await settle();

    const detailView = host.querySelector('[data-testid="case-detail-view"]');
    expect(detailView?.textContent).toContain(CASE_B_ID);
    expect(detailView?.textContent).toContain('UNDER_REVIEW');
  });

  it('row 5: Document link and content OK on the current case -> Preview shown', async () => {
    const deferredContentA = createDeferred<Response>();
    docContentHandler = (pathname) => {
      if (pathname.endsWith(DOC_A_ID)) {
        return deferredContentA.promise;
      }
      return Promise.resolve(
        new Response(new Uint8Array([1, 2, 3, 4]), {
          status: 200,
          headers: { 'Content-Type': 'image/png' },
        }),
      );
    };

    mountScreen();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();
    host.querySelector<HTMLElement>(`[data-doc-id="${DOC_A_ID}"]`)!.click();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();
    host.querySelector<HTMLElement>(`[data-doc-id="${DOC_B_ID}"]`)!.click();
    await settle();

    deferredContentA.resolve(errorResponse('linkExpired', 'Link A expired', 410));
    await settle();

    const previewImg = host.querySelector<HTMLImageElement>('.xc-verification-doc-preview img');
    expect(previewImg).not.toBeNull();
    expect(previewImg!.getAttribute('src')).toContain(`/api/verification/content/mock-token-${DOC_B_ID}`);
    expect(host.querySelector('[data-error]')).toBeNull();
  });

  it("edge case: Approve after the switch sends B's id and version", async () => {
    const deferredA = createDeferred<Response>();
    caseViewHandler = (caseId) => {
      if (caseId === CASE_A_ID) return deferredA.promise;
      return Promise.resolve(
        jsonResponse({
          version: CASE_B_DATA.case.version,
          changed: true,
          data: CASE_B_DATA,
        }),
      );
    };

    mountScreen();
    await settle();

    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    deferredA.resolve(
      jsonResponse({
        version: CASE_A_DATA.case.version,
        changed: true,
        data: CASE_A_DATA,
      }),
    );
    await settle();

    const expiresInput = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(expiresInput).not.toBeNull();
    expiresInput!.value = '2030-12-31';
    expiresInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const approveBtn = host.querySelector<HTMLButtonElement>('[data-action="approve"]');
    expect(approveBtn).not.toBeNull();
    approveBtn!.click();
    await settle();

    const approveReqs = sentRequests.filter((r) => r.pathname.endsWith('/approve'));
    expect(approveReqs.length).toBe(1);
    expect(approveReqs[0].pathname).toBe(
      `/api/operations/verification/cases/${CASE_B_ID}/approve`,
    );
    expect(approveReqs[0].body).toEqual({
      id: CASE_B_ID,
      expectedVersion: 3,
      expiresAt: '2030-12-31',
    });
  });

  it("in-flight switch: A loaded -> click B with B pending -> no A detail, and no approve/start-review request carrying A's id", async () => {
    const deferredB = createDeferred<Response>();
    caseViewHandler = (caseId) => {
      if (caseId === CASE_B_ID) return deferredB.promise;
      return Promise.resolve(
        jsonResponse({
          version: CASE_A_DATA.case.version,
          changed: true,
          data: CASE_A_DATA,
        }),
      );
    };

    mountScreen();
    await settle();

    // 1. Load Case A first
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_A_ID}"]`)!.click();
    await settle();

    expect(host.querySelector('[data-testid="case-detail-view"]')?.textContent).toContain(
      CASE_A_ID,
    );
    const savedStartReviewBtn = host.querySelector<HTMLButtonElement>(
      '[data-action="start-review"]',
    );
    const savedExpiresInput = host.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    const savedApproveBtn = host.querySelector<HTMLButtonElement>('[data-action="approve"]');
    expect(savedStartReviewBtn).not.toBeNull();
    expect(savedExpiresInput).not.toBeNull();
    expect(savedApproveBtn).not.toBeNull();

    savedExpiresInput!.value = '2030-12-31';
    savedExpiresInput!.dispatchEvent(new Event('input', { bubbles: true }));

    // 2. Click Case B while B's read stays pending
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_B_ID}"]`)!.click();
    await settle();

    // Attempt to click any currently rendered or previously held start-review / approve controls
    host.querySelector<HTMLButtonElement>('[data-action="start-review"]')?.click();
    host.querySelector<HTMLButtonElement>('[data-action="approve"]')?.click();
    savedStartReviewBtn!.click();
    savedApproveBtn!.click();
    await settle();

    // Assert no A detail is shown while B is pending
    expect(host.querySelector('[data-testid="case-detail-view"]')).toBeNull();
    expect(host.querySelector(`[data-doc-id="${DOC_A_ID}"]`)).toBeNull();

    // Assert no approve or start-review request carrying A's id was sent
    const aWriteReqs = sentRequests.filter(
      (r) =>
        (r.pathname.endsWith('/approve') || r.pathname.endsWith('/start-review')) &&
        (r.pathname.includes(CASE_A_ID) || (r.body as { id?: string } | null)?.id === CASE_A_ID),
    );
    expect(aWriteReqs).toEqual([]);
  });
});
