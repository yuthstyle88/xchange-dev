// V2-cash-calculators (#511): the mount in #467's shift detail, the folder's text table, English,
// the four shared error keys, and the rules on the folder's source (ticket §4 criteria 2–6, §7,
// §10 rows W1, W7, W8; design §12.20.6, W6, W7, W10). Written before the implementation by the
// lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// The mount: #467's shift detail (`web/src/screens/admin/cash/ShiftDetail.tsx`, `openShiftDetail`)
// renders the valuation panel then the profit panel after its table, for every shift, open or
// closed (`admin-html:175`; `shift-valuation.js:5`, `profit-summary.js:4`). It is driven the way
// a user reaches it: the cash menu `../menu.ts` mounted as the shell mounts it (its `Component`
// with `branch()`, `period()`, `shown()`, beside the page's `Dialog`), a click on a shift id.
// The panels' requests carry the menu's branch. The two page-wide reads AppShell makes
// (`PAGE_WIDE`, `shared/testing/page-wide.ts`) are answered and never counted.
//
// The folder's files are read through eager globs, so on a branch without them each test fails on
// an assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { screenLookup, setLang } from '../../../../shared/i18n';
import { en as sharedEn } from '../../../../shared/i18n/en';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { formatMoney } from '../../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../../menus';
import { Dialog, closeDialog } from '../../parts/Dialog';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../../shared/testing/page-wide';

