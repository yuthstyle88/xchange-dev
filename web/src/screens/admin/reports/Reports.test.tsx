// V2-reports (#469): the reports menu's screen (ticket §7–§10 P1–P4; design §12.1, §12.2, §12.6,
// §12.8, §12.14, W7, W10). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; `admin-html:N` = line N of the decoded admin page, ticket §2):
// the four stat cards (`:69`), the payment report (`:70`), the currency report (`:71`) and its
// reference banner (`:103`, `:144`); the served tabs, their keyboard and the rewritten footnote
// (`src/devices-public/admin-workspace.js:99-120`); the export, hidden on the employee tab (`:112`,
// `:147`; `admin-html:170`); the employee summary (`src/devices-public/employee-summary.js`). Every
// figure is the server's (W7): the screen formats the decimal strings of `admin_reports`' answer
// and never computes one — the fixtures below carry nets and totals no browser arithmetic would
// produce from the other figures.
//
// The menu is the record `./menu.ts` exports, mounted as the shell mounts it: its `Component`
// with `branch()`, `period()` and `shown()` (design §12.3), beside the page's one `Dialog` and
// `Toast` (V2-parts). `AdminMenuProps` carries no `ApiClient`: the menu builds its own with
// `new ApiClient()` in its folder (lane controller's decision for every V2 menu, #462). The fake
// server is therefore the global `fetch`: it answers the views by name and records every request.
// The two page-wide reads AppShell makes (`/api/v3/views/me`, `/api/v3/views/kiosk_notifications`)
// are answered and never counted as the menu's own.
//
// The menu's texts are asserted as v2's Thai literals (W6), since the keys of its `i18n.ts` are the
// implementer's. The menu is read through an eager glob, so on a branch without it each test fails
// on an assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { th as partsTh } from '../parts/i18n';
import { requestRefresh } from '../parts/refresh';
import { toast, Toast } from '../parts/toast';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/reports/menu.ts exists').toBeDefined();
  return m!.menu;
}

const REPORTS = 'admin_reports';
const TRANSACTIONS = 'admin_transactions';
const THAI = /[฀-๿]/;
const TABS = ['ช่องทางชำระเงิน', 'สกุลเงินและส่วนต่าง', 'แยกพนักงาน'];
const EXPORT = 'ส่งออกรายการ CSV';
const EMPLOYEE_CSV = 'ดาวน์โหลด CSV';
const FOOTNOTE = 'รวมเฉพาะรายการสำเร็จตามช่วงวันที่และสาขาที่เลือก · ยอดรับ–จ่ายและส่วนต่างเรทไม่ใช่กำไรสุทธิ';
/** v2's base-page footnote, which the served page rewrites (`admin-workspace.js:115`). */
const OLD_FOOTNOTE = 'นับเฉพาะรายการสำเร็จในช่วงวันที่ที่เลือก';
const REFERENCE_NOTE =
  'ไม่รวมค่าบริการและค่าใช้จ่าย ส่วนต่างนี้ไม่ถูกบวกเข้าลิ้นชัก และสำหรับเงินที่รับซื้อไว้ยังไม่ใช่กำไรจากการขายต่อ';
/** v2's LIVE export header, `admin-html:170`. */
const CSV_HEADERS = [
  'Branch ID',
  'Branch',
  'Mode',
  'Document Ref',
  'Date',
  'Type',
  'Currency',
  'Amount',
  'Rate',
  'Principal THB',
  'Fee THB',
  'Bank Fee THB',
  'Settlement THB',
  'Method',
  'Status',
  'Shift ID',
  'Reference Rate',
  'Reference Value THB',
  'Reference Spread THB',
  'Reference Spread %',
];
/** The employee CSV's header, `employee-summary.js:23`. */
const EMPLOYEE_CSV_HEADERS = [
  'พนักงาน',
  'เลขรายการ',
  'วันที่',
  'สถานะ',
  'ช่องทาง',
  'ประเภท',
  'สกุลเงิน',
  'จำนวน',
  'รับเข้า THB',
  'จ่ายออก THB',
  'ค่าบริการ THB',
  'ค่าธรรมเนียมธนาคาร THB',
];

// ── the answer ───────────────────────────────────────────────────────────────

interface Channel {
  method: string;
  count: number;
  received: string;
  paid: string;
  net: string;
}

interface EmployeeGroup {
  staff: string;
  count: number;
  completed: number;
  pending: number;
  cancelled: number;
  received: string;
  paid: string;
  net: string;
  fees: string;
  bankFees: string;
  methods: Channel[];
  currencies: { currency: string; received: string; paid: string }[];
}

interface Totals {
  people: number;
  completed: number;
  received: string;
  paid: string;
  net: string;
}

interface ReportsData {
  period: { from: string; to: string };
  stats: { received: string; paid: string; fees: string; bankFees: string };
  byChannel: Channel[];
  byCurrency: { currency: string; buyAmount: string; sellAmount: string; principal: string; fees: string }[];
  reference: { referenced: number; unreferenced: number; value: string; spread: string; percent: string | null };
  byEmployee: EmployeeGroup[];
  employeeTotals: Totals;
}

function ch(method: string, count: number, received: string, paid: string, net: string): Channel {
  return { method, count, received, paid, net };
}

function methods(over: Record<string, Channel> = {}): Channel[] {
  return ['Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer'].map(
    (m) => over[m] ?? ch(m, 0, '0.00', '0.00', '0.00'),
  );
}

