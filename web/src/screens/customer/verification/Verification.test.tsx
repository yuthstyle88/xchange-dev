// Written before the implementation by the test writer gemini/warut-6 (ticket author claude/e92db6). Do not edit, delete or weaken.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { audience, setAudience, setLang } from '../../../shared/i18n';
import { mount } from './main';

// =================================================================================================
// Node.js dynamic import interfaces (for TS strict browser environment without @types/node)
// =================================================================================================

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
  readdirSync(dir: string): string[];
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
}

interface NodeUrl {
  fileURLToPath(u: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';
const urlName = 'node:url';

let fs: NodeFs;
let path: NodePath;
let dir = '';

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const { fileURLToPath } = (await import(/* @vite-ignore */ urlName)) as unknown as NodeUrl;
  dir = path.dirname(fileURLToPath(import.meta.url)) + '/';
});

// =================================================================================================
// Pure local types (NO generated types imported)
// =================================================================================================

interface VerificationCaseView {
  id: string;
  status: 'DRAFT' | 'SUBMITTED' | 'UNDER_REVIEW' | 'VERIFIED' | 'REJECTED' | 'NEEDS_UPDATE' | 'EXPIRED' | 'REVOKED';
  policyVersion: number;
  submittedAt: string | null;
  decidedAt: string | null;
  expiresAt: string | null;
  decisionReasonCode: string | null;
  version: number;
}

interface VerificationDocumentView {
  id: string;
  caseId: string;
  documentType: 'PASSPORT' | 'NATIONAL_ID';
  mime: string;
  sizeBytes: number;
  uploadedAt: string;
}

interface MyVerificationView {
  case: VerificationCaseView | null;
  documents: VerificationDocumentView[];
  readiness: 'NONE' | 'PENDING' | 'NEEDS_UPDATE' | 'VERIFIED' | 'EXPIRED';
}

interface RecordedCall {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: unknown;
}

interface MockBackendOptions {
  viewData?: MyVerificationView;
  viewStatus?: number;
  viewErrorCode?: string;
  viewErrorMessage?: string;
  profileData?: { displayName: string; permissions: string[] };
  profileStatus?: number;
  openCaseResponse?: { case: VerificationCaseView } | { error: { code: string; message: string } };
  openCaseStatus?: number;
  uploadResponse?: { document: VerificationDocumentView } | { error: { code: string; message: string } };
  uploadStatus?: number;
  removeResponse?: { ok: boolean } | { error: { code: string; message: string } };
  removeStatus?: number;
  submitResponse?: { case: VerificationCaseView } | { error: { code: string; message: string } };
  submitStatus?: number;
  linkResponse?: { url: string; expiresAt: string } | { error: { code: string; message: string } };
  linkStatus?: number;
}

const WAIT_OPTS = { timeout: 200, interval: 20 };

