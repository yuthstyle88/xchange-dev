import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { mount } from './main';

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

interface MyVerificationCaseDetail extends VerificationCaseView {
  documents: VerificationDocumentView[];
}

interface MyVerificationView {
  case: MyVerificationCaseDetail | null;
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
  profileData?: { displayName: string; permissions: string[] };
}

const WAIT_OPTS = { timeout: 200, interval: 20 };

function setupMockFetch(options: MockBackendOptions = {}) {
  const calls: RecordedCall[] = [];

  const defaultView: MyVerificationView = {
    case: null,
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

    if (u.pathname.includes('/api/v3/views/my_profile')) {
      return new Response(
        JSON.stringify(options.profileData ?? defaultProfile),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }

    if (u.pathname.includes('/api/v3/views/my_verification')) {
      return new Response(
        JSON.stringify(options.viewData ?? defaultView),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }

    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  };

  vi.stubGlobal('fetch', fetchHandler);
  return { calls };
}

function makeCase(status: VerificationCaseView['status'], expiresAt: string | null) {
  return {
    id: 'case-test-1',
    status,
    policyVersion: 1,
    submittedAt: '2026-10-10T10:00:00Z',
    decidedAt: '2026-10-10T11:00:00Z',
    expiresAt,
    decisionReasonCode: status === 'EXPIRED' ? 'Expired' : null,
    version: 1,
    documents: [],
  };
}

describe('VerificationScreen expiresAt formatting (#4892)', () => {
  let host: HTMLDivElement;
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
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  // Row 1: VERIFIED, 2027-10-23T16:59:59.… -> data-expires-at="2027-10-23" ; text ใช้ได้ถึง: 2027-10-23
  it('row 1: VERIFIED with timestamp -> data-expires-at="2027-10-23" and formatted text in th', async () => {
    setupMockFetch({
      viewData: {
        case: makeCase('VERIFIED', '2027-10-23T16:59:59.999999+00:00'),
        readiness: 'VERIFIED',
      },
    });
    dispose = mount(host);
    await vi.waitFor(() => {
      const el = host.querySelector('[data-expires-at]');
      expect(el).not.toBeNull();
      expect(el?.getAttribute('data-expires-at')).toBe('2027-10-23');
      expect(el?.textContent).toBe('ใช้ได้ถึง: 2027-10-23');
    }, WAIT_OPTS);
  });

  // Row 2: row 1 in en -> text Valid until: 2027-10-23
  it('row 2: row 1 in en -> text Valid until: 2027-10-23', async () => {
    setLang('en');
    setupMockFetch({
      viewData: {
        case: makeCase('VERIFIED', '2027-10-23T16:59:59.999999+00:00'),
        readiness: 'VERIFIED',
      },
    });
    dispose = mount(host);
    await vi.waitFor(() => {
      const el = host.querySelector('[data-expires-at]');
      expect(el).not.toBeNull();
      expect(el?.getAttribute('data-expires-at')).toBe('2027-10-23');
      expect(el?.textContent).toBe('Valid until: 2027-10-23');
    }, WAIT_OPTS);
  });

  // Row 3: VERIFIED, 2027-10-23T17:00:00+… -> 2027-10-24 (Bangkok is UTC+7)
  it('row 3: VERIFIED with 2027-10-23T17:00:00+00:00 -> 2027-10-24 (Bangkok is UTC+7)', async () => {
    setupMockFetch({
      viewData: {
        case: makeCase('VERIFIED', '2027-10-23T17:00:00+00:00'),
        readiness: 'VERIFIED',
      },
    });
    dispose = mount(host);
    await vi.waitFor(() => {
      const el = host.querySelector('[data-expires-at]');
      expect(el).not.toBeNull();
      expect(el?.getAttribute('data-expires-at')).toBe('2027-10-24');
      expect(el?.textContent).toBe('ใช้ได้ถึง: 2027-10-24');
    }, WAIT_OPTS);
  });

  // Row 4: VERIFIED, 2027-10-23 -> 2027-10-23
  it('row 4: VERIFIED with date-only 2027-10-23 -> 2027-10-23', async () => {
    setupMockFetch({
      viewData: {
        case: makeCase('VERIFIED', '2027-10-23'),
        readiness: 'VERIFIED',
      },
    });
    dispose = mount(host);
    await vi.waitFor(() => {
      const el = host.querySelector('[data-expires-at]');
      expect(el).not.toBeNull();
      expect(el?.getAttribute('data-expires-at')).toBe('2027-10-23');
      expect(el?.textContent).toBe('ใช้ได้ถึง: 2027-10-23');
    }, WAIT_OPTS);
  });

  // Row 5: EXPIRED (the :659 card), 2027-10-23T16:59:59.999999+00:00 -> data-expires-at="2027-10-23"
  it('row 5: EXPIRED card with timestamp -> data-expires-at="2027-10-23"', async () => {
    setupMockFetch({
      viewData: {
        case: makeCase('EXPIRED', '2027-10-23T16:59:59.999999+00:00'),
        readiness: 'EXPIRED',
      },
    });
    dispose = mount(host);
    await vi.waitFor(() => {
      const card = host.querySelector('[data-case-state="EXPIRED"]');
      expect(card).not.toBeNull();
      const el = card?.querySelector('[data-expires-at]');
      expect(el).not.toBeNull();
      expect(el?.getAttribute('data-expires-at')).toBe('2027-10-23');
      expect(el?.textContent).toBe('ใช้ได้ถึง: 2027-10-23');
    }, WAIT_OPTS);
  });

  // Row 6: expiresAt: null -> no [data-expires-at], as today
  it('row 6: expiresAt: null -> no [data-expires-at], as today', async () => {
    setupMockFetch({
      viewData: {
        case: makeCase('VERIFIED', null),
        readiness: 'VERIFIED',
      },
    });
    dispose = mount(host);
    await vi.waitFor(() => {
      expect(host.querySelector('[data-case-state="VERIFIED"]')).not.toBeNull();
    }, WAIT_OPTS);
    expect(host.querySelector('[data-expires-at]')).toBeNull();
  });
});
