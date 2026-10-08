// V2-cash (#467): the cash menu's screen (ticket §7–§10 rows A1–A4, A6, §9 edge cases; design
// §12.2, §12.6, §12.9, W7, W10). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; `admin-html:N` = line N of the decoded admin page, ticket §2):
// the status banner, the four stat cards, the reference summary, the stock table and the button
// states (`:101-108`); the dialogs (`:125-130`); the example (`:145`); the opening picker
// (`src/devices-public/admin-opening-currencies.js`); the stock filter and the history
// (`admin-workspace.js:122-135`). LIVE only (design §12.5). Every figure is the server's (W7): the
// screen formats the decimal strings of `admin_cash`, `admin_cash_movements` and `admin_shifts`
// and never computes one — the fixtures below carry figures that do not add up, so a screen that
// computed them would show something else. The close dialog shows the server's expected figures
// and echoes the counts as typed; the difference is shown only from the server's answer after the
// close (design §12.9, ticket §9 row 4).
//
// Panel A5 (the valuation and profit calculators) is not this ticket's (controller,
// issuecomment-6066362902): the shift detail renders without it — no panel, no placeholder, no
// TODO text, no input, no compute request.
//
// The menu is the record `./menu.ts` exports, mounted as the shell mounts it: its `Component` with
// `branch()`, `period()` and `shown()` (design §12.3), beside the page's one `Dialog` and `Toast`
// (V2-parts). `AdminMenuProps` carries no `ApiClient`: the menu builds its own with
// `new ApiClient()` in its folder (lane controller's decision for every V2 menu, #462). The fake
// server is therefore the global `fetch` that client calls: it answers the three views and
// `/api/rpc`, and records every request. The two page-wide reads AppShell makes
// (`/api/v3/views/me`, `/api/v3/views/kiosk_notifications`) are answered and never counted as the
// menu's own. Writes go through V2-parts' `pendingWrite` (one store per page), so every test
// leaves no write pending.
//
// The menu is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { th as partsTh } from '../parts/i18n';
import { pendingWrite } from '../parts/pending-write';
import { requestRefresh } from '../parts/refresh';
import { Toast } from '../parts/toast';

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
  expect(m, 'web/src/screens/admin/cash/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/cash/i18n.ts exists').toBeDefined();
  return m!;
}

const CASH = '/api/v3/views/admin_cash';
const MOVES = '/api/v3/views/admin_cash_movements';
const SHIFTS = '/api/v3/views/admin_shifts';
const VIEWS = [CASH, MOVES, SHIFTS];
/** AppShell's page-wide reads: answered, never the menu's own requests. */
const PAGE_WIDE = ['/api/v3/views/me', '/api/v3/views/kiosk_notifications'];
const ROLE_REFUSAL = 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้';

/** Panel A5's texts (`shift-valuation.js:9`, `:13-14`; `profit-summary.js:7-13`). */
const A5_TEXTS = [
  'กำไรส่วนต่างจากการตีราคา (ประมาณการ)',
  'เปิดอัตราแลกเปลี่ยน ธปท. ↗',
  'วันที่เรทที่ ธปท. เผยแพร่',
  'คำนวณส่วนต่าง',
  'เงินคงเหลือรวมเป็นบาท',
  'กำไรและยอดถอนได้',
  'กำไรที่เกิดขึ้นแล้ว',
  'ยอดถอนได้',
  'รอข้อมูล',
  'ข้อมูลที่ใช้คำนวณ',
  'คำนวณกำไรและยอดถอนได้',
];

// ── the answers ──────────────────────────────────────────────────────────────

interface Amount {
  code: string;
  amount: string;
}
interface StockRow {
  code: string;
  opening: string;
  net: string;
  expected: string;
}
interface CashData {
  shift: { id: string; openedAt: string; staff: string } | null;
  stock: StockRow[];
  stats: { opening: string; incoming: string; outgoing: string; remaining: string } | null;
  reference: { withReference: number; withoutReference: number; spread: string; percent: string };
  currencies: string[];
  lastCounted: Amount[] | null;
}
interface MovementRow {
  id: string;
  created: string;
  shiftId: string;
  direction: string;
  currency: string;
  amount: string;
  reason: string;
  staff: string;
}
interface Balance {
  code: string;
  expected: string;
  counted: string | null;
  difference: string | null;
}
interface ShiftRow {
  id: string;
  status: string;
  openedAt: string;
  staff: string;
  note: string;
  difference: Amount[] | null;
  balances: Balance[];
}

/** The register the server answers here: deliberately short, so a hard-coded list shows. */
const REGISTER = ['THB', 'EUR', 'USD'];

