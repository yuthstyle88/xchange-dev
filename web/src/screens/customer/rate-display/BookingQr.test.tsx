// Written before the implementation by the test writer gemini/warut-4 (ticket author claude/e92db6), against design D9a (PR 1656). Do not edit, delete or weaken.
//
// Contract for RD-4 (issue #1374, design D9a in PR 1656; updated to /app/queue per §0a R-3):
// The implementer must keep the following DOM hooks:
// - `[data-booking-qr]`: the container element for the booking QR code on the rate board.
// - `data-qr-url`: attribute on `[data-booking-qr]` holding the exact target URL:
//   `${location.origin}/app/queue?branch=${branchCode}`.
// - `<svg>`: inline SVG element drawn inside `[data-booking-qr]` client-side via shared
//   `QrCode` component wrapping `qrcode-generator`.
// - Text caption: visible caption displaying the path `/app/queue?branch=${branchCode}`.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { setAudience, setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { warmModules } from '../../../shared/testing/warm-modules';

// ── qrcode-generator mock ───────────────────────────────────────────────────────────────────────────────

const { mockAddData, mockMake, mockCreateSvgTag, mockFactory } = vi.hoisted(() => {
  const mockAddData = vi.fn();
  const mockMake = vi.fn();
  const mockCreateSvgTag = vi.fn(
    (_opts?: unknown) =>
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100"/></svg>',
  );
  const mockFactory = vi.fn((_typeNumber?: number, _errorCorrectionLevel?: string) => ({
    addData: mockAddData,
    make: mockMake,
    createSvgTag: mockCreateSvgTag,
  }));
  return { mockAddData, mockMake, mockCreateSvgTag, mockFactory };
});

vi.mock('qrcode-generator', () => ({
  default: mockFactory,
  qrcode: mockFactory,
  __esModule: true,
}));

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
  lines: [
    makePublishedLineDto({ currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.34500000', sellRate: '22.10000000', lineId: 'l-jpy', bands: [] }),
    makePublishedLineDto({ currency: 'KRW', group: 'ALL', unitSize: '1000', buyRate: '25.00000000', sellRate: '26.50000000', lineId: 'l-krw', bands: [] }),
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '35.12000000', sellRate: '35.42000000', lineId: 'l-usd', bands: [] }),
  ],
  bandHeadings: [],
});