const GROUPS: EmployeeGroup[] = [
  {
    staff: 'anan',
    count: 2,
    completed: 2,
    pending: 0,
    cancelled: 0,
    received: '402.00',
    paid: '59.00',
    net: '343.00',
    fees: '3.00',
    bankFees: '0.50',
    methods: methods({
      Alipay: ch('Alipay', 1, '0.00', '59.00', '-59.00'),
      PromptPay: ch('PromptPay', 1, '402.00', '0.00', '402.00'),
    }),
    currencies: [{ currency: 'USD', received: '2.00', paid: '10.00' }],
  },
  {
    staff: 'dao',
    count: 3,
    completed: 3,
    pending: 0,
    cancelled: 0,
    received: '763.00',
    paid: '3025.00',
    net: '-2262.01',
    fees: '8.00',
    bankFees: '1.00',
    methods: methods({
      Cash: ch('Cash', 2, '0.00', '3025.00', '-3025.00'),
      'Bank Transfer': ch('Bank Transfer', 1, '763.00', '0.00', '763.01'),
    }),
    currencies: [
      { currency: 'USD', received: '101.00', paid: '0.00' },
      { currency: 'EUR', received: '0.00', paid: '20.00' },
    ],
  },
  {
    staff: 'somchai',
    count: 1,
    completed: 1,
    pending: 0,
    cancelled: 0,
    received: '0.00',
    paid: '2000.00',
    net: '-2000.00',
    fees: '0.00',
    bankFees: '0.00',
    methods: methods({ 'Credit Card': ch('Credit Card', 1, '0.00', '2000.00', '-2000.00') }),
    currencies: [{ currency: 'JPY', received: '10000.00', paid: '0.00' }],
  },
];

/** The totals line per `staff|staffQ` as the server answers it; any other filter: nobody. */
const TOTALS: Record<string, Totals> = {
  '|': { people: 3, completed: 6, received: '1165.00', paid: '5084.00', net: '-3919.01' },
  'dao|': { people: 1, completed: 3, received: '763.00', paid: '3025.00', net: '-2262.01' },
  '|O': { people: 2, completed: 4, received: '763.00', paid: '5025.00', net: '-4262.01' },
  '|o': { people: 2, completed: 4, received: '763.00', paid: '5025.00', net: '-4262.01' },
};
const NOBODY: Totals = { people: 0, completed: 0, received: '0.00', paid: '0.00', net: '0.00' };

/**
 * Figures chosen so that nothing the screen could compute matches what it must show: the nets are
 * not received − paid, and the currencies are not in code order.
 */
function report(over: Partial<ReportsData> = {}): ReportsData {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    stats: { received: '1160.00', paid: '3309.00', fees: '71.00', bankFees: '17.50' },
    byChannel: [
      ch('Cash', 1, '0.00', '2950.00', '-2950.00'),
      ch('Bank Transfer', 2, '1120.00', '300.00', '820.01'),
      ch('Credit Card', 1, '0.00', '59.00', '-59.00'),
      ch('Alipay', 0, '0.00', '0.00', '0.00'),
      ch('PromptPay', 1, '40.00', '0.00', '40.00'),
    ],
    byCurrency: [
      { currency: 'USD', buyAmount: '100.00', sellAmount: '10.00', principal: '3400.00', fees: '7.00' },
      { currency: 'EUR', buyAmount: '0.00', sellAmount: '20.00', principal: '760.00', fees: '3.00' },
      { currency: 'JPY', buyAmount: '10000.00', sellAmount: '0.00', principal: '2000.00', fees: '0.00' },
    ],
    reference: { referenced: 2, unreferenced: 3, value: '4131.66', spread: '68.34', percent: '1.65' },
    byEmployee: GROUPS,
    employeeTotals: TOTALS['|']!,
    ...over,
  };
}

function empty(): ReportsData {
  return report({
    stats: { received: '0.00', paid: '0.00', fees: '0.00', bankFees: '0.00' },
    byChannel: ['Cash', 'Bank Transfer', 'Credit Card', 'Alipay', 'PromptPay'].map((m) =>
      ch(m, 0, '0.00', '0.00', '0.00'),
    ),
    byCurrency: [],
    reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: null },
    byEmployee: [],
    employeeTotals: NOBODY,
  });
}

function tx(n: number, over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: `XC-20261006-${String(n).padStart(6, '0')}`,
    date: '2026-10-06',
    created: '2026-10-06T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '30.0000',
    unitSize: '1',
    principal: '3000.00',
    fee: '5.00',
    bankFee: '0.00',
    settlement: '2994.99',
    method: 'Cash',
    customer: `Customer ${n}`,
    customerId: `CUS-${n}`,
    staff: 'dao',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
}

/** Rows numbered `from` down to `to`, newest first, as `admin_transactions` pages them. */
function txs(from: number, to: number, over: Partial<TransactionRow> = {}): TransactionRow[] {
  const out: TransactionRow[] = [];
  for (let n = from; n >= to; n--) out.push(tx(n, over));
  return out;
}

// ── the fake server: the global fetch ────────────────────────────────────────

interface Seen {
  path: string;
  view: string;
  params: URLSearchParams;
  method: string;
}

class FakeServer {
  requests: Seen[] = [];
  version = 7;
  data: ReportsData = report();
  refusal: { status: number; body: unknown } | undefined;
  branches = [
    { id: 'MAIN', name: 'Silom #2' },
    { id: 'B9', name: 'Ari' },
  ];
  /** `admin_transactions`' first page, per `staff` value (`''` = no staff). */
  first: Record<string, TransactionRow[]> = { '': [] };
  /** The rows after `before=<id>`. */
  older: Record<string, TransactionRow[]> = {};

