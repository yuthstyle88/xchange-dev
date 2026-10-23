// CB-Home (#4833, docs/design/customer-branch-id.md rules CB-1..CB-4):
// HomeScreen keys the selected branch by id, sends id to planner preview/save,
// and calls the code-keyed public rates path with the matched entry's current code.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchDto, PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
  makePlannerDraftDto,
  makePlannerPreviewAnswer,
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { HomeScreen, type HomeScreenProps } from './HomeScreen';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;

function text(key: string): string {
  expect(TH[key], `i18n key ${key}`).toBeTypeOf('string');
  return TH[key]!;
}

const BRANCH_1: PublicBranchDto = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: '991 Rama I Rd',
  openingHours: { mon: [{ open: '10:00', close: '20:00' }] },
  timezone: 'Asia/Bangkok',
  bookingEnabled: true,
  queueOpen: true,
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

const PREVIEW = makePlannerPreviewAnswer({
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-12',
});

const DRAFT = makePlannerDraftDto({
  id: 'draft-1',
  branch: 'br-1',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100' },
  version: 1,
  expiresAt: '2026-10-19T08:00:00Z',
  createdAt: '2026-10-12T08:00:00Z',
  updatedAt: '2026-10-12T08:00:00Z',
});

const BRANCHES_INTERVAL = 600_000;
const RATES_INTERVAL = 60_000;

interface Call {
  method: string;
  url: string;
  body?: string;
}

function makeServer(initialBranches: PublicBranchDto[] = [BRANCH_1]) {
  let branches: PublicBranchDto[] = initialBranches;
  const calls: Call[] = [];

  const fetch = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const body = typeof init?.body === 'string' ? init.body : undefined;
    calls.push({ method, url, body });

    if (url === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
    }
    if (url === '/api/public/orgs/XC/branches') {
      const payload: PublicBranchesData = { branches };
      return new Response(JSON.stringify(payload), { status: 200 });
    }
    if (url.startsWith('/api/public/orgs/XC/branches/') && url.endsWith('/rates')) {
      return new Response(JSON.stringify(RATES), { status: 200 });
    }
    if (url === '/api/v3/compute/planner_preview') {
      return new Response(JSON.stringify(PREVIEW), { status: 200 });
    }
    if (url === '/api/app/planner-drafts') {
      return new Response(JSON.stringify(DRAFT), { status: 200 });
    }
    return new Response('Not Found', { status: 404 });
  });

  return {
    fetch,
    calls,
    setBranches(next: PublicBranchDto[]) {
      branches = next;
    },
  };
}

let dispose: (() => void) | undefined;

function mount(server: ReturnType<typeof makeServer>, props: Partial<HomeScreenProps> = {}) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <HomeScreen
        fetch={server.fetch}
        org="XC"
        ratesIntervalMs={RATES_INTERVAL}
        branchesIntervalMs={BRANCHES_INTERVAL}
        {...props}
      />
    ),
    host,
  );
  return host;
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

