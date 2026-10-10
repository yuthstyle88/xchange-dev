// V2-transactions (#464): the admin's transaction list (ticket §7, §9, §10 T1–T5; design §12.1,
// §12.2, §12.6, §12.8, §12.14). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// The menu is mounted as the shell mounts it: its `Component` with `branch`, `period` and `shown`
// accessors, beside the page's one `<Dialog/>` and `<Toast/>` (V2-parts). `AdminMenuProps`
// carries no `ApiClient`, so the server is a fake `fetch` (stubbed globally) that answers as the
// `admin_transactions` view does: a version; `since` equal to it → `changed: false`; otherwise the
// first page; `before=<id>` → the older rows the test gives. Assertions read the requests' query
// keys, so the menu may use `ApiClient.view` for the first page (it carries `since`) and
// `ApiClient.page` / `walkPages` for older pages (they carry `before`). Reads AppShell makes
// (`/api/v3/views/me`, `/api/v3/views/kiosk_notifications`) are answered and never counted.
//
// The menu's texts are v2's Thai, verbatim (W6) — asserted as the literals of the served page,
// since the keys of its `i18n.ts` are the implementer's. Figures are the server's strings (W7):
// the rows below carry figures no browser arithmetic would produce.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import { toast, Toast } from '../parts/toast';

const loaded = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });

function theMenu(): AdminMenu {
  const m = loaded['./menu.ts'];
  expect(m, 'web/src/screens/admin/transactions/menu.ts exists').toBeDefined();
  return m!.menu;
}

const VIEW = 'admin_transactions';
const THAI = /[฀-๿]/;
const HEADERS = ['รายการ / เวลา', 'ลูกค้า', 'ประเภท', 'จำนวนเงิน', 'ยอดชำระ THB', 'สถานะ'];
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

