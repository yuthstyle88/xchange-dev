// V2-rates (#468): the rates menu's screen — R1 the table, R2 the LIVE edit dialog with its BOT
// line, R3 the BOT panel and its 60 s refresh, R4 the history dialog, R5 the link to the kiosk
// currency page (ticket §7–§10; design §12.1, §12.2, §12.6, §12.14, §12.16, W6, W7, W10).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete
// or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; `admin-html:N` = line N of the decoded admin page, ticket §2):
// the rates section (`:19`, LIVE wording `:100`) and table (`:68`), the LIVE edit dialog (`:134`),
// `src/online/public/rates-reference.js` (`:5` the BOT line, `:7` the panel, `:8` the history,
// `:10` the form's BOT line and partner fields, `:11`–`:12` the reads), and the tool link of
// `src/devices-public/admin-workspace.js:136-138`. Every figure is the server's (W7): the screen
// shows the decimal strings of `admin_rates`, `rates_reference` and `admin_rate_history` as they
// come — the spreads included — and computes none. Times are v2's
// `new Date(x).toLocaleString('th-TH', {timeZone: 'Asia/Bangkok'})`.
//
// The menu is mounted as the shell mounts it: its `Component` with `branch()`, `period()` and
// `shown()` (design §12.3), beside the page's one `Dialog` and `Toast` (V2-parts). The menu builds
// its own `new ApiClient()` (controller's decision for every V2 menu, #462), so the fake API is
// the global `fetch`: it answers the views by name and records every request. The two page-wide
// reads AppShell makes (`/api/v3/views/me`, `/api/v3/views/kiosk_notifications`) are answered and
// never counted as the menu's own.
//
// The menu is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { th as partsTh } from '../parts/i18n';
import { requestRefresh } from '../parts/refresh';
import { Toast, toast } from '../parts/toast';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/rates/menu.ts exists').toBeDefined();
  return m!.menu;
}

const RATES = '/api/v3/views/admin_rates';
const REFERENCE = '/api/v3/views/rates_reference';
const HISTORY = '/api/v3/views/admin_rate_history';
const BRANCHES = '/api/v3/views/admin_branches';
const RPC = '/api/rpc';
/** AppShell's page-wide reads: answered, never the menu's own requests. */
const PAGE_WIDE = ['/api/v3/views/me', '/api/v3/views/kiosk_notifications'];

const BANK_DOWN = 'ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท';
const OWNER_ONLY = 'เฉพาะเจ้าของระบบเท่านั้น';
const SELL_BELOW_BUY = 'เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ';
const PARTNER_PAIR = 'กรอกชื่อคู่ค้าและเรทต้นทุนให้ครบคู่';
const JPY_NOTE = 'แปลงจากต่อ 100 เยนเป็นต่อ 1 เยนแล้ว';

/** v2's `rateTime` (`rates-reference.js:4`, `admin-html:68`). */
function time(iso: string): string {
  return new Date(iso).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
}

// ── the answers ──────────────────────────────────────────────────────────────

interface RateRow {
  code: string;
  name: string;
  symbol: string;
  buy: string | null;
  sell: string | null;
  spread: string | null;
  updated: string;
  partnerName: string;
  partnerCost: string | null;
  partnerSpread: string | null;
}

interface RatesData {
  updated: string;
  rates: RateRow[];
}

interface RefRow {
  code: string;
  date: string;
  unit: number;
  buyingSight: string;
  buyingTransfer: string;
  selling: string;
}

interface RefData {
  rows: RefRow[];
  fetchedAt: string;
  error: string | null;
}

interface HistoryRow {
  id: string;
  code: string;
  buy: string;
  sell: string;
  updated: string;
  actor: string;
  partnerName: string;
  partnerCost: string | null;
  botReference: { fetchedAt: string; rows: RefRow[] } | null;
}

/**
 * Figures chosen so that nothing the screen could compute matches what it must show: USD's
 * `spread` is not sell − buy and its `partnerSpread` not sell − partnerCost.
 */
function usd(over: Partial<RateRow> = {}): RateRow {
  return {
    code: 'USD',
    name: 'ดอลลาร์สหรัฐ',
    symbol: '$',
    buy: '33.2500',
    sell: '34.1000',
    spread: '9.9999',
    updated: '2026-10-06T05:00:01.000Z',
    partnerName: 'SuperRich',
    partnerCost: '33.9000',
    partnerSpread: '8.8888',
    ...over,
  };
}

function ratesData(over: Partial<RatesData> = {}): RatesData {
  return {
    updated: '2026-10-06T05:00:03.000Z',
    rates: [
      usd(),
      {
        code: 'EUR',
        name: 'ยูโร',
        symbol: '€',
        buy: '37.3552',
        sell: '38.0844',
        spread: '0.7292',
        updated: '2026-10-06T05:00:02.000Z',
        partnerName: '',
        partnerCost: null,
        partnerSpread: null,
      },
      {
        code: 'JPY',
        name: 'เยนญี่ปุ่น',
        symbol: '¥',
        buy: '0.2095',
        sell: '0.2163',
        spread: '0.0068',
        updated: '2026-10-06T05:00:03.000Z',
        partnerName: '',
        partnerCost: null,
        partnerSpread: null,
      },
      {
        code: 'KRW',
        name: 'วอนเกาหลีใต้',
        symbol: '',
        buy: null,
        sell: null,
        spread: null,
        updated: '',
        partnerName: '',
        partnerCost: null,
        partnerSpread: null,
      },
    ],
    ...over,
  };
}