  reads(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === REPORTS).map((r) => r.params);
  }

  walks(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === TRANSACTIONS).map((r) => r.params);
  }

  /** The menu's own requests: everything but AppShell's page-wide reads. */
  own(): Seen[] {
    return this.requests.filter((r) => r.view !== 'me' && r.view !== 'kiosk_notifications');
  }

  answer(url: string, init?: RequestInit): Promise<Response> {
    const u = new URL(url, 'http://desk.test');
    const view = u.pathname.startsWith('/api/v3/views/') ? u.pathname.slice('/api/v3/views/'.length) : '';
    this.requests.push({ path: u.pathname, view, params: u.searchParams, method: (init?.method ?? 'GET').toUpperCase() });
    const ok = (body: unknown) => reply(200, body);
    const p = u.searchParams;
    switch (view) {
      case REPORTS: {
        if (this.refusal) return reply(this.refusal.status, this.refusal.body);
        if (p.get('since') === String(this.version)) return ok({ version: this.version, changed: false });
        const key = `${p.get('staff') ?? ''}|${p.get('staffQ') ?? ''}`;
        const data = { ...this.data, employeeTotals: this.data.byEmployee.length ? (TOTALS[key] ?? NOBODY) : NOBODY };
        return ok({ version: this.version, changed: true, data });
      }
      case TRANSACTIONS: {
        const before = p.get('before');
        if (before !== null) return ok({ version: this.version, changed: true, data: this.older[before] ?? [] });
        return ok({ version: this.version, changed: true, data: this.first[p.get('staff') ?? ''] ?? [] });
      }
      case 'admin_branches':
        return ok({ version: this.version, changed: true, data: { branches: this.branches } });
      case 'transaction_documents':
        return ok({ version: this.version, changed: true, data: { ref: p.get('ref'), files: [] } });
      case 'me':
        return ok({ version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: null } });
      case 'kiosk_notifications':
        return ok({ version: this.version, changed: false });
      default:
        return reply(404, { error: { code: 'notFound', message: 'not found' } });
    }
  }
}

function reply(status: number, body: unknown): Promise<Response> {
  return Promise.resolve(
    new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } }),
  );
}

/** A request's query as a plain object, without the paging key `limit` when asked. */
function keysOf(p: URLSearchParams): Record<string, string> {
  return Object.fromEntries([...p.entries()]);
}

// ── mounting ─────────────────────────────────────────────────────────────────

let server: FakeServer;
let root: HTMLElement;
let dispose: (() => void) | undefined;
let setBranch: Setter<string>;
let setPeriod: Setter<{ from: string; to: string }>;
let setShown: Setter<boolean>;

beforeEach(() => {
  setLang('th');
  server = new FakeServer();
  vi.stubGlobal(
    'fetch',
    vi.fn((input: unknown, init?: RequestInit) => server.answer(String(input), init)),
  );
  location.hash = '#reports';
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  closeDialog();
  toast(''); // the page's one toast outlives a test
  vi.unstubAllGlobals();
  vi.useRealTimers();
  vi.restoreAllMocks();
  location.hash = '';
  setLang('th');
});

function mount(opts: { branch?: string; from?: string; to?: string; shown?: boolean } = {}): HTMLElement {
  const C = theMenu().Component;
  const [branch, sb] = createSignal(opts.branch ?? 'MAIN');
  const [period, sp] = createSignal({ from: opts.from ?? '2026-10-05', to: opts.to ?? '2026-10-06' });
  const [shown, ss] = createSignal(opts.shown ?? true);
  setBranch = sb;
  setPeriod = sp;
  setShown = ss;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () => (
      <>
        <C branch={branch} period={period} shown={shown} />
        <Dialog />
        <Toast />
      </>
    ),
    root,
  );
  return root;
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 10));
}

/** Mounted, read once, settled. */
async function shownReports(opts: Parameters<typeof mount>[0] = {}): Promise<HTMLElement> {
  const el = mount(opts);
  await vi.waitFor(() => expect(server.reads().length).toBeGreaterThan(0));
  await settle();
  return el;
}

// ── helpers on the DOM ───────────────────────────────────────────────────────

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

/** Not hidden itself nor inside a hidden element. */
function visible(el: Element): boolean {
  for (let e: Element | null = el; e; e = e.parentElement) {
    if ((e as HTMLElement).hidden) return false;
    if ((e as HTMLElement).style?.display === 'none') return false;
  }
  return true;
}

function tabs(): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
}

function panels(): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('[role="tabpanel"]')];
}

function activePanel(): HTMLElement {
  const open = panels().filter((p) => !p.hidden);
  expect(open.length, 'exactly one tab panel is shown').toBe(1);
  return open[0]!;
}

async function openTab(name: string): Promise<HTMLElement> {
  const tab = tabs().find((t) => text(t) === name);
  expect(tab, `the tab ${name}`).toBeDefined();
  tab!.click();
  await settle();
  return activePanel();
}

function buttons(el: ParentNode, name: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
}

function visibleButtons(name: string): HTMLButtonElement[] {
  return buttons(root, name).filter(visible);
}

function cards(el: ParentNode): { label: string; value: string; unit: string; note: string }[] {
  return [...el.querySelectorAll<HTMLElement>('.stat-card')].map((c) => ({
    label: text(c.querySelector('.stat-label')),
    value: text(c.querySelector('.stat-value .value')),
    unit: text(c.querySelector('.stat-value .unit')),
    note: text(c.querySelector('.stat-note')),
  }));
}

function tables(el: ParentNode): HTMLTableElement[] {
  return [...el.querySelectorAll<HTMLTableElement>('table')];
}

function headers(table: HTMLTableElement): string[] {
  return [...table.querySelectorAll('thead th')].map((th) => text(th));
}

function rows(table: HTMLTableElement): string[][] {
  return [...table.querySelectorAll('tbody tr')].map((tr) => [...tr.querySelectorAll('td')].map((td) => text(td)));
}

/** The table whose header row is exactly `names`. */
function tableWith(el: ParentNode, names: string[]): HTMLTableElement {
  const found = tables(el).filter((t) => JSON.stringify(headers(t)) === JSON.stringify(names));
  expect(found.length, `one table headed ${names.join(' | ')}`).toBe(1);
  return found[0]!;
}

function toastText(): string {
  return text(root.querySelector('.toast[role="status"]'));
}

function m(v: string): string {
  return formatMoney(v);
}

// ── when it reads ────────────────────────────────────────────────────────────