function tx(n: number, over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id: `XC-20261008-${String(n).padStart(6, '0')}`,
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    unitSize: '1',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3209.99',
    method: 'Cash',
    customer: `Customer ${n}`,
    customerId: `CUS-${n}`,
    staff: 'front',
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

/** Rows numbered `from` down to `to`, newest first. */
function txs(from: number, to: number): TransactionRow[] {
  const out: TransactionRow[] = [];
  for (let n = from; n >= to; n--) out.push(tx(n));
  return out;
}

// ── the fake server ──────────────────────────────────────────────────────────

interface Seen {
  path: string;
  view: string;
  params: URLSearchParams;
  method: string;
}

class FakeServer {
  requests: Seen[] = [];
  version = 41;
  /** The first page per branch. */
  first: Record<string, TransactionRow[]> = { B1: [], B2: [] };
  /** The rows after `before=<id>`. */
  older: Record<string, TransactionRow[]> = {};
  /** When set, every read of the view is answered with it. */
  refusal: { status: number; body: unknown } | undefined;
  branches = [
    { id: 'B1', name: 'Silom #2' },
    { id: 'B2', name: 'Ari' },
  ];
  /** A branch whose first page is answered only once `release()` is called. */
  held: { branch: string; release: () => void; gate: Promise<void> } | undefined;

  hold(branch: string): void {
    let release = () => {};
    const gate = new Promise<void>((r) => (release = r));
    this.held = { branch, release, gate };
  }

  /** The menu's reads of `admin_transactions`, oldest first. */
  reads(): URLSearchParams[] {
    return this.requests.filter((r) => r.view === VIEW).map((r) => r.params);
  }

  answer(url: string, init?: RequestInit): Promise<Response> {
    const u = new URL(url, 'http://desk.test');
    const view = u.pathname.startsWith('/api/v3/views/') ? u.pathname.slice('/api/v3/views/'.length) : '';
    this.requests.push({ path: u.pathname, view, params: u.searchParams, method: (init?.method ?? 'GET').toUpperCase() });
    const ok = (body: unknown) => reply(200, body);
    switch (view) {
      case VIEW: {
        if (this.refusal) return reply(this.refusal.status, this.refusal.body);
        const before = u.searchParams.get('before');
        if (before !== null) return ok({ version: this.version, changed: true, data: this.older[before] ?? [] });
        if (u.searchParams.get('since') === String(this.version)) return ok({ version: this.version, changed: false });
        const branch = u.searchParams.get('branch') ?? '';
        const body = { version: this.version, changed: true, data: this.first[branch] ?? [] };
        if (this.held?.branch === branch) return this.held.gate.then(() => reply(200, body));
        return ok(body);
      }
      case 'admin_branches':
        return ok({ version: this.version, changed: true, data: { branches: this.branches } });
      case 'transaction_documents':
        return ok({ version: this.version, changed: true, data: { ref: u.searchParams.get('ref'), files: [] } });
      case 'admin_shift_state':
        return ok({ version: this.version, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B1' } });
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
  location.hash = '#transactions';
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
  const [branch, sb] = createSignal(opts.branch ?? 'B1');
  const [period, sp] = createSignal({ from: opts.from ?? '2026-10-01', to: opts.to ?? '2026-10-08' });
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

function shownIds(): string[] {
  return [...root.querySelectorAll('tbody tr')].map((tr) => tr.querySelector('button')?.textContent ?? '');
}

/** A label's own words: its text without the control's (a select's options). */
function labelText(label: HTMLLabelElement): string {
  const copy = label.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('input, select, textarea').forEach((c) => c.remove());
  return (copy.textContent ?? '').trim();
}

/** The control the `<label>` whose own words are `text` names (nested or by `for`). */
function control<T extends HTMLElement>(text: string): T {
  const labels = [...root.querySelectorAll('label')].filter((l) => labelText(l) === text);
  expect(labels.map((l) => l.textContent), `one label "${text}"`).toHaveLength(1);
  const label = labels[0]!;
  const c = (label.control ?? label.querySelector('input, select')) as T | null;
  expect(c, `the control labelled "${text}"`).toBeTruthy();
  return c!;
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function typeInto(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function button(text: string): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === text);
  expect(found, `one button "${text}"`).toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

function hasButton(text: string): boolean {
  return [...root.querySelectorAll('button')].some((b) => (b.textContent ?? '').trim() === text);
}

/** The query of a read as an object of its keys, `limit` included. */
function keysOf(p: URLSearchParams): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of p) out[k] = v;
  return out;
}

async function firstRead(): Promise<URLSearchParams> {
  await vi.waitFor(() => expect(server.reads().length).toBeGreaterThan(0));
  return server.reads()[0]!;
}

// ── reading (design §12.6, §12.1) ────────────────────────────────────────────

describe('V2-transactions: when it reads (#464 §7; design §12.6)', () => {
  it('reads nothing while hidden; when shown, the first page of the branch and period, 100 at most', async () => {
    server.first.B1 = [tx(3), tx(2), tx(1)];
    mount({ shown: false });
    await settle();
    expect(server.reads(), 'no read while hidden').toHaveLength(0);
    setShown(true);
    const p = await firstRead();
    expect(keysOf(p)).toEqual({ branch: 'B1', limit: '100', from: '2026-10-01', to: '2026-10-08' });
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(3).id, tx(2).id, tx(1).id]));
  });

  it('never polls: no interval timer, and no read after the first until something changes', async () => {
    const interval = vi.spyOn(globalThis, 'setInterval');
    server.first.B1 = [tx(1)];
    mount();
    await firstRead();
    await settle();
    await settle();
    expect(server.reads(), 'one read').toHaveLength(1);
    expect(interval, 'no timer of its own').not.toHaveBeenCalled();
  });

  it('a period change reads the first page again with the new dates and without since; an open period sends none', async () => {
    server.first.B1 = [tx(1)];
    mount();
    await firstRead();
    setPeriod({ from: '2026-09-01', to: '2026-09-30' });
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(keysOf(server.reads()[1]!)).toEqual({ branch: 'B1', limit: '100', from: '2026-09-01', to: '2026-09-30' });
    setPeriod({ from: '', to: '' });
    await vi.waitFor(() => expect(server.reads().length).toBe(3));
    expect(keysOf(server.reads()[2]!)).toEqual({ branch: 'B1', limit: '100' });
  });

  it("a branch change empties the list, reads the new branch's first page without since or before, and shows only its rows", async () => {
    server.first.B1 = txs(200, 101);
    server.older[tx(101).id] = [tx(100)];
    server.first.B2 = [tx(900)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));
    button('โหลดเพิ่ม').click();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(101));
    server.hold('B2');
    setBranch('B2');
    await vi.waitFor(() => expect(server.reads().some((p) => p.get('branch') === 'B2')).toBe(true));
    await settle();
    expect(shownIds(), "while B2 is read, none of B1's rows are shown under it (admin-html:163)").toEqual([]);
    server.held!.release();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(900).id]));
    const b2 = server.reads().filter((p) => p.get('branch') === 'B2');
    expect(b2.map(keysOf)).toEqual([{ branch: 'B2', limit: '100', from: '2026-10-01', to: '2026-10-08' }]);
  });

  it('refreshTick re-reads the first page with the since it holds: unchanged keeps the rows, changed replaces them', async () => {
    server.first.B1 = [tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(2).id, tx(1).id]));
    requestRefresh();
    await vi.waitFor(() => expect(server.reads().length).toBe(2));
    expect(keysOf(server.reads()[1]!)).toEqual({
      branch: 'B1',
      limit: '100',
      from: '2026-10-01',
      to: '2026-10-08',
      since: '41',
    });
    await settle();
    expect(shownIds(), 'changed: false keeps what is shown').toEqual([tx(2).id, tx(1).id]);
    server.version = 42;
    server.first.B1 = [tx(3), tx(2), tx(1)];
    requestRefresh();
    await vi.waitFor(() => expect(shownIds()).toEqual([tx(3).id, tx(2).id, tx(1).id]));
    expect(server.reads()[2]!.get('since')).toBe('41');
  });

  it('a refusal shows its sentence through errorText, and no rows', async () => {
    server.refusal = { status: 500, body: { error: { code: 'serverError', message: 'raw server text' } } };
    mount();
    await firstRead();
    await vi.waitFor(() => expect(root.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง'));
    expect(root.textContent).not.toContain('raw server text');
    expect(shownIds()).toEqual([]);
  });
});

