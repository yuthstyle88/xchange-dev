// Written before the implementation by the test writer gemini/warut-4 (ticket author claude/e92db6). Part A. Do not edit, delete or weaken.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';

// =================================================================================================
// Node.js dynamic import interfaces (for TS strict browser environment without @types/node)
// =================================================================================================

interface NodeFs {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: string): string;
}

interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fsMod: NodeFs;
let pathMod: NodePath;

beforeAll(async () => {
  fsMod = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  pathMod = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
});

// =================================================================================================
// Types & Interfaces (pure local declarations — NO generated type imported)
// =================================================================================================

interface QueueCaseItem {
  id: string;
  status: 'SUBMITTED' | 'UNDER_REVIEW';
  submittedAt: string;
  customerDisplayName: string;
  identityLast4: string;
  branchId: string | null;
}

interface CaseDocumentItem {
  id: string;
  documentType: string;
  mime: string;
  sizeBytes: number;
  uploadedAt: string;
}

interface CaseAuditEvent {
  actorId: string;
  role: string;
  action: string;
  objectRef: string;
  beforeState?: Record<string, unknown> | null;
  afterState?: Record<string, unknown> | null;
  createdAt: string;
}

interface CaseDetail {
  id: string;
  status: string;
  version: number;
  submittedAt: string;
  customerDisplayName: string;
  identityLast4: string;
  branchId: string | null;
  expiresAt?: string | null;
  decisionReasonCode?: string | null;
}

interface SentRequest {
  method: string;
  url: string;
  pathname: string;
  search: string;
  headers: Record<string, string>;
  body: any;
}

// S-5 reason codes list (from ticket #1400 / VF-7 / blueprint §16)
const S5_REASON_CODES = [
  'DOCUMENT_UNREADABLE',
  'DOCUMENT_EXPIRED',
  'DETAILS_MISMATCH',
  'DOCUMENT_NOT_ACCEPTED',
  'SUSPECTED_FRAUD',
  'OTHER',
] as const;

// Test fixtures
const CASE_ID_1 = '11111111-1111-1111-1111-111111111111';
const CASE_ID_2 = '22222222-2222-2222-2222-222222222222';
const DOC_ID_1 = 'dddddddd-1111-1111-1111-111111111111';

const SECRET_IDENTITY_FULL = 'AA1231234';
const SECRET_IDENTITY_LAST4 = '1234';
const SECRET_DOC_BYTES_STR = 'CONFIDENTIAL_PASSPORT_IMAGE_RAW_BYTES_FOR_VF7_TESTING';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const USER_WITHOUT_PERM_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000002',
  role: 'TELLER',
  branch: 'MAIN',
  name: 'Teller Somsri',
  branchName: 'Head Office',
});

