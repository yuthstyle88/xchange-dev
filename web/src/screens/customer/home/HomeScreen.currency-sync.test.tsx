// #2539: Customer home — sync calcCurrency to the first available currency when the branch's
// rates do not include the chosen currency, and clear stale preview results.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
  type PublishedRatesData,
  makePlannerPreviewAnswer,
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { HomeScreen, type HomeScreenProps } from './HomeScreen';

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
      name: 'สาขาสนามบินสุวรรณภูมิ',
      address: 'ชั้น 2 อาคารผู้โดยสาร',
      openingHours: { mon: [{ open: '00:00', close: '23:59' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

const PREVIEW_ANSWER = makePlannerPreviewAnswer({
  thb: '3900.00',
  rate: '39.00000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-16',
});

interface Call {
  method: string;
  url: string;
  body?: Record<string, unknown>;
}

function mockServer(getRates: (branch: string) => PublishedRatesData) {
  const calls: Call[] = [];

  const fetch = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
    const url = input;
    const method = init?.method ?? 'GET';
    const body =
      typeof init?.body === 'string'
        ? (JSON.parse(init.body) as Record<string, unknown>)
        : undefined;

    calls.push({ method, url, body });

    if (url === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url.includes('/branches') && !url.includes('/rates')) {
      return new Response(JSON.stringify(BRANCHES), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url.includes('/rates')) {
      const match = url.match(/\/branches\/([^/]+)\/rates/);
      const branchCode = match ? match[1]! : 'XC-B01';
      const rates = getRates(branchCode);
      return new Response(JSON.stringify(rates), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url === '/api/v3/compute/planner_preview') {
      return new Response(JSON.stringify(PREVIEW_ANSWER), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url === '/api/app/planner-drafts') {
      return new Response(
        JSON.stringify({
          id: 'draft-1',
          branch: 'XC-B01',
          currency: body?.currency ?? 'EUR',
          customerSide: 'CUSTOMER_BUY',
          amount: { quantity: '100' },
          version: 1,
          expiresAt: '2026-10-19T08:00:00Z',
          createdAt: '2026-10-16T08:00:00Z',
          updatedAt: '2026-10-16T08:00:00Z',
        }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      );
    }

    return new Response('Not Found', { status: 404 });
  });

  return { fetch, calls };
}

let dispose: (() => void) | undefined;

function mount(props: Partial<HomeScreenProps> = {}, server: ReturnType<typeof mockServer>) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <HomeScreen
        fetch={server.fetch}
        ratesIntervalMs={60_000}
        branchesIntervalMs={600_000}
        {...props}
      />
    ),
    host,
  );
  return { host, server };
}

async function settle(ms = 50) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 10; i += 1) {
    await vi.advanceTimersByTimeAsync(50);
    await Promise.resolve();
  }
}

