// V2-bookings (#463 §10, decision 2): paging test for the "load more" control
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

const loadedMenu = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = loadedMenu['./menu.ts'];
  expect(m).toBeDefined();
  return m!.menu;
}

const BOOKINGS = 'admin_bookings';
const SETTINGS = 'admin_booking_settings';
const IGNORED = PAGE_WIDE;

interface Row {
  id: string;
  branchId: string;
  name: string;
  phone: string;
  member: boolean;
  type: 'BUY' | 'SELL';
  currency: string;
  amount: string;
  rate: string;
  status: string;
  createdAt: string;
  expiresAt: string | null;
  transactionRef: string | null;
  counterRate: string | null;
  counterPrincipal: string | null;
}

function makeRow(i: number): Row {
  return {
    id: `BK-${String(i).padStart(3, '0')}`,
    branchId: 'MAIN',
    name: `Customer ${i}`,
    phone: '081 234 5678',
    member: false,
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '36.2500',
    status: 'CONFIRMED',
    createdAt: '2026-10-09T05:00:00.000Z',
    expiresAt: '2999-01-01T00:00:00.000Z',
    transactionRef: null,
    counterRate: null,
    counterPrincipal: null,
  };
}

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
}

let sent: Sent[];
let pagedResponses: Map<string, Row[]>;

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const method = (init?.method ?? 'GET').toUpperCase();
  if (IGNORED.includes(url.pathname)) {
    return new Response(JSON.stringify({ version: 1, changed: true, data: {} }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  }
  sent.push({ method, path: url.pathname, query: url.searchParams });
  if (method === 'GET' && url.pathname.startsWith('/api/v3/views/')) {
    const view = url.pathname.slice('/api/v3/views/'.length);
    if (view === SETTINGS) {
      return new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: {
            rules: { enabled: true, minutes: 30 },
            memberCount: 0,
            publicUrl: '',
            counterRates: [],
          },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }
    if (view === BOOKINGS) {
      const before = url.searchParams.get('before');
      const q = url.searchParams.get('q');
      const key = before ? (q ? `${before}-q-${q}` : before) : (q ? `initial-q-${q}` : 'initial');
      const rows = pagedResponses.get(key) ?? [];
      return new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: rows,
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }
  }
  return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } });
}

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function mount() {
  const [branch] = createSignal('MAIN');
  const [shown] = createSignal(true);
  const period = () => ({ from: '', to: '' });
  const Menu = theMenu().Component;
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(
    () =>
      (
        <>
          <Menu branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    container,
  );
}

function loadMoreButton(): HTMLButtonElement | null {
  const buttons = [...container.querySelectorAll<HTMLButtonElement>('button')];
  return buttons.find((b) => b.textContent?.trim() === 'โหลดเพิ่มเติม' || b.textContent?.trim() === 'Load more') ?? null;
}

function rowsShown(): HTMLTableRowElement[] {
  return [...container.querySelectorAll<HTMLTableRowElement>('tbody tr')];
}

function searchInput(): HTMLInputElement {
  const input = container.querySelector<HTMLInputElement>('input[placeholder="เลขจอง / ชื่อ / เบอร์โทร"]');
  expect(input).not.toBeNull();
  return input!;
}

beforeEach(() => {
  setLang('th');
  sent = [];
  pagedResponses = new Map();
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  container?.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('V2-bookings: keyset pagination (#463 decision 2)', () => {
  it('shows load-more when 50 rows come back, reads next page with before and limit 50, and hides when short page arrives', async () => {
    // 50 initial rows (BK-001 to BK-050)
    const firstPage = Array.from({ length: 50 }, (_, i) => makeRow(i + 1));
    // 20 rows on next page (BK-051 to BK-070)
    const secondPage = Array.from({ length: 20 }, (_, i) => makeRow(i + 51));

    pagedResponses.set('initial', firstPage);
    pagedResponses.set('BK-050', secondPage);

    mount();
    await vi.waitFor(() => expect(rowsShown()).toHaveLength(50));
    await settle();

    // The load more button is shown
    const btn = loadMoreButton();
    expect(btn, 'load more button is rendered').not.toBeNull();
    expect(btn!.textContent?.trim()).toBe('โหลดเพิ่มเติม');

    // Click load more
    btn!.click();
    await vi.waitFor(() => expect(rowsShown()).toHaveLength(70));

    // Verify the page read request
    const pagedReads = sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${BOOKINGS}`);
    expect(pagedReads.length).toBe(2);
    const secondRead = pagedReads[1]!;
    expect(secondRead.query.get('branch')).toBe('MAIN');
    expect(secondRead.query.get('before')).toBe('BK-050');
    expect(secondRead.query.get('limit')).toBe('50');

    // Short page (< 50) means no more rows: load more button is hidden
    await settle();
    expect(loadMoreButton()).toBeNull();
  });

  it('load-more keeps current search query q in request', async () => {
    const firstPage = Array.from({ length: 50 }, (_, i) => makeRow(i + 1));
    const secondPage = Array.from({ length: 20 }, (_, i) => makeRow(i + 51));

    pagedResponses.set('initial-q-somchai', firstPage);
    pagedResponses.set('BK-050-q-somchai', secondPage);

    mount();
    await settle();

    const box = searchInput();
    box.value = 'somchai';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    box.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(50));
    await settle();

    const btn = loadMoreButton();
    expect(btn).not.toBeNull();
    btn!.click();

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(70));

    const pagedReads = sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${BOOKINGS}`);
    const lastRead = pagedReads[pagedReads.length - 1]!;
    expect(lastRead.query.get('branch')).toBe('MAIN');
    expect(lastRead.query.get('before')).toBe('BK-050');
    expect(lastRead.query.get('limit')).toBe('50');
    expect(lastRead.query.get('q')).toBe('somchai');
  });
});