const QUEUE_FIXTURE: QueueCaseItem[] = [
  {
    id: CASE_ID_1,
    status: 'SUBMITTED',
    submittedAt: '2026-10-14T06:00:00Z',
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: SECRET_IDENTITY_LAST4,
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

const CASE_FIXTURE_1: CaseDetail = {
  id: CASE_ID_1,
  status: 'SUBMITTED',
  version: 1,
  submittedAt: '2026-10-14T06:00:00Z',
  customerDisplayName: 'Somchai Jaidee',
  identityLast4: SECRET_IDENTITY_LAST4,
  branchId: 'MAIN',
};

const DOC_FIXTURE_1: CaseDocumentItem = {
  id: DOC_ID_1,
  documentType: 'PASSPORT',
  mime: 'image/png',
  sizeBytes: 1024,
  uploadedAt: '2026-10-14T06:00:00Z',
};

const AUDIT_FIXTURE_1: CaseAuditEvent = {
  actorId: '00000000-0000-4000-8000-000000000099',
  role: 'CUSTOMER',
  action: 'verification_case.submit',
  objectRef: CASE_ID_1,
  beforeState: { status: 'DRAFT' },
  afterState: { status: 'SUBMITTED' },
  createdAt: '2026-10-14T06:00:00Z',
};

// =================================================================================================
// Test harness & Mock fetch
// =================================================================================================

describe('VF-9 Part A: Operations Verification Screen (Up-front tests)', () => {
  let sentRequests: SentRequest[] = [];
  let currentSession: ReturnType<typeof meOf> = REVIEWER_SESSION;
  let queueVersion = 10;
  let queueItems: QueueCaseItem[] = [...QUEUE_FIXTURE];
  let caseDetail: CaseDetail = { ...CASE_FIXTURE_1 };
  let caseDocuments: CaseDocumentItem[] = [{ ...DOC_FIXTURE_1 }];
  let caseAudit: CaseAuditEvent[] = [{ ...AUDIT_FIXTURE_1 }];

  let queueStatus = 200;
  let queueErrorCode: string | null = null;
  let queueErrorMessage: string | null = null;

  let caseStatus = 200;
  let caseErrorCode: string | null = null;

  let commandStatus = 200;
  let commandErrorCode: string | null = null;
  let commandErrorMessage: string | null = null;

  let contentStatus = 200;
  let contentErrorCode: string | null = null;
  let contentErrorMessage: string | null = null;

  function jsonResponse(data: unknown, status = 200): Response {
    return new Response(JSON.stringify(data), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  function errorResponse(code: string, message: string, status = 400): Response {
    return new Response(JSON.stringify({ error: { code, message } }), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  beforeEach(() => {
    setLang('th');
    sentRequests = [];
    currentSession = REVIEWER_SESSION;
    queueVersion = 10;
    queueItems = [...QUEUE_FIXTURE];
    caseDetail = { ...CASE_FIXTURE_1 };
    caseDocuments = [{ ...DOC_FIXTURE_1 }];
    caseAudit = [{ ...AUDIT_FIXTURE_1 }];

    queueStatus = 200;
    queueErrorCode = null;
    queueErrorMessage = null;

    caseStatus = 200;
    caseErrorCode = null;

    commandStatus = 200;
    commandErrorCode = null;
    commandErrorMessage = null;

    contentStatus = 200;
    contentErrorCode = null;
    contentErrorMessage = null;

    document.body.innerHTML = '';
    vi.resetModules();

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const parsedUrl = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      let body: any = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({
        method,
        url: rawUrl,
        pathname: parsedUrl.pathname,
        search: parsedUrl.search,
        headers,
        body,
      });

      // 1. Session endpoints (staff_me or me)
      if (parsedUrl.pathname === '/api/v3/views/staff_me' || parsedUrl.pathname === '/api/v3/views/me') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: currentSession,
        });
      }

      // 2. Queue view (v3 envelope: version, changed, data)
      if (parsedUrl.pathname === '/api/v3/views/verification_queue') {
        if (queueStatus !== 200) {
          return errorResponse(queueErrorCode ?? 'serverError', queueErrorMessage ?? 'Error', queueStatus);
        }
        const sinceParam = parsedUrl.searchParams.get('since');
        if (sinceParam !== null && parseInt(sinceParam, 10) === queueVersion) {
          return jsonResponse({ changed: false, version: queueVersion });
        }
        return jsonResponse({
          version: queueVersion,
          changed: true,
          data: queueItems,
        });
      }

      // 3. Case detail view (v3 envelope: version, changed, data: { case, documents, audit })
      if (parsedUrl.pathname === '/api/v3/views/verification_case') {
        if (caseStatus !== 200) {
          return errorResponse(caseErrorCode ?? 'serverError', 'Case load error', caseStatus);
        }
        return jsonResponse({
          version: caseDetail.version,
          changed: true,
          data: {
            case: caseDetail,
            documents: caseDocuments,
            audit: caseAudit,
          },
        });
      }

      // 4. Reviewer commands (start-review, approve, reject, request-update, revoke, link)
      if (parsedUrl.pathname.startsWith('/api/operations/verification/')) {
        if (commandStatus !== 200) {
          return errorResponse(
            commandErrorCode ?? 'validationFailed',
            commandErrorMessage ?? 'Command failed',
            commandStatus,
          );
        }

        if (parsedUrl.pathname.endsWith('/start-review')) {
          caseDetail = { ...caseDetail, status: 'UNDER_REVIEW', version: (body?.expectedVersion ?? 1) + 1 };
          return jsonResponse({ ...caseDetail });
        }

        if (parsedUrl.pathname.endsWith('/approve')) {
          caseDetail = {
            ...caseDetail,
            status: 'VERIFIED',
            expiresAt: body?.expiresAt ?? '2030-01-01',
            version: (body?.expectedVersion ?? 1) + 1,
          };
          return jsonResponse({ ...caseDetail });
        }

        if (parsedUrl.pathname.endsWith('/reject')) {
          caseDetail = {
            ...caseDetail,
            status: 'REJECTED',
            decisionReasonCode: body?.reasonCode ?? 'OTHER',
            version: (body?.expectedVersion ?? 1) + 1,
          };
          return jsonResponse({ ...caseDetail });
        }

        if (parsedUrl.pathname.endsWith('/request-update')) {
          caseDetail = {
            ...caseDetail,
            status: 'NEEDS_UPDATE',
            decisionReasonCode: body?.reasonCode ?? 'OTHER',
            version: (body?.expectedVersion ?? 1) + 1,
          };
          return jsonResponse({ ...caseDetail });
        }

        if (parsedUrl.pathname.endsWith('/revoke')) {
          caseDetail = {
            ...caseDetail,
            status: 'REVOKED',
            decisionReasonCode: body?.reasonCode ?? 'OTHER',
            version: (body?.expectedVersion ?? 1) + 1,
          };
          return jsonResponse({ ...caseDetail });
        }

        if (parsedUrl.pathname.includes('/documents/') && parsedUrl.pathname.endsWith('/link')) {
          return jsonResponse({
            url: `/api/verification/content/mock-token-${DOC_ID_1}`,
            expiresAt: new Date(Date.now() + 60_000).toISOString(),
          });
        }
      }

      // 5. Document content route
      if (parsedUrl.pathname.startsWith('/api/verification/content/')) {
        if (contentStatus !== 200) {
          return errorResponse(contentErrorCode ?? 'linkExpired', contentErrorMessage ?? 'link has expired', contentStatus);
        }
        return new Response(new Uint8Array([1, 2, 3, 4]), {
          status: 200,
          headers: {
            'Content-Type': 'image/png',
            'Cache-Control': 'private, no-store',
            'X-Content-Type-Options': 'nosniff',
            'Content-Disposition': 'attachment; filename="document.png"',
          },
        });
      }

      return errorResponse('notFound', `Route not found: ${parsedUrl.pathname}`, 404);
    });
  });

  afterEach(() => {
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  async function settleDom(): Promise<void> {
    for (let i = 0; i < 20; i++) {
      if (vi.isFakeTimers()) {
        await vi.advanceTimersByTimeAsync(0);
      } else {
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    }
  }

  async function mountPage(): Promise<void> {
    const host = document.createElement('div');
    host.id = 'app';
    document.body.appendChild(host);

    await import('./main');
    await settleDom();
  }

  async function openCase(id: string = CASE_ID_1): Promise<void> {
    const selectTrigger = document.querySelector<HTMLElement>(`[data-case-id="${id}"]`);
    expect(selectTrigger, `a case selection trigger [data-case-id="${id}"] must exist`).not.toBeNull();
    selectTrigger?.click();
    await settleDom();
  }

  // ===============================================================================================
  // Group 1: Page Entry, AppShell & Permission Gate (§4.2, §7, §9.1)
  // ===============================================================================================
  describe('Group 1: Page Entry, AppShell & Permission Gate', () => {
    it('1. main.tsx mounts AppShell and renders verification queue for a user with VerificationReview permission', async () => {
      await mountPage();

      const shell = document.querySelector('.xc-shell');
      expect(shell, 'the entry must render the shared AppShell').not.toBeNull();

      // Expect queue items rendered in screen
      const bodyText = document.body.textContent ?? '';
      expect(bodyText).toContain('Somchai Jaidee');
      expect(bodyText).toContain('Alice Smith');
    });

    it('2. gate: a user without VerificationReview permission is refused access and does not see reviewer queue', async () => {
      currentSession = USER_WITHOUT_PERM_SESSION;
      await mountPage();

      const bodyText = document.body.textContent ?? '';
      expect(bodyText).not.toContain('Somchai Jaidee');
      expect(bodyText).not.toContain('Alice Smith');

      // Must show refusal sentence / forbidden access notice
      const refusedElement = document.querySelector('[data-refused], [data-error], .refusal-notice');
      expect(refusedElement !== null || bodyText.includes('ไม่มีสิทธิ์') || bodyText.includes('Permission denied')).toBe(true);
    });

    it('3. gate: source files enforce gate via VerificationReview permission and never hardcode role names', () => {
      const currentDir = pathMod.dirname(new URL(import.meta.url).pathname);
      const files: string[] = fsMod.readdirSync(currentDir).filter((f: string) => f.endsWith('.ts') || f.endsWith('.tsx'));
      const sourceFiles: string[] = files.filter((f: string) => !f.includes('.test.'));

      let foundVerificationReview = false;
      for (const file of sourceFiles) {
        const content = fsMod.readFileSync(pathMod.join(currentDir, file), 'utf-8');
        const roleMatch = content.match(/VERIFICATION_REVIEWER|'TELLER'|'EXECUTIVE'|'ADMIN'/);
        expect(
          roleMatch,
          `Source file ${file} must not contain hardcoded role names outside test mocks (pattern: can(me, 'VerificationReview'))`,
        ).toBeNull();
        if (content.includes('VerificationReview')) {
          foundVerificationReview = true;
        }
      }
      expect(foundVerificationReview, 'at least one source file must reference VerificationReview permission').toBe(true);
    });

    it('4. refusal: a 400 RoleNotAllowed / roleNotAllowedHere answer shows a worded refusal sentence (not blank)', async () => {
      queueStatus = 400;
      queueErrorCode = 'roleNotAllowedHere';
      queueErrorMessage = 'role not allowed';

      await mountPage();

      const shell = document.querySelector('.xc-shell');
      expect(shell).not.toBeNull();

      // Assert queue rows are not shown
      const queueRows = document.querySelectorAll('[data-case-row], [data-case-id]');
      expect(queueRows.length, 'queue items must not be shown on 400 RoleNotAllowed refusal').toBe(0);

      const bodyText = document.body.textContent ?? '';
      expect(bodyText).not.toContain('Somchai Jaidee');
      expect(bodyText).not.toContain('Alice Smith');
      expect(bodyText).not.toContain('role not allowed'); // Must show worded sentence, not raw server error

      // Pin the screen's own refusal sentence ([data-refused] element holding it)
      const refusedElement = document.querySelector<HTMLElement>('[data-refused]');
      expect(refusedElement, 'a [data-refused] element holding the worded refusal must exist').not.toBeNull();
      const refusedText = refusedElement?.textContent ?? '';
      expect(
        refusedText.includes('ไม่มีสิทธิ์') ||
          refusedText.includes('not permitted') ||
          refusedText.includes('Permission denied'),
        'refusal element must contain worded refusal sentence',
      ).toBe(true);
    });

    it('5. language: screen supports TH and EN only, and never renders Chinese (zh-CN) text (L-A4)', async () => {
      // 5a. Thai check
      setLang('th');
      await mountPage();
      let bodyText = document.body.textContent ?? '';
      expect(bodyText, 'queue must be rendered in TH before checking language constraints').toContain('Somchai Jaidee');
      expect(/[\u4e00-\u9fa5]/.test(bodyText), 'screen must not contain Chinese characters in TH').toBe(false);

      // 5b. English check
      setLang('en');
      document.body.innerHTML = '';
      vi.resetModules();
      await mountPage();
      bodyText = document.body.textContent ?? '';
      expect(bodyText, 'queue must be rendered in EN before checking language constraints').toContain('Somchai Jaidee');
      expect(/[\u4e00-\u9fa5]/.test(bodyText), 'screen must not contain Chinese characters in EN').toBe(false);

      // 5c. Assert screen's i18n dictionary (if any) has no Chinese (zh) translations
      const currentDir = pathMod.dirname(new URL(import.meta.url).pathname);
      const files: string[] = fsMod.readdirSync(currentDir);
      const i18nFiles = files.filter((f: string) => f.includes('i18n') || f === 'zh.ts' || f === 'zh-CN.ts');
      expect(i18nFiles.some((f) => f.startsWith('zh')), 'no Chinese translation file in verification screen').toBe(false);
      for (const f of i18nFiles) {
        const content = fsMod.readFileSync(pathMod.join(currentDir, f), 'utf-8');
        expect(content, `i18n file ${f} must not declare a zh or zh-CN table`).not.toMatch(/\b(zh|zh_CN|zh-CN)\s*:/);
      }
    });
  });

  // ===============================================================================================
  // Group 2: Reviewer Queue & Privacy Constraints (§4.5, §7, §9.1)
  // ===============================================================================================
  describe('Group 2: Reviewer Queue & Privacy Constraints', () => {
    it('6. queue view: renders SUBMITTED and UNDER_REVIEW cases in scope with display name and identity last 4', async () => {
      await mountPage();

      const rows = document.querySelectorAll('[data-case-row], tr[data-case-id], [data-case-id]');
      expect(rows.length).toBeGreaterThan(0);

      const content = document.body.textContent ?? '';
      expect(content).toContain('Somchai Jaidee');
      expect(content).toContain(SECRET_IDENTITY_LAST4);
      expect(content).toContain('Alice Smith');
      expect(content).toContain('5678');
    });

    it('7. privacy constraint: DOM never leaks full identity numbers, raw document bytes, or base64 data', async () => {
      await mountPage();

      const domHtml = document.body.innerHTML;
      expect(domHtml).not.toContain(SECRET_IDENTITY_FULL);
      expect(domHtml).not.toContain(SECRET_DOC_BYTES_STR);
      expect(domHtml).toContain(SECRET_IDENTITY_LAST4);
    });

    it('8. queue polling: since parameter returning { changed: false } preserves queue without redrawing', async () => {
      // Fake timers installed before mountPage() so interval/timeout registrations are captured
      vi.useFakeTimers({ toFake: ['setInterval', 'setTimeout'] });
      try {
        await mountPage();

        const rowBefore = document.querySelector<HTMLElement>(`[data-case-id="${CASE_ID_1}"]`);
        expect(rowBefore, 'queue row element must exist before poll').not.toBeNull();

        // Advance fake timers by poll interval to trigger periodic queue fetch
        await vi.advanceTimersByTimeAsync(30_000);
        const refreshBtn = document.querySelector<HTMLElement>('[data-action="refresh"], button.refresh');
        if (refreshBtn && sentRequests.filter((r) => r.pathname === '/api/v3/views/verification_queue').length < 2) {
          refreshBtn.click();
        }

        await settleDom();

        const queueRequests = sentRequests.filter((r) => r.pathname === '/api/v3/views/verification_queue');
        expect(queueRequests.length, 'poll must issue a second verification_queue request').toBeGreaterThanOrEqual(2);
        expect(queueRequests[1].search, 'second verification_queue request must carry since=10').toContain('since=10');

        const rowAfter = document.querySelector<HTMLElement>(`[data-case-id="${CASE_ID_1}"]`);
        expect(rowAfter, 'queue row must still exist after changed: false').not.toBeNull();
        expect(rowAfter, 'queue row element reference must be the exact same DOM node (no redraw)').toBe(rowBefore);
      } finally {
        vi.useRealTimers();
      }
    });
  });

  // ===============================================================================================
  // Group 3: Case View & Document Links (§4.5, §9.1)
  // ===============================================================================================
  describe('Group 3: Case View & Document Links', () => {
    it('9. case view: selecting a case reads verification_case view and displays case metadata, document metadata, and audit trail', async () => {
      await mountPage();
      await openCase(CASE_ID_1);

      const caseReq = sentRequests.find(
        (r) => r.pathname === '/api/v3/views/verification_case' && r.search.includes(`id=${CASE_ID_1}`),
      );
      expect(caseReq, 'screen must fetch verification_case for the selected case').toBeDefined();

      const text = document.body.textContent ?? '';
      expect(text).toContain('PASSPORT');
      expect(text).toContain('SUBMITTED');

      // Assert at least one audit trail field (action or state transition)
      expect(
        text.includes('verification_case.submit') || text.includes('DRAFT'),
        'screen must display audit trail event action or state transition',
      ).toBe(true);
    });

    it('10. document link: loads document image only via link URL from POST /api/operations/verification/documents/{id}/link, never a /files/ URL', async () => {
      await mountPage();
      await openCase(CASE_ID_1);

      // Open document preview
      const previewBtn =
        document.querySelector<HTMLElement>(`[data-doc-id="${DOC_ID_1}"] button, [data-doc-id="${DOC_ID_1}"]`) ??
        document.querySelector<HTMLElement>('[data-action="view-document"], button.view-doc');
      expect(previewBtn, 'document link trigger must exist').not.toBeNull();
      previewBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const linkReq = sentRequests.find(
        (r) => r.method === 'POST' && r.pathname === `/api/operations/verification/documents/${DOC_ID_1}/link`,
      );
      expect(linkReq, 'screen must call document link endpoint').toBeDefined();
      expect(linkReq?.headers['idempotency-key'], 'link command must carry Idempotency-Key').toBeTruthy();
      expect(linkReq?.body?.expectedVersion, 'link command body must send expectedVersion').toBe(1);

      // Verify rendered img uses content URL and NEVER a /files/ URL
      const images = [...document.querySelectorAll<HTMLImageElement>('img')];
      for (const img of images) {
        expect(img.src, 'image src must never point to /files/').not.toContain('/files/');
      }
      const contentImg = images.find((img) => img.src.includes('/api/verification/content/'));
      expect(contentImg, 'screen must display image using link content URL').toBeDefined();
    });

    it('11. linkExpired: when document content fetch returns linkExpired (HTTP 410), the screen asks for a new link', async () => {
      await mountPage();
      await openCase(CASE_ID_1);

      // Link command returns 200, but content fetch returns 410 linkExpired
      contentStatus = 410;
      contentErrorCode = 'linkExpired';
      contentErrorMessage = 'link has expired';

      const previewBtn =
        document.querySelector<HTMLElement>(`[data-doc-id="${DOC_ID_1}"] button, [data-doc-id="${DOC_ID_1}"]`) ??
        document.querySelector<HTMLElement>('[data-action="view-document"], button.view-doc');
      expect(previewBtn, 'document link trigger must exist').not.toBeNull();
      previewBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const initialLinkReqs = sentRequests.filter(
        (r) => r.method === 'POST' && r.pathname === `/api/operations/verification/documents/${DOC_ID_1}/link`,
      );
      expect(initialLinkReqs.length, 'opening document must call link command').toBe(1);

      // Assert worded "ask for a new link" text and renew control
      const text = document.body.textContent ?? '';
      const renewBtn = document.querySelector<HTMLElement>(
        '[data-action="refresh-link"], [data-action="renew-link"], button.renew-link',
      );
      expect(renewBtn, 'renew control [data-action="refresh-link"] must exist').not.toBeNull();
      expect(
        text.includes('ขอลิงก์ใหม่') ||
          text.includes('ขอลิงก์') ||
          text.toLowerCase().includes('ask for a new link') ||
          text.toLowerCase().includes('renew link') ||
          text.toLowerCase().includes('refresh link') ||
          renewBtn?.textContent?.includes('ลิงก์') ||
          renewBtn?.textContent?.toLowerCase().includes('link'),
        'must display worded message asking for a new link',
      ).toBe(true);

      // Assert that clicking renew control sends a new POST .../link command
      contentStatus = 200;
      renewBtn?.click();
      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const linkRequestsAfter = sentRequests.filter(
        (r) => r.method === 'POST' && r.pathname === `/api/operations/verification/documents/${DOC_ID_1}/link`,
      );
      expect(linkRequestsAfter.length, 'clicking renew control must send a new POST .../link command').toBeGreaterThan(
        initialLinkReqs.length,
      );
    });
  });

  // ===============================================================================================
  // Group 4: Decision Commands & S-5 Reasons (§4.5, §7 [required], §9.1)
  // ===============================================================================================
  describe('Group 4: Decision Commands & S-5 Reasons', () => {
    it('12. start-review: moves SUBMITTED case to UNDER_REVIEW, sending expectedVersion and Idempotency-Key', async () => {
      await mountPage();
      await openCase(CASE_ID_1);

      const startBtn = document.querySelector<HTMLElement>('[data-action="start-review"], button.start-review');
      expect(startBtn, 'start-review button must exist').not.toBeNull();
      startBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const startReq = sentRequests.find(
        (r) => r.method === 'POST' && r.pathname === `/api/operations/verification/cases/${CASE_ID_1}/start-review`,
      );
      expect(startReq, 'must POST to start-review endpoint').toBeDefined();
      expect(startReq?.headers['idempotency-key'], 'start-review must carry Idempotency-Key').toBeTruthy();
      expect(startReq?.body?.expectedVersion).toBe(1);
    });

    it('13. approve: requires a future expiresAt date and sends expectedVersion, expiresAt, and Idempotency-Key', async () => {
      caseDetail = { ...caseDetail, status: 'UNDER_REVIEW', version: 2 };
      await mountPage();
      await openCase(CASE_ID_1);

      const dateInput = document.querySelector<HTMLInputElement>('input[type="date"], input[name="expiresAt"]');
      expect(dateInput, 'expiresAt date input must exist for approve action').not.toBeNull();

      const approveBtn = document.querySelector<HTMLElement>('[data-action="approve"], button.approve');
      expect(approveBtn, 'approve button must exist').not.toBeNull();

      // 13a. Attempt without date is blocked by client validation
      if (dateInput) dateInput.value = '';
      approveBtn?.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
      expect(sentRequests.some((r) => r.pathname.endsWith('/approve')), 'approve without date must not be sent').toBe(false);

      // 13b. Date in past or today is blocked
      if (dateInput) {
        dateInput.value = '2020-01-01';
        dateInput.dispatchEvent(new Event('input'));
      }
      approveBtn?.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
      expect(sentRequests.some((r) => r.pathname.endsWith('/approve')), 'approve with past date must not be sent').toBe(false);

      // 13c. Future date succeeds
      if (dateInput) {
        dateInput.value = '2030-01-01';
        dateInput.dispatchEvent(new Event('input'));
      }
      approveBtn?.click();
      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const appReq = sentRequests.find((r) => r.method === 'POST' && r.pathname.endsWith('/approve'));
      expect(appReq, 'approve with future date must be sent').toBeDefined();
      expect(appReq?.headers['idempotency-key'], 'approve must carry Idempotency-Key').toBeTruthy();
      expect(appReq?.body?.expectedVersion).toBe(2);
      expect(appReq?.body?.expiresAt).toBe('2030-01-01');
    });

    it('14. reject: requires an S-5 reasonCode and sends expectedVersion, reasonCode, and Idempotency-Key', async () => {
      caseDetail = { ...caseDetail, status: 'UNDER_REVIEW', version: 2 };
      await mountPage();
      await openCase(CASE_ID_1);

      const rejectTrigger = document.querySelector<HTMLElement>('[data-action="reject"], button.reject');
      expect(rejectTrigger, 'reject trigger must exist').not.toBeNull();
      rejectTrigger?.click();

      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      // Verify S-5 reasons are supported
      expect(S5_REASON_CODES.length).toBe(6);

      const reasonSelect = document.querySelector<HTMLSelectElement>('select[name="reasonCode"], select.reason-select');
      const reasonOption = document.querySelector<HTMLInputElement>('input[value="DOCUMENT_EXPIRED"]');
      expect(reasonSelect !== null || reasonOption !== null, 'S-5 reason selection must be offered').toBe(true);

      if (reasonSelect) {
        reasonSelect.value = 'DOCUMENT_EXPIRED';
        reasonSelect.dispatchEvent(new Event('change'));
      } else if (reasonOption) {
        reasonOption.click();
      }

      const confirmRejectBtn =
        document.querySelector<HTMLElement>('[data-action="confirm-reject"]') ?? rejectTrigger;
      confirmRejectBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const rejReq = sentRequests.find((r) => r.method === 'POST' && r.pathname.endsWith('/reject'));
      expect(rejReq, 'reject command must be sent').toBeDefined();
      expect(rejReq?.headers['idempotency-key'], 'reject must carry Idempotency-Key').toBeTruthy();
      expect(rejReq?.body?.expectedVersion).toBe(2);
      expect(rejReq?.body?.reasonCode).toBe('DOCUMENT_EXPIRED');
    });

    it('15. request-update: requires an S-5 reasonCode and sends expectedVersion, reasonCode, and Idempotency-Key', async () => {
      caseDetail = { ...caseDetail, status: 'UNDER_REVIEW', version: 2 };
      await mountPage();
      await openCase(CASE_ID_1);

      const reqUpdateTrigger = document.querySelector<HTMLElement>('[data-action="request-update"], button.request-update');
      expect(reqUpdateTrigger, 'request-update trigger must exist').not.toBeNull();
      reqUpdateTrigger?.click();

      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const reasonSelect = document.querySelector<HTMLSelectElement>('select[name="reasonCode"], select.reason-select');
      const reasonOption = document.querySelector<HTMLInputElement>('input[value="DOCUMENT_UNREADABLE"]');
      if (reasonSelect) {
        reasonSelect.value = 'DOCUMENT_UNREADABLE';
        reasonSelect.dispatchEvent(new Event('change'));
      } else if (reasonOption) {
        reasonOption.click();
      }

      const confirmBtn =
        document.querySelector<HTMLElement>('[data-action="confirm-request-update"]') ?? reqUpdateTrigger;
      confirmBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const updReq = sentRequests.find((r) => r.method === 'POST' && r.pathname.endsWith('/request-update'));
      expect(updReq, 'request-update command must be sent').toBeDefined();
      expect(updReq?.headers['idempotency-key'], 'request-update must carry Idempotency-Key').toBeTruthy();
      expect(updReq?.body?.expectedVersion).toBe(2);
      expect(updReq?.body?.reasonCode).toBe('DOCUMENT_UNREADABLE');
    });

    it('16. revoke: on a VERIFIED case requires an S-5 reasonCode and sends expectedVersion, reasonCode, and Idempotency-Key', async () => {
      caseDetail = { ...caseDetail, status: 'VERIFIED', version: 3 };
      await mountPage();
      await openCase(CASE_ID_1);

      const revokeTrigger = document.querySelector<HTMLElement>('[data-action="revoke"], button.revoke');
      expect(revokeTrigger, 'revoke trigger must exist for verified case').not.toBeNull();
      revokeTrigger?.click();

      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const reasonSelect = document.querySelector<HTMLSelectElement>('select[name="reasonCode"], select.reason-select');
      const reasonOption = document.querySelector<HTMLInputElement>('input[value="SUSPECTED_FRAUD"]');
      if (reasonSelect) {
        reasonSelect.value = 'SUSPECTED_FRAUD';
        reasonSelect.dispatchEvent(new Event('change'));
      } else if (reasonOption) {
        reasonOption.click();
      }

      const confirmBtn = document.querySelector<HTMLElement>('[data-action="confirm-revoke"]') ?? revokeTrigger;
      confirmBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const revReq = sentRequests.find((r) => r.method === 'POST' && r.pathname.endsWith('/revoke'));
      expect(revReq, 'revoke command must be sent').toBeDefined();
      expect(revReq?.headers['idempotency-key'], 'revoke must carry Idempotency-Key').toBeTruthy();
      expect(revReq?.body?.expectedVersion).toBe(3);
      expect(revReq?.body?.reasonCode).toBe('SUSPECTED_FRAUD');
    });
  });

  // ===============================================================================================
  // Group 5: Idempotency & Error Handling (§4.5, §7 [required], §9.1)
  // ===============================================================================================
  describe('Group 5: Idempotency & Error Handling', () => {
    it('17. idempotency: retrying a failed decision command keeps the exact same Idempotency-Key across retries', async () => {
      caseDetail = { ...caseDetail, status: 'UNDER_REVIEW', version: 2 };
      commandStatus = 500;
      commandErrorCode = 'internalError';
      commandErrorMessage = 'Server temporary failure';

      await mountPage();
      await openCase(CASE_ID_1);

      const dateInput = document.querySelector<HTMLInputElement>('input[type="date"], input[name="expiresAt"]');
      if (dateInput) {
        dateInput.value = '2030-01-01';
        dateInput.dispatchEvent(new Event('input'));
      }

      const approveBtn = document.querySelector<HTMLElement>('[data-action="approve"], button.approve');
      approveBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const firstApprove = sentRequests.find((r) => r.method === 'POST' && r.pathname.endsWith('/approve'));
      expect(firstApprove, 'first approve attempt must be made').toBeDefined();
      const firstIdemKey = firstApprove?.headers['idempotency-key'];
      expect(firstIdemKey, 'first attempt must have Idempotency-Key').toBeTruthy();

      // Click retry
      const retryBtn = document.querySelector<HTMLElement>('[data-action="retry"], button.retry') ?? approveBtn;
      retryBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const approveRequests = sentRequests.filter((r) => r.method === 'POST' && r.pathname.endsWith('/approve'));
      expect(approveRequests.length, 'retry must issue second approve POST').toBe(2);
      expect(approveRequests[1].headers['idempotency-key'], 'retry must reuse exact same Idempotency-Key').toBe(firstIdemKey);
    });

    it('18. idempotencyConflict: HTTP 409 idempotencyConflict displays a worded conflict error', async () => {
      commandStatus = 409;
      commandErrorCode = 'idempotencyConflict';
      commandErrorMessage = 'idempotency conflict with different body';

      await mountPage();
      await openCase(CASE_ID_1);

      const startBtn = document.querySelector<HTMLElement>('[data-action="start-review"], button.start-review');
      startBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const text = document.body.textContent ?? '';
      expect(text).not.toContain('idempotency conflict with different body');
      const errorBox = document.querySelector<HTMLElement>('[data-error], .error-message, .toast-error');
      expect(errorBox, 'must render error box for idempotencyConflict').not.toBeNull();
      const errText = errorBox?.textContent ?? '';
      expect(
        errText.includes('ซ้ำ') ||
          errText.toLowerCase().includes('conflict') ||
          errText.toLowerCase().includes('already sent'),
        'must display worded conflict error',
      ).toBe(true);
    });

    it('19. staleVersion: HTTP 409/400 staleVersion reloads the case and does NOT silently re-send the decision', async () => {
      caseDetail = { ...caseDetail, status: 'UNDER_REVIEW', version: 2 };
      commandStatus = 409;
      commandErrorCode = 'staleVersion';
      commandErrorMessage = 'stale version';

      await mountPage();
      await openCase(CASE_ID_1);

      // Count case reads after opening the case
      const initialCaseReadsCount = sentRequests.filter((r) => r.pathname === '/api/v3/views/verification_case').length;
      expect(initialCaseReadsCount, 'opening case must have read verification_case').toBeGreaterThanOrEqual(1);

      const dateInput = document.querySelector<HTMLInputElement>('input[type="date"], input[name="expiresAt"]');
      if (dateInput) {
        dateInput.value = '2030-01-01';
        dateInput.dispatchEvent(new Event('input'));
      }

      const approveBtn = document.querySelector<HTMLElement>('[data-action="approve"], button.approve');
      approveBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      // Screen must reload case detail from server
      const caseReadsAfter = sentRequests.filter((r) => r.pathname === '/api/v3/views/verification_case').length;
      expect(caseReadsAfter, 'staleVersion must trigger a reload of the case view').toBeGreaterThan(initialCaseReadsCount);

      // Decision must NOT be re-sent silently
      const approveRequests = sentRequests.filter((r) => r.method === 'POST' && r.pathname.endsWith('/approve'));
      expect(approveRequests.length, 'decision must NOT be re-sent silently').toBe(1);

      // User must be alerted with worded message
      const text = document.body.textContent ?? '';
      expect(text).not.toContain('stale version');
      const errorBox = document.querySelector<HTMLElement>('[data-error], .error-message, .toast-error');
      expect(errorBox, 'must render error notification for staleVersion').not.toBeNull();
      const errText = errorBox?.textContent ?? '';
      expect(
        errText.includes('ข้อมูล') ||
          errText.includes('เปลี่ยนแปลง') ||
          errText.toLowerCase().includes('changed') ||
          errText.toLowerCase().includes('reload') ||
          errText.toLowerCase().includes('refresh'),
        'must display worded notification explaining data changed / reloading',
      ).toBe(true);
    });

    it('20. outOfScope: HTTP 403 outOfScope displays a worded scope/permission error', async () => {
      commandStatus = 403;
      commandErrorCode = 'outOfScope';
      commandErrorMessage = 'Action out of scope';

      await mountPage();
      await openCase(CASE_ID_1);

      const startBtn = document.querySelector<HTMLElement>('[data-action="start-review"], button.start-review');
      startBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const text = document.body.textContent ?? '';
      expect(text).not.toContain('Action out of scope');
      const errorBox = document.querySelector<HTMLElement>('[data-error], .error-message, .toast-error');
      expect(errorBox, 'must render worded error box for outOfScope').not.toBeNull();
      const errText = errorBox?.textContent ?? '';
      expect(
        errText.includes('ไม่มีสิทธิ์') ||
          errText.includes('นอกขอบเขต') ||
          errText.includes('นอกสิทธิ์') ||
          errText.toLowerCase().includes('scope') ||
          errText.toLowerCase().includes('permission') ||
          errText.toLowerCase().includes('not allowed'),
        'must display worded scope/permission error',
      ).toBe(true);
    });

    it('21. validationFailed: server validationFailed displays a worded error with field indication', async () => {
      commandStatus = 400;
      commandErrorCode = 'validationFailed';
      commandErrorMessage = 'invalid reason code';

      await mountPage();
      await openCase(CASE_ID_1);

      const startBtn = document.querySelector<HTMLElement>('[data-action="start-review"], button.start-review');
      startBtn?.click();

      for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

      const text = document.body.textContent ?? '';
      expect(text).not.toContain('invalid reason code');
      const errorBox = document.querySelector<HTMLElement>('[data-error], .error-message, .toast-error');
      expect(errorBox, 'must render worded validation error').not.toBeNull();
      const errText = errorBox?.textContent ?? '';
      expect(
        errText.includes('ข้อมูลไม่ถูกต้อง') ||
          errText.includes('ไม่ถูกต้อง') ||
          errText.toLowerCase().includes('validation') ||
          errText.toLowerCase().includes('invalid'),
        'must display worded validation error',
      ).toBe(true);
    });
  });
});
