// V2-bookings (#463): the bookings menu, LIVE (ticket §10 K1–K3, §9; design §12.1, §12.6, §12.14,
// §12.17). Written before the implementation by the lane controller (claude/7ca80c). Do not
// edit, delete or weaken; add tests in a file of your own.
//
// v2 is `xchange-v2-source@4d0bdbb` `src/online/public/bookings.js:N` and the decoded admin page
// `admin-html:19` (the toolbar). v3 reads the two views of `api/tests/v3_admin_bookings_view_test.rs`
// — `admin_bookings` (rows, filter `q`) and `admin_booking_settings` (rules, member count,
// public url, counter rates) — plus the shell's `admin_branches` for the branch's name (lane
// controller on #462, decision 2), and writes through `ApiClient.post('/api/bookings', {action,
// payload: {branchId, …}})`, as v2's `bookingApi` does (`bookings.js:5`).
//
// The menu is mounted through its record, `menu.Component({branch, period, shown})`, as the shell
// mounts it, beside the parts' `<Dialog/>` and `<Toast/>` that the shell renders once. The menu
// builds its own `new ApiClient()` in its folder (lane controller on #462, decision 1), so the
// fake sits under it: the global `fetch` answers as the server would, and records every request. The page-wide reads AppShell makes (`PAGE_WIDE`, `shared/testing/page-wide.ts`)
// are answered and never counted.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import { Toast } from '../parts/toast';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

const loadedMenu = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const loadedTable = import.meta.glob<{ th: Record<string, string>; en: Record<string, string> }>(
  './i18n.ts',
  { eager: true },
);