function refRow(code: string, unit: number, s: string, t: string, l: string): RefRow {
  return { code, date: '2026-10-06', unit, buyingSight: s, buyingTransfer: t, selling: l };
}

function refData(over: Partial<RefData> = {}): RefData {
  return {
    rows: [
      refRow('USD', 1, '33.4218', '33.5008', '33.8278'),
      refRow('GBP', 1, '44.0195', '44.1299', '44.9090'),
      refRow('JPY', 100, '0.2095', '0.2101', '0.2163'),
    ],
    fetchedAt: '2026-10-06T04:30:00.000Z',
    error: null,
    ...over,
  };
}

/** v2's `botSummary` first line (`rates-reference.js:5`), for a row the screen shows as is. */
function botLine(r: RefRow): string {
  return `ธปท. วันที่ ${r.date} · ซื้อตั๋วเงิน ${r.buyingSight} · ซื้อเงินโอน ${r.buyingTransfer} · ขาย ${r.selling}`;
}

function historyRow(i: number, over: Partial<HistoryRow> = {}): HistoryRow {
  return {
    id: String(1000 - i),
    code: 'USD',
    buy: `${30 + i}.0000`,
    sell: `${31 + i}.0000`,
    updated: `2026-10-06T0${i % 10}:00:00.000Z`,
    actor: `staff${i}`,
    partnerName: '',
    partnerCost: null,
    botReference: null,
    ...over,
  };
}

// ── the fake API: the global fetch ──────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  body: unknown;
}

type Reply = { status: number; body: unknown };

let sent: Sent[];
let rates: Record<string, { version: number; data: RatesData }>;
let reference: { version: number; data: RefData };
let referenceRefusal: { status: number; code: string; message: string } | null;
let referenceGate: Promise<void> | null;
let history: Record<string, HistoryRow[]>;
let rpc: (body: { action: string; payload: Record<string, unknown> }) => Reply;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function answer(path: string, p: URLSearchParams, body: unknown): Promise<Reply> {
  const since = p.get('since');
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
      },
    };
  }
  if (path === '/api/v3/views/kiosk_notifications') {
    return { status: 200, body: { version: 1, changed: false } };
  }
  if (path === RATES) {
    const branch = p.get('branch') ?? 'MAIN';
    const entry = rates[branch];
    if (!entry) return { status: 400, body: { error: { code: 'noSuchBranch', message: 'ไม่พบสาขาที่เลือก กรุณารีเฟรช' } } };
    if (since !== null && Number(since) === entry.version) {
      return { status: 200, body: { version: entry.version, changed: false } };
    }
    return { status: 200, body: { version: entry.version, changed: true, data: structuredClone(entry.data) } };
  }
  if (path === REFERENCE) {
    if (referenceGate) await referenceGate;
    if (referenceRefusal) {
      return {
        status: referenceRefusal.status,
        body: { error: { code: referenceRefusal.code, message: referenceRefusal.message } },
      };
    }
    if (since !== null && Number(since) === reference.version) {
      return { status: 200, body: { version: reference.version, changed: false } };
    }
    return { status: 200, body: { version: reference.version, changed: true, data: structuredClone(reference.data) } };
  }
  if (path === HISTORY) {
    const rows = history[p.get('branch') ?? 'MAIN'] ?? [];
    const before = p.get('before');
    const start = before === null ? 0 : rows.findIndex((r) => r.id === before) + 1;
    const limit = Number(p.get('limit') ?? '50');
    return { status: 200, body: { version: 1, changed: true, data: structuredClone(rows.slice(start, start + limit)) } };
  }
  if (path === BRANCHES) {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { branches: [{ id: 'MAIN', name: 'สาขาหลัก' }, { id: 'B9', name: 'Nine' }] },
      },
    };
  }
  if (path === RPC) {
    return rpc(body as { action: string; payload: Record<string, unknown> });
  }
  return { status: 500, body: { error: { code: 'serverError', message: `unexpected ${path}` } } };
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
  sent.push({ path: url.pathname, params: url.searchParams, method, body });
  const reply = await answer(url.pathname, url.searchParams, body);
  return json(reply.status, reply.body);
}

/** The requests the menu made itself. */
function own(): Sent[] {
  return sent.filter((s) => !PAGE_WIDE.includes(s.path));
}

function readsOf(path: string): Sent[] {
  return sent.filter((s) => s.path === path);
}

function posts(): Sent[] {
  return sent.filter((s) => s.method === 'POST');
}

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    if (vi.isFakeTimers()) {
      await vi.advanceTimersByTimeAsync(1);
    } else {
      await new Promise((r) => setTimeout(r, 0));
    }
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;
let hidden = false;

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
  setShown: (s: boolean) => void;
}

function mount(init: { branch?: string; shown?: boolean } = {}): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown, setShown] = createSignal(init.shown ?? true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    el,
  );
  return { el, setBranch, setShown };
}

function setHidden(value: boolean): void {
  hidden = value;
  document.dispatchEvent(new Event('visibilitychange'));
}

