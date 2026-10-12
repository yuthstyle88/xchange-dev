// EX-4 (#1186): the A01 screen over `GET /api/executive/overview` (ticket §8 behaviour table, §9 edge cases;
// design `docs/design/blueprint/executive-finance.md` §4.1–§4.5, §8.1, D-2, D-5). Written before the
// implementation by the test-writer claude/5d45b017, on the FE lane controller's order (claude/5edf7bb5).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// The answer is EX-3's (#1071), typed by the generated `executive_overview.ts`, with the contract points the
// BE controller fixed on #1071 (issuecomment-6108369072): alert kinds `ratePending` / `staleData` /
// `countDifference`; `branches=B1,B2`, where an empty item is `400 validationFailed` and an id outside scope
// is `403 outOfScope`; an undecodable exception row's alert carries only the row id as its detail.
//
// The screen computes no figure (D-2, D-3): every number, every "unavailable" and every drill-down comes from
// the answer, and money is shown with the shared `formatMoney` from the decimal string.
//
// The DOM hooks these tests read are part of the contract [required]:
//   - a KPI tile is `[data-kpi="<metric key>"]`, in the answer's order;
//   - a branch-table row is `[data-branch="<branch id>"]`;
//   - an alert is `[data-alert-kind="<wire kind>"]`;
//   - a drill-down is an element `[data-drilldown]` inside its tile, row or alert; activating it (`click()`)
//     sets `location.hash` to `#<menu>?<the drill-down's filter>`;
//   - a time the answer gives (`asOf`) is a `<time datetime="<the answer's string>">`;
//   - the filters are `select[name="channel"]`, `[name="currency"]` (a select or an input), and one
//     `input[type="checkbox"][name="branch"][value="<id>"]` per branch of the answer;
//   - a refusal is shown in `.error-banner`, the admin screens' banner.
//
// The fake API is the global `fetch`: it answers `/api/executive/overview` and records every request. The
// menu is read through an eager glob, so on a branch without it each test fails on an assertion that the
// file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type {
  AlertDto,
  BranchRowDto,
  DefinitionDto,
  DrilldownDto,
  ExecutiveOverviewData,
  MetricDto,
} from '../../../shared/api/generated/executive_overview';
import { errorText, setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/executive/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/executive/i18n.ts exists').toBeDefined();
  return m!;
}

const ENDPOINT = '/api/executive/overview';

// ── the answer ───────────────────────────────────────────────────────────────

const FROM = '2026-10-05';
const TO = '2026-10-06';

/** No digit in any definition, so a tile's digits can only be its figure. */
const DEFINITIONS: DefinitionDto[] = [
  { key: 'buy_volume_thb', th: 'มูลค่าบาทของรายการซื้อที่สำเร็จ ไม่ใช่รายได้', en: 'THB value of completed buys; not revenue' },
  { key: 'sell_volume_thb', th: 'มูลค่าบาทของรายการขายที่สำเร็จ', en: 'THB value of completed sells' },
  { key: 'realized_margin_thb', th: 'เงินบาทที่ได้รับลบต้นทุนที่ปันส่วน', en: 'THB received less allocated cost' },
  { key: 'repeat_customers', th: 'ลูกค้าที่เคยมีรายการสำเร็จมาก่อน', en: 'Customers with an earlier completed transaction' },
  { key: 'booking_conversion', th: 'การจองที่สำเร็จหารด้วยการจองที่ยืนยัน', en: 'Completed bookings over confirmed bookings' },
  { key: 'commission_thb', th: 'ค่าคอมมิชชันของช่วงเวลา', en: 'Commission expense of the period' },
];

/** E-1's Thai label of each KPI key (design §3). */
const LABEL: Record<string, string> = {
  buy_volume_thb: 'ยอดซื้อ',
  sell_volume_thb: 'ยอดขาย',
  realized_margin_thb: 'ส่วนต่างรับรู้',
  repeat_customers: 'ลูกค้าซ้ำ',
  booking_conversion: 'จอง → สำเร็จ',
  commission_thb: 'ค่าคอม',
};

function drill(view: string, filter: Record<string, string>, dropped: string[] = []): DrilldownDto {
  return { view, filter, dropped };
}

