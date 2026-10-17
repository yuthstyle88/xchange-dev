// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MyVerificationCaseDetail } from '../../../shared/api/generated/my_verification';
import { setAudience, setLang } from '../../../shared/i18n';
import { mount } from './main';

const WAIT_OPTS = { timeout: 2000 };

const DRAFT_CASE: MyVerificationCaseDetail = {
  id: 'case-draft-r13',
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
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('VF-10 (#1401) R13: Upload validation errors', () => {
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

  it('shows server user-facing sentence "ไฟล์ใหญ่เกิน 15 MB" next to upload when upload answers 400 validationFailed on field file', async () => {
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
          displayName: 'Customer R13',
          permissions: ['CustomerSelf'],
        });
      }
      if (url.includes('/api/v3/views/my_verification')) {
        return jsonResponse({ readiness: 'NONE', case: DRAFT_CASE });
      }
      if (
        url.includes('/api/app/verification/cases/case-draft-r13/documents') &&
        method === 'POST'
      ) {
        return jsonResponse(
          {
            error: {
              code: 'validationFailed',
              field: 'file',
              message: 'ไฟล์ใหญ่เกิน 15 MB',
            },
          },
          400,
        );
      }
      return jsonResponse({});
    });

    const host = mountScreen();

    await vi.waitFor(() => {
      expect(host.querySelector('input[type="file"]')).not.toBeNull();
    }, WAIT_OPTS);

    const fileInput = host.querySelector('input[type="file"]') as HTMLInputElement;
    const hugeFile = new File(['oversized-bytes'], 'oversized.jpg', {
      type: 'image/jpeg',
    });
    Object.defineProperty(fileInput, 'files', {
      value: [hugeFile],
      configurable: true,
    });
    fileInput.dispatchEvent(new Event('change', { bubbles: true }));

    const uploadBtn = host.querySelector('[data-action="upload"]') as HTMLElement;
    expect(uploadBtn).not.toBeNull();
    uploadBtn.click();

    await vi.waitFor(() => {
      const errBox = host.querySelector('[data-upload-error]');
      expect(errBox).not.toBeNull();
      const errText = errBox?.textContent ?? '';
      expect(errText).toContain('ไฟล์ใหญ่เกิน 15 MB');
      expect(errText.toLowerCase()).not.toContain('validationfailed');
    }, WAIT_OPTS);
  });

  it('shows user-facing sentence next to upload when upload answers 400 documentTypeNotAccepted, never raw code', async () => {
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
          displayName: 'Customer R13',
          permissions: ['CustomerSelf'],
        });
      }
      if (url.includes('/api/v3/views/my_verification')) {
        return jsonResponse({ readiness: 'NONE', case: DRAFT_CASE });
      }
      if (
        url.includes('/api/app/verification/cases/case-draft-r13/documents') &&
        method === 'POST'
      ) {
        return jsonResponse(
          {
            error: {
              code: 'documentTypeNotAccepted',
              field: 'documentType',
              message: 'document type not accepted',
            },
          },
          400,
        );
      }
      return jsonResponse({});
    });

    const host = mountScreen();

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

    await vi.waitFor(() => {
      const errBox = host.querySelector('[data-upload-error]');
      expect(errBox).not.toBeNull();
      const errText = errBox?.textContent ?? '';
      expect(
        errText.includes('ไม่รองรับประเภทเอกสารนี้') ||
          errText.includes('document type not accepted'),
      ).toBe(true);
      expect(errText.toLowerCase()).not.toContain('documenttypenotaccepted');
    }, WAIT_OPTS);
  });
});