const BRANCHES_ENABLED: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยาม',
      address: null,
      openingHours: { mon: [{ open: '09:00', close: '18:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      code: 'XC-B02',
      name: 'สาขาชิดลม',
      address: null,
      openingHours: { mon: [{ open: '10:00', close: '20:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: true,
    },
  ],
};

const BRANCHES_DISABLED: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยาม',
      address: null,
      openingHours: { mon: [{ open: '09:00', close: '18:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: true,
    },
  ],
};

function routeOf(path: string): Route | 'other' {
  if (path === '/api/public/host') return 'host';
  if (path.startsWith('/api/public/orgs/') && path.endsWith('/rates')) return 'rates';
  if (path.startsWith('/api/public/orgs/') && path.endsWith('/branches')) return 'branches';
  return 'other';
}

function createServer(over: Partial<Record<Route, Reply[]>> = {}) {
  const queue: Record<Route, Reply[]> = {
    host: [{ status: 200, body: { org: 'XC' } }],
    rates: [{ status: 200, body: RATES, etag: '"p1-s0"' }],
    branches: [{ status: 200, body: BRANCHES_ENABLED, etag: '"b1"' }],
    ...over,
  };
  const calls: Call[] = [];
  const fetchMock = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
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
  return { fetch: fetchMock, calls, count: (r: Route | 'other') => calls.filter((c) => c.route === r).length };
}

// ── mounting through real entry main.tsx ───────────────────────────────────────────────────────────────

const BRANCHES_MS = 600_000;

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 10; i += 1) {
    await vi.advanceTimersByTimeAsync(0);
  }
}

async function mountThroughMain(search = '?branch=XC-B01', server: ReturnType<typeof createServer>): Promise<HTMLElement> {
  window.history.replaceState(null, '', `/rates${search}`);
  let root = document.getElementById('root');
  if (!root) {
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
  }
  vi.stubGlobal('fetch', server.fetch);
  await import('./main');
  await settle();
  return root;
}

beforeAll(() => warmModules(() => import('./main')), 60_000);

beforeEach(() => {
  mockAddData.mockClear();
  mockMake.mockClear();
  mockCreateSvgTag.mockClear();
  mockFactory.mockClear();
  vi.resetModules();
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
  window.localStorage.clear();
  setAudience('customer');
  setLang('th');
  document.body.innerHTML = '<div id="root"></div>';
});

afterEach(() => {
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
  setLang('th');
  setAudience('staff');
});

// ── RD-4: Booking QR Tests (D9a in PR 1656, #1374) ─────────────────────────────────────────────────────

describe('RD-4: Booking QR on P01 Rate Board (#1374, D9a)', () => {
  // §0a R-3: Target updated from /app/book to /app/queue
  it('1. bookingEnabled true: renders [data-booking-qr] with <svg> and data-qr-url pointing to /app/queue?branch=<code>', async () => {
    const s = createServer({
      branches: [{ status: 200, body: BRANCHES_ENABLED }],
    });
    const root = await mountThroughMain('?branch=XC-B01', s);

    const qrElement = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrElement, 'element with [data-booking-qr] must be present when bookingEnabled is true').not.toBeNull();

    const expectedUrl = `${window.location.origin}/app/queue?branch=XC-B01`;
    expect(
      qrElement?.getAttribute('data-qr-url'),
      'data-qr-url attribute must match <origin>/app/queue?branch=<branch code>',
    ).toBe(expectedUrl);

    const svg = qrElement?.querySelector('svg');
    expect(svg, 'booking QR must contain an inline <svg> element drawn client-side').not.toBeNull();

    expect(
      mockAddData,
      'qrcode-generator addData must receive the exact booking URL',
    ).toHaveBeenCalledWith(expectedUrl);
  });

  it('2. bookingEnabled false: renders no [data-booking-qr] element', async () => {
    const s = createServer({
      branches: [{ status: 200, body: BRANCHES_DISABLED }],
    });
    const root = await mountThroughMain('?branch=XC-B01', s);

    const qrElement = root.querySelector('[data-booking-qr]');
    expect(qrElement, 'no [data-booking-qr] should be rendered when bookingEnabled is false').toBeNull();
    expect(mockAddData, 'qrcode-generator must not be called when bookingEnabled is false').not.toHaveBeenCalled();
  });

  it('3. branch missing from the list: renders no [data-booking-qr] element', async () => {
    const s = createServer({
      branches: [{ status: 200, body: BRANCHES_ENABLED }],
    });
    // Branch XC-B99 does not exist in BRANCHES_ENABLED
    const root = await mountThroughMain('?branch=XC-B99', s);

    const qrElement = root.querySelector('[data-booking-qr]');
    expect(qrElement, 'no [data-booking-qr] should be rendered when the branch is not in the branch list').toBeNull();
    expect(mockAddData, 'qrcode-generator must not be called when branch is missing').not.toHaveBeenCalled();
  });

  // §0a R-3: Path updated to /app/queue
  it('4. caption text shows the /app/queue?branch=<code> path', async () => {
    const s = createServer({
      branches: [{ status: 200, body: BRANCHES_ENABLED }],
    });
    const root = await mountThroughMain('?branch=XC-B01', s);

    const expectedPath = '/app/queue?branch=XC-B01';
    const textContent = root.textContent ?? '';
    expect(
      textContent,
      `caption text on rate board must display the path ${expectedPath} for accessibility and non-scanning readers`,
    ).toContain(expectedPath);
  });

  it('5. language switch: target URL in data-qr-url remains unchanged across TH, EN, and ZH', async () => {
    const s = createServer({
      branches: [{ status: 200, body: BRANCHES_ENABLED }],
    });
    const root = await mountThroughMain('?branch=XC-B01', s);

    const qrElement = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrElement, '[data-booking-qr] must exist').not.toBeNull();

    // §0a R-3: Target updated to /app/queue
    const expectedUrl = `${window.location.origin}/app/queue?branch=XC-B01`;
    expect(qrElement?.getAttribute('data-qr-url')).toBe(expectedUrl);

    // Switch to EN
    setLang('en');
    await settle();
    expect(qrElement?.getAttribute('data-qr-url'), 'target URL must not change on language switch to EN').toBe(
      expectedUrl,
    );

    // Switch to ZH
    setLang('zh');
    await settle();
    expect(qrElement?.getAttribute('data-qr-url'), 'target URL must not change on language switch to ZH').toBe(
      expectedUrl,
    );
  });

  it('6. 304 poll of branches list preserves the [data-booking-qr] DOM node (no redraw)', async () => {
    const s = createServer({
      branches: [
        { status: 200, body: BRANCHES_ENABLED, etag: '"b1"' },
        { status: 304 },
      ],
    });
    const root = await mountThroughMain('?branch=XC-B01', s);

    const qrBefore = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrBefore, '[data-booking-qr] must be present before poll').not.toBeNull();

    // Advance to branches poll interval (600 s)
    await settle(BRANCHES_MS);

    const qrAfter = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrAfter, '[data-booking-qr] must still be present after 304 poll').not.toBeNull();
    expect(qrAfter, '304 branch poll must not redraw or replace the QR DOM node').toBe(qrBefore);
  });

  it('7. privacy constraint: QR URL and attributes contain no customer data or PII', async () => {
    const s = createServer({
      branches: [{ status: 200, body: BRANCHES_ENABLED }],
    });
    const root = await mountThroughMain('?branch=XC-B01', s);

    const qrElement = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrElement, '[data-booking-qr] must exist').not.toBeNull();

    const qrUrl = qrElement?.getAttribute('data-qr-url') ?? '';
    // Must be exactly `<origin>/app/queue?branch=<code>` without additional parameters, tokens, or PII (§0a R-3)
    expect(qrUrl).toMatch(/^https?:\/\/[^/]+\/app\/queue\?branch=[A-Za-z0-9_-]+$/);
  });
});
