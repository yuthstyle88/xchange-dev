// APP-12 customer branches screen tests (design customer-app.md §4.5, §9.1 C08).
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { lang, setAudience, setLang } from '../../../shared/i18n';
import { formatOpeningHours } from '../../../shared/ui/openingHours';
import { BranchesScreen } from './BranchesScreen';
import * as table from './i18n';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;
const EN = table.en as unknown as Texts;
const ZH = (table as unknown as { zh: Texts }).zh;

function text(key: string, t: Texts = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

type NodeFs = {
  readFileSync: (p: string, enc?: string) => string;
  existsSync: (p: string) => boolean;
};

let fs: NodeFs;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  const pathName = 'node:path';
  const urlName = 'node:url';
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ pathName)) as unknown as { dirname: (p: string) => string };
  const { fileURLToPath } = (await import(/* @vite-ignore */ urlName)) as unknown as {
    fileURLToPath: (u: string) => string;
  };
  dir = path.dirname(fileURLToPath(import.meta.url)) + '/';
});

type Route = 'host' | 'branches';
type Reply = { status: number; body?: unknown; etag?: string } | 'network';

interface Call {
  route: Route | 'other';
  path: string;
  ifNoneMatch: string | null;
}

const SAMPLE_BRANCHES: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: 'ชั้น G สยามพารากอน 991 ถ.พระราม 1 แขวงปทุมวัน เขตปทุมวัน กรุงเทพฯ 10330',
      openingHours: {
        mon: [{ open: '10:00', close: '20:00' }],
        tue: [{ open: '10:00', close: '20:00' }],
      },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      code: 'XC-B02',
      name: 'สาขาสนามบินดอนเมือง',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

function routeOf(path: string): Route | 'other' {
  if (path === '/api/public/host') return 'host';
  if (path === '/api/public/orgs/XC/branches') return 'branches';
  return 'other';
}

function server(over: Partial<Record<Route, Reply[]>> = {}) {
  const queue: Record<Route, Reply[]> = {
    host: [{ status: 200, body: { org: 'XC' } }],
    branches: [{ status: 200, body: SAMPLE_BRANCHES, etag: '"b1-v1"' }],
    ...over,
  };
  const calls: Call[] = [];
  const fetch = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
    const path = new URL(url, 'http://test.local').pathname;
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

const POLL_INTERVAL_MS = 600_000;
let dispose: (() => void) | undefined;

function mount(s: ReturnType<typeof server>, props?: { org?: string }) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <BranchesScreen fetch={s.fetch} org={props?.org} pollIntervalMs={POLL_INTERVAL_MS} />,
    host,
  );
  return host;
}

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 5; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function buttonWithText(host: HTMLElement, label: string): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find(
    (b) => (b.textContent ?? '').trim() === label || b.getAttribute('aria-label') === label,
  );
}

