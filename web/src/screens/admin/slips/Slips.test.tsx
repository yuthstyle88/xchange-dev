// V2-slips (#465): the admin menu สลิป (`slips`) — ticket §7, §8, §9, §10 (L1, L2); design §12.1,
// §12.3, §12.6, §12.14. Written before the implementation by the lane controller (claude/7ca80c).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`, `src/online/public/slips.js`; the section `admin-html:19`):
// - the section: `ค้นหาสลิป` search (`type=search`, `maxlength="100"`, placeholder
//   `เลขรายการ / ลูกค้า / สกุลเงิน / เลขอ้างอิง`), `สถานะรายการ` select with `ทั้งหมด` / `สำเร็จ`
//   (COMPLETED) / `ค้างบันทึก` (PENDING) / `ยกเลิก` (CANCELLED), the summary (`role="status"`), the
//   table, the pagination;
// - the summary `สาขา <branch> · พบ <n> สลิป ตามช่วงวันที่และตัวกรอง` (`:10`); before the server
//   answered, `รอข้อมูลจากเซิร์ฟเวอร์` and `ยังโหลดข้อมูลสลิปไม่ได้ กรุณากดรีเฟรช` (`:8`);
// - the table (`:11`): `วัน / เวลา`, `เลขรายการ` (the id opens the detail, with `<currency> ·
//   <type>` under it), `ลูกค้า`, `ยอดรับ–จ่าย THB` (the settlement; v2's `~` only marks the column numeric —
//   `table()` at `admin-html:62` strips it and adds `class="num"`), `เลขอ้างอิงชำระเงิน` (`—`
//   when none), `สถานะรายการ`, `สลิป` (`เปิดสลิป ↗` in a new tab with `rel=noopener`, and
//   `ดาวน์โหลด` = `<slipUrl>?download=1`, `:5`); empty: `ยังไม่มีสลิปที่ตรงกับตัวกรอง` with
//   `แนบสลิปในขั้นตอนรับ–จ่ายเงินของหน้าทำรายการ แล้วบันทึกรายการให้สำเร็จ หรือเปลี่ยนช่วงวันที่เพื่อค้นหาย้อนหลัง`;
// - 50 rows a page with `ก่อนหน้า` / `หน้า <n> …` / `ถัดไป` (`:9`, `:12-13`); any change of branch,
//   period, search or status goes back to page 1 (`:7`); the export button is hidden (`:16`, L2);
// - the detail is the shared `TransactionDetail` (V2-parts) in the page's dialog, titled
//   `รายละเอียดรายการ` (`admin-html:77`).
//
// v3: the rows are the paged view `admin_slips` (`?branch=&limit=50&before=` + the filter `from`,
// `to`, `q`, `status`); the server filters, the screen never filters nor sorts what it was given
// (design §12.1). The menu is read when shown, on a branch / period / filter change and on
// `refreshTick`; never on a timer (design §12.6). A branch change goes back to page 1 and keeps the
// search and the status, as v2 does (`slips.js:7`, `admin-html:162-163`; W9 — controller decision on
// #465, replacing ticket §9's "filters cleared").
//
// The menu reads through `ApiClient` (done criterion 4), a `new ApiClient()` built in its own folder:
// the menu record's `Component` takes only `AdminMenuProps` (controller decision on #462), so these
// tests answer at `fetch` — a fake server behind the real `ApiClient`. Branch names come from the
// shell's `admin_branches` view (same decision).
// Any other read (the `me` view, the queue card's `kiosk_notifications`, `admin_branches`,
// `transaction_documents`) is answered and ignored; only `admin_slips` reads are counted.
//
// `menu.ts` and `i18n.ts` are read through eager globs, so on a branch without them each test fails
// on the assertion `… exists` instead of the whole file failing to load.
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
  expect(m, 'web/src/screens/admin/slips/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

function table(): Table {
  const m = tableModule['./i18n.ts'];
  expect(m, 'web/src/screens/admin/slips/i18n.ts exists').toBeDefined();
  return m!;
}

/** Each text the menu draws is a value of its own table (W10) — never another table's. */
function own(text: string): string {
  expect(Object.values(table().th), `"${text}" is a value of slips/i18n.ts th`).toContain(text);
  return text;
}

const THAI = /[฀-๿]/;
const BIG = '90071992547409.93';

const T = {
  search: 'ค้นหาสลิป',
  placeholder: 'เลขรายการ / ลูกค้า / สกุลเงิน / เลขอ้างอิง',
  statusLabel: 'สถานะรายการ',
  all: 'ทั้งหมด',
  completed: 'สำเร็จ',
  pending: 'ค้างบันทึก',
  cancelled: 'ยกเลิก',
  waiting: 'รอข้อมูลจากเซิร์ฟเวอร์',
  notLoaded: 'ยังโหลดข้อมูลสลิปไม่ได้ กรุณากดรีเฟรช',
  headers: ['วัน / เวลา', 'เลขรายการ', 'ลูกค้า', 'ยอดรับ–จ่าย THB', 'เลขอ้างอิงชำระเงิน', 'สถานะรายการ', 'สลิป'],
  openSlip: 'เปิดสลิป ↗',
  download: 'ดาวน์โหลด',
  empty: 'ยังไม่มีสลิปที่ตรงกับตัวกรอง',
  emptyNote:
    'แนบสลิปในขั้นตอนรับ–จ่ายเงินของหน้าทำรายการ แล้วบันทึกรายการให้สำเร็จ หรือเปลี่ยนช่วงวันที่เพื่อค้นหาย้อนหลัง',
  prev: 'ก่อนหน้า',
  next: 'ถัดไป',
  detailTitle: 'รายละเอียดรายการ',
  export: 'ส่งออกรายการ CSV',
};

// ── rows ─────────────────────────────────────────────────────────────────────

function row(n: number, over: Partial<TransactionRow> = {}): TransactionRow {
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
    method: 'Bank Transfer',
    customer: `Customer ${n}`,
    customerId: `CX-${String(n).padStart(8, '0')}`,
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: `TRX-${n}`,
    slipUrl: `/files/slip-key-${n}`,
    ...over,
  };
}

