// V2-overview (#461): the overview menu's screen (ticket §7–§10 B1–B6; design §12.1, §12.2,
// §12.6, §12.14, W7, W10). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`; `admin-html:N` = line N of the decoded admin page, ticket §2):
// the four stat cards (`:64`), the BUY/SELL activity bars (`:65`), the THB flow (`:66`, the dark
// note `:17`), the latest five (`:67`, a row opens the detail), the LIVE new-transaction button
// (`:131`), and the served status strip (`src/devices-public/admin-workspace.js:152-158`). Every
// figure is the server's (W7): the screen formats the decimal strings of `admin_overview`'s
// answer and never computes one — the bars' widths are the answer's `buyShare` / `sellShare`.
//
// The menu is the record `./menu.ts` exports, mounted as the shell mounts it: its `Component`
// with `branch()`, `period()` and `shown()` (design §12.3), beside the page's one `Dialog` and
// `Toast` (V2-parts). `AdminMenuProps` carries no `ApiClient`: the menu builds its own with
// `new ApiClient()` in its folder, as `admin/main.tsx:11` does (lane controller's decision for every
// V2 menu, #462). The fake API is therefore the global `fetch` that client calls: it answers the
// views by name and records every request. The two
// page-wide reads AppShell makes (`PAGE_WIDE`, `shared/testing/page-wide.ts`) are
// answered and never counted as the menu's own.
//
// The menu is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { th as partsTh } from '../parts/i18n';
import { requestRefresh } from '../parts/refresh';
import { Toast } from '../parts/toast';
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
  expect(m, 'web/src/screens/admin/overview/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/overview/i18n.ts exists').toBeDefined();
  return m!;
}

const VIEW = '/api/v3/views/admin_overview';

// ── the answer ───────────────────────────────────────────────────────────────

interface Totals {
  volume: string;
  buyPrincipal: string;
  sellPrincipal: string;
  buyCount: number;
  sellCount: number;
  buyShare: string;
  sellShare: string;
  received: string;
  paid: string;
  net: string;
  fees: string;
  completedCount: number;
  cancelledCount: number;
  pendingCount: number;
}

interface OverviewData {
  period: { from: string; to: string };
  totals: Totals;
  latest: TransactionRow[];
  status: { shiftOpen: boolean; pendingCount: number };
}

function row(id: string, created: string, over: Partial<TransactionRow> = {}): TransactionRow {
  return {
    id,
    date: created.slice(0, 10),
    created,
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '30.0000',
    unitSize: '1',
    principal: '3000.00',
    fee: '50.00',
    bankFee: '0.00',
    settlement: '2950.00',
    method: 'Cash',
    customer: `Customer ${id}`,
    customerId: 'CX-00000001',
    staff: 'EMP-001',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'S-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
    ...over,
  };
}

/**
 * Figures chosen so that nothing the screen could compute matches what it must show: the shares
 * are not 3300 / 4400 and 1100 / 4400, and the latest rows are not in `created` order.
 */
function answer(over: Partial<OverviewData> = {}, totals: Partial<Totals> = {}): OverviewData {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    totals: {
      volume: '4400.00',
      buyPrincipal: '3300.00',
      sellPrincipal: '1100.00',
      buyCount: 2,
      sellCount: 1,
      buyShare: '12.34',
      sellShare: '87.66',
      received: '1120.00',
      paid: '3250.00',
      net: '-2130.00',
      fees: '70.00',
      completedCount: 3,
      cancelledCount: 4,
      pendingCount: 5,
      ...totals,
    },
    latest: [
      row('XC-2026-0002', '2026-10-06T03:00:00.000Z', { type: 'SELL', principal: '1100.00', settlement: '1120.00' }),
      row('XC-2026-0003', '2026-10-06T04:00:00.000Z', { amount: '10.00', principal: '300.00', settlement: '300.00' }),
      row('XC-2026-0001', '2026-10-05T02:00:00.000Z'),
    ],
    status: { shiftOpen: false, pendingCount: 6 },
    ...over,
  };
}

// ── the fake API: the global fetch ──────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

type Reply = { status: number; body: unknown };
type Handler = (params: URLSearchParams) => Reply;

let sent: Sent[];
let handlers: Record<string, Handler>;

function ok(version: number, data: unknown): Reply {
  return { status: 200, body: { version, changed: true, data } };
}