describe('V2-reports: what it reads, and when (#469 §7, §8; design §12.1, §12.6)', () => {
  it('on show: one read of admin_reports for the branch and the period, nothing written, no rows walked', async () => {
    await shownReports({ branch: 'B9' });
    expect(server.reads().length).toBe(1);
    const q = server.reads()[0]!;
    expect(q.get('branch')).toBe('B9');
    expect(q.get('from')).toBe('2026-10-05');
    expect(q.get('to')).toBe('2026-10-06');
    expect(q.has('since'), 'the first read carries no since').toBe(false);
    expect(q.has('staff') || q.has('staffQ'), 'no employee is picked yet').toBe(false);
    expect(server.walks(), 'no CSV or employee list is read before it is asked for').toEqual([]);
    for (const r of server.own()) {
      expect(['admin_reports', 'admin_branches'], `${r.path} is the menu's own read`).toContain(r.view);
      expect(r.method).toBe('GET');
    }
  });

  it('an open side of the period is not sent', async () => {
    await shownReports({ from: '', to: '2026-10-06' });
    const q = server.reads()[0]!;
    expect(q.get('from') ?? '').toBe('');
    expect(q.get('to')).toBe('2026-10-06');
  });

  it('reads nothing while it is not shown, and reads when it becomes shown', async () => {
    mount({ shown: false });
    await settle();
    expect(server.reads().length).toBe(0);
    setShown(true);
    await vi.waitFor(() => expect(server.reads().length).toBe(1));
  });

  it('a branch or period change reads again without since; the refresh tick reads with the since it holds', async () => {
    await shownReports({ branch: 'MAIN' });
    setBranch('B9');
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(server.reads()[1]!.get('branch')).toBe('B9');
    expect(server.reads()[1]!.has('since'), 'another branch: no since').toBe(false);

    setPeriod({ from: '2026-10-01', to: '2026-10-03' });
    await vi.waitFor(() => expect(server.reads().length).toBe(3));
    expect(keysOf(server.reads()[2]!)).toEqual({ branch: 'B9', from: '2026-10-01', to: '2026-10-03' });

    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(4));
    expect(keysOf(server.reads()[3]!)).toEqual({ branch: 'B9', from: '2026-10-01', to: '2026-10-03', since: '7' });
  });

  it('changed: false keeps what is shown', async () => {
    await shownReports();
    server.version = 7; // the next read carries since=7 and is answered unchanged
    server.data = empty();
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    await settle();
    expect(cards(activePanel()).map((c) => c.value)).toEqual([m('1160.00'), m('3309.00'), m('71.00'), m('17.50')]);
  });

  it('never sends a period whose from is after its to (v2 admin-html:57; the shell shows date-error)', async () => {
    mount({ from: '2026-10-07', to: '2026-10-06' });
    await settle();
    expect(server.reads().length).toBe(0);
  });

  it('switching tabs or language reads nothing', async () => {
    await shownReports();
    await openTab(TABS[1]!);
    await openTab(TABS[2]!);
    await openTab(TABS[0]!);
    setLang('en');
    await settle();
    expect(server.reads().length).toBe(1);
  });

  it('a refused read shows the refusal through errorText', async () => {
    server.refusal = { status: 500, body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } };
    await shownReports();
    expect(text(root)).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });
});

// ── P2: the tabs ─────────────────────────────────────────────────────────────

