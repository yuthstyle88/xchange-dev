// R4-2 (docs/design/web-silent-failures.md §8.1, §8.2): the customer home's branch section shows its
// error only when it has no data; once a branch is chosen the rate state and its Retry come only from
// the rate poller; a failed first branch read with no branch chosen leaves the rate section empty.
// Up-front tests: rows (a)-(c) are red on main; row (d) is a guard and is green on main.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
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
  publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-12T08:00:00Z' }),
  sourceDate: '2026-10-12',
  stale: false,
  lines: [
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] }),
    makePublishedLineDto({ currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '22.34500000', sellRate: '23.10000000', lineId: 'l-jpy', bands: [] }),
    makePublishedLineDto({ currency: 'EUR', group: 'ALL', unitSize: '1', buyRate: '38.50000000', sellRate: '39.00000000', lineId: 'l-eur', bands: [] }),
  ],
  bandHeadings: [],
});

const BRANCHES_INTERVAL = 600_000;
const RATES_INTERVAL = 60_000;

/**
 * The fake server. `failBranches` / `failRates` make the next branch-list / rate reads answer 500
 * until they are switched off again; every request is counted.
 */
function scriptedServer() {
  const state = { failBranches: false, failRates: false };
  const calls = { branches: 0, rates: 0 };
  const fetch = vi.fn(async (url: string): Promise<Response> => {
    if (url === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
    }
    if (url.includes('/branches') && !url.includes('/rates')) {
      calls.branches += 1;
      if (state.failBranches) return new Response('Server Error', { status: 500 });
      return new Response(JSON.stringify(BRANCHES), { status: 200 });
    }
    if (url.includes('/rates')) {
      calls.rates += 1;
      if (state.failRates) return new Response('Rates Error', { status: 500 });
      return new Response(JSON.stringify(RATES), { status: 200 });
    }
    return new Response('Not Found', { status: 404 });
  });
  return { fetch, state, calls };
}

let dispose: (() => void) | undefined;

function mount(server: ReturnType<typeof scriptedServer>, props: Partial<HomeScreenProps> = {}) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <HomeScreen
        fetch={server.fetch}
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

const branchSection = (host: HTMLElement) =>
  host.querySelector<HTMLElement>('.xc-home-branch-section')!;
const ratesSection = (host: HTMLElement) =>
  host.querySelector<HTMLElement>('.xc-home-rates-section')!;
const rateRows = (host: HTMLElement) =>
  host.querySelectorAll<HTMLTableRowElement>('.xc-home-rates-section .xc-home-table tbody tr');

/** Loads branches and rates, then lets one later branch poll fail. */
async function loadThenFailBranchPoll(
  server: ReturnType<typeof scriptedServer>,
  host: HTMLElement,
): Promise<void> {
  await settle();
  expect(host.querySelector('#branch-select'), 'branches loaded').not.toBeNull();
  expect(rateRows(host).length, 'rates loaded').toBe(3);

  const branchReads = server.calls.branches;
  server.state.failBranches = true;
  await vi.advanceTimersByTimeAsync(BRANCHES_INTERVAL);
  await settle();
  expect(server.calls.branches, 'the later branch poll ran (and failed)').toBe(branchReads + 1);
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

describe('HomeScreen after a later poll failure (R4-2)', () => {
  it('(a) a later branch poll failure keeps the branch list and the rate board', async () => {
    const server = scriptedServer();
    const host = mount(server);
    await loadThenFailBranchPoll(server, host);

    const branches = branchSection(host);
    expect(branches.textContent, 'no branch error over a list already shown').not.toContain(
      text('loadBranchesFailed'),
    );
    expect(branches.querySelector('#branch-select'), 'the branch picker stays').not.toBeNull();
    expect(branches.textContent).toContain('สาขาสยามพารากอน');

    expect(rateRows(host).length, 'the rate board keeps its lines').toBe(3);
    expect(ratesSection(host).textContent).toContain('USD');
    expect(ratesSection(host).textContent, 'no rate error from a branch poll').not.toContain(
      text('loadRatesFailed'),
    );
  });

  it('(b) with the rate poll failing too, the rate error shows and its Retry restarts only the rate read', async () => {
    const server = scriptedServer();
    const host = mount(server);
    await settle();
    expect(rateRows(host).length, 'rates loaded').toBe(3);

    server.state.failBranches = true;
    server.state.failRates = true;
    await vi.advanceTimersByTimeAsync(BRANCHES_INTERVAL);
    await settle();

    expect(ratesSection(host).textContent, 'the rate failure is worded').toContain(
      text('loadRatesFailed'),
    );
    const retry = host.querySelector<HTMLButtonElement>('.xc-home-rates-section .xc-home-retry-btn');
    expect(retry, 'the rate error has a Retry').not.toBeNull();

    server.state.failRates = false;
    const rateReads = server.calls.rates;
    const branchReads = server.calls.branches;
    retry!.click();
    await settle();

    expect(server.calls.rates, 'Retry sends one rate read').toBe(rateReads + 1);
    expect(server.calls.branches, 'Retry sends no branch read').toBe(branchReads);
    expect(rateRows(host).length, 'after a good rate read the board is back').toBe(3);
    expect(ratesSection(host).textContent).not.toContain(text('loadRatesFailed'));

    const branches = branchSection(host);
    expect(branches.textContent, 'the branch list is still shown').toContain('สาขาสยามพารากอน');
    expect(branches.textContent).not.toContain(text('loadBranchesFailed'));
  });

  it('(c) a failed first branch read with no ?branch shows the branch error and nothing in the rate section', async () => {
    const server = scriptedServer();
    server.state.failBranches = true;
    const host = mount(server);
    await settle();

    expect(branchSection(host).textContent, 'the branch error shows').toContain(
      text('loadBranchesFailed'),
    );

    const rates = ratesSection(host);
    expect(rates.textContent, 'no endless "Loading rates…"').not.toContain(text('loadingRates'));
    expect(rates.querySelector('.xc-home-loading'), 'no rate spinner').toBeNull();
    expect(rates.textContent, 'no rate error').not.toContain(text('loadRatesFailed'));
    expect(rates.querySelector('.xc-home-error'), 'no rate error box').toBeNull();
    expect(rates.querySelector('.xc-home-retry-btn'), 'no rate Retry').toBeNull();
    expect(rates.querySelector('.xc-home-table'), 'no rate board').toBeNull();
    expect(rates.querySelector('.xc-home-empty'), 'no empty-rates line').toBeNull();
    expect(server.calls.rates, 'no branch chosen: no rate read').toBe(0);
  });

  it('(d) guard: a failed first branch read with ?branch shows the branch error, and its Retry loads the list', async () => {
    const server = scriptedServer();
    server.state.failBranches = true;
    const host = mount(server, { search: '?branch=XC-B01' });
    await settle();

    const branches = branchSection(host);
    expect(branches.textContent, 'the branch error shows').toContain(text('loadBranchesFailed'));
    const retry = branches.querySelector<HTMLButtonElement>('.xc-home-retry-btn');
    expect(retry, 'the branch error has a Retry').not.toBeNull();
    expect(retry!.textContent?.trim()).toBe(text('retry'));

    server.state.failBranches = false;
    retry!.click();
    await settle();

    expect(branchSection(host).textContent).toContain('สาขาสยามพารากอน');
    expect(branchSection(host).textContent).not.toContain(text('loadBranchesFailed'));
  });
});