function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  sent.push({ path: url.pathname, params: url.searchParams, method });
  let reply: Reply;
  if (url.pathname === '/api/v3/views/me') {
    reply = ok(1, { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' });
  } else if (url.pathname === QUEUE_CARD_PATH) {
    reply = { status: 200, body: queueCardUnchanged() };
  } else {
    const handler = handlers[url.pathname];
    reply = handler
      ? handler(url.searchParams)
      : { status: 500, body: { error: { code: 'serverError', message: `unexpected ${url.pathname}` } } };
  }
  return Promise.resolve(
    new Response(JSON.stringify(reply.body), {
      status: reply.status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

/** The requests the menu made itself. */
function own(): Sent[] {
  return sent.filter((s) => !PAGE_WIDE.includes(s.path));
}

function reads(): Sent[] {
  return sent.filter((s) => s.path === VIEW);
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
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

function mount(
  init: { branch?: string; period?: { from: string; to: string }; shown?: boolean } = {},
): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal(init.period ?? { from: '2026-10-05', to: '2026-10-06' });
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
  sent = [];
  handlers = {
    [VIEW]: () => ok(7, answer()),
    '/api/v3/views/admin_shift_state': (p) =>
      ok(7, { shiftOpen: false, deskUrl: `/desk?branch=${encodeURIComponent(p.get('branch') ?? '')}` }),
    '/api/v3/views/transaction_documents': (p) => ok(7, { ref: p.get('ref'), files: [] }),
  };
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  location.hash = '#overview';
  setLang('th');
});

afterEach(async () => {
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

function text(el: Element): string {
  return (el.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function cards(el: HTMLElement): HTMLElement[] {
  return [...el.querySelectorAll<HTMLElement>('.stat-card')];
}

function card(c: HTMLElement): { label: string; value: string; unit: string; note: string } {
  return {
    label: text(c.querySelector('.stat-label')!),
    value: text(c.querySelector('.stat-value .value')!),
    unit: text(c.querySelector('.stat-value .unit')!),
    note: text(c.querySelector('.stat-note')!),
  };
}

function buttonsNamed(el: ParentNode, name: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
}

function linksNamed(el: ParentNode, name: string): HTMLAnchorElement[] {
  return [...el.querySelectorAll<HTMLAnchorElement>('a')].filter((a) => text(a) === name);
}

/** The inline widths in percent, in document order. */
function percentWidths(el: HTMLElement): string[] {
  return [...el.querySelectorAll<HTMLElement>('[style]')]
    .map((e) => e.style.width)
    .filter((w) => w.endsWith('%'));
}

/** The text of the parent of each leaf element whose own text is exactly `label`. */
function besides(el: HTMLElement, label: string): string[] {
  return [...el.querySelectorAll<HTMLElement>('*')]
    .filter((e) => e.children.length === 0 && text(e) === label)
    .map((e) => text(e.parentElement!));
}

function toastText(el: HTMLElement): string | null {
  const status = [...el.querySelectorAll<HTMLElement>('.toast[role="status"]')][0];
  if (!status || status.hidden) return null;
  const t = text(status);
  return t === '' ? null : t;
}

// ── when it reads ────────────────────────────────────────────────────────────

describe('V2-overview: what it reads, and when (#461 §7, §8; design §12.1, §12.6)', () => {
  it('on show: one read of admin_overview for the branch and the period, and no other request of its own', async () => {
    mount({ branch: 'B9', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();
    expect(reads().length).toBe(1);
    const q = reads()[0]!.params;
    expect(q.get('branch')).toBe('B9');
    expect(q.get('from')).toBe('2026-10-05');
    expect(q.get('to')).toBe('2026-10-06');
    expect(q.has('since'), 'the first read carries no since').toBe(false);
    expect(own().map((s) => s.path)).toEqual([VIEW]);
    expect(own().every((s) => s.method === 'GET'), 'the menu writes nothing').toBe(true);
  });

  it('an open side of the period is not sent', async () => {
    mount({ period: { from: '', to: '2026-10-06' } });
    await settle();
    const q = reads()[0]!.params;
    expect(q.get('from') ?? '').toBe('');
    expect(q.get('to')).toBe('2026-10-06');
  });

  it('reads nothing while it is not shown, and reads when it becomes shown', async () => {
    const m = mount({ shown: false });
    await settle();
    expect(reads().length).toBe(0);
    m.setShown(true);
    await settle();
    expect(reads().length).toBe(1);
  });

  it('a branch or period change reads again without since; the refresh tick reads with the since it holds', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(reads().length).toBe(1);

    m.setBranch('B9');
    await settle();
    expect(reads().length).toBe(2);
    expect(reads()[1]!.params.get('branch')).toBe('B9');
    expect(reads()[1]!.params.has('since'), 'another branch: no since').toBe(false);

    m.setPeriod({ from: '2026-10-01', to: '2026-10-03' });
    await settle();
    expect(reads().length).toBe(3);
    expect(reads()[2]!.params.get('from')).toBe('2026-10-01');
    expect(reads()[2]!.params.get('to')).toBe('2026-10-03');
    expect(reads()[2]!.params.has('since'), 'another filter: no since (design §12.1)').toBe(false);

    requestRefresh();
    await settle();
    expect(reads().length).toBe(4);
    const q = reads()[3]!.params;
    expect(q.get('since')).toBe('7');
    expect(q.get('branch')).toBe('B9');
    expect(q.get('from')).toBe('2026-10-01');
    expect(q.get('to')).toBe('2026-10-03');
    expect(own().every((s) => s.path === VIEW)).toBe(true);
  });

  it('changed: false keeps what is shown', async () => {
    const m = mount();
    await settle();
    expect(text(m.el)).toContain(formatMoney('4400.00'));
    handlers[VIEW] = () => ({ status: 200, body: { version: 7, changed: false } });
    requestRefresh();
    await settle();
    expect(reads().length).toBe(2);
    expect(cards(m.el).map((c) => card(c).value)[0]).toBe(formatMoney('4400.00'));
    expect(text(m.el)).toContain('XC-2026-0003');
  });

  it('never sends a period whose from is after its to (v2 admin-html:57; the shell shows date-error)', async () => {
    mount({ period: { from: '2026-10-07', to: '2026-10-06' } });
    await settle();
    expect(reads().length).toBe(0);
  });
});

// ── B1–B3: the figures ───────────────────────────────────────────────────────

describe("V2-overview: the server's figures, formatted, never computed (B1–B3; W7)", () => {
  it('B1: four stat cards — labels, values, units and notes of admin-html:64', async () => {
    const m = mount();
    await settle();
    const shown = cards(m.el).map(card);
    expect(shown.length).toBe(4);
    expect(shown[0]).toEqual({
      label: 'ยอดแลกเงินรวม',
      value: formatMoney('4400.00'),
      unit: 'THB',
      note: 'มูลค่าเงินต้นของรายการสำเร็จ',
    });
    expect(shown[1]).toEqual({
      label: 'ร้านรับซื้อ',
      value: formatMoney('3300.00'),
      unit: 'THB',
      note: '2 รายการ · ร้านจ่ายเงินบาท',
    });
    expect(shown[2]).toEqual({
      label: 'ร้านขาย',
      value: formatMoney('1100.00'),
      unit: 'THB',
      note: '1 รายการ · ร้านรับเงินบาท',
    });
    expect(shown[3]).toEqual({
      label: 'รายการสำเร็จ',
      value: '3',
      unit: 'รายการ',
      note: 'ยกเลิก 4 · ค้าง 5 รายการ',
    });
  });

  it("B2: the activity bars — each side's name, count and principal, the width the server answered", async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    for (const t of ['EXCHANGE ACTIVITY', 'สัดส่วนยอดแลกเงิน', 'มูลค่าเงินต้น / THB', 'รวมเฉพาะรายการสำเร็จ · ไม่รวมค่าบริการ']) {
      expect(all).toContain(t);
    }
    expect(all).toContain(partsTh.buy);
    expect(all).toContain(partsTh.sell);
    expect(all).toContain('/ 2 รายการ');
    expect(all).toContain('/ 1 รายการ');
    expect(percentWidths(m.el), 'BUY then SELL, the answer’s shares verbatim').toEqual(['12.34%', '87.66%']);
  });

  it('B2: a zero volume answers zero shares, shown as given', async () => {
    handlers[VIEW] = () =>
      ok(7, answer({ latest: [] }, {
        volume: '0.00', buyPrincipal: '0.00', sellPrincipal: '0.00', buyCount: 0, sellCount: 0,
        buyShare: '0', sellShare: '0', received: '0.00', paid: '0.00', net: '0.00', fees: '0.00',
        completedCount: 0, cancelledCount: 0, pendingCount: 0,
      }));
    const m = mount();
    await settle();
    expect(percentWidths(m.el)).toEqual(['0%', '0%']);
  });

  it('B3: THB flow — the signed net with + when positive, received and paid, the dark note', async () => {
    const m = mount();
    await settle();
    let all = text(m.el);
    for (const t of [
      'THB FLOW',
      'รับเข้า − จ่ายออก',
      'ยอดชำระรวมค่าบริการ ไม่ใช่กำไร',
      'ไม่ใช่ยอดเงินสดคงเหลือในลิ้นชัก',
    ]) {
      expect(all).toContain(t);
    }
    expect(besides(m.el, 'รับเข้า').some((t) => t.includes(formatMoney('1120.00'))), 'รับเข้า and its figure').toBe(true);
    expect(besides(m.el, 'จ่ายออก').some((t) => t.includes(formatMoney('3250.00'))), 'จ่ายออก and its figure').toBe(true);
    expect(all).toContain(formatMoney('-2130.00'));
    expect(all).not.toContain(`+${formatMoney('-2130.00')}`);

    handlers[VIEW] = () => ok(8, answer({}, { net: '1910.00' }));
    requestRefresh();
    await settle();
    all = text(m.el);
    expect(all).toContain(`+${formatMoney('1910.00')}`);

    handlers[VIEW] = () => ok(9, answer({}, { net: '0.00' }));
    requestRefresh();
    await settle();
    all = text(m.el);
    expect(all).not.toContain('+0.00');
    expect(all).not.toContain('+1,910.00');
  });

  it('no transaction in the period: the totals the server sends and the empty text of admin-html:62', async () => {
    handlers[VIEW] = () =>
      ok(7, answer({ latest: [] }, {
        volume: '0.00', buyPrincipal: '0.00', sellPrincipal: '0.00', buyCount: 0, sellCount: 0,
        buyShare: '0', sellShare: '0', received: '0.00', paid: '0.00', net: '0.00', fees: '0.00',
        completedCount: 0, cancelledCount: 0, pendingCount: 0,
      }));
    const m = mount();
    await settle();
    expect(cards(m.el).map((c) => card(c).value)).toEqual(['0.00', '0.00', '0.00', '0']);
    expect(text(m.el)).toContain('ไม่พบรายการในช่วงที่เลือก');
  });
});

// ── B4: the latest five ──────────────────────────────────────────────────────

describe('V2-overview: the latest rows (B4)', () => {
  it("shows the server's rows in the server's order under v2's heading, with ดูทั้งหมด → to transactions", async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('LATEST TRANSACTIONS');
    expect(all).toContain('รายการล่าสุด');
    const more = linksNamed(m.el, 'ดูทั้งหมด →');
    expect(more.length).toBe(1);
    expect(more[0]!.getAttribute('href')).toBe('#transactions');
    const ids = [...m.el.querySelectorAll('tbody tr')].map((tr) => text(tr.querySelector('button')!));
    expect(ids, 'not re-sorted in the browser').toEqual(['XC-2026-0002', 'XC-2026-0003', 'XC-2026-0001']);
  });

  it('a row opens TransactionDetail for that exchange', async () => {
    const m = mount();
    await settle();
    const before = own().length;
    buttonsNamed(m.el, 'XC-2026-0003')[0]!.click();
    await settle();
    const detail = document.querySelector('.transaction-detail');
    expect(detail, "V2-parts' TransactionDetail is shown").not.toBeNull();
    expect(text(detail!)).toContain('XC-2026-0003');
    expect(text(detail!)).toContain(partsTh.liveNote);
    const docs = own().slice(before).filter((s) => s.path === '/api/v3/views/transaction_documents');
    expect(docs.length).toBe(1);
    expect(docs[0]!.params.get('ref')).toBe('XC-2026-0003');
    expect(docs[0]!.params.get('branch')).toBe('MAIN');
  });
});

// ── B5: the status strip ─────────────────────────────────────────────────────

describe('V2-overview: the status strip (B5; admin-workspace.js:152-158)', () => {
  it('a closed shift: its words, the cash link, the pending count and the check button', async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    for (const t of [
      'สถานะสาขาขณะนี้ · ไม่อิงช่วงวันที่',
      'ยังไม่เปิดกะ',
      'ตรวจนับเงินก่อนเริ่มทำรายการ',
      'รายการรอบันทึกทั้งหมด',
      '6 รายการ',
      'ตรวจสถานะและเอกสารก่อนบันทึกซ้ำ',
    ]) {
      expect(all).toContain(t);
    }
    expect(all).not.toContain('กะเปิดอยู่');
    const cash = linksNamed(m.el, 'ดูเงินสดและกะ →');
    expect(cash.length).toBe(1);
    expect(cash[0]!.getAttribute('href')).toBe('#cash');
    const check = buttonsNamed(m.el, 'ตรวจรายการ →');
    expect(check.length).toBe(1);
    expect(check[0]!.disabled).toBe(false);
  });

  it('an open shift says กะเปิดอยู่', async () => {
    handlers[VIEW] = () => ok(7, answer({ status: { shiftOpen: true, pendingCount: 0 } }));
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('กะเปิดอยู่');
    expect(all).not.toContain('ยังไม่เปิดกะ');
    expect(all).toContain('0 รายการ');
  });

  it("ตรวจรายการ → opens transactions with status PENDING, search and type cleared, all dates (design §12.14)", async () => {
    const m = mount();
    await settle();
    buttonsNamed(m.el, 'ตรวจรายการ →')[0]!.click();
    await settle();
    const hash = location.hash.slice(1);
    const [id, query] = hash.split('?');
    expect(id).toBe('transactions');
    const q = new URLSearchParams(query ?? '');
    expect(q.get('status')).toBe('PENDING');
    expect(q.has('from') && q.get('from') === '').toBe(true);
    expect(q.has('to') && q.get('to') === '').toBe(true);
    expect(q.get('q') ?? '').toBe('');
    expect(q.get('type') ?? '').toBe('');
    expect(own().every((s) => s.method === 'GET')).toBe(true);
  });

  it('a refused read: the refusal through errorText, the strip cannot say, the check is disabled', async () => {
    handlers[VIEW] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } },
    });
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(all).toContain('ยังโหลดสถานะไม่ได้');
    expect(all).toContain('กดรีเฟรชเพื่อตรวจสอบข้อมูลล่าสุด');
    expect(all).toContain('—');
    expect(buttonsNamed(m.el, 'ตรวจรายการ →')[0]!.disabled).toBe(true);
  });
});

// ── B6: the new-transaction button ───────────────────────────────────────────

describe('V2-overview: new transaction, LIVE (B6; admin-html:131)', () => {
  it("V2-parts' button: no open shift → the toast and #cash, read from admin_shift_state, nothing written", async () => {
    const m = mount({ branch: 'B9' });
    await settle();
    const buttons = buttonsNamed(m.el, partsTh.newTransactionBtn);
    expect(buttons.length).toBe(1);
    buttons[0]!.click();
    await settle();
    const shift = own().filter((s) => s.path === '/api/v3/views/admin_shift_state');
    expect(shift.length).toBe(1);
    expect(shift[0]!.params.get('branch')).toBe('B9');
    expect(location.hash).toBe('#cash');
    expect(toastText(m.el)).toBe(partsTh.openShiftFirst);
    expect(own().every((s) => s.method === 'GET'), 'the button navigates, it does not write').toBe(true);
  });
});

// ── languages ────────────────────────────────────────────────────────────────

describe('V2-overview: setLang (W10 point 5)', () => {
  it('labels switch to the en table; the figures keep their digits', async () => {
    const { th, en } = texts();
    const key = (thai: string): string => {
      const found = Object.entries(th).find(([, v]) => v === thai);
      expect(found, `${thai} is in the menu's table`).toBeDefined();
      return found![0];
    };
    const m = mount();
    await settle();
    setLang('en');
    await settle();
    const all = text(m.el);
    for (const thai of ['ยอดแลกเงินรวม', 'รายการล่าสุด', 'สถานะสาขาขณะนี้ · ไม่อิงช่วงวันที่', 'ดูเงินสดและกะ →']) {
      expect(all).toContain(en[key(thai)]!);
      expect(all).not.toContain(thai);
    }
    expect(cards(m.el).map((c) => card(c).value)).toEqual([
      formatMoney('4400.00'),
      formatMoney('3300.00'),
      formatMoney('1100.00'),
      '3',
    ]);
    expect(reads().length, 'a language switch reads nothing').toBe(1);
  });
});
