// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { makeMyVerificationCaseDetail } from '../../../shared/testing/fixtures';
import { mount } from './main';

const WAIT_OPTS = { timeout: 2000 };

const DRAFT_CASE = makeMyVerificationCaseDetail({
  id: 'case-draft-r4-4',
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

describe('R4-4 (#4775): Verification upload server, 401 and non-JSON errors', () => {
  const cleanups: Array<() => void> = [];

  function mountScreen(): HTMLElement {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const dispose = mount(host);
    cleanups.push(() => {
      dispose();
      host.remove();
    });
    return host;
  }

  function mockFetchWithUploadAnswer(uploadAnswer: () => Response): void {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();

      if (url.includes('/api/v3/views/my_profile')) {
        return jsonResponse({
          displayName: 'Customer R4-4',
          permissions: ['CustomerSelf'],
        });
      }
      if (url.includes('/api/v3/views/my_verification')) {
        return jsonResponse({ readiness: 'NONE', case: DRAFT_CASE });
      }
      if (
        url.includes('/api/app/verification/cases/case-draft-r4-4/documents') &&
        method === 'POST'
      ) {
        return uploadAnswer();
      }
      return jsonResponse({});
    });
  }

  async function selectFileAndUpload(host: HTMLElement): Promise<string> {
    await vi.waitFor(() => {
      expect(host.querySelector('input[type="file"]')).not.toBeNull();
    }, WAIT_OPTS);

    const fileInput = host.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File(['sample-bytes'], 'doc.jpg', { type: 'image/jpeg' });
    Object.defineProperty(fileInput, 'files', {
      value: [file],
      configurable: true,
    });
    fileInput.dispatchEvent(new Event('change', { bubbles: true }));

    const uploadBtn = host.querySelector('[data-action="upload"]') as HTMLElement;
    expect(uploadBtn).not.toBeNull();
    uploadBtn.click();

    let errText = '';
    await vi.waitFor(() => {
      const errBox = host.querySelector('[data-upload-error]');
      expect(errBox).not.toBeNull();
      errText = errBox?.textContent?.trim() ?? '';
      expect(errText.length).toBeGreaterThan(0);
    }, WAIT_OPTS);
    return errText;
  }

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
  });

  afterEach(() => {
    while (cleanups.length > 0) {
      cleanups.pop()?.();
    }
    vi.restoreAllMocks();
    document.body.innerHTML = '';
  });

  it('U1: 502 with an HTML body <html>Bad Gateway</html> shows wordedError(err) and never raw body', async () => {
    const htmlBody = '<html>Bad Gateway</html>';
    mockFetchWithUploadAnswer(
      () => new Response(htmlBody, { status: 502, headers: { 'Content-Type': 'text/html' } }),
    );

    const host = mountScreen();
    const errText = await selectFileAndUpload(host);

    const expected = wordedError(
      new ApiError({ code: 'httpError', message: htmlBody, status: 502 }),
    );
    expect(errText).toBe(expected);
    expect(errText).not.toContain('Bad Gateway');
    expect(errText).not.toContain('<html>');
  });

  it('U2: 413 with a non-JSON body shows worded error and never raw body', async () => {
    const rawBody = '413 Request Entity Too Large (nginx)';
    mockFetchWithUploadAnswer(
      () => new Response(rawBody, { status: 413, headers: { 'Content-Type': 'text/plain' } }),
    );

    const host = mountScreen();
    const errText = await selectFileAndUpload(host);

    const expected = wordedError(
      new ApiError({ code: 'httpError', message: rawBody, status: 413 }),
    );
    expect(errText).toBe(expected);
    expect(errText).not.toContain(rawBody);
    expect(errText).not.toContain('Request Entity Too Large');
  });

  it('U3: 500 {error:{code:"serverError",message:"db down"}} shows worded error and never "db down"', async () => {
    mockFetchWithUploadAnswer(() =>
      jsonResponse({ error: { code: 'serverError', message: 'db down' } }, 500),
    );

    const host = mountScreen();
    const errText = await selectFileAndUpload(host);

    const expected = wordedError(
      new ApiError({ code: 'serverError', message: 'db down', status: 500 }),
    );
    expect(errText).toBe(expected);
    expect(errText).not.toContain('db down');
  });

  it('U4: 401 {error:{code:"signIn",message:"raw"}} shows worded error and never "raw"', async () => {
    mockFetchWithUploadAnswer(() =>
      jsonResponse({ error: { code: 'signIn', message: 'raw' } }, 401),
    );

    const host = mountScreen();
    const errText = await selectFileAndUpload(host);

    const expected = wordedError(
      new ApiError({ code: 'signIn', message: 'raw', status: 401 }),
    );
    expect(errText).toBe(expected);
    expect(errText).not.toContain('raw');
  });

  it('U5: 400 {error:{code:"documentTypeNotAccepted",message:"PDF only"}} keeps existing sentence with (PDF only)', async () => {
    mockFetchWithUploadAnswer(() =>
      jsonResponse(
        { error: { code: 'documentTypeNotAccepted', message: 'PDF only' } },
        400,
      ),
    );

    const host = mountScreen();
    const errText = await selectFileAndUpload(host);

    expect(errText).toContain('(PDF only)');
    expect(errText.toLowerCase()).not.toContain('documenttypenotaccepted');
  });
});