// ── the filters (T1; design §12.1, §12.14) ───────────────────────────────────

describe("V2-transactions: v2's filters (#464 T1)", () => {
  it("shows v2's search, type and status controls with v2's labels, placeholder and options (admin-html:18, :100; admin-workspace.js:84)", async () => {
    mount();
    await firstRead();
    const search = control<HTMLInputElement>('ค้นหารายการ');
    expect(search.tagName).toBe('INPUT');
    expect(search.type).toBe('search');
    expect(search.placeholder).toBe('เลขรายการ / ชื่อลูกค้า / สกุลเงิน');
    const kind = control<HTMLSelectElement>('ประเภทรายการ');
    expect([...kind.options].map((o) => [o.value, o.textContent])).toEqual([
      ['', 'ทั้งหมด'],
      ['BUY', 'ร้านรับซื้อ'],
      ['SELL', 'ร้านขาย'],
    ]);
    const status = control<HTMLSelectElement>('สถานะ');
    expect([...status.options].map((o) => [o.value, o.textContent])).toEqual([
      ['', 'ทั้งหมด'],
      ['COMPLETED', 'สำเร็จ'],
      ['CANCELLED', 'ยกเลิก'],
      ['PENDING', 'รอบันทึก'],
    ]);
    expect(kind.value).toBe('');
    expect(status.value).toBe('');
    expect(search.value).toBe('');
  });

  it('each filter change reads the first page again with the filter in the query and without since', async () => {
    server.first.B1 = [tx(1)];
    mount();
    await firstRead();
    choose(control<HTMLSelectElement>('ประเภทรายการ'), 'SELL');
    await vi.waitFor(() => expect(server.reads().some((p) => p.get('type') === 'SELL')).toBe(true));
    const bySell = server.reads().find((p) => p.get('type') === 'SELL')!;
    expect(keysOf(bySell)).toEqual({ branch: 'B1', limit: '100', from: '2026-10-01', to: '2026-10-08', type: 'SELL' });

    choose(control<HTMLSelectElement>('สถานะ'), 'PENDING');
    await vi.waitFor(() => expect(server.reads().some((p) => p.get('status') === 'PENDING')).toBe(true));
    const byPending = server.reads().find((p) => p.get('status') === 'PENDING')!;
    expect(byPending.get('since'), 'a new filter is read without since (§12.1)').toBeNull();
    expect(byPending.get('type')).toBe('SELL');

    typeInto(control<HTMLInputElement>('ค้นหารายการ'), '  Somchai ');
    await vi.waitFor(() => expect(server.reads().some((p) => p.get('q') === 'Somchai')).toBe(true));
    const byQ = server.reads().find((p) => p.get('q') === 'Somchai')!;
    expect(byQ.get('since')).toBeNull();
    expect(byQ.get('before')).toBeNull();
    expect(server.reads().some((p) => (p.get('q') ?? '').includes(' ')), 'the search is trimmed (admin-html:58)').toBe(false);
  });

  it('arriving from the overview with #transactions?status=PENDING&from=&to= reads PENDING with search and type cleared (admin-workspace.js:157)', async () => {
    location.hash = '#transactions?status=PENDING&from=&to=';
    mount({ from: '', to: '' });
    const p = await firstRead();
    expect(keysOf(p)).toEqual({ branch: 'B1', limit: '100', status: 'PENDING' });
    expect(control<HTMLSelectElement>('สถานะ').value).toBe('PENDING');
    expect(control<HTMLSelectElement>('ประเภทรายการ').value).toBe('');
    expect(control<HTMLInputElement>('ค้นหารายการ').value).toBe('');
  });

  it('reads q and type from the hash query too (design §12.14)', async () => {
    location.hash = '#transactions?q=Mary&type=SELL';
    mount();
    const p = await firstRead();
    expect(p.get('q')).toBe('Mary');
    expect(p.get('type')).toBe('SELL');
    expect(p.get('status')).toBeNull();
    expect(control<HTMLInputElement>('ค้นหารายการ').value).toBe('Mary');
    expect(control<HTMLSelectElement>('ประเภทรายการ').value).toBe('SELL');
  });
});