describe('HomeScreen branch id vs code (CB-Home #4833)', () => {
  it('H1: ?branch=br-1 selects br-1 and renders option text Siam (MAIN)', async () => {
    const server = makeServer();
    const host = mount(server, { search: '?branch=br-1' });
    await settle();

    const select = host.querySelector<HTMLSelectElement>('#branch-select')!;
    expect(select.value).toBe('br-1');
    const option = select.querySelector<HTMLOptionElement>('option[value="br-1"]')!;
    expect(option.textContent?.trim()).toBe('Siam (MAIN)');
  });

  it('H2: ?branch=MAIN (printed code link) resolves to select value br-1', async () => {
    const server = makeServer();
    const host = mount(server, { search: '?branch=MAIN' });
    await settle();

    const select = host.querySelector<HTMLSelectElement>('#branch-select')!;
    expect(select.value).toBe('br-1');
  });

  it('H3: with ?branch=br-1, rates are read from /branches/MAIN/rates and never /branches/br-1/rates', async () => {
    const server = makeServer();
    mount(server, { search: '?branch=br-1' });
    await settle();

    const rateUrls = server.calls.filter((c) => c.url.endsWith('/rates')).map((c) => c.url);
    expect(rateUrls).toContain('/api/public/orgs/XC/branches/MAIN/rates');
    expect(rateUrls).not.toContain('/api/public/orgs/XC/branches/br-1/rates');
  });

  it('H4: computing an estimate and saving the plan both send branch: br-1', async () => {
    const server = makeServer();
    const host = mount(server, { search: '?branch=br-1' });
    await settle();

    const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(text('calculate')),
    )!;
    calcBtn.click();
    await settle();

    const previewCall = server.calls.find((c) => c.url === '/api/v3/compute/planner_preview')!;
    expect(JSON.parse(previewCall.body ?? '{}').branch).toBe('br-1');

    const saveBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(text('savePlan')),
    )!;
    saveBtn.click();
    await settle();

    const saveCall = server.calls.find((c) => c.url === '/api/app/planner-drafts')!;
    expect(JSON.parse(saveCall.body ?? '{}').branch).toBe('br-1');
  });

  it('H5: when another branch has code br-1, ?branch=br-1 selects id br-1 (id beats code)', async () => {
    const branch2: PublicBranchDto = {
      ...BRANCH_1,
      id: 'br-2',
      code: 'br-1',
      name: 'Other',
    };
    const server = makeServer([branch2, BRANCH_1]);
    const host = mount(server, { search: '?branch=br-1' });
    await settle();

    const select = host.querySelector<HTMLSelectElement>('#branch-select')!;
    expect(select.value).toBe('br-1');
    expect(host.querySelector('.xc-home-branch-name')?.textContent).toBe('Siam');
  });

  it('H6: when the next branches poll renames code to MAIN2, select stays br-1 with Siam (MAIN2) and rates read uses /branches/MAIN2/rates', async () => {
    const server = makeServer();
    const host = mount(server, { search: '?branch=br-1' });
    await settle();

    server.setBranches([{ ...BRANCH_1, code: 'MAIN2' }]);
    await vi.advanceTimersByTimeAsync(BRANCHES_INTERVAL);
    await settle();

    const select = host.querySelector<HTMLSelectElement>('#branch-select')!;
    expect(select.value).toBe('br-1');
    const option = select.querySelector<HTMLOptionElement>('option[value="br-1"]')!;
    expect(option.textContent?.trim()).toBe('Siam (MAIN2)');

    const rateUrls = server.calls.filter((c) => c.url.endsWith('/rates')).map((c) => c.url);
    expect(rateUrls).toContain('/api/public/orgs/XC/branches/MAIN2/rates');
  });

  it('H7: when the branches poll answers the same list again, the rates poller is not restarted', async () => {
    const server = makeServer([BRANCH_1]);
    mount(server, {
      search: '?branch=br-1',
      branchesIntervalMs: 10_000,
      ratesIntervalMs: 60_000,
    });
    await settle();

    const countRates = () => server.calls.filter((c) => c.url.endsWith('/rates')).length;
    expect(countRates()).toBe(1);

    // Clone the branch object to simulate a fresh JSON response with the same id & code
    server.setBranches([{ ...BRANCH_1 }]);
    await vi.advanceTimersByTimeAsync(10_000);
    await Promise.resolve();

    const branchReads = server.calls.filter((c) => c.url === '/api/public/orgs/XC/branches').length;
    expect(branchReads).toBe(2);
    expect(countRates()).toBe(1);
  });

  it('H8 (review): ?branch=MAIN and the list read fails -> Calculate is disabled and no request carries the code', async () => {
    const server = makeServer();
    const ok = server.fetch.getMockImplementation()!;
    server.fetch.mockImplementation(async (url: string, init?: RequestInit) =>
      url === '/api/public/orgs/XC/branches' ? new Response('x', { status: 500 }) : ok(url, init));
    const host = mount(server, { search: '?branch=MAIN' });
    await settle();
    const calcBtn = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.includes(text('calculate')))!;
    expect(calcBtn.disabled).toBe(true);
    calcBtn.click();
    await settle();
    expect(server.calls.find((c) => c.url === '/api/v3/compute/planner_preview')).toBeUndefined();
  });
});
