import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { setAudience, setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import * as table from './i18n';
import { RateDisplay } from './RateDisplay';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;

function text(key: string, t: Texts = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: 'pub-1', version: 7, publishedAt: '2026-10-11T02:00:00Z' }),
  sourceDate: '2026-10-10',
  stale: false,
  bandHeadings: [],
  lines: [
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] }),
  ],
});

const BRANCH_MAIN = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: null,
  openingHours: null,
  timezone: 'Asia/Bangkok',
  bookingEnabled: true,
  queueOpen: true,
};

const DEFAULT_BRANCHES: PublicBranchesData = {
  branches: [BRANCH_MAIN],
};

interface Call {
  path: string;
}

function createServer(options: {
  branches?: PublicBranchesData | Promise<PublicBranchesData>;
} = {}) {
  const calls: Call[] = [];
  const fetch = vi.fn(async (url: string): Promise<Response> => {
    const path = new URL(url, 'http://board.test').pathname;
    calls.push({ path });

    if (path === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (path === '/api/public/orgs/XC/branches') {
      const branchesData = options.branches ? await options.branches : DEFAULT_BRANCHES;
      return new Response(JSON.stringify(branchesData), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (path.startsWith('/api/public/orgs/XC/branches/') && path.endsWith('/rates')) {
      return new Response(JSON.stringify(RATES), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    return new Response('{}', { status: 404 });
  });

  return {
    fetch,
    calls,
    countRates: (code?: string) =>
      calls.filter((c) =>
        code
          ? c.path === `/api/public/orgs/XC/branches/${encodeURIComponent(code)}/rates`
          : c.path.includes('/rates'),
      ).length,
  };
}

let dispose: (() => void) | undefined;

function mount(search: string, s: ReturnType<typeof createServer>) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <RateDisplay fetch={s.fetch} search={search} ratesIntervalMs={60_000} branchesIntervalMs={600_000} />
    ),
    host,
  );
  return host;
}

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 10; i += 1) {
    await vi.advanceTimersByTimeAsync(20);
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
  vi.useRealTimers();
  setLang('th');
  setAudience('staff');
});

describe('RateDisplay branch ID resolution (CB-6, #4835)', () => {
  it('D1: ?branch=br-1 -> rates are read from /api/public/orgs/XC/branches/MAIN/rates', async () => {
    const s = createServer();
    mount('?branch=br-1', s);
    await settle();

    expect(s.countRates('MAIN')).toBe(1);
    expect(s.calls.some((c) => c.path === '/api/public/orgs/XC/branches/MAIN/rates')).toBe(true);
  });

  it('D2: ?branch=MAIN -> the same rates path /branches/MAIN/rates', async () => {
    const s = createServer();
    mount('?branch=MAIN', s);
    await settle();

    expect(s.countRates('MAIN')).toBe(1);
    expect(s.calls.some((c) => c.path === '/api/public/orgs/XC/branches/MAIN/rates')).toBe(true);
  });

  it('D3: ?branch=br-1; the branch list has not answered -> no rates request is made; loading shows', async () => {
    let resolveBranches!: (data: PublicBranchesData) => void;
    const branchesPromise = new Promise<PublicBranchesData>((resolve) => {
      resolveBranches = resolve;
    });

    const s = createServer({ branches: branchesPromise });
    const host = mount('?branch=br-1', s);

    // Let host resolve, but branches is still pending
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(0);

    expect(s.countRates()).toBe(0);
    expect(host.querySelector('.xc-rate-display-loading')).not.toBeNull();

    // Now resolve branches
    resolveBranches(DEFAULT_BRANCHES);
    await settle();
    expect(s.countRates('MAIN')).toBe(1);
  });

  it('D4: ?branch=GONE -> after the list answers, the not-found message shows and no rates request is made', async () => {
    const s = createServer();
    const host = mount('?branch=GONE', s);
    await settle();

    expect(s.countRates()).toBe(0);
    expect(host.textContent).toContain(text('notFound'));
  });

  it('D5: ?branch=br-1 and ?branch=MAIN -> the booking QR data-qr-url uses code MAIN', async () => {
    // 1. With ID ?branch=br-1
    const s1 = createServer();
    const host1 = mount('?branch=br-1', s1);
    await settle();
    const qr1 = host1.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qr1).not.toBeNull();
    expect(qr1?.getAttribute('data-qr-url')).toBe(`${window.location.origin}/app/queue?branch=MAIN`);
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';

    // 2. With code ?branch=MAIN
    const s2 = createServer();
    const host2 = mount('?branch=MAIN', s2);
    await settle();
    const qr2 = host2.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qr2).not.toBeNull();
    expect(qr2?.getAttribute('data-qr-url')).toBe(`${window.location.origin}/app/queue?branch=MAIN`);
  });

  it('D6: the list also holds { id: "br-2", code: "br-1" } -> rates are read from /branches/MAIN/rates', async () => {
    const branchesWithShadow: PublicBranchesData = {
      branches: [
        BRANCH_MAIN,
        {
          id: 'br-2',
          code: 'br-1',
          name: 'Other',
          address: null,
          openingHours: null,
          timezone: 'Asia/Bangkok',
          bookingEnabled: true,
          queueOpen: true,
        },
      ],
    };

    const s = createServer({ branches: branchesWithShadow });
    mount('?branch=br-1', s);
    await settle();

    // Resolves id 'br-1' first (branch MAIN), not code 'br-1' (branch br-2)
    expect(s.countRates('MAIN')).toBe(1);
    expect(s.countRates('br-1')).toBe(0);
  });
});