beforeEach(() => {
  sent = [];
  rates = {
    MAIN: { version: 7, data: ratesData() },
    // the same version number as MAIN: a since carried over from MAIN would hide B9's rates
    B9: {
      version: 7,
      data: ratesData({ rates: [usd({ buy: '31.0000', sell: '32.0000', spread: '1.0000' })] }),
    },
  };
  reference = { version: 3, data: refData() };
  referenceRefusal = null;
  referenceGate = null;
  history = { MAIN: [] };
  rpc = () => ({ status: 200, body: { version: 1 } });
  hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast(''); // the page's one toast is module state: clear what an earlier test left
  location.hash = '#rates';
  setLang('th');
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
  hidden = false;
  location.hash = '';
  setLang('th');
  await settle();
});

// ── helpers on the DOM ───────────────────────────────────────────────────────

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function buttonsNamed(el: ParentNode, name: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = buttonsNamed(el, name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

/** The table whose header cells start with `first`. */
function tableStarting(el: ParentNode, first: string): HTMLTableElement {
  const found = [...el.querySelectorAll<HTMLTableElement>('table')].filter(
    (t) => text(t.querySelector('th')) === first,
  );
  expect(found.length, `one table whose first header is ${first}`).toBe(1);
  return found[0]!;
}

function headers(t: HTMLTableElement): string[] {
  return [...t.querySelectorAll('thead th')].map((th) => text(th)).filter((h) => h !== '');
}

function rows(t: HTMLTableElement): HTMLTableRowElement[] {
  return [...t.querySelectorAll<HTMLTableRowElement>('tbody tr')];
}

function cells(r: HTMLTableRowElement): string[] {
  return [...r.querySelectorAll('td')].map((td) => text(td));
}

function rowOf(t: HTMLTableElement, code: string): HTMLTableRowElement {
  const found = rows(t).filter((r) => text(r.querySelector('td')).includes(code));
  expect(found.length, `one row of ${code}`).toBe(1);
  return found[0]!;
}

function theDialog(): HTMLDialogElement {
  const d = document.querySelector('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

function dialogOpen(): boolean {
  const d = document.querySelector('dialog');
  return !!d && d.hasAttribute('open');
}

/** The control a `<label>` starting with `label` names. */
function field(el: ParentNode, label: string): HTMLInputElement {
  const found = [...el.querySelectorAll('label')].filter((l) => text(l).startsWith(label));
  expect(found.length, `one label starting ${JSON.stringify(label)}`).toBe(1);
  const control = (found[0] as HTMLLabelElement).control as HTMLInputElement | null;
  expect(control, `the control of ${label}`).not.toBeNull();
  return control!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function toastText(): string | null {
  const status = document.querySelector<HTMLElement>('.toast[role="status"]');
  if (!status || status.hidden) return null;
  const t = text(status);
  return t === '' ? null : t;
}

async function openEdit(el: HTMLElement, code: string): Promise<HTMLDialogElement> {
  const b = el.querySelector<HTMLButtonElement>(`button[aria-label="แก้ไขเรท ${code}"]`);
  expect(b, `the edit button of ${code}`).not.toBeNull();
  b!.click();
  await settle();
  expect(dialogOpen(), 'the dialog opened').toBe(true);
  return theDialog();
}

// ── when it reads ────────────────────────────────────────────────────────────

describe('V2-rates: what it reads, and when (#468 §7; design §12.1, §12.6, §12.16)', () => {
  it('on show: one read of admin_rates and one of rates_reference, neither with since, and nothing else', async () => {
    mount({ branch: 'B9' });
    await settle();
    expect(readsOf(RATES).length).toBe(1);
    expect(readsOf(RATES)[0]!.params.get('branch')).toBe('B9');
    expect(readsOf(RATES)[0]!.params.has('since'), 'the first read carries no since').toBe(false);
    expect(readsOf(REFERENCE).length, 'rates-reference.js:11: the panel reads when shown').toBe(1);
    expect(readsOf(REFERENCE)[0]!.params.has('since')).toBe(false);
    expect(readsOf(REFERENCE)[0]!.params.has('fresh'), 'not a forced fetch').toBe(false);
    expect(own().every((s) => s.path === RATES || s.path === REFERENCE), JSON.stringify(own().map((s) => s.path))).toBe(true);
    expect(own().every((s) => s.method === 'GET'), 'the menu writes nothing on show').toBe(true);
  });

  it('reads nothing while it is not shown, and reads when it becomes shown', async () => {
    const m = mount({ shown: false });
    await settle();
    expect(own().length).toBe(0);
    m.setShown(true);
    await settle();
    expect(readsOf(RATES).length).toBe(1);
    expect(readsOf(REFERENCE).length).toBe(1);
  });

  it('a branch change reads admin_rates again without since; the refresh tick reads with the since it holds', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    m.setBranch('B9');
    await settle();
    expect(readsOf(RATES).length).toBe(2);
    expect(readsOf(RATES)[1]!.params.get('branch')).toBe('B9');
    expect(readsOf(RATES)[1]!.params.has('since'), 'another branch: no since').toBe(false);
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(cells(rowOf(t, 'USD'))[1], "B9's rate").toBe('31.0000');

    requestRefresh();
    await settle();
    expect(readsOf(RATES).length).toBe(3);
    expect(readsOf(RATES)[2]!.params.get('since')).toBe('7');
    expect(readsOf(RATES)[2]!.params.get('branch')).toBe('B9');
  });

  it('changed: false keeps what is shown', async () => {
    const m = mount();
    await settle();
    requestRefresh();
    await settle();
    expect(readsOf(RATES).length).toBe(2);
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(cells(rowOf(t, 'USD'))[1]).toBe('33.2500');
    expect(rows(t).length).toBe(4);
  });
});

// ── R1: the table ────────────────────────────────────────────────────────────

describe("V2-rates R1: the rate table shows the server's figures as they come (admin-html:19, :68; W7)", () => {
  it("one row per currency in the answer's order: symbol, code, name, buy, sell, the server's spread, edit", async () => {
    const m = mount();
    await settle();
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(headers(t)).toEqual(['สกุลเงิน', 'ร้านรับซื้อ', 'ร้านขาย', 'ส่วนต่างเรท']);
    expect(rows(t).map((r) => text(r.querySelector('td')))).toEqual([
      expect.stringContaining('USD'),
      expect.stringContaining('EUR'),
      expect.stringContaining('JPY'),
      expect.stringContaining('KRW'),
    ]);

    const usdRow = rowOf(t, 'USD');
    const first = usdRow.querySelector('td')!;
    expect(text(first)).toContain('ดอลลาร์สหรัฐ');
    const symbol = [...first.querySelectorAll('[aria-hidden="true"]')].map((e) => text(e));
    expect(symbol, "the symbol is shown, hidden from screen readers").toContain('$');
    expect(cells(usdRow).slice(1, 4), "spread is the server's 9.9999, not sell − buy").toEqual(['33.2500', '34.1000', '9.9999']);
    const edit = usdRow.querySelector('button');
    expect(text(edit)).toBe('แก้ไขเรท');
    expect(edit!.getAttribute('aria-label')).toBe('แก้ไขเรท USD');

    expect(cells(rowOf(t, 'JPY')).slice(1, 4)).toEqual(['0.2095', '0.2163', '0.0068']);
  });

  it('a currency with no rate shows ยังไม่ตั้ง and its spread — (ticket §9)', async () => {
    const m = mount();
    await settle();
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(cells(rowOf(t, 'KRW')).slice(1, 4)).toEqual(['ยังไม่ตั้ง', 'ยังไม่ตั้ง', '—']);
  });

  it("the section's LIVE note, heading, pill and the last-edited line", async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('เรทหน้าร้านจาก เซิร์ฟเวอร์ · THB ต่อ 1 หน่วยเงินต่างประเทศ');
    expect(all).toContain('EXCHANGE RATES');
    expect(all).toContain('เรทหน้าร้าน');
    expect(all).toContain('เรทที่ผู้ดูแลกำหนด');
    expect(all).toContain(`แก้ไขล่าสุด ${time('2026-10-06T05:00:03.000Z')}`);
  });
});

// ── R5: the kiosk currency page ──────────────────────────────────────────────

describe('V2-rates R5: the link to the kiosk currency page (admin-workspace.js:136-138)', () => {
  it('carries the picked branch and follows a change of it', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(text(m.el)).toContain('กำหนดสกุลเงินที่ลูกค้าเลือกได้ พร้อมเทียบเรทกสิกร');
    const link = (): HTMLAnchorElement => {
      const found = [...m.el.querySelectorAll('a')].filter((a) => text(a).includes('จัดการสกุลเงินหน้าร้าน →'));
      expect(found.length, 'one link to the currency page').toBe(1);
      return found[0]!;
    };
    expect(link().getAttribute('href')).toBe('/kiosk-currencies.html?branch=MAIN');
    const icon = link().querySelector('[aria-hidden="true"]');
    expect(text(icon), 'the icon, hidden from screen readers').toBe('◎');
    m.setBranch('B9');
    await settle();
    expect(link().getAttribute('href')).toBe('/kiosk-currencies.html?branch=B9');
  });
});

// ── R2: the LIVE edit dialog ─────────────────────────────────────────────────

describe("V2-rates R2: the LIVE edit dialog, its BOT line and the save (admin-html:134; rates-reference.js:10)", () => {
  it("opens as v2's: title, note, the BOT line, the fields filled from the row, and re-reads the reference", async () => {
    const m = mount();
    await settle();
    const before = readsOf(REFERENCE).length;
    const d = await openEdit(m.el, 'USD');
    expect(text(d.querySelector('h2'))).toBe('ตั้งเรทจริง USD');
    const body = text(d);
    expect(body).toContain('ใช้กับรายการใหม่ของหน้าทำรายการ การแก้ไขจะบันทึกผู้แก้และเวลาใน เซิร์ฟเวอร์');
    expect(body).toContain(botLine(refRow('USD', 1, '33.4218', '33.5008', '33.8278')));
    expect(body).toContain(`ดึงข้อมูล ${time('2026-10-06T04:30:00.000Z')}`);
    expect(body).not.toContain(JPY_NOTE);
    expect(body).toContain('ระบบจะดึงและบันทึกเรท ธปท. ใหม่อีกครั้งตอนกดบันทึก หากดึงไม่ได้จะยังไม่ปรับเรทร้าน');
    expect(readsOf(REFERENCE).length, 'v2 refreshes the BOT when the form opens').toBe(before + 1);

    const buy = field(d, 'ร้านรับซื้อ');
    const sell = field(d, 'ร้านขาย');
    const name = field(d, 'ชื่อคู่ค้า');
    const cost = field(d, 'เรทที่ร้านรับซื้อจากคู่ค้า (THB ต่อ 1 USD)');
    expect([buy.value, sell.value, name.value, cost.value]).toEqual(['33.2500', '34.1000', 'SuperRich', '33.9000']);
    for (const input of [buy, sell]) {
      expect(input.type).toBe('number');
      expect(input.required).toBe(true);
      expect(input.getAttribute('min')).toBe('0.0001');
      expect(input.getAttribute('max')).toBe('1000000');
      expect(input.getAttribute('step')).toBe('0.0001');
    }
    expect(cost.type).toBe('number');
    expect(cost.required).toBe(false);
    expect(cost.getAttribute('min')).toBe('0.0001');
    expect(cost.getAttribute('max')).toBe('1000000');
    expect(cost.getAttribute('step')).toBe('0.0001');
    expect(name.getAttribute('maxlength')).toBe('100');
    expect(name.getAttribute('placeholder')).toBe('ไม่ระบุได้ หากยังไม่มีต้นทุนคู่ค้า');
    expect(buttonsNamed(d, 'ยกเลิก').length).toBe(1);
    expect(buttonsNamed(d, 'บันทึกเรทลง เซิร์ฟเวอร์').length).toBe(1);
  });

  it('JPY: the BOT line carries the per-100 note; a currency the BOT lacks: ยังไม่มีข้อมูลอ้างอิง; an unset rate: empty fields (ticket §9)', async () => {
    const m = mount();
    await settle();
    let d = await openEdit(m.el, 'JPY');
    expect(text(d)).toContain(botLine(refRow('JPY', 100, '0.2095', '0.2101', '0.2163')));
    expect(text(d)).toContain(JPY_NOTE);
    button(d, 'ยกเลิก').click();
    await settle();
    expect(dialogOpen(), 'ยกเลิก closes').toBe(false);

    d = await openEdit(m.el, 'KRW');
    expect(text(d)).toContain('ยังไม่มีข้อมูลอ้างอิง');
    expect(text(d)).not.toContain('ธปท. วันที่');
    expect(field(d, 'ร้านรับซื้อ').value).toBe('');
    expect(field(d, 'ร้านขาย').value).toBe('');
    expect(field(d, 'ชื่อคู่ค้า').value).toBe('');
    expect(field(d, 'เรทที่ร้านรับซื้อจากคู่ค้า (THB ต่อ 1 KRW)').value).toBe('');
    expect(posts().length).toBe(0);
  });

  it("the BOT line shows the reference's error instead (rates-reference.js:6)", async () => {
    reference = { version: 4, data: refData({ rows: [], error: BANK_DOWN }) };
    const m = mount();
    await settle();
    const d = await openEdit(m.el, 'USD');
    expect(text(d)).toContain(BANK_DOWN);
    expect(text(d)).not.toContain('ธปท. วันที่');
  });

  it("saves through /api/rpc adminSaveRate with v2's payload — typed text, the partner trimmed, the row's updated — then toasts, closes and re-reads with since", async () => {
    const m = mount();
    await settle();
    const d = await openEdit(m.el, 'USD');
    type(field(d, 'ร้านรับซื้อ'), '33.3');
    type(field(d, 'ร้านขาย'), '34.2');
    type(field(d, 'ชื่อคู่ค้า'), '  Vasu Exchange  ');
    type(field(d, 'เรทที่ร้านรับซื้อจากคู่ค้า (THB ต่อ 1 USD)'), '33.95');
    rpc = () => {
      rates.MAIN = { version: 8, data: ratesData({ rates: [usd({ buy: '33.3000', sell: '34.2000' })] }) };
      return { status: 200, body: { version: 1 } };
    };
    button(d, 'บันทึกเรทลง เซิร์ฟเวอร์').click();
    await settle();

    expect(posts().length).toBe(1);
    const post = posts()[0]!;
    expect(post.path).toBe(RPC);
    const b = post.body as { action: string; payload: Record<string, unknown> };
    expect(b.action).toBe('adminSaveRate');
    const { requestId, ...rest } = b.payload;
    expect(rest).toEqual({
      code: 'USD',
      buy: '33.3',
      sell: '34.2',
      partnerName: 'Vasu Exchange',
      partnerCost: '33.95',
      previousUpdated: '2026-10-06T05:00:01.000Z',
      branchId: 'MAIN',
    });
    expect(String(requestId)).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);

    expect(toastText()).toBe(partsTh.serverSaved);
    expect(dialogOpen(), 'the dialog closed').toBe(false);
    expect(readsOf(RATES).length, 'the view is re-read').toBeGreaterThanOrEqual(2);
    expect(readsOf(RATES)[1]!.params.get('since'), 'with the since it holds').toBe('7');
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(cells(rowOf(t, 'USD')).slice(1, 3)).toEqual(['33.3000', '34.2000']);
  });

  it('without a partner: partnerName "" and partnerCost null; a currency never set sends previousUpdated ""', async () => {
    const m = mount();
    await settle();
    const d = await openEdit(m.el, 'KRW');
    type(field(d, 'ร้านรับซื้อ'), '0.0231');
    type(field(d, 'ร้านขาย'), '0.0259');
    button(d, 'บันทึกเรทลง เซิร์ฟเวอร์').click();
    await settle();
    expect(posts().length).toBe(1);
    const b = posts()[0]!.body as { action: string; payload: Record<string, unknown> };
    expect(b.payload).toMatchObject({
      code: 'KRW',
      buy: '0.0231',
      sell: '0.0259',
      partnerName: '',
      partnerCost: null,
      previousUpdated: '',
      branchId: 'MAIN',
    });
  });

  it('a partner name without a cost, or a cost without a name, is refused in the form with v2\'s sentence and nothing is sent', async () => {
    const m = mount();
    await settle();
    let d = await openEdit(m.el, 'EUR');
    type(field(d, 'ชื่อคู่ค้า'), 'Vasu');
    button(d, 'บันทึกเรทลง เซิร์ฟเวอร์').click();
    await settle();
    expect(text(theDialog())).toContain(PARTNER_PAIR);
    expect(posts().length).toBe(0);
    expect(dialogOpen()).toBe(true);
    button(theDialog(), 'ยกเลิก').click();
    await settle();

    d = await openEdit(m.el, 'EUR');
    type(field(d, 'เรทที่ร้านรับซื้อจากคู่ค้า (THB ต่อ 1 EUR)'), '38.5');
    button(d, 'บันทึกเรทลง เซิร์ฟเวอร์').click();
    await settle();
    expect(text(theDialog())).toContain(PARTNER_PAIR);
    expect(posts().length).toBe(0);
  });

  it("a selling rate below the buying rate is not saved and v2's sentence is shown", async () => {
    rpc = (body) => {
      const p = body.payload;
      return Number(p.sell) < Number(p.buy)
        ? { status: 400, body: { error: SELL_BELOW_BUY } }
        : { status: 200, body: { version: 1 } };
    };
    const m = mount();
    await settle();
    const d = await openEdit(m.el, 'USD');
    type(field(d, 'ร้านรับซื้อ'), '35');
    type(field(d, 'ร้านขาย'), '34');
    button(d, 'บันทึกเรทลง เซิร์ฟเวอร์').click();
    await settle();
    expect(text(document.body)).toContain(SELL_BELOW_BUY);
    expect(toastText()).not.toBe(partsTh.serverSaved);
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(cells(rowOf(t, 'USD'))[1]).toBe('33.2500');
  });
});

// ── R3: the BOT panel and its 60 s refresh ───────────────────────────────────

describe('V2-rates R3: the BOT reference panel (rates-reference.js:5-7)', () => {
  it("v2's panel: heading, the 60 s sentence, the fetched line, the buttons and the BOT's source link", async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('BOT REFERENCE / PARTNER COST');
    expect(all).toContain('เทียบเรท ธปท. และต้นทุนคู่ค้า');
    expect(all).toContain('บาทต่อ 1 หน่วยเงินต่างประเทศ · ธปท. เผยแพร่เรทถัวเฉลี่ยรายวัน · ตรวจข้อมูลทุก 60 วินาทีขณะเปิดหน้านี้');
    expect(all).toContain(`ดึงข้อมูลล่าสุด ${time('2026-10-06T04:30:00.000Z')}`);
    expect(buttonsNamed(m.el, 'โหลดเรท ธปท. ล่าสุด').length).toBe(1);
    expect(buttonsNamed(m.el, 'ประวัติการปรับเรท').length).toBe(1);
    const source = [...m.el.querySelectorAll('a')].filter((a) => text(a) === 'แหล่งข้อมูล ธปท. ↗');
    expect(source.length).toBe(1);
    expect(source[0]!.getAttribute('href')).toBe('https://www.bot.or.th/th/statistics/exchange-rate.html');
    expect(source[0]!.getAttribute('target')).toBe('_blank');
    expect(source[0]!.getAttribute('rel')).toBe('noopener');
  });

  it("the comparison table: the shop's rates, the BOT's, the partner's cost and the server's margin, — where none", async () => {
    const m = mount();
    await settle();
    const t = tableStarting(m.el, 'คู่เงิน');
    expect(headers(t)).toEqual([
      'คู่เงิน',
      'ร้านรับซื้อ / ขาย',
      'ธปท. ซื้อตั๋ว / โอน / ขาย',
      'ต้นทุนจากคู่ค้า',
      'ส่วนต่างขาย − ต้นทุน',
    ]);
    expect(rows(t).length, 'one row per currency of admin_rates').toBe(4);
    const u = cells(rowOf(t, 'USD/THB'));
    expect(u[0]).toBe('USD/THB');
    expect(u[1]).toBe('33.2500 / 34.1000');
    expect(u[2]).toContain('33.4218 / 33.5008 / 33.8278');
    expect(u[2]).toContain('2026-10-06');
    expect(u[3]).toContain('33.9000');
    expect(u[3]).toContain('SuperRich');
    expect(u[4], "the server's partnerSpread, not sell − cost").toBe('8.8888');

    const e = cells(rowOf(t, 'EUR/THB'));
    expect(e[2], 'the BOT has no EUR row').toBe('—');
    expect(e[3]).toContain('—');
    expect(e[3]).toContain('ยังไม่ระบุ');
    expect(e[4]).toBe('—');

    expect(cells(rowOf(t, 'JPY/THB'))[2]).toContain('0.2095 / 0.2101 / 0.2163');
    expect(cells(rowOf(t, 'KRW/THB'))[1]).toBe('— / —');
  });

  it('before the first answer: กำลังดึงข้อมูล ธปท.…', async () => {
    let release: () => void = () => {};
    referenceGate = new Promise((r) => {
      release = r;
    });
    const m = mount();
    await settle();
    expect(text(m.el)).toContain('กำลังดึงข้อมูล ธปท.…');
    release();
    referenceGate = null;
    await settle();
    expect(text(m.el)).not.toContain('กำลังดึงข้อมูล ธปท.…');
    expect(text(m.el)).toContain(`ดึงข้อมูลล่าสุด ${time('2026-10-06T04:30:00.000Z')}`);
  });

  it("BOT unavailable (ticket §9): the reference's error sentence in place of the fetched line, the BOT column —", async () => {
    reference = { version: 4, data: refData({ rows: [], error: BANK_DOWN }) };
    const m = mount();
    await settle();
    expect(text(m.el)).toContain(BANK_DOWN);
    expect(text(m.el)).not.toContain('ดึงข้อมูลล่าสุด');
    const t = tableStarting(m.el, 'คู่เงิน');
    expect(cells(rowOf(t, 'USD/THB'))[2]).toBe('—');
  });

  it("a refused read (a staff caller: the reference is the owner's) shows the refusal's sentence; the rates still show", async () => {
    referenceRefusal = { status: 403, code: 'ownerOnly', message: OWNER_ONLY };
    const m = mount();
    await settle();
    expect(text(m.el)).toContain(OWNER_ONLY);
    const t = tableStarting(m.el, 'สกุลเงิน');
    expect(cells(rowOf(t, 'USD'))[1]).toBe('33.2500');
  });

  it('โหลดเรท ธปท. ล่าสุด reads with fresh=1 and no since, and shows the new rows', async () => {
    const m = mount();
    await settle();
    reference = {
      version: 5,
      data: refData({ rows: [refRow('USD', 1, '33.5000', '33.6000', '33.9000')], fetchedAt: '2026-10-06T05:10:00.000Z' }),
    };
    button(m.el, 'โหลดเรท ธปท. ล่าสุด').click();
    await settle();
    const last = readsOf(REFERENCE).at(-1)!;
    expect(last.params.get('fresh')).toBe('1');
    expect(last.params.has('since')).toBe(false);
    expect(text(m.el)).toContain('33.5000 / 33.6000 / 33.9000');
    expect(text(m.el)).toContain(`ดึงข้อมูลล่าสุด ${time('2026-10-06T05:10:00.000Z')}`);
  });
});