// ── the table (T2) and older pages (§9) ──────────────────────────────────────

describe('V2-transactions: the table and its pages (#464 T2, §9)', () => {
  it("shows the rows through TransactionTable: v2's LIVE headers, the server's figures, the status words", async () => {
    server.first.B1 = [
      tx(3, { status: 'PENDING', settlement: '90071992547409.93' }),
      tx(2, { status: 'CANCELLED', type: 'SELL', method: 'PromptPay' }),
      tx(1),
    ];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(3));
    expect([...root.querySelectorAll('thead th')].map((th) => th.textContent)).toEqual(HEADERS);
    const [pending, cancelled, done] = [...root.querySelectorAll('tbody tr')];
    expect(pending!.textContent).toContain('รอบันทึกเอกสาร');
    expect(pending!.textContent).toContain('90,071,992,547,409.93');
    expect(cancelled!.textContent).toContain('ยกเลิก');
    expect(cancelled!.textContent).toContain('ร้านขาย');
    expect(cancelled!.textContent).toContain('พร้อมเพย์');
    expect(done!.textContent).toContain('สำเร็จ');
    expect(done!.textContent).toContain(formatMoney('3209.99'));
  });

  it("says how many it found, v2's sentence, when the first page holds them all (admin-html:67)", async () => {
    server.first.B1 = [tx(3), tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(3));
    expect(root.textContent).toContain('พบ 3 รายการ · คลิกเลขรายการเพื่อดูรายละเอียด');
    expect(hasButton('โหลดเพิ่ม'), 'nothing older to read').toBe(false);
  });

  it("shows the table's empty state when nothing matches", async () => {
    mount();
    await firstRead();
    await vi.waitFor(() => expect(root.textContent).toContain('ไม่พบรายการในช่วงที่เลือก'));
    expect(shownIds()).toEqual([]);
  });

  it('a full page offers โหลดเพิ่ม: it reads the next 100 before the last id with the same filter, adds them below, and goes when a page is short', async () => {
    location.hash = '#transactions?type=SELL';
    server.first.B1 = txs(250, 151);
    server.older[tx(151).id] = txs(150, 51);
    server.older[tx(51).id] = txs(50, 21);
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));
    button('โหลดเพิ่ม').click();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(200));
    button('โหลดเพิ่ม').click();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(230));
    expect(shownIds()).toEqual(txs(250, 21).map((r) => r.id));
    expect(hasButton('โหลดเพิ่ม'), 'a short page ends it').toBe(false);
    const paged = server.reads().filter((p) => p.get('before') !== null).map(keysOf);
    expect(paged).toEqual([
      { branch: 'B1', limit: '100', from: '2026-10-01', to: '2026-10-08', type: 'SELL', before: tx(151).id },
      { branch: 'B1', limit: '100', from: '2026-10-01', to: '2026-10-08', type: 'SELL', before: tx(51).id },
    ]);
  });
});

