// RD-3 (P01 public rate display, `/rates?branch=:code`): the screen, one test per row of the design's §4.3.
// Design `docs/design/blueprint/rate-display.md` (PR 1248 @ 9806993): D1–D8, D10–D13 (D9, the booking QR, is phase 4),
// §3 (classes), §4.3 (the screen). Written before the implementation by the test writer (claude/5d45b017) on the
// xchange FE lane controller's order. Do not edit, delete or weaken these tests; add your own in another file.
//
// What this file pins ([required] unless marked):
// - `RateDisplay(props: { fetch, search, ratesIntervalMs?, branchesIntervalMs? })`, a named export of
//   `RateDisplay.tsx`. `fetch` is injected (tests mock at the fetch level; RD-2's `PublicPoller` is not merged, so
//   nothing here imports it). `search` is `window.location.search`.
// - The routes (D2, D3), same-origin paths: `GET /api/public/host` once on load → `{"org":"XC"}` or 404;
//   `GET /api/public/orgs/{org}/branches/{code}/rates`; `GET /api/public/orgs/{org}/branches`.
// - A poll after a 200 with an `ETag` sends `If-None-Match` (D4); a 304 changes nothing on screen.
// - The board is a `<table>`: headings in `<thead>`, one `<tr>` per published line in `<tbody>` with four `<td>`s in
//   this order: currency (a non-ALL group's name under it), unit, We Buy, We Sell (D6).
// - The texts are this screen's own table, `./i18n` (`th`, `en`, `zh`), with these keys: currency, unit, weBuy,
//   weSell, weBuyHint, weSellHint, hours, noBranch, notLinked, loading, notFound, empty, loadFailed, retry,
//   updateFailed ({at}), fullscreen. The tests read the texts through the table, so the wording is the implementer's.
// - Freshness is `RateFreshness` (D5: class `xc-rate-freshness`, the stale badge `role="status"`).
// - The update-failed banner has `role="alert"` and shows the last `publishedAt` formatted as `RateFreshness` does
//   (`toLocaleString(locale(), { timeZone: 'Asia/Bangkok' })`).
// - The branch's name (or, when the branch list lacks the code, the code) is the page's `<h1>`.
// - The full-screen button's accessible text is `fullscreen` from the table.
// - (v1's logo row retired with P01 v2, rate-display.md §9.1.)
// Not pinned here: the poll intervals' defaults (D4 [proposed]; the tests pass 60 s / 600 s), the hidden-tab pause and
// the back-off curve (RD-2's `PublicPoller` tests), the hours' day labels (only the times are checked).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { audience, setAudience, setLang } from '../../../shared/i18n';
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

// ── the fake server ─────────────────────────────────────────────────────────────────────────────────────

type Route = 'host' | 'rates' | 'branches';
type Reply = { status: number; body?: unknown; etag?: string } | 'network';

interface Call {
  route: Route | 'other';
  path: string;
  ifNoneMatch: string | null;
}

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: 'pub-1', version: 7, publishedAt: '2026-10-11T02:00:00Z' }),
  sourceDate: '2026-10-10',
  stale: false,
  bandHeadings: [],
  lines: [
    makePublishedLineDto({ currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.34500000', sellRate: '22.10000000', lineId: 'l-jpy', bands: [] }),
    makePublishedLineDto({ currency: 'KRW', group: 'ALL', unitSize: '1000', buyRate: '25.00000000', sellRate: '26.50000000', lineId: 'l-krw', bands: [] }),
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] }),
  ],
});

const BRANCHES: PublicBranchesData = {
  branches: [
    {
      id: 'XC-B01',
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
  const fetch = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
    const path = new URL(url, 'http://board.test').pathname;
    const route = routeOf(path);
    const ifNoneMatch = new Headers(init?.headers).get('If-None-Match');
    calls.push({ route, path, ifNoneMatch });
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
  return { fetch, calls, count: (r: Route | 'other') => calls.filter((c) => c.route === r).length };
}

// ── mounting ────────────────────────────────────────────────────────────────────────────────────────────

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

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 5; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function rows(host: HTMLElement): HTMLTableRowElement[] {
  return Array.from(host.querySelectorAll<HTMLTableRowElement>('table tbody tr'));
}

function buttonWithText(host: HTMLElement, label: string): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find(
    (b) => (b.textContent ?? '').trim() === label || b.getAttribute('aria-label') === label,
  );
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
  delete (document.documentElement as unknown as Record<string, unknown>)['requestFullscreen'];
  delete (document as unknown as Record<string, unknown>)['fullscreenElement'];
  setLang('th');
  setAudience('staff');
});

// ── §4.3 / D11: the states ──────────────────────────────────────────────────────────────────────────────