function setupMockFetch(options: MockBackendOptions = {}) {
  const calls: RecordedCall[] = [];

  const defaultView: MyVerificationView = {
    case: null,
    documents: [],
    readiness: 'NONE',
  };

  const defaultProfile = {
    displayName: 'Customer Test',
    permissions: ['CustomerSelf', 'SignedIn'],
  };

  const fetchHandler = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(url, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    const headers: Record<string, string> = {};
    if (init?.headers) {
      new Headers(init.headers).forEach((v, k) => {
        headers[k.toLowerCase()] = v;
      });
    }

    let parsedBody: unknown = init?.body;
    if (typeof init?.body === 'string') {
      try {
        parsedBody = JSON.parse(init.body);
      } catch {
        parsedBody = init.body;
      }
    }

    calls.push({ method, url: u.pathname, headers, body: parsedBody });

    // Profile view for CustomerShell & CustomerSelf gate: GET /api/v3/views/my_profile
    if (u.pathname.includes('/api/v3/views/my_profile')) {
      if (options.profileStatus && options.profileStatus !== 200) {
        return new Response(
          JSON.stringify({ error: { code: 'unauthorized', message: 'Unauthorized' } }),
          { status: options.profileStatus, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(
        JSON.stringify(options.profileData ?? defaultProfile),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }

    // design docs/design/blueprint/verification.md §4.4 line 218: view my_verification v3 view
    // the open or newest case: status, reason code, expiresAt, documents as metadata, readiness
    if (u.pathname.includes('/api/v3/views/my_verification')) {
      if (options.viewStatus && options.viewStatus !== 200) {
        return new Response(
          JSON.stringify({
            error: {
              code: options.viewErrorCode ?? 'error',
              message: options.viewErrorMessage ?? 'Failed to load verification',
            },
          }),
          { status: options.viewStatus, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(
        JSON.stringify(options.viewData ?? defaultView),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }

    // design docs/design/blueprint/verification.md §4.4 line 213: verificationCaseOpen POST /api/app/verification/cases
    // returns the open case if one exists
    if (u.pathname === '/api/app/verification/cases' && method === 'POST') {
      const status = options.openCaseStatus ?? 200;
      const resp = options.openCaseResponse ?? {
        case: {
          id: 'case-new-1',
          status: 'DRAFT',
          policyVersion: 1,
          submittedAt: null,
          decidedAt: null,
          expiresAt: null,
          decisionReasonCode: null,
          version: 1,
        },
      };
      return new Response(JSON.stringify(resp), {
        status,
        headers: { 'content-type': 'application/json' },
      });
    }

    // design docs/design/blueprint/verification.md §4.4 line 214: document upload (multipart documentType, file; header Idempotency-Key) POST /api/app/verification/cases/{id}/documents
    // bytes go to the store (§12 S-1, S-2), never to the DB
    if (u.pathname.match(/\/api\/app\/verification\/cases\/[^/]+\/documents/) && method === 'POST') {
      const status = options.uploadStatus ?? 200;
      const resp = options.uploadResponse ?? {
        document: {
          id: 'doc-uploaded-1',
          caseId: 'case-new-1',
          documentType: 'PASSPORT',
          mime: 'image/jpeg',
          sizeBytes: 2048,
          uploadedAt: '2026-10-14T10:00:00Z',
        },
      };
      return new Response(JSON.stringify(resp), {
        status,
        headers: { 'content-type': 'application/json' },
      });
    }

    // design docs/design/blueprint/verification.md §4.4 line 215: verificationDocumentRemove POST /api/app/verification/documents/{id}/remove
    // DRAFT/NEEDS_UPDATE only
    if (u.pathname.match(/\/api\/app\/verification\/documents\/[^/]+\/remove/) && method === 'POST') {
      const status = options.removeStatus ?? 200;
      const resp = options.removeResponse ?? { ok: true };
      return new Response(JSON.stringify(resp), {
        status,
        headers: { 'content-type': 'application/json' },
      });
    }

    // design docs/design/blueprint/verification.md §4.4 line 216: verificationCaseSubmit POST /api/app/verification/cases/{id}/submit
    // from DRAFT or NEEDS_UPDATE
    if (u.pathname.match(/\/api\/app\/verification\/cases\/[^/]+\/submit/) && method === 'POST') {
      const status = options.submitStatus ?? 200;
      const resp = options.submitResponse ?? {
        case: {
          id: 'case-new-1',
          status: 'SUBMITTED',
          policyVersion: 1,
          submittedAt: '2026-10-14T10:05:00Z',
          decidedAt: null,
          expiresAt: null,
          decisionReasonCode: null,
          version: 2,
        },
      };
      return new Response(JSON.stringify(resp), {
        status,
        headers: { 'content-type': 'application/json' },
      });
    }

    // design docs/design/blueprint/verification.md §4.4 line 217: verificationMyDocumentLink → {url, expiresAt} POST /api/app/verification/documents/{id}/link
    // owner only; audited
    if (u.pathname.match(/\/api\/app\/verification\/documents\/[^/]+\/link/) && method === 'POST') {
      const status = options.linkStatus ?? 200;
      const resp = options.linkResponse ?? {
        url: 'https://downloads.xchange.test/doc-link-token',
        expiresAt: '2026-10-14T10:01:00Z',
      };
      return new Response(JSON.stringify(resp), {
        status,
        headers: { 'content-type': 'application/json' },
      });
    }

    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };

  vi.stubGlobal('fetch', fetchHandler);
  return { calls };
}

describe('VF-10 (#1401) Customer Verification Screen C10', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    host.id = 'root';
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  // ===============================================================================================
  // Group 1: HTML entry, main.tsx and LK-1 shell contract
  // ===============================================================================================
  describe('Group 1: HTML entry and main.tsx shell contract', () => {
    it('web/pages/app-verification.html exists with xc-path, xc-gate=anyone and module script pointing to main.tsx', () => {
      const htmlPath = path.resolve(dir, '../../../../pages/app-verification.html');
      expect(fs.existsSync(htmlPath), 'web/pages/app-verification.html must exist').toBe(true);

      const html = fs.readFileSync(htmlPath, 'utf-8');
      expect(html).toContain('name="xc-path" content="/app/verification"');
      expect(html).toContain('name="xc-gate" content="anyone"');
      expect(html).toContain('src="/src/screens/customer/verification/main.tsx"');
    });

    it('main.tsx sets audience to customer before render and composes CustomerShell', () => {
      const mainPath = path.resolve(dir, 'main.tsx');
      expect(fs.existsSync(mainPath), 'web/src/screens/customer/verification/main.tsx must exist').toBe(true);

      const mainSrc = fs.readFileSync(mainPath, 'utf-8');
      // Must set audience customer before render (LN-1)
      expect(mainSrc).toMatch(/setAudience\(['"]customer['"]\)/);

      // Must import CustomerShell from .../_shell/Shell
      expect(mainSrc).toMatch(/import\s*\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s*['"][./]*_shell\/Shell['"]/);
      expect(mainSrc).toMatch(/<CustomerShell\b/);

      // Must not import AppShell directly (composed via CustomerShell)
      expect(mainSrc).not.toMatch(/import\s*\{[^}]*\bAppShell\b[^}]*\}/);
      expect(mainSrc).not.toMatch(/<LangSwitch/);
    });

    it('main.tsx executes setAudience customer and exports mount function to mount page through real entry', () => {
      setAudience('staff');
      expect(audience()).toBe('staff');
      setupMockFetch();
      dispose = mount(host);
      expect(audience()).toBe('customer');
      // CustomerShell container is rendered into host
      expect(host.querySelector('.xc-shell') !== null || host.childNodes.length > 0).toBe(true);
    });
  });

  // ===============================================================================================
  // Group 2: Role literals forbidden & gated by CustomerSelf permission (can / needs)
  // ===============================================================================================
  describe('Group 2: Role literals forbidden & gated by CustomerSelf permission', () => {
    it('verification screen source code contains NO role literals across any module in the directory (gated by CustomerSelf permission only)', () => {
      const allFiles = fs.readdirSync(dir);
      const codeFiles = allFiles.filter(
        (f) => (f.endsWith('.ts') || f.endsWith('.tsx')) && !f.includes('.test.'),
      );
      expect(codeFiles.length, 'must find screen code files').toBeGreaterThan(0);

      // All 9 roles from xc-auth/src/domain/access/role.rs
      const rolePattern = /\b(CUSTOMER|TELLER|VERIFICATION_REVIEWER|ADMIN|EXECUTIVE|RATE_MANAGER|BRANCH_MANAGER|FINANCE|MARKETING)\b/;

      for (const file of codeFiles) {
        const filePath = path.resolve(dir, file);
        const content = fs.readFileSync(filePath, 'utf-8');
        const lines = content.split('\n');
        for (let i = 0; i < lines.length; i++) {
          const line = lines[i]!.trim();
          if (line.startsWith('//') || line.startsWith('*') || line.startsWith('/*')) continue;
          expect(
            rolePattern.test(line),
            `File ${file}:${i + 1} must not contain role literal "${line}" (use can(me, 'CustomerSelf') or needs instead)`,
          ).toBe(false);
        }
      }
    });

    it('mounts and renders verification screen when caller holds CustomerSelf permission', async () => {
      setupMockFetch({
        profileData: { displayName: 'Verified Customer', permissions: ['CustomerSelf', 'SignedIn'] },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        expect(host.querySelector('[data-verification-screen], [data-screen="verification"]')).not.toBeNull();
      }, WAIT_OPTS);
    });

    it('blocks access when caller lacks CustomerSelf permission (gated by can() / needs)', async () => {
      setupMockFetch({
        profileData: { displayName: 'Staff User', permissions: ['Teller', 'SignedIn'] },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        // Verification form must not be accessible to non-customer callers
        expect(host.querySelector('[data-screen="verification"], [data-verification-screen]')).toBeNull();
      }, WAIT_OPTS);
    });
  });

  // ===============================================================================================
  // Group 3: C10 four steps in TH / EN / 中文 (Blueprint §15, §19 L-A4, §9.1 VF-10)
  // ===============================================================================================
  describe('Group 3: C10 four steps in TH / EN / 中文', () => {
    it('renders the four C10 steps in Thai (เลือกเอกสาร → ส่งข้อมูล → รอตรวจ → ผลตรวจ/แก้ไข)', async () => {
      setLang('th');
      setupMockFetch();

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        expect(text).toContain('เลือกเอกสาร');
        expect(text).toContain('ส่งข้อมูล');
        expect(text).toContain('รอตรวจ');
        expect(text).toContain('ผลตรวจ/แก้ไข');
      }, WAIT_OPTS);
    });

    it('renders the four C10 steps in English', async () => {
      setLang('en');
      setupMockFetch();

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = (host.textContent ?? '').toLowerCase();
        // Step 1: choose/select document
        expect(text.includes('select document') || text.includes('choose document')).toBe(true);
        // Step 2: upload/submit documents
        expect(text.includes('upload') || text.includes('submit')).toBe(true);
        // Step 3: waiting/under review
        expect(text.includes('under review') || text.includes('awaiting')).toBe(true);
        // Step 4: result/update/edit
        expect(text.includes('result') || text.includes('update')).toBe(true);
      }, WAIT_OPTS);
    });

    it('renders the four C10 steps in Chinese (中文)', async () => {
      setLang('zh');
      setupMockFetch();

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        // Step 1: 选择证件
        expect(text.includes('选择证件') || text.includes('选择文件')).toBe(true);
        // Step 2: 提交信息 / 上传证件
        expect(text.includes('提交信息') || text.includes('上传证件') || text.includes('上传')).toBe(true);
        // Step 3: 等待审核
        expect(text.includes('等待审核') || text.includes('审核中')).toBe(true);
        // Step 4: 审核结果/修改
        expect(text.includes('结果') || text.includes('修改')).toBe(true);
      }, WAIT_OPTS);
    });
  });

  // ===============================================================================================
  // Group 4: Step 1 & Step 2: Document selection, upload with Idempotency-Key, remove, submit
  // ===============================================================================================
  describe('Group 4: Document selection, upload with Idempotency-Key, remove, submit', () => {
    it('Step 1: allows choosing document type and opening case via POST /api/app/verification/cases', async () => {
      const { calls } = setupMockFetch();

      dispose = mount(host);

      await vi.waitFor(() => {
        const startBtn = host.querySelector<HTMLButtonElement>('[data-action="start"], [data-action="open-case"]');
        expect(startBtn).not.toBeNull();
      }, WAIT_OPTS);

      const startBtn = host.querySelector<HTMLButtonElement>('[data-action="start"], [data-action="open-case"]')!;
      startBtn.click();

      await vi.waitFor(() => {
        // design docs/design/blueprint/verification.md §4.4 line 213: verificationCaseOpen POST /api/app/verification/cases
        const openCall = calls.find((c) => c.url === '/api/app/verification/cases' && c.method === 'POST');
        expect(openCall).toBeDefined();
        expect(openCall?.headers['idempotency-key']).toBeDefined();
      }, WAIT_OPTS);
    });

    it('Step 2: multipart document upload sends POST /api/app/verification/cases/{id}/documents with Idempotency-Key header', async () => {
      const draftCase: VerificationCaseView = {
        id: 'case-draft-44',
        status: 'DRAFT',
        policyVersion: 1,
        submittedAt: null,
        decidedAt: null,
        expiresAt: null,
        decisionReasonCode: null,
        version: 1,
      };

      const { calls } = setupMockFetch({
        viewData: { case: draftCase, documents: [], readiness: 'NONE' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        expect(host.querySelector('input[type="file"]')).not.toBeNull();
      }, WAIT_OPTS);

      const fileInput = host.querySelector<HTMLInputElement>('input[type="file"]')!;
      const testFile = new File(['mock image content'], 'passport.jpg', { type: 'image/jpeg' });
      Object.defineProperty(fileInput, 'files', { value: [testFile] });
      fileInput.dispatchEvent(new Event('change', { bubbles: true }));

      const uploadBtn = host.querySelector<HTMLButtonElement>('[data-action="upload"]');
      uploadBtn?.click();

      await vi.waitFor(() => {
        // design docs/design/blueprint/verification.md §4.4 line 214: document upload POST /api/app/verification/cases/{id}/documents
        const uploadCall = calls.find(
          (c) => c.url.includes('/api/app/verification/cases/case-draft-44/documents') && c.method === 'POST',
        );
        expect(uploadCall).toBeDefined();
        // Idempotency-Key header must be sent (§09, §10, §9.1 VF-10)
        expect(uploadCall?.headers['idempotency-key']).toBeDefined();
        expect(uploadCall?.headers['idempotency-key']!.length).toBeGreaterThan(0);
      }, WAIT_OPTS);
    });

    it('upload retried after network failure keeps the same Idempotency-Key header', async () => {
      const draftCase: VerificationCaseView = {
        id: 'case-draft-44',
        status: 'DRAFT',
        policyVersion: 1,
        submittedAt: null,
        decidedAt: null,
        expiresAt: null,
        decisionReasonCode: null,
        version: 1,
      };

      let attempt = 0;
      const recordedKeys: string[] = [];

      setupMockFetch({
        viewData: { case: draftCase, documents: [], readiness: 'NONE' },
      });

      const originalFetch = globalThis.fetch;
      vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/documents') && (init?.method ?? 'GET').toUpperCase() === 'POST') {
          const headers = new Headers(init?.headers);
          const key = headers.get('idempotency-key') ?? '';
          recordedKeys.push(key);
          attempt++;
          if (attempt === 1) {
            return new Response(JSON.stringify({ error: { code: 'networkError', message: 'Temporary error' } }), {
              status: 503,
              headers: { 'content-type': 'application/json' },
            });
          }
        }
        return originalFetch(input, init);
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        expect(host.querySelector('input[type="file"]')).not.toBeNull();
      }, WAIT_OPTS);

      const fileInput = host.querySelector<HTMLInputElement>('input[type="file"]')!;
      const testFile = new File(['mock bytes'], 'passport.jpg', { type: 'image/jpeg' });
      Object.defineProperty(fileInput, 'files', { value: [testFile] });
      fileInput.dispatchEvent(new Event('change', { bubbles: true }));

      const uploadBtn = host.querySelector<HTMLButtonElement>('[data-action="upload"]');
      uploadBtn?.click();

      await vi.waitFor(() => {
        expect(recordedKeys.length).toBeGreaterThanOrEqual(1);
      }, WAIT_OPTS);

      // Trigger retry via pinned [data-action="retry"]
      const retryBtn = host.querySelector<HTMLButtonElement>('[data-action="retry"]');
      expect(retryBtn, 'retry button must be rendered on upload error').not.toBeNull();
      retryBtn!.click();

      await vi.waitFor(() => {
        expect(recordedKeys.length).toBeGreaterThanOrEqual(2);
        // The retry must preserve the exact same Idempotency-Key
        expect(recordedKeys[1]).toBe(recordedKeys[0]);
      }, WAIT_OPTS);
    });

    it('allows removing an uploaded document in DRAFT via POST /api/app/verification/documents/{id}/remove', async () => {
      const draftCase: VerificationCaseView = {
        id: 'case-draft-44',
        status: 'DRAFT',
        policyVersion: 1,
        submittedAt: null,
        decidedAt: null,
        expiresAt: null,
        decisionReasonCode: null,
        version: 1,
      };

      const docs: VerificationDocumentView[] = [
        {
          id: 'doc-rem-1',
          caseId: 'case-draft-44',
          documentType: 'PASSPORT',
          mime: 'image/jpeg',
          sizeBytes: 2048,
          uploadedAt: '2026-10-14T10:00:00Z',
        },
      ];

      const { calls } = setupMockFetch({
        viewData: { case: draftCase, documents: docs, readiness: 'PENDING' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const removeBtn = host.querySelector<HTMLButtonElement>('[data-action="remove"], [data-remove-doc="doc-rem-1"]');
        expect(removeBtn).not.toBeNull();
      }, WAIT_OPTS);

      const removeBtn = host.querySelector<HTMLButtonElement>('[data-action="remove"], [data-remove-doc="doc-rem-1"]')!;
      removeBtn.click();

      await vi.waitFor(() => {
        // design docs/design/blueprint/verification.md §4.4 line 215: verificationDocumentRemove POST /api/app/verification/documents/{id}/remove
        const removeCall = calls.find(
          (c) => c.url.includes('/api/app/verification/documents/doc-rem-1/remove') && c.method === 'POST',
        );
        expect(removeCall).toBeDefined();
        expect(removeCall?.headers['idempotency-key']).toBeDefined();
      }, WAIT_OPTS);
    });

    it('submits case via POST /api/app/verification/cases/{id}/submit with Idempotency-Key header', async () => {
      const draftCase: VerificationCaseView = {
        id: 'case-draft-44',
        status: 'DRAFT',
        policyVersion: 1,
        submittedAt: null,
        decidedAt: null,
        expiresAt: null,
        decisionReasonCode: null,
        version: 1,
      };

      const docs: VerificationDocumentView[] = [
        {
          id: 'doc-sub-1',
          caseId: 'case-draft-44',
          documentType: 'PASSPORT',
          mime: 'image/jpeg',
          sizeBytes: 2048,
          uploadedAt: '2026-10-14T10:00:00Z',
        },
      ];

      const { calls } = setupMockFetch({
        viewData: { case: draftCase, documents: docs, readiness: 'PENDING' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const submitBtn = host.querySelector<HTMLButtonElement>('[data-action="submit"]');
        expect(submitBtn).not.toBeNull();
      }, WAIT_OPTS);

      const submitBtn = host.querySelector<HTMLButtonElement>('[data-action="submit"]')!;
      submitBtn.click();

      await vi.waitFor(() => {
        // design docs/design/blueprint/verification.md §4.4 line 216: verificationCaseSubmit POST /api/app/verification/cases/{id}/submit
        const submitCall = calls.find(
          (c) => c.url.includes('/api/app/verification/cases/case-draft-44/submit') && c.method === 'POST',
        );
        expect(submitCall).toBeDefined();
        expect(submitCall?.headers['idempotency-key']).toBeDefined();
      }, WAIT_OPTS);
    });
  });

  // ===============================================================================================
  // Group 5: Step 3: Waiting state & No reviewer mutation from customer UI
  // ===============================================================================================
  describe('Group 5: Step 3: Waiting state & No reviewer mutation from customer UI', () => {
    it('Step 3: in SUBMITTED or UNDER_REVIEW state, displays waiting text and freezes document modifications', async () => {
      const submittedCase: VerificationCaseView = {
        id: 'case-sub-10',
        status: 'SUBMITTED',
        policyVersion: 1,
        submittedAt: '2026-10-14T10:00:00Z',
        decidedAt: null,
        expiresAt: null,
        decisionReasonCode: null,
        version: 2,
      };

      const docs: VerificationDocumentView[] = [
        {
          id: 'doc-sub-10',
          caseId: 'case-sub-10',
          documentType: 'PASSPORT',
          mime: 'image/jpeg',
          sizeBytes: 4096,
          uploadedAt: '2026-10-14T10:00:00Z',
        },
      ];

      setupMockFetch({
        viewData: { case: submittedCase, documents: docs, readiness: 'PENDING' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        expect(text.includes('รอตรวจ') || text.includes('ตรวจสอบ')).toBe(true);
      }, WAIT_OPTS);

      // Frozen documents after submit (§16 "ลูกค้าแก้ไม่ได้"): no remove button and no upload input
      expect(host.querySelector('[data-action="remove"]')).toBeNull();
      expect(host.querySelector('input[type="file"]')).toBeNull();
      expect(host.querySelector('[data-action="upload"]')).toBeNull();
    });

    it('customer UI has NO button or route that sets a reviewer state (VERIFIED, UNDER_REVIEW, REJECTED, REVOKED)', async () => {
      const reviewCase: VerificationCaseView = {
        id: 'case-rev-5',
        status: 'UNDER_REVIEW',
        policyVersion: 1,
        submittedAt: '2026-10-14T09:00:00Z',
        decidedAt: null,
        expiresAt: null,
        decisionReasonCode: null,
        version: 3,
      };

      const { calls } = setupMockFetch({
        viewData: { case: reviewCase, documents: [], readiness: 'PENDING' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        expect(host.childNodes.length).toBeGreaterThan(0);
      }, WAIT_OPTS);

      // Verify no reviewer transition actions exist (§18 "customer ไม่มีสิทธิ set VERIFIED")
      const buttons = Array.from(host.querySelectorAll('button, a'));
      for (const btn of buttons) {
        const action = (btn.getAttribute('data-action') ?? '').toLowerCase();
        const text = (btn.textContent ?? '').toLowerCase();
        expect(action).not.toBe('approve');
        expect(action).not.toBe('reject');
        expect(action).not.toBe('startreview');
        expect(action).not.toBe('revoke');
        expect(text).not.toContain('อนุมัติ');
        expect(text).not.toContain('ปฏิเสธ');
        expect(text).not.toContain('approve');
        expect(text).not.toContain('reject');
      }

      // Assert that no request was made to reviewer/operations routes (pinning "no route" half of §18)
      const reviewerCalls = calls.filter((c) => c.url.includes('/api/operations/verification/'));
      expect(reviewerCalls, 'customer screen must never call /api/operations/verification/ routes').toHaveLength(0);
    });
  });

  // ===============================================================================================
  // Group 6: Step 4: Decision states & Resubmission as a NEW case (§18 / V2)
  // ===============================================================================================
  describe('Group 6: Step 4: Decision states & Resubmission as a NEW case', () => {
    it('VERIFIED state: displays expiresAt and hides edit buttons', async () => {
      const verifiedCase: VerificationCaseView = {
        id: 'case-ver-1',
        status: 'VERIFIED',
        policyVersion: 1,
        submittedAt: '2026-10-10T10:00:00Z',
        decidedAt: '2026-10-10T11:00:00Z',
        expiresAt: '2027-10-10T23:59:59Z',
        decisionReasonCode: null,
        version: 4,
      };

      const docs: VerificationDocumentView[] = [
        {
          id: 'doc-v-1',
          caseId: 'case-ver-1',
          documentType: 'PASSPORT',
          mime: 'image/jpeg',
          sizeBytes: 4096,
          uploadedAt: '2026-10-10T10:00:00Z',
        },
      ];

      const { calls } = setupMockFetch({
        viewData: { case: verifiedCase, documents: docs, readiness: 'VERIFIED' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        expect(text).toContain('2027');
      }, WAIT_OPTS);

      // No edit buttons in VERIFIED
      expect(host.querySelector('[data-action="remove"]')).toBeNull();
      expect(host.querySelector('input[type="file"]')).toBeNull();

      // Can request document link via POST /api/app/verification/documents/{id}/link (line 217)
      const linkBtn = host.querySelector<HTMLButtonElement>('[data-action="view-document"], [data-view-doc="doc-v-1"]');
      linkBtn?.click();

      await vi.waitFor(() => {
        const linkCall = calls.find(
          (c) => c.url.includes('/api/app/verification/documents/doc-v-1/link') && c.method === 'POST',
        );
        expect(linkCall).toBeDefined();
      }, WAIT_OPTS);
    });

    it('NEEDS_UPDATE state: shows reason and opens fix path on the SAME case', async () => {
      const updateCase: VerificationCaseView = {
        id: 'case-update-77',
        status: 'NEEDS_UPDATE',
        policyVersion: 1,
        submittedAt: '2026-10-12T10:00:00Z',
        decidedAt: '2026-10-12T12:00:00Z',
        expiresAt: null,
        decisionReasonCode: 'Blurry photo taken at night',
        version: 3,
      };

      const docs: VerificationDocumentView[] = [
        {
          id: 'doc-bad-1',
          caseId: 'case-update-77',
          documentType: 'PASSPORT',
          mime: 'image/jpeg',
          sizeBytes: 2048,
          uploadedAt: '2026-10-12T10:00:00Z',
        },
      ];

      const { calls } = setupMockFetch({
        viewData: { case: updateCase, documents: docs, readiness: 'NEEDS_UPDATE' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        // S-5 is open: show server's reason text as given
        expect(text).toContain('Blurry photo taken at night');
      }, WAIT_OPTS);

      // Resubmit on the SAME case id (line 216)
      const resubmitBtn = host.querySelector<HTMLButtonElement>(
        '[data-action="resubmit"], [data-action="submit"]',
      );
      expect(resubmitBtn).not.toBeNull();
      resubmitBtn!.click();

      await vi.waitFor(() => {
        const submitCall = calls.find(
          (c) => c.url.includes('/api/app/verification/cases/case-update-77/submit') && c.method === 'POST',
        );
        expect(submitCall, 'resubmission for NEEDS_UPDATE must use the SAME case').toBeDefined();
      }, WAIT_OPTS);
    });

    it('REJECTED state: shows server reason and offers update-and-resubmit, creating a NEW case (§18 / V2)', async () => {
      const rejectedCase: VerificationCaseView = {
        id: 'case-rej-9',
        status: 'REJECTED',
        policyVersion: 1,
        submittedAt: '2026-10-11T10:00:00Z',
        decidedAt: '2026-10-11T14:00:00Z',
        expiresAt: null,
        decisionReasonCode: 'Passport signature missing on page 2',
        version: 3,
      };

      const { calls } = setupMockFetch({
        viewData: { case: rejectedCase, documents: [], readiness: 'NONE' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        // Show server's reason text as given (S-5 open, do NOT pin reason code list)
        expect(text).toContain('Passport signature missing on page 2');
      }, WAIT_OPTS);

      // Offers update-and-resubmit (§18 / V2) via pinned data-action
      const resubmitBtn = host.querySelector<HTMLButtonElement>(
        '[data-action="update-and-resubmit"]',
      );
      expect(resubmitBtn, 'update-and-resubmit button must be rendered in REJECTED state').not.toBeNull();
      resubmitBtn!.click();

      await vi.waitFor(() => {
        // Must call case open to create a NEW case (§18 / V2, line 213), NOT resubmit on the ended case
        const openCall = calls.find((c) => c.url === '/api/app/verification/cases' && c.method === 'POST');
        expect(openCall, 'resubmission after REJECTED must create a NEW case via POST /api/app/verification/cases').toBeDefined();
      }, WAIT_OPTS);
    });

    it('EXPIRED state: shows server reason and offers update-and-resubmit, creating a NEW case (§18 / V2)', async () => {
      const expiredCase: VerificationCaseView = {
        id: 'case-exp-8',
        status: 'EXPIRED',
        policyVersion: 1,
        submittedAt: '2025-10-10T10:00:00Z',
        decidedAt: '2025-10-10T11:00:00Z',
        expiresAt: '2026-10-10T23:59:59Z',
        decisionReasonCode: 'Identification document expired on 2026-10-10',
        version: 5,
      };

      const { calls } = setupMockFetch({
        viewData: { case: expiredCase, documents: [], readiness: 'EXPIRED' },
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        // Show server's reason text as given (S-5 open)
        expect(text.includes('Identification document expired on 2026-10-10') || text.includes('2026-10-10')).toBe(true);
      }, WAIT_OPTS);

      // Offers update-and-resubmit (§18 / V2) via pinned data-action
      const resubmitBtn = host.querySelector<HTMLButtonElement>(
        '[data-action="update-and-resubmit"]',
      );
      expect(resubmitBtn, 'update-and-resubmit button must be rendered in EXPIRED state').not.toBeNull();
      resubmitBtn!.click();

      await vi.waitFor(() => {
        // Must call case open to create a NEW case (§18 / V2, line 213)
        const openCall = calls.find((c) => c.url === '/api/app/verification/cases' && c.method === 'POST');
        expect(openCall, 'resubmission after EXPIRED must create a NEW case via POST /api/app/verification/cases').toBeDefined();
      }, WAIT_OPTS);
    });
  });

  // ===============================================================================================
  // Group 7: Refusal handling: 400 roleNotAllowed and 403 outOfScope shown as worded messages, never raw codes
  // ===============================================================================================
  describe('Group 7: Refusal handling as worded messages, never raw codes', () => {
    it('handles 400 roleNotAllowed by displaying a worded message, NEVER the raw code roleNotAllowed', async () => {
      setupMockFetch({
        viewStatus: 400,
        viewErrorCode: 'roleNotAllowed',
        viewErrorMessage: 'Caller does not possess customer role',
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        const lower = text.toLowerCase();
        const hasWordedMessage =
          text.includes('ไม่มีสิทธิ์') ||
          lower.includes('not permitted') ||
          lower.includes('caller does not possess') ||
          text.includes('กรุณาเข้าสู่ระบบ');
        expect(hasWordedMessage, 'must display a worded refusal message').toBe(true);
        // Raw code must never be shown to the user (checked case-insensitively)
        expect(lower, 'raw error code roleNotAllowed must not be displayed').not.toContain('rolenotallowed');
      }, WAIT_OPTS);
    });

    it('handles 403 outOfScope by displaying a worded message, NEVER the raw code outOfScope', async () => {
      setupMockFetch({
        viewStatus: 403,
        viewErrorCode: 'outOfScope',
        viewErrorMessage: 'Access out of scope for current user',
      });

      dispose = mount(host);

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        const lower = text.toLowerCase();
        const hasWordedMessage =
          text.includes('ไม่มีสิทธิ์') ||
          lower.includes('access denied') ||
          lower.includes('out of scope');
        expect(hasWordedMessage, 'must display a worded refusal message').toBe(true);
        // Raw codes must never be shown to the user (checked case-insensitively)
        expect(lower, 'raw error code outOfScope must not be displayed').not.toContain('outofscope');
      }, WAIT_OPTS);
    });
  });
});
