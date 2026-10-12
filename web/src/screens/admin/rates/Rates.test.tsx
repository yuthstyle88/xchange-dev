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
// reads AppShell makes (`PAGE_WIDE`, `shared/testing/page-wide.ts`) are answered and
// never counted as the menu's own.
//
// The menu is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import.
//
// Retired in part by #602 (RE-5), by its ticket author (claude/e5ab6c), as `docs/design/blueprint/
// rates.md` §9 lists ("the direct save → publications", RE-5/RE-7): D-3 retires `adminSaveRate` and
// the `rates` table, so the menu no longer reads `admin_rates` or `admin_rate_history` and no longer
// saves a rate; §3.3/§4.2 show every rate through `formatRate` per its unit (BoT JPY per 100, not
// converted to per 1). Removed, because each pins what those sections replace:
// - "what it reads, and when" (4 tests): reads of `admin_rates`, its `since` and its table;
// - R1 (3): the table of `admin_rates` with the server's spread and the LIVE / per-1 note;
// - R2 (7): the LIVE edit dialog that saves through `/api/rpc` `adminSaveRate`, its partner fields,
//   the per-1 JPY note;
// - R3 (5 of 6): the per-1 panel sentence, the comparison table of `admin_rates` and partner cost,
//   the 4-place reference text, "the rates still show" from `admin_rates`;
// - R3 poll (1 of 3): asserts `admin_rates` is read and that nothing but it and `rates_reference`
//   is read;
// - R4 (3): the history paged from `admin_rate_history` with its BoT copy and partner columns;
// - English mode (1): the LIVE edit dialog.
// Kept, because the rebuilt menu still does it: R5 (the kiosk currency link), R3 "before the first
// answer", and the two R3 poll tests on `rates_reference`. The new behaviour is pinned in
// `RatesDraft.test.tsx` and `web/src/shared/ui/rate.test.ts`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

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
  if (path === QUEUE_CARD_PATH) {
    return { status: 200, body: queueCardUnchanged() };
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

function readsOf(path: string): Sent[] {
  return sent.filter((s) => s.path === path);
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

// ── R3: the BOT panel and its 60 s refresh ───────────────────────────────────

describe('V2-rates R3: the BOT reference panel (rates-reference.js:5-7)', () => {
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

});

describe('V2-rates R3: the 60 s refresh — the one admin poll, on its own version (rates-reference.js:12; design §12.16)', () => {
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
