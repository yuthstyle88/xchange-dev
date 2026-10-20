// RW-5 (#2013): C01 HomeScreen preview and planner-draft save through ApiClient & wordedError
// Design: docs/design/xchange-v3-web.md §12.23, row RW-5.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
  makePlannerDraftDto,
  makePlannerPreviewAnswer,
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { HomeScreen } from './HomeScreen';
import { th as screenTh } from './i18n';

const BRANCHES: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: 'ชั้น G สยามพารากอน',
      openingHours: { mon: [{ open: '10:00', close: '20:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-12T08:00:00Z' }),
  sourceDate: '2026-10-12',
  stale: false,
  lines: [
    makePublishedLineDto({
      currency: 'USD',
      group: 'ALL',
      unitSize: '1',
      buyRate: '35.12000000',
      sellRate: '35.42000000',
      lineId: 'l-usd',
      bands: [],
    }),
  ],
  bandHeadings: [],
});

const PREVIEW_ANSWER = makePlannerPreviewAnswer({
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-12',
});

const DRAFT_ANSWER = makePlannerDraftDto({
  id: 'draft-rw5-001',
  branch: 'XC-B01',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100' },
  version: 1,
  expiresAt: '2026-10-19T08:00:00Z',
  createdAt: '2026-10-12T08:00:00Z',
  updatedAt: '2026-10-12T08:00:00Z',
});

interface RecordedRequest {
  method: string;
  url: string;
  path: string;
  headers: Headers;
  rawBody: string | undefined;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('RW-5 C01 HomeScreen ApiClient & wordedError (#2013 §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let previewResponder: (req: RecordedRequest) => Response | Promise<Response>;
  let draftResponder: (req: RecordedRequest) => Response | Promise<Response>;

  beforeAll(() => warmModules(() => import('./HomeScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    previewResponder = () => json(200, PREVIEW_ANSWER);
    draftResponder = () => json(200, DRAFT_ANSWER);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  const testFetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const u = new URL(input, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers);
    const rawBody = typeof init?.body === 'string' ? init.body : undefined;

    const rec: RecordedRequest = {
      method,
      url: input,
      path: u.pathname,
      headers,
      rawBody,
    };
    sentRequests.push(rec);

    if (u.pathname === '/api/public/host') {
      return json(200, { org: 'XC' });
    }
    if (u.pathname.includes('/branches') && !u.pathname.includes('/rates')) {
      return json(200, BRANCHES);
    }
    if (u.pathname.includes('/rates')) {
      return json(200, RATES);
    }
    if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
      return previewResponder(rec);
    }
    if (u.pathname === '/api/app/planner-drafts' && method === 'POST') {
      return draftResponder(rec);
    }
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  };

  async function mountScreen(): Promise<void> {
    dispose = render(
      () => (
        <HomeScreen
          fetch={testFetch}
          org="XC"
          ratesIntervalMs={60_000}
          branchesIntervalMs={600_000}
        />
      ),
      host,
    );
    await flush();
  }

  function clickCalculate(): void {
    const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(screenTh.calculate),
    );
    expect(calcBtn, 'calculate button').toBeDefined();
    calcBtn!.click();
  }

  function clickSavePlan(): void {
    const saveBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(screenTh.savePlan),
    );
    expect(saveBtn, 'save plan button').toBeDefined();
    saveBtn!.click();
  }

  it('row 1: preview and draft save — each request carries x-xchange-request: 1; the save also carries Idempotency-Key', async () => {
    await mountScreen();

    clickCalculate();
    await flush();

    clickSavePlan();
    await flush();

    const previewReq = sentRequests.find(
      (r) => r.method === 'POST' && r.path === '/api/v3/compute/planner_preview',
    );
    expect(previewReq, 'POST /api/v3/compute/planner_preview was sent').toBeDefined();
    expect(
      previewReq!.headers.get('x-xchange-request'),
      'preview sends x-xchange-request: 1',
    ).toBe('1');

    const saveReq = sentRequests.find(
      (r) => r.method === 'POST' && r.path === '/api/app/planner-drafts',
    );
    expect(saveReq, 'POST /api/app/planner-drafts was sent').toBeDefined();
    expect(saveReq!.headers.get('x-xchange-request'), 'save sends x-xchange-request: 1').toBe('1');
    expect(
      saveReq!.headers.get('idempotency-key')?.trim(),
      'save sends non-empty Idempotency-Key',
    ).toBeTruthy();
  });

  it('row 2: save retry after a network failure reuses the same key and byte-equal body', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountScreen();

    clickCalculate();
    await flush();

    // First attempt fails with network TypeError
    clickSavePlan();
    await flush();

    // Retry save plan
    clickSavePlan();
    await flush();

    const savePosts = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/app/planner-drafts',
    );
    expect(savePosts).toHaveLength(2);
    const [first, second] = savePosts;
    expect(first!.headers.get('idempotency-key')?.trim()).toBeTruthy();
    expect(second!.headers.get('idempotency-key')).toBe(first!.headers.get('idempotency-key'));
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it("row 3: save answers 500 {code:'roleNotAllowedHere'} and shows th['error.serverError']", async () => {
    draftResponder = () =>
      json(500, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Role is not allowed on this route',
        },
      });

    await mountScreen();

    clickCalculate();
    await flush();

    clickSavePlan();
    await flush();

    const alert = host.querySelector('.xc-home-estimate-card .xc-home-alert-error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(alert!.textContent).not.toContain('Role is not allowed on this route');
  });

  it('row 4: save answers 409 with an unknown code and a sentence and shows that sentence (W10 (4))', async () => {
    const sentence = 'Planner draft limit reached for today';
    draftResponder = () =>
      json(409, {
        error: {
          code: 'plannerDraftQuotaExceeded',
          message: sentence,
        },
      });

    await mountScreen();

    clickCalculate();
    await flush();

    clickSavePlan();
    await flush();

    const alert = host.querySelector('.xc-home-estimate-card .xc-home-alert-error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sentence);
    expect(alert!.textContent).not.toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain(screenTh.planSaveFailed);
  });

  it('row 5: preview network TypeError shows error.network', async () => {
    previewResponder = () => {
      throw new TypeError('Failed to fetch');
    };

    await mountScreen();

    clickCalculate();
    await flush();

    const alert = host.querySelector('.xc-home-planner-section .xc-home-alert-error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.network']);
    expect(alert!.textContent).not.toContain('Failed to fetch');
    expect(alert!.textContent).not.toContain(screenTh.networkError);
  });
});