describe('Customer home currency sync (#2539 §8 & §9)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    setAudience('customer');
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('row 1: Rates list EUR, JPY only (no USD), user presses Calculate without touching the select -> Request sends currency: EUR, select shows EUR, and Save request sends currency: EUR', async () => {
    const ratesEurJpy = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
        makePublishedLineDto({
          currency: 'JPY',
          group: 'ALL',
          unitSize: '100',
          buyRate: '22.34500000',
          sellRate: '23.10000000',
          lineId: 'l-jpy',
          bands: [],
        }),
      ],
      bandHeadings: [],
    });

    const server = mockServer(() => ratesEurJpy);
    const { host } = mount({}, server);
    await settle();

    const currSelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    expect(currSelect).not.toBeNull();
    // Select must visibly show 'EUR' and its value must be 'EUR'
    expect(currSelect.value).toBe('EUR');

    // Click Calculate without touching currency select
    const calcBtn = host.querySelector<HTMLButtonElement>('.xc-home-calc-actions button')!;
    expect(calcBtn).not.toBeNull();
    calcBtn.click();
    await settle();

    // Verify the preview request was sent with currency: 'EUR' (not 'USD')
    const computeCalls = server.calls.filter((c) => c.url === '/api/v3/compute/planner_preview');
    expect(computeCalls).toHaveLength(1);
    expect(computeCalls[0]!.body?.currency).toBe('EUR');

    // Also verify the Save request's currency field (HomeScreen.tsx:336)
    const saveBtn = host.querySelector<HTMLButtonElement>('.xc-home-save-actions button')!;
    expect(saveBtn).not.toBeNull();
    saveBtn.click();
    await settle();

    const saveCalls = server.calls.filter((c) => c.url === '/api/app/planner-drafts');
    expect(saveCalls).toHaveLength(1);
    expect(saveCalls[0]!.body?.currency).toBe('EUR');
  });

  it('row 2: User picked JPY, branch switch gives rates EUR, USD -> calcCurrency becomes EUR; the stale JPY preview is cleared', async () => {
    const ratesB01 = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
        makePublishedLineDto({
          currency: 'JPY',
          group: 'ALL',
          unitSize: '100',
          buyRate: '22.34500000',
          sellRate: '23.10000000',
          lineId: 'l-jpy',
          bands: [],
        }),
      ],
      bandHeadings: [],
    });

    const ratesB02 = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-2', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
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

    const server = mockServer((b) => (b === 'XC-B02' ? ratesB02 : ratesB01));
    const { host } = mount({}, server);
    await settle();

    const currSelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currSelect.value = 'JPY';
    currSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // Click Calculate for JPY -> preview shown
    const calcBtn = host.querySelector<HTMLButtonElement>('.xc-home-calc-actions button')!;
    calcBtn.click();
    await settle();

    expect(host.querySelector('.xc-home-estimate-card')).not.toBeNull();

    // Switch branch to XC-B02 (which only has EUR and USD, no JPY)
    const branchSelect = host.querySelector<HTMLSelectElement>('#branch-select')!;
    branchSelect.value = 'XC-B02';
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // calcCurrency must become 'EUR' (the first available currency)
    expect(currSelect.value).toBe('EUR');
    // Stale preview must be cleared
    expect(host.querySelector('.xc-home-estimate-card')).toBeNull();

    // Calculating now sends 'EUR'
    calcBtn.click();
    await settle();

    const computeCalls = server.calls.filter((c) => c.url === '/api/v3/compute/planner_preview');
    expect(computeCalls[computeCalls.length - 1]!.body?.currency).toBe('EUR');
  });

  it('row 2 (same branch poll): Rates poll on same branch drops selected JPY -> calcCurrency becomes first remaining currency and estimate card is cleared', async () => {
    const ratesWithJpy = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
        makePublishedLineDto({
          currency: 'JPY',
          group: 'ALL',
          unitSize: '100',
          buyRate: '22.34500000',
          sellRate: '23.10000000',
          lineId: 'l-jpy',
          bands: [],
        }),
      ],
      bandHeadings: [],
    });

    const ratesWithoutJpy = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-2', version: 2, publishedAt: '2026-10-16T08:01:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
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

    let currentRates = ratesWithJpy;
    const server = mockServer(() => currentRates);
    const { host } = mount({}, server);
    await settle();

    // Pick JPY on currency-select
    const currSelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currSelect.value = 'JPY';
    currSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // Press Calculate -> estimate card shown
    const calcBtn = host.querySelector<HTMLButtonElement>('.xc-home-calc-actions button')!;
    calcBtn.click();
    await settle();

    expect(host.querySelector('.xc-home-estimate-card')).not.toBeNull();

    // Next rates poll on the SAME branch returns rates without JPY
    currentRates = ratesWithoutJpy;
    await vi.advanceTimersByTimeAsync(60_000); // ratesIntervalMs
    await settle();

    // Currency select must show the first remaining currency (EUR)
    expect(currSelect.value).toBe('EUR');
    // Estimate card must be cleared (pins the new clearPreview() at HomeScreen.tsx:260)
    expect(host.querySelector('.xc-home-estimate-card')).toBeNull();
  });

  it('row 3: Current currency still available after a rates refresh -> Unchanged, no preview cleared', async () => {
    const ratesB01 = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
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

    const ratesB02WithEur = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-2', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [
        makePublishedLineDto({
          currency: 'EUR',
          group: 'ALL',
          unitSize: '1',
          buyRate: '38.50000000',
          sellRate: '39.00000000',
          lineId: 'l-eur',
          bands: [],
        }),
        makePublishedLineDto({
          currency: 'GBP',
          group: 'ALL',
          unitSize: '1',
          buyRate: '45.00000000',
          sellRate: '46.00000000',
          lineId: 'l-gbp',
          bands: [],
        }),
      ],
      bandHeadings: [],
    });

    let currentRates = ratesB01;
    const server = mockServer(() => currentRates);
    const { host } = mount({}, server);
    await settle();

    const currSelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currSelect.value = 'EUR';
    currSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    const calcBtn = host.querySelector<HTMLButtonElement>('.xc-home-calc-actions button')!;
    calcBtn.click();
    await settle();

    expect(host.querySelector('.xc-home-estimate-card')).not.toBeNull();

    // Rates refresh: new rates publication also includes EUR
    currentRates = ratesB02WithEur;
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();

    // EUR is still available, so currency stays EUR and preview is NOT cleared
    expect(currSelect.value).toBe('EUR');
    expect(host.querySelector('.xc-home-estimate-card')).not.toBeNull();
  });

  it('edge case (§9): Rates empty (fallback list) keeps today behaviour with USD selected', async () => {
    const emptyRates = makePublishedRatesData({
      publication: makePublishedPublicationDto({ id: 'pub-empty', version: 1, publishedAt: '2026-10-16T08:00:00Z' }),
      sourceDate: '2026-10-16',
      stale: false,
      lines: [],
      bandHeadings: [],
    });

    const server = mockServer(() => emptyRates);
    const { host } = mount({}, server);
    await settle();

    const currSelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    expect(currSelect.value).toBe('USD');
  });
});