function metric(
  key: string,
  value: string | null,
  unavailable: MetricDto['unavailable'] = null,
  drilldown: DrilldownDto | null = null,
): MetricDto {
  return { key, value, unavailable, drilldown };
}

function kpis(): MetricDto[] {
  return [
    metric('buy_volume_thb', '1234567.89', null, drill('admin_transactions', { from: FROM, to: TO, type: 'BUY' }, ['currency', 'channel'])),
    metric('sell_volume_thb', '765.40', null, drill('admin_transactions', { from: FROM, to: TO, type: 'SELL' })),
    metric('realized_margin_thb', null, 'awaitsLedger'),
    metric('repeat_customers', '12', null, drill('admin_customers', {}, ['repeat', 'from', 'to'])),
    metric('booking_conversion', null, 'awaitsBookingDesign'),
    metric('commission_thb', null, 'awaitsBroker'),
  ];
}

function branchRow(id: string, over: Partial<BranchRowDto> = {}): BranchRowDto {
  return {
    branchId: id,
    name: id === 'B1' ? 'สาขาหนึ่ง' : 'สาขาสอง',
    volumeThb: id === 'B1' ? '2000.50' : '3100.25',
    availableCash: metric('available_cash', id === 'B1' ? '150000.00' : '98765.43'),
    inTransit: metric('in_transit', id === 'B1' ? '5000.00' : '4321.09'),
    shifts: { open: 1, closedVerified: 2, closedUnverified: 3 },
    asOf: id === 'B1' ? '2026-10-06T03:00:00Z' : '2026-10-06T01:00:00Z',
    pendingEvents: 0,
    drilldown: drill('admin_overview', { branch: id, from: FROM, to: TO }, ['currency', 'channel']),
    ...over,
  };
}

function alerts(): AlertDto[] {
  return [
    {
      kind: 'ratePending',
      branchId: 'B1',
      since: '2026-10-06T02:00:00Z',
      detail: 'RQ-77',
      drilldown: drill('approval_requests', { branch: 'B1', type: 'RATE_PUBLICATION' }),
    },
    {
      // an exception row the reader could not decode: only its id (#1071 ruling, gap 4); a target with no menu
      kind: 'countDifference',
      branchId: 'B2',
      since: null,
      detail: 'EXC-9f3a',
      drilldown: drill('cash_counts', { branch: 'B2' }, ['difference']),
    },
    {
      kind: 'staleData',
      branchId: 'B2',
      since: '2026-10-06T00:30:00Z',
      detail: 'XC-2026-0009',
      drilldown: drill('admin_transactions', { branch: 'B2' }),
    },
  ];
}

function data(over: Partial<ExecutiveOverviewData> = {}): ExecutiveOverviewData {
  return {
    period: { from: FROM, to: TO },
    kpis: kpis(),
    branches: [branchRow('B1'), branchRow('B2')],
    alerts: alerts(),
    definitions: DEFINITIONS,
    ...over,
  };
}

const VERSIONS = { B1: 4, B2: 9 };

// ── the fake API: the global fetch ──────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  credentials: RequestCredentials | undefined;
}

type Reply = { status: number; body: unknown };

let sent: Sent[];
let reply: (params: URLSearchParams) => Reply;

function answered(versions: Record<string, number>, d: ExecutiveOverviewData): Reply {
  return { status: 200, body: { changed: true, versions, data: d } };
}

function unchanged(versions: Record<string, number>): Reply {
  return { status: 200, body: { changed: false, versions } };
}

