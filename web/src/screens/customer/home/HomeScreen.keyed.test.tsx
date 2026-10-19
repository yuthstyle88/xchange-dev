// KW-6a (#2103, design docs/design/xchange-v3-web.md §12.24):
// C01 HomeScreen.tsx planner-draft save idempotency key lifecycle through mocked fetch only.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import type { PlannerPreviewAnswer } from '../../../shared/api/generated/planner_preview';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import { setAudience, setLang } from '../../../shared/i18n';
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

const RATES: PublishedRatesData = {
  publication: { id: 'pub-1', version: 1, publishedAt: '2026-10-12T08:00:00Z' },
  sourceDate: '2026-10-12',
  stale: false,
  lines: [
    {
      currency: 'USD',
      group: 'ALL',
      unitSize: '1',
      buyRate: '35.12000000',
      sellRate: '35.42000000',
      lineId: 'l-usd',
      bands: [],
    },
  ],
  bandHeadings: [],
};

const PREVIEW_ANSWER: PlannerPreviewAnswer = {
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-12',
};

const DRAFT_ANSWER: PlannerDraftDto = {
  id: 'draft-kw6a-001',
  branch: 'XC-B01',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100' },
  version: 1,
  expiresAt: '2026-10-19T08:00:00Z',
  createdAt: '2026-10-12T08:00:00Z',
  updatedAt: '2026-10-12T08:00:00Z',
};

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

describe('KW-6a: C01 HomeScreen keyedWrite behaviour (#2103 §9)', () => {
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

  function clickCalculate(): void {
    const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(screenTh.calculate),
    );
    expect(calcBtn, 'calculate button').toBeDefined();
    calcBtn!.click();
  }

  async function mountAndCalculate(): Promise<void> {
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

    clickCalculate();
    await flush();
  }

  function clickSavePlan(): void {
    const saveBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(screenTh.savePlan),
    );
    expect(saveBtn, 'save plan button').toBeDefined();
    saveBtn!.click();
  }

  function draftWrites(): RecordedRequest[] {
    return sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/app/planner-drafts');
  }

  it('row 1: submit -> network TypeError -> submit again with the same input sends 2 requests with the same key and a byte-equal body', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountAndCalculate();

    clickSavePlan();
    await flush();

    clickSavePlan();
    await flush();

    const writes = draftWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('row 2: submit -> 500 -> submit again sends the same key', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(500, { error: { code: 'internalError', message: 'Internal server error' } });
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountAndCalculate();

    clickSavePlan();
    await flush();

    clickSavePlan();
    await flush();

    const writes = draftWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('row 3: submit -> 4xx refusal -> submit again sends a new key', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(409, {
          error: { code: 'plannerDraftQuotaExceeded', message: 'Planner draft limit reached' },
        });
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountAndCalculate();

    clickSavePlan();
    await flush();

    clickSavePlan();
    await flush();

    const writes = draftWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('row 4: submit -> network error -> edit the input -> submit sends a new key', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountAndCalculate();

    clickSavePlan();
    await flush();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    clickCalculate();
    await flush();

    clickSavePlan();
    await flush();

    const writes = draftWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(second!.rawBody).not.toBe(first!.rawBody);
    expect(JSON.parse(second!.rawBody!).amount).toEqual({ quantity: '250' });
  });

  it('row 5: submit -> 2xx -> submit the same input again sends a new key', async () => {
    await mountAndCalculate();

    clickSavePlan();
    await flush();

    clickSavePlan();
    await flush();

    const writes = draftWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBeTruthy();
    expect(key2).not.toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });

  it('rework: Save -> 500 -> Calculate with preview 500 -> Calculate with preview 200 -> Save with the same input reuses the same key', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(500, { error: { code: 'internalError', message: 'Internal server error' } });
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountAndCalculate();

    clickSavePlan();
    await flush();

    previewResponder = () =>
      json(500, { error: { code: 'internalError', message: 'Preview failed' } });
    clickCalculate();
    await flush();

    previewResponder = () => json(200, PREVIEW_ANSWER);
    clickCalculate();
    await flush();

    clickSavePlan();
    await flush();

    const writes = draftWrites();
    expect(writes).toHaveLength(2);
    const [first, second] = writes;
    const key1 = first!.headers.get('idempotency-key')?.trim();
    const key2 = second!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);
    expect(second!.rawBody).toBe(first!.rawBody);
  });
});