// ── detail (T3) and cancel (T4) ──────────────────────────────────────────────

describe('V2-transactions: detail and cancel (#464 T3, T4)', () => {
  it('an id opens TransactionDetail of that row in the page dialog, titled รายละเอียดรายการ, with no new list read', async () => {
    server.first.B1 = [
      tx(2),
      tx(1, { type: 'SELL', referenceRate: '34.4432', referenceValue: '1024.00', referenceSpread: '-54.80', referencePercent: '-5.351563' }),
    ];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(2));
    const reads = server.reads().length;
    const open = [...root.querySelectorAll('tbody button')].find((b) => b.textContent === tx(1).id) as HTMLButtonElement;
    open.click();
    const dialog = root.querySelector('dialog')!;
    await vi.waitFor(() => expect(dialog.open).toBe(true));
    expect(dialog.querySelector('h2')?.textContent).toBe('รายละเอียดรายการ');
    expect(dialog.textContent).toContain('ข้อมูลจาก เซิร์ฟเวอร์ · รายละเอียดนี้ไม่ใช่ใบเสร็จ');
    expect(dialog.textContent).toContain(tx(1).id);
    expect(dialog.textContent).toContain('-5.351563%');
    expect(dialog.textContent).toContain('34.4432');
    await vi.waitFor(() =>
      expect(server.requests.some((r) => r.view === 'transaction_documents' && r.params.get('ref') === tx(1).id)).toBe(true),
    );
    expect(server.reads().length, 'the row is in hand: no list read').toBe(reads);
    button('ปิด').click();
    await vi.waitFor(() => expect(dialog.open).toBe(false));
  });

  it("cancel writes nothing in LIVE: the detail's cancel is disabled and clicking it sends no request (admin-html:132, :133)", async () => {
    server.first.B1 = [tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(1));
    (root.querySelector('tbody button') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(root.querySelector('dialog')!.open).toBe(true));
    const cancel = button('รายการจริง: ตรวจสอบการคืนเงินกับผู้ดูแล');
    expect(cancel.disabled).toBe(true);
    const before = server.requests.length;
    cancel.click();
    cancel.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    expect(server.requests.filter((r) => r.method !== 'GET'), 'no write of any kind').toEqual([]);
    expect(server.requests.slice(before).filter((r) => r.view === VIEW)).toEqual([]);
  });

  it('offers ＋ ทำรายการแลกเงิน, as the served page does on transactions (admin-workspace.js:149)', async () => {
    mount();
    await firstRead();
    expect(hasButton('＋ ทำรายการแลกเงิน')).toBe(true);
  });
});

// ── CSV (T5; design §12.8) ───────────────────────────────────────────────────