interface PanelProps {
  api: ApiClient;
  branch: string;
  shiftId: string;
}
interface PanelsModule {
  ShiftValuationPanel?: (props: PanelProps) => JSX.Element;
  ProfitSummaryPanel?: (props: PanelProps) => JSX.Element;
}
interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const valuationFiles = import.meta.glob<PanelsModule>('./ShiftValuationPanel.tsx', { eager: true });
const profitFiles = import.meta.glob<PanelsModule>('./ProfitSummaryPanel.tsx', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const menuFiles = import.meta.glob<MenuModule>('../menu.ts', { eager: true });
const ownMenu = import.meta.glob('./menu.ts', { eager: true });
const sources = import.meta.glob<string>('./**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });
const shiftDetailSource = import.meta.glob<string>('../ShiftDetail.tsx', { query: '?raw', import: 'default', eager: true });

function valuationPanel(): (props: PanelProps) => JSX.Element {
  const m = valuationFiles['./ShiftValuationPanel.tsx'];
  expect(m?.ShiftValuationPanel, 'calculators/ShiftValuationPanel.tsx exports ShiftValuationPanel').toBeTypeOf('function');
  return m!.ShiftValuationPanel!;
}

function profitPanel(): (props: PanelProps) => JSX.Element {
  const m = profitFiles['./ProfitSummaryPanel.tsx'];
  expect(m?.ProfitSummaryPanel, 'calculators/ProfitSummaryPanel.tsx exports ProfitSummaryPanel').toBeTypeOf('function');
  return m!.ProfitSummaryPanel!;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/cash/calculators/i18n.ts exists').toBeDefined();
  return m!;
}

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['../menu.ts'];
  expect(m, 'web/src/screens/admin/cash/menu.ts exists').toBeDefined();
  return m!.menu;
}

const THAI = /[฀-๿]/;
const NS = 'admin.cash.calculators';
const VALUATION = '/api/v3/compute/shift_valuation';
const PROFIT = '/api/v3/compute/profit_summary';
const CASH = '/api/v3/views/admin_cash';
const MOVES = '/api/v3/views/admin_cash_movements';
const SHIFTS = '/api/v3/views/admin_shifts';
const sharedThR = sharedTh as Record<string, string>;
const sharedEnR = sharedEn as Record<string, string>;

const VALUATION_HEADING = 'กำไรส่วนต่างจากการตีราคา (ประมาณการ)';
const PROFIT_HEADING = 'กำไรและยอดถอนได้';

// ── the answers ──────────────────────────────────────────────────────────────

const LISTED = [
  { currency: 'THB', opening: '10000.00', current: '6600.00', needsRate: false },
  { currency: 'USD', opening: '100.00', current: '200.00', needsRate: true },
];
const COMPUTED = {
  currencies: LISTED,
  result: {
    rows: [
      { currency: 'THB', rate: '1.000000', value: '6600.00' },
      { currency: 'USD', rate: '34.500000', value: '6900.00' },
    ],
    opening: '13450.00',
    current: '13500.00',
    movements: '500.00',
    difference: '77.77',
    adjusted: '-450.00',
  },
};
const PROFIT_ANSWER = { realized: '1234567.89', unrealized: '-200.01', withdrawable: '42.00' };

function cashData(): unknown {
  return {
    openShifts: [{ shiftId: 'S-0001', tellerId: 'u-1', tellerName: 'EMP-007', openedAt: '2026-10-06T02:30:00.000Z' }],
    stock: [{ code: 'THB', opening: '100000.00', net: '-2230.00', expected: '97770.00' }],
    stats: { opening: '100000.00', incoming: '920.00', outgoing: '3150.00', remaining: '12345.67' },
    reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
    currencies: ['THB', 'USD'],
    lastCounted: null,
  };
}

function shiftRows(): unknown[] {
  return [
    {
      id: 'S-0001', status: 'OPEN', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-007', note: '', difference: null,
      balances: [{ code: 'THB', expected: '97770.00', counted: null, difference: null }],
    },
    {
      id: 'S-0000', status: 'CLOSED', openedAt: '2026-10-05T02:00:00.000Z', staff: 'EMP-002', note: 'USD ขาด 5',
      difference: [{ code: 'USD', amount: '-5.00' }],
      balances: [
        { code: 'THB', expected: '1000.00', counted: '1000.00', difference: '0.00' },
        { code: 'USD', expected: '100.00', counted: '95.00', difference: '-5.00' },
      ],
    },
  ];
}

// ── the fake server: the global fetch ────────────────────────────────────────

interface Call {
  path: string;
  method: string;
  body: Record<string, unknown>;
}
type Reply = { status: number; body: unknown };

class Server {
  calls: Call[] = [];
  valuation: (body: Record<string, unknown>) => Reply = (body) =>
    body.rates == null ? { status: 200, body: { currencies: LISTED } } : { status: 200, body: COMPUTED };
  profit: (body: Record<string, unknown>) => Reply = () => ({ status: 200, body: PROFIT_ANSWER });

  own(): Call[] {
    return this.calls.filter((c) => !PAGE_WIDE.includes(c.path));
  }

  compute(): Call[] {
    return this.calls.filter((c) => c.path.startsWith('/api/v3/compute/'));
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const body =
      typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : {};
    this.calls.push({ path: url.pathname, method, body });
    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'content-type': 'application/json' },
      });
    if (url.pathname === '/api/v3/views/me') {
      return json({ status: 200, body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } } });
    }
    if (url.pathname === QUEUE_CARD_PATH) {
      return json({ status: 200, body: queueCardUnchanged() });
    }
    if (method === 'GET' && url.pathname === CASH) {
      return json({ status: 200, body: { version: 3, changed: true, data: cashData() } });
    }
    if (method === 'GET' && url.pathname === MOVES) {
      return json({ status: 200, body: { version: 3, changed: true, data: [] } });
    }
    if (method === 'GET' && url.pathname === SHIFTS) {
      return json({ status: 200, body: { version: 3, changed: true, data: shiftRows() } });
    }
    if (method === 'POST' && url.pathname === VALUATION) return json(this.valuation(body));
    if (method === 'POST' && url.pathname === PROFIT) return json(this.profit(body));
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

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  location.hash = '#cash';
  setLang('th');
});

