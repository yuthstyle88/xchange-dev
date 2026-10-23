import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
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
      id: 'XC-B01',
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: 'ชั้น G สยามพารากอน',
      openingHours: { mon: [{ open: '10:00', close: '20:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      id: 'XC-B02',
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

describe('Customer home: clear calculate error on input change (#2661)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let previewResponder: (req: RecordedRequest) => Response | Promise<Response>;

  beforeAll(() => warmModules(() => import('./HomeScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    previewResponder = () =>
      json(400, {
        error: { code: 'rateUnavailable', message: 'อัตราแลกเปลี่ยนไม่พร้อมใช้งาน' },
      });
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

  function errorAlert(): HTMLElement | null {
    return host.querySelector<HTMLElement>('.xc-home-alert.xc-home-alert-error');
  }

  function estimateCard(): HTMLElement | null {
    return host.querySelector<HTMLElement>('.xc-home-estimate-card');
  }

  it('row 1: Calculate fails (rate unavailable / validation) -> Error alert shown (unchanged)', async () => {
    await mountScreen();
    clickCalculate();
    await flush();

    expect(errorAlert()).not.toBeNull();
    expect(errorAlert()!.textContent).toContain('ไม่มีอัตราแลกเปลี่ยนสำหรับเงื่อนไขนี้');
    expect(estimateCard()).toBeNull();
  });

  it('row 2: Then change currency -> Alert gone at once', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(errorAlert()).not.toBeNull();

    const currSelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currSelect.value = 'EUR';
    currSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    expect(errorAlert()).toBeNull();
  });

  it('row 3: Then switch Buy ↔ Sell -> Alert gone at once', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(errorAlert()).not.toBeNull();

    const sellBtn = Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-home-toggle-btn')).find(
      (b) => b.textContent?.includes(screenTh.customerSell),
    )!;
    sellBtn.click();
    await flush();

    expect(errorAlert()).toBeNull();
  });

  it('row 4: Then edit the amount -> Alert gone at once', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(errorAlert()).not.toBeNull();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '200';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    expect(errorAlert()).toBeNull();
  });

  it('row 5: Then change branch -> Alert gone at once', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(errorAlert()).not.toBeNull();

    const branchSelect = host.querySelector<HTMLSelectElement>('#branch-select')!;
    branchSelect.value = 'XC-B02';
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    expect(errorAlert()).toBeNull();
  });

  it('row 6: Then Calculate with valid inputs -> Estimate card shown, no error', async () => {
    await mountScreen();
    clickCalculate();
    await flush();
    expect(errorAlert()).not.toBeNull();

    // Switch previewResponder to succeed
    previewResponder = () => json(200, PREVIEW_ANSWER);

    // Edit amount to clear error and recalculate
    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '150';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(errorAlert()).toBeNull();

    clickCalculate();
    await flush();

    expect(errorAlert()).toBeNull();
    expect(estimateCard()).not.toBeNull();
  });
});