function refused(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function fakeFetch(input: string | URL | Request, init?: RequestInit): Promise<Response> {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(raw, 'http://localhost');
  const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
  const credentials = init?.credentials ?? (input instanceof Request ? input.credentials : undefined);
  sent.push({ path: url.pathname, params: url.searchParams, method, credentials });
  let r: Reply;
  if (url.pathname === '/api/v3/views/me') {
    r = { status: 200, body: { version: 1, changed: false } };
  } else if (url.pathname === QUEUE_CARD_PATH) {
    r = { status: 200, body: queueCardUnchanged() };
  } else if (url.pathname === ENDPOINT) {
    r = reply(url.searchParams);
  } else {
    r = refused(500, 'serverError', `unexpected ${url.pathname}`);
  }
  return Promise.resolve(
    new Response(JSON.stringify(r.body), { status: r.status, headers: { 'content-type': 'application/json' } }),
  );
}

function own(): Sent[] {
  return sent.filter((s) => !PAGE_WIDE.includes(s.path));
}

function reads(): Sent[] {
  return sent.filter((s) => s.path === ENDPOINT);
}

function last(): Sent {
  const all = reads();
  expect(all.length, 'A01 has read the overview').toBeGreaterThan(0);
  return all[all.length - 1]!;
}

const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement;
let hidden = false;

interface Mounted {
  el: HTMLElement;
  setPeriod: (p: { from: string; to: string }) => void;
  setShown: (s: boolean) => void;
}

function mount(init: { shown?: boolean } = {}): Mounted {
  const menu = theMenu();
  const [branch] = createSignal('MAIN');
  const [period, setPeriod] = createSignal({ from: FROM, to: TO });
  const [shown, setShown] = createSignal(init.shown ?? true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => (<Component branch={branch} period={period} shown={shown} />) as JSX.Element, root);
  return { el: root, setPeriod, setShown };
}

beforeEach(() => {
  sent = [];
  reply = () => answered(VERSIONS, data());
  hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  location.hash = '#executive';
  setLang('th');
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  root?.remove();
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

function squash(s: string): string {
  return s.replace(/\s+/g, '');
}

function tiles(el: HTMLElement): HTMLElement[] {
  return [...el.querySelectorAll<HTMLElement>('[data-kpi]')];
}

function tile(el: HTMLElement, key: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-kpi="${key}"]`);
  expect(found.length, `one KPI tile [data-kpi="${key}"]`).toBe(1);
  return found[0]!;
}

function row(el: HTMLElement, id: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-branch="${id}"]`);
  expect(found.length, `one branch row [data-branch="${id}"]`).toBe(1);
  return found[0]!;
}

function alertsOf(el: HTMLElement, kind: string): HTMLElement[] {
  return [...el.querySelectorAll<HTMLElement>(`[data-alert-kind="${kind}"]`)];
}

function theAlert(el: HTMLElement, kind: string): HTMLElement {
  const found = alertsOf(el, kind);
  expect(found.length, `one alert [data-alert-kind="${kind}"]`).toBe(1);
  return found[0]!;
}

function drilldownIn(scope: HTMLElement, what: string): HTMLElement {
  const found = scope.querySelectorAll<HTMLElement>('[data-drilldown]');
  expect(found.length, `${what} has one drill-down`).toBe(1);
  return found[0]!;
}

/** `#menu?a=1&b=2` → its menu and its query as a sorted list of pairs. */
function parseHash(hash: string): { menu: string; query: [string, string][] } {
  const raw = hash.startsWith('#') ? hash.slice(1) : hash;
  const [menu, qs = ''] = raw.split('?');
  const query = [...new URLSearchParams(qs).entries()].sort(([a], [b]) => a.localeCompare(b));
  return { menu: menu!, query };
}

function sorted(filter: Record<string, string>): [string, string][] {
  return Object.entries(filter).sort(([a], [b]) => a.localeCompare(b));
}

async function activate(el: HTMLElement): Promise<void> {
  el.click();
  await settle();
}

function banner(el: HTMLElement): string | null {
  const b = el.querySelector('.error-banner');
  return b ? text(b) : null;
}

function change(el: HTMLInputElement | HTMLSelectElement): void {
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

// ── what it reads, and when ──────────────────────────────────────────────────

describe('EX-4 A01: what it reads (ticket §7, §8; design §4.1, §8.1)', () => {
  it('on show: one GET of /api/executive/overview with the period, credentials same-origin, no versions, nothing else of its own', async () => {
    mount();
    await settle();
    expect(reads().length).toBe(1);
    const q = reads()[0]!;
    expect(q.method).toBe('GET');
    expect(q.credentials).toBe('same-origin');
    expect(q.params.get('from')).toBe(FROM);
    expect(q.params.get('to')).toBe(TO);
    expect(q.params.has('versions'), 'the first read carries no versions').toBe(false);
    expect(q.params.has('branches'), 'no branch chosen = all of them: no branches key').toBe(false);
    expect(own().map((s) => s.path), 'A01 reads nothing but its overview').toEqual([ENDPOINT]);
  });

  it('a hidden menu (shown() false) reads nothing', async () => {
    mount({ shown: false });
    await settle();
    expect(reads().length).toBe(0);
  });

  it('a new period refetches the whole answer: the new dates, and no versions', async () => {
    const m = mount();
    await settle();
    m.setPeriod({ from: '2026-09-01', to: '2026-09-30' });
    await settle();
    const q = last();
    expect(reads().length).toBe(2);
    expect(q.params.get('from')).toBe('2026-09-01');
    expect(q.params.get('to')).toBe('2026-09-30');
    expect(q.params.has('versions')).toBe(false);
  });
});

// ── the header and the KPI tiles ─────────────────────────────────────────────

describe('EX-4 A01: the header and the six KPI tiles (§4.2, E-1, D-2)', () => {
  it('the header says ทุกสาขา and shows the oldest as_of of the branches shown', async () => {
    const m = mount();
    await settle();
    expect(text(m.el)).toContain('ทุกสาขา');
    expect(text(m.el)).toContain('ข้อมูลล่าสุด');
    const outside = [...m.el.querySelectorAll<HTMLTimeElement>('time[datetime]')].filter(
      (t) => !t.closest('[data-branch]') && !t.closest('[data-alert-kind]'),
    );
    expect(
      outside.map((t) => t.getAttribute('datetime')),
      "the header's time is B2's as_of, the older of the two",
    ).toContain('2026-10-06T01:00:00Z');
    expect(outside.map((t) => t.getAttribute('datetime'))).not.toContain('2026-10-06T03:00:00Z');
  });

  it('one tile per KPI, in the answer’s order (not a fixed order of its own)', async () => {
    const reversed = kpis().reverse();
    reply = () => answered(VERSIONS, data({ kpis: reversed }));
    const m = mount();
    await settle();
    expect(tiles(m.el).map((t) => t.getAttribute('data-kpi'))).toEqual(reversed.map((k) => k.key));
  });

  it("each tile shows its E-1 label and its definition from the answer's definitions", async () => {
    const m = mount();
    await settle();
    for (const d of DEFINITIONS) {
      const t = squash(text(tile(m.el, d.key)));
      expect(t, `${d.key}: label`).toContain(squash(LABEL[d.key]!));
      expect(t, `${d.key}: definition`).toContain(squash(d.th));
    }
  });

  it('a figure is the decimal string through formatMoney, exactly: "1234567.89" → 1,234,567.89', async () => {
    const m = mount();
    await settle();
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('1234567.89'));
    expect(formatMoney('1234567.89')).toBe('1,234,567.89');
    expect(text(tile(m.el, 'sell_volume_thb')), 'the trailing zero is kept: no float').toContain('765.40');
    expect(text(tile(m.el, 'repeat_customers'))).toContain('12');
  });

  it('an unavailable KPI (awaitsBroker) shows its reason sentence and no number at all', async () => {
    const m = mount();
    await settle();
    const t = text(tile(m.el, 'commission_thb'));
    expect(/\d/.test(t), `no digit in the tile: ${t}`).toBe(false);
    expect(t).not.toContain('null');
    const rest = squash(t).replace(squash(LABEL.commission_thb!), '').replace(squash(DEFINITIONS[5]!.th), '');
    expect(rest, 'the tile says why there is no figure').not.toBe('');
  });

  it('each reason has its own fixed sentence: the same tile reads differently for each of the four', async () => {
    const reasons: NonNullable<MetricDto['unavailable']>[] = ['awaitsLedger', 'awaitsBookingDesign', 'awaitsBroker', 'awaitsCampaigns'];
    const seen: string[] = [];
    for (const reason of reasons) {
      reply = () =>
        answered(VERSIONS, data({ kpis: kpis().map((k) => (k.key === 'commission_thb' ? metric(k.key, null, reason) : k)) }));
      const m = mount();
      await settle();
      const t = text(tile(m.el, 'commission_thb'));
      expect(/\d/.test(t), `${reason}: no digit`).toBe(false);
      seen.push(t);
      dispose?.();
      dispose = undefined;
      root.remove();
    }
    expect(new Set(seen).size, `four different sentences: ${JSON.stringify(seen)}`).toBe(4);
  });
});

// ── the branch table ─────────────────────────────────────────────────────────

describe('EX-4 A01: the branch table (§4.2)', () => {
  it('one row per branch, with its name, ยอด, เงินคงเหลือ with in-transit beside it, ปิดกะ and its as_of', async () => {
    const m = mount();
    await settle();
    const r1 = row(m.el, 'B1');
    expect(text(r1)).toContain('สาขาหนึ่ง');
    expect(text(r1)).toContain(formatMoney('2000.50'));
    expect(text(r1)).toContain(formatMoney('150000.00'));
    expect(text(r1)).toContain(formatMoney('5000.00'));
    expect(r1.querySelector('time')?.getAttribute('datetime')).toBe('2026-10-06T03:00:00Z');
    const r2 = row(m.el, 'B2');
    expect(text(r2)).toContain('สาขาสอง');
    expect(text(r2)).toContain(formatMoney('98765.43'));
    expect(text(r2)).toContain(formatMoney('4321.09'));
  });

  it("a branch row's drill-down opens the per-branch overview with the row's filter", async () => {
    const m = mount();
    await settle();
    await activate(drilldownIn(row(m.el, 'B1'), 'the B1 row'));
    const h = parseHash(location.hash);
    expect(h.menu).toBe('overview');
    expect(h.query).toEqual(sorted({ branch: 'B1', from: FROM, to: TO }));
  });
});

// ── the alerts ───────────────────────────────────────────────────────────────

describe('EX-4 A01: ต้องดำเนินการ (§4.3; #1071 rulings, gaps 4–6)', () => {
  it('one row per alert, each with its own kind label, its branch and its detail', async () => {
    const m = mount();
    await settle();
    expect(text(m.el)).toContain('ต้องดำเนินการ');
    const rate = text(theAlert(m.el, 'ratePending'));
    expect(rate).toContain('เรทรออนุมัติ');
    expect(rate.includes('สาขาหนึ่ง') || rate.includes('B1'), `the branch: ${rate}`).toBe(true);
    expect(rate).toContain('RQ-77');
    const stale = text(theAlert(m.el, 'staleData'));
    expect(stale).toContain('ข้อมูลค้าง');
    expect(stale).toContain('XC-2026-0009');
    const count = text(theAlert(m.el, 'countDifference'));
    expect(count).toContain('เงินนับไม่ตรง');
    expect(count.includes('สาขาสอง') || count.includes('B2'), `the branch: ${count}`).toBe(true);
  });

  it("an undecodable exception row's alert shows its id and assumes no other field", async () => {
    const m = mount();
    await settle();
    const count = theAlert(m.el, 'countDifference');
    expect(text(count)).toContain('EXC-9f3a');
    expect(text(count)).not.toContain('undefined');
    expect(text(count)).not.toContain('null');
  });

  it("an alert's drill-down to the approvals list carries its filter", async () => {
    const m = mount();
    await settle();
    await activate(drilldownIn(theAlert(m.el, 'ratePending'), 'the ratePending alert'));
    const h = parseHash(location.hash);
    expect(h.menu).toBe('approvals');
    expect(h.query).toEqual(sorted({ branch: 'B1', type: 'RATE_PUBLICATION' }));
  });

  it('a drill-down to a view with no admin menu (cash_counts) is no link, and the alert still shows', async () => {
    const m = mount();
    await settle();
    const count = theAlert(m.el, 'countDifference');
    expect(count.querySelectorAll('[data-drilldown]').length).toBe(0);
    expect(text(count)).toContain('EXC-9f3a');
  });

  it('no alerts: no alert row, and no error', async () => {
    reply = () => answered(VERSIONS, data({ alerts: [] }));
    const m = mount();
    await settle();
    expect(m.el.querySelectorAll('[data-alert-kind]').length).toBe(0);
    expect(banner(m.el)).toBeNull();
  });
});

// ── drill-down from the KPI tiles ────────────────────────────────────────────

describe('EX-4 A01: drill-down from a KPI (§4.4; ticket §7 targetMenu)', () => {
  it('ยอดซื้อ opens transactions with its filter and says which filters it does not pass yet', async () => {
    const m = mount();
    await settle();
    const t = tile(m.el, 'buy_volume_thb');
    const note = text(t);
    expect(note).toContain('ตัวกรองนี้ยังไม่ส่งต่อ');
    expect(note).toContain('currency');
    expect(note).toContain('channel');
    await activate(drilldownIn(t, 'the ยอดซื้อ tile'));
    const h = parseHash(location.hash);
    expect(h.menu).toBe('transactions');
    expect(h.query).toEqual(sorted({ from: FROM, to: TO, type: 'BUY' }));
  });

  it('a drill-down with nothing dropped shows no dropped note', async () => {
    const m = mount();
    await settle();
    expect(text(tile(m.el, 'sell_volume_thb'))).not.toContain('ตัวกรองนี้ยังไม่ส่งต่อ');
  });

  it('ลูกค้าซ้ำ opens customers; an empty filter is just the menu', async () => {
    const m = mount();
    await settle();
    await activate(drilldownIn(tile(m.el, 'repeat_customers'), 'the ลูกค้าซ้ำ tile'));
    const h = parseHash(location.hash);
    expect(h.menu).toBe('customers');
    expect(h.query).toEqual([]);
  });

  it('every target view maps to its menu: admin_transactions, admin_customers, approval_requests, admin_cash, admin_overview', async () => {
    const targets: [string, string][] = [
      ['admin_transactions', 'transactions'],
      ['admin_customers', 'customers'],
      ['approval_requests', 'approvals'],
      ['admin_cash', 'cash'],
      ['admin_overview', 'overview'],
    ];
    for (const [view, menu] of targets) {
      reply = () =>
        answered(
          VERSIONS,
          data({
            kpis: kpis().map((k) =>
              k.key === 'sell_volume_thb' ? metric(k.key, '765.40', null, drill(view, { branch: 'B1' })) : k,
            ),
          }),
        );
      const m = mount();
      await settle();
      await activate(drilldownIn(tile(m.el, 'sell_volume_thb'), `the ${view} drill-down`));
      expect(parseHash(location.hash).menu, view).toBe(menu);
      dispose?.();
      dispose = undefined;
      root.remove();
      location.hash = '#executive';
    }
  });

  it('a KPI whose view has no menu shows its figure and no link', async () => {
    reply = () =>
      answered(
        VERSIONS,
        data({
          kpis: kpis().map((k) => (k.key === 'sell_volume_thb' ? metric(k.key, '765.40', null, drill('cash_counts', {})) : k)),
        }),
      );
    const m = mount();
    await settle();
    const t = tile(m.el, 'sell_volume_thb');
    expect(t.querySelectorAll('[data-drilldown]').length).toBe(0);
    expect(text(t)).toContain('765.40');
  });
});

// ── filters ──────────────────────────────────────────────────────────────────

describe('EX-4 A01: the filters (§4.1)', () => {
  it('a channel refetches with that channel and no versions; the options are the four channels and all', async () => {
    const m = mount();
    await settle();
    const select = m.el.querySelector<HTMLSelectElement>('select[name="channel"]');
    expect(select, 'select[name="channel"]').not.toBeNull();
    const values = [...select!.options].map((o) => o.value).sort();
    expect(values).toEqual(['', 'Booking', 'Kiosk', 'Referred', 'WalkIn'].sort());
    select!.value = 'Kiosk';
    change(select!);
    await settle();
    expect(reads().length).toBe(2);
    expect(last().params.get('channel')).toBe('Kiosk');
    expect(last().params.has('versions')).toBe(false);
  });

  it('a currency refetches with that currency and no versions', async () => {
    const m = mount();
    await settle();
    const field = m.el.querySelector<HTMLInputElement | HTMLSelectElement>('[name="currency"]');
    expect(field, '[name="currency"]').not.toBeNull();
    let chosen = 'USD';
    if (field instanceof HTMLSelectElement) {
      const option = [...field.options].find((o) => o.value !== '');
      expect(option, 'the currency select offers a currency').toBeDefined();
      chosen = option!.value;
    }
    field!.value = chosen;
    change(field!);
    await settle();
    expect(last().params.get('currency')).toBe(chosen);
    expect(last().params.has('versions')).toBe(false);
  });

  it('branches: one box per branch; the chosen ones are sent comma-separated, never with an empty item; none = all', async () => {
    const m = mount();
    await settle();
    const box = (id: string): HTMLInputElement => {
      const b = m.el.querySelector<HTMLInputElement>(`input[type="checkbox"][name="branch"][value="${id}"]`);
      expect(b, `the box for ${id}`).not.toBeNull();
      return b!;
    };
    box('B2').click();
    await settle();
    expect(last().params.get('branches')).toBe('B2');
    expect(last().params.has('versions')).toBe(false);
    box('B1').click();
    await settle();
    const both = last().params.get('branches') ?? '';
    expect(both.split(',').sort()).toEqual(['B1', 'B2']);
    expect(/(^,|,,|,$)/.test(both), `no empty item in ${both}`).toBe(false);
    box('B1').click();
    await settle();
    box('B2').click();
    await settle();
    expect(last().params.has('branches'), 'nothing chosen = all: no branches key').toBe(false);
  });

  it("a filter's answer replaces the table: a branch it does not list leaves, with its alerts", async () => {
    const m = mount();
    await settle();
    expect(alertsOf(m.el, 'ratePending').length).toBe(1);
    reply = (p) =>
      p.get('branches') === 'B2'
        ? answered({ B2: 9 }, data({ branches: [branchRow('B2')], alerts: alerts().filter((a) => a.branchId === 'B2') }))
        : answered(VERSIONS, data());
    const b2 = m.el.querySelector<HTMLInputElement>('input[type="checkbox"][name="branch"][value="B2"]');
    expect(b2, 'the box for B2').not.toBeNull();
    b2!.click();
    await settle();
    expect(last().params.get('branches')).toBe('B2');
    expect(m.el.querySelectorAll('[data-branch="B1"]').length, 'B1 is not in the answer').toBe(0);
    expect(alertsOf(m.el, 'ratePending').length, "B1's alert left with it").toBe(0);
    expect(text(row(m.el, 'B2'))).toContain('สาขาสอง');
  });
});

// ── polling (D-5) ────────────────────────────────────────────────────────────

describe('EX-4 A01: polling every 60 s with the versions it holds (§4.5, D-5)', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
  });

  it('after 60 s it sends the versions of the answer it holds', async () => {
    mount();
    await settle();
    expect(reads().length).toBe(1);
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(2);
    const v = last().params.get('versions');
    expect(v, 'the poll carries versions').not.toBeNull();
    expect(JSON.parse(v!)).toEqual(VERSIONS);
    expect(last().params.get('from')).toBe(FROM);
  });

  it('{changed:false} changes nothing on screen, and the next poll sends the same versions', async () => {
    const m = mount();
    await settle();
    const before = m.el.innerHTML;
    const nodes = [row(m.el, 'B1'), row(m.el, 'B2'), tile(m.el, 'buy_volume_thb')];
    reply = () => unchanged(VERSIONS);
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(m.el.innerHTML).toBe(before);
    const after = [row(m.el, 'B1'), row(m.el, 'B2'), tile(m.el, 'buy_volume_thb')];
    after.forEach((n, i) => expect(n, 'the same node').toBe(nodes[i]));
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(3);
    expect(JSON.parse(last().params.get('versions')!)).toEqual(VERSIONS);
  });

  it("one branch moved: only its row and its alerts are replaced, the other row keeps its node, KPIs are replaced", async () => {
    const m = mount();
    await settle();
    const b1 = row(m.el, 'B1');
    expect(alertsOf(m.el, 'staleData').length).toBe(1);
    const moved = { B1: 4, B2: 10 };
    reply = () =>
      answered(moved, {
        period: { from: FROM, to: TO },
        kpis: kpis().map((k) => (k.key === 'buy_volume_thb' ? metric(k.key, '2000000.00', null, k.drilldown) : k)),
        branches: [branchRow('B2', { volumeThb: '999.99' })],
        alerts: [],
        definitions: DEFINITIONS,
      });
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(row(m.el, 'B1'), "B1 did not move: the same node").toBe(b1);
    expect(text(row(m.el, 'B1'))).toContain(formatMoney('2000.50'));
    expect(text(row(m.el, 'B2'))).toContain(formatMoney('999.99'));
    expect(alertsOf(m.el, 'ratePending').length, "B1's alert stays").toBe(1);
    expect(alertsOf(m.el, 'staleData').length, "B2's alerts are B2's new ones: none").toBe(0);
    expect(alertsOf(m.el, 'countDifference').length).toBe(0);
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('2000000.00'));
    reply = () => unchanged(moved);
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(JSON.parse(last().params.get('versions')!)).toEqual(moved);
  });

  it('no request while the tab is hidden; polls again once it is visible', async () => {
    mount();
    await settle();
    hidden = true;
    await vi.advanceTimersByTimeAsync(180_000);
    await settle();
    expect(reads().length, 'hidden: no poll').toBe(1);
    hidden = false;
    await vi.advanceTimersByTimeAsync(61_000);
    await settle();
    expect(reads().length).toBeGreaterThan(1);
  });

  it('no request while the menu is not shown', async () => {
    const m = mount();
    await settle();
    m.setShown(false);
    await settle();
    await vi.advanceTimersByTimeAsync(180_000);
    await settle();
    expect(reads().length).toBe(1);
  });
});