afterEach(async () => {
  await settle();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

// ── helpers ──────────────────────────────────────────────────────────────────

function norm(s: string): string {
  return s.replace(/\s+/g, ' ').trim();
}

function spaced(el: Node | null | undefined): string {
  if (!el) return '';
  const parts: string[] = [];
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) parts.push(n.textContent ?? '');
  return norm(parts.join(' '));
}

function text(el: Element | null | undefined): string {
  return norm(el?.textContent ?? '');
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

function dialog(): HTMLDialogElement {
  const d = document.querySelector<HTMLDialogElement>('dialog');
  expect(d, 'the page dialog').not.toBeNull();
  return d!;
}

/** The one element whose own text is `t` (the smallest holding it). */
function elementWithText(within: ParentNode, t: string): Element {
  const found = [...within.querySelectorAll('*')].filter(
    (e) => text(e) === t && ![...e.children].some((c) => text(c) === t),
  );
  expect(found.length, `one element reading ${JSON.stringify(t)}`).toBe(1);
  return found[0]!;
}

function before(a: Node, b: Node): boolean {
  return (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
}

/** Every Thai character in the subtree's text and attribute values, with where it is. */
function thaiIn(el: Element): string[] {
  const found: string[] = [];
  const t = spaced(el);
  if (THAI.test(t)) found.push(`text: ${t.slice(0, 300)}`);
  for (const e of [el, ...el.querySelectorAll('*')]) {
    for (const a of [...e.attributes]) {
      if (THAI.test(a.value)) found.push(`<${e.tagName.toLowerCase()} ${a.name}="${a.value}">`);
    }
  }
  return found;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

// ── the mount in #467's shift detail ─────────────────────────────────────────

function mountMenu(branch: string): HTMLElement {
  const menu = theMenu();
  const [b] = createSignal(branch);
  const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={b} period={period} shown={shown} />
          <Dialog />
        </>
      ) as JSX.Element,
    root,
  );
  return root;
}

describe("V2-cash-calculators: both panels in #467's shift detail (ticket §7 Mount; admin-html:175)", () => {
  for (const [id, status] of [
    ['S-0000', 'closed'],
    ['S-0001', 'open'],
  ] as const) {
    it(`the ${status} shift: its table, then the valuation panel, then the profit panel; one valuation read with the menu's branch`, async () => {
      const el = mountMenu('B2');
      await settle();
      const reads = server.own().length;
      button(el, id).click();
      await settle();
      const d = dialog();
      const table = [...d.querySelectorAll('table')].find((t) => [...t.querySelectorAll('th')].some((th) => text(th) === 'นับจริง'));
      expect(table, "#467's shift table is still there").toBeDefined();
      const valuation = elementWithText(d, VALUATION_HEADING);
      const profit = elementWithText(d, PROFIT_HEADING);
      expect(before(table!, valuation), 'the valuation panel comes after the shift table').toBe(true);
      expect(before(valuation, profit), 'the profit panel comes after the valuation panel').toBe(true);

      const asked = server.own().slice(reads);
      expect(asked.map((c) => c.path), 'opening the detail asks the valuation once, the profit nothing').toEqual([VALUATION]);
      expect(asked[0]!.body.branch, "the menu's branch").toBe('B2');
      expect(asked[0]!.body.shiftId, 'the clicked shift').toBe(id);
      expect(asked[0]!.body.rates == null).toBe(true);
      // the rate input the answer asks for is rendered in the dialog
      expect(d.querySelectorAll('input[type="number"]').length, 'USD rate + eight profit figures').toBe(9);
      expect(/\bTODO\b|coming soon|เร็ว ๆ นี้|เร็วๆ นี้|ยังไม่รองรับ/i.test(text(d)), 'no placeholder text').toBe(false);
    });
  }

  it('a later branch is what a later detail sends', async () => {
    const menu = theMenu();
    const [b, setB] = createSignal('B2');
    const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
    const [shown] = createSignal(true);
    const Component = menu.Component;
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () =>
        (
          <>
            <Component branch={b} period={period} shown={shown} />
            <Dialog />
          </>
        ) as JSX.Element,
      root,
    );
    await settle();
    setB('B3');
    await settle();
    button(root, 'S-0000').click();
    await settle();
    const v = server.compute().filter((c) => c.path === VALUATION);
    expect(v.length).toBe(1);
    expect(v[0]!.body.branch).toBe('B3');
  });

  it('ShiftDetail.tsx imports both panels from ./calculators', () => {
    const src = shiftDetailSource['../ShiftDetail.tsx'];
    expect(src, 'web/src/screens/admin/cash/ShiftDetail.tsx exists').toBeDefined();
    expect(src!).toMatch(/import\s*\{[^}]*\bShiftValuationPanel\b[^}]*\}\s*from\s*['"]\.\/calculators\/ShiftValuationPanel['"]/);
    expect(src!).toMatch(/import\s*\{[^}]*\bProfitSummaryPanel\b[^}]*\}\s*from\s*['"]\.\/calculators\/ProfitSummaryPanel['"]/);
    expect(src!.indexOf('<ShiftValuationPanel'), 'renders ShiftValuationPanel').toBeGreaterThan(-1);
    expect(src!.indexOf('<ProfitSummaryPanel'), 'renders ProfitSummaryPanel after it').toBeGreaterThan(src!.indexOf('<ShiftValuationPanel'));
  });
});

// ── English (W10) ────────────────────────────────────────────────────────────

describe('V2-cash-calculators: in English, no Thai anywhere in either panel (W10; §9 row 6)', () => {
  it('both panels, shown, computed and refused, carry no Thai in text or attributes; the time is en-GB', async () => {
    const Valuation = valuationPanel();
    const Profit = profitPanel();
    setLang('en');
    vi.useFakeTimers({ toFake: ['Date'] });
    const now = new Date('2026-10-28T17:06:07.000Z');
    vi.setSystemTime(now);
    root = document.createElement('div');
    document.body.appendChild(root);
    const api = new ApiClient();
    dispose = render(
      () => (
        <>
          <Valuation api={api} branch="B2" shiftId="S-0000" />
          <Profit api={api} branch="B2" shiftId="S-0000" />
        </>
      ),
      root,
    );
    await settle();
    expect(thaiIn(root), 'on show').toEqual([]);
    expect(root.querySelectorAll('input[type="number"]').length, 'USD rate + eight profit figures').toBe(9);

    // the valuation: date, kind (the first option), USD rate, submit
    const date = root.querySelector<HTMLInputElement>('input[type="date"]')!;
    expect(date, 'the date input').not.toBeNull();
    type(date, '2026-10-27');
    const numbers = [...root.querySelectorAll<HTMLInputElement>('input[type="number"]')];
    type(numbers[0]!, '34.5');
    const valuationForm = date.form;
    expect(valuationForm, 'the valuation inputs are in a form').not.toBeNull();
    valuationForm!.querySelector<HTMLButtonElement>('button:not([type="button"])')!.click();
    await settle();
    expect(server.compute().filter((c) => c.path === VALUATION && c.body.rates != null).length).toBe(1);
    const all = spaced(root);
    expect(all).toContain('2026-10-27');
    expect(all).toContain(`${formatMoney('77.77')} THB`);
    expect(all).toContain(norm(now.toLocaleString('en-GB', { timeZone: 'Asia/Bangkok' })));

    // the profit: eight figures, the confirmation, submit
    for (const input of numbers.slice(1)) type(input, '1');
    const box = root.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    box.click();
    box.form!.querySelector<HTMLButtonElement>('button:not([type="button"])')!.click();
    await settle();
    expect(server.compute().filter((c) => c.path === PROFIT).length).toBe(1);
    expect(spaced(root)).toContain(`${formatMoney('1234567.89')} THB`);
    expect(thaiIn(root), 'after both results').toEqual([]);

    // refusals: the shared English
    server.valuation = (body) =>
      body.rates == null ? { status: 200, body: { currencies: LISTED } } : { status: 400, body: { error: { code: 'valuationRate', message: 'กรอกเรทให้ครบทุกสกุลที่มียอด', field: 'rates' } } };
    server.profit = () => ({ status: 400, body: { error: { code: 'profitNegative', message: 'ต้นทุน ทุน และเงินสำรองต้องไม่ติดลบ' } } });
    valuationForm!.querySelector<HTMLButtonElement>('button:not([type="button"])')!.click();
    await settle();
    box.form!.querySelector<HTMLButtonElement>('button:not([type="button"])')!.click();
    await settle();
    const refused = spaced(root);
    expect(refused).toContain('Enter a rate for every currency with a balance.');
    expect(refused).toContain('Cost, capital and reserves must not be negative.');
    expect(thaiIn(root), 'after both refusals').toEqual([]);
  });
});

// ── the folder's text table (W1, W10) ────────────────────────────────────────

/** v2's fixed texts of V1, V3, V6, P1–P4 (no variable part): each is a value of the folder's `th`. */
const V2_TEXTS = [
  'กำไรส่วนต่างจากการตีราคา (ประมาณการ)',
  'รวมเงินคงเหลือทุกสกุลเป็น THB แล้วเทียบเงินตั้งต้นด้วยเรทชุดเดียวกัน ไม่บวกเงินตั้งต้นซ้ำ',
  'กรอกเรทจาก ธปท. ด้วยตนเอง ยังไม่ดึงข้อมูลอัตโนมัติ ตัวเลขนี้ไม่ใช่กำไรสุทธิหรือกำไรที่รับรู้แล้ว และไม่รวมบัญชีธนาคาร/หนี้สิน',
  'เปิดอัตราแลกเปลี่ยน ธปท. ↗',
  'วันที่เรทที่ ธปท. เผยแพร่',
  'ประเภทเรท',
  'ซื้อเงินโอน',
  'ซื้อตั๋วเงิน',
  'ขาย',
  'กรอกเป็นบาทต่อ 1 หน่วยเสมอ หากแหล่งข้อมูลระบุ 100 JPY ให้หาร 100 ก่อน',
  'กรอกเรทอ้างอิง ธปท.',
  'คำนวณส่วนต่าง',
  'สกุล',
  'ตั้งต้น',
  'คงเหลือตามระบบ',
  'เรท THB',
  'มูลค่าคงเหลือ THB',
  'เงินคงเหลือรวมเป็นบาท',
  'เงินตั้งต้นรวมเป็นบาท (เรทเดียวกัน)',
  'ส่วนต่างก่อนปรับรับ–จ่ายอื่น',
  'รับ–จ่ายเงินอื่นสุทธิ (รับเข้า − จ่ายออก)',
  'ส่วนต่างหลังตัดผลรับ–จ่ายอื่น',
  'ค่าบวก = มูลค่าเพิ่ม / ค่าลบ = มูลค่าลด ไม่ใช่กำไรสุทธิ รายการรับ–จ่ายอื่นอาจรวมค่าใช้จ่าย จึงแสดงการปรับแยก ไม่รวมส่วนต่างตรวจนับเงินจริง (Cash Variance) และยังไม่บันทึกผลคำนวณลงบัญชี',
  'กำไรและยอดถอนได้',
  'แบบคำนวณทดลองจากข้อมูลที่ผู้ดูแลตรวจสอบและกรอกเอง ระบบยังไม่มีต้นทุนซื้อรายล็อตเพื่อคำนวณกำไรที่เกิดขึ้นแล้วอัตโนมัติ ไม่สั่งถอนหรือเปลี่ยนยอดบัญชี',
  'กำไรที่เกิดขึ้นแล้ว',
  'สุทธิและยังไม่ถอน · ผู้ดูแลกรอก',
  'ส่วนต่างตีราคา',
  'เงินต่างประเทศที่ยังถืออยู่ · ยังไม่รับรู้',
  'ยอดถอนได้',
  'เพดานประมาณการ · ไม่ใช่คำสั่งถอน',
  'รอข้อมูล',
  'ข้อมูลที่ใช้คำนวณ',
  'กำไรสุทธิที่เกิดขึ้นแล้วและยังไม่ได้ถอน (THB)',
  'รวมกำไรขายจบและค่าบริการ หักค่าใช้จ่าย/ขาดทุน/กำไรที่ถอนแล้ว',
  'มูลค่าอ้างอิงเงินต่างประเทศคงเหลือทั้งหมด (THB)',
  'ผลรวมจำนวนคงเหลือ × เรทอ้างอิง ไม่รวม THB',
  'ต้นทุนบัญชีเงินต่างประเทศที่ยังเหลือ (THB)',
  'ต้นทุนของเงินที่ยังไม่ขาย ไม่ใช่ต้นทุนซื้อสะสมทั้งหมด',
  'มูลค่าทรัพย์สินสุทธิที่ขายได้จริง (THB)',
  'รวม THB และบัญชีที่เกี่ยวข้อง หักค่าขายและหนี้สินแล้ว',
  'ทุนเป้าหมายที่ต้องเหลือ (THB)',
  'ระบุทุนที่เจ้าของกำหนด รวมทุนเติมที่ต้องเก็บ',
  'เงินกันความเสี่ยงเพิ่มเติม (THB)',
  'กันนอกเหนือจากทุนเป้าหมาย',
  'THB พร้อมใช้ที่ตรวจสอบแล้ว',
  'ตรวจเงินจริงและภาระจ่ายก่อนกรอก',
  'THB ขั้นต่ำสำหรับหมุนเวียน',
  'เงินบาทที่ต้องคงไว้ทำรายการต่อ',
  'มูลค่าขายสุทธิให้ใช้เรทคู่ค้ารับซื้อจริง หักค่าธรรมเนียมและหนี้สิน ไม่ใช้ส่วนต่างเรท ธปท. เป็นกำไรที่รับรู้แล้ว ทุนเป้าหมายต้องรวมเงินเติมทุนที่ต้องเก็บไว้',
  'ตรวจข้อมูล ต้นทุน และยอดที่ถอนก่อนหน้านี้แล้ว',
  'คำนวณกำไรและยอดถอนได้',
  'ส่วนต่างตีราคา = มูลค่าอ้างอิงเงินต่างประเทศคงเหลือ − ต้นทุนคงเหลือ ส่วนยอดถอนได้ใช้ค่าต่ำสุดของกำไรสุทธิที่ยังไม่ถอน / มูลค่าขายสุทธิเกินทุนและเงินสำรอง / THB เกินเงินหมุนเวียนขั้นต่ำ โดยไม่ต่ำกว่าศูนย์',
  'ข้อมูลเปลี่ยน กรุณาคำนวณใหม่',
] as const;

describe('V2-cash-calculators: the folder\'s own text table (W1; design W6, W10 point 1)', () => {
  it('exports th and en with the same keys; en holds no Thai', () => {
    const { th, en } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(en)) {
      expect(v.trim(), `en.${k} is filled in`).not.toBe('');
      expect(THAI.test(v), `en.${k} holds no Thai: ${v}`).toBe(false);
    }
  });

  it("holds v2's fixed texts verbatim, each as a whole value of th", () => {
    const values = new Set(Object.values(texts().th));
    for (const t of V2_TEXTS) expect(values.has(t), `th holds ${JSON.stringify(t)}`).toBe(true);
  });

  it('is collected under admin.cash.calculators.* — the folder has no menu.ts, so no menu lists it', () => {
    const { th, en } = texts();
    for (const key of Object.keys(th)) {
      expect(screenLookup.th[`${NS}.${key}`], `${NS}.${key} (th)`).toBe(th[key]);
      expect(screenLookup.en[`${NS}.${key}`], `${NS}.${key} (en)`).toBe(en[key]);
    }
    expect(Object.keys(ownMenu), 'no calculators/menu.ts').toEqual([]);
  });
});