describe('V2-transactions: CSV (#464 T5; design §12.8)', () => {
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

  it("ส่งออกรายการ CSV walks admin_transactions with the active filter to the end and writes v2's LIVE file", async () => {
    location.hash = '#transactions?q=Mary&type=SELL&status=COMPLETED';
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
    server.first.B1 = first;
    server.older[tx(201).id] = [tx(200), tx(199)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(100));
    const before = server.reads().length;
    button('ส่งออกรายการ CSV').click();
    const text = await csvText();
    const walk = server.reads().slice(before).map(keysOf);
    const filter = { branch: 'B1', limit: '100', from: '2026-10-01', to: '2026-10-08', q: 'Mary', type: 'SELL', status: 'COMPLETED' };
    expect(walk).toEqual([filter, { ...filter, before: tx(201).id }]);
    const lines = text.split('\r\n');
    expect(lines).toHaveLength(1 + 102);
    expect(lines[0]).toBe(quoted(CSV_HEADERS));
    expect(lines[1]).toBe(
      quoted([
        'B1', 'Silom #2', 'LIVE', tx(300).id, '2026-10-08', 'SELL', 'USD', '1500.50', '32.6000', '48916.30',
        '5.25', '1.50', '48921.55', 'Bank Transfer', 'COMPLETED', '', '32.8731', '49326.09', '409.79', '0.830775',
      ]),
    );
    expect(lines[2]).toBe(
      quoted([
        'B1', 'Silom #2', 'LIVE', tx(299).id, '2026-10-08', 'BUY', 'USD', '100.00', '32.1000', '3210.00',
        '0.00', '0.00', '3209.99', 'Cash', 'COMPLETED', 'SH-1', '', '', '', '',
      ]),
    );
    expect(lines.slice(1).map((l) => l.split(',')[3])).toEqual(
      [...txs(300, 201), tx(200), tx(199)].map((r) => `"${r.id}"`),
    );
    expect(names).toEqual(['XCHANGE-LIVE-Silom__2-2026-10-09.csv']);
  });

  it("guards a cell starting with = + @ - as v2 does (admin-html:84): a negative spread, a branch name", async () => {
    server.branches = [{ id: 'B1', name: '=HYPERLINK("x")' }];
    server.first.B1 = [
      tx(1, { referenceRate: '34.4432', referenceValue: '1024.00', referenceSpread: '-54.80', referencePercent: '-5.351563' }),
    ];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(1));
    button('ส่งออกรายการ CSV').click();
    const [, row] = (await csvText()).split('\r\n');
    const cells = row!.split('","');
    expect(cells[1]).toBe(`'=HYPERLINK(""x"")`);
    expect(cells[18]).toBe(`'-54.80`);
    expect(cells[19]).toBe(`'-5.351563"`);
    expect(names).toEqual(['XCHANGE-LIVE-_HYPERLINK__x__-2026-10-09.csv']);
  });

  it('with nothing to export, toasts ไม่มีรายการให้ส่งออก and hands over no file (admin-html:170)', async () => {
    mount();
    await firstRead();
    await settle();
    expect(root.querySelector('[role="status"]')?.textContent ?? '').not.toContain('ไม่มีรายการให้ส่งออก');
    button('ส่งออกรายการ CSV').click();
    await vi.waitFor(() => expect(root.textContent).toContain('ไม่มีรายการให้ส่งออก'));
    await settle();
    expect(blobs).toHaveLength(0);
    expect(names).toHaveLength(0);
  });
});

// ── language (W10) ───────────────────────────────────────────────────────────

describe('V2-transactions: its texts go through its table (W10)', () => {
  it('in English, the menu shows no Thai of its own', async () => {
    setLang('en');
    server.first.B1 = [tx(2), tx(1)];
    mount();
    await vi.waitFor(() => expect(shownIds()).toHaveLength(2));
    await settle();
    const thai = [...root.querySelectorAll('*')]
      .flatMap((el) => [
        ...[...el.childNodes].filter((n) => n.nodeType === Node.TEXT_NODE).map((n) => n.textContent ?? ''),
        el.getAttribute('placeholder') ?? '',
        el.getAttribute('aria-label') ?? '',
        el.getAttribute('title') ?? '',
      ])
      .filter((t) => THAI.test(t));
    expect(thai).toEqual([]);
  });
});
