// #4940: written before the fix by the ticket author claude/studio-claude-4 (sub-controller IV-logic under XC FE claude/e8020026).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// Contract these rows set (ticket #4940 §2, design docs/design/web-input-validation.md T25):
// - Customer upload checks the file before any request. A refusal is a line at the file input,
//   `[data-field-error="file"]`, worded from the screen's own keys in `./i18n`:
//   `fileRequired` (no file chosen), `fileTooLarge` (over 15 MB), `fileEmpty` (0 bytes),
//   `fileTypeNotAccepted` (not JPEG, PNG or PDF by `file.type`, the extension as fallback).
// - Ops branch intake keeps the server's own sentence for a file refusal. The server answers
//   400 `{ error: { code: 'validationFailed', field: 'file', message: <sentence> } }`
//   (the shape `VerificationUploadErrors.test.tsx` already pins for the customer upload).
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { makeMyVerificationCaseDetail } from '../../../shared/testing/fixtures';
import { meOf } from '../../../test-support/me';
import { th as opsTh } from '../../operations/verification/i18n';
import { VerificationScreen as OpsVerificationScreen } from '../../operations/verification/VerificationScreen';
import { th } from './i18n';
import { mount } from './main';

const WAIT_OPTS = { timeout: 2000 };
const MB = 1024 * 1024;

/** The screen's own wording; a key the screen does not have yet reads undefined. */
const word = (key: string): string | undefined => (th as Record<string, string>)[key];

const DRAFT_CASE = makeMyVerificationCaseDetail({
  id: 'case-draft-4940',
  organizationId: 'org-1',
  customerId: 'cust-1',
  provider: 'INTERNAL',
  status: 'DRAFT',
  policyVersion: 1,
  branchId: null,
  submittedAt: null,
  decidedAt: null,
  expiresAt: null,
  decisionReasonCode: null,
  version: 1,
  documents: [],
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function urlOf(input: RequestInfo | URL): string {
  return typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
}

/** A File whose reported size is `size`, without allocating it. */
function fileOfSize(name: string, type: string, size: number): File {
  const f = new File(['x'], name, { type });
  Object.defineProperty(f, 'size', { value: size, configurable: true });
  return f;
}

describe('#4940 customer upload: the file is checked before any request (VF3, VF4, VF5, VF7)', () => {
  const cleanups: Array<() => void> = [];
  let uploads = 0;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    uploads = 0;
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
      const url = urlOf(input);
      const method = (init?.method ?? 'GET').toUpperCase();
      if (url.includes('/api/v3/views/my_profile')) {
        return jsonResponse({ displayName: 'Customer 4940', permissions: ['CustomerSelf'] });
      }
      if (url.includes('/api/v3/views/my_verification')) {
        return jsonResponse({ readiness: 'NONE', case: DRAFT_CASE });
      }
      if (url.includes('/api/app/verification/cases/case-draft-4940/documents') && method === 'POST') {
        uploads++;
        return jsonResponse({ id: 'doc-1' });
      }
      return jsonResponse({});
    });
  });

  afterEach(() => {
    while (cleanups.length > 0) cleanups.pop()?.();
    vi.restoreAllMocks();
    document.body.innerHTML = '';
  });

  async function mountScreen(): Promise<HTMLElement> {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const dispose = mount(host);
    cleanups.push(() => {
      dispose();
      host.remove();
    });
    await vi.waitFor(() => {
      expect(host.querySelector('input[type="file"]')).not.toBeNull();
    }, WAIT_OPTS);
    return host;
  }

  function choose(host: HTMLElement, file: File): void {
    const input = host.querySelector('input[type="file"]') as HTMLInputElement;
    Object.defineProperty(input, 'files', { value: [file], configurable: true });
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function upload(host: HTMLElement): Promise<void> {
    (host.querySelector('[data-action="upload"]') as HTMLElement).click();
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
  }

  async function lineAtFile(host: HTMLElement): Promise<string> {
    let text = '';
    await vi.waitFor(() => {
      const el = host.querySelector('[data-field-error="file"]');
      expect(el, 'a line at the file input, [data-field-error="file"]').not.toBeNull();
      text = el!.textContent?.trim() ?? '';
    }, WAIT_OPTS);
    return text;
  }

  it('VF3: Upload with no file chosen shows fileRequired ("choose a file") at the file input; nothing is sent', async () => {
    expect(word('fileRequired'), 'th.fileRequired').toBeTruthy();
    const host = await mountScreen();

    await upload(host);

    expect(await lineAtFile(host)).toBe(word('fileRequired'));
    expect(uploads, 'no upload request').toBe(0);
  });

  it('VF4: a 16 MB JPEG shows fileTooLarge at the file input; nothing is sent', async () => {
    expect(word('fileTooLarge'), 'th.fileTooLarge').toBeTruthy();
    const host = await mountScreen();

    choose(host, fileOfSize('passport.jpg', 'image/jpeg', 16 * MB));
    await upload(host);

    expect(await lineAtFile(host)).toBe(word('fileTooLarge'));
    expect(uploads, 'no upload request').toBe(0);
  });

  it('VF5: a .gif shows fileTypeNotAccepted at the file input; nothing is sent', async () => {
    expect(word('fileTypeNotAccepted'), 'th.fileTypeNotAccepted').toBeTruthy();
    const host = await mountScreen();

    choose(host, fileOfSize('passport.gif', 'image/gif', 2048));
    await upload(host);

    expect(await lineAtFile(host)).toBe(word('fileTypeNotAccepted'));
    expect(uploads, 'no upload request').toBe(0);
  });

  it('VF7: a 0-byte file shows fileEmpty, not the 15 MB line; nothing is sent', async () => {
    expect(word('fileEmpty'), 'th.fileEmpty').toBeTruthy();
    expect(word('fileEmpty'), 'the empty-file line differs from the 15 MB line').not.toBe(word('fileTooLarge'));
    const host = await mountScreen();

    choose(host, fileOfSize('passport.jpg', 'image/jpeg', 0));
    await upload(host);

    const text = await lineAtFile(host);
    expect(text).toBe(word('fileEmpty'));
    expect(text).not.toBe(word('fileTooLarge'));
    expect(uploads, 'no upload request').toBe(0);
  });
});

