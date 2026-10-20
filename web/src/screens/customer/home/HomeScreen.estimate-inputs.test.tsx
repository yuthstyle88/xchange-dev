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
    {
      code: 'XC-B02',
      name: 'สาขาอโศก',
      address: 'อโศก',
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
    makePublishedLineDto({
      currency: 'EUR',
      group: 'ALL',
      unitSize: '1',
      buyRate: '38.10000000',
      sellRate: '38.50000000',
      lineId: 'l-eur',
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
  id: 'draft-2312-001',
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

describe('Customer home: clear estimate on input change and drop stale calculate (#2312)', () => {
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

  function estimateCard(): HTMLElement | null {
    return host.querySelector<HTMLElement>('.xc-home-estimate-card');
  }

  function savePlanBtn(): HTMLButtonElement | undefined {
    return Array.from(host.querySelectorAll<HTMLButtonElement>('button')).find((b) =>
      b.textContent?.includes(screenTh.savePlan),
    );
  }

  it('row 1: Calculate USD, then pick EUR -> Estimate card hidden', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    const currencySelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currencySelect.value = 'EUR';
    currencySelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    expect(estimateCard()).toBeNull();
  });

  it('row 2: Calculate, then edit the amount -> Estimate card hidden; Save plan not offered', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();
    expect(savePlanBtn()).toBeDefined();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    expect(estimateCard()).toBeNull();
    expect(savePlanBtn()).toBeUndefined();
  });

  it('row 3: An input change during a Calculate -> That answer is dropped', async () => {
    let resolvePreview!: (res: Response) => void;
    previewResponder = () =>
      new Promise<Response>((r) => {
        resolvePreview = r;
      });

    await mountScreen();
    clickCalculate();

    const currencySelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currencySelect.value = 'EUR';
    currencySelect.dispatchEvent(new Event('change', { bubbles: true }));

    resolvePreview(json(200, PREVIEW_ANSWER));
    await flush();

    expect(estimateCard()).toBeNull();
  });

  it('row 4: Calculate and show -> Estimate shown (kept), and changing branch, side, or amount type hides the estimate until recalculated', async () => {
    await mountScreen();
    clickCalculate();
    await flush();

    expect(estimateCard()).not.toBeNull();
    expect(estimateCard()!.textContent).toContain('35.42000000');

    const branchSelect = host.querySelector<HTMLSelectElement>('#branch-select')!;
    branchSelect.value = 'XC-B02';
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    const sellBtn = Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-home-toggle-btn')).find(
      (b) => b.textContent?.includes(screenTh.customerSell),
    )!;
    sellBtn.click();
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    const amountTypeSelect = host.querySelector<HTMLSelectElement>('#amount-type-select')!;
    amountTypeSelect.value = 'budgetThb';
    amountTypeSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();
  });

  it('row 5: Save, 500, recalculate, Save with the same input -> Same key (kept), while an input change clears savedPlanId and savePlanError', async () => {
    let attempts = 0;
    draftResponder = () => {
      attempts++;
      if (attempts === 1) {
        return json(500, { error: { code: 'internalError', message: 'Internal server error' } });
      }
      return json(200, DRAFT_ANSWER);
    };

    await mountScreen();
    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    expect(host.querySelector('.xc-home-alert-error')).not.toBeNull();

    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    expect(host.querySelector('.xc-home-alert-success')?.textContent).toContain('draft-2312-001');

    const writes = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/app/planner-drafts',
    );
    expect(writes).toHaveLength(2);
    const key1 = writes[0]!.headers.get('idempotency-key')?.trim();
    const key2 = writes[1]!.headers.get('idempotency-key')?.trim();
    expect(key1).toBeTruthy();
    expect(key2).toBe(key1);

    const currencySelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currencySelect.value = 'EUR';
    currencySelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();
  });

  it('edge case: Changing an input back to the same value still needs a new Calculate', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();

    amountInput.value = '100';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();
  });

  it('savedPlanId reset: Calculate -> Save OK (success alert shown) -> edit the amount -> Calculate again -> no success alert', async () => {
    await mountScreen();
    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    expect(host.querySelector('.xc-home-alert-success')?.textContent).toContain('draft-2312-001');

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();
  });

  it('savePlanError reset: Calculate -> Save answers 500 (save error shown) -> edit the amount -> Calculate again -> no save error', async () => {
    draftResponder = () =>
      json(500, { error: { code: 'internalError', message: 'Internal server error' } });

    await mountScreen();
    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    expect(host.querySelector('.xc-home-alert-error')).not.toBeNull();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();
    expect(host.querySelector('.xc-home-alert-error')).toBeNull();
  });
});
