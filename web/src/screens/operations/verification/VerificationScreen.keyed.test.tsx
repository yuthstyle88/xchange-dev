import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_ID_1 = '11111111-1111-4111-8111-111111111111';
const CASE_ID_2 = '22222222-2222-4222-8222-222222222222';
const DOC_ID_1 = 'dddddddd-1111-4111-8111-111111111111';

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
  | { kind: 'http'; status: number; body: unknown };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('KW-6b (#2104): VerificationScreen JSON writes on keyedWrite', () => {
  let writes: RecordedWrite[] = [];
  let writePlan: WriteStep[] = [];
  let caseStatus: 'SUBMITTED' | 'UNDER_REVIEW' | 'VERIFIED' = 'SUBMITTED';
  let caseVersion = 1;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    writes = [];
    writePlan = [];
    caseStatus = 'SUBMITTED';
    caseVersion = 1;
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
              status: caseStatus === 'VERIFIED' ? 'UNDER_REVIEW' : caseStatus,
              submittedAt: '2026-10-14T06:00:00Z',
              customerDisplayName: 'Somchai Jaidee',
              identityLast4: '1234',
              branchId: 'MAIN',
            },
            {
              id: CASE_ID_2,
              status: caseStatus === 'VERIFIED' ? 'UNDER_REVIEW' : caseStatus,
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
              version: caseVersion,
              submittedAt: '2026-10-14T06:00:00Z',
              customerDisplayName: requestedId === CASE_ID_2 ? 'Alice Smith' : 'Somchai Jaidee',
              identityLast4: requestedId === CASE_ID_2 ? '5678' : '1234',
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
            status: caseStatus,
            version: caseVersion,
            submittedAt: '2026-10-14T06:00:00Z',
          });
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
    await selectCase(caseId);
  }

  async function selectCase(caseId: string): Promise<void> {
    const row = document.querySelector<HTMLElement>(`[data-case-row][data-case-id="${caseId}"]`);
    expect(row, `case row ${caseId} must exist`).not.toBeNull();
    row!.click();
    await flush();
  }

  function setExpiresDate(date: string): void {
    const dateInput = document.querySelector<HTMLInputElement>('input[name="expiresAt"]');
    expect(dateInput, 'expiresAt input must exist').not.toBeNull();
    dateInput!.value = date;
    dateInput!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  async function openRejectWithReason(reason: string): Promise<void> {
    const rejectBtn = document.querySelector<HTMLElement>('[data-action="reject"]');
    if (!document.querySelector('[data-testid="reason-decision-panel"]')) {
      expect(rejectBtn, 'reject button must exist').not.toBeNull();
      rejectBtn!.click();
      await flush();
    }
    const select = document.querySelector<HTMLSelectElement>('select[name="reasonCode"]');
    expect(select, 'reasonCode select must exist').not.toBeNull();
    select!.value = reason;
    select!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  describe('start-review', () => {
    it('Row 1: send -> network TypeError -> retry sends 2 requests with the same key and byte-equal body', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);

      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="retry"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
    });

    it('Row 2: send -> 500 -> retry sends the same key', async () => {
      writePlan = [
        { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'boom' } } },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);

      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="retry"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
    });

    it('Row 3: send -> 4xx refusal (not staleVersion, e.g. outOfScope) -> send again uses a new key', async () => {
      writePlan = [
        { kind: 'http', status: 403, body: { error: { code: 'outOfScope', message: 'out of scope' } } },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);

      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });

    it('Row 4: send -> network error -> select another case -> send uses a new key', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_2, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T05:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);

      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();

      await selectCase(CASE_ID_2);
      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
    });

    it('Row 5: send -> 2xx -> send the same body again uses a new key', async () => {
      writePlan = [
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'SUBMITTED', version: 1, submittedAt: '2026-10-14T06:00:00Z' },
        },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);

      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="start-review"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.rawBody).toBe(writes[1]!.rawBody);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });
  });

  describe('approve', () => {
    beforeEach(() => {
      caseStatus = 'UNDER_REVIEW';
      caseVersion = 2;
    });

    it('Row 1: send -> network TypeError -> retry sends 2 requests with the same key and byte-equal body', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      setExpiresDate('2030-01-01');

      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="retry"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
    });

    it('Row 2: send -> 500 -> retry sends the same key', async () => {
      writePlan = [
        { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'boom' } } },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      setExpiresDate('2030-01-01');

      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="retry"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
    });

    it('Row 3: send -> 4xx refusal (not staleVersion, e.g. outOfScope) -> send again uses a new key', async () => {
      writePlan = [
        { kind: 'http', status: 403, body: { error: { code: 'outOfScope', message: 'out of scope' } } },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      setExpiresDate('2030-01-01');

      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });

    it('Row 4: send -> network error -> choose another expiresAt date -> send uses a new key', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      setExpiresDate('2030-01-01');

      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();

      setExpiresDate('2030-06-01');
      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
    });

    it('Row 5: send -> 2xx -> send the same body again uses a new key', async () => {
      writePlan = [
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T06:00:00Z' },
        },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'VERIFIED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      setExpiresDate('2030-01-01');

      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="approve"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.rawBody).toBe(writes[1]!.rawBody);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });
  });

  describe('reject (reason decision)', () => {
    beforeEach(() => {
      caseStatus = 'UNDER_REVIEW';
      caseVersion = 2;
    });

    it('Row 1: send -> network TypeError -> retry sends 2 requests with the same key and byte-equal body', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      await openRejectWithReason('DOCUMENT_UNREADABLE');

      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="retry"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
    });

    it('Row 2: send -> 500 -> retry sends the same key', async () => {
      writePlan = [
        { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'boom' } } },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      await openRejectWithReason('DOCUMENT_UNREADABLE');

      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="retry"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
    });

    it('Row 3: send -> 4xx refusal (not staleVersion, e.g. outOfScope) -> send again uses a new key', async () => {
      writePlan = [
        { kind: 'http', status: 403, body: { error: { code: 'outOfScope', message: 'out of scope' } } },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      await openRejectWithReason('DOCUMENT_UNREADABLE');

      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });

    it('Row 4: send -> network error -> choose another reason -> send uses a new key', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      await openRejectWithReason('DOCUMENT_UNREADABLE');

      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();

      await openRejectWithReason('DOCUMENT_EXPIRED');
      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
    });

    it('Row 5: send -> 2xx -> send the same body again uses a new key', async () => {
      writePlan = [
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'UNDER_REVIEW', version: 2, submittedAt: '2026-10-14T06:00:00Z' },
        },
        {
          kind: 'http',
          status: 200,
          body: { id: CASE_ID_1, status: 'REJECTED', version: 3, submittedAt: '2026-10-14T06:00:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);
      await openRejectWithReason('DOCUMENT_UNREADABLE');

      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();

      await openRejectWithReason('DOCUMENT_UNREADABLE');
      document.querySelector<HTMLElement>('[data-action="confirm-reject"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.rawBody).toBe(writes[1]!.rawBody);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });
  });

  describe('document link', () => {
    it('Row 1: send -> network TypeError -> retry with same document and version sends 2 requests with the same key and byte-equal body', async () => {
      writePlan = [
        { kind: 'network' },
        {
          kind: 'http',
          status: 200,
          body: { url: '/api/verification/content/token-1', expiresAt: '2026-10-14T06:05:00Z' },
        },
      ];
      await mountAndOpenCase(CASE_ID_1);

      document.querySelector<HTMLElement>('[data-action="view-document"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="view-document"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBe(writes[0]!.key);
      expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
    });

    it('Row 5: send -> 2xx -> send the same body again uses a new key', async () => {
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

      document.querySelector<HTMLElement>('[data-action="view-document"]')!.click();
      await flush();
      document.querySelector<HTMLElement>('[data-action="view-document"]')!.click();
      await flush();

      expect(writes).toHaveLength(2);
      expect(writes[0]!.rawBody).toBe(writes[1]!.rawBody);
      expect(writes[0]!.key).toBeTruthy();
      expect(writes[1]!.key).toBeTruthy();
      expect(writes[1]!.key).not.toBe(writes[0]!.key);
    });
  });
});