describe('#4940 ops branch intake: a server file refusal keeps the server sentence (VF6)', () => {
  const ROUTE_BRANCH_CASES = '/api/operations/verification/branch-cases';
  const SERVER_SENTENCE = 'ไฟล์ใหญ่เกิน 15 MB';
  let intakes = 0;
  let dispose: (() => void) | undefined;

  const REVIEWER_SESSION = meOf({
    id: '00000000-0000-4000-8000-000000000001',
    role: 'DOCUMENTS',
    branch: 'MAIN',
    name: 'Reviewer Somchai',
    branchName: 'Head Office',
  });

  beforeEach(() => {
    setLang('th');
    intakes = 0;
    document.body.innerHTML = '';
    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(urlOf(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (url.pathname === '/api/v3/views/me' || url.pathname === '/api/v3/views/staff_me') {
        return jsonResponse({ version: 1, changed: true, data: REVIEWER_SESSION });
      }
      if (url.pathname === '/api/v3/views/verification_policy') {
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            policyVersion: 1,
            branchReviewAllowed: true,
            acceptedDocumentTypes: ['PASSPORT', 'NATIONAL_ID'],
            documentsRequired: 1,
          },
        });
      }
      if (url.pathname === '/api/v3/views/verification_queue') {
        return jsonResponse({ version: 1, changed: true, data: [] });
      }
      if (url.pathname === ROUTE_BRANCH_CASES && method === 'POST') {
        intakes++;
        return jsonResponse(
          { error: { code: 'validationFailed', field: 'file', message: SERVER_SENTENCE } },
          400,
        );
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
    for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
  }

  it('VF6: the intake answer 400 validationFailed on field file shows the server sentence, not errValidationFailed', async () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <OpsVerificationScreen />, host);
    await flush();

    const cust = document.querySelector<HTMLInputElement>('input[name="customerId"]')!;
    cust.value = 'cust-A';
    cust.dispatchEvent(new Event('input', { bubbles: true }));
    const fileInput = document.querySelector<HTMLInputElement>('input[name="file"]')!;
    Object.defineProperty(fileInput, 'files', {
      value: [new File(['passport-bytes'], 'passport.jpg', { type: 'image/jpeg' })],
      configurable: true,
    });
    fileInput.dispatchEvent(new Event('change', { bubbles: true }));
    document.querySelector<HTMLElement>('[data-action="submit-branch-case"]')!.click();
    await flush();

    expect(intakes, 'the valid file is sent once').toBe(1);
    const text = document.body.textContent ?? '';
    expect(text, 'the server sentence shows').toContain(SERVER_SENTENCE);
    expect(text, 'not the generic validation line').not.toContain(opsTh.errValidationFailed);
  });
});
