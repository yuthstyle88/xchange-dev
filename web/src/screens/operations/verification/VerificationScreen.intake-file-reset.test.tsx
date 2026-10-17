import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { th as screenTh } from './i18n';
import { VerificationScreen } from './VerificationScreen';

const REVIEWER_SESSION = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'DOCUMENTS',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const ROUTE_BRANCH_CASES = '/api/operations/verification/branch-cases';

interface RecordedCall {
  method: string;
  pathname: string;
  headers: Headers;
  body: unknown;
}

type IntakeOutcome =
  | { kind: 'ok'; body: unknown; status?: number }
  | { kind: 'error'; status: number; body: unknown };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('VerificationScreen at-branch intake file reset (#2532)', () => {
  let calls: RecordedCall[] = [];
  let intakeOutcome: IntakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 200 };
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    calls = [];
    intakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 200 };
    document.body.innerHTML = '';

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);

      calls.push({
        method,
        pathname: url.pathname,
        headers,
        body: init?.body,
      });

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
        if (intakeOutcome.kind === 'error') {
          return jsonResponse(intakeOutcome.body, intakeOutcome.status);
        }
        return jsonResponse(intakeOutcome.body, intakeOutcome.status ?? 200);
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

  async function mountScreen(): Promise<void> {
    dispose?.();
    document.body.innerHTML = '';
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <VerificationScreen />, host);
    await flush();
  }

  function setCustomerId(id: string): void {
    const custInput = document.querySelector<HTMLInputElement>('input[name="customerId"]');
    expect(custInput).not.toBeNull();
    custInput!.value = id;
    custInput!.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function chooseFile(file: File): HTMLInputElement {
    const fileInput = document.querySelector<HTMLInputElement>('input[name="file"]');
    expect(fileInput).not.toBeNull();
    let selectedFiles: File[] = [file];
    Object.defineProperty(fileInput!, 'files', {
      get: () => selectedFiles,
      configurable: true,
    });
    Object.defineProperty(fileInput!, 'value', {
      get: () => (selectedFiles.length > 0 ? `C:\\fakepath\\${selectedFiles[0]!.name}` : ''),
      set: (next: string) => {
        if (next === '') {
          selectedFiles = [];
        }
      },
      configurable: true,
    });
    fileInput!.dispatchEvent(new Event('change', { bubbles: true }));
    return fileInput!;
  }

  async function clickSubmit(): Promise<void> {
    const submitBtn = document.querySelector<HTMLElement>('[data-action="submit-branch-case"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();
    await flush();
  }

  const intakeCalls = () => calls.filter((c) => c.pathname === ROUTE_BRANCH_CASES && c.method === 'POST');

  it('row 1: Submit A with file X (200), type B, Submit with no new file -> No POST; errBranchFormIncomplete is shown and input.value is empty', async () => {
    await mountScreen();

    const fileX = new File(['passport-a-bytes'], 'passport-a.jpg', { type: 'image/jpeg' });
    setCustomerId('cust-A');
    const fileInput = chooseFile(fileX);
    await clickSubmit();

    expect(intakeCalls()).toHaveLength(1);
    expect(document.querySelector('[data-testid="branch-case-success"]')).not.toBeNull();

    setCustomerId('cust-B');
    await clickSubmit();

    expect(intakeCalls()).toHaveLength(1);
    const alertEl = document.querySelector('[data-error]');
    expect(alertEl).not.toBeNull();
    expect(alertEl?.textContent).toContain(screenTh.errBranchFormIncomplete);
    expect(fileInput.value).toBe('');
  });

  it('row 2: Submit A with file X (200), type B, choose file Y, Submit -> POST carries customerId B and file Y', async () => {
    await mountScreen();

    const fileX = new File(['passport-a-bytes'], 'passport-a.jpg', { type: 'image/jpeg' });
    setCustomerId('cust-A');
    const fileInput = chooseFile(fileX);
    await clickSubmit();

    expect(intakeCalls()).toHaveLength(1);

    // Verify that after A's submit cleared the file input, submitting B before choosing file Y is blocked
    setCustomerId('cust-B');
    await clickSubmit();
    expect(intakeCalls()).toHaveLength(1);
    expect(fileInput.value).toBe('');

    // Now choose file Y and submit B
    const fileY = new File(['passport-b-bytes-longer'], 'passport-b.png', { type: 'image/png' });
    chooseFile(fileY);
    await clickSubmit();

    expect(intakeCalls()).toHaveLength(2);
    const secondForm = intakeCalls()[1]!.body as FormData;
    expect(secondForm).toBeInstanceOf(FormData);
    expect(secondForm.get('customerId')).toBe('cust-B');
    const uploadedFile = secondForm.get('file') as File;
    expect(uploadedFile).toBeInstanceOf(File);
    expect(uploadedFile.name).toBe('passport-b.png');
    expect(uploadedFile.size).toBe(fileY.size);
  });

  it('row 3: Submit A with file X fails (500) -> File X is kept so the reviewer can retry, and cleared once retry succeeds', async () => {
    intakeOutcome = {
      kind: 'error',
      status: 500,
      body: { error: { code: 'internalError', message: 'server error' } },
    };
    await mountScreen();

    const fileX = new File(['passport-a-bytes'], 'passport-a.jpg', { type: 'image/jpeg' });
    setCustomerId('cust-A');
    const fileInput = chooseFile(fileX);
    await clickSubmit();

    expect(intakeCalls()).toHaveLength(1);
    expect(document.querySelector('[data-error]')).not.toBeNull();

    // Retry without re-choosing file X: succeeds with file X
    intakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 200 };
    await clickSubmit();

    expect(intakeCalls()).toHaveLength(2);
    const retryForm = intakeCalls()[1]!.body as FormData;
    expect(retryForm.get('customerId')).toBe('cust-A');
    expect((retryForm.get('file') as File).name).toBe('passport-a.jpg');
    expect(fileInput.value).toBe('');

    // After the retry succeeds, typing B without a new file sends nothing
    setCustomerId('cust-B');
    await clickSubmit();
    expect(intakeCalls()).toHaveLength(2);
    expect(document.querySelector('[data-error]')?.textContent).toContain(
      screenTh.errBranchFormIncomplete,
    );
  });
});