// ── the four shared error keys (criteria 5, 6) ───────────────────────────────

describe('V2-cash-calculators: the four shared error keys (ticket §7; design §12.20.6)', () => {
  it('th: the server sentences byte for byte, and a Thai translation of originRejected', () => {
    expect(sharedThR['error.valuationRate']).toBe('กรอกเรทให้ครบทุกสกุลที่มียอด');
    expect(sharedThR['error.profitIncomplete']).toBe('กรอกข้อมูลกำไรและเงินสำรองให้ครบก่อนคำนวณ');
    expect(sharedThR['error.profitNegative']).toBe('ต้นทุน ทุน และเงินสำรองต้องไม่ติดลบ');
    const origin = sharedThR['error.originRejected'];
    expect(origin, 'error.originRejected (th)').toBeTypeOf('string');
    expect(THAI.test(origin!), 'a Thai sentence (W10 point 4)').toBe(true);
  });

  it("en: the decided English, and the server's ORIGIN_REJECTED kept", () => {
    expect(sharedEnR['error.valuationRate']).toBe('Enter a rate for every currency with a balance.');
    expect(sharedEnR['error.profitIncomplete']).toBe('Fill in the profit and reserve figures before calculating.');
    expect(sharedEnR['error.profitNegative']).toBe('Cost, capital and reserves must not be negative.');
    expect(sharedEnR['error.originRejected']).toBe('ORIGIN_REJECTED');
  });
});

