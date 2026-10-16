// #2440: an older branch's or search term's answer must not land last and replace the list, its
// version or its rates. Render and fake set-up copied from the neighbouring bookings tests; here
// every view read is HELD until the test answers it, so a test chooses the order answers land in.
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

function row(id: string, branchId: string): Row {
  return {
    id,
    branchId,
    name: `Customer ${id}`,
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

function settingsWith(usdBuy: string) {
  return {
    rules: { enabled: true, minutes: 30 },
    memberCount: 0,
    publicUrl: '',
    counterRates: [{ currency: 'USD', buy: usdBuy, sell: usdBuy }],
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
  body: { action?: string; payload?: Record<string, unknown> } | undefined;
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
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  sent.push({ method, path: url.pathname, query: url.searchParams, body });
  if (method === 'GET' && url.pathname.startsWith('/api/v3/views/')) {
    const view = url.pathname.slice('/api/v3/views/'.length);
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
  if (method === 'POST' && url.pathname === '/api/v2/bookings') {
    return respond({ status: 200, body: { saved: true } });
  }
  return respond({ status: 404, body: { error: { code: 'notFound', message: 'no route' } } });
}

/** The oldest unanswered read of `view` that matches `match`. */
async function heldRead(view: string, match: (q: URLSearchParams) => boolean): Promise<Held> {
  let found: Held | undefined;
  await vi.waitFor(() => {
    found = held.find((h) => !h.answered && h.view === view && match(h.query));
    expect(found, `a held ${view} read`).toBeDefined();
  });
  return found!;
}

const ofBranch = (b: string) => (q: URLSearchParams) => q.get('branch') === b && !q.has('before');

function ok(version: number, data: unknown): Reply {
  return { status: 200, body: { version, changed: true, data } };
}

const FAIL: Reply = { status: 500, body: { error: { code: 'serverError', message: 'boom' } } };

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

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function inMenu<T extends Element>(selector: string): T[] {
  return [...container.querySelectorAll<T>(selector)].filter((e) => !e.closest('dialog'));
}

function rowIds(): string[] {
  return inMenu<HTMLTableRowElement>('tbody tr').map((r) => norm(r.querySelector('strong')?.textContent));
}

function menuButton(text: string): HTMLButtonElement {
  const found = inMenu<HTMLButtonElement>('button').filter((b) => norm(b.textContent).includes(text));
  expect(found.length, `one menu button "${text}"`).toBe(1);
  return found[0]!;
}

function setValue(el: HTMLInputElement | HTMLSelectElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function search(): HTMLInputElement {
  const input = container.querySelector<HTMLInputElement>('input[type="search"]');
  expect(input).not.toBeNull();
  return input!;
}

function bookingReads(): Sent[] {
  return sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${BOOKINGS}`);
}

function lastBookingRead(): Sent {
  const all = bookingReads();
  expect(all.length).toBeGreaterThan(0);
  return all[all.length - 1]!;
}

/** Mounted on MAIN with both first reads answered. */
async function openedOnMain(rows: Row[], version = 3) {
  const handles = mount();
  (await heldRead(BOOKINGS, ofBranch('MAIN'))).answer(ok(version, rows));
  (await heldRead(SETTINGS, ofBranch('MAIN'))).answer(ok(version, settingsWith('36.2500')));
  await settle();
  return handles;
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

describe('#2440: an older read that lands last does not replace the newer one', () => {
  it("branch A → B, A's list answer lands after B's: B's rows are shown and the next refresh sends B's version", async () => {
    const { setBranch } = mount();
    const aList = await heldRead(BOOKINGS, ofBranch('MAIN'));
    (await heldRead(SETTINGS, ofBranch('MAIN'))).answer(ok(3, settingsWith('36.2500')));

    setBranch('AIRPORT');
    (await heldRead(BOOKINGS, ofBranch('AIRPORT'))).answer(ok(20, [row('BK-B1', 'AIRPORT')]));
    (await heldRead(SETTINGS, ofBranch('AIRPORT'))).answer(ok(20, settingsWith('30.0000')));
    await vi.waitFor(() => expect(rowIds()).toEqual(['BK-B1']));

    aList.answer(ok(3, [row('BK-A1', 'MAIN'), row('BK-A2', 'MAIN')]));
    await settle();
    expect(rowIds(), "A's late answer does not replace B's rows").toEqual(['BK-B1']);

    menuButton('โหลดการจองล่าสุด').click();
    const refresh = await heldRead(BOOKINGS, ofBranch('AIRPORT'));
    expect(refresh.query.get('since'), "B's version, not A's").toBe('20');
  });

  it("branch A → B, A's settings answer lands after B's: Create uses B's counter rates", async () => {
    const { setBranch } = mount();
    (await heldRead(BOOKINGS, ofBranch('MAIN'))).answer(ok(3, []));
    const aSettings = await heldRead(SETTINGS, ofBranch('MAIN'));

    setBranch('AIRPORT');
    (await heldRead(BOOKINGS, ofBranch('AIRPORT'))).answer(ok(20, []));
    (await heldRead(SETTINGS, ofBranch('AIRPORT'))).answer(ok(20, settingsWith('30.0000')));
    await settle();

    aSettings.answer(ok(3, settingsWith('36.2500')));
    await settle();

    menuButton('สร้างการจอง').click();
    await settle();
    const d = container.querySelector('dialog')!;
    expect(d.open).toBe(true);
    const inputs = [...d.querySelectorAll<HTMLInputElement>('input')];
    const [name, phone] = inputs;
    const amount = inputs.find((i, idx) => idx > 1 && i.type === 'text' && !i.readOnly)!;
    setValue(name!, 'Walk In');
    setValue(phone!, '081 234 5678');
    setValue(amount, '100');
    await settle();
    d.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    await vi.waitFor(() => expect(sent.filter((s) => s.method === 'POST')).toHaveLength(1));
    const payload = sent.find((s) => s.method === 'POST')!.body!.payload!;
    expect(payload.branchId).toBe('AIRPORT');
    expect(payload.expectedRate, "B's counter rate, not A's").toBe('30.0000');
  });

  it("search q1 → q2, q1's answer lands last: q2's rows are shown", async () => {
    await openedOnMain([row('BK-ALL', 'MAIN')]);

    setValue(search(), 'q1');
    const q1 = await heldRead(BOOKINGS, (q) => q.get('q') === 'q1');
    setValue(search(), 'q2');
    const q2 = await heldRead(BOOKINGS, (q) => q.get('q') === 'q2');

    q2.answer(ok(5, [row('BK-Q2', 'MAIN')]));
    await vi.waitFor(() => expect(rowIds()).toEqual(['BK-Q2']));

    q1.answer(ok(4, [row('BK-Q1', 'MAIN')]));
    await settle();
    expect(rowIds(), "q1's late answer does not replace q2's rows").toEqual(['BK-Q2']);
  });

  it('load more in A, switch to B before it answers: B\'s list unchanged, no A rows appended', async () => {
    const firstPage = Array.from({ length: 50 }, (_, i) => row(`BK-A${String(i + 1).padStart(3, '0')}`, 'MAIN'));
    const { setBranch } = await openedOnMain(firstPage);
    expect(rowIds()).toHaveLength(50);

    menuButton('โหลดเพิ่มเติม').click();
    const page = await heldRead(BOOKINGS, (q) => q.get('branch') === 'MAIN' && q.has('before'));

    setBranch('AIRPORT');
    (await heldRead(BOOKINGS, ofBranch('AIRPORT'))).answer(ok(20, [row('BK-B1', 'AIRPORT')]));
    (await heldRead(SETTINGS, ofBranch('AIRPORT'))).answer(ok(20, settingsWith('30.0000')));
    await vi.waitFor(() => expect(rowIds()).toEqual(['BK-B1']));

    page.answer(ok(3, [row('BK-A051', 'MAIN'), row('BK-A052', 'MAIN')]));
    await settle();
    expect(rowIds(), 'neither A page 1 nor A page 2 comes back').toEqual(['BK-B1']);
  });

  it("A's read fails after B's succeeded: no error is shown and B's rows stay", async () => {
    const { setBranch } = mount();
    const aList = await heldRead(BOOKINGS, ofBranch('MAIN'));
    const aSettings = await heldRead(SETTINGS, ofBranch('MAIN'));

    setBranch('AIRPORT');
    (await heldRead(BOOKINGS, ofBranch('AIRPORT'))).answer(ok(20, [row('BK-B1', 'AIRPORT')]));
    (await heldRead(SETTINGS, ofBranch('AIRPORT'))).answer(ok(20, settingsWith('30.0000')));
    await vi.waitFor(() => expect(rowIds()).toEqual(['BK-B1']));
    const before = norm(inMenu<HTMLElement>('.admin-bookings')[0]?.textContent);

    aList.answer(FAIL);
    aSettings.answer(FAIL);
    await settle();
    expect(rowIds(), "B's rows stay").toEqual(['BK-B1']);
    expect(norm(inMenu<HTMLElement>('.admin-bookings')[0]?.textContent), 'no error text appears').toBe(before);
  });
});

// Edge cases (ticket part 9): today's behaviour that the fix must keep. These are guards; they are
// green on origin/main as well as with the fix.
describe('#2440 edge cases: kept behaviour', () => {
  it('a same-branch refresh (no switch) still sends the held version', async () => {
    await openedOnMain([row('BK-A1', 'MAIN')], 7);
    menuButton('โหลดการจองล่าสุด').click();
    const list = await heldRead(BOOKINGS, ofBranch('MAIN'));
    const sett = await heldRead(SETTINGS, ofBranch('MAIN'));
    expect(list.query.get('since')).toBe('7');
    expect(sett.query.get('since')).toBe('7');
    list.answer({ status: 200, body: { version: 7, changed: false } });
    sett.answer({ status: 200, body: { version: 7, changed: false } });
    await settle();
    expect(rowIds()).toEqual(['BK-A1']);
  });

  it('clearing the search sends no filter, as today', async () => {
    await openedOnMain([row('BK-ALL', 'MAIN')]);
    setValue(search(), 'somchai');
    (await heldRead(BOOKINGS, (q) => q.get('q') === 'somchai')).answer(ok(4, [row('BK-S', 'MAIN')]));
    await vi.waitFor(() => expect(rowIds()).toEqual(['BK-S']));

    setValue(search(), '');
    const cleared = await heldRead(BOOKINGS, ofBranch('MAIN'));
    expect(cleared.query.has('q')).toBe(false);
    expect(cleared.query.has('since')).toBe(false);
    expect(lastBookingRead().query.has('q')).toBe(false);
    cleared.answer(ok(5, [row('BK-ALL', 'MAIN')]));
    await vi.waitFor(() => expect(rowIds()).toEqual(['BK-ALL']));
  });
});