describe('V2-rates R3: the 60 s refresh — the one admin poll, on its own version (rates-reference.js:12; design §12.16)', () => {
  it('every 60 s while shown and visible: one read of rates_reference with the since it holds; nothing else is polled', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    const m = mount();
    await settle();
    expect(readsOf(REFERENCE).length).toBe(1);

    await vi.advanceTimersByTimeAsync(59_000);
    expect(readsOf(REFERENCE).length, 'not before 60 s').toBe(1);
    await vi.advanceTimersByTimeAsync(1_500);
    await settle();
    expect(readsOf(REFERENCE).length, 'at 60 s').toBe(2);
    expect(readsOf(REFERENCE)[1]!.params.get('since')).toBe('3');
    expect(readsOf(REFERENCE)[1]!.params.has('fresh')).toBe(false);
    expect(text(m.el), 'changed: false keeps the panel').toContain('33.4218 / 33.5008 / 33.8278');

    reference = {
      version: 6,
      data: refData({ rows: [refRow('USD', 1, '33.7000', '33.8000', '34.0000')], fetchedAt: '2026-10-06T05:20:00.000Z' }),
    };
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(readsOf(REFERENCE).length).toBe(3);
    expect(readsOf(REFERENCE)[2]!.params.get('since')).toBe('3');
    expect(text(m.el)).toContain('33.7000 / 33.8000 / 34.0000');

    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(readsOf(REFERENCE).length).toBe(4);
    expect(readsOf(REFERENCE)[3]!.params.get('since'), 'the since of the last answer').toBe('6');

    expect(readsOf(RATES).length, 'admin_rates is never polled (design §12.6)').toBe(1);
    expect(own().every((s) => s.path === RATES || s.path === REFERENCE)).toBe(true);
  });

  it('no read while the tab is hidden; reads again once it is visible', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    mount();
    await settle();
    expect(readsOf(REFERENCE).length).toBe(1);
    setHidden(true);
    await vi.advanceTimersByTimeAsync(180_000);
    await settle();
    expect(readsOf(REFERENCE).length, 'hidden: no read').toBe(1);
    setHidden(false);
    await vi.advanceTimersByTimeAsync(61_000);
    await settle();
    expect(readsOf(REFERENCE).length, 'visible again: it reads').toBeGreaterThan(1);
    expect(readsOf(REFERENCE).at(-1)!.params.get('since')).toBe('3');
  });

  it('no read while the menu is not shown, none after it is gone', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    const m = mount();
    await settle();
    m.setShown(false);
    await settle();
    const count = readsOf(REFERENCE).length;
    await vi.advanceTimersByTimeAsync(180_000);
    await settle();
    expect(readsOf(REFERENCE).length, 'not shown: no read').toBe(count);

    m.setShown(true);
    await settle();
    expect(readsOf(REFERENCE).length, 'shown again: one read').toBe(count + 1);

    dispose?.();
    dispose = undefined;
    const after = sent.length;
    await vi.advanceTimersByTimeAsync(300_000);
    await settle();
    expect(sent.length, 'unmounted: the timer stopped').toBe(after);
  });
});

