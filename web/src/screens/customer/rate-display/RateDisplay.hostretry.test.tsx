import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { setAudience, setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { RateDisplay } from './RateDisplay';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;

function text(key: string, t: Texts = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

type Route = 'host' | 'rates' | 'branches';
type Reply = { status: number; body?: unknown; etag?: string } | 'network';

interface Call {
  at: number;
  route: Route | 'other';
  path: string;
}

const RATES: PublishedRatesData = {
  publication: { id: 'pub-1', version: 7, publishedAt: '2026-10-11T02:00:00Z' },
  sourceDate: '2026-10-10',
  stale: false,
  lines: [
    { currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.34500000', sellRate: '22.10000000', lineId: 'l-jpy', bands: [] },
    { currency: 'KRW', group: 'ALL', unitSize: '1000', buyRate: '25.00000000', sellRate: '26.50000000', lineId: 'l-krw', bands: [] },
    { currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] },
  ],
  bandHeadings: [],
};

const BRANCHES: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยาม',
      address: null,
      openingHours: { mon: [{ open: '09:00', close: '18:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

function routeOf(path: string): Route | 'other' {
  if (path === '/api/public/host') return 'host';
  if (path === '/api/public/orgs/XC/branches/XC-B01/rates') return 'rates';
  if (path === '/api/public/orgs/XC/branches') return 'branches';
  return 'other';
}

function server(over: Partial<Record<Route, Reply[]>> = {}) {
  const queue: Record<Route, Reply[]> = {
    host: [{ status: 200, body: { org: 'XC' } }],
    rates: [{ status: 200, body: RATES, etag: '"p1-s0"' }],
    branches: [{ status: 200, body: BRANCHES, etag: '"b1"' }],
    ...over,
  };
  const calls: Call[] = [];
  const fetch = vi.fn(async (url: string): Promise<Response> => {
    const path = new URL(url, 'http://board.test').pathname;
    const route = routeOf(path);
    calls.push({ at: Date.now(), route, path });
    if (route === 'other') return new Response('{}', { status: 404 });
    const replies = queue[route];
    const reply = replies.length > 1 ? replies.shift()! : replies[0]!;
    if (reply === 'network') throw new TypeError('network down');
    const headers: Record<string, string> = {};
    if (reply.etag) headers['ETag'] = reply.etag;
    if (reply.status === 304) return new Response(null, { status: 304, headers });
    headers['Content-Type'] = 'application/json';
    return new Response(JSON.stringify(reply.body ?? {}), { status: reply.status, headers });
  });
  return {
    fetch,
    calls,
    count: (r: Route | 'other') => calls.filter((c) => c.route === r).length,
    hostTimes: () => calls.filter((c) => c.route === 'host').map((c) => c.at),
  };
}

const RATES_MS = 60_000;
const BRANCHES_MS = 600_000;
let dispose: (() => void) | undefined;

function mount(search: string, s: ReturnType<typeof server>) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <RateDisplay fetch={s.fetch} search={search} ratesIntervalMs={RATES_MS} branchesIntervalMs={BRANCHES_MS} />
    ),
    host,
  );
  return host;
}

function unmount() {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
}

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 5; i += 1) {
    await vi.advanceTimersByTimeAsync(0);
  }
}

function rows(host: HTMLElement): HTMLTableRowElement[] {
  return Array.from(host.querySelectorAll<HTMLTableRowElement>('table tbody tr'));
}

