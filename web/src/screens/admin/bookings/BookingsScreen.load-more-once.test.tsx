// #2610: clicking "Load more" twice before the page arrives must not send
// duplicate requests or duplicate rows. An in-flight guard disables the button
// while loading more and ignores subsequent clicks until settled.
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

function makeRow(i: number, branchId = 'MAIN'): Row {
  return {
    id: `BK-${String(i).padStart(3, '0')}`,
    branchId,
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

type Reply = { status: number; body: unknown };

interface Held {
  view: string;
  query: URLSearchParams;
  answered: boolean;
  answer: (r: Reply) => void;
}

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
}

let held: Held[];
let sent: Sent[];

function respond(r: Reply): Response {
  return new Response(JSON.stringify(r.body), {
    status: r.status,
    headers: { 'content-type': 'application/json' },
  });
}

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const method = (init?.method ?? 'GET').toUpperCase();
  if (IGNORED.includes(url.pathname)) {
    return respond({
      status: 200,
      body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN' } },
    });
  }
  sent.push({ method, path: url.pathname, query: url.searchParams });
  if (method === 'GET' && url.pathname.startsWith('/api/v3/views/')) {
    const view = url.pathname.slice('/api/v3/views/'.length);
    if (view === SETTINGS) {
      return respond({
        status: 200,
        body: {
          version: 1,
          changed: true,
          data: {
            rules: { enabled: true, minutes: 30 },
            memberCount: 0,
            publicUrl: '',
            counterRates: [],
          },
        },
      });
    }
    if (view === BOOKINGS) {
      return new Promise<Response>((resolve) => {
        const h: Held = {
          view,
          query: url.searchParams,
          answered: false,
          answer: (r) => {
            h.answered = true;
            resolve(respond(r));
          },
        };
        held.push(h);
      });
    }
  }
  return respond({ status: 404, body: { error: { code: 'notFound' } } });
}

async function heldBookingRead(match: (q: URLSearchParams) => boolean): Promise<Held> {
  let found: Held | undefined;
  await vi.waitFor(() => {
    found = held.find((h) => !h.answered && h.view === BOOKINGS && match(h.query));
    expect(found, 'a held booking read').toBeDefined();
  });
  return found!;
}

function ok(data: unknown, version = 1): Reply {
  return { status: 200, body: { version, changed: true, data } };
}

const FAIL: Reply = { status: 500, body: { error: { code: 'serverError', message: 'Failed to load' } } };

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => realSetTimeout(r, 0));
}

function mount() {
  const [branch, setBranch] = createSignal('MAIN');
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
  return { setBranch };
}

function loadMoreButton(): HTMLButtonElement | null {
  const buttons = [...container.querySelectorAll<HTMLButtonElement>('button')];
  return buttons.find((b) => b.textContent?.trim() === 'โหลดเพิ่มเติม' || b.textContent?.trim() === 'Load more') ?? null;
}

function rowsShown(): HTMLTableRowElement[] {
  return [...container.querySelectorAll<HTMLTableRowElement>('tbody tr')];
}

function rowIds(): string[] {
  return rowsShown().map((r) => r.querySelector('strong')?.textContent?.trim() ?? '');
}