function cards(host: HTMLElement): HTMLElement[] {
  return Array.from(host.querySelectorAll<HTMLElement>('.xc-branch-card'));
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

describe('BranchesScreen (C08 / APP-12)', () => {
  it('formatOpeningHours formats hours per day in mon..sun order with missing/empty days as closed', () => {
    expect(formatOpeningHours(null)).toBeNull();
    expect(formatOpeningHours(undefined)).toBeNull();
    expect(formatOpeningHours({}, 'en')).toBe(
      'Mon: Closed, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );

    // Mon-Tue only: days absent from object (wed..sun) are formatted as closed
    setLang('en');
    expect(
      formatOpeningHours(
        {
          mon: [{ open: '09:00', close: '18:00' }],
          tue: [{ open: '09:00', close: '18:00' }],
        },
        lang(),
      ),
    ).toBe(
      'Mon: 09:00 - 18:00, Tue: 09:00 - 18:00, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );

    // Split day: multiple intervals on a single day
    expect(
      formatOpeningHours(
        {
          mon: [{ open: '09:00', close: '12:00' }, { open: '13:00', close: '18:00' }],
        },
        lang(),
      ),
    ).toBe(
      'Mon: 09:00 - 12:00, 13:00 - 18:00, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );

    // Explicit empty day array is formatted as closed
    expect(
      formatOpeningHours(
        {
          mon: [{ open: '09:00', close: '18:00' }],
          tue: [],
        },
        lang(),
      ),
    ).toBe(
      'Mon: 09:00 - 18:00, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );

    // Thai translation
    setLang('th');
    expect(
      formatOpeningHours(
        {
          mon: [{ open: '09:00', close: '18:00' }],
        },
        lang(),
      ),
    ).toBe(
      'จันทร์: 09:00 - 18:00, อังคาร: ปิดทำการ, พุธ: ปิดทำการ, พฤหัสบดี: ปิดทำการ, ศุกร์: ปิดทำการ, เสาร์: ปิดทำการ, อาทิตย์: ปิดทำการ',
    );

    // Chinese translation
    setLang('zh');
    expect(
      formatOpeningHours(
        {
          mon: [{ open: '09:00', close: '18:00' }],
        },
        lang(),
      ),
    ).toBe(
      '周一: 09:00 - 18:00, 周二: 休息, 周三: 休息, 周四: 休息, 周五: 休息, 周六: 休息, 周日: 休息',
    );
  });

  it('shows loading indicator on initial load before response arrives', () => {
    const s = server();
    const host = mount(s);
    expect(host.textContent).toContain(text('loading'));
  });

  it('resolves org from /api/public/host when not passed in props', async () => {
    const s = server();
    const host = mount(s);
    await settle();
    expect(s.calls[0]?.path).toBe('/api/public/host');
    expect(s.count('host')).toBe(1);
    expect(s.count('branches')).toBe(1);
    expect(cards(host)).toHaveLength(2);
  });

  it('uses org directly when provided in props without calling /api/public/host', async () => {
    const s = server();
    const host = mount(s, { org: 'XC' });
    await settle();
    expect(s.count('host')).toBe(0);
    expect(s.count('branches')).toBe(1);
    expect(cards(host)).toHaveLength(2);
  });

  it('a host 404 shows notLinked message and a retry button; branches route is never called', async () => {
    const s = server({ host: [{ status: 404, body: { error: 'notFound' } }] });
    const host = mount(s);
    await settle();
    expect(host.textContent).toContain(text('notLinked'));
    expect(s.count('host')).toBe(1);
    expect(s.count('branches')).toBe(0);
    expect(cards(host)).toHaveLength(0);

    const retry = buttonWithText(host, text('retry'));
    expect(retry).toBeDefined();
  });

  it('a host network failure shows notLinked / error and retry re-fetches', async () => {
    const s = server({
      host: ['network', { status: 200, body: { org: 'XC' } }],
    });
    const host = mount(s);
    await settle();
    expect(host.textContent).toContain(text('loadFailed'));
    expect(cards(host)).toHaveLength(0);

    const retry = buttonWithText(host, text('retry'));
    expect(retry).toBeDefined();
    retry!.click();
    await settle();
    expect(cards(host)).toHaveLength(2);
  });

  it('an empty branch list shows the empty state message', async () => {
    const s = server({
      branches: [{ status: 200, body: { branches: [] }, etag: '"empty"' }],
    });
    const host = mount(s);
    await settle();
    expect(host.textContent).toContain(text('empty'));
    expect(cards(host)).toHaveLength(0);
  });

  it('a failed branch list fetch shows loadFailed and a retry button; retry fetches again and displays branches', async () => {
    const s = server({
      branches: [
        { status: 500, body: {} },
        { status: 200, body: SAMPLE_BRANCHES, etag: '"b1-v1"' },
      ],
    });
    const host = mount(s);
    await settle();
    expect(host.textContent).toContain(text('loadFailed'));
    expect(cards(host)).toHaveLength(0);

    const retry = buttonWithText(host, text('retry'));
    expect(retry).toBeDefined();
    const beforeCount = s.count('branches');
    retry!.click();
    await settle();
    expect(s.count('branches')).toBe(beforeCount + 1);
    expect(cards(host)).toHaveLength(2);
    expect(host.textContent).not.toContain(text('loadFailed'));
  });

  it('a 404 from branches route shows notFound message and a retry button', async () => {
    const s = server({
      branches: [{ status: 404, body: { error: 'notFound' } }],
    });
    const host = mount(s);
    await settle();
    expect(host.textContent).toContain(text('notFound'));
    expect(cards(host)).toHaveLength(0);
    const retry = buttonWithText(host, text('retry'));
    expect(retry).toBeDefined();
  });

  it('renders all branch details, badges, and links', async () => {
    const s = server();
    const host = mount(s);
    await settle();

    const branchCards = cards(host);
    expect(branchCards).toHaveLength(2);

    // Branch 1: Siam Paragon
    const b1 = branchCards[0]!;
    expect(b1.querySelector('.xc-branch-name')?.textContent).toBe('สาขาสยามพารากอน');
    expect(b1.querySelector('.xc-branch-code')?.textContent).toBe('XC-B01');
    expect(b1.textContent).toContain(SAMPLE_BRANCHES.branches[0]!.address!);
    expect(b1.textContent).toContain('จันทร์: 10:00 - 20:00');
    expect(b1.textContent).toContain('อังคาร: 10:00 - 20:00');
    expect(b1.textContent).toContain('พุธ: ปิดทำการ');
    expect(b1.textContent).toContain('Asia/Bangkok');
    expect(b1.textContent).toContain(text('queueOpen'));
    expect(b1.textContent).toContain(text('bookingEnabled'));

    // Rates link
    const ratesLink = b1.querySelector<HTMLAnchorElement>('.xc-branch-rates-link');
    expect(ratesLink).not.toBeNull();
    expect(ratesLink?.getAttribute('href')).toBe('/rates?branch=XC-B01');
    expect(ratesLink?.textContent).toContain(text('viewRates'));

    // Booking link (present because bookingEnabled is true)
    const bookLink = b1.querySelector<HTMLAnchorElement>('.xc-branch-book-link');
    expect(bookLink).not.toBeNull();
    expect(bookLink?.getAttribute('href')).toBe('/app/book?branch=XC-B01');

    // Branch 2: Don Mueang (queue closed, booking disabled, address null, hours null)
    const b2 = branchCards[1]!;
    expect(b2.querySelector('.xc-branch-name')?.textContent).toBe('สาขาสนามบินดอนเมือง');
    expect(b2.querySelector('.xc-branch-code')?.textContent).toBe('XC-B02');
    expect(b2.textContent).toContain(text('noAddress'));
    expect(b2.textContent).toContain(text('noHours'));
    expect(b2.textContent).toContain(text('queueClosed'));
    expect(b2.textContent).toContain(text('bookingDisabled'));
    expect(b2.querySelector('.xc-branch-book-link')).toBeNull();
  });

  it('filters branch list by search query and shows noSearchResults when no match', async () => {
    const s = server();
    const host = mount(s);
    await settle();

    const searchInput = host.querySelector<HTMLInputElement>('.xc-branches-search-input');
    expect(searchInput).not.toBeNull();

    // Search by code
    searchInput!.value = 'B01';
    searchInput!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(cards(host)).toHaveLength(1);
    expect(cards(host)[0]!.textContent).toContain('XC-B01');

    // Search by name
    searchInput!.value = 'ดอนเมือง';
    searchInput!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(cards(host)).toHaveLength(1);
    expect(cards(host)[0]!.textContent).toContain('XC-B02');

    // Search by address
    searchInput!.value = 'พระราม 1';
    searchInput!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(cards(host)).toHaveLength(1);
    expect(cards(host)[0]!.textContent).toContain('XC-B01');

    // Search with no match
    searchInput!.value = 'NonexistentBranch';
    searchInput!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(cards(host)).toHaveLength(0);
    expect(host.textContent).toContain(text('noSearchResults'));

    // Clear search
    searchInput!.value = '';
    searchInput!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    expect(cards(host)).toHaveLength(2);
  });

  it('language switch TH → EN → 中文 changes headings, labels, and sets <html lang>', async () => {
    const s = server();
    const host = mount(s);
    await settle();

    const steps: [string, Parameters<typeof setLang>[0], Texts, string, string][] = [
      ['TH', 'th', TH, 'th', 'ปิดทำการ'],
      ['EN', 'en', EN, 'en', 'Closed'],
      ['中文', 'zh', ZH, 'zh-CN', '休息'],
    ];

    for (const [label, langCode, t, htmlLang, closedWord] of steps) {
      setLang(langCode);
      await settle();
      expect(document.documentElement.lang, label).toBe(htmlLang);

      const title = host.querySelector('h1')?.textContent ?? '';
      expect(title, `${label}: title`).toContain(text('title', t));

      const subtitle = host.querySelector('.xc-branches-subtitle')?.textContent ?? '';
      expect(subtitle, `${label}: subtitle`).toContain(text('subtitle', t));

      expect(host.textContent, `${label}: address label`).toContain(text('address', t));
      expect(host.textContent, `${label}: hours label`).toContain(text('hours', t));
      expect(host.textContent, `${label}: timezone label`).toContain(text('timezone', t));
      expect(host.textContent, `${label}: queueOpen badge`).toContain(text('queueOpen', t));
      expect(host.textContent, `${label}: bookingEnabled badge`).toContain(text('bookingEnabled', t));
      expect(host.textContent, `${label}: viewRates button`).toContain(text('viewRates', t));
      expect(host.textContent, `${label}: closed status`).toContain(closedWord);
    }
  });

  it('polling with If-None-Match sends ETag and a 304 keeps displayed branches', async () => {
    const s = server({
      branches: [
        { status: 200, body: SAMPLE_BRANCHES, etag: '"etag-1"' },
        { status: 304, etag: '"etag-1"' },
      ],
    });
    const host = mount(s);
    await settle();
    expect(s.calls.find((c) => c.route === 'branches')?.ifNoneMatch).toBeNull();
    expect(cards(host)).toHaveLength(2);

    // Advance time by poll interval
    await settle(POLL_INTERVAL_MS);
    expect(s.count('branches')).toBe(2);
    const secondCall = s.calls.filter((c) => c.route === 'branches')[1];
    expect(secondCall?.ifNoneMatch).toBe('"etag-1"');
    expect(cards(host)).toHaveLength(2);
  });

  it('a branch with empty openingHours {} renders as closed every day', async () => {
    const s = server({
      branches: [
        {
          status: 200,
          body: {
            branches: [
              {
                code: 'XC-B03',
                name: 'สาขาทองหล่อ',
                address: 'ทองหล่อ',
                openingHours: {},
                timezone: 'Asia/Bangkok',
                bookingEnabled: false,
                queueOpen: false,
              },
            ],
          },
          etag: '"b3-v1"',
        },
      ],
    });
    const host = mount(s);
    await settle();

    const card = cards(host)[0]!;
    expect(card.querySelector('.xc-branch-name')?.textContent).toBe('สาขาทองหล่อ');
    expect(card.textContent).toContain('จันทร์: ปิดทำการ');
    expect(card.textContent).toContain('อังคาร: ปิดทำการ');
    expect(card.textContent).toContain('พุธ: ปิดทำการ');
    expect(card.textContent).toContain('พฤหัสบดี: ปิดทำการ');
    expect(card.textContent).toContain('ศุกร์: ปิดทำการ');
    expect(card.textContent).toContain('เสาร์: ปิดทำการ');
    expect(card.textContent).toContain('อาทิตย์: ปิดทำการ');
    expect(card.textContent).not.toContain(text('noHours'));

    setLang('en');
    await settle();
    expect(card.textContent).toContain(
      'Mon: Closed, Tue: Closed, Wed: Closed, Thu: Closed, Fri: Closed, Sat: Closed, Sun: Closed',
    );
  });

  it('pages/app-branches.html is the anyone page at /app/branches that loads this screen', () => {
    const page = fs.readFileSync(dir + '../../../../pages/app-branches.html', 'utf8');
    expect(page).toContain('<meta name="xc-path" content="/app/branches">');
    expect(page).toContain('<meta name="xc-gate" content="anyone">');
    expect(page).toContain('src="/src/screens/customer/branches/main.tsx"');
  });

  it('main.tsx source sets audience customer before render and mounts CustomerShell', () => {
    const content = fs.readFileSync(dir + 'main.tsx', 'utf8');
    expect(content).toContain("setAudience('customer');");
    expect(content).toContain("CustomerShell route=\"/app/branches\"");
    expect(content.indexOf("setAudience('customer');")).toBeLessThan(
      content.indexOf('render('),
    );
  });
});
