import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_ID_1 = '11111111-1111-4111-8111-111111111111';
const DOC_ID_1 = 'dddddddd-1111-4111-8111-111111111111';
const DOC_ID_2 = 'dddddddd-2222-4222-8222-222222222222';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'DOCUMENTS',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

interface RecordedWrite {
  method: string;
  pathname: string;
  key: string | null;
  rawBody: string;
  body: unknown;
}

type WriteStep =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown }
  | { kind: 'deferred'; resolve: (res: Response) => void; promise: Promise<Response> };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function deferred(): { promise: Promise<Response>; resolve: (res: Response) => void } {
  let resolve!: (res: Response) => void;
  const promise = new Promise<Response>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('#2714: VerificationScreen document view in-flight guard and preview reset', () => {
  let writes: RecordedWrite[] = [];
  let writePlan: WriteStep[] = [];
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    writes = [];
    writePlan = [];
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
            branchReviewAllowed: false,
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
              status: 'SUBMITTED',
              submittedAt: '2026-10-14T06:00:00Z',
              customerDisplayName: 'Somchai Jaidee',
              identityLast4: '1234',
              branchId: 'MAIN',
            },
          ],
        });
      }

      if (url.pathname === '/api/v3/views/verification_case') {
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
            documents: [
              {
                id: DOC_ID_1,
                documentType: 'PASSPORT',
                mime: 'image/png',
                sizeBytes: 1024,
                uploadedAt: '2026-10-14T06:00:00Z',
              },
              {
                id: DOC_ID_2,
                documentType: 'NATIONAL_ID',
                mime: 'image/png',
                sizeBytes: 2048,
                uploadedAt: '2026-10-14T06:01:00Z',
              },
            ],
            audit: [],
          },
        });
      }

      if (url.pathname.startsWith('/api/verification/content/')) {
        return new Response('img-bytes', { status: 200, headers: { 'Content-Type': 'image/png' } });
      }

      if (method === 'POST' && url.pathname.startsWith('/api/operations/verification/')) {
        const rawBody = typeof init?.body === 'string' ? init.body : '';
        let parsedBody: unknown = null;
        try {
          parsedBody = JSON.parse(rawBody);
        } catch {
          parsedBody = rawBody;
        }
        writes.push({
          method,
          pathname: url.pathname,
          key: headers.get('idempotency-key'),
          rawBody,
          body: parsedBody,
        });

        const next = writePlan.shift();
        if (!next) {
          return jsonResponse({
            id: CASE_ID_1,
            status: 'SUBMITTED',
            version: 1,
            submittedAt: '2026-10-14T06:00:00Z',
          });
        }
        if (next.kind === 'deferred') {
          return await next.promise;
        }
        if (next.kind === 'network') {
          throw new TypeError('Failed to fetch');
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

  async function mountAndOpenCase(caseId = CASE_ID_1): Promise<void> {
    dispose?.();
    document.body.innerHTML = '';
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <VerificationScreen />, host);
    await flush();
    const row = document.querySelector<HTMLElement>(`[data-case-row][data-case-id="${caseId}"]`);
    expect(row, `case row ${caseId} must exist`).not.toBeNull();
    row!.click();
    await flush();
  }

  it('1. Double click View while the link POST is held: One POST, buttons disabled in flight, preview shows with no error', async () => {
    const d = deferred();
    writePlan = [
      { kind: 'deferred', promise: d.promise, resolve: d.resolve },
    ];
    await mountAndOpenCase(CASE_ID_1);

    const viewBtn = document.querySelector<HTMLButtonElement>(`[data-action="view-document"][data-doc-id="${DOC_ID_1}"]`);
    expect(viewBtn, 'view button must exist').not.toBeNull();
    expect(viewBtn!.disabled).toBe(false);

    // First click initiates request
    viewBtn!.click();
    await flush();

    // While in flight, button is disabled
    expect(viewBtn!.disabled).toBe(true);

    // Second click while in flight must be ignored
    viewBtn!.click();
    await flush();

    expect(writes).toHaveLength(1);

    // Resolve the link POST
    d.resolve(jsonResponse({ url: '/api/verification/content/token-1', expiresAt: '2026-10-14T06:05:00Z' }, 200));
    await flush();

    expect(writes).toHaveLength(1);
    expect(document.querySelector('.xc-verification-doc-preview')).not.toBeNull();
    expect(document.querySelector('[data-error]')).toBeNull();
    expect(viewBtn!.disabled).toBe(false);
  });

  it("2. Document 1's preview is open; View on document 2 fails: document 2 error shown and document 1's preview is gone", async () => {
    writePlan = [
      {
        kind: 'http',
        status: 200,
        body: { url: '/api/verification/content/token-1', expiresAt: '2026-10-14T06:05:00Z' },
      },
      {
        kind: 'http',
        status: 500,
        body: { error: { code: 'serverError', message: 'Internal error' } },
      },
    ];
    await mountAndOpenCase(CASE_ID_1);

    // Open doc 1 successfully
    const viewBtn1 = document.querySelector<HTMLButtonElement>(`[data-action="view-document"][data-doc-id="${DOC_ID_1}"]`);
    viewBtn1!.click();
    await flush();

    expect(document.querySelector('.xc-verification-doc-preview')).not.toBeNull();
    expect(document.querySelector('[data-error]')).toBeNull();

    // Now click View on doc 2 which fails with 500
    const viewBtn2 = document.querySelector<HTMLButtonElement>(`[data-action="view-document"][data-doc-id="${DOC_ID_2}"]`);
    viewBtn2!.click();
    await flush();

    expect(document.querySelector('[data-error]')).not.toBeNull();
    expect(document.querySelector('.xc-verification-doc-preview')).toBeNull();
  });

  it('3. View fails (500), then View again: second request sent with same key and body (kept)', async () => {
    writePlan = [
      {
        kind: 'http',
        status: 500,
        body: { error: { code: 'serverError', message: 'Internal server error' } },
      },
      {
        kind: 'http',
        status: 200,
        body: { url: '/api/verification/content/token-1', expiresAt: '2026-10-14T06:05:00Z' },
      },
    ];
    await mountAndOpenCase(CASE_ID_1);

    const viewBtn = document.querySelector<HTMLButtonElement>(`[data-action="view-document"][data-doc-id="${DOC_ID_1}"]`);
    viewBtn!.click();
    await flush();

    expect(writes).toHaveLength(1);
    expect(document.querySelector('[data-error]')).not.toBeNull();
    expect(viewBtn!.disabled).toBe(false);

    viewBtn!.click();
    await flush();

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('4. View succeeds, then View again: uses a new key (kept)', async () => {
    writePlan = [
      {
        kind: 'http',
        status: 200,
        body: { url: '/api/verification/content/token-1', expiresAt: '2026-10-14T06:05:00Z' },
      },
      {
        kind: 'http',
        status: 200,
        body: { url: '/api/verification/content/token-2', expiresAt: '2026-10-14T06:10:00Z' },
      },
    ];
    await mountAndOpenCase(CASE_ID_1);

    const viewBtn = document.querySelector<HTMLButtonElement>(`[data-action="view-document"][data-doc-id="${DOC_ID_1}"]`);
    viewBtn!.click();
    await flush();

    expect(writes).toHaveLength(1);
    expect(viewBtn!.disabled).toBe(false);

    viewBtn!.click();
    await flush();

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