// ── the folder's source (criteria 3, 4; W7, W8, W10) ─────────────────────────

describe("V2-cash-calculators: the folder's source", () => {
  const own = (): [string, string][] => Object.entries(sources).filter(([path]) => !path.includes('.test.'));

  it('holds the two panels and the text table', () => {
    const paths = own().map(([p]) => p);
    expect(paths).toContain('./ShiftValuationPanel.tsx');
    expect(paths).toContain('./ProfitSummaryPanel.tsx');
    expect(paths).toContain('./i18n.ts');
  });

  it('never parses, rounds or scales a figure, and never calls fetch (criteria 3, 4)', () => {
    const forbidden = /Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100|fetch\(/;
    expect(own().length).toBeGreaterThan(0);
    for (const [path, src] of own()) {
      src.split('\n').forEach((line, i) => {
        expect(forbidden.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });

  it("calls the two actions through act('compute', …) and shows refusals through wordedError", () => {
    const all = own().map(([, src]) => src).join('\n');
    // `.act`, its type arguments if any, then ('compute', '<action>'
    expect(all).toMatch(/\.act\b[^(]{0,160}\(\s*['"]compute['"]\s*,\s*['"]shift_valuation['"]/);
    expect(all).toMatch(/\.act\b[^(]{0,160}\(\s*['"]compute['"]\s*,\s*['"]profit_summary['"]/);
    expect(all).toMatch(/\bwordedError\s*\(/);
  });

  it("reads its texts through screenT('admin.cash.calculators', …) only — never the cash menu's or the shell's table", () => {
    const calls: string[] = [];
    for (const [path, src] of own()) {
      for (const m of src.matchAll(/screenT\s*(?:<[^>]*>)?\s*\(\s*(['"`])([^'"`]*)\1/g)) calls.push(`${path}: ${m[2]}`);
      expect(/from\s+'\.\.\/i18n'|from\s+'\.\.\/\.\.\/i18n'/.test(src), `${path} imports no other screen's table`).toBe(false);
    }
    expect(calls.length, 'at least one screenT call').toBeGreaterThan(0);
    for (const c of calls) expect(c.endsWith(`: ${NS}`), c).toBe(true);
  });
});