// ── R4: the history dialog ───────────────────────────────────────────────────

describe('V2-rates R4: the history dialog walks admin_rate_history to the end (rates-reference.js:8; design §12.14)', () => {
  it('pages of 100 with before = the last id until a short page; the whole history in one table', async () => {
    history.MAIN = Array.from({ length: 150 }, (_, i) => historyRow(i));
    const m = mount();
    await settle();
    button(m.el, 'ประวัติการปรับเรท').click();
    await settle();
    const pages = readsOf(HISTORY);
    expect(pages.length).toBe(2);
    expect(pages[0]!.params.get('limit')).toBe('100');
    expect(pages[0]!.params.has('before')).toBe(false);
    expect(pages[0]!.params.get('branch')).toBe('MAIN');
    expect(pages[1]!.params.get('limit')).toBe('100');
    expect(pages[1]!.params.get('before')).toBe(history.MAIN[99]!.id);

    const d = theDialog();
    expect(dialogOpen()).toBe(true);
    expect(text(d.querySelector('h2'))).toBe('ประวัติการปรับเรท · สาขาหลัก');
    expect(text(d)).toContain('ค่า ธปท. ที่เซิร์ฟเวอร์ดึงขณะบันทึกแต่ละครั้ง · รายการเก่าอาจยังไม่มีค่าอ้างอิง');
    const t = tableStarting(d, 'เวลา / ผู้แก้');
    expect(headers(t)).toEqual(['เวลา / ผู้แก้', 'คู่เงิน / เรทร้าน', 'คู่ค้า / ต้นทุน', 'สำเนาเรท ธปท.']);
    expect(rows(t).length).toBe(150);
  });

  it("each row: time and who, the pair and the shop's rates, the partner and cost, the BOT copy or ยังไม่มีข้อมูลอ้างอิง", async () => {
    history.MAIN = [
      historyRow(0, {
        id: '9',
        code: 'JPY',
        buy: '0.2000',
        sell: '0.2200',
        updated: '2026-10-06T05:00:09.000Z',
        actor: 'owner',
        partnerName: 'Vasu',
        partnerCost: '0.2100',
        botReference: {
          fetchedAt: '2026-10-06T04:59:00.000Z',
          rows: [refRow('JPY', 100, '0.2095', '0.2101', '0.2163')],
        },
      }),
      historyRow(1, {
        id: '8',
        code: 'USD',
        buy: '33.0000',
        sell: '34.0000',
        updated: '2026-10-06T05:00:08.000Z',
        actor: 'rates1',
        botReference: { fetchedAt: '2026-10-06T05:00:00.000Z', rows: [] },
      }),
      historyRow(2, { id: '7', code: 'EUR', buy: '37.0000', sell: '38.0000', actor: 'seed', botReference: null }),
    ];
    const m = mount();
    await settle();
    button(m.el, 'ประวัติการปรับเรท').click();
    await settle();
    const t = tableStarting(theDialog(), 'เวลา / ผู้แก้');
    const [jpy, usdRow, eur] = rows(t).map(cells);

    expect(jpy![0]).toContain(time('2026-10-06T05:00:09.000Z'));
    expect(jpy![0]).toContain('owner');
    expect(jpy![1]).toContain('JPY/THB');
    expect(jpy![1]).toContain('0.2000 / 0.2200');
    expect(jpy![2]).toContain('Vasu');
    expect(jpy![2]).toContain('0.2100');
    expect(jpy![3]).toContain(botLine(refRow('JPY', 100, '0.2095', '0.2101', '0.2163')));
    expect(jpy![3]).toContain(`ดึงข้อมูล ${time('2026-10-06T04:59:00.000Z')}`);
    expect(jpy![3]).toContain(JPY_NOTE);

    expect(usdRow![0]).toContain('rates1');
    expect((usdRow![2]!.match(/—/g) ?? []).length, `no partner: — and —: ${usdRow![2]}`).toBe(2);
    expect(usdRow![3], "a copy without the currency's row").toBe('ยังไม่มีข้อมูลอ้างอิง');
    expect(eur![3], 'no copy at all').toBe('ยังไม่มีข้อมูลอ้างอิง');
  });

  it('an empty history shows the empty table of v2', async () => {
    const m = mount();
    await settle();
    button(m.el, 'ประวัติการปรับเรท').click();
    await settle();
    expect(readsOf(HISTORY).length).toBe(1);
    expect(text(theDialog())).toContain(partsTh.emptyTransactions);
  });
});

describe('V2-rates English mode (W10)', () => {
  it('in English mode: edit dialog opens with no Thai text (W10)', async () => {
    setLang('en');
    try {
      const m = mount();
      await settle();
      const b = m.el.querySelector<HTMLButtonElement>('button[aria-label="Edit rate USD"]');
      expect(b, 'the edit button in en mode').not.toBeNull();
      b!.click();
      await settle();
      expect(dialogOpen(), 'the dialog opened').toBe(true);
      const d = theDialog();
      expect(text(d)).toContain('Set Live Rate USD');
      expect(text(d)).toContain('2026');
      expect(text(d)).not.toContain('2569');
      expect(text(d)).not.toMatch(/[ก-๙]/);
      const attrs = Array.from(d.querySelectorAll('[aria-label], [title], [placeholder]')).map(
        (e) => `${e.getAttribute('aria-label') ?? ''} ${e.getAttribute('title') ?? ''} ${e.getAttribute('placeholder') ?? ''}`,
      );
      expect(attrs.filter((l) => /[ก-๙]/.test(l))).toEqual([]);
    } finally {
      setLang('th');
    }
  });
});