function theMenu(): AdminMenu {
  const m = loadedMenu['./menu.ts'];
  expect(m, 'web/src/screens/admin/bookings/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

const BOOKINGS = 'admin_bookings';
const SETTINGS = 'admin_booking_settings';
const BRANCHES = 'admin_branches';
const IGNORED = PAGE_WIDE;
const THAI = /[฀-๿]/;

// ── what the server answers ──────────────────────────────────────────────────

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

interface Settings {
  rules: { enabled: boolean; minutes: number };
  memberCount: number;
  publicUrl: string;
  counterRates: { currency: string; buy: string; sell: string }[];
}

const PAST = '2020-01-01T00:00:00.000Z';
const FUTURE = '2999-01-01T00:00:00.000Z';

function bookingRow(over: Partial<Row>): Row {
  return {
    id: 'BK-x',
    branchId: 'MAIN',
    name: 'Somchai Jaidee',
    phone: '081 234 5678',
    member: false,
    type: 'BUY',
    currency: 'USD',
    amount: '1000.00',
    rate: '36.2500',
    status: 'CONFIRMED',
    createdAt: '2026-10-09T05:00:00.000Z',
    expiresAt: FUTURE,
    transactionRef: null,
    counterRate: null,
    counterPrincipal: null,
    ...over,
  };
}

const PENDING = bookingRow({
  id: 'BK-p1',
  name: 'Somchai Pending',
  member: true,
  status: 'PENDING',
  expiresAt: null,
  counterRate: '36.5000',
  // deliberately not amount × rate: the screen shows the server's figure and computes nothing
  counterPrincipal: '11111.11',
});
const CONFIRMED = bookingRow({
  id: 'BK-c1',
  name: 'Anan Walkin',
  phone: '02 111 2222',
  type: 'SELL',
  currency: 'EUR',
  amount: '250.50',
  rate: '40.1000',
  expiresAt: '2026-10-09T05:30:02.000Z',
});
const EXPIRED = bookingRow({ id: 'BK-e1', name: 'Expired One', status: 'EXPIRED', expiresAt: FUTURE });
const USED = bookingRow({ id: 'BK-u1', name: 'Used One', status: 'USED', transactionRef: 'XC-DOC-1' });
const CANCELLED = bookingRow({ id: 'BK-x1', name: 'Cancelled One', status: 'CANCELLED' });

function settingsOf(over: Partial<Settings> = {}): Settings {
  return {
    rules: { enabled: true, minutes: 30 },
    memberCount: 2,
    publicUrl: '',
    counterRates: [
      { currency: 'EUR', buy: '39.5000', sell: '40.1000' },
      { currency: 'USD', buy: '36.2500', sell: '36.7500' },
    ],
    ...over,
  };
}

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
  body: { action?: string; payload?: Record<string, unknown> } | undefined;
  headers: Record<string, string>;
}

type Reply = { status: number; body: unknown };

interface Server {
  version: number;
  rows: Row[];
  settings: Settings;
  /** Answers one read before the default does; `undefined` falls through. */
  read?: (view: string, query: URLSearchParams) => Promise<Reply | undefined> | Reply | undefined;
  post: (action: string, payload: Record<string, unknown>) => Reply;
}

let server: Server;
let sent: Sent[];

function respond(r: Reply): Response {
  return new Response(JSON.stringify(r.body), {
    status: r.status,
    headers: { 'content-type': 'application/json' },
  });
}

function headersOf(init?: RequestInit): Record<string, string> {
  const out: Record<string, string> = {};
  const h = init?.headers;
  if (!h) return out;
  new Headers(h).forEach((v, k) => {
    out[k.toLowerCase()] = v;
  });
  return out;
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
  sent.push({ method, path: url.pathname, query: url.searchParams, body, headers: headersOf(init) });
  if (method === 'GET' && url.pathname.startsWith('/api/v3/views/')) {
    const view = url.pathname.slice('/api/v3/views/'.length);
    const custom = server.read ? await server.read(view, url.searchParams) : undefined;
    if (custom) return respond(custom);
    const since = url.searchParams.get('since');
    if (since !== null && Number(since) === server.version && !url.searchParams.has('before')) {
      return respond({ status: 200, body: { version: server.version, changed: false } });
    }
    if (view === BOOKINGS) {
      return respond({ status: 200, body: { version: server.version, changed: true, data: server.rows } });
    }
    if (view === SETTINGS) {
      return respond({ status: 200, body: { version: server.version, changed: true, data: server.settings } });
    }
    if (view === BRANCHES) {
      // VERSIONED = false (design §12.4): always the whole list
      return respond({
        status: 200,
        body: {
          version: server.version,
          changed: true,
          data: { branches: [{ id: 'MAIN', name: 'XC-B' }, { id: 'AIRPORT', name: 'Airport' }] },
        },
      });
    }
  }
  if (method === 'POST' && url.pathname === '/api/bookings') {
    return respond(server.post(body?.action ?? '', body?.payload ?? {}));
  }
  return respond({ status: 404, body: { error: { code: 'notFound', message: `no ${method} ${url.pathname}` } } });
}

function reads(view: string): Sent[] {
  return sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${view}`);
}

function lastRead(view: string): Sent {
  const all = reads(view);
  expect(all.length, `${view} was read`).toBeGreaterThan(0);
  return all[all.length - 1]!;
}

function posts(): Sent[] {
  return sent.filter((s) => s.method === 'POST');
}

// ── rendering ────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

const realSetTimeout = globalThis.setTimeout;

/** Lets the menu's promises and effects run (with or without fake timers). */
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function mount(opts: { branch?: string; shown?: boolean } = {}) {
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [shown, setShown] = createSignal(opts.shown ?? true);
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
  return { setBranch, setShown };
}

/** Mounted, shown, and the first rows on screen. */
async function opened(opts: { branch?: string } = {}) {
  const handles = mount(opts);
  await vi.waitFor(() => {
    expect(reads(BOOKINGS).length).toBeGreaterThan(0);
    expect(reads(SETTINGS).length).toBeGreaterThan(0);
  });
  await settle();
  return handles;
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function visible(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (e.hasAttribute('hidden')) return false;
    if (e instanceof HTMLElement && e.style.display === 'none') return false;
  }
  return true;
}

function theDialog(): HTMLDialogElement | null {
  const d = container.querySelector('dialog');
  return d && d.open ? d : null;
}

function openDialogEl(): HTMLDialogElement {
  const d = theDialog();
  expect(d, 'a dialog is open').not.toBeNull();
  return d!;
}

/** Elements outside the open dialog: the menu itself. */
function inMenu<T extends Element>(selector: string): T[] {
  return [...container.querySelectorAll<T>(selector)].filter((e) => !e.closest('dialog') && visible(e));
}

function buttonsIn(root: ParentNode, text: string): HTMLButtonElement[] {
  return [...root.querySelectorAll('button')].filter((b) => norm(b.textContent) === text && visible(b));
}

function menuButton(text: string): HTMLButtonElement {
  const found = inMenu<HTMLButtonElement>('button').filter((b) => norm(b.textContent) === text);
  expect(found.length, `one menu button "${text}"`).toBe(1);
  return found[0]!;
}

function dialogButton(text: string): HTMLButtonElement {
  const found = buttonsIn(openDialogEl(), text);
  expect(found.length, `one dialog button "${text}"`).toBe(1);
  return found[0]!;
}

function rowsShown(): HTMLTableRowElement[] {
  return inMenu<HTMLTableRowElement>('tbody tr');
}

function rowOf(id: string): HTMLTableRowElement {
  const found = rowsShown().filter((r) => norm(r.textContent).includes(id));
  expect(found.length, `one row for ${id}`).toBe(1);
  return found[0]!;
}

function cells(row: HTMLTableRowElement): string[] {
  return [...row.querySelectorAll('td')].map((c) => norm(c.textContent));
}

/** The menu's own text: not the dialog, not the page's toast. */
function menuText(): string {
  return inMenu<HTMLElement>(':scope > *')
    .filter((e) => e.tagName !== 'DIALOG' && !(e.getAttribute('role') === 'status' && e.classList.contains('toast')))
    .map((e) => norm(e.textContent))
    .join(' ');
}

function toastText(): string | null {
  const status = [...container.querySelectorAll('[role="status"]')].find((s) => s.classList.contains('toast'));
  if (!status || !visible(status)) return null;
  const t = norm(status.textContent);
  return t === '' ? null : t;
}

function setValue(el: HTMLInputElement | HTMLSelectElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

/** A label's own words, without the text of the control it wraps. */
function labelText(label: HTMLLabelElement): string {
  const copy = label.cloneNode(true) as HTMLLabelElement;
  copy.querySelectorAll('input, select, textarea, option, button').forEach((c) => c.remove());
  return norm(copy.textContent);
}

function controlOf(label: HTMLLabelElement): HTMLInputElement | HTMLSelectElement | null {
  const inner = label.querySelector<HTMLInputElement | HTMLSelectElement>('input, select, textarea');
  if (inner) return inner;
  if (label.htmlFor) return document.getElementById(label.htmlFor) as HTMLInputElement | HTMLSelectElement | null;
  return null;
}

function labelsOf(root: ParentNode): HTMLLabelElement[] {
  return [...root.querySelectorAll('label')].filter((l) => visible(l));
}

function field(root: ParentNode, text: string): HTMLInputElement | HTMLSelectElement {
  const found = labelsOf(root).filter((l) => labelText(l) === text);
  expect(found.length, `one field labelled "${text}"`).toBe(1);
  const control = controlOf(found[0]!);
  expect(control, `"${text}" labels a control`).not.toBeNull();
  return control!;
}

function search(): HTMLInputElement {
  const found = inMenu<HTMLLabelElement>('label').filter((l) => labelText(l) === 'ค้นหาการจอง');
  expect(found.length, 'one search box labelled ค้นหาการจอง').toBe(1);
  return controlOf(found[0]!) as HTMLInputElement;
}

beforeEach(() => {
  setLang('th');
  sent = [];
  server = {
    version: 7,
    rows: [PENDING, CONFIRMED],
    settings: settingsOf(),
    post: () => ({ status: 200, body: { saved: true } }),
  };
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  container?.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  setLang('th');
});

// ── reading (K1, design §12.6, §12.1) ─────────────────────────────────────────

describe('V2-bookings: when the menu reads (#463 K1; design §12.6)', () => {
  it('reads nothing while hidden; once shown, reads both views for its branch without a since', async () => {
    const { setShown } = mount({ shown: false });
    await settle();
    expect(reads(BOOKINGS)).toHaveLength(0);
    expect(reads(SETTINGS)).toHaveLength(0);

    setShown(true);
    await vi.waitFor(() => {
      expect(reads(BOOKINGS)).toHaveLength(1);
      expect(reads(SETTINGS)).toHaveLength(1);
    });
    for (const view of [BOOKINGS, SETTINGS]) {
      const q = lastRead(view).query;
      expect(q.get('branch'), view).toBe('MAIN');
      expect(q.has('since'), `${view}: the first read has no since`).toBe(false);
      expect(q.has('q'), `${view}: no search, no q`).toBe(false);
    }
    await settle();
    expect(rowsShown()).toHaveLength(2);
  });

  it('re-reads both views with the since it holds on โหลดการจองล่าสุด and on the page refresh; changed:false keeps the rows', async () => {
    await opened();
    expect(rowsShown()).toHaveLength(2);

    menuButton('โหลดการจองล่าสุด').click();
    await vi.waitFor(() => expect(reads(BOOKINGS)).toHaveLength(2));
    await vi.waitFor(() => expect(reads(SETTINGS)).toHaveLength(2));
    expect(lastRead(BOOKINGS).query.get('since')).toBe('7');
    expect(lastRead(SETTINGS).query.get('since')).toBe('7');
    await settle();
    expect(rowsShown(), 'changed:false keeps what is shown').toHaveLength(2);
    rowOf('BK-p1');

    server.version = 8;
    server.rows = [USED, PENDING, CONFIRMED];
    requestRefresh();
    await vi.waitFor(() => expect(reads(BOOKINGS)).toHaveLength(3));
    expect(lastRead(BOOKINGS).query.get('since')).toBe('7');
    await vi.waitFor(() => expect(rowsShown()).toHaveLength(3));
    rowOf('BK-u1');

    requestRefresh();
    await vi.waitFor(() => expect(reads(BOOKINGS)).toHaveLength(4));
    expect(lastRead(BOOKINGS).query.get('since'), 'the since of the last answer').toBe('8');
  });

  it("reads the new branch without a since when the header's branch changes", async () => {
    const { setBranch } = await opened();
    setBranch('AIRPORT');
    await vi.waitFor(() => expect(reads(BOOKINGS)).toHaveLength(2));
    await vi.waitFor(() => expect(reads(SETTINGS)).toHaveLength(2));
    for (const view of [BOOKINGS, SETTINGS]) {
      const q = lastRead(view).query;
      expect(q.get('branch'), view).toBe('AIRPORT');
      expect(q.has('since'), `${view}: another branch's version is not this one's`).toBe(false);
    }
  });

  it('reads nothing on a timer: ten minutes later, no request of its own', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await opened();
    const before = sent.length;
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(sent.slice(before).map((s) => `${s.method} ${s.path}`)).toEqual([]);
  });

  it('shows กำลังโหลดการจอง… while the first read is on its way', async () => {
    let release: (r: Reply) => void = () => undefined;
    server.read = (view) =>
      view === BOOKINGS && reads(BOOKINGS).length === 1
        ? new Promise<Reply>((resolve) => {
            release = resolve;
          })
        : undefined;
    mount();
    await vi.waitFor(() => expect(reads(BOOKINGS)).toHaveLength(1));
    await settle();
    expect(menuText()).toContain('กำลังโหลดการจอง…');
    release({ status: 200, body: { version: 7, changed: true, data: [PENDING] } });
    await vi.waitFor(() => expect(rowsShown()).toHaveLength(1));
    expect(menuText()).not.toContain('กำลังโหลดการจอง…');
  });

  it("shows a refused read's sentence in place of the table (bookings.js:6)", async () => {
    server.read = (view) =>
      view === BOOKINGS
        ? { status: 500, body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } }
        : undefined;
    await opened();
    await vi.waitFor(() => expect(menuText()).toContain('เซิร์ฟเวอร์ขัดข้อง'));
    expect(rowsShown()).toHaveLength(0);
  });
});

// ── the list (K1) ────────────────────────────────────────────────────────────

describe('V2-bookings: the list as v2 shows it (#463 K1)', () => {
  it('shows the toolbar, the search box and the info line (admin-html:19, bookings.js:9)', async () => {
    await opened();
    menuButton('โหลดการจองล่าสุด');
    menuButton('ตั้งค่าการจอง');
    const box = search();
    expect(box.placeholder).toBe('เลขจอง / ชื่อ / เบอร์โทร');
    expect(box.maxLength).toBe(100);
    expect(menuText()).toContain('ลูกค้าใหม่ 30 นาที · เปิดรับจองเมื่อสาขาเปิดกะและตั้งเรทแล้ว · Member ที่ยืนยันตัวตน 2 คน');
  });

  it('says ปิดรับจองใหม่ and the stored minutes when bookings are closed', async () => {
    server.settings = settingsOf({ rules: { enabled: false, minutes: 45 }, memberCount: 0 });
    await opened();
    expect(menuText()).toContain('ลูกค้าใหม่ 45 นาที · ปิดรับจองใหม่ · Member ที่ยืนยันตัวตน 0 คน');
  });

  it("shows v2's headers and one row per booking in the server's order", async () => {
    server.rows = [PENDING, CONFIRMED, EXPIRED, USED, CANCELLED];
    await opened();
    const headers = inMenu<HTMLTableCellElement>('thead th').map((h) => norm(h.textContent));
    expect(headers.slice(0, 5)).toEqual(['เลขจอง / ลูกค้า', 'รายการ', '~เรท', 'หมดอายุ', 'สถานะ']);
    expect(headers).toHaveLength(6);
    const order = ['BK-p1', 'BK-c1', 'BK-e1', 'BK-u1', 'BK-x1'];
    expect(rowsShown().map((r) => order.find((id) => cells(r)[0]!.startsWith(id)))).toEqual(order);
  });

  it("fills each row's cells from the server's strings: money and rates as sent, no arithmetic", async () => {
    await opened();
    const p = cells(rowOf('BK-p1'));
    expect(p[0]).toContain('BK-p1');
    expect(p[0]).toContain('Somchai Pending · 081 234 5678');
    expect(p[0], 'the Member pill of a member booking').toContain('Member');
    expect(p[1]).toContain('ร้านรับซื้อ');
    expect(p[1]).toContain('1,000.00');
    expect(p[1]).toContain('USD');
    expect(p[2]).toBe('36.2500');
    expect(p[3]).toBe('รอพนักงานกำหนด');
    expect(p[4]).toBe('Member · รอยืนยัน');

    const c = cells(rowOf('BK-c1'));
    expect(c[0]).toContain('Anan Walkin · 02 111 2222');
    expect(c[0]).not.toContain('Member');
    expect(c[1]).toContain('ร้านขาย');
    expect(c[1]).toContain('250.50');
    expect(c[1]).toContain('EUR');
    expect(c[2]).toBe('40.1000');
    expect(c[3]).toBe(new Date('2026-10-09T05:30:02.000Z').toLocaleString('th-TH'));
    expect(c[4]).toBe('ล็อกเรทแล้ว');
  });

  it("offers each status v2's actions: confirm and cancel, the desk hand-off and cancel, or nothing", async () => {
    server.rows = [PENDING, CONFIRMED, EXPIRED, USED, CANCELLED];
    await opened();
    const pending = rowOf('BK-p1');
    expect(buttonsIn(pending, 'ยืนยันเรทและเวลา')).toHaveLength(1);
    expect(buttonsIn(pending, 'ยกเลิก')).toHaveLength(1);
    expect(pending.querySelectorAll('a')).toHaveLength(0);

    const confirmed = rowOf('BK-c1');
    expect(buttonsIn(confirmed, 'ยืนยันเรทและเวลา')).toHaveLength(0);
    expect(buttonsIn(confirmed, 'ยกเลิก')).toHaveLength(1);
    const links = [...confirmed.querySelectorAll('a')].filter((a) => norm(a.textContent) === 'ทำรายการหน้าร้าน ↗');
    expect(links).toHaveLength(1);
    expect(links[0]!.getAttribute('href')).toBe('/desk?branch=MAIN&booking=BK-c1');
    expect(links[0]!.getAttribute('target')).toBe('_blank');
    expect(links[0]!.getAttribute('rel') ?? '').toContain('noopener');

    for (const [id, word] of [
      ['BK-e1', 'หมดอายุ'],
      ['BK-u1', 'ทำรายการแล้ว'],
      ['BK-x1', 'ยกเลิก'],
    ] as const) {
      const row = rowOf(id);
      expect(cells(row)[4], id).toBe(word);
      expect(row.querySelectorAll('button'), `${id}: no action`).toHaveLength(0);
      expect(row.querySelectorAll('a'), `${id}: no desk link`).toHaveLength(0);
    }
  });

  it('takes the status from the server and does no time arithmetic (ticket §9 row 3)', async () => {
    server.rows = [
      bookingRow({ id: 'BK-late', status: 'CONFIRMED', expiresAt: PAST }),
      bookingRow({ id: 'BK-early', status: 'EXPIRED', expiresAt: FUTURE }),
    ];
    await opened();
    expect(cells(rowOf('BK-late'))[4]).toBe('ล็อกเรทแล้ว');
    expect([...rowOf('BK-late').querySelectorAll('a')].map((a) => norm(a.textContent))).toEqual([
      'ทำรายการหน้าร้าน ↗',
    ]);
    expect(cells(rowOf('BK-early'))[4]).toBe('หมดอายุ');
    expect(rowOf('BK-early').querySelectorAll('button, a')).toHaveLength(0);
  });

  it("shows v2's empty sentence when the branch has no booking (ticket §9 row 1)", async () => {
    server.rows = [];
    await opened();
    expect(rowsShown()).toHaveLength(0);
    expect(menuText()).toContain('ยังไม่มีการจองที่ตรงกับคำค้นในสาขานี้');
    expect(menuText(), 'bookings have no period').not.toContain('เปลี่ยนช่วงวันที่');
  });

  it('shows the public booking link only when publicUrl is not empty (ticket §9 row 2)', async () => {
    await opened();
    const link = () =>
      inMenu<HTMLAnchorElement>('a').filter((a) => norm(a.textContent) === 'เปิดเว็บจองสำหรับลูกค้า ↗');
    expect(link(), 'v3 answers publicUrl "" — nothing renders').toHaveLength(0);

    server.version = 8;
    server.settings = settingsOf({ publicUrl: 'https://book.example.test/' });
    menuButton('โหลดการจองล่าสุด').click();
    await vi.waitFor(() => expect(link()).toHaveLength(1));
    expect(link()[0]!.getAttribute('href')).toBe('https://book.example.test/');
    expect(link()[0]!.getAttribute('target')).toBe('_blank');
  });
});

// ── search (K2) ──────────────────────────────────────────────────────────────

describe('V2-bookings: search is the server\'s q filter (#463 K2; design §12.1)', () => {
  it('reads admin_bookings with q, trimmed, without a since, and shows the rows the server answered', async () => {
    await opened();
    server.version = 9;
    server.rows = [USED];
    setValue(search(), '  somchai  ');
    await vi.waitFor(() => expect(lastRead(BOOKINGS).query.get('q')).toBe('somchai'));
    const read = lastRead(BOOKINGS);
    expect(read.query.has('since'), 'a new filter is read without the old since').toBe(false);
    expect(read.query.get('branch')).toBe('MAIN');
    await vi.waitFor(() => expect(rowsShown()).toHaveLength(1));
    // the server's answer is shown as it is: the screen filters nothing itself
    rowOf('BK-u1');
  });

  it('keeps q on a refresh with the since of the q read; clearing q reads everything without a since', async () => {
    await opened();
    server.version = 9;
    setValue(search(), 'suda');
    await vi.waitFor(() => expect(lastRead(BOOKINGS).query.get('q')).toBe('suda'));
    await settle();
    const count = reads(BOOKINGS).length;

    menuButton('โหลดการจองล่าสุด').click();
    await vi.waitFor(() => expect(reads(BOOKINGS).length).toBeGreaterThan(count));
    expect(lastRead(BOOKINGS).query.get('q')).toBe('suda');
    expect(lastRead(BOOKINGS).query.get('since')).toBe('9');

    setValue(search(), '');
    await vi.waitFor(() => expect(lastRead(BOOKINGS).query.has('q')).toBe(false));
    expect(lastRead(BOOKINGS).query.has('since')).toBe(false);
  });
});

// ── confirm (K1, design §12.14) ──────────────────────────────────────────────

describe('V2-bookings: confirming a member booking (#463 K1; bookings.js:12)', () => {
  it("shows the server's rate and principal, and sends the rate back with the expiry as ISO", async () => {
    await opened();
    buttonsIn(rowOf('BK-p1'), 'ยืนยันเรทและเวลา')[0]!.click();
    await settle();
    const d = openDialogEl();
    expect(norm(d.querySelector('h2')?.textContent)).toBe('ยืนยันการจอง Member');
    expect(norm(d.textContent)).toContain('Somchai Pending · 1,000.00 USD');
    expect(norm(d.textContent)).toContain('เรทที่จะล็อก: 36.5000 · ยอดเงินต้น 11,111.11 THB');
    const expiry = field(d, 'วันเวลาหมดอายุ (เวลาของเครื่องนี้)') as HTMLInputElement;
    expect(expiry.type).toBe('datetime-local');
    expect(expiry.required).toBe(true);
    dialogButton('กลับ');

    setValue(expiry, '2026-10-10T18:30');
    dialogButton('ยืนยันล็อกเรท').click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
    const post = posts()[0]!;
    expect(post.path).toBe('/api/bookings');
    expect(post.headers['x-xchange-request']).toBe('1');
    expect(post.body).toEqual({
      action: 'confirm',
      payload: {
        branchId: 'MAIN',
        id: 'BK-p1',
        expiresAt: new Date('2026-10-10T18:30').toISOString(),
        expectedRate: '36.5000',
      },
    });
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(reads(BOOKINGS).length).toBeGreaterThan(1));
    expect(lastRead(BOOKINGS).query.get('since')).toBe('7');
  });

  it('says ยังไม่ตั้งเรท and — when the branch has no counter rate for it', async () => {
    server.rows = [{ ...PENDING, counterRate: null, counterPrincipal: null }];
    await opened();
    buttonsIn(rowOf('BK-p1'), 'ยืนยันเรทและเวลา')[0]!.click();
    await settle();
    expect(norm(openDialogEl().textContent)).toContain('เรทที่จะล็อก: ยังไม่ตั้งเรท · ยอดเงินต้น — THB');
  });

  it("shows a refusal's sentence in the dialog and lets the owner send again", async () => {
    server.post = () => ({ status: 400, body: { error: 'เรทเปลี่ยน กรุณาโหลดใหม่' } });
    await opened();
    buttonsIn(rowOf('BK-p1'), 'ยืนยันเรทและเวลา')[0]!.click();
    await settle();
    setValue(field(openDialogEl(), 'วันเวลาหมดอายุ (เวลาของเครื่องนี้)'), '2026-10-10T18:30');
    dialogButton('ยืนยันล็อกเรท').click();
    await vi.waitFor(() => expect(norm(openDialogEl().textContent)).toContain('เรทเปลี่ยน กรุณาโหลดใหม่'));
    expect(dialogButton('ยืนยันล็อกเรท').disabled).toBe(false);
    dialogButton('ยืนยันล็อกเรท').click();
    await vi.waitFor(() => expect(posts()).toHaveLength(2));
  });
});

// ── cancel (K1) ──────────────────────────────────────────────────────────────

describe('V2-bookings: cancelling (#463 K1; bookings.js:18)', () => {
  it('asks first; กลับ sends nothing; ยืนยันยกเลิก cancels and re-reads', async () => {
    await opened();
    buttonsIn(rowOf('BK-c1'), 'ยกเลิก')[0]!.click();
    await settle();
    let d = openDialogEl();
    expect(norm(d.querySelector('h2')?.textContent)).toBe('ยกเลิกการจอง');
    expect(norm(d.textContent)).toContain('ยกเลิก BK-c1 และยุติการล็อกเรทของการจองนี้?');
    dialogButton('กลับ').click();
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    expect(posts()).toHaveLength(0);

    buttonsIn(rowOf('BK-c1'), 'ยกเลิก')[0]!.click();
    await settle();
    d = openDialogEl();
    dialogButton('ยืนยันยกเลิก').click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
    expect(posts()[0]!.body).toEqual({ action: 'cancel', payload: { branchId: 'MAIN', id: 'BK-c1' } });
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(reads(BOOKINGS).length).toBeGreaterThan(1));
  });

  it("toasts a refusal's sentence", async () => {
    server.post = () => ({ status: 400, body: { error: 'การจองนี้ถูกใช้ทำรายการแล้ว' } });
    await opened();
    buttonsIn(rowOf('BK-c1'), 'ยกเลิก')[0]!.click();
    await settle();
    dialogButton('ยืนยันยกเลิก').click();
    await vi.waitFor(() => expect(toastText()).toBe('การจองนี้ถูกใช้ทำรายการแล้ว'));
  });
});

// ── settings (K1) ────────────────────────────────────────────────────────────

describe('V2-bookings: the booking settings (#463 K1; bookings.js:16)', () => {
  it("shows the branch's rules and saves minutes as a number and enabled as a boolean", async () => {
    await opened();
    menuButton('ตั้งค่าการจอง').click();
    await vi.waitFor(() =>
      expect(norm(openDialogEl().querySelector('h2')?.textContent)).toBe('ตั้งค่าการจองสาขา XC-B'),
    );
    const d = openDialogEl();
    const minutes = field(d, 'ล็อกเรทลูกค้าใหม่ (นาที)') as HTMLInputElement;
    expect(minutes.type).toBe('number');
    expect(minutes.min).toBe('5');
    expect(minutes.max).toBe('1440');
    expect(minutes.required).toBe(true);
    expect(minutes.value).toBe('30');
    const enabled = field(d, 'เปิดรับจองเมื่อมีกะเปิดและเรทพร้อม') as HTMLInputElement;
    expect(enabled.type).toBe('checkbox');
    expect(enabled.checked).toBe(true);
    expect(norm(d.textContent)).toContain('Member ให้พนักงานกำหนดเวลาหมดอายุแต่ละรายการ');

    setValue(minutes, '45');
    enabled.click();
    dialogButton('บันทึก').click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
    expect(posts()[0]!.body).toEqual({
      action: 'settings',
      payload: { branchId: 'MAIN', minutes: 45, enabled: false },
    });
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(reads(SETTINGS).length).toBeGreaterThan(1));
  });

  it("names the header's branch in the title, from the shell's admin_branches", async () => {
    const { setBranch } = await opened();
    setBranch('AIRPORT');
    await vi.waitFor(() => expect(lastRead(BOOKINGS).query.get('branch')).toBe('AIRPORT'));
    await settle();
    menuButton('ตั้งค่าการจอง').click();
    await vi.waitFor(() =>
      expect(norm(openDialogEl().querySelector('h2')?.textContent)).toBe('ตั้งค่าการจองสาขา Airport'),
    );
    expect(reads(BRANCHES).length, 'the names come from admin_branches').toBeGreaterThan(0);
  });

  it("shows a refusal's sentence in the dialog", async () => {
    server.post = () => ({ status: 400, body: { error: 'กำหนดเวลา 5–1440 นาที' } });
    await opened();
    menuButton('ตั้งค่าการจอง').click();
    await settle();
    dialogButton('บันทึก').click();
    await vi.waitFor(() => expect(norm(openDialogEl().textContent)).toContain('กำหนดเวลา 5–1440 นาที'));
  });
});

// ── create (K3, design §12.17) ───────────────────────────────────────────────

function createButton(): HTMLButtonElement {
  const found = inMenu<HTMLButtonElement>('button').filter((b) => norm(b.textContent).includes('สร้างการจอง'));
  expect(found.length, 'one create button in the list header').toBe(1);
  return found[0]!;
}

const FIELD_ORDER = ['ชื่อ', 'เบอร์ติดต่อ', 'ประเภท', 'สกุลเงิน', 'จำนวนเงิน', 'เรท', 'รหัสลูกค้า'];

async function openCreate(): Promise<HTMLDialogElement> {
  createButton().click();
  await settle();
  const d = openDialogEl();
  expect(norm(d.querySelector('h2')?.textContent)).toContain('สร้างการจอง');
  return d;
}

function submitButton(d: HTMLDialogElement): HTMLButtonElement {
  const found = [...d.querySelectorAll('button')].filter(
    (b) => b.type === 'submit' && norm(b.textContent).includes('สร้างการจอง') && visible(b),
  );
  expect(found.length, 'one submit button').toBe(1);
  return found[0]!;
}

function fill(d: HTMLDialogElement, values: Partial<Record<string, string>>): void {
  for (const [label, value] of Object.entries(values)) {
    setValue(field(d, label), value!);
  }
}

describe('V2-bookings: creating a booking, a v3 addition (#463 K3; design §12.17)', () => {
  it('opens from the list header a dialog whose fields are the create payload in its order', async () => {
    await opened();
    menuButton('โหลดการจองล่าสุด');
    const d = await openCreate();
    const shown = labelsOf(d)
      .map(labelText)
      .filter((t) => FIELD_ORDER.includes(t));
    expect(shown).toEqual(FIELD_ORDER);
    const type = field(d, 'ประเภท') as HTMLSelectElement;
    expect([...type.options].map((o) => [o.value, norm(o.textContent)])).toEqual([
      ['BUY', 'ร้านรับซื้อ'],
      ['SELL', 'ร้านขาย'],
    ]);
    const currency = field(d, 'สกุลเงิน') as HTMLSelectElement;
    expect([...currency.options].map((o) => o.value)).toEqual(['EUR', 'USD']);
    expect((field(d, 'เรท') as HTMLInputElement).readOnly).toBe(true);
  });

  it("shows the server's counter rate for the chosen currency and side, unchanged", async () => {
    await opened();
    const d = await openCreate();
    const rate = () => (field(d, 'เรท') as HTMLInputElement).value;
    fill(d, { ประเภท: 'BUY', สกุลเงิน: 'USD' });
    await settle();
    expect(rate()).toBe('36.2500');
    fill(d, { ประเภท: 'SELL' });
    await settle();
    expect(rate()).toBe('36.7500');
    fill(d, { สกุลเงิน: 'EUR' });
    await settle();
    expect(rate()).toBe('40.1000');
    fill(d, { ประเภท: 'BUY' });
    await settle();
    expect(rate()).toBe('39.5000');
  });

  it('sends exactly the create payload — the amount as typed, the rate unchanged — then closes and re-reads', async () => {
    vi.spyOn(crypto, 'randomUUID').mockReturnValue('aaaaaaaa-0000-4000-8000-000000000001');
    server.post = (action, payload) => ({
      status: 200,
      body: { saved: true, booking: { ...CONFIRMED, id: 'BK-new', action, payload } },
    });
    await opened();
    const d = await openCreate();
    fill(d, {
      ชื่อ: 'Somchai Jaidee',
      เบอร์ติดต่อ: '081 234 5678',
      ประเภท: 'SELL',
      สกุลเงิน: 'EUR',
      จำนวนเงิน: '250.5',
      รหัสลูกค้า: 'C-MAIN-1',
    });
    await settle();
    submitButton(d).click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
    const post = posts()[0]!;
    expect(post.path).toBe('/api/bookings');
    expect(post.body).toEqual({
      action: 'create',
      payload: {
        branchId: 'MAIN',
        requestId: 'aaaaaaaa-0000-4000-8000-000000000001',
        name: 'Somchai Jaidee',
        phone: '081 234 5678',
        type: 'SELL',
        currency: 'EUR',
        amount: '250.5',
        expectedRate: '40.1000',
        customerId: 'C-MAIN-1',
      },
    });
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(reads(BOOKINGS).length).toBeGreaterThan(1));
    expect(lastRead(BOOKINGS).query.get('since')).toBe('7');
  });

  it('leaves customerId out, or empty, when none is typed (a walk-in)', async () => {
    await opened();
    const d = await openCreate();
    fill(d, { ชื่อ: 'Walk In', เบอร์ติดต่อ: '081 234 5678', ประเภท: 'BUY', สกุลเงิน: 'USD', จำนวนเงิน: '100' });
    await settle();
    submitButton(d).click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
    const payload = posts()[0]!.body!.payload!;
    expect([undefined, '']).toContain(payload['customerId']);
    expect(payload['expectedRate']).toBe('36.2500');
    expect(payload['amount']).toBe('100');
  });

  it("checks before sending, with the server's own sentences, and sends nothing then", async () => {
    await opened();
    const good = { ชื่อ: 'Walk In', เบอร์ติดต่อ: '081 234 5678', ประเภท: 'BUY', สกุลเงิน: 'USD', จำนวนเงิน: '100' };
    const cases: [Partial<Record<string, string>>, string][] = [
      [{ ชื่อ: '' }, 'กรอกข้อมูลให้ครบและไม่เกินกำหนด'],
      [{ ชื่อ: 'ก'.repeat(101) }, 'กรอกข้อมูลให้ครบและไม่เกินกำหนด'],
      [{ เบอร์ติดต่อ: 'call me' }, 'เบอร์ติดต่อไม่ถูกต้อง'],
      [{ เบอร์ติดต่อ: '12-34-5' }, 'เบอร์ติดต่อไม่ถูกต้อง'],
      [{ จำนวนเงิน: '' }, 'จำนวนเงินหรือเรทไม่ถูกต้อง'],
      [{ จำนวนเงิน: '0' }, 'จำนวนเงินหรือเรทไม่ถูกต้อง'],
      [{ จำนวนเงิน: '0.00' }, 'จำนวนเงินหรือเรทไม่ถูกต้อง'],
    ];
    for (const [bad, sentence] of cases) {
      closeDialog();
      await settle();
      const d = await openCreate();
      fill(d, { ...good, ...bad });
      await settle();
      submitButton(d).click();
      await settle();
      expect(norm(openDialogEl().textContent), JSON.stringify(bad)).toContain(sentence);
      expect(posts(), JSON.stringify(bad)).toHaveLength(0);
    }
    closeDialog();
    await settle();
    const d = await openCreate();
    fill(d, good);
    await settle();
    submitButton(d).click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
  });

  it("keeps one requestId for every retry of a dialog, makes a new one for the next, and shows the server's refusal", async () => {
    let n = 0;
    vi.spyOn(crypto, 'randomUUID').mockImplementation(
      () => `bbbbbbbb-0000-4000-8000-${String(++n).padStart(12, '0')}` as `${string}-${string}-${string}-${string}-${string}`,
    );
    server.post = () => ({ status: 400, body: { error: 'เรทเปลี่ยน กรุณาโหลดใหม่' } });
    await opened();
    const good = { ชื่อ: 'Walk In', เบอร์ติดต่อ: '081 234 5678', ประเภท: 'BUY', สกุลเงิน: 'USD', จำนวนเงิน: '100' };

    let d = await openCreate();
    fill(d, good);
    await settle();
    submitButton(d).click();
    await vi.waitFor(() => expect(posts()).toHaveLength(1));
    await vi.waitFor(() => expect(norm(openDialogEl().textContent)).toContain('เรทเปลี่ยน กรุณาโหลดใหม่'));
    submitButton(openDialogEl()).click();
    await vi.waitFor(() => expect(posts()).toHaveLength(2));
    const first = posts()[0]!.body!.payload!['requestId'];
    expect(first).toMatch(/^bbbbbbbb-/);
    expect(posts()[1]!.body!.payload!['requestId'], 'a retry resends the same request').toBe(first);

    closeDialog();
    await settle();
    d = await openCreate();
    fill(d, good);
    await settle();
    submitButton(d).click();
    await vi.waitFor(() => expect(posts()).toHaveLength(3));
    const second = posts()[2]!.body!.payload!['requestId'];
    expect(second).toMatch(/^bbbbbbbb-/);
    expect(second, 'a new dialog is a new request').not.toBe(first);
  });
});

// ── text (W10) ───────────────────────────────────────────────────────────────

describe('V2-bookings: every word through the menu table (#463 "Builds on" W10)', () => {
  it('switches the toolbar, headers and statuses to English with the language', async () => {
    const t = loadedTable['./i18n.ts'];
    expect(t, 'web/src/screens/admin/bookings/i18n.ts exists').toBeDefined();
    const { th, en } = t!;
    const keyOf = (thai: string) => {
      const k = Object.keys(th).find((key) => th[key] === thai);
      expect(k, `${thai} is a value of th`).toBeDefined();
      return k!;
    };
    await opened();
    setLang('en');
    await settle();
    menuButton(en[keyOf('โหลดการจองล่าสุด')]!);
    menuButton(en[keyOf('ตั้งค่าการจอง')]!);
    const headers = inMenu<HTMLTableCellElement>('thead th').map((h) => norm(h.textContent));
    expect(headers[0]).toBe(en[keyOf('เลขจอง / ลูกค้า')]);
    expect(cells(rowOf('BK-c1'))[4]).toBe(en[keyOf('ล็อกเรทแล้ว')]);
    expect(menuText()).not.toMatch(THAI);
  });
});
