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
// - The logo is `<img src={logoUrl}>` with `logoUrl` imported from `./logo.png`.
// Not pinned here: the poll intervals' defaults (D4 [proposed]; the tests pass 60 s / 600 s), the hidden-tab pause and
// the back-off curve (RD-2's `PublicPoller` tests), the hours' day labels (only the times are checked).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { audience, fill, locale, setAudience, setLang } from '../../../shared/i18n';
import * as table from './i18n';
import logoUrl from './logo.png';
import { RateDisplay } from './RateDisplay';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;
const EN = table.en as unknown as Texts;
const ZH = (table as unknown as { zh: Texts }).zh;

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

const RATES: PublishedRatesData = {
  publication: { id: 'pub-1', version: 7, publishedAt: '2026-10-11T02:00:00Z' },
  sourceDate: '2026-10-10',
  stale: false,
  bandHeadings: [],
  lines: [
    { currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.34500000', sellRate: '22.10000000', lineId: 'l-jpy', bands: [] },
    { currency: 'KRW', group: 'ALL', unitSize: '1000', buyRate: '25.00000000', sellRate: '26.50000000', lineId: 'l-krw', bands: [] },
    { currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] },
  ],
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

function cells(row: HTMLTableRowElement): string[] {
  return Array.from(row.querySelectorAll('td')).map((td) => (td.textContent ?? '').trim());
}

function at(publishedAt: string): string {
  return new Date(publishedAt).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
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

  it('no publication shows the empty state and no freshness line', async () => {
    const s = server({
      rates: [{ status: 200, body: { publication: null, sourceDate: null, stale: false, lines: [] }, etag: '"none"' }],
    });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(host.textContent).toContain(text('empty'));
    expect(host.querySelector('.xc-rate-freshness')).toBeNull();
    expect(rows(host)).toHaveLength(0);
  });

  it('a failed first load shows the error and a retry button; retry fetches again and the board appears', async () => {
    const s = server({ rates: [{ status: 500, body: {} }, { status: 200, body: RATES, etag: '"p1-s0"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(host.textContent).toContain(text('loadFailed'));
    expect(rows(host)).toHaveLength(0);
    const retry = buttonWithText(host, text('retry'));
    expect(retry, 'a retry button').toBeDefined();
    const before = s.count('rates');
    retry!.click();
    await settle();
    expect(s.count('rates')).toBe(before + 1);
    expect(rows(host)).toHaveLength(3);
    expect(host.textContent).not.toContain(text('loadFailed'));
  });

  it('a failed poll after a success keeps the rows and shows the banner with the last publishedAt; the next success clears it', async () => {
    const s = server({
      rates: [{ status: 200, body: RATES, etag: '"p1-s0"' }, 'network', { status: 200, body: RATES, etag: '"p1-s0"' }],
    });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(rows(host)).toHaveLength(3);
    expect(host.querySelector('[role="alert"]')).toBeNull();

    await settle(RATES_MS);
    expect(s.count('rates')).toBe(2);
    expect(rows(host), 'the rows stay').toHaveLength(3);
    const banner = host.querySelector('[role="alert"]');
    expect(banner, 'the update-failed banner').not.toBeNull();
    expect(banner!.textContent).toContain(fill(text('updateFailed'), { at: at(RATES.publication!.publishedAt) }));

    await settle(300_000);
    expect(s.count('rates')).toBeGreaterThanOrEqual(3);
    expect(host.querySelector('[role="alert"]'), 'the next success clears the banner').toBeNull();
    expect(rows(host)).toHaveLength(3);
  });

  it('a 304 poll leaves the very same row nodes, after sending If-None-Match', async () => {
    const s = server({ rates: [{ status: 200, body: RATES, etag: '"p1-s0"' }, { status: 304, etag: '"p1-s0"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    const before = rows(host);
    expect(before).toHaveLength(3);
    await settle(RATES_MS);
    const polls = s.calls.filter((c) => c.route === 'rates');
    expect(polls).toHaveLength(2);
    expect(polls[1]!.ifNoneMatch).toBe('"p1-s0"');
    const after = rows(host);
    expect(after).toHaveLength(3);
    after.forEach((row, i) => expect(row, `row ${i}`).toBe(before[i]));
  });
});

// ── §4.3 / D6: the board ────────────────────────────────────────────────────────────────────────────────

describe('the board (D6)', () => {
  it('shows the lines in the route order with the unit cells 100 JPY, 1000 KRW, 1 USD; USD buy 35.12, JPY buy 21.345', async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(rows(host)).toHaveLength(3);
    const table = rows(host).map(cells);
    expect(table.map((c) => c[0])).toEqual(['JPY', 'KRW', 'USD']);
    expect(table.map((c) => c[1])).toEqual(['100 JPY', '1000 KRW', '1 USD']);
    expect(table[2]![2]).toBe('35.12');
    expect(table[2]![3]).toBe('35.42');
    expect(table[0]![2]).toBe('21.345');
    expect(table[0]![3]).toBe('22.10');
  });

  it('shows 12345678.12345678 exactly, and a rate a float cannot hold (1234567890.12345678) too', async () => {
    // §4.3's value survives a double (16 significant digits); the second line's 17 digits do not, so a screen that
    // parses rates as numbers fails here.
    const body: PublishedRatesData = {
      ...RATES,
      lines: [
        { currency: 'BTC', group: 'ALL', unitSize: '1', buyRate: '12345678.12345678', sellRate: '99999999.99999999', lineId: 'l-btc', bands: [] },
        { currency: 'XAU', group: 'ALL', unitSize: '1', buyRate: '1234567890.12345678', sellRate: '1234567890.12345679', lineId: 'l-xau', bands: [] },
      ],
    };
    const s = server({ rates: [{ status: 200, body, etag: '"p2-s0"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(rows(host)).toHaveLength(2);
    const [btc, xau] = rows(host).map(cells);
    expect(btc![2]).toBe('12345678.12345678');
    expect(btc![3]).toBe('99999999.99999999');
    expect(xau![2]).toBe('1234567890.12345678');
    expect(xau![3]).toBe('1234567890.12345679');
  });

  it('shows a null sell rate as —', async () => {
    const body: PublishedRatesData = {
      ...RATES,
      lines: [{ currency: 'EUR', group: 'ALL', unitSize: '1', buyRate: '38.00000000', sellRate: null, lineId: 'l-eur', bands: [] }],
    };
    const s = server({ rates: [{ status: 200, body, etag: '"p3-s0"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(rows(host)).toHaveLength(1);
    const [row] = rows(host).map(cells);
    expect(row![2]).toBe('38.00');
    expect(row![3]).toBe('—');
  });

  it('gives a G100 line its own row, with G100 under the currency (owner Q-2 A)', async () => {
    const body: PublishedRatesData = {
      ...RATES,
      lines: [
        { currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] },
        { currency: 'USD', group: 'G100', unitSize: '1', buyRate: '35.20000000', sellRate: '35.40000000', lineId: 'l-usd-g100', bands: [] },
      ],
    };
    const s = server({ rates: [{ status: 200, body, etag: '"p4-s0"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    const all = rows(host);
    expect(all).toHaveLength(2);
    const first = all[0]!.querySelector('td')!;
    const second = all[1]!.querySelector('td')!;
    expect(first.textContent).toContain('USD');
    expect(first.textContent).not.toContain('G100');
    expect(second.textContent).toContain('USD');
    expect(second.textContent).toContain('G100');
    expect(cells(all[1]!)[2]).toBe('35.20');
  });
});

// ── §4.3 / D5: freshness ────────────────────────────────────────────────────────────────────────────────

describe('freshness (D5)', () => {
  it('stale: the badge with role status is visible, and the rates are still shown', async () => {
    const s = server({ rates: [{ status: 200, body: { ...RATES, stale: true }, etag: '"p1-s1"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(host.querySelector('[role="status"]')).not.toBeNull();
    expect(rows(host)).toHaveLength(3);
  });

  it('shows the source date and the published time, separately', async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    const line = host.querySelector('.xc-rate-freshness');
    expect(line, 'RateFreshness').not.toBeNull();
    expect(line!.textContent).toContain('2026-10-10');
    expect(line!.textContent).toContain(at(RATES.publication!.publishedAt));
    expect(host.querySelector('[role="status"]'), 'not stale: no badge').toBeNull();
  });
});

// ── §4.3 / D7: languages ────────────────────────────────────────────────────────────────────────────────

describe('languages (D7)', () => {
  const HEADINGS = ['weBuy', 'weSell', 'weBuyHint', 'weSellHint', 'unit', 'hours'];

  it('each heading has its own Thai, English and Chinese text', () => {
    for (const key of HEADINGS) {
      const th = text(key, TH);
      const en = text(key, EN);
      const zh = text(key, ZH);
      expect(new Set([th, en, zh]).size, key).toBe(3);
    }
  });

  it('TH → EN → 中文 changes the headings, keeps every digit, and sets <html lang>', async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    const digits = () => rows(host).map((r) => cells(r).slice(1));
    const thDigits = digits();
    const langButton = (label: string) => {
      const b = Array.from(host.querySelectorAll('button')).find((x) => (x.textContent ?? '').trim() === label);
      expect(b, `the ${label} button of LangSwitch`).toBeDefined();
      return b!;
    };

    const steps: [string, Texts, string][] = [
      ['TH', TH, 'th'],
      ['EN', EN, 'en'],
      ['中文', ZH, 'zh-CN'],
    ];
    for (const [label, t, htmlLang] of steps) {
      langButton(label).click();
      await settle();
      expect(document.documentElement.lang, label).toBe(htmlLang);
      const head = host.querySelector('table thead')?.textContent ?? '';
      expect(head, `${label}: We Buy`).toContain(text('weBuy', t));
      expect(head, `${label}: We Sell`).toContain(text('weSell', t));
      expect(head, `${label}: unit`).toContain(text('unit', t));
      expect(host.textContent, `${label}: the buy explanation`).toContain(text('weBuyHint', t));
      expect(host.textContent, `${label}: the sell explanation`).toContain(text('weSellHint', t));
      expect(host.textContent, `${label}: hours`).toContain(text('hours', t));
      expect(digits(), `${label}: every digit unchanged`).toEqual(thDigits);
    }
  });

  it('never says real time or live, in any language', async () => {
    const s = server({ rates: [{ status: 200, body: { ...RATES, stale: true }, etag: '"p1-s1"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    for (const l of ['th', 'en', 'zh'] as const) {
      setLang(l);
      await settle();
      const shown = (host.textContent ?? '') + ' ' + document.title;
      expect(shown.toLowerCase(), l).not.toMatch(/real[\s-]?time|\blive\b/);
      expect(shown, l).not.toContain('เรียลไทม์');
      expect(shown, l).not.toContain('实时');
    }
    for (const t of [TH, EN, ZH]) {
      for (const value of Object.values(t)) {
        expect(value.toLowerCase()).not.toMatch(/real[\s-]?time|\blive\b/);
        expect(value).not.toContain('เรียลไทม์');
        expect(value).not.toContain('实时');
      }
    }
  });
});

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
interface NodeCrypto {
  createHash(algorithm: 'sha256'): { update(data: Uint8Array): { digest(encoding: 'hex'): string } };
}

let fs: NodeFs;
let crypto: NodeCrypto;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  const cryptoName = 'node:crypto';
  fs = (await import(/* @vite-ignore */ fsName)) as NodeFs;
  crypto = (await import(/* @vite-ignore */ cryptoName)) as NodeCrypto;
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

  it('shows the committed logo, whose bytes are the company asset (sha256 9149f7e8…95ca)', async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    const img = host.querySelector('img');
    expect(img, 'the logo').not.toBeNull();
    expect(img!.getAttribute('src')).toBe(logoUrl);
    const sha = crypto.createHash('sha256').update(fs.readFileSync(dir + 'logo.png')).digest('hex');
    // the full hash, as web/src/screens/receipt-sample/print.test.tsx:21 pins the original
    expect(sha).toBe('9149f7e888dfab6e5e65a5c374da5debf306f047ffecff1b2a21aad4999c95ca');
  });

  it("titles the board with the branch's name from the branch list", async () => {
    const s = server();
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(s.count('branches')).toBeGreaterThanOrEqual(1);
    expect(host.querySelector('h1')?.textContent?.trim()).toBe('สาขาสยาม');
  });

  it('titles the board with the code when the branch list does not have it', async () => {
    const s = server({ branches: [{ status: 200, body: { branches: [] }, etag: '"b0"' }] });
    const host = mount('?branch=XC-B01', s);
    await settle();
    expect(host.querySelector('h1')?.textContent?.trim()).toBe('XC-B01');
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
