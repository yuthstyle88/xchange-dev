// V2-customers (#466): the admin menu ลูกค้าและสมาชิก (`customers`) — ticket §7, §8, §9, §10 (C1–C4);
// design §12.1, §12.3, §12.6, §12.10, §12.14. Written before the implementation by the lane
// controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; the served page = the admin page + ADMIN_SCRIPTS, design W9):
// - C1 `src/online/public/customers.js:2-13` and the section `admin-html:19`: `ค้นหาลูกค้า` search
//   (`type=search`, `maxlength="100"`, placeholder `ชื่อ / รหัสลูกค้า / สัญชาติ`), the summary
//   (`role="status"`) `สาขา <branch> · ลูกค้า <all> คน · พบ <found> คน · ประวัติทุกช่วงวันที่`, the table
//   (`:11`: `รหัสลูกค้า` — the id is a button opening the history —, `ชื่อ` with a `Member` pill,
//   `สัญชาติ` or `—`, `~รายการสำเร็จ`, `~ยอดแลกเงินรวม (THB)`, `รายการล่าสุด` or `ยังไม่มีรายการ`, and
//   `ดูประวัติ`; `~` marks a numeric column and is not shown, `admin-html` `table()`), the two empty
//   texts of `:10`, 50 a page with `ก่อนหน้า` / `หน้า <n> …` / `ถัดไป` (`:12-13`); before the server
//   answered, `รอข้อมูลจากเซิร์ฟเวอร์` and `ยังโหลดข้อมูลลูกค้าไม่ได้ กรุณากดรีเฟรช` (`:7`); a branch change
//   goes back to page 1 and clears the search (`:6`, ticket §9); the menu is undated (`:21`,
//   `admin-workspace.js:146`): the customers are every date's.
// - C2 `customers.js:15-17`: the dialog `ประวัติลูกค้า` — the note, the customer's details, the
//   volume note, its own search (`ค้นหาในประวัติ`, placeholder `เลขรายการ / สกุลเงิน / วันที่`), the
//   table (`วัน / เวลา`, `เลขรายการ` — a button opening the exchange's detail, `customers.js:22` —,
//   `ประเภท`, `~จำนวน`, `~มูลค่า THB`, `สถานะ` with customers.js's own words), its empty text, `ปิด`.
//
// v3: the list is the paged view `admin_customers` (`?branch=&limit=50&before=` + `q`), the history
// `admin_customer_history` (`?branch=&customer=` + `q`); the server filters and sorts, the screen
// never filters nor sorts what it was given (design §12.1). The menu reads when shown, on a branch
// change and on `refreshTick`; never on a timer (design §12.6).
//
// The menu reads through `ApiClient` (done criterion 4), a `new ApiClient()` built in its own folder:
// the menu record's `Component` takes only `AdminMenuProps` (controller decision on #462), so these
// tests answer at `fetch` — a fake server behind the real `ApiClient`. Branch names come from the
// shell's `admin_branches` view (same decision). The page-wide reads AppShell makes (the `me` view,
// the queue card's read, `PAGE_WIDE` in `shared/testing/page-wide.ts`) and `transaction_documents` are answered and ignored;
// only `admin_customers` and `admin_customer_history` are counted.
//
// The rows of `admin_customers` are typed here by hand (`CustomerRow`) so this file compiles before
// the view's generated `admin_customers.ts` exists; its fields are the ones
// `api/tests/v3_admin_customers_view_test.rs` pins (`CUSTOMER_FIELDS`). The history rows are the
// shared `TransactionRow`; their mapping is #464's (`TransactionRow::from_record`).
//
// `menu.ts` and `i18n.ts` are read through eager globs, so on a branch without them each test fails
// on the assertion `… exists` instead of the whole file failing to load.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: 'all'` pin → `needs: 'all'` (#1030 §3.10: `MenuDef.roles` becomes `needs`). Nothing else changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { closeDialog, Dialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import type { AdminMenu } from '../menus';

