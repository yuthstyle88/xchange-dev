// APP-8 (C01 customer home screen, `/app`): Vitest specs
// Design: docs/design/blueprint/customer-app.md §4.5, §9.1.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
import * as table from './i18n';

import { HomeScreen, type HomeScreenProps, branchFromSearch } from './HomeScreen';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;
const EN = table.en as unknown as Texts;
const ZH = (table as unknown as { zh: Texts }).zh;

function text(key: string, t: Texts = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

// ── Mock Data ─────────────────────────────────────────────────────────────

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
      name: 'สาขาสนามบินสุวรรณภูมิ',
      address: 'ชั้น 2 อาคารผู้โดยสาร',
      openingHours: { mon: [{ open: '00:00', close: '23:59' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({
    id: 'pub-1',
    version: 1,
    publishedAt: '2026-10-12T08:00:00Z',
  }),
  sourceDate: '2026-10-12',
  stale: false,
  lines: [
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd' }),
    makePublishedLineDto({ currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '22.34500000', sellRate: '23.10000000', lineId: 'l-jpy' }),
    makePublishedLineDto({ currency: 'EUR', group: 'ALL', unitSize: '1', buyRate: '38.50000000', sellRate: '39.00000000', lineId: 'l-eur' }),
  ],
});

const PREVIEW_ANSWER = makePlannerPreviewAnswer({
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-12',
});

const DRAFT_ANSWER = makePlannerDraftDto({
  id: 'draft-999-uuid',
  branch: 'XC-B01',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100' },
  version: 1,
  expiresAt: '2026-10-19T08:00:00Z',
  createdAt: '2026-10-12T08:00:00Z',
  updatedAt: '2026-10-12T08:00:00Z',
});

// ── Mock Server ───────────────────────────────────────────────────────────

interface Call {
  method: string;
  url: string;
  body?: string;
  headers?: Headers;
}

function mockServer(overrides: {
  host?: { status: number; body?: unknown };
  branches?: { status: number; body?: unknown };
  rates?: { status: number; body?: unknown };
  preview?: { status: number; body?: unknown };
  draft?: { status: number; body?: unknown };
} = {}) {
  const calls: Call[] = [];

  const fetch = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
    const url = input;
    const method = init?.method ?? 'GET';
    const body = typeof init?.body === 'string' ? init.body : undefined;
    const headers = new Headers(init?.headers);

    calls.push({ method, url, body, headers });

    if (url === '/api/public/host') {
      const resp = overrides.host ?? { status: 200, body: { org: 'XC' } };
      return new Response(JSON.stringify(resp.body ?? {}), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url.includes('/branches') && !url.includes('/rates')) {
      const resp = overrides.branches ?? { status: 200, body: BRANCHES };
      return new Response(JSON.stringify(resp.body ?? {}), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url.includes('/rates')) {
      const resp = overrides.rates ?? { status: 200, body: RATES };
      return new Response(JSON.stringify(resp.body ?? {}), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url === '/api/v3/compute/planner_preview') {
      const resp = overrides.preview ?? { status: 200, body: PREVIEW_ANSWER };
      return new Response(JSON.stringify(resp.body ?? {}), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url === '/api/app/planner-drafts') {
      const resp = overrides.draft ?? { status: 200, body: DRAFT_ANSWER };
      return new Response(JSON.stringify(resp.body ?? {}), {
        status: resp.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    return new Response('Not Found', { status: 404 });
  });

  return { fetch, calls };
}

// ── Test Setup ────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;

function mount(props: Partial<HomeScreenProps> = {}, server = mockServer()) {
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

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
  window.localStorage.clear();
  setAudience('customer');
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
});

// ── Test Suites ───────────────────────────────────────────────────────────

describe('HomeScreen (C01 Customer Home)', () => {
  describe('Branch loading, empty, error, and retry states', () => {
    it('shows loading branches on initial load', () => {
      const { host } = mount();
      expect(host.textContent).toContain(text('loadingBranches'));
    });

    it('renders branch list and details after branches load', async () => {
      const { host } = mount();
      await settle();

      const select = host.querySelector<HTMLSelectElement>('#branch-select');
      expect(select).not.toBeNull();
      expect(select?.value).toBe('XC-B01');

      expect(host.textContent).toContain('สาขาสยามพารากอน');
      expect(host.textContent).toContain('ชั้น G สยามพารากอน');
      expect(host.textContent).toContain('10:00 - 20:00');
      expect(host.textContent).toContain(text('bookingEnabled'));
      expect(host.textContent).toContain(text('queueOpen'));
    });

    it('honors ?branch= query parameter when present in search', async () => {
      const { host } = mount({ search: '?branch=XC-B02' });
      await settle();

      const select = host.querySelector<HTMLSelectElement>('#branch-select');
      expect(select?.value).toBe('XC-B02');
      expect(host.textContent).toContain('สาขาสนามบินสุวรรณภูมิ');
      expect(host.textContent).toContain(text('bookingDisabled'));
      expect(host.textContent).toContain(text('queueClosed'));
    });

    it('displays empty state when branch list is empty', async () => {
      const s = mockServer({ branches: { status: 200, body: { branches: [] } } });
      const { host } = mount({}, s);
      await settle();

      expect(host.textContent).toContain(text('emptyBranches'));
    });

    it('displays error state and retries on failure', async () => {
      let fail = true;
      const fetch = vi.fn(async (url: string) => {
        if (url === '/api/public/host') {
          return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
        }
        if (url.includes('/branches')) {
          if (fail) {
            fail = false;
            return new Response('Server Error', { status: 500 });
          }
          return new Response(JSON.stringify(BRANCHES), { status: 200 });
        }
        return new Response(JSON.stringify(RATES), { status: 200 });
      });

      const { host } = mount({ fetch });
      await settle();

      expect(host.textContent).toContain(text('loadBranchesFailed'));
      const retryBtn = host.querySelector<HTMLButtonElement>('.xc-home-retry-btn');
      expect(retryBtn).not.toBeNull();
      expect(retryBtn?.textContent?.trim()).toBe(text('retry'));

      retryBtn?.click();
      await settle();

      expect(host.textContent).toContain('สาขาสยามพารากอน');
    });
  });

  describe('Exchange rates loading, empty, error, and retry states', () => {
    it('displays exchange rate board with currencies and rates', async () => {
      const { host } = mount();
      await settle();

      const rows = host.querySelectorAll<HTMLTableRowElement>('.xc-home-table tbody tr');
      expect(rows.length).toBe(3);

      expect(host.textContent).toContain('USD');
      expect(host.textContent).toContain('35.12');
      expect(host.textContent).toContain('35.42');

      expect(host.textContent).toContain('JPY');
      expect(host.textContent).toContain('22.345');

      expect(host.textContent).toContain('EUR');
    });

    it('displays empty state when published rates have no lines', async () => {
      const s = mockServer({
        rates: {
          status: 200,
          body: { publication: null, sourceDate: null, stale: false, lines: [] },
        },
      });
      const { host } = mount({}, s);
      await settle();

      expect(host.textContent).toContain(text('emptyRates'));
    });

    it('displays error state and retries when loading rates fails', async () => {
      let failRates = true;
      const fetch = vi.fn(async (url: string) => {
        if (url === '/api/public/host') {
          return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
        }
        if (url.includes('/branches') && !url.includes('/rates')) {
          return new Response(JSON.stringify(BRANCHES), { status: 200 });
        }
        if (url.includes('/rates')) {
          if (failRates) {
            failRates = false;
            return new Response('Rates Error', { status: 500 });
          }
          return new Response(JSON.stringify(RATES), { status: 200 });
        }
        return new Response('404', { status: 404 });
      });

      const { host } = mount({ fetch });
      await settle();

      expect(host.textContent).toContain(text('loadRatesFailed'));
      const retryBtn = host.querySelector<HTMLButtonElement>('.xc-home-rates-section .xc-home-retry-btn');
      expect(retryBtn).not.toBeNull();

      retryBtn?.click();
      await settle();

      expect(host.textContent).toContain('USD');
    });

    it('re-fetches rates when branch selection changes', async () => {
      const { host, server } = mount();
      await settle();

      const select = host.querySelector<HTMLSelectElement>('#branch-select');
      expect(select).not.toBeNull();

      select!.value = 'XC-B02';
      select!.dispatchEvent(new Event('change', { bubbles: true }));
      await settle();

      const ratesCalls = server.calls.filter((c) => c.url.includes('/rates'));
      expect(ratesCalls.some((c) => c.url.includes('XC-B02'))).toBe(true);
    });
  });

  describe('Currency Planner, preview estimate, and P5 disclaimer', () => {
    it('computes preview estimate via POST /api/v3/compute/planner_preview', async () => {
      const { host, server } = mount();
      await settle();

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      expect(calcBtn).toBeDefined();

      calcBtn?.click();
      await settle();

      const previewCall = server.calls.find((c) => c.url === '/api/v3/compute/planner_preview');
      expect(previewCall).toBeDefined();
      expect(previewCall?.method).toBe('POST');

      const parsedBody = JSON.parse(previewCall?.body ?? '{}');
      expect(parsedBody.branch).toBe('XC-B01');
      expect(parsedBody.currency).toBe('USD');
      expect(parsedBody.customerSide).toBe('CUSTOMER_BUY');
      expect(parsedBody.amount).toEqual({ quantity: '100' });

      // Check results
      expect(host.textContent).toContain(text('estimateResult'));
      expect(host.textContent).toContain('3,542.00 THB');
      expect(host.textContent).toContain('35.42000000');
    });

    it('P5 rule: an estimate is never shown as a final rate (disclaimer present)', async () => {
      const { host } = mount();
      await settle();

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      const disclaimer = host.querySelector('.xc-home-disclaimer');
      expect(disclaimer).not.toBeNull();
      expect(disclaimer?.textContent).toContain(text('disclaimer'));
    });

    it('validates amount input before computing', async () => {
      const { host, server } = mount();
      await settle();

      const input = host.querySelector<HTMLInputElement>('#amount-input');
      expect(input).not.toBeNull();

      input!.value = '-50';
      input!.dispatchEvent(new Event('input', { bubbles: true }));

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      expect(host.textContent).toContain(text('validationError'));
      const previewCall = server.calls.find((c) => c.url === '/api/v3/compute/planner_preview');
      expect(previewCall).toBeUndefined();
    });

    it('supports switching to budgetThb amount input mode', async () => {
      const { host, server } = mount();
      await settle();

      const amountTypeSelect = host.querySelector<HTMLSelectElement>('#amount-type-select');
      expect(amountTypeSelect).not.toBeNull();

      amountTypeSelect!.value = 'budgetThb';
      amountTypeSelect!.dispatchEvent(new Event('change', { bubbles: true }));

      const input = host.querySelector<HTMLInputElement>('#amount-input');
      input!.value = '5000';
      input!.dispatchEvent(new Event('input', { bubbles: true }));

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      const previewCall = server.calls.find((c) => c.url === '/api/v3/compute/planner_preview');
      expect(previewCall).toBeDefined();
      const parsedBody = JSON.parse(previewCall?.body ?? '{}');
      expect(parsedBody.amount).toEqual({ budgetThb: '5000' });
    });

    it('shows stale rate notice when preview response has stale: true', async () => {
      const s = mockServer({
        preview: {
          status: 200,
          body: {
            thb: '3542.00',
            rate: '35.42000000',
            unitSize: '1',
            stale: true,
            sourceDate: '2026-10-12',
          },
        },
      });
      const { host } = mount({}, s);
      await settle();

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      expect(host.textContent).toContain(text('estimateResult'));
      const staleEl = host.querySelector('.xc-rate-stale');
      expect(staleEl).not.toBeNull();
      expect(staleEl?.textContent?.trim()).toBeTruthy();
    });

    it('displays rate unavailable error when preview returns rateUnavailable code', async () => {
      const s = mockServer({
        preview: {
          status: 422,
          body: {
            error: {
              code: 'rateUnavailable',
              message: 'Rate not available',
            },
          },
        },
      });
      const { host } = mount({}, s);
      await settle();

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      expect(host.textContent).toContain(text('rateUnavailable'));
      expect(host.textContent).not.toContain(text('validationError'));
    });

    it('displays validation error when preview returns validationFailed code', async () => {
      const s = mockServer({
        preview: {
          status: 400,
          body: {
            error: {
              code: 'validationFailed',
              message: 'Invalid amount',
            },
          },
        },
      });
      const { host } = mount({}, s);
      await settle();

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      expect(host.textContent).toContain(text('validationError'));
      expect(host.textContent).not.toContain(text('rateUnavailable'));
    });
  });

  describe('Saving planner draft (plannerDraftSave)', () => {
    it('saves draft to POST /api/app/planner-drafts with Idempotency-Key', async () => {
      const { host, server } = mount();
      await settle();

      // First calculate
      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      // Click save plan
      const saveBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('savePlan')),
      );
      expect(saveBtn).toBeDefined();

      saveBtn?.click();
      await settle();

      const saveCall = server.calls.find((c) => c.url === '/api/app/planner-drafts');
      expect(saveCall).toBeDefined();
      expect(saveCall?.method).toBe('POST');
      expect(saveCall?.headers?.get('Idempotency-Key')).toBeTruthy();

      const body = JSON.parse(saveCall?.body ?? '{}');
      expect(body.branch).toBe('XC-B01');
      expect(body.currency).toBe('USD');
      expect(body.customerSide).toBe('CUSTOMER_BUY');
      expect(body.amount).toEqual({ quantity: '100' });

      // Shows saved confirmation
      expect(host.textContent).toContain('draft-999-uuid');
    });
  });

  describe('Language switching and MoneyText locale independence', () => {

    it('switches languages smoothly to EN and ZH', async () => {
      const { host } = mount();
      await settle();

      expect(host.textContent).toContain(text('calculatorTitle', TH));

      setLang('en');
      await settle();
      expect(host.textContent).toContain(text('calculatorTitle', EN));

      setLang('zh');
      await settle();
      expect(host.textContent).toContain(text('calculatorTitle', ZH));
    });

    it('opening hours format follows the selected language', async () => {
      const { host } = mount();
      await settle();

      // Default is Thai
      const hoursEl = host.querySelector('.xc-home-branch-hours');
      expect(hoursEl).not.toBeNull();
      expect(hoursEl?.textContent).toContain('จันทร์');

      // Switch to English
      setLang('en');
      await settle();
      expect(hoursEl?.textContent).toContain('Mon');
      expect(hoursEl?.textContent).not.toContain('จันทร์');

      // Switch to Chinese
      setLang('zh');
      await settle();
      expect(hoursEl?.textContent).toContain('周一');
    });

    it('amounts rendered with MoneyText do not change numbers with locale', async () => {
      const { host } = mount();
      await settle();

      const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
        b.textContent?.includes(text('calculate')),
      );
      calcBtn?.click();
      await settle();

      // In Thai
      const estimateTh = host.querySelector('.xc-home-estimate-value');
      expect(estimateTh?.textContent?.trim()).toBe('3,542.00 THB');

      // In English
      setLang('en');
      await settle();
      const estimateEn = host.querySelector('.xc-home-estimate-value');
      expect(estimateEn?.textContent?.trim()).toBe('3,542.00 THB');

      // In Chinese
      setLang('zh');
      await settle();
      const estimateZh = host.querySelector('.xc-home-estimate-value');
      expect(estimateZh?.textContent?.trim()).toBe('3,542.00 THB');
    });
  });

  describe('Quick Services Navigation Links', () => {
    it('renders links to all customer app services', async () => {
      const { host } = mount();
      await settle();

      const links = Array.from(host.querySelectorAll<HTMLAnchorElement>('.xc-home-service-link'));
      const hrefs = links.map((l) => l.getAttribute('href'));

      expect(hrefs).toContain('/app/book');
      expect(hrefs).toContain('/app/branches');
      expect(hrefs).toContain('/app/journey');
      expect(hrefs).toContain('/app/history');
      expect(hrefs).toContain('/app/assistant');
      expect(hrefs).toContain('/app/notifications');
    });
  });

  describe('branchFromSearch helper', () => {
    it('extracts branch from search string', () => {
      expect(branchFromSearch('?branch=XC-B01')).toBe('XC-B01');
      expect(branchFromSearch('?foo=bar&branch=XC-B02&baz=1')).toBe('XC-B02');
      expect(branchFromSearch('')).toBeNull();
      expect(branchFromSearch('?branch=  ')).toBeNull();
      expect(branchFromSearch(undefined)).toBeNull();
    });
  });
});