beforeEach(() => {
  vi.useFakeTimers({ now: 0, toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
  window.localStorage.clear();
  setAudience('customer');
  setLang('th');
});

afterEach(() => {
  unmount();
  vi.useRealTimers();
  setLang('th');
  setAudience('staff');
});

describe('RateDisplay host retry and error states (#2191)', () => {
  it('row 1: Host 404 -> notLinked; the rates route is never called', async () => {
    const s1 = server({ host: [{ status: 404, body: { error: 'notFound' } }] });
    const host1 = mount('?branch=XC-B01', s1);
    await settle(RATES_MS * 3);
    expect(host1.textContent).toContain(text('notLinked'));
    expect(host1.textContent).not.toContain(text('loadFailed'));
    expect(s1.count('host')).toBe(1);
    expect(s1.count('rates')).toBe(0);
    expect(rows(host1)).toHaveLength(0);
    unmount();

    // After a 503 retry, a 404 transitions from loadFailed to notLinked and stops retrying
    const s2 = server({
      host: [
        { status: 503, body: { error: 'unavailable' } },
        { status: 404, body: { error: 'notFound' } },
      ],
    });
    const host2 = mount('?branch=XC-B01', s2);
    await settle();
    expect(host2.textContent).toContain(text('loadFailed'));
    expect(host2.textContent).not.toContain(text('notLinked'));

    await settle(60_000);
    expect(s2.count('host')).toBe(2);
    expect(host2.textContent).toContain(text('notLinked'));
    expect(host2.textContent).not.toContain(text('loadFailed'));
    expect(s2.count('rates')).toBe(0);

    await settle(300_000);
    expect(s2.count('host')).toBe(2);
  });

  it('row 2: Host 200 with an org -> Pollers start as today', async () => {
    const s1 = server();
    const host1 = mount('?branch=XC-B01', s1);
    await settle();
    expect(s1.count('host')).toBe(1);
    expect(s1.count('rates')).toBe(1);
    expect(s1.count('branches')).toBe(1);
    expect(rows(host1)).toHaveLength(3);
    await settle(RATES_MS * 3);
    expect(s1.count('host')).toBe(1);
    unmount();

    // And when Host 200 with an org arrives after an initial 503, pollers start and host stops retrying
    const s2 = server({
      host: [
        { status: 503, body: {} },
        { status: 200, body: { org: 'XC' } },
      ],
    });
    const host2 = mount('?branch=XC-B01', s2);
    await settle();
    expect(s2.count('rates')).toBe(0);

    await settle(60_000);
    await settle(1);
    expect(s2.count('host')).toBe(2);
    expect(s2.count('rates')).toBe(1);
    expect(s2.count('branches')).toBe(1);
    expect(rows(host2)).toHaveLength(3);

    await settle(300_000);
    expect(s2.count('host')).toBe(2);
  });

  it('row 3: Host fetch throws, or 503 -> loadFailed, never notLinked; the host is asked again after the backoff; on success the board appears', async () => {
    for (const firstFailure of ['network', { status: 503, body: { error: 'unavailable' } }] as const) {
      const s = server({
        host: [firstFailure, { status: 200, body: { org: 'XC' } }],
      });
      const host = mount('?branch=XC-B01', s);
      await settle();

      expect(s.count('host')).toBe(1);
      expect(s.count('rates')).toBe(0);
      expect(host.textContent).toContain(text('loadFailed'));
      expect(host.textContent).not.toContain(text('notLinked'));
      expect(host.querySelector('button'), 'unattended screen: no retry button on host failure').toBeNull();
      expect(rows(host)).toHaveLength(0);

      // Before 60 s backoff elapses, no second request yet
      await settle(59_000);
      expect(s.count('host')).toBe(1);

      // At 60 s, /api/public/host is asked again and succeeds -> pollers start and board appears
      await settle(1_000);
      await settle(1);
      expect(s.count('host')).toBe(2);
      expect(s.count('rates')).toBe(1);
      expect(rows(host)).toHaveLength(3);
      expect(host.textContent).not.toContain(text('loadFailed'));
      expect(host.textContent).not.toContain(text('notLinked'));

      unmount();
    }
  });

  it('row 4: Host 200 with no org -> notLinked', async () => {
    for (const body of [{}, { org: '' }, { org: null }]) {
      const s = server({ host: [{ status: 200, body }] });
      const host = mount('?branch=XC-B01', s);
      await settle(RATES_MS * 3);
      expect(host.textContent).toContain(text('notLinked'));
      expect(host.textContent).not.toContain(text('loadFailed'));
      expect(s.count('host')).toBe(1);
      expect(s.count('rates')).toBe(0);
      unmount();
    }

    // After a 503 retry, a 200 with no org transitions to notLinked and stops retrying
    const s2 = server({
      host: [
        { status: 503, body: {} },
        { status: 200, body: {} },
      ],
    });
    const host2 = mount('?branch=XC-B01', s2);
    await settle();
    expect(host2.textContent).toContain(text('loadFailed'));
    expect(host2.textContent).not.toContain(text('notLinked'));

    await settle(60_000);
    expect(s2.count('host')).toBe(2);
    expect(host2.textContent).toContain(text('notLinked'));
    expect(host2.textContent).not.toContain(text('loadFailed'));
    await settle(300_000);
    expect(s2.count('host')).toBe(2);
  });

  it('edge case 1: The page is unmounted while a retry is scheduled -> the timer is cleared', async () => {
    const s = server({
      host: [
        { status: 503, body: {} },
        { status: 200, body: { org: 'XC' } },
      ],
    });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(s.count('host')).toBe(1);
    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));

    await settle(30_000);
    unmount();

    await settle(300_000);
    expect(s.count('host')).toBe(1);
    expect(s.count('rates')).toBe(0);
  });

  it('edge case 2: Repeated 503 -> the backoff stops growing at 300 s', async () => {
    const s = server({
      host: [{ status: 503, body: {} }],
    });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(s.hostTimes()).toEqual([0]);
    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));

    // Delay 1: 60 s -> t = 60_000
    await settle(60_000);
    expect(s.hostTimes()).toEqual([0, 60_000]);

    // Delay 2: 120 s -> t = 180_000
    await settle(120_000);
    expect(s.hostTimes()).toEqual([0, 60_000, 180_000]);

    // Delay 3: 240 s -> t = 420_000
    await settle(240_000);
    expect(s.hostTimes()).toEqual([0, 60_000, 180_000, 420_000]);

    // Delay 4: capped at 300 s -> t = 720_000
    await settle(300_000);
    expect(s.hostTimes()).toEqual([0, 60_000, 180_000, 420_000, 720_000]);

    // Delay 5: stays capped at 300 s -> t = 1_020_000
    await settle(300_000);
    expect(s.hostTimes()).toEqual([0, 60_000, 180_000, 420_000, 720_000, 1_020_000]);
    expect(s.count('rates')).toBe(0);
  });
});