interface Table {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuModule = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const tableModule = import.meta.glob<Table>('./i18n.ts', { eager: true });

function menu(): AdminMenu {
  const m = menuModule['./menu.ts'];
  expect(m, 'web/src/screens/admin/customers/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

function table(): Table {
  const m = tableModule['./i18n.ts'];
  expect(m, 'web/src/screens/admin/customers/i18n.ts exists').toBeDefined();
  return m!;
}

/** Each text the menu draws is a value of its own table (W10) — never another table's. */
function own(text: string): string {
  expect(Object.values(table().th), `"${text}" is a value of customers/i18n.ts th`).toContain(text);
  return text;
}

const THAI = /[฀-๿]/;
const BIG = '90071992547409.93';

const T = {
  search: 'ค้นหาลูกค้า',
  placeholder: 'ชื่อ / รหัสลูกค้า / สัญชาติ',
  waiting: 'รอข้อมูลจากเซิร์ฟเวอร์',
  notLoaded: 'ยังโหลดข้อมูลลูกค้าไม่ได้ กรุณากดรีเฟรช',
  headers: ['รหัสลูกค้า', 'ชื่อ', 'สัญชาติ', 'รายการสำเร็จ', 'ยอดแลกเงินรวม (THB)', 'รายการล่าสุด', ''],
  member: 'Member',
  noActivity: 'ยังไม่มีรายการ',
  showHistory: 'ดูประวัติ',
  emptyBranch: 'ยังไม่มีข้อมูลลูกค้าในสาขานี้',
  emptyBranchNote:
    'ลูกค้าจะแสดงเมื่อยืนยันข้อมูลลูกค้าในหน้าทำรายการ ข้อมูลจาก Google เดิมยังไม่ได้ถูกนำเข้า',
  noMatch: 'ไม่พบลูกค้าที่ตรงกับคำค้น',
  noMatchNote: 'ค้นหาด้วยชื่อ รหัสลูกค้า หรือสัญชาติ',
  prev: 'ก่อนหน้า',
  next: 'ถัดไป',
  // the history dialog (customers.js:16-17)
  historyTitle: 'ประวัติลูกค้า',
  historyDetails: ['รหัสลูกค้า', 'ชื่อ', 'สัญชาติ', 'เพิ่มเมื่อ', 'รายการสำเร็จ', 'ยอดแลกเงินรวม'],
  volumeNote: 'ยอดรวมเฉพาะเงินต้นของรายการสำเร็จ ไม่ใช่กำไร ไม่รวมรายการยกเลิกหรือค้างบันทึก',
  historySearch: 'ค้นหาในประวัติ',
  historyPlaceholder: 'เลขรายการ / สกุลเงิน / วันที่',
  historyHeaders: ['วัน / เวลา', 'เลขรายการ', 'ประเภท', 'จำนวน', 'มูลค่า THB', 'สถานะ'],
  completed: 'สำเร็จ',
  cancelled: 'ยกเลิก',
  pending: 'ค้างบันทึก',
  historyEmpty: 'ไม่มีรายการที่ตรงกับคำค้น หรือยังไม่มีประวัติแลกเงิน',
  close: 'ปิด',
  detailTitle: 'รายละเอียดรายการ',
};

// ── rows ─────────────────────────────────────────────────────────────────────

/** A row of `admin_customers` (the fields `v3_admin_customers_view_test.rs` pins). */
interface CustomerRow {
  id: string;
  name: string;
  nationality: string;
  created: string;
  member: boolean;
  completed: number;
  volume: string;
  lastAt: string | null;
}

function cx(n: number): string {
  return `CX-${String(n).padStart(8, '0')}`;
}

function customer(n: number, over: Partial<CustomerRow> = {}): CustomerRow {
  return {
    id: cx(n),
    name: `Customer ${n}`,
    nationality: 'THA',
    created: '2026-10-01T02:03:04.000Z',
    member: false,
    completed: 2,
    volume: '6420.00',
    lastAt: '2026-10-08T03:04:05.000Z',
    ...over,
  };
}

function customers(from: number, count: number): CustomerRow[] {
  return Array.from({ length: count }, (_, i) => customer(from + i));
}

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
    settlement: '3210.00',
    method: 'Cash',
    customer: 'Customer 1',
    customerId: cx(1),
    staff: 'front1',
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

// ── a fake server behind the real ApiClient ──────────────────────────────────

type Refusal = { status: number; code: string; message: string; field?: string };
type ListAnswer = { rows: CustomerRow[] } | Refusal;
type HistoryAnswer = { rows: TransactionRow[] } | Refusal;

interface Read<A> {
  params: URLSearchParams;
  respond: (answer: A) => void;
}

let lists: Read<ListAnswer>[];
let histories: Read<HistoryAnswer>[];
let autoList: ((params: URLSearchParams) => ListAnswer) | null;
let autoHistory: ((params: URLSearchParams) => HistoryAnswer) | null;
let fetchSpy: ReturnType<typeof vi.fn>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewResponse(answer: ListAnswer | HistoryAnswer): Response {
  if ('rows' in answer) return json(200, { version: 3, changed: true, data: answer.rows });
  const error: Record<string, string> = { code: answer.code, message: answer.message };
  if (answer.field) error.field = answer.field;
  return json(answer.status, { error });
}

function serve(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const view = url.pathname.startsWith('/api/v3/views/')
    ? url.pathname.replace(/^\/api\/v3\/views\//, '')
    : null;
  if (view === 'admin_customers') {
    return new Promise<Response>((resolve) => {
      const read: Read<ListAnswer> = {
        params: url.searchParams,
        respond: (a) => resolve(viewResponse(a)),
      };
      lists.push(read);
      if (autoList) read.respond(autoList(url.searchParams));
    });
  }
  if (view === 'admin_customer_history') {
    return new Promise<Response>((resolve) => {
      const read: Read<HistoryAnswer> = {
        params: url.searchParams,
        respond: (a) => resolve(viewResponse(a)),
      };
      histories.push(read);
      if (autoHistory) read.respond(autoHistory(url.searchParams));
    });
  }
  if (view === 'admin_branches') {
    return Promise.resolve(
      json(200, {
        version: 3,
        changed: true,
        data: { branches: [{ id: 'MAIN', name: 'Main Branch' }, { id: 'B9', name: 'Nine' }] },
      }),
    );
  }
  if (view === 'transaction_documents') {
    // the detail's receipt section: left loading, it is TransactionDetail's (V2-parts)
    return new Promise<Response>(() => {});
  }
  // the page-wide reads AppShell makes (`PAGE_WIDE`, `shared/testing/page-wide.ts`): answered, ignored
  return Promise.resolve(json(200, { version: 3, changed: false }));
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    for (let j = 0; j < 5; j++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

/** Waits (real time, up to 2 s, so a short debounce is allowed) until `ok` holds. */
async function until(ok: () => boolean, what: string): Promise<void> {
  const start = Date.now();
  while (!ok()) {
    if (Date.now() - start > 2000) throw new Error(`timed out waiting for: ${what}`);
    await settle();
  }
  await settle();
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement;

interface Mounted {
  setBranch: (b: string) => void;
  setPeriod: (p: { from: string; to: string }) => void;
  setShown: (s: boolean) => void;
}

function mount(opts: { shown?: boolean; branch?: string } = {}): Mounted {
  const m = menu();
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
  const [shown, setShown] = createSignal(opts.shown ?? true);
  root = document.createElement('div');
  document.body.appendChild(root);
  const C = m.Component;
  dispose = render(
    (): JSX.Element => (
      <>
        <C branch={branch} period={period} shown={shown} />
        <Dialog />
      </>
    ),
    root,
  );
  return { setBranch, setPeriod, setShown };
}

function text(): string {
  return root.textContent ?? '';
}

/** The customers' panel: the root element. */
function listPanel(): HTMLElement {
  return root;
}

function bodyRows(scope: ParentNode = listPanel()): HTMLTableRowElement[] {
  return [...scope.querySelectorAll<HTMLTableRowElement>('table tbody tr')];
}

function cells(tr: HTMLTableRowElement): string[] {
  return [...tr.querySelectorAll('td')].map((td) => td.textContent ?? '');
}

function shownIds(): string[] {
  return bodyRows().map((tr) => {
    const id = [...tr.querySelectorAll('button')].find((b) => /^CX-/.test(b.textContent ?? ''));
    return id?.textContent ?? '';
  });
}

function buttonIn(scope: ParentNode, label: string): HTMLButtonElement {
  const b = [...scope.querySelectorAll('button')].find(
    (x) => (x.textContent ?? '').trim() === label,
  );
  expect(b, `a button "${label}"`).toBeDefined();
  return b!;
}

function button(label: string): HTMLButtonElement {
  return buttonIn(listPanel(), label);
}

function searchInput(scope: ParentNode = listPanel()): HTMLInputElement {
  const input = scope.querySelector<HTMLInputElement>('input[type="search"]');
  expect(input, 'the search input').not.toBeNull();
  return input!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function dialog(): HTMLDialogElement {
  const d = root.querySelector('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

function dialogOpen(): boolean {
  return root.querySelector('dialog')?.hasAttribute('open') ?? false;
}

function dialogTitle(): string {
  return dialog().querySelector('h2')?.textContent ?? '';
}

function lastList(): Read<ListAnswer> {
  const r = lists[lists.length - 1];
  expect(r, 'an admin_customers read').toBeDefined();
  return r!;
}

function lastHistory(): Read<HistoryAnswer> {
  const r = histories[histories.length - 1];
  expect(r, 'an admin_customer_history read').toBeDefined();
  return r!;
}

/** The value the read carried, '' for a key not sent. */
function sent<A>(r: Read<A>, key: string): string {
  return r.params.get(key) ?? '';
}

function pageText(scope: ParentNode = listPanel()): string {
  const m = (scope.textContent ?? '').match(/หน้า\s*\d+/);
  return m ? m[0].replace(/\s+/g, ' ') : '';
}

/** Opens the history of the first row through its `ดูประวัติ` button. */
async function openHistory(rowIndex = 0): Promise<void> {
  const tr = bodyRows()[rowIndex];
  expect(tr, `row ${rowIndex}`).toBeDefined();
  buttonIn(tr!, T.showHistory).click();
  await until(() => dialogOpen() && histories.length > 0, 'the history dialog and its read');
}

beforeEach(() => {
  lists = [];
  histories = [];
  autoList = null;
  autoHistory = null;
  fetchSpy = vi.fn(serve);
  vi.stubGlobal('fetch', fetchSpy);
  setLang('th');
});

afterEach(() => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

// ── the menu record ──────────────────────────────────────────────────────────

describe('V2-customers: the menu record (#466 §7; design §12.3)', () => {
  it("is customers, order 8 in group data, undated, roles 'all', ♙ ลูกค้าและสมาชิก, with v2's heading through its own table", () => {
    const m = menu();
    expect(m.id).toBe('customers');
    expect(m.order).toBe(8);
    expect(m.group).toBe('data');
    expect(m.dated).toBe(false);
    expect(m.needs).toBe('all');
    expect(m.icon).toBe('♙');
    expect(m.label()).toBe(own('ลูกค้าและสมาชิก'));
    expect(m.heading.eyebrow()).toBe(own('CUSTOMERS'));
    expect(m.heading.title()).toBe(own('ลูกค้าและสมาชิก'));
    expect(m.heading.subtitle()).toBe(own('ประวัติลูกค้าและคำขอลงทะเบียน แยกเป็นหมวดเพื่อค้นหาได้เร็ว'));
    expect(typeof m.Component).toBe('function');
  });

  it('has an English table with exactly the Thai keys, and the record follows the language', () => {
    const { th, en } = table();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(en)) {
      expect(typeof v, k).toBe('string');
      expect(v.length, k).toBeGreaterThan(0);
    }
    setLang('en');
    const m = menu();
    expect(m.label()).not.toMatch(THAI);
    expect(m.heading.title()).not.toMatch(THAI);
    expect(m.heading.subtitle()).not.toMatch(THAI);
  });
});

// ── reading the list (C1) ────────────────────────────────────────────────────

describe('V2-customers: what the list reads (#466 §8, C1; design §12.1, §12.6)', () => {
  it('reads nothing while hidden, then page 1 of admin_customers: the branch, 50 rows, no search and no period', async () => {
    autoList = () => ({ rows: customers(1, 3) });
    const m = mount({ shown: false });
    await settle();
    expect(lists.length, 'no read while the menu is not shown').toBe(0);
    m.setShown(true);
    await until(() => lists.length > 0, 'the first read');
    const r = lastList();
    expect(sent(r, 'branch')).toBe('MAIN');
    expect(sent(r, 'limit')).toBe('50');
    expect(r.params.has('before'), 'page 1 has no cursor').toBe(false);
    expect(sent(r, 'q')).toBe('');
    expect(r.params.has('from'), 'the customers are every date’s (customers.js:9)').toBe(false);
    expect(r.params.has('to')).toBe(false);
  });

  it('reads only through ApiClient — v3 views — and never on a timer', async () => {
    autoList = () => ({ rows: customers(1, 3) });
    // fake timers from before the mount, so a timer the menu starts is a fake one; they move
    // with real time too, so the reads still settle
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    await until(() => lists.length === 1, 'the first read');
    for (const call of fetchSpy.mock.calls) {
      const url = String(call[0]);
      expect(url, 'every request is a v3 view read').toMatch(/^\/api\/v3\/views\//);
    }
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    await settle();
    expect(lists.length, 'no admin menu polls (design §12.6)').toBe(1);
  });

  it('re-reads when refreshTick moves (the shell’s รีเฟรช, a saved write)', async () => {
    autoList = () => ({ rows: customers(1, 3) });
    mount();
    await until(() => lists.length === 1, 'the first read');
    requestRefresh();
    await until(() => lists.length === 2, 'a read after requestRefresh');
    expect(sent(lastList(), 'branch')).toBe('MAIN');
  });

  it('sends the search trimmed, back to page 1, and draws every row the server answered in its order', async () => {
    autoList = (p) => ({ rows: p.has('before') ? customers(51, 3) : customers(1, 50) });
    mount();
    await until(() => bodyRows().length === 50, 'page 1');
    button(T.next).click();
    await until(() => lastList().params.has('before'), 'page 2 read');

    // the server answers rows that do not contain the search, in an order that is not by name
    const answered = [
      customer(7, { name: 'Zed', lastAt: '2026-10-01T01:00:00.000Z' }),
      customer(3, { name: 'Amy', lastAt: null }),
      customer(5, { name: 'Kim', lastAt: '2026-10-08T09:00:00.000Z' }),
    ];
    autoList = () => ({ rows: answered });
    type(searchInput(), '  Somchai  ');
    await until(() => sent(lastList(), 'q') === 'Somchai', 'a read with the search');
    expect(lastList().params.has('before'), 'a new search starts at page 1').toBe(false);
    await until(() => shownIds().length === 3, 'the answer drawn');
    expect(shownIds(), 'the server’s rows, in the server’s order, none dropped').toEqual(
      answered.map((r) => r.id),
    );
    expect(pageText()).toBe('หน้า 1');
  });

  it('on a branch change reads the new branch at page 1 with the search cleared (customers.js:6, ticket §9)', async () => {
    autoList = (p) => ({ rows: p.has('before') ? customers(51, 3) : customers(1, 50) });
    const m = mount();
    await until(() => bodyRows().length === 50, 'page 1');
    type(searchInput(), 'Customer');
    await until(() => sent(lastList(), 'q') === 'Customer', 'a read with the search');
    button(T.next).click();
    await until(() => lastList().params.has('before'), 'page 2 read');

    m.setBranch('B9');
    await until(() => sent(lastList(), 'branch') === 'B9', 'a read of B9');
    const r = lastList();
    expect(r.params.has('before'), 'page 1').toBe(false);
    expect(sent(r, 'q'), 'the search is cleared').toBe('');
    await settle();
    expect(searchInput().value, 'the search box is emptied').toBe('');
    expect(pageText()).toBe('หน้า 1');
  });

  it('draws only the answer to the newest read: an older answer arriving late is dropped', async () => {
    const m = mount();
    await until(() => lists.length === 1, 'the MAIN read');
    const mainRead = lists[0]!;
    m.setBranch('B9');
    await until(() => lists.length === 2, 'the B9 read');
    lists[1]!.respond({ rows: [customer(900, { name: 'Nine Customer' })] });
    await until(() => shownIds().length === 1, 'B9 drawn');
    mainRead.respond({ rows: customers(1, 3) });
    await settle();
    expect(shownIds(), 'MAIN’s late answer is not drawn over B9').toEqual([cx(900)]);
  });
});

// ── what the list draws (C1) ─────────────────────────────────────────────────

describe('V2-customers: the search, the summary and the table (C1; customers.js:2-13, admin-html:19)', () => {
  it('has v2’s search: labelled, maxlength 100, v2’s placeholder', async () => {
    autoList = () => ({ rows: [] });
    mount();
    await until(() => lists.length === 1, 'the first read');
    const input = searchInput();
    expect(input.maxLength).toBe(100);
    expect(input.getAttribute('placeholder')).toBe(own(T.placeholder));
    expect(listPanel().textContent).toContain(own(T.search));
  });

  it('draws the rows: id button, name with Member, nationality or —, completed, the server’s volume, last activity, ดูประวัติ', async () => {
    const answered = [
      customer(1, { member: true, volume: BIG, completed: 12 }),
      customer(2, { nationality: '', completed: 0, volume: '0.00', lastAt: null }),
    ];
    autoList = () => ({ rows: answered });
    mount();
    await until(() => bodyRows().length === 2, 'two rows');

    const headers = [...listPanel().querySelectorAll('table thead th')];
    expect(headers.map((h) => h.textContent?.trim())).toEqual(T.headers.map((h) => (h ? own(h) : h)));
    expect(
      headers.map((h) => h.classList.contains('num')),
      'the two figures are numeric columns (v2’s `~`)',
    ).toEqual([false, false, false, true, true, false, false]);

    const [r1, r2] = bodyRows();
    const c1 = cells(r1!);
    expect(c1).toHaveLength(7);
    const idButton = r1!.querySelectorAll('td')[0]!.querySelector('button');
    expect(idButton?.textContent, 'the id is a button').toBe(cx(1));
    expect(c1[1]).toContain('Customer 1');
    expect(c1[1]).toContain(own(T.member));
    expect(c1[2]!.trim()).toBe('THA');
    expect(c1[3]!.trim()).toBe('12');
    expect(c1[4]!.trim(), 'the server’s volume, formatted, never computed').toBe(formatMoney(BIG));
    expect(c1[5], 'the last activity in Bangkok (03:04Z = 10:04)').toContain('10:04');
    expect(c1[6]!.trim()).toBe(own(T.showHistory));

    const c2 = cells(r2!);
    expect(c2[1]).toContain('Customer 2');
    expect(c2[1], 'no Member pill for a customer who is not one').not.toContain(T.member);
    expect(c2[2]!.trim()).toBe('—');
    expect(c2[3]!.trim()).toBe('0');
    expect(c2[4]!.trim()).toBe('0.00');
    expect(c2[5]!.trim()).toBe(own(T.noActivity));
  });

  it('says how many customers the branch has in v2’s sentence (one short page, no search)', async () => {
    autoList = () => ({ rows: customers(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    const status = listPanel().querySelector('[role="status"]');
    expect(status, 'the summary is a status line').not.toBeNull();
    // the branch's name, read from the shell's `admin_branches` view (controller, #462)
    expect((status!.textContent ?? '').trim()).toBe(
      'สาขา Main Branch · ลูกค้า 3 คน · พบ 3 คน · ประวัติทุกช่วงวันที่',
    );
  });

  it('shows the no-customer texts when the branch answers none, and the no-match texts for a search (ticket §9)', async () => {
    autoList = () => ({ rows: [] });
    mount();
    await until(() => text().includes(T.emptyBranch), 'the no-customer text');
    expect(listPanel().textContent).toContain(own(T.emptyBranch));
    expect(listPanel().textContent).toContain(own(T.emptyBranchNote));
    expect(listPanel().textContent).not.toContain(T.noMatch);
    expect(bodyRows()).toHaveLength(0);

    type(searchInput(), 'nobody');
    await until(() => sent(lastList(), 'q') === 'nobody', 'a read with the search');
    await until(() => text().includes(T.noMatch), 'the no-match text');
    expect(listPanel().textContent).toContain(own(T.noMatch));
    expect(listPanel().textContent).toContain(own(T.noMatchNote));
    expect(listPanel().textContent).not.toContain(T.emptyBranch);
  });

  it('shows v2’s not-loaded text and the refusal through errorText when the read fails', async () => {
    autoList = () => ({ status: 403, code: 'roleNotAllowedHere', message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้' });
    mount();
    await until(() => text().includes(T.notLoaded), 'the not-loaded text');
    expect(listPanel().textContent).toContain(own(T.notLoaded));
    expect(listPanel().textContent).toContain(own(T.waiting));
    expect(text()).toContain(errorText('roleNotAllowedHere', 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้'));
    expect(bodyRows()).toHaveLength(0);
  });

  it('draws no Thai of its own in English', async () => {
    autoList = () => ({ rows: customers(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    setLang('en');
    await settle();
    expect(text()).not.toMatch(THAI);
  });
});

describe('V2-customers: pages of 50 (C1; customers.js:8, :12-13)', () => {
  it('pages forward by the last id and back, with ก่อนหน้า / ถัดไป enabled only where there is a page', async () => {
    const page1 = customers(1, 50);
    const page2 = customers(51, 3);
    autoList = (p) => ({ rows: p.get('before') === page1[49]!.id ? page2 : page1 });
    mount();
    await until(() => bodyRows().length === 50, 'page 1');
    expect(pageText()).toBe('หน้า 1');
    expect(button(own(T.prev)).disabled, 'no page before 1').toBe(true);
    expect(button(own(T.next)).disabled, 'a full page may have a next').toBe(false);

    button(T.next).click();
    await until(() => shownIds()[0] === page2[0]!.id, 'page 2 drawn');
    const r = lastList();
    expect(sent(r, 'before')).toBe(page1[49]!.id);
    expect(sent(r, 'limit')).toBe('50');
    expect(shownIds()).toEqual(page2.map((x) => x.id));
    expect(pageText()).toBe('หน้า 2');
    expect(button(T.next).disabled, 'a short page is the last').toBe(true);
    expect(button(T.prev).disabled).toBe(false);

    button(T.prev).click();
    await until(() => shownIds()[0] === page1[0]!.id, 'page 1 drawn again');
    expect(shownIds()).toEqual(page1.map((x) => x.id));
    expect(pageText()).toBe('หน้า 1');
  });

  it('a page of fewer than 50 has no next', async () => {
    autoList = () => ({ rows: customers(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    expect(button(own(T.next)).disabled).toBe(true);
    expect(button(own(T.prev)).disabled).toBe(true);
  });
});

// ── the history dialog (C2) ──────────────────────────────────────────────────

describe('V2-customers: the history dialog (C2; customers.js:15-17, :22; design §12.14)', () => {
  it('opens ประวัติลูกค้า from ดูประวัติ or from the id, with the customer’s details from the list row, and reads that customer’s history', async () => {
    const row = customer(4, {
      name: 'Malee Rakdee',
      nationality: 'JAPANESE',
      created: '2026-10-01T02:03:04.000Z',
      completed: 7,
      volume: BIG,
    });
    autoList = () => ({ rows: [customer(1), row] });
    autoHistory = () => ({ rows: [] });
    mount();
    await until(() => bodyRows().length === 2, 'two rows');

    await openHistory(1);
    expect(dialogTitle()).toBe(own(T.historyTitle));
    const r = lastHistory();
    expect(sent(r, 'branch')).toBe('MAIN');
    expect(sent(r, 'customer')).toBe(row.id);
    expect(r.params.has('before'), 'the first page').toBe(false);
    expect(sent(r, 'q')).toBe('');

    const d = dialog().textContent ?? '';
    expect(d).toContain('สาขา Main Branch · ประวัติทุกช่วงวันที่');
    const terms = [...dialog().querySelectorAll('dt')].map((dt) => dt.textContent?.trim());
    expect(terms).toEqual(T.historyDetails.map(own));
    const values = [...dialog().querySelectorAll('dd')].map((dd) => (dd.textContent ?? '').trim());
    expect(values[0]).toBe(row.id);
    expect(values[1]).toBe('Malee Rakdee');
    expect(values[2]).toBe('JAPANESE');
    expect(values[3], 'when the customer was added, in Bangkok (02:03Z = 09:03)').toContain('09:03');
    expect(values[4]).toBe('7 รายการ');
    expect(values[5], 'the server’s volume, never computed').toBe(`${formatMoney(BIG)} THB`);
    expect(d).toContain(own(T.volumeNote));
    const input = searchInput(dialog());
    expect(input.maxLength).toBe(100);
    expect(input.getAttribute('placeholder')).toBe(own(T.historyPlaceholder));
    expect(d).toContain(own(T.historySearch));

    // the id button opens the same dialog
    closeDialog();
    await settle();
    const idButton = [...bodyRows()[1]!.querySelectorAll('button')].find((b) => b.textContent === row.id);
    expect(idButton, 'the id is a button').toBeDefined();
    idButton!.click();
    await until(() => dialogOpen() && histories.length === 2, 'the dialog again');
    expect(dialogTitle()).toBe(T.historyTitle);
    expect(sent(lastHistory(), 'customer')).toBe(row.id);
  });

  it('draws the history rows in the server’s order: time, id button, type, amount and currency, value, customers.js’s status words', async () => {
    const answered = [
      tx(3, { created: '2026-10-08T09:00:00.000Z', amount: BIG, currency: 'EUR', principal: '1.10' }),
      tx(1, { created: '2026-10-01T01:00:00.000Z', type: 'SELL', status: 'CANCELLED' }),
      tx(2, { status: 'PENDING' }),
    ];
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = () => ({ rows: answered });
    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 3, 'three history rows');

    const headers = [...dialog().querySelectorAll('table thead th')];
    expect(headers.map((h) => h.textContent?.trim())).toEqual(T.historyHeaders.map(own));
    expect(headers.map((h) => h.classList.contains('num'))).toEqual([false, false, false, true, true, false]);

    const rows = bodyRows(dialog());
    const ids = rows.map((tr) => tr.querySelectorAll('td')[1]!.querySelector('button')?.textContent);
    expect(ids, 'the server’s order, ids as buttons').toEqual(answered.map((t) => t.id));
    const c1 = cells(rows[0]!);
    expect(c1[0], 'the time in Bangkok (09:00Z = 16:00)').toContain('16:00');
    expect(c1[2]!.trim()).toBe('ร้านรับซื้อ');
    expect(c1[3]!.trim()).toBe(`${formatMoney(BIG)} EUR`);
    expect(c1[4]!.trim(), 'the server’s principal').toBe('1.10');
    expect(c1[5]!.trim()).toBe(own(T.completed));
    const c2 = cells(rows[1]!);
    expect(c2[2]!.trim()).toBe('ร้านขาย');
    expect(c2[5]!.trim()).toBe(own(T.cancelled));
    expect(cells(rows[2]!)[5]!.trim(), 'customers.js’s word for PENDING').toBe(own(T.pending));
  });

  it('searches the history on the server: the search trimmed, the same customer, page 1, and v2’s empty text', async () => {
    autoList = () => ({ rows: [customer(1), customer(2)] });
    autoHistory = (p) => ({ rows: p.get('q') ? [] : [tx(1), tx(2)] });
    mount();
    await until(() => bodyRows().length === 2, 'two customers');
    await openHistory(1);
    await until(() => bodyRows(dialog()).length === 2, 'the history');

    type(searchInput(dialog()), '  usd  ');
    await until(() => sent(lastHistory(), 'q') === 'usd', 'a history read with the search');
    const r = lastHistory();
    expect(sent(r, 'customer')).toBe(cx(2));
    expect(sent(r, 'branch')).toBe('MAIN');
    expect(r.params.has('before')).toBe(false);
    await until(() => (dialog().textContent ?? '').includes(T.historyEmpty), 'the empty text');
    expect(dialog().textContent).toContain(own(T.historyEmpty));
    expect(bodyRows(dialog())).toHaveLength(0);
    expect(dialog().querySelector('input[type="search"]'), 'the search stays').not.toBeNull();
  });

  it('opens the shared TransactionDetail of a history row, and ปิด closes the dialog', async () => {
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = () => ({ rows: [tx(1, { customer: 'Malee Rakdee' })] });
    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 1, 'the history');
    expect(buttonIn(dialog(), own(T.close))).toBeDefined();

    const idButton = bodyRows(dialog())[0]!.querySelectorAll('td')[1]!.querySelector('button');
    idButton!.click();
    await until(() => dialogTitle() === T.detailTitle, 'the detail');
    expect(dialogTitle()).toBe(own(T.detailTitle));
    // the slip section is TransactionDetail's (V2-parts), so its presence proves the part is used
    expect(dialog().textContent).toContain('สลิป / หลักฐานรับ–จ่าย');
    expect(dialog().textContent).toContain('Malee Rakdee');

    closeDialog();
    await settle();
    await openHistory();
    buttonIn(dialog(), T.close).click();
    await settle();
    expect(dialogOpen(), 'ปิด closes the dialog').toBe(false);
  });
});

// ── load-older control in history dialog (#466 controller decision) ─────────

describe('V2-customers: load-older control in history dialog (#466 controller decision)', () => {
  it('shows load-older button on a full page of history, pages with before, and hides it on a short page', async () => {
    const page1 = Array.from({ length: 50 }, (_, i) => tx(i + 1));
    const page2 = [tx(51), tx(52)];
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = (p) => {
      const before = p.get('before');
      if (before === page1[49]!.id) {
        return { rows: page2 };
      }
      return { rows: page1 };
    };

    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 50, 'page 1 of 50 history rows');

    const loadOlderBtn = buttonIn(dialog(), own('โหลดรายการเก่ากว่า'));
    expect(loadOlderBtn, 'load-older button exists on full page').toBeDefined();

    loadOlderBtn.click();
    await until(() => bodyRows(dialog()).length === 52, 'next page appended: 52 total rows');
    const r = lastHistory();
    expect(sent(r, 'before')).toBe(page1[49]!.id);
    expect(sent(r, 'customer')).toBe(cx(1));

    // On a short page (length 2 < 50), the load-older button is hidden
    const btnAfter = [...dialog().querySelectorAll('button')].find(
      (b) => (b.textContent ?? '').trim() === 'โหลดรายการเก่ากว่า',
    );
    expect(btnAfter, 'load-older button is hidden after short page').toBeUndefined();
  });

  it('shows error through errorText in history dialog on refusal distinguishing from err.message', async () => {
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = () => ({ status: 403, code: 'roleNotAllowedHere', message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้' });
    setLang('en');
    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    const tr = bodyRows()[0]!;
    const showHistoryBtn = [...tr.querySelectorAll('button')].find((b) =>
      (b.textContent ?? '').includes('View History'),
    )!;
    showHistoryBtn.click();
    await until(
      () => (dialog().textContent ?? '').includes('Role not permitted for this action here'),
      'history error banner in English',
    );
    expect(dialog().querySelector('.banner.error')).not.toBeNull();
    expect(dialog().textContent).toContain('Role not permitted for this action here');
    expect(dialog().textContent).not.toContain('บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้');
    expect(dialog().textContent).not.toContain(T.historyEmpty);
  });

  it('load-older keeps the current search query q', async () => {
    const page1 = Array.from({ length: 50 }, (_, i) => tx(i + 1));
    const page2 = [tx(51)];
    autoList = () => ({ rows: [customer(1)] });
    autoHistory = (p) => {
      const before = p.get('before');
      if (before === page1[49]!.id) {
        return { rows: page2 };
      }
      return { rows: page1 };
    };

    mount();
    await until(() => bodyRows().length === 1, 'one customer');
    await openHistory();
    await until(() => bodyRows(dialog()).length === 50, '50 history rows');

    // Type into history search
    const searchInput = dialog().querySelector<HTMLInputElement>('input[type="search"]')!;
    expect(searchInput).not.toBeNull();
    searchInput.value = 'USD';
    searchInput.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    // Now click load older
    const loadOlderBtn = buttonIn(dialog(), own('โหลดรายการเก่ากว่า'));
    expect(loadOlderBtn).toBeDefined();
    loadOlderBtn.click();
    await until(() => bodyRows(dialog()).length === 51, 'page 2 appended');

    const r = lastHistory();
    expect(sent(r, 'before')).toBe(page1[49]!.id);
    expect(sent(r, 'q')).toBe('USD');
  });



  it('hides summary sentence on full page and page 2, and shows total count vs found count during search on short page', async () => {
    const page1 = customers(1, 50);
    const page2 = customers(51, 20);
    autoList = (p) => {
      const q = p.get('q');
      if (q === 'Target') {
        return { rows: [customer(1, { name: 'Target Customer' })] };
      }
      return { rows: p.get('before') === page1[49]!.id ? page2 : page1 };
    };

    mount();
    await until(() => bodyRows().length === 50, 'page 1 of 50 customers');
    // On full page 1 (50 customers), summary sentence must be hidden
    expect(listPanel().querySelector('[role="status"]'), 'hidden on full page 1').toBeNull();

    // Page to page 2 (20 customers)
    button(T.next).click();
    await until(() => shownIds()[0] === page2[0]!.id, 'page 2 drawn');
    // On page 2, summary sentence must be hidden
    expect(listPanel().querySelector('[role="status"]'), 'hidden on page 2').toBeNull();

    // Search after full first page: summary sentence must still be hidden because unfiltered branch has > 1 page
    type(searchInput(), 'Target');
    await until(() => sent(lastList(), 'q') === 'Target', 'search sent');
    await until(() => bodyRows().length === 1, 'one search match');
    expect(listPanel().querySelector('[role="status"]'), 'hidden during search when branch total is unknown').toBeNull();
  });

  it('shows total count and found count during search when branch has one short page', async () => {
    autoList = (p) => {
      const q = p.get('q');
      if (q === 'Customer 1') {
        return { rows: [customer(1)] };
      }
      return { rows: customers(1, 3) };
    };

    mount();
    await until(() => bodyRows().length === 3, 'three customers on page 1');
    const status1 = listPanel().querySelector('[role="status"]');
    expect(status1, 'status shown on short page 1').not.toBeNull();
    expect(status1!.textContent).toContain('ลูกค้า 3 คน · พบ 3 คน');

    // Search on short page: total count (3) is preserved while found count is 1
    type(searchInput(), 'Customer 1');
    await until(() => sent(lastList(), 'q') === 'Customer 1', 'search sent');
    await until(() => bodyRows().length === 1, 'one match');
    const status2 = listPanel().querySelector('[role="status"]');
    expect(status2, 'status shown during search on short branch').not.toBeNull();
    expect(status2!.textContent).toContain('ลูกค้า 3 คน · พบ 1 คน');
  });

  it('shows list refusal through errorText in English distinguishing from err.message', async () => {
    autoList = () => ({ status: 403, code: 'roleNotAllowedHere', message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้' });
    setLang('en');
    mount();
    await until(() => text().includes('Role not permitted for this action here'), 'list refusal in English');
    expect(listPanel().querySelector('.refusal-message')?.textContent).toContain(
      'Role not permitted for this action here',
    );
    expect(listPanel().textContent).not.toContain('บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้');
  });
});