// ── refusals ─────────────────────────────────────────────────────────────────

describe('EX-4 A01: refusals (§8 Refusal; #1071 ruling, gap 7)', () => {
  it('outOfScope: the error banner with errorText, and no tiles, rows or alerts', async () => {
    reply = () => refused(403, 'outOfScope', 'branch B9 is outside the caller scope');
    const m = mount();
    await settle();
    expect(banner(m.el)).toBe(errorText('outOfScope', 'branch B9 is outside the caller scope'));
    expect(tiles(m.el).length).toBe(0);
    expect(m.el.querySelectorAll('[data-branch]').length).toBe(0);
    expect(m.el.querySelectorAll('[data-alert-kind]').length).toBe(0);
  });

  it('validationFailed: the error banner with errorText', async () => {
    reply = () => refused(400, 'validationFailed', 'branches has an empty item');
    const m = mount();
    await settle();
    expect(banner(m.el)).toBe(errorText('validationFailed', 'branches has an empty item'));
    expect(tiles(m.el).length).toBe(0);
  });

  it('a refusal after an answer leaves no partial data on screen', async () => {
    const m = mount();
    await settle();
    expect(tiles(m.el).length).toBe(6);
    reply = () => refused(403, 'outOfScope', 'scope changed');
    const select = m.el.querySelector<HTMLSelectElement>('select[name="channel"]');
    expect(select, 'select[name="channel"]').not.toBeNull();
    select!.value = 'Booking';
    change(select!);
    await settle();
    expect(banner(m.el)).toBe(errorText('outOfScope', 'scope changed'));
    expect(tiles(m.el).length).toBe(0);
    expect(m.el.querySelectorAll('[data-branch]').length).toBe(0);
    expect(m.el.querySelectorAll('[data-alert-kind]').length).toBe(0);
  });
});

// ── language (L-A4) ──────────────────────────────────────────────────────────

describe('EX-4 A01: English (L-A4)', () => {
  it("in en: the tiles' labels, the alert labels and the definitions are the en texts", async () => {
    const { th, en } = texts();
    const enOf = (thText: string): string => {
      const found = Object.entries(th).find(([, v]) => squash(v) === squash(thText));
      expect(found, `${thText} is a value of the th table`).toBeDefined();
      return en[found![0]]!;
    };
    setLang('en');
    const m = mount();
    await settle();
    const buy = text(tile(m.el, 'buy_volume_thb'));
    expect(buy).toContain(enOf('ยอดซื้อ'));
    expect(buy).toContain(DEFINITIONS[0]!.en);
    expect(buy).not.toContain(DEFINITIONS[0]!.th);
    expect(text(theAlert(m.el, 'ratePending'))).toContain(enOf('เรทรออนุมัติ'));
    expect(text(m.el)).not.toContain('ยอดซื้อ');
  });
});
