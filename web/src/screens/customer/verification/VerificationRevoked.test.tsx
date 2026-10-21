// VerificationRevoked.test.tsx — tests for REVOKED case status (#4058).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { mount } from './main';

interface RecordedCall {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: unknown;
}

const WAIT_OPTS = { timeout: 200, interval: 20 };

let host: HTMLElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
});

function setupMockFetch(caseOverrides: Record<string, unknown> = {}) {
  const calls: RecordedCall[] = [];

  const defaultCase = {
    id: 'case-rev-1',
    status: 'REVOKED',
    policyVersion: 1,
    submittedAt: '2026-10-10T10:00:00Z',
    decidedAt: '2026-10-12T14:00:00Z',
    expiresAt: null,
    decisionReasonCode: 'Document revoked due to compliance review',
    version: 3,
    documents: [],
    ...caseOverrides,
  };

  const defaultProfile = {
    displayName: 'Customer Test',
    permissions: ['CustomerSelf', 'SignedIn'],
  };

  vi.spyOn(globalThis, 'fetch').mockImplementation(async (input, init) => {
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

    if (u.pathname.includes('/api/v3/views/my_profile')) {
      return new Response(JSON.stringify(defaultProfile), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }

    if (u.pathname.includes('/api/v3/views/my_verification')) {
      return new Response(
        JSON.stringify({
          case: defaultCase,
          readiness: 'NONE',
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }

    if (u.pathname === '/api/app/verification/cases' && method === 'POST') {
      return new Response(
        JSON.stringify({
          id: 'case-new-2',
          status: 'DRAFT',
          policyVersion: 1,
          submittedAt: null,
          decidedAt: null,
          expiresAt: null,
          decisionReasonCode: null,
          version: 1,
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }

    return new Response(JSON.stringify({}), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  });

  return { calls };
}

describe('VerificationScreen REVOKED state', () => {
  it('row 1: REVOKED case renders section[data-case-state=REVOKED] with revoked title, reason, and update-and-resubmit button', async () => {
    setupMockFetch({
      decisionReasonCode: 'Signature mismatch upon audit',
    });

    dispose = mount(host);

    await vi.waitFor(() => {
      const section = host.querySelector<HTMLElement>('section[data-case-state="REVOKED"]');
      expect(section, 'section[data-case-state=REVOKED] must be rendered').not.toBeNull();

      const title = section!.querySelector('.xc-verification-card-title');
      expect(title?.textContent).toBe(th.revokedTitle);

      const reason = section!.querySelector('[data-decision-reason="true"]');
      expect(reason?.textContent).toContain('Signature mismatch upon audit');

      const btn = section!.querySelector<HTMLButtonElement>('[data-action="update-and-resubmit"]');
      expect(btn).not.toBeNull();
    }, WAIT_OPTS);
  });

  it('row 2: REVOKED case sets step bar at 4', async () => {
    setupMockFetch();

    dispose = mount(host);

    await vi.waitFor(() => {
      const activeStep = host.querySelector<HTMLElement>('.xc-verification-step--active');
      expect(activeStep?.getAttribute('data-step')).toBe('4');
    }, WAIT_OPTS);
  });

  it('row 3: clicking update-and-resubmit in REVOKED state opens a new case via POST /api/app/verification/cases', async () => {
    const { calls } = setupMockFetch();

    dispose = mount(host);

    let btn: HTMLButtonElement | null = null;
    await vi.waitFor(() => {
      btn = host.querySelector<HTMLButtonElement>('[data-action="update-and-resubmit"]');
      expect(btn).not.toBeNull();
    }, WAIT_OPTS);

    btn!.click();

    await vi.waitFor(() => {
      const openCall = calls.find(
        (c) => c.url === '/api/app/verification/cases' && c.method === 'POST',
      );
      expect(openCall, 'must open a new case via POST /api/app/verification/cases').toBeDefined();
    }, WAIT_OPTS);
  });
});