describe('V2-reports: the three tabs as served (P2; admin-workspace.js:99-120)', () => {
  it('a tablist ประเภทรายงาน of three tabs, the first selected, each controlling its own panel', async () => {
    await shownReports();
    const list = root.querySelector('[role="tablist"]');
    expect(list).not.toBeNull();
    expect(list!.getAttribute('aria-label')).toBe('ประเภทรายงาน');
    expect(tabs().map((t) => text(t))).toEqual(TABS);
    expect(tabs().map((t) => t.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false']);
    expect(tabs().map((t) => t.tabIndex)).toEqual([0, -1, -1]);
    expect(panels().length).toBe(3);
    tabs().forEach((t, i) => {
      const panel = panels()[i]!;
      expect(t.getAttribute('aria-controls')).toBe(panel.id);
      expect(panel.getAttribute('aria-labelledby')).toBe(t.id);
      expect(panel.hidden, `panel ${i}`).toBe(i !== 0);
    });
  });

  it('a click selects a tab and shows only its panel', async () => {
    await shownReports();
    await openTab(TABS[1]!);
    expect(tabs().map((t) => t.getAttribute('aria-selected'))).toEqual(['false', 'true', 'false']);
    expect(tabs().map((t) => t.tabIndex)).toEqual([-1, 0, -1]);
    expect(panels().map((p) => p.hidden)).toEqual([true, false, true]);
  });

  it('ArrowRight / ArrowLeft wrap, Home and End jump; the new tab is selected and focused', async () => {
    await shownReports();
    const press = async (key: string): Promise<void> => {
      const current = tabs().find((t) => t.getAttribute('aria-selected') === 'true')!;
      const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      current.dispatchEvent(e);
      expect(e.defaultPrevented, `${key} is handled`).toBe(true);
      await settle();
    };
    const selected = (): number => tabs().findIndex((t) => t.getAttribute('aria-selected') === 'true');
    for (const [key, expected] of [
      ['ArrowRight', 1],
      ['ArrowRight', 2],
      ['ArrowRight', 0],
      ['ArrowLeft', 2],
      ['Home', 0],
      ['End', 2],
      ['ArrowLeft', 1],
    ] as const) {
      await press(key);
      expect(selected(), key).toBe(expected);
      expect(document.activeElement, `${key}: focus follows`).toBe(tabs()[expected]);
      expect(panels().map((p) => p.hidden)).toEqual([0, 1, 2].map((i) => i !== expected));
    }
  });

  it('the export is shown on the payment and currency tabs and hidden on the employee tab (admin-workspace.js:112)', async () => {
    await shownReports();
    expect(visibleButtons(EXPORT).length).toBe(1);
    await openTab(TABS[1]!);
    expect(visibleButtons(EXPORT).length).toBe(1);
    await openTab(TABS[2]!);
    expect(visibleButtons(EXPORT).length, 'hidden on แยกพนักงาน').toBe(0);
    expect(visibleButtons(EMPLOYEE_CSV).length, "the employee CSV is its own").toBe(1);
    await openTab(TABS[0]!);
    expect(visibleButtons(EXPORT).length).toBe(1);
  });
});

// ── P1: payment channels ─────────────────────────────────────────────────────

describe("V2-reports: the payment tab — the server's figures, formatted, never computed (P1; W7)", () => {
  it('four stat cards with the labels, notes and unit of admin-html:69', async () => {
    await shownReports();
    expect(cards(activePanel())).toEqual([
      { label: 'รับเงินบาทรวม', value: m('1160.00'), unit: 'THB', note: 'ยอดชำระจากรายการร้านขาย' },
      { label: 'จ่ายเงินบาทรวม', value: m('3309.00'), unit: 'THB', note: 'ยอดชำระจากรายการร้านรับซื้อ' },
      { label: 'ค่าบริการลูกค้า', value: m('71.00'), unit: 'THB', note: 'รวมอยู่ในยอดชำระแล้ว' },
      { label: 'ค่าธรรมเนียมธนาคาร', value: m('17.50'), unit: 'THB', note: 'แยกจากยอดชำระลูกค้า' },
    ]);
  });

  it("the payment report: v2's card, the five channels in the server's order with v2's names, the served footnote", async () => {
    await shownReports();
    const panel = activePanel();
    const all = text(panel);
    for (const t of ['SETTLEMENT SUMMARY', 'สรุปตามช่องทางชำระเงิน', 'THB', FOOTNOTE]) expect(all).toContain(t);
    expect(all).not.toContain(OLD_FOOTNOTE);
    const table = tableWith(panel, ['ช่องทาง', 'จำนวนรายการ', 'รับเข้า', 'จ่ายออก', 'รับเข้า − จ่ายออก']);
    expect(rows(table)).toEqual([
      [partsTh.methodCash, '1', m('0.00'), m('2950.00'), m('-2950.00')],
      [partsTh.methodBankTransfer, '2', m('1120.00'), m('300.00'), m('820.01')],
      [partsTh.methodCreditCard, '1', m('0.00'), m('59.00'), m('-59.00')],
      [partsTh.methodAlipay, '0', m('0.00'), m('0.00'), m('0.00')],
      [partsTh.methodPromptPay, '1', m('40.00'), m('0.00'), m('40.00')],
    ]);
    expect(text(root)).not.toContain('SCB Transfer');
  });

  it('no exchange in the period: zeros from the server and the five rows still there (ticket §9 row 1)', async () => {
    server.data = empty();
    await shownReports();
    const panel = activePanel();
    expect(cards(panel).map((c) => c.value)).toEqual(['0.00', '0.00', '0.00', '0.00']);
    const table = tableWith(panel, ['ช่องทาง', 'จำนวนรายการ', 'รับเข้า', 'จ่ายออก', 'รับเข้า − จ่ายออก']);
    expect(rows(table).map((r) => r.slice(1))).toEqual(Array(5).fill(['0', '0.00', '0.00', '0.00']));
  });
});

// ── P1: currencies and the reference comparison ──────────────────────────────

describe('V2-reports: the currency tab (P1; admin-html:71, :103, :144)', () => {
  it("the currency report: v2's card and its rows in the server's order", async () => {
    await shownReports();
    const panel = await openTab(TABS[1]!);
    const all = text(panel);
    for (const t of ['CURRENCY BREAKDOWN', 'สรุปตามสกุลเงิน']) expect(all).toContain(t);
    const table = tableWith(panel, [
      'สกุลเงิน',
      'ร้านรับซื้อ (หน่วยเงิน)',
      'ร้านขาย (หน่วยเงิน)',
      'เงินต้นรวม THB',
      'ค่าบริการ THB',
    ]);
    expect(rows(table)).toEqual([
      ['USD', m('100.00'), m('10.00'), m('3400.00'), m('7.00')],
      ['EUR', m('0.00'), m('20.00'), m('760.00'), m('3.00')],
      ['JPY', m('10000.00'), m('0.00'), m('2000.00'), m('0.00')],
    ]);
  });

  it("the reference banner: the server's spread and percent, the counts, v2's note", async () => {
    await shownReports();
    const all = text(await openTab(TABS[1]!));
    expect(all).toContain(`ส่วนต่างเทียบเรทอ้างอิงในช่วงวันที่: ${m('68.34')} THB (1.65%)`);
    expect(all).toContain(`2 รายการมีเรทอ้างอิง · 3 รายการยังไม่ระบุ · ${REFERENCE_NOTE}`);
  });

  it('a negative spread is shown as the server signs it', async () => {
    server.data = report({
      reference: { referenced: 3, unreferenced: 2, value: '8251.66', spread: '-51.66', percent: '-0.63' },
    });
    await shownReports();
    const all = text(await openTab(TABS[1]!));
    expect(all).toContain(`ส่วนต่างเทียบเรทอ้างอิงในช่วงวันที่: ${m('-51.66')} THB (-0.63%)`);
  });

  it('no exchange in the period: v2\'s empty table and ยังไม่มีเรทอ้างอิง', async () => {
    server.data = empty();
    await shownReports();
    const panel = await openTab(TABS[1]!);
    const all = text(panel);
    expect(all).toContain(partsTh.emptyTransactions);
    expect(all).toContain(partsTh.emptyTransactionsNote);
    expect(all).toContain('ส่วนต่างเทียบเรทอ้างอิงในช่วงวันที่: ยังไม่มีเรทอ้างอิง');
    expect(all).toContain(`0 รายการมีเรทอ้างอิง · 0 รายการยังไม่ระบุ · ${REFERENCE_NOTE}`);
  });
});

// ── P3: the employee summary ─────────────────────────────────────────────────

const SUMMARY_HEADERS = [
  'พนักงาน',
  'ทั้งหมด',
  'สำเร็จ',
  'ค้าง',
  'ยกเลิก',
  'รับเข้า THB',
  'จ่ายออก THB',
  'สุทธิ THB',
  'ค่าบริการ',
  'ค่าธรรมเนียมธนาคาร',
];

function summaryRow(g: EmployeeGroup): string[] {
  return [
    g.staff,
    String(g.count),
    String(g.completed),
    String(g.pending),
    String(g.cancelled),
    m(g.received),
    m(g.paid),
    m(g.net),
    m(g.fees),
    m(g.bankFees),
  ];
}

function sections(panel: HTMLElement): HTMLDetailsElement[] {
  return [...panel.querySelectorAll<HTMLDetailsElement>('details')];
}

function select(panel: HTMLElement): HTMLSelectElement {
  const s = panel.querySelector<HTMLSelectElement>('select');
  expect(s, 'the employee select').not.toBeNull();
  return s!;
}

function search(panel: HTMLElement): HTMLInputElement {
  const s = panel.querySelector<HTMLInputElement>('input[type="search"]');
  expect(s, 'the employee search').not.toBeNull();
  return s!;
}

async function employeeTab(opts: Parameters<typeof mount>[0] = {}): Promise<HTMLElement> {
  await shownReports(opts);
  return openTab(TABS[2]!);
}

describe('V2-reports: the employee tab (P3; employee-summary.js)', () => {
  it("v2's heading, note, controls and totals line, with every employee of the period in the select", async () => {
    const panel = await employeeTab();
    const all = text(panel);
    for (const t of [
      'สรุปยอดแยกพนักงาน',
      'อิงสาขาและช่วงวันที่ด้านบน · รับเข้า/จ่ายออกจากมุมร้าน หน่วย THB · รวมเฉพาะรายการสำเร็จ · ค่าบริการรวมในยอดรับ–จ่ายแล้ว ค่าธรรมเนียมธนาคารแสดงแยก · ยอดสุทธิไม่ใช่กำไร',
      'ค้นหาพนักงาน',
    ]) {
      expect(all).toContain(t);
    }
    const s = select(panel);
    expect(text(s.closest('label'))).toContain('พนักงาน');
    expect([...s.options].map((o) => [o.value, text(o)])).toEqual([
      ['', 'ทุกคน'],
      ['anan', 'anan'],
      ['dao', 'dao'],
      ['somchai', 'somchai'],
    ]);
    expect(s.value).toBe('');
    expect(search(panel).placeholder).toBe('ชื่อบัญชี / รหัสพนักงาน');
    expect(all).toContain(
      `รวม 3 คน · สำเร็จ 6 รายการ · รับเข้า ${m('1165.00')} · จ่ายออก ${m('5084.00')} · สุทธิ ${m('-3919.01')} THB`,
    );
  });

  it("the summary table: one row per employee with the server's figures, in the server's order", async () => {
    const panel = await employeeTab();
    expect(rows(tableWith(panel, SUMMARY_HEADERS))).toEqual(GROUPS.map(summaryRow));
  });

  it("each employee's section: channels with the summary's names, foreign currency in units, closed while everyone is shown", async () => {
    const panel = await employeeTab();
    const secs = sections(panel);
    expect(secs.length).toBe(3);
    expect(secs.map((d) => text(d.querySelector('summary')))).toEqual([
      'anan · 2 รายการ — ดูแยกช่องทางและทุกรายการ',
      'dao · 3 รายการ — ดูแยกช่องทางและทุกรายการ',
      'somchai · 1 รายการ — ดูแยกช่องทางและทุกรายการ',
    ]);
    expect(secs.map((d) => d.open), 'employee-summary.js:14: open only when one is picked').toEqual([false, false, false]);
    const dao = secs[1]!;
    const all = text(dao);
    for (const t of ['รับ–จ่ายเงินบาท แยกช่องทาง', 'เงินต่างประเทศที่รับ–จ่าย', 'รายการทั้งหมดของพนักงาน']) {
      expect(all).toContain(t);
    }
    expect(rows(tableWith(dao, ['ช่องทาง', 'สำเร็จ', 'รับเข้า THB', 'จ่ายออก THB', 'สุทธิ THB']))).toEqual([
      [partsTh.methodCash, '2', m('0.00'), m('3025.00'), m('-3025.00')],
      [partsTh.methodCreditCard, '0', m('0.00'), m('0.00'), m('0.00')],
      ['Alipay', '0', m('0.00'), m('0.00'), m('0.00')],
      ['PromptPay', '0', m('0.00'), m('0.00'), m('0.00')],
      [partsTh.methodBankTransfer, '1', m('763.00'), m('0.00'), m('763.01')],
    ]);
    expect(rows(tableWith(dao, ['สกุลเงิน', 'รับเข้า (หน่วยเงิน)', 'จ่ายออก (หน่วยเงิน)']))).toEqual([
      ['USD', m('101.00'), m('0.00')],
      ['EUR', m('0.00'), m('20.00')],
    ]);
  });

  it('picking an employee reads the totals with staff, shows only that employee, and opens their section', async () => {
    const panel = await employeeTab();
    const s = select(panel);
    s.value = 'dao';
    s.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(keysOf(server.reads()[1]!), 'a new filter: no since').toEqual({
      branch: 'MAIN',
      from: '2026-10-05',
      to: '2026-10-06',
      staff: 'dao',
    });
    await settle();
    const now = activePanel();
    expect(rows(tableWith(now, SUMMARY_HEADERS))).toEqual([summaryRow(GROUPS[1]!)]);
    expect(sections(now).map((d) => text(d.querySelector('summary')))).toEqual([
      'dao · 3 รายการ — ดูแยกช่องทางและทุกรายการ',
    ]);
    expect(sections(now)[0]!.open).toBe(true);
    expect(text(now)).toContain(
      `รวม 1 คน · สำเร็จ 3 รายการ · รับเข้า ${m('763.00')} · จ่ายออก ${m('3025.00')} · สุทธิ ${m('-2262.01')} THB`,
    );
    expect([...select(now).options].map((o) => o.value), 'the select still lists everyone').toEqual([
      '',
      'anan',
      'dao',
      'somchai',
    ]);
    expect(select(now).value).toBe('dao');

    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(3));
    expect(keysOf(server.reads()[2]!), 'the refresh keeps the filter and sends its since').toEqual({
      branch: 'MAIN',
      from: '2026-10-05',
      to: '2026-10-06',
      staff: 'dao',
      since: '7',
    });
  });

  it('searching reads the totals with staffQ and shows the employees whose name holds it, case folded', async () => {
    const panel = await employeeTab();
    const input = search(panel);
    input.value = 'O';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() =>
      expect(server.reads().some((q) => q.get('staffQ') === 'O' && !q.has('since') && !q.has('staff'))).toBe(true),
    );
    await settle();
    const now = activePanel();
    expect(rows(tableWith(now, SUMMARY_HEADERS))).toEqual([summaryRow(GROUPS[1]!), summaryRow(GROUPS[2]!)]);
    expect(text(now)).toContain(
      `รวม 2 คน · สำเร็จ 4 รายการ · รับเข้า ${m('763.00')} · จ่ายออก ${m('5025.00')} · สุทธิ ${m('-4262.01')} THB`,
    );
    expect(search(now).value, 'the search keeps what was typed').toBe('O');
  });

  it("a section's list is admin_transactions with that employee and the period; an id opens the detail", async () => {
    server.first.dao = [
      tx(3, { type: 'SELL', currency: 'EUR', method: 'SCB Transfer', settlement: '763.00' }),
      tx(2),
      tx(1, { amount: '1.00', settlement: '30.00' }),
    ];
    const panel = await employeeTab();
    const dao = sections(panel)[1]!;
    dao.open = true;
    dao.dispatchEvent(new Event('toggle'));
    await vi.waitFor(() =>
      expect(server.walks().some((q) => q.get('staff') === 'dao')).toBe(true),
    );
    const q = server.walks().find((w) => w.get('staff') === 'dao')!;
    expect(q.get('branch')).toBe('MAIN');
    expect(q.get('from')).toBe('2026-10-05');
    expect(q.get('to')).toBe('2026-10-06');
    expect(q.has('staffQ')).toBe(false);
    await vi.waitFor(() => expect(text(sections(activePanel())[1])).toContain(tx(3).id));
    const ids = [...sections(activePanel())[1]!.querySelectorAll('button')]
      .map((b) => text(b))
      .filter((t) => t.startsWith('XC-'));
    expect(ids, "the server's order: newest first").toEqual([tx(3).id, tx(2).id, tx(1).id]);

    const before = server.own().length;
    buttons(sections(activePanel())[1]!, tx(2).id)[0]!.click();
    await settle();
    const detail = document.querySelector('.transaction-detail');
    expect(detail, "V2-parts' TransactionDetail is shown").not.toBeNull();
    expect(text(detail)).toContain(tx(2).id);
    const docs = server.own().slice(before).filter((r) => r.view === 'transaction_documents');
    expect(docs.length).toBe(1);
    expect(docs[0]!.params.get('ref')).toBe(tx(2).id);
  });

  it('no exchange in the period: an empty summary and the zero totals line', async () => {
    server.data = empty();
    const panel = await employeeTab();
    expect(sections(panel)).toEqual([]);
    expect([...select(panel).options].map((o) => o.value)).toEqual(['']);
    expect(text(panel)).toContain(
      `รวม 0 คน · สำเร็จ 0 รายการ · รับเข้า 0.00 · จ่ายออก 0.00 · สุทธิ 0.00 THB`,
    );
  });
});