/** An open shift whose figures do not add up: what is shown must be what is answered. */
function openCash(): CashData {
  return {
    shift: { id: 'S-0001', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-007' },
    stock: [
      { code: 'THB', opening: '100000.00', net: '-2230.00', expected: '97770.00' },
      { code: 'USD', opening: '1000.00', net: '7.25', expected: '1100.50' },
      { code: 'EUR', opening: '0.00', net: '0.00', expected: '0.00' },
    ],
    stats: { opening: '100000.00', incoming: '920.00', outgoing: '3150.00', remaining: '12345.67' },
    reference: { withReference: 2, withoutReference: 1, spread: '240.00', percent: '6.74' },
    currencies: REGISTER,
    lastCounted: [
      { code: 'THB', amount: '5000.00' },
      { code: 'USD', amount: '100.00' },
    ],
  };
}

function noShift(): CashData {
  return {
    shift: null,
    stock: [],
    stats: null,
    reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
    currencies: REGISTER,
    lastCounted: null,
  };
}

function movements(): MovementRow[] {
  // the server's order: not the order of `created`
  return [
    { id: 'MV-2', created: '2026-10-06T03:00:00.000Z', shiftId: 'S-0001', direction: 'OUT', currency: 'USD', amount: '10.25', reason: 'ส่งเซฟ', staff: 'EMP-007' },
    { id: 'MV-1', created: '2026-10-06T04:00:00.000Z', shiftId: 'S-0000', direction: 'IN', currency: 'THB', amount: '500.00', reason: 'เงินเติมลิ้นชัก', staff: 'EMP-002' },
  ];
}

function shifts(): ShiftRow[] {
  return [
    {
      id: 'S-0001', status: 'OPEN', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-007', note: '', difference: null,
      balances: [
        { code: 'THB', expected: '97770.00', counted: null, difference: null },
        { code: 'USD', expected: '1100.50', counted: null, difference: null },
      ],
    },
    {
      id: 'S-0000', status: 'CLOSED', openedAt: '2026-10-05T02:00:00.000Z', staff: 'EMP-002', note: 'USD ขาด 5 · EUR เกิน 2',
      difference: [
        { code: 'EUR', amount: '2.00' },
        { code: 'USD', amount: '-5.00' },
      ],
      balances: [
        { code: 'THB', expected: '1000.00', counted: '1000.00', difference: '0.00' },
        { code: 'EUR', expected: '10.00', counted: '12.00', difference: '2.00' },
        { code: 'USD', expected: '100.00', counted: '95.00', difference: '-5.00' },
      ],
    },
    {
      id: 'S-OLD', status: 'CLOSED', openedAt: '2026-10-04T02:00:00.000Z', staff: 'EMP-003', note: '', difference: [],
      balances: [{ code: 'THB', expected: '2000.00', counted: '2000.00', difference: '0.00' }],
    },
  ];
}

// ── the fake server: the global fetch ────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}
interface Post {
  action: string;
  payload: Record<string, unknown>;
}
type Reply = { status: number; body: unknown };

class Server {
  version = 7;
  cash: CashData = openCash();
  /** The answer to `all=true`; the plain answer when unset. */
  cashAll: CashData | null = null;
  moves: MovementRow[] = movements();
  shifts: ShiftRow[] = shifts();
  /** A refusal `admin_cash` answers instead of its data. */
  cashRefusal: Reply | null = null;
  sent: Sent[] = [];
  posts: Post[] = [];
  /** How `/api/rpc` answers an action; `undefined` = saved. */
  rpc: (action: string, payload: Record<string, unknown>) => Reply | 'network' | 'hang' | undefined = () =>
    undefined;
  /** Called after an accepted write, before the answer: the server's new state. */
  afterWrite: ((action: string) => void) | undefined;
  private held: (() => void) | undefined;

  release(): void {
    const go = this.held;
    this.held = undefined;
    go?.();
  }

  own(): Sent[] {
    return this.sent.filter((s) => !PAGE_WIDE.includes(s.path));
  }

  reads(path: string): Sent[] {
    return this.sent.filter((s) => s.path === path);
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    this.sent.push({ path: url.pathname, params: url.searchParams, method });
    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'content-type': 'application/json' },
      });
    if (url.pathname === '/api/v3/views/me') {
      return json({ status: 200, body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } } });
    }
    if (url.pathname === '/api/v3/views/kiosk_notifications') {
      return json({ status: 200, body: { version: 1, changed: false } });
    }
    if (method === 'GET' && url.pathname === CASH) {
      if (this.cashRefusal) return json(this.cashRefusal);
      if (url.searchParams.get('since') === String(this.version)) {
        return json({ status: 200, body: { version: this.version, changed: false } });
      }
      const data = url.searchParams.get('all') === 'true' && this.cashAll ? this.cashAll : this.cash;
      return json({ status: 200, body: { version: this.version, changed: true, data } });
    }
    if (method === 'GET' && url.pathname === MOVES) {
      return json({ status: 200, body: { version: this.version, changed: true, data: this.moves } });
    }
    if (method === 'GET' && url.pathname === SHIFTS) {
      return json({ status: 200, body: { version: this.version, changed: true, data: this.shifts } });
    }
    if (method === 'POST' && url.pathname === '/api/rpc') {
      const body = JSON.parse(String(init?.body ?? '{}')) as Post;
      this.posts.push(body);
      if (body.action === 'adminAbandonRequest') {
        return json({ status: 200, body: { ok: true, requestResolution: 'CANCELLED' } });
      }
      const how = this.rpc(body.action, body.payload);
      if (how === 'network') throw new TypeError('Failed to fetch');
      if (how === 'hang') {
        await new Promise<void>((resolve) => {
          this.held = resolve;
        });
      } else if (how) {
        return json(how);
      }
      this.version += 1;
      this.afterWrite?.(body.action);
      return json({ status: 200, body: { ok: true } });
    }
    return json({ status: 500, body: { error: { code: 'serverError', message: `unexpected ${method} ${url.pathname}` } } });
  };
}

let server: Server;

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement;

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
  setPeriod: (p: { from: string; to: string }) => void;
  setShown: (s: boolean) => void;
}

function mount(init: { branch?: string; shown?: boolean } = {}): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown, setShown] = createSignal(init.shown ?? true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return { el: root, setBranch, setPeriod, setShown };
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  location.hash = '#cash';
  setLang('th');
});