beforeEach(() => {
  setLang('th');
  held = [];
  sent = [];
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

describe('#2610: admin bookings load-more in-flight guard', () => {
  it('row 1: two clicks before the answer -> one page request, button disabled, no duplicate rows', async () => {
    mount();

    // Answer initial 50 bookings
    const first50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 1));
    const initialRead = await heldBookingRead((q) => !q.has('before'));
    initialRead.answer(ok(first50));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(50));
    await settle();

    const btn = loadMoreButton();
    expect(btn, 'load more button visible').not.toBeNull();
    expect(btn!.disabled, 'button initially enabled').toBe(false);

    // First click: triggers handleLoadMore
    btn!.click();
    await settle();

    // Button should be disabled while load-more is in flight
    expect(btn!.disabled, 'button disabled while load is in flight').toBe(true);

    // A page request has been sent and is held
    const heldPage = await heldBookingRead((q) => q.get('before') === 'BK-050');
    expect(heldPage, 'first page request held').toBeDefined();

    // Second click before answer
    btn!.click();
    await settle();

    // Still only 1 page request sent
    const pageReads = sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${BOOKINGS}` && s.query.has('before'));
    expect(pageReads.length, 'only one page request sent despite two clicks').toBe(1);

    // Answer the held page with next 50 bookings (BK-051 to BK-100)
    const second50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 51));
    heldPage.answer(ok(second50));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(100));
    await settle();

    // Verify row ids have no duplicates
    const ids = rowIds();
    expect(ids).toHaveLength(100);
    const uniqueIds = new Set(ids);
    expect(uniqueIds.size, 'all 100 rows must be unique with no duplicates').toBe(100);

    // Button is enabled again after answer
    expect(btn!.disabled, 'button enabled again after response arrives').toBe(false);
  });

  it('row 2: click after the answer -> next page with the new before', async () => {
    mount();

    const first50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 1));
    const initialRead = await heldBookingRead((q) => !q.has('before'));
    initialRead.answer(ok(first50));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(50));
    await settle();

    const btn = loadMoreButton();
    expect(btn).not.toBeNull();

    // First load-more
    btn!.click();
    const firstPage = await heldBookingRead((q) => q.get('before') === 'BK-050');
    const second50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 51));
    firstPage.answer(ok(second50));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(100));
    await settle();

    // Click after first page arrives
    expect(btn!.disabled).toBe(false);
    btn!.click();
    await settle();

    // Verifies next page request uses the new before (BK-100)
    const secondPage = await heldBookingRead((q) => q.get('before') === 'BK-100');
    expect(secondPage).toBeDefined();

    const thirdPage = Array.from({ length: 20 }, (_, i) => makeRow(i + 101));
    secondPage.answer(ok(thirdPage));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(120));
    await settle();

    // Short page hides the button
    expect(loadMoreButton()).toBeNull();
  });

  it('row 3: search or branch change during the load -> the page is dropped (kept)', async () => {
    const { setBranch } = mount();

    const first50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 1));
    const initialRead = await heldBookingRead((q) => !q.has('before'));
    initialRead.answer(ok(first50));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(50));
    await settle();

    const btn = loadMoreButton();
    expect(btn).not.toBeNull();

    btn!.click();
    const heldPage = await heldBookingRead((q) => q.get('before') === 'BK-050');

    // Branch changes while page read is in-flight
    setBranch('BRANCH-2');
    await settle();

    // Now page answers for old branch
    const second50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 51));
    heldPage.answer(ok(second50));
    await settle();

    // The late page answer from the old branch must be dropped
    const ids = rowIds();
    expect(ids.some((id) => id === 'BK-051'), 'late page rows must not be appended').toBe(false);
  });

  it('edge case (§9): a failed load clears the guard so the button works again', async () => {
    mount();

    const first50 = Array.from({ length: 50 }, (_, i) => makeRow(i + 1));
    const initialRead = await heldBookingRead((q) => !q.has('before'));
    initialRead.answer(ok(first50));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(50));
    await settle();

    const btn = loadMoreButton();
    expect(btn).not.toBeNull();

    // Click load more
    btn!.click();
    await settle();
    expect(btn!.disabled, 'button disabled while load is in flight').toBe(true);

    const heldPage = await heldBookingRead((q) => q.get('before') === 'BK-050');
    // Fail the request
    heldPage.answer(FAIL);

    await settle();

    // Button should be re-enabled and guard cleared
    expect(btn!.disabled, 'button re-enabled after failure').toBe(false);

    // Can click again and retry
    btn!.click();
    await settle();

    const retryPage = await heldBookingRead((q) => q.get('before') === 'BK-050');
    expect(retryPage, 'retry page request sent').toBeDefined();

    const retryRows = Array.from({ length: 20 }, (_, i) => makeRow(i + 51));
    retryPage.answer(ok(retryRows));

    await vi.waitFor(() => expect(rowsShown()).toHaveLength(70));
  });
});