// ── P4: the CSVs ─────────────────────────────────────────────────────────────

describe('V2-reports: CSV (P4; admin-html:170, employee-summary.js:22-25; design §12.8)', () => {
  let blobs: Blob[];
  let names: string[];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  beforeEach(() => {
    blobs = [];
    names = [];
    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download);
    });
    vi.useFakeTimers({ toFake: ['Date'], shouldAdvanceTime: true });
    // 03:30 on 2026-10-09 in Bangkok; still 2026-10-08 in UTC
    vi.setSystemTime(new Date('2026-10-08T20:30:00.000Z'));
  });

  afterEach(() => {
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  });

  async function csvText(): Promise<string> {
    await vi.waitFor(() => expect(blobs).toHaveLength(1));
    const bytes = new Uint8Array(await blobs[0]!.arrayBuffer());
    expect([...bytes.slice(0, 3)], 'a BOM').toEqual([0xef, 0xbb, 0xbf]);
    return new TextDecoder().decode(bytes.slice(3));
  }

  function quoted(cells: string[]): string {
    return cells.map((c) => `"${c.replaceAll('"', '""')}"`).join(',');
  }

  it("ส่งออกรายการ CSV walks admin_transactions with the period only, to the end, and writes v2's LIVE file oldest first", async () => {
    const first = txs(300, 201);
    first[0] = tx(300, {
      type: 'SELL',
      method: 'Bank Transfer',
      amount: '1500.50',
      rate: '32.6000',
      principal: '48916.30',
      fee: '5.25',
      bankFee: '1.50',
      settlement: '48921.55',
      referenceRate: '32.8731',
      referenceValue: '49326.09',
      referenceSpread: '409.79',
      referencePercent: '0.830775',
      shiftId: null,
    });
    server.first[''] = first;
    server.older[tx(201).id] = [tx(200), tx(199)];
    // an employee is picked on the employee tab: the report's export ignores it (admin-html:170 period())
    const panel = await employeeTab();
    const s = select(panel);
    s.value = 'dao';
    s.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    await openTab(TABS[0]!);
    await settle();

    const before = server.walks().length;
    visibleButtons(EXPORT)[0]!.click();
    const text = await csvText();
    const walk = server.walks().slice(before).map(keysOf);
    const filter = { branch: 'MAIN', limit: '100', from: '2026-10-05', to: '2026-10-06' };
    expect(walk).toEqual([filter, { ...filter, before: tx(201).id }]);
    const lines = text.split('\r\n');
    expect(lines).toHaveLength(1 + 102);
    expect(lines[0]).toBe(quoted(CSV_HEADERS));
    expect(
      lines.slice(1).map((l) => l.split(',')[3]),
      "v2's period() order: the oldest first (admin-html:57, :170)",
    ).toEqual([tx(199), tx(200), ...txs(300, 201).reverse()].map((r) => `"${r.id}"`));
    expect(lines[102]).toBe(
      quoted([
        'MAIN', 'Silom #2', 'LIVE', tx(300).id, '2026-10-06', 'SELL', 'USD', '1500.50', '32.6000', '48916.30',
        '5.25', '1.50', '48921.55', 'Bank Transfer', 'COMPLETED', '', '32.8731', '49326.09', '409.79', '0.830775',
      ]),
    );
    expect(lines[1]).toBe(
      quoted([
        'MAIN', 'Silom #2', 'LIVE', tx(199).id, '2026-10-06', 'BUY', 'USD', '100.00', '30.0000', '3000.00',
        '5.00', '0.00', '2994.99', 'Cash', 'COMPLETED', 'SH-1', '', '', '', '',
      ]),
    );
    expect(names).toEqual(['XCHANGE-LIVE-Silom__2-2026-10-09.csv']);
  });

  it('the export works from the currency tab too, for the branch shown', async () => {
    server.first[''] = [tx(1)];
    await shownReports({ branch: 'B9' });
    await openTab(TABS[1]!);
    visibleButtons(EXPORT)[0]!.click();
    const [, row] = (await csvText()).split('\r\n');
    expect(row!.startsWith(quoted(['B9', 'Ari', 'LIVE', tx(1).id]))).toBe(true);
    expect(server.walks()[0]!.get('branch')).toBe('B9');
    expect(names).toEqual(['XCHANGE-LIVE-Ari-2026-10-09.csv']);
  });

  it('with nothing to export, toasts ไม่มีรายการให้ส่งออก and hands over no file (admin-html:170)', async () => {
    await shownReports();
    visibleButtons(EXPORT)[0]!.click();
    await vi.waitFor(() => expect(toastText()).toContain('ไม่มีรายการให้ส่งออก'));
    await settle();
    expect(blobs).toHaveLength(0);
    expect(names).toHaveLength(0);
  });

  it("ดาวน์โหลด CSV walks admin_transactions with the employee filter and writes v2's employee file oldest first", async () => {
    server.first.dao = [
      tx(3, {
        type: 'SELL',
        currency: 'EUR',
        amount: '20.00',
        method: 'SCB Transfer',
        fee: '3.00',
        bankFee: '1.00',
        settlement: '763.00',
      }),
      tx(2, { method: 'PromptPay' }),
      tx(1, { amount: '1.00', fee: '0.00', settlement: '30.00', staff: '=cmd' }),
    ];
    const panel = await employeeTab();
    const s = select(panel);
    s.value = 'dao';
    s.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    await settle();
    const before = server.walks().length;
    visibleButtons(EMPLOYEE_CSV)[0]!.click();
    const text = await csvText();
    const walk = server.walks().slice(before).map(keysOf);
    expect(walk).toEqual([{ branch: 'MAIN', limit: '100', from: '2026-10-05', to: '2026-10-06', staff: 'dao' }]);
    const lines = text.split('\r\n');
    expect(lines[0]).toBe(quoted(EMPLOYEE_CSV_HEADERS));
    expect(lines.slice(1)).toEqual([
      quoted(["'=cmd", tx(1).id, '2026-10-06', 'COMPLETED', partsTh.methodCash, 'BUY', 'USD', '1.00', '0', '30.00', '0.00', '0.00']),
      quoted(['dao', tx(2).id, '2026-10-06', 'COMPLETED', 'PromptPay', 'BUY', 'USD', '100.00', '0', '2994.99', '5.00', '0.00']),
      quoted(['dao', tx(3).id, '2026-10-06', 'COMPLETED', partsTh.methodBankTransfer, 'SELL', 'EUR', '20.00', '763.00', '0', '3.00', '1.00']),
    ]);
    expect(names).toEqual(['XC-Employees-2026-10-05-2026-10-06.csv']);
  });

  it('the employee CSV carries the search as staffQ and an open period side as an empty name part', async () => {
    server.first[''] = [tx(1)];
    const panel = await employeeTab({ from: '' });
    const input = search(panel);
    input.value = 'O';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => expect(server.reads().some((q) => q.get('staffQ') === 'O')).toBe(true));
    await settle();
    visibleButtons(EMPLOYEE_CSV)[0]!.click();
    await csvText();
    expect(keysOf(server.walks()[0]!)).toEqual({ branch: 'MAIN', limit: '100', to: '2026-10-06', staffQ: 'O' });
    expect(names).toEqual(['XC-Employees--2026-10-06.csv']);
  });
});

// ── language (W10) ───────────────────────────────────────────────────────────

describe('V2-reports: its texts go through its table (W10)', () => {
  it('in English, no tab of the menu shows Thai of its own', async () => {
    setLang('en');
    await shownReports();
    const thai = (): string[] =>
      [...root.querySelectorAll('*')]
        .flatMap((el) => [
          ...[...el.childNodes].filter((n) => n.nodeType === Node.TEXT_NODE).map((n) => n.textContent ?? ''),
          el.getAttribute('placeholder') ?? '',
          el.getAttribute('aria-label') ?? '',
          el.getAttribute('title') ?? '',
        ])
        .filter((t) => THAI.test(t));
    expect(thai()).toEqual([]);
    for (const tab of tabs()) {
      tab.click();
      await settle();
      expect(thai(), text(tab)).toEqual([]);
    }
    expect(cards(panels()[0]!).map((c) => c.value)).toEqual([m('1160.00'), m('3309.00'), m('71.00'), m('17.50')]);
  });
});