afterEach(async () => {
  server.release();
  await settle();
  // V2-parts' pendingWrite is one store per page: leave no write pending for the next test
  await pendingWrite.abandon();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

// ── helpers on the DOM ───────────────────────────────────────────────────────

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function norm(s: string): string {
  return s.replace(/\s+/g, ' ').trim();
}

function when(iso: string): string {
  return norm(new Date(iso).toLocaleString('th-TH'));
}

function buttonsNamed(el: ParentNode, name: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = buttonsNamed(el, name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

function cards(el: HTMLElement): { label: string; value: string; unit: string; note: string }[] {
  return [...el.querySelectorAll<HTMLElement>('.stat-card')].map((c) => ({
    label: text(c.querySelector('.stat-label')),
    value: text(c.querySelector('.stat-value .value')),
    unit: text(c.querySelector('.stat-value .unit')),
    note: text(c.querySelector('.stat-note')),
  }));
}

/** The table whose header row holds `header`, as rows of cell texts. */
function tableWith(el: ParentNode, header: string): string[][] {
  const tables = [...el.querySelectorAll('table')].filter((t) =>
    [...t.querySelectorAll('th')].some((th) => text(th) === header),
  );
  expect(tables.length, `one table with the header ${JSON.stringify(header)}`).toBe(1);
  return [...tables[0]!.querySelectorAll('tbody tr')].map((tr) =>
    [...tr.querySelectorAll('td')].map((td) => text(td)),
  );
}

function headersOf(el: ParentNode, header: string): string[] {
  const table = [...el.querySelectorAll('table')].find((t) =>
    [...t.querySelectorAll('th')].some((th) => text(th) === header),
  );
  expect(table, `a table with the header ${JSON.stringify(header)}`).toBeDefined();
  return [...table!.querySelectorAll('th')].map((th) => text(th));
}

function dialog(): HTMLDialogElement {
  const d = document.querySelector<HTMLDialogElement>('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

function dialogTitle(): string {
  return text(dialog().querySelector('h2'));
}

/**
 * The control of the label whose own text starts with `label`: nested in it, or named by `for`.
 */
function field<T extends HTMLElement = HTMLInputElement>(within: ParentNode, label: string): T {
  const labels = [...within.querySelectorAll<HTMLLabelElement>('label')].filter((l) => {
    const clone = l.cloneNode(true) as HTMLElement;
    clone.querySelectorAll('input,select,textarea,small,span').forEach((n) => n.remove());
    return norm(clone.textContent ?? '').startsWith(label);
  });
  expect(labels.length, `one field labelled ${JSON.stringify(label)}`).toBe(1);
  const l = labels[0]!;
  const nested = l.querySelector<T>('input,select,textarea');
  const named = l.htmlFor ? document.getElementById(l.htmlFor) : null;
  const control = (nested ?? named) as T | null;
  expect(control, `the control of ${JSON.stringify(label)}`).not.toBeNull();
  return control!;
}

function type(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function toastText(): string | null {
  const status = document.querySelector<HTMLElement>('.toast[role="status"]');
  if (!status || status.hidden) return null;
  const t = text(status);
  return t === '' ? null : t;
}

/** The rows of the open dialog's selected currencies, by code (admin-opening-currencies.js:6). */
function openingRows(): string[] {
  return [...dialog().querySelectorAll<HTMLLabelElement>('label')]
    .map((l) => {
      const clone = l.cloneNode(true) as HTMLElement;
      clone.querySelectorAll('input,select,textarea').forEach((n) => n.remove());
      return norm(clone.textContent ?? '');
    })
    .filter((t) => /^[A-Z]{3} · /.test(t))
    .map((t) => t.slice(0, 3));
}

function writes(): Post[] {
  return server.posts.filter((p) => p.action !== 'adminAbandonRequest');
}

// ── when it reads ────────────────────────────────────────────────────────────

describe('V2-cash: what it reads, and when (#467 §7; design §12.1, §12.6)', () => {
  it('on show: one read of each view for the branch — admin_cash without since or all, the movements by 100 — and nothing else', async () => {
    mount({ branch: 'B9' });
    await settle();
    for (const view of VIEWS) {
      expect(server.reads(view).length, view).toBe(1);
      const q = server.reads(view)[0]!.params;
      expect(q.get('branch'), view).toBe('B9');
      expect(q.has('since'), `${view}: the first read carries no since`).toBe(false);
    }
    expect(server.reads(CASH)[0]!.params.get('all') ?? 'false', 'the shown currencies by default').toBe('false');
    expect(server.reads(MOVES)[0]!.params.get('limit'), "v2's newest 100 (admin-html:106)").toBe('100');
    expect(server.own().map((s) => s.path).sort()).toEqual([...VIEWS].sort());
    expect(server.own().every((s) => s.method === 'GET'), 'reading writes nothing').toBe(true);
  });

  it('reads nothing while it is not shown, and reads when it becomes shown', async () => {
    const m = mount({ shown: false });
    await settle();
    expect(server.own().length).toBe(0);
    m.setShown(true);
    await settle();
    for (const view of VIEWS) expect(server.reads(view).length, view).toBe(1);
  });

  it('a branch change reads all three again without since; the period is not its business (not dated)', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    m.setBranch('B9');
    await settle();
    for (const view of VIEWS) {
      expect(server.reads(view).length, view).toBe(2);
      expect(server.reads(view)[1]!.params.get('branch'), view).toBe('B9');
      expect(server.reads(view)[1]!.params.has('since'), `${view}: another branch, no since`).toBe(false);
    }
    const before = server.own().length;
    m.setPeriod({ from: '2026-10-01', to: '2026-10-03' });
    await settle();
    expect(server.own().length, 'a period change reads nothing').toBe(before);
  });

  it('the refresh tick reads admin_cash with the since it holds; changed: false keeps what is shown', async () => {
    const m = mount();
    await settle();
    expect(text(m.el)).toContain(formatMoney('12345.67'));
    requestRefresh();
    await settle();
    const reads = server.reads(CASH);
    expect(reads.length).toBe(2);
    expect(reads[1]!.params.get('since')).toBe('7');
    expect(reads[1]!.params.get('branch')).toBe('MAIN');
    expect(text(m.el), 'unchanged: the figures stay').toContain(formatMoney('12345.67'));
    expect(text(m.el)).toContain('S-0001');
  });

  it('the stock filter reads admin_cash with all=true, without since, and shows that answer', async () => {
    server.cashAll = { ...openCash(), stock: [...openCash().stock, { code: 'GBP', opening: '0.00', net: '0.00', expected: '0.00' }] };
    const m = mount();
    await settle();
    expect(tableWith(m.el, 'ยกมาตอนเปิดกะ').map((r) => r[0])).toEqual(['THB', 'USD', 'EUR']);
    const box = field(m.el, 'แสดงทุกสกุล รวมยอดศูนย์');
    expect(box.type).toBe('checkbox');
    expect(box.checked).toBe(false);
    box.click();
    await settle();
    const reads = server.reads(CASH);
    expect(reads.length).toBe(2);
    expect(reads[1]!.params.get('all')).toBe('true');
    expect(reads[1]!.params.has('since'), 'another filter: no since (design §12.1)').toBe(false);
    expect(tableWith(m.el, 'ยกมาตอนเปิดกะ').map((r) => r[0])).toEqual(['THB', 'USD', 'EUR', 'GBP']);
    box.click();
    await settle();
    expect(server.reads(CASH).length).toBe(3);
    expect(server.reads(CASH)[2]!.params.get('all') ?? 'false').toBe('false');
    expect(server.reads(CASH)[2]!.params.has('since')).toBe(false);
  });
});

// ── A1: banner, stats, reference summary, buttons ────────────────────────────

describe('V2-cash: the banner, the stats and the buttons (A1; admin-html:101-108)', () => {
  it('no shift: the banner, four cards at —, no reference rate yet, open enabled, move and close disabled', async () => {
    server.cash = noShift();
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('ยังไม่มีกะเปิด');
    expect(all).toContain('ตรวจนับเงินสดและเงินต่างประเทศ แล้วกรอกยอดยกมาตอนเปิดกะ');
    expect(cards(m.el)).toEqual([
      { label: 'เงินสดตั้งต้น', value: '—', unit: 'THB', note: 'ยอด THB ที่ตรวจนับตอนเปิดกะ' },
      { label: 'เงินสดรับเข้า', value: '—', unit: 'THB', note: 'จากแลกเงินและรับเงินเข้าลิ้นชัก' },
      { label: 'เงินสดจ่ายออก', value: '—', unit: 'THB', note: 'จ่ายให้ลูกค้าและจ่ายนอกการแลกเงิน' },
      { label: 'เงินสด THB คงเหลือ', value: '—', unit: 'THB', note: 'ตั้งต้น + รับเข้า − จ่ายออก' },
    ]);
    expect(all).toContain('ส่วนต่างเทียบเรทอ้างอิงในกะ: ยังไม่มีเรทอ้างอิง');
    expect(all).toContain('0 รายการมีเรทอ้างอิง · 0 รายการยังไม่ระบุ · ไม่รวมค่าบริการและค่าใช้จ่าย');
    expect(all).toContain('เริ่มจากยอดเงินที่ตรวจนับจริง');
    expect(all).toContain('รายการก่อนเปิดกะจะไม่ถูกนำมาหักซ้ำจากยอดยกมา');
    expect(button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').disabled).toBe(false);
    expect(button(m.el, 'รับ–จ่ายเงินสด').disabled).toBe(true);
    expect(button(m.el, 'ตรวจนับและปิดกะ').disabled).toBe(true);
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(false);
    for (const t of ['CASH & CURRENCY STOCK', 'เงินสดและสต็อกตามกะ']) expect(all).toContain(t);
    expect(all).toContain(
      'เงินสด THB ขยับเฉพาะรายการชำระเงินสด เงินต่างประเทศขยับทั้งรายการเงินสดและโอนธนาคาร ค่าธรรมเนียมธนาคารไม่หักจากลิ้นชักโดยอัตโนมัติ',
    );
  });

  it("an open shift: its id, when and by whom, the server's four figures, the reference summary; open disabled", async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('กะกำลังเปิด · S-0001');
    expect(all).toContain(`เปิด ${when('2026-10-06T02:30:00.000Z')} · EMP-007 · ยอดกะไม่ขึ้นกับตัวกรองวันที่ของรายงาน`);
    expect(all).not.toContain('ยังไม่มีกะเปิด');
    expect(cards(m.el).map((c) => [c.label, c.value])).toEqual([
      ['เงินสดตั้งต้น', formatMoney('100000.00')],
      ['เงินสดรับเข้า', formatMoney('920.00')],
      ['เงินสดจ่ายออก', formatMoney('3150.00')],
      ['เงินสด THB คงเหลือ', formatMoney('12345.67')],
    ]);
    expect(all).toContain(`ส่วนต่างเทียบเรทอ้างอิงในกะ: ${formatMoney('240.00')} THB (6.74%)`);
    expect(all).toContain('2 รายการมีเรทอ้างอิง · 1 รายการยังไม่ระบุ · ไม่รวมค่าบริการและค่าใช้จ่าย');
    expect(button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').disabled).toBe(true);
    expect(button(m.el, 'รับ–จ่ายเงินสด').disabled).toBe(false);
    expect(button(m.el, 'ตรวจนับและปิดกะ').disabled).toBe(false);
  });

  it('a refused read: the refusal is shown and the three write buttons are disabled (v2 :108, offline)', async () => {
    server.cashRefusal = { status: 500, body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } } };
    const m = mount();
    await settle();
    expect(text(m.el)).toContain('เซิร์ฟเวอร์ขัดข้อง');
    for (const name of ['ตั้งเงินสดเริ่มต้น / เปิดกะ', 'รับ–จ่ายเงินสด', 'ตรวจนับและปิดกะ']) {
      expect(button(m.el, name).disabled, name).toBe(true);
    }
  });

  it('while a write is pending every write button is disabled (pendingWrite.busy, v2 :108)', async () => {
    server.cash = noShift();
    server.rpc = () => 'hang';
    const m = mount();
    await settle();
    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();
    type(field(dialog(), 'THB · '), '100');
    button(dialog(), 'ยืนยันเปิดกะ').click();
    await settle();
    expect(writes().length).toBe(1);
    for (const name of ['ตั้งเงินสดเริ่มต้น / เปิดกะ', 'รับ–จ่ายเงินสด', 'ตรวจนับและปิดกะ']) {
      expect(button(m.el, name).disabled, `${name} while pending`).toBe(true);
    }
    server.release();
    await settle();
    expect(toastText()).toBe(partsTh.serverSaved);
  });
});

// ── A2: the stock table, the filter, the history ─────────────────────────────

describe('V2-cash: the stock table, its filter and the history (A2; admin-html:105-107, admin-workspace.js:122-135)', () => {
  it("the stock rows as the server answered them, in its order, formatted, never computed", async () => {
    const m = mount();
    await settle();
    expect(headersOf(m.el, 'ยกมาตอนเปิดกะ')).toEqual(['สกุลเงิน', 'ยกมาตอนเปิดกะ', 'เคลื่อนไหวสุทธิ', 'คงเหลือตามระบบ']);
    expect(tableWith(m.el, 'ยกมาตอนเปิดกะ')).toEqual([
      ['THB', formatMoney('100000.00'), formatMoney('-2230.00'), formatMoney('97770.00')],
      ['USD', formatMoney('1000.00'), formatMoney('7.25'), formatMoney('1100.50')],
      ['EUR', '0.00', '0.00', '0.00'],
    ]);
    const all = text(m.el);
    expect(all).toContain('แสดงทุกสกุล รวมยอดศูนย์');
    expect(all).toContain('ปกติแสดง THB สกุลที่ตั้งเรท และสกุลที่มียอดหรือการเคลื่อนไหว');
  });

  it('the history sits in one <details>: the movements and the shifts, newest first as answered', async () => {
    const m = mount();
    await settle();
    const details = [...m.el.querySelectorAll('details')].filter(
      (d) => text(d.querySelector('summary')) === 'ประวัติรับ–จ่ายและกะที่ผ่านมา',
    );
    expect(details.length).toBe(1);
    const history = details[0]!;
    expect(text(history)).toContain('ประวัติรับ–จ่ายนอกการแลกเงิน');
    expect(text(history)).toContain('ประวัติกะ');
    expect(headersOf(history, 'เหตุผล')).toEqual(['วัน / เวลา', 'กะ', 'รายการ', 'จำนวน', 'เหตุผล']);
    expect(tableWith(history, 'เหตุผล')).toEqual([
      [when('2026-10-06T03:00:00.000Z'), 'S-0001', 'จ่ายออก', `${formatMoney('10.25')} USD`, 'ส่งเซฟ'],
      [when('2026-10-06T04:00:00.000Z'), 'S-0000', 'รับเข้า', `${formatMoney('500.00')} THB`, 'เงินเติมลิ้นชัก'],
    ]);
    expect(headersOf(history, 'ผลต่างตอนปิด')).toEqual(['กะ', 'เปิดเมื่อ', 'ผู้เปิด', 'สถานะ', 'ผลต่างตอนปิด']);
    expect(tableWith(history, 'ผลต่างตอนปิด')).toEqual([
      ['S-0001', when('2026-10-06T02:30:00.000Z'), 'EMP-007', 'กำลังเปิด', '—'],
      ['S-0000', when('2026-10-05T02:00:00.000Z'), 'EMP-002', 'ปิดแล้ว', `EUR ${formatMoney('2.00')} · USD ${formatMoney('-5.00')}`],
      ['S-OLD', when('2026-10-04T02:00:00.000Z'), 'EMP-003', 'ปิดแล้ว', 'ตรงตามระบบ'],
    ]);
  });

  it('empty history: the sentences of admin-html:106-107', async () => {
    server.cash = noShift();
    server.moves = [];
    server.shifts = [];
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('ยังไม่มีรายการรับ–จ่าย');
    expect(all).toContain('ยังไม่มีประวัติกะ');
  });
});

// ── A3: opening a shift ──────────────────────────────────────────────────────

describe('V2-cash: the open dialog and its currency picker (A3; admin-html:127, admin-opening-currencies.js)', () => {
  it("prefills the last count, searches the server's register, adds and removes, and sends what was typed", async () => {
    server.cash = { ...noShift(), lastCounted: [{ code: 'THB', amount: '5000.00' }, { code: 'USD', amount: '100.00' }] };
    server.afterWrite = () => {
      server.cash = openCash();
    };
    const m = mount({ branch: 'B9' });
    await settle();
    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();
    expect(dialogTitle()).toBe('ตั้งเงินสดเริ่มต้น / เปิดกะ');
    expect(text(dialog())).toContain(
      'กรอกยอดเงินจริงเฉพาะสกุลที่มี ค้นหาแล้วกดเพิ่มสกุลเงินได้ สกุลที่ไม่ได้เพิ่มจะบันทึกเป็น 0 · เพิ่มสกุลที่มียอดจากกะก่อนให้แล้ว กรุณาตรวจนับยืนยันอีกครั้ง',
    );
    expect(openingRows()).toEqual(['THB', 'USD']);
    expect(field(dialog(), 'THB · เงินสดตั้งต้น').value).toBe('5000.00');
    expect(field(dialog(), 'USD · ').value).toBe('100.00');
    expect(server.own().filter((s) => !VIEWS.includes(s.path)).length, 'the dialog reads nothing').toBe(0);

    const search = field(dialog(), 'ค้นหาและเพิ่มสกุลเงิน');
    expect(search.getAttribute('placeholder')).toBe('เช่น USD, ดอลลาร์, EUR');
    type(search, 'gbp');
    await settle();
    expect(text(dialog()), "GBP is not in the server's register").toContain('ไม่พบสกุลเงิน หรือเพิ่มสกุลนี้แล้ว');
    type(search, 'usd');
    await settle();
    expect(text(dialog()), 'USD is added already').toContain('ไม่พบสกุลเงิน หรือเพิ่มสกุลนี้แล้ว');
    type(search, 'eur');
    await settle();
    const add = [...dialog().querySelectorAll('button')].filter((b) => text(b).startsWith('+ '));
    expect(add.map((b) => text(b).slice(0, 5))).toEqual(['+ EUR']);
    add[0]!.click();
    await settle();
    expect(openingRows()).toEqual(['THB', 'USD', 'EUR']);
    expect(field(dialog(), 'EUR · ').value, 'an added currency starts empty').toBe('');
    expect(search.value, 'the search is cleared').toBe('');

    const remove = [...dialog().querySelectorAll('button')].filter((b) => b.getAttribute('aria-label') === 'ลบสกุลเงิน USD');
    expect(remove.length).toBe(1);
    expect(text(remove[0]!)).toBe('ลบ');
    expect(
      [...dialog().querySelectorAll('button')].some((b) => b.getAttribute('aria-label') === 'ลบสกุลเงิน THB'),
      'THB cannot be removed',
    ).toBe(false);
    remove[0]!.click();
    await settle();
    expect(openingRows()).toEqual(['THB', 'EUR']);

    type(field(dialog(), 'THB · เงินสดตั้งต้น'), '4800');
    type(field(dialog(), 'EUR · '), '20.5');
    button(dialog(), 'ยืนยันเปิดกะ').click();
    await settle();

    expect(writes().length).toBe(1);
    const { action, payload } = writes()[0]!;
    expect(action).toBe('adminOpenShift');
    expect(Object.keys(payload).sort()).toEqual(['branchId', 'opening', 'requestId']);
    expect(payload.branchId).toBe('B9');
    expect(String(payload.requestId)).toMatch(/^[0-9a-f-]{36}$/);
    const opening = payload.opening as Record<string, unknown>;
    expect(opening.THB, 'as typed, a string (W7)').toBe('4800');
    expect(opening.EUR).toBe('20.5');
    for (const [code, value] of Object.entries(opening)) {
      if (code !== 'THB' && code !== 'EUR') {
        expect(['0', 0, '0.00'], `${code}: not added, so 0 (admin-opening-currencies.js:19)`).toContain(value);
      }
    }
    expect(toastText()).toBe(partsTh.serverSaved);
    expect(document.querySelector('dialog')?.open ?? false, 'the dialog closes').toBe(false);
    expect(server.reads(CASH).length, 'the views are read again after the write').toBeGreaterThan(1);
    expect(text(m.el)).toContain('กะกำลังเปิด · S-0001');
  });

  it('no shift ever closed: only THB at 0, the note without the prefill sentence (ticket §9 row 3)', async () => {
    server.cash = noShift();
    const m = mount();
    await settle();
    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();
    const note = text(dialog());
    expect(note).toContain('กรอกยอดเงินจริงเฉพาะสกุลที่มี ค้นหาแล้วกดเพิ่มสกุลเงินได้ สกุลที่ไม่ได้เพิ่มจะบันทึกเป็น 0');
    expect(note).not.toContain('เพิ่มสกุลที่มียอดจากกะก่อนให้แล้ว');
    expect(openingRows()).toEqual(['THB']);
    expect(field(dialog(), 'THB · เงินสดตั้งต้น').value).toBe('0');
  });

  it('Enter in the search adds the only match', async () => {
    server.cash = noShift();
    const m = mount();
    await settle();
    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();
    const search = field(dialog(), 'ค้นหาและเพิ่มสกุลเงิน');
    type(search, 'usd');
    await settle();
    search.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle();
    expect(openingRows()).toEqual(['THB', 'USD']);
    expect(writes().length, 'Enter in the search does not submit').toBe(0);
  });

  it("a refused open — a second open shift, or a role the server refuses — shows the server's sentence (§8, §9 row 1)", async () => {
    server.cash = noShift();
    server.rpc = () => ({ status: 400, body: { error: 'มีกะเปิดอยู่แล้ว กรุณารีเฟรช' } });
    const m = mount();
    await settle();
    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();
    button(dialog(), 'ยืนยันเปิดกะ').click();
    await settle();
    expect(dialogTitle()).toBe(partsTh.notConfirmed);
    expect(text(dialog())).toContain('มีกะเปิดอยู่แล้ว กรุณารีเฟรช');

    server.rpc = () => ({ status: 403, body: { error: ROLE_REFUSAL } });
    button(dialog(), partsTh.retry).click();
    await settle();
    expect(text(dialog())).toContain(ROLE_REFUSAL);
    expect(writes().length).toBe(2);
    expect(writes()[1]!.payload.requestId, 'the retry keeps the request id').toBe(writes()[0]!.payload.requestId);
  });

  it('a write with no answer: the pending dialog, and the retry sends the same request (§9 row 2)', async () => {
    server.cash = noShift();
    let failNext = true;
    server.rpc = () => {
      if (failNext) {
        failNext = false;
        return 'network';
      }
      return undefined;
    };
    const m = mount();
    await settle();
    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();
    type(field(dialog(), 'THB · เงินสดตั้งต้น'), '100');
    button(dialog(), 'ยืนยันเปิดกะ').click();
    await settle();
    expect(dialogTitle()).toBe(partsTh.notConfirmed);
    expect(text(dialog())).toContain(partsTh.notConfirmedNote);
    button(dialog(), partsTh.retry).click();
    await settle();
    expect(writes().length).toBe(2);
    expect(writes()[1]).toEqual(writes()[0]);
    expect(toastText()).toBe(partsTh.serverSaved);
  });
});

// ── A4: movement, close, shift detail ────────────────────────────────────────

describe('V2-cash: the movement dialog (A4; admin-html:128)', () => {
  it("v2's form over the server's register; sends the typed amount and the trimmed reason", async () => {
    const m = mount();
    await settle();
    button(m.el, 'รับ–จ่ายเงินสด').click();
    await settle();
    expect(dialogTitle()).toBe('บันทึกรับ–จ่ายเงินสด');
    expect(text(dialog())).toContain(
      'สำหรับนำเงินเข้าลิ้นชัก นำฝาก หรือค่าใช้จ่ายเงินสด ไม่ต้องบันทึกซ้ำรายการแลกเงิน ระบบนี้บันทึกบัญชีเท่านั้น ไม่ได้โอนเงินจริง',
    );
    const currency = field<HTMLSelectElement>(dialog(), 'สกุลเงิน');
    expect([...currency.options].map((o) => o.value)).toEqual(REGISTER);
    const direction = field<HTMLSelectElement>(dialog(), 'ประเภท');
    expect([...direction.options].map((o) => [o.value, text(o)])).toEqual([
      ['IN', 'รับเข้า'],
      ['OUT', 'จ่ายออก'],
    ]);
    const reason = field(dialog(), 'เหตุผล / อ้างอิง');
    expect(reason.getAttribute('placeholder')).toBe('เช่น เงินเติมลิ้นชัก หรือ นำเงินฝากธนาคาร');
    expect(reason.maxLength).toBe(200);
    choose(currency, 'USD');
    choose(direction, 'OUT');
    type(field(dialog(), 'จำนวน'), '250.50');
    type(reason, '  นำเงินฝากธนาคาร  ');
    button(dialog(), 'บันทึกรับ–จ่าย').click();
    await settle();
    expect(writes().length).toBe(1);
    const { action, payload } = writes()[0]!;
    expect(action).toBe('adminMoveStock');
    expect(Object.keys(payload).sort()).toEqual(['amount', 'branchId', 'currency', 'direction', 'reason', 'requestId', 'shiftId']);
    expect(payload).toMatchObject({
      shiftId: 'S-0001',
      currency: 'USD',
      direction: 'OUT',
      amount: '250.50',
      reason: 'นำเงินฝากธนาคาร',
      branchId: 'MAIN',
    });
    expect(toastText()).toBe(partsTh.serverSaved);
  });

  it("a blank reason is v2's sentence and nothing is sent", async () => {
    const m = mount();
    await settle();
    button(m.el, 'รับ–จ่ายเงินสด').click();
    await settle();
    type(field(dialog(), 'จำนวน'), '10');
    type(field(dialog(), 'เหตุผล / อ้างอิง'), '   ');
    button(dialog(), 'บันทึกรับ–จ่าย').click();
    await settle();
    expect(writes().length).toBe(0);
    const alert = dialog().querySelector('[role="alert"]');
    expect(text(alert)).toBe('กรุณาระบุเหตุผล');
  });

  it('a move refused for the role is shown as the server answered it (§8: move as BACKEND)', async () => {
    server.rpc = () => ({ status: 403, body: { error: ROLE_REFUSAL } });
    const m = mount();
    await settle();
    button(m.el, 'รับ–จ่ายเงินสด').click();
    await settle();
    type(field(dialog(), 'จำนวน'), '10');
    type(field(dialog(), 'เหตุผล / อ้างอิง'), 'ทดสอบ');
    button(dialog(), 'บันทึกรับ–จ่าย').click();
    await settle();
    expect(text(dialog())).toContain(ROLE_REFUSAL);
  });
});

describe('V2-cash: the close dialog (A4; admin-html:129; design §12.9)', () => {
  it("shows the server's expected figures, echoes the counts as typed, computes no difference, and sends both", async () => {
    server.afterWrite = (action) => {
      if (action !== 'adminCloseShift') return;
      server.cash = noShift();
      server.shifts = [
        { ...shifts()[0]!, status: 'CLOSED', note: 'USD เกิน', difference: [{ code: 'USD', amount: '134.00' }] },
        ...shifts().slice(1),
      ];
    };
    const m = mount();
    await settle();
    button(m.el, 'ตรวจนับและปิดกะ').click();
    await settle();
    expect(dialogTitle()).toBe('ตรวจนับและปิดกะ');
    const thb = field(dialog(), 'THB · นับได้จริง');
    const usd = field(dialog(), 'USD');
    const eur = field(dialog(), 'EUR');
    const all = () => text(dialog());
    expect(all()).toContain(formatMoney('97770.00'));
    expect(all()).toContain(formatMoney('1100.50'));
    expect(all(), "v2's live difference column is not ported").not.toContain('นับได้ − ตามระบบ');
    type(thb, '97770');
    type(usd, '1234.5');
    type(eur, '0');
    await settle();
    expect(all(), 'the count is echoed as typed').toContain('1234.5');
    expect(all(), 'no difference while typing (134.00)').not.toContain('134.00');
    expect(all()).not.toContain('-134.00');
    const note = field(dialog(), 'เหตุผลยอดต่าง / หมายเหตุ');
    expect(note.getAttribute('placeholder')).toBe('ต้องระบุเมื่อมีเงินขาดหรือเกิน');
    type(note, ' USD เกิน ');
    expect(buttonsNamed(dialog(), 'กลับไปตรวจนับ').length).toBe(1);
    button(dialog(), 'ยืนยันปิดกะ').click();
    await settle();

    expect(writes().length).toBe(1);
    const { action, payload } = writes()[0]!;
    expect(action).toBe('adminCloseShift');
    expect(Object.keys(payload).sort()).toEqual(['branchId', 'counted', 'expected', 'note', 'requestId', 'shiftId']);
    expect(payload.shiftId).toBe('S-0001');
    expect(payload.note).toBe('USD เกิน');
    expect(payload.counted).toMatchObject({ THB: '97770', USD: '1234.5', EUR: '0' });
    expect(payload.expected, "the server's expected figures, sent back for its drift check").toMatchObject({
      THB: '97770.00',
      USD: '1100.50',
    });
    for (const value of Object.values(payload.counted as Record<string, unknown>)) {
      expect(typeof value, 'counts as typed strings (W7)').toBe('string');
    }
    expect(toastText()).toBe(partsTh.serverSaved);
    // the difference comes from the server's answer after the close (design §12.9)
    expect(server.reads(SHIFTS).length).toBeGreaterThan(1);
    const rows = tableWith(m.el, 'ผลต่างตอนปิด');
    expect(rows[0]!.slice(3)).toEqual(['ปิดแล้ว', `USD ${formatMoney('134.00')}`]);
  });
});

describe('V2-cash: the shift detail (A4; admin-html:130) — without panel A5', () => {
  it('a closed shift: its title, who, the expected / counted / difference rows and the note; nothing read', async () => {
    const m = mount();
    await settle();
    const before = server.own().length;
    button(m.el, 'S-0000').click();
    await settle();
    expect(dialogTitle()).toBe('ผลตรวจนับกะที่ปิดแล้ว');
    const d = dialog();
    expect(text(d)).toContain('S-0000 · EMP-002');
    expect(headersOf(d, 'นับจริง')).toEqual(['สกุลเงิน', 'ตามระบบ', 'นับจริง', 'ผลต่าง']);
    expect(tableWith(d, 'นับจริง')).toEqual([
      ['THB', formatMoney('1000.00'), formatMoney('1000.00'), '0.00'],
      ['EUR', '10.00', '12.00', '2.00'],
      ['USD', '100.00', '95.00', '-5.00'],
    ]);
    expect(text(d)).toContain('USD ขาด 5 · EUR เกิน 2');
    expect(server.own().length, 'the detail is the row already read').toBe(before);
    button(d, 'ปิด').click();
    await settle();
    expect(document.querySelector('dialog')?.open ?? false).toBe(false);
  });

  it("an open shift: the server's running expected, — where nothing was counted", async () => {
    const m = mount();
    await settle();
    button(m.el, 'S-0001').click();
    await settle();
    expect(dialogTitle()).toBe('รายละเอียดกะที่เปิด');
    expect(tableWith(dialog(), 'นับจริง')).toEqual([
      ['THB', formatMoney('97770.00'), '—', '—'],
      ['USD', formatMoney('1100.50'), '—', '—'],
    ]);
  });

  it('renders no calculator: no A5 text, no input, no form, no placeholder or TODO, no compute request', async () => {
    const m = mount();
    await settle();
    for (const id of ['S-0000', 'S-0001']) {
      button(m.el, id).click();
      await settle();
      const d = dialog();
      const all = text(d);
      for (const t of A5_TEXTS) expect(all.includes(t), `${id}: no ${JSON.stringify(t)}`).toBe(false);
      expect(d.querySelectorAll('input, select, textarea, form').length, `${id}: nothing to type into`).toBe(0);
      expect(/\bTODO\b|coming soon|เร็ว ๆ นี้|เร็วๆ นี้|ยังไม่รองรับ/i.test(all), `${id}: no placeholder`).toBe(false);
      closeDialog();
      await settle();
    }
    expect(server.sent.some((s) => s.path.startsWith('/api/v3/compute')), 'no compute action is called').toBe(false);
    expect(server.own().every((s) => VIEWS.includes(s.path))).toBe(true);
  });
});

// ── A6: the example ──────────────────────────────────────────────────────────

describe('V2-cash: ดูตัวอย่าง 500,000 (A6; admin-html:145)', () => {
  it("v2's worked example, verbatim, read from nothing and written nowhere", async () => {
    server.cash = noShift();
    const m = mount();
    await settle();
    const before = server.own().length;
    button(m.el, 'ดูตัวอย่าง 500,000').click();
    await settle();
    expect(dialogTitle()).toBe('ตัวอย่างเงินสดตั้งต้น 500,000 บาท');
    const all = text(dialog());
    for (const t of [
      'ตัวอย่างคำนวณเท่านั้น ไม่บันทึกหรือเปลี่ยนยอดของคุณ',
      'เงินสดตั้งต้น',
      '500,000.00 THB',
      'ลูกค้าขาย USD ให้ร้าน',
      '10,000.00 USD',
      'เรทอ้างอิง',
      '32.50 → 325,000.00 THB',
      'เรทที่ร้านจ่ายลูกค้า',
      '31.59 → 315,900.00 THB',
      'ส่วนต่างเทียบเรทอ้างอิง',
      '9,100.00 THB · 2.80%',
      'เงินสด THB คงเหลือ',
      '184,100.00 THB',
      'สต็อก USD หลังรับซื้อ',
      'มุมมองร้านคือ “รับซื้อ USD” ร้านจ่าย THB ออก ส่วนต่าง 9,100 บาทเป็นมูลค่าเทียบเรทอ้างอิง ยังไม่ได้รับเป็นเงินสดเพิ่ม และสมมติว่าไม่มีค่าบริการ',
    ]) {
      expect(all, t).toContain(t);
    }
    expect(server.own().length).toBe(before);
    button(dialog(), 'เข้าใจแล้ว').click();
    await settle();
    expect(document.querySelector('dialog')?.open ?? false).toBe(false);
  });
});

// ── languages ────────────────────────────────────────────────────────────────

describe('V2-cash: setLang (W10 point 5)', () => {
  it('labels switch to the en table; the figures keep their digits; nothing is read again', async () => {
    const { th, en } = texts();
    const key = (thai: string): string => {
      const found = Object.entries(th).find(([, v]) => v === thai);
      expect(found, `${thai} is in the menu's table`).toBeDefined();
      return found![0];
    };
    const m = mount();
    await settle();
    const reads = server.own().length;
    setLang('en');
    await settle();
    const all = text(m.el);
    for (const thai of ['เงินสดรับเข้า', 'ตั้งเงินสดเริ่มต้น / เปิดกะ', 'เงินสดและสต็อกตามกะ', 'ประวัติรับ–จ่ายและกะที่ผ่านมา']) {
      expect(all).toContain(en[key(thai)]!);
      expect(all).not.toContain(thai);
    }
    expect(cards(m.el).map((c) => c.value)).toEqual([
      formatMoney('100000.00'),
      formatMoney('920.00'),
      formatMoney('3150.00'),
      formatMoney('12345.67'),
    ]);
    expect(server.own().length, 'a language switch reads nothing').toBe(reads);
  });
});