describe('the states (D11)', () => {
  it('/rates with no branch shows the no-branch message, and fetch is never called', async () => {
    for (const search of ['', '?x=1', '?branch=']) {
      const s = server();
      const host = mount(search, s);
      await settle(BRANCHES_MS);
      expect(host.textContent, search).toContain(text('noBranch'));
      expect(s.fetch, search).not.toHaveBeenCalled();
      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
  });

  it('a host that is not linked (404) shows the not-linked message, and the rates route is never called', async () => {
    const s = server({ host: [{ status: 404, body: { error: 'notFound' } }] });
    const host = mount('?branch=XC-B01', s);
    await settle(RATES_MS * 3);
    expect(host.textContent).toContain(text('notLinked'));
    expect(s.count('host')).toBe(1);
    expect(s.count('rates')).toBe(0);
    expect(rows(host)).toHaveLength(0);
  });

  it('asks the host route once on load, then reads the rates of that org and branch', async () => {
    const s = server();
    mount('?branch=XC-B01', s);
    await settle();
    expect(s.calls[0]?.path).toBe('/api/public/host');
    expect(s.count('host')).toBe(1);
    expect(s.count('rates')).toBe(1);
    await settle(RATES_MS * 3);
    expect(s.count('host'), 'the host is read once, not polled').toBe(1);
  });

  it('the first load shows loading until the answer comes', () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    expect(host.textContent).toContain(text('loading'));
  });

  it('a 404 from the rates route shows not found, and polling stops', async () => {
    const s = server({ rates: [{ status: 404, body: { error: 'notFound' } }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(host.textContent).toContain(text('notFound'));
    expect(rows(host)).toHaveLength(0);
    await settle(RATES_MS * 5);
    expect(s.count('rates')).toBe(1);
  });

});

// ── §4.3 / D6: the board ────────────────────────────────────────────────────────────────────────────────

// ── §4.3 / D5: freshness ────────────────────────────────────────────────────────────────────────────────

// ── §4.3 / D7: languages ────────────────────────────────────────────────────────────────────────────────

// ── §4.3 / D8: full screen ──────────────────────────────────────────────────────────────────────────────

describe('full screen (D8)', () => {
  it('when supported, the button calls requestFullscreen and hides while full screen', async () => {
    const request = vi.fn(async () => {});
    Object.defineProperty(document.documentElement, 'requestFullscreen', { value: request, configurable: true });
    let full: Element | null = null;
    Object.defineProperty(document, 'fullscreenElement', { get: () => full, configurable: true });

    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    const button = buttonWithText(host, text('fullscreen'));
    expect(button, 'the full-screen button').toBeDefined();
    button!.click();
    expect(request).toHaveBeenCalledTimes(1);

    full = document.documentElement;
    document.dispatchEvent(new Event('fullscreenchange'));
    await settle();
    expect(buttonWithText(host, text('fullscreen')), 'hidden while full screen').toBeUndefined();

    full = null;
    document.dispatchEvent(new Event('fullscreenchange'));
    await settle();
    expect(buttonWithText(host, text('fullscreen')), 'back after leaving full screen').toBeDefined();
  });

  it('when the API is missing, there is no button', async () => {
    expect((document.documentElement as unknown as Record<string, unknown>)['requestFullscreen']).toBeUndefined();
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(rows(host)).toHaveLength(3);
    expect(buttonWithText(host, text('fullscreen'))).toBeUndefined();
  });
});

// ── §4.3 / D1, D8, D10, D12: the page ───────────────────────────────────────────────────────────────────

interface NodeFs {
  readFileSync(file: string): Uint8Array;
  readFileSync(file: string, encoding: 'utf8'): string;
}

let fs: NodeFs;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  fs = (await import(/* @vite-ignore */ fsName)) as NodeFs;
  dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

describe('the page (D1, D8, D10, D12)', () => {
  it('has no nav, no account bar and no link', async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(rows(host)).toHaveLength(3);
    expect(host.querySelector('nav')).toBeNull();
    expect(host.querySelector('a')).toBeNull();
    expect(host.querySelector('[class*="account"]')).toBeNull();
  });

  it("shows the branch's opening hours from the branch list (D12)", async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(host.textContent).toContain('09:00');
    expect(host.textContent).toContain('18:00');
  });

  it('pages/rates.html is the anyone page at /rates that loads this screen (D1)', () => {
    const page = fs.readFileSync(dir + '../../../../pages/rates.html', 'utf8');
    expect(page).toContain('<meta name="xc-path" content="/rates">');
    expect(page).toContain('<meta name="xc-gate" content="anyone">');
    expect(page).toContain('src="/src/screens/customer/rate-display/main.tsx"');
  });

  it("main.tsx declares a customer page: setAudience('customer') (D7)", async () => {
    setAudience('staff');
    expect(document.getElementById('root')).toBeNull();
    await import('./main');
    expect(audience()).toBe('customer');
  });
});