function rows(from: number, count: number): TransactionRow[] {
  return Array.from({ length: count }, (_, i) => row(from + i));
}

// ── a fake server behind the real ApiClient ──────────────────────────────────

interface Read {
  params: URLSearchParams;
  respond: (answer: Answer) => void;
}

type Answer =
  | { rows: TransactionRow[] }
  | { status: number; code: string; message: string; field?: string };

let reads: Read[];
let auto: ((params: URLSearchParams) => Answer) | null;
let fetchSpy: ReturnType<typeof vi.fn>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function toResponse(answer: Answer): Response {
  if ('rows' in answer) return json(200, { version: 3, changed: true, data: answer.rows });
  const error: Record<string, string> = { code: answer.code, message: answer.message };
  if (answer.field) error.field = answer.field;
  return json(answer.status, { error });
}

function serve(input: unknown): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const view = url.pathname.replace(/^\/api\/v3\/views\//, '');
  if (url.pathname.startsWith('/api/v3/views/') && view === 'admin_slips') {
    return new Promise<Response>((resolve) => {
      const read: Read = { params: url.searchParams, respond: (a) => resolve(toResponse(a)) };
      reads.push(read);
      if (auto) read.respond(auto(url.searchParams));
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
  // the page-wide reads AppShell makes (`me`, `kiosk_notifications`): answered, ignored
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

function bodyRows(): HTMLTableRowElement[] {
  return [...root.querySelectorAll<HTMLTableRowElement>('table tbody tr')];
}

function shownIds(): string[] {
  return bodyRows().map((tr) => {
    const id = [...tr.querySelectorAll('button')].find((b) => /^XC-/.test(b.textContent ?? ''));
    return id?.textContent ?? '';
  });
}

function button(label: string): HTMLButtonElement {
  const b = [...root.querySelectorAll('button')].find((x) => (x.textContent ?? '').trim() === label);
  expect(b, `a button "${label}"`).toBeDefined();
  return b!;
}

function searchInput(): HTMLInputElement {
  const input = root.querySelector<HTMLInputElement>('input[type="search"]');
  expect(input, 'the search input').not.toBeNull();
  return input!;
}

function statusSelect(): HTMLSelectElement {
  const select = root.querySelector<HTMLSelectElement>('select');
  expect(select, 'the status select').not.toBeNull();
  return select!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function last(): Read {
  const r = reads[reads.length - 1];
  expect(r, 'an admin_slips read').toBeDefined();
  return r!;
}

/** The filter the read carried, '' for a key not sent. */
function sent(r: Read, key: string): string {
  return r.params.get(key) ?? '';
}

function pageText(): string {
  const m = text().match(/หน้า\s*\d+/);
  return m ? m[0].replace(/\s+/g, ' ') : '';
}

beforeEach(() => {
  reads = [];
  auto = null;
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

describe('V2-slips: the menu record (#465 §7; design §12.3)', () => {
  it("is slips, order 9 in group data, undated, roles 'all', ▧ สลิปและหลักฐาน, with v2's heading through its own table", () => {
    const m = menu();
    expect(m.id).toBe('slips');
    expect(m.order).toBe(9);
    expect(m.group).toBe('data');
    expect(m.dated).toBe(false);
    expect(m.roles).toBe('all');
    expect(m.icon).toBe('▧');
    expect(m.label()).toBe(own('สลิปและหลักฐาน'));
    expect(m.heading.eyebrow()).toBe(own('PAYMENT SLIPS'));
    expect(m.heading.title()).toBe(own('สลิปและหลักฐาน'));
    expect(m.heading.subtitle()).toBe(own('ค้นหาหลักฐานรับ–จ่ายจากรายการแลกเงิน'));
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

// ── reading ──────────────────────────────────────────────────────────────────

describe('V2-slips: what the menu reads (#465 §8; design §12.1, §12.6)', () => {
  it('reads nothing while hidden, then page 1 of admin_slips: the branch, 50 rows, the shell period, no search or status', async () => {
    auto = () => ({ rows: rows(1, 3) });
    const m = mount({ shown: false });
    await settle();
    expect(reads.length, 'no read while the menu is not shown').toBe(0);
    m.setShown(true);
    await until(() => reads.length > 0, 'the first read');
    const r = last();
    expect(sent(r, 'branch')).toBe('MAIN');
    expect(sent(r, 'limit')).toBe('50');
    expect(r.params.has('before'), 'page 1 has no cursor').toBe(false);
    expect(sent(r, 'from')).toBe('2026-10-01');
    expect(sent(r, 'to')).toBe('2026-10-08');
    expect(sent(r, 'q')).toBe('');
    expect(sent(r, 'status')).toBe('');
  });

  it('never calls fetch except through ApiClient for views, and never re-reads on a timer', async () => {
    auto = () => ({ rows: rows(1, 3) });
    // fake timers from before the mount, so a timer the menu starts is a fake one; they move
    // with real time too, so the reads still settle
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mount();
    await until(() => reads.length === 1, 'the first read');
    for (const call of fetchSpy.mock.calls) {
      expect(String(call[0]), 'every request is a v3 view read').toMatch(/^\/api\/v3\/views\//);
    }
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    await settle();
    expect(reads.length, 'no admin menu polls (design §12.6)').toBe(1);
  });

  it('re-reads when refreshTick moves (the shell’s รีเฟรช, a saved write)', async () => {
    auto = () => ({ rows: rows(1, 3) });
    mount();
    await until(() => reads.length === 1, 'the first read');
    requestRefresh();
    await until(() => reads.length === 2, 'a read after requestRefresh');
    expect(sent(last(), 'branch')).toBe('MAIN');
  });

  it('follows the shell period although the menu is undated, back to page 1 (slips.js:4, :7; design §12.14)', async () => {
    auto = (p) => ({ rows: p.has('before') ? rows(51, 3) : rows(1, 50) });
    const m = mount();
    await until(() => bodyRows().length === 50, 'page 1');
    button(T.next).click();
    await until(() => last().params.has('before'), 'page 2 read');
    m.setPeriod({ from: '2026-09-01', to: '2026-09-30' });
    await until(() => sent(last(), 'from') === '2026-09-01', 'a read with the new period');
    const r = last();
    expect(sent(r, 'to')).toBe('2026-09-30');
    expect(r.params.has('before'), 'page 1 again').toBe(false);
    await until(() => shownIds()[0] === row(1).id, 'page 1 drawn');
  });

  it('sends the search trimmed and the status as chosen, each change back to page 1, and draws every row the server answered in its order', async () => {
    auto = (p) => ({ rows: p.has('before') ? rows(51, 3) : rows(1, 50) });
    mount();
    await until(() => bodyRows().length === 50, 'page 1');
    button(T.next).click();
    await until(() => last().params.has('before'), 'page 2 read');

    // the server answers rows that do not contain the search, in an order that is not by time
    const answered = [
      row(7, { created: '2026-10-01T01:00:00.000Z', customer: 'Zed' }),
      row(3, { created: '2026-10-08T09:00:00.000Z', customer: 'Amy' }),
      row(5, { created: '2026-10-05T05:00:00.000Z', customer: 'Kim' }),
    ];
    auto = () => ({ rows: answered });
    type(searchInput(), '  Trx-1  ');
    await until(() => sent(last(), 'q') === 'Trx-1', 'a read with the search');
    expect(last().params.has('before'), 'a new search starts at page 1').toBe(false);
    await until(() => shownIds().length === 3, 'the answer drawn');
    expect(shownIds(), 'the server’s rows, in the server’s order, none dropped').toEqual(
      answered.map((r) => r.id),
    );

    // back on page 2 of a search, then a status: page 1 again
    auto = (p) => ({ rows: p.has('before') ? rows(51, 3) : rows(1, 50) });
    type(searchInput(), 'Trx');
    await until(() => sent(last(), 'q') === 'Trx' && bodyRows().length === 50, 'page 1 of the search');
    button(T.next).click();
    await until(() => last().params.has('before'), 'page 2 of the search');
    choose(statusSelect(), 'PENDING');
    await until(() => sent(last(), 'status') === 'PENDING', 'a read with the status');
    expect(sent(last(), 'q')).toBe('Trx');
    expect(last().params.has('before'), 'a new status starts at page 1').toBe(false);
  });

  it('on a branch change reads the new branch at page 1 keeping the search and the status (slips.js:7, admin-html:162-163)', async () => {
    auto = (p) => ({ rows: p.has('before') ? rows(51, 3) : rows(1, 50) });
    const m = mount();
    await until(() => bodyRows().length === 50, 'page 1');
    type(searchInput(), 'Customer');
    await until(() => sent(last(), 'q') === 'Customer', 'a read with the search');
    choose(statusSelect(), 'COMPLETED');
    await until(() => sent(last(), 'status') === 'COMPLETED', 'a read with the status');
    button(T.next).click();
    await until(() => last().params.has('before'), 'page 2 read');

    m.setBranch('B9');
    await until(() => sent(last(), 'branch') === 'B9', 'a read of B9');
    const r = last();
    expect(r.params.has('before'), 'page 1').toBe(false);
    expect(sent(r, 'q'), 'the search is kept').toBe('Customer');
    expect(sent(r, 'status'), 'the status is kept').toBe('COMPLETED');
    await settle();
    expect(searchInput().value).toBe('Customer');
    expect(statusSelect().value).toBe('COMPLETED');
    expect(pageText()).toBe('หน้า 1');
  });

  it('draws only the answer to the newest read: an older answer arriving late is dropped', async () => {
    const m = mount();
    await until(() => reads.length === 1, 'the MAIN read');
    const mainRead = reads[0]!;
    m.setBranch('B9');
    await until(() => reads.length === 2, 'the B9 read');
    reads[1]!.respond({ rows: [row(900, { customer: 'Nine Customer' })] });
    await until(() => shownIds().length === 1, 'B9 drawn');
    mainRead.respond({ rows: rows(1, 3) });
    await settle();
    expect(shownIds(), 'MAIN’s late answer is not drawn over B9').toEqual([row(900).id]);
  });
});

// ── what the menu draws ──────────────────────────────────────────────────────

describe('V2-slips: the section, the summary and the table (L1; slips.js:2-16, admin-html:19)', () => {
  it('has v2’s search (maxlength 100) and status select', async () => {
    auto = () => ({ rows: [] });
    mount();
    await until(() => reads.length === 1, 'the first read');
    const input = searchInput();
    expect(input.maxLength).toBe(100);
    expect(input.getAttribute('placeholder')).toBe(own(T.placeholder));
    expect(text()).toContain(own(T.search));
    expect(text()).toContain(own(T.statusLabel));
    const options = [...statusSelect().options].map((o) => [o.value, o.textContent?.trim()]);
    expect(options).toEqual([
      ['', own(T.all)],
      ['COMPLETED', own(T.completed)],
      ['PENDING', own(T.pending)],
      ['CANCELLED', own(T.cancelled)],
    ]);
  });

  it('draws the rows: time, id with currency · type, customer, settlement as sent, reference or —, status word, slip links', async () => {
    const answered = [
      row(1, { settlement: BIG, paymentRef: null, status: 'COMPLETED' }),
      row(2, { type: 'SELL', currency: 'EUR', status: 'CANCELLED', paymentRef: 'KB-77' }),
      row(3, { status: 'PENDING' }),
    ];
    auto = () => ({ rows: answered });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');

    const ths = [...root.querySelectorAll('table thead th')];
    const headers = ths.map((h) => h.textContent?.trim());
    expect(headers).toEqual(T.headers.map(own));
    // the settlement column is v2's one numeric column (`~` in `admin-html:62`'s `table()`)
    expect(ths.map((h) => h.classList.contains('num'))).toEqual([false, false, false, true, false, false, false]);

    const [r1, r2, r3] = bodyRows();
    const cells = (tr: HTMLTableRowElement) => [...tr.querySelectorAll('td')].map((td) => td.textContent ?? '');

    const c1 = cells(r1!);
    expect(c1).toHaveLength(7);
    expect(c1[0], 'the time of created in Bangkok (03:04Z = 10:04)').toContain('10:04');
    expect(c1[1]).toContain(answered[0]!.id);
    expect(c1[1]).toContain('USD · ร้านรับซื้อ');
    expect(c1[2]).toContain('Customer 1');
    expect(c1[3]!.trim(), 'the server’s settlement, formatted, never computed').toBe(formatMoney(BIG));
    expect(c1[4]!.trim()).toBe('—');
    expect(c1[5]!.trim()).toBe(own(T.completed));

    const c2 = cells(r2!);
    expect(c2[1]).toContain('EUR · ร้านขาย');
    expect(c2[4]!.trim()).toBe('KB-77');
    expect(c2[5]!.trim()).toBe(own(T.cancelled));
    expect(cells(r3!)[5]!.trim()).toBe(own(T.pending));

    const links = [...r1!.querySelectorAll('a')];
    const open = links.find((a) => a.textContent?.trim() === own(T.openSlip));
    const download = links.find((a) => a.textContent?.trim() === own(T.download));
    expect(open, 'the open link').toBeDefined();
    expect(open!.getAttribute('href')).toBe('/files/slip-key-1');
    expect(open!.getAttribute('target')).toBe('_blank');
    expect(open!.getAttribute('rel') ?? '').toMatch(/\bnoopener\b/);
    expect(download, 'the download link').toBeDefined();
    expect(download!.getAttribute('href')).toBe('/files/slip-key-1?download=1');
  });

  it('says how many slips were found in the branch, in v2’s sentence', async () => {
    auto = () => ({ rows: rows(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    const status = root.querySelector('[role="status"]');
    expect(status, 'the summary is a status line').not.toBeNull();
    // the branch's name, read from the shell's `admin_branches` view (controller, #462)
    const summary = (status!.textContent ?? '').trim();
    expect(summary).toBe('สาขา Main Branch · พบ 3 สลิป ตามช่วงวันที่และตัวกรอง');
  });

  it('shows v2’s empty text when no slip matches (ticket §9)', async () => {
    auto = () => ({ rows: [] });
    mount();
    await until(() => text().includes(T.empty), 'the empty text');
    expect(text()).toContain(own(T.empty));
    expect(text()).toContain(own(T.emptyNote));
    expect(root.querySelector('table tbody tr')).toBeNull();
  });

  it('shows v2’s not-loaded text and the refusal through errorText when the read fails', async () => {
    auto = () => ({ status: 403, code: 'roleNotAllowedHere', message: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้' });
    mount();
    await until(() => text().includes(T.notLoaded), 'the not-loaded text');
    expect(text()).toContain(own(T.notLoaded));
    expect(text()).toContain(own(T.waiting));
    expect(text()).toContain(errorText('roleNotAllowedHere', 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้'));
    expect(root.querySelector('table tbody tr')).toBeNull();
  });

  it('has no export button and reads no CSV walk (L2; slips.js:16)', async () => {
    auto = () => ({ rows: rows(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    expect(text()).not.toContain(T.export);
    expect(reads.some((r) => sent(r, 'limit') === '100'), 'no walkPages read').toBe(false);
  });

  it('draws no Thai of its own in English', async () => {
    auto = () => ({ rows: rows(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    setLang('en');
    await settle();
    expect(text()).not.toMatch(THAI);
  });
});

describe('V2-slips: pages of 50 (L1; slips.js:9, :12-13)', () => {
  it('pages forward by the last id and back, with ก่อนหน้า / ถัดไป enabled only where there is a page', async () => {
    const page1 = rows(1, 50);
    const page2 = rows(51, 3);
    auto = (p) => ({ rows: p.get('before') === page1[49]!.id ? page2 : page1 });
    mount();
    await until(() => bodyRows().length === 50, 'page 1');
    expect(pageText()).toBe('หน้า 1');
    expect(button(own(T.prev)).disabled, 'no page before 1').toBe(true);
    expect(button(own(T.next)).disabled, 'a full page may have a next').toBe(false);

    button(T.next).click();
    await until(() => shownIds()[0] === page2[0]!.id, 'page 2 drawn');
    const r = last();
    expect(sent(r, 'before')).toBe(page1[49]!.id);
    expect(sent(r, 'limit')).toBe('50');
    expect(sent(r, 'from')).toBe('2026-10-01');
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
    auto = () => ({ rows: rows(1, 3) });
    mount();
    await until(() => bodyRows().length === 3, 'three rows');
    expect(button(own(T.next)).disabled).toBe(true);
    expect(button(own(T.prev)).disabled).toBe(true);
  });
});

describe('V2-slips: the detail (slips.js:15, :17-18; design §12.14)', () => {
  it('opens the shared TransactionDetail of the row in the page dialog, titled รายละเอียดรายการ', async () => {
    const answered = [row(1), row(2, { customer: 'Second Person' })];
    auto = () => ({ rows: answered });
    mount();
    await until(() => bodyRows().length === 2, 'two rows');
    const idButton = [...bodyRows()[1]!.querySelectorAll('button')].find(
      (b) => b.textContent === answered[1]!.id,
    );
    expect(idButton, 'the id is a button').toBeDefined();
    idButton!.click();
    await settle();
    const dialog = root.querySelector('dialog');
    expect(dialog?.hasAttribute('open'), 'the dialog is open').toBe(true);
    const dialogText = dialog!.textContent ?? '';
    expect(dialog!.querySelector('h2')?.textContent).toBe(own(T.detailTitle));
    expect(dialogText).toContain('Second Person');
    // the slip section is TransactionDetail's (V2-parts), so its presence proves the part is used
    expect(dialogText).toContain('สลิป / หลักฐานรับ–จ่าย');
  });
});
