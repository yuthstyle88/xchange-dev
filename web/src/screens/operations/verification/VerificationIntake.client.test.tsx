import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
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
  credentials?: RequestCredentials;
}

type IntakeOutcome =
  | { kind: 'ok'; body: unknown; status?: number }
  | { kind: 'error'; status: number; body: unknown }
  | { kind: 'network' };

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('RW-3 (#2011): VerificationScreen at-branch intake through ApiClient', () => {
  let calls: RecordedCall[] = [];
  let branchReviewAllowed = true;
  let intakeOutcome: IntakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 201 };
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    calls = [];
    branchReviewAllowed = true;
    intakeOutcome = { kind: 'ok', body: { caseId: 'case-101' }, status: 201 };
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
        credentials: init?.credentials,
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
            branchReviewAllowed,
            acceptedDocumentTypes: ['PASSPORT', 'NATIONAL_ID'],
            documentsRequired: 1,
          },
        });
      }

      if (url.pathname === '/api/v3/views/verification_queue') {
        return jsonResponse({ version: 1, changed: true, data: [] });
      }

      if (url.pathname === ROUTE_BRANCH_CASES && method === 'POST') {
        if (intakeOutcome.kind === 'network') {
          throw new TypeError('Failed to fetch');
        }
        if (intakeOutcome.kind === 'error') {
          return jsonResponse(intakeOutcome.body, intakeOutcome.status);
        }
        return jsonResponse(intakeOutcome.body, intakeOutcome.status ?? 201);
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

  async function fillAndSubmitIntake(): Promise<void> {
    const custInput = document.querySelector<HTMLInputElement>('input[name="customerId"]');
    if (custInput) {
      custInput.value = 'cust-001';
      custInput.dispatchEvent(new Event('input', { bubbles: true }));
    }

    const docSelect = document.querySelector<HTMLSelectElement>('select[name="documentType"]');
    if (docSelect) {
      docSelect.value = 'PASSPORT';
      docSelect.dispatchEvent(new Event('change', { bubbles: true }));
    }

    const fileInput = document.querySelector<HTMLInputElement>('input[name="file"]');
    if (fileInput) {
      const file = new File(['passport-bytes'], 'passport.jpg', { type: 'image/jpeg' });
      Object.defineProperty(fileInput, 'files', { value: [file], configurable: true });
      fileInput.dispatchEvent(new Event('change', { bubbles: true }));
    }

    const submitBtn = document.querySelector<HTMLElement>('[data-action="submit-branch-case"]');
    submitBtn?.click();
    await flush();
  }

  const intakeCalls = () => calls.filter((c) => c.pathname === ROUTE_BRANCH_CASES && c.method === 'POST');

  it('Row 1: an intake submit carries x-xchange-request: 1 and an Idempotency-Key', async () => {
    await mountScreen();
    await fillAndSubmitIntake();

    const matching = intakeCalls();
    expect(matching).toHaveLength(1);
    const req = matching[0]!;
    expect(req.credentials).toBe('same-origin');
    expect(req.body).toBeInstanceOf(FormData);
    expect(req.headers.get('x-xchange-request')).toBe('1');
    expect(req.headers.get('idempotency-key')).toBeTruthy();
    expect(req.headers.get('content-type')).toBeNull();
  });

  it('Row 2: 500 {code:"roleNotAllowedHere"} shows th["error.serverError"]', async () => {
    intakeOutcome = {
      kind: 'error',
      status: 500,
      body: { error: { code: 'roleNotAllowedHere', message: 'internal failure' } },
    };
    await mountScreen();
    await fillAndSubmitIntake();

    const alertEl = document.querySelector('[data-error]');
    expect(alertEl).not.toBeNull();
    expect(alertEl?.textContent).toContain(sharedTh['error.serverError']);
    expect(alertEl?.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
  });

  it('Row 3: 4xx keyed refusal shows its key text', async () => {
    intakeOutcome = {
      kind: 'error',
      status: 403,
      body: { error: { code: 'roleNotAllowedHere', message: 'raw server refusal' } },
    };
    await mountScreen();
    await fillAndSubmitIntake();

    const alertEl = document.querySelector('[data-error]');
    expect(alertEl).not.toBeNull();
    expect(alertEl?.textContent).toContain(sharedTh['error.roleNotAllowedHere']);
  });

  it('Row 4: network TypeError shows error.network', async () => {
    intakeOutcome = { kind: 'network' };
    await mountScreen();
    await fillAndSubmitIntake();

    const alertEl = document.querySelector('[data-error]');
    expect(alertEl).not.toBeNull();
    expect(alertEl?.textContent).toContain(sharedTh['error.network']);
    expect(alertEl?.textContent).not.toContain('Failed to fetch');
  });

  it('Row 5: when the at-branch form is hidden (VF-9b fail-closed), sends no intake request at all', async () => {
    branchReviewAllowed = false;
    await mountScreen();
    await fillAndSubmitIntake();

    expect(intakeCalls()).toHaveLength(0);
    expect(document.querySelector('[data-testid="branch-verification-form"]')).toBeNull();
  });

  it('400 {code:"branchReviewNotAllowed", message:"branchReviewNotAllowed"} shows the screen errBranchReviewNotAllowed sentence', async () => {
    intakeOutcome = {
      kind: 'error',
      status: 400,
      body: { error: { code: 'branchReviewNotAllowed', message: 'branchReviewNotAllowed' } },
    };
    await mountScreen();
    await fillAndSubmitIntake();

    const alertEl = document.querySelector('[data-error]');
    expect(alertEl).not.toBeNull();
    expect(alertEl?.textContent).toContain(screenTh.errBranchReviewNotAllowed);
  });
});
