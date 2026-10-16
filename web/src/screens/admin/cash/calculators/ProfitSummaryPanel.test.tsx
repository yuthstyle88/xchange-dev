// V2-cash-calculators (#511): the profit panel of the shift detail (ticket §8–§10 rows W1, W5–W8,
// §9 edge cases; design §12.20.1 P1–P6, §12.20.5, §12.20.6). Written before the implementation by
// the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your
// own.
//
// v2 (`xchange-v2-source@4d0bdbb`, `src/devices-public/profit-summary.js`): the panel `:7`, the
// eight inputs `:8-10`, the cards `:12`, the reset on input and the submit `:13`. `<money>` is
// v2's `money()` (`admin-html:45`), here `formatMoney` of the server's decimal string.
//
// The panel computes nothing (W7, W8): the answers below do not follow from the typed figures. It
// sends the eight figures as typed, trimmed (controller, #511 issuecomment-6070066755), and reads
// nothing until submitted (P6).
//
// The panel is mounted with the ticket's [proposed] props `{ api, branch, shiftId }` (plain
// values); its requests reach the global `fetch`, which the fake server below replaces.
//
// The panel is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import type { ProfitSummary } from '../../../../shared/api/generated/profit_summary';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { formatMoney } from '../../../../shared/ui/MoneyText';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../../shared/testing/page-wide';

interface PanelProps {
  api: ApiClient;
  branch: string;
  shiftId: string;
}
interface PanelModule {
  ProfitSummaryPanel: (props: PanelProps) => JSX.Element;
}

const panelFiles = import.meta.glob<PanelModule>('./ProfitSummaryPanel.tsx', { eager: true });

function thePanel(): PanelModule['ProfitSummaryPanel'] {
  const m = panelFiles['./ProfitSummaryPanel.tsx'];
  expect(m, 'web/src/screens/admin/cash/calculators/ProfitSummaryPanel.tsx exists').toBeDefined();
  expect(typeof m!.ProfitSummaryPanel, 'it exports ProfitSummaryPanel').toBe('function');
  return m!.ProfitSummaryPanel;
}

const PROFIT = '/api/v3/compute/profit_summary';
const BRANCH = 'B2';
const SHIFT = 'S-0042';
const shared = sharedTh as Record<string, string>;

/** v2's eight inputs, in order: key, label, hint (`profit-summary.js:8`). */
const DEFINITIONS = [
  ['realized', 'กำไรสุทธิที่เกิดขึ้นแล้วและยังไม่ได้ถอน (THB)', 'รวมกำไรขายจบและค่าบริการ หักค่าใช้จ่าย/ขาดทุน/กำไรที่ถอนแล้ว'],
  ['foreignValue', 'มูลค่าอ้างอิงเงินต่างประเทศคงเหลือทั้งหมด (THB)', 'ผลรวมจำนวนคงเหลือ × เรทอ้างอิง ไม่รวม THB'],
  ['foreignCost', 'ต้นทุนบัญชีเงินต่างประเทศที่ยังเหลือ (THB)', 'ต้นทุนของเงินที่ยังไม่ขาย ไม่ใช่ต้นทุนซื้อสะสมทั้งหมด'],
  ['netAssets', 'มูลค่าทรัพย์สินสุทธิที่ขายได้จริง (THB)', 'รวม THB และบัญชีที่เกี่ยวข้อง หักค่าขายและหนี้สินแล้ว'],
  ['capital', 'ทุนเป้าหมายที่ต้องเหลือ (THB)', 'ระบุทุนที่เจ้าของกำหนด รวมทุนเติมที่ต้องเก็บ'],
  ['reserve', 'เงินกันความเสี่ยงเพิ่มเติม (THB)', 'กันนอกเหนือจากทุนเป้าหมาย'],
  ['cash', 'THB พร้อมใช้ที่ตรวจสอบแล้ว', 'ตรวจเงินจริงและภาระจ่ายก่อนกรอก'],
  ['cashMinimum', 'THB ขั้นต่ำสำหรับหมุนเวียน', 'เงินบาทที่ต้องคงไว้ทำรายการต่อ'],
] as const;

/** The three cards: title, answer key, note (`profit-summary.js:12`). */
const CARDS = [
  ['กำไรที่เกิดขึ้นแล้ว', 'realized', 'สุทธิและยังไม่ถอน · ผู้ดูแลกรอก'],
  ['ส่วนต่างตีราคา', 'unrealized', 'เงินต่างประเทศที่ยังถืออยู่ · ยังไม่รับรู้'],
  ['ยอดถอนได้', 'withdrawable', 'เพดานประมาณการ · ไม่ใช่คำสั่งถอน'],
] as const;

const WAITING = 'รอข้อมูล';
const CHANGED = 'ข้อมูลเปลี่ยน กรุณาคำนวณใหม่';
const CONFIRM = 'ตรวจข้อมูล ต้นทุน และยอดที่ถอนก่อนหน้านี้แล้ว';
const SUBMIT = 'คำนวณกำไรและยอดถอนได้';

/** §12.20.5's worked example, as typed. */
const TYPED: Record<(typeof DEFINITIONS)[number][0], string> = {
  realized: '1000',
  foreignValue: '5000',
  foreignCost: '5200',
  netAssets: '20000',
  capital: '15000',
  reserve: '2000',
  cash: '8000',
  cashMinimum: '5000',
};

/** Not what those figures give (1000.00 / -200.00 / 1000.00): the panel shows what is answered. */
function answered(realized = '1234567.89'): ProfitSummary {
  return { realized, unrealized: '-200.01', withdrawable: '42.00' };
}

// ── the fake server: the global fetch ────────────────────────────────────────

interface Call {
  path: string;
  method: string;
  headers: Record<string, string>;
  body: Record<string, unknown>;
}
type Reply = { status: number; body: unknown };

function refusal(status: number, code: string, message: string, field?: string): Reply {
  return { status, body: { error: { code, message, ...(field ? { field } : {}) } } };
}

class Server {
  calls: Call[] = [];
  answer: (body: Record<string, unknown>, n: number) => Reply = () => ({ status: 200, body: answered() });
  hold = false;
  private held: { index: number; go: () => void }[] = [];

  release(index: number): void {
    const found = this.held.find((h) => h.index === index);
    expect(found, `request #${index} is held`).toBeDefined();
    this.held = this.held.filter((h) => h !== found);
    found!.go();
  }

  releaseAll(): void {
    const all = this.held;
    this.held = [];
    for (const h of all) h.go();
  }

  computeCalls(): Call[] {
    return this.calls.filter((c) => c.path.startsWith('/api/v3/compute/'));
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers: Record<string, string> = {};
    new Headers(init?.headers ?? {}).forEach((v, k) => {
      headers[k] = v;
    });
    const body =
      typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : {};
    this.calls.push({ path: url.pathname, method, headers, body });
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
    if (method === 'POST' && url.pathname === PROFIT) {
      const index = this.calls.filter((c) => c.path === PROFIT).length;
      const reply = this.answer(body, index);
      if (this.hold) await new Promise<void>((go) => this.held.push({ index, go }));
      return json(reply);
    }
    return json(refusal(500, 'serverError', `unexpected ${method} ${url.pathname}`));
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
let root: HTMLElement | undefined;

async function mount(): Promise<HTMLElement> {
  const Panel = thePanel();
  root = document.createElement('div');
  document.body.appendChild(root);
  const api = new ApiClient();
  dispose = render(() => <Panel api={api} branch={BRANCH} shiftId={SHIFT} />, root);
  await settle();
  return root;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  setLang('th');
});

afterEach(async () => {
  server.releaseAll();
  await settle();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

// ── helpers on the DOM ───────────────────────────────────────────────────────

function norm(s: string): string {
  return s.replace(/\s+/g, ' ').trim();
}

/** Every text node, in order, joined by one space. */
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

function ownText(l: HTMLLabelElement): string {
  const clone = l.cloneNode(true) as HTMLElement;
  clone.querySelectorAll('input,select,textarea,small').forEach((n) => n.remove());
  return norm(clone.textContent ?? '');
}

function field(within: ParentNode, label: string): HTMLInputElement {
  const labels = [...within.querySelectorAll<HTMLLabelElement>('label')].filter((l) => ownText(l) === label);
  expect(labels.length, `one field labelled ${JSON.stringify(label)}`).toBe(1);
  const l = labels[0]!;
  const nested = l.querySelector<HTMLInputElement>('input,select,textarea');
  const named = l.htmlFor ? (document.getElementById(l.htmlFor) as HTMLInputElement | null) : null;
  const control = nested ?? named;
  expect(control, `the control of ${JSON.stringify(label)}`).not.toBeNull();
  return control!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

/** Types a value a number input's sanitizer would turn into '': `value` reads back what was typed. */
function typeRaw(input: HTMLInputElement, raw: string): void {
  input.value = raw.trim();
  Object.defineProperty(input, 'value', {
    configurable: true,
    get: () => raw,
    set: () => {},
  });
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function alertLine(el: ParentNode): HTMLElement {
  const alerts = el.querySelectorAll<HTMLElement>('[role="alert"]');
  expect(alerts.length, 'one role="alert" line').toBe(1);
  return alerts[0]!;
}

function confirmBox(el: ParentNode): HTMLInputElement {
  return field(el, CONFIRM);
}

/** Types the eight figures (th labels), ticks the confirmation and presses the button. */
async function submit(el: HTMLElement, typed: Record<string, string> = TYPED): Promise<void> {
  for (const [key, label] of DEFINITIONS) type(field(el, label), typed[key]!);
  const box = confirmBox(el);
  if (!box.checked) box.click();
  button(el, SUBMIT).click();
  await settle();
}

/** "title amount note" for each card, in order. */
function cardsShow(el: HTMLElement, amounts: [string, string, string]): void {
  const all = spaced(el);
  CARDS.forEach(([title, , note], i) => {
    expect(all, title).toContain(`${title} ${amounts[i]} ${note}`);
  });
}

// ── W1, W5: P1, P2 ───────────────────────────────────────────────────────────

describe('V2-cash-calculators: the profit panel on show (W1, W5; profit-summary.js:7-12)', () => {
  it('reads nothing on show (P6)', async () => {
    await mount();
    expect(server.computeCalls().length).toBe(0);
    expect(server.calls.length).toBe(0);
  });

  it("v2's heading, note, three cards each รอข้อมูล with their notes, the paragraph and the footnote, verbatim", async () => {
    const el = await mount();
    const all = spaced(el);
    for (const t of [
      'กำไรและยอดถอนได้',
      'แบบคำนวณทดลองจากข้อมูลที่ผู้ดูแลตรวจสอบและกรอกเอง ระบบยังไม่มีต้นทุนซื้อรายล็อตเพื่อคำนวณกำไรที่เกิดขึ้นแล้วอัตโนมัติ ไม่สั่งถอนหรือเปลี่ยนยอดบัญชี',
      'มูลค่าขายสุทธิให้ใช้เรทคู่ค้ารับซื้อจริง หักค่าธรรมเนียมและหนี้สิน ไม่ใช้ส่วนต่างเรท ธปท. เป็นกำไรที่รับรู้แล้ว ทุนเป้าหมายต้องรวมเงินเติมทุนที่ต้องเก็บไว้',
      'ส่วนต่างตีราคา = มูลค่าอ้างอิงเงินต่างประเทศคงเหลือ − ต้นทุนคงเหลือ ส่วนยอดถอนได้ใช้ค่าต่ำสุดของกำไรสุทธิที่ยังไม่ถอน / มูลค่าขายสุทธิเกินทุนและเงินสำรอง / THB เกินเงินหมุนเวียนขั้นต่ำ โดยไม่ต่ำกว่าศูนย์',
    ]) {
      expect(all, t).toContain(t);
    }
    const headings = [...el.querySelectorAll('h1,h2,h3,h4,h5,h6')].map((h) => text(h));
    expect(headings, 'the heading is a heading').toContain('กำไรและยอดถอนได้');
    cardsShow(el, [WAITING, WAITING, WAITING]);
    expect(text(alertLine(el)), 'the alert starts empty').toBe('');
  });

  it('a <details> ข้อมูลที่ใช้คำนวณ holding the eight inputs in v2 order, each labelled with its hint as placeholder and <small>', async () => {
    const el = await mount();
    const details = [...el.querySelectorAll('details')].filter((d) => text(d.querySelector('summary')) === 'ข้อมูลที่ใช้คำนวณ');
    expect(details.length, 'one <details> with the summary ข้อมูลที่ใช้คำนวณ').toBe(1);
    const d = details[0]!;
    const numberInputs = [...d.querySelectorAll<HTMLInputElement>('input[type="number"]')];
    expect(numberInputs.length, 'eight number inputs').toBe(8);
    DEFINITIONS.forEach(([key, label, hint], i) => {
      const input = field(d, label);
      expect(input, `${key} is input number ${i + 1}`).toBe(numberInputs[i]);
      expect(input.getAttribute('type')).toBe('number');
      expect(input.getAttribute('step'), `${key} step`).toBe('0.01');
      expect(input.required, `${key} required`).toBe(true);
      expect(input.getAttribute('placeholder'), `${key} placeholder`).toBe(hint);
      const owner = input.closest('label') ?? [...d.querySelectorAll<HTMLLabelElement>('label')].find((l) => l.htmlFor === input.id)!;
      expect([...owner.querySelectorAll('small')].map((s) => text(s)), `${key} <small>`).toEqual([hint]);
      if (key === 'realized' || key === 'netAssets') {
        expect(input.hasAttribute('min'), `${key} may be negative`).toBe(false);
      } else {
        expect(input.getAttribute('min'), `${key} min`).toBe('0');
      }
    });
    const box = field(d, CONFIRM);
    expect(box.getAttribute('type')).toBe('checkbox');
    expect(box.required).toBe(true);
    expect(button(d, SUBMIT)).toBeDefined();
    expect(d.querySelector('[role="alert"]'), 'the alert line is in the form').not.toBeNull();
  });
});

// ── W5: P3, P4 ───────────────────────────────────────────────────────────────

describe('V2-cash-calculators: the profit submit, its result and the reset (W5; profit-summary.js:13)', () => {
  it('sends the branch and the eight figures as typed strings, nothing else', async () => {
    const el = await mount();
    await submit(el);
    const calls = server.computeCalls();
    expect(calls.length).toBe(1);
    const c = calls[0]!;
    expect(c.path).toBe(PROFIT);
    expect(c.method).toBe('POST');
    expect(c.headers['x-xchange-request'], 'sent by ApiClient.act').toBe('1');
    expect(c.body).toEqual({ branch: BRANCH, ...TYPED });
  });

  it('trims each typed figure before sending: " 1" is sent as "1" (controller, issuecomment-6070066755)', async () => {
    const el = await mount();
    for (const [key, label] of DEFINITIONS) type(field(el, label), TYPED[key]);
    typeRaw(field(el, DEFINITIONS[0][1]), ' 1');
    typeRaw(field(el, DEFINITIONS[3][1]), '-5 ');
    typeRaw(field(el, DEFINITIONS[7][1]), ' 0.5 ');
    confirmBox(el).click();
    button(el, SUBMIT).click();
    await settle();
    const calls = server.computeCalls();
    expect(calls.length).toBe(1);
    expect(calls[0]!.body).toEqual({ branch: BRANCH, ...TYPED, realized: '1', netAssets: '-5', cashMinimum: '0.5' });
  });

  it("shows the three answered figures as `<money> THB` and the line คำนวณเมื่อ <now, th-TH> · ไม่ได้บันทึกหรือถอนเงิน", async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    const now = new Date('2026-10-28T05:06:07.000Z');
    vi.setSystemTime(now);
    const el = await mount();
    await submit(el);
    cardsShow(el, [`${formatMoney('1234567.89')} THB`, `${formatMoney('-200.01')} THB`, `${formatMoney('42.00')} THB`]);
    expect(text(alertLine(el))).toBe(`คำนวณเมื่อ ${norm(now.toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' }))} · ไม่ได้บันทึกหรือถอนเงิน`);
  });

  it('any input puts the cards back to รอข้อมูล and says ข้อมูลเปลี่ยน กรุณาคำนวณใหม่ (P3; §9 row 4)', async () => {
    const el = await mount();
    await submit(el);
    cardsShow(el, [`${formatMoney('1234567.89')} THB`, `${formatMoney('-200.01')} THB`, `${formatMoney('42.00')} THB`]);
    type(field(el, DEFINITIONS[5][1]), '2500');
    await settle();
    cardsShow(el, [WAITING, WAITING, WAITING]);
    expect(text(alertLine(el))).toBe(CHANGED);
    expect(server.computeCalls().length, 'typing sends nothing').toBe(1);
  });

  it('the confirmation checkbox is an input too: unticking it resets the cards (P3)', async () => {
    const el = await mount();
    await submit(el);
    confirmBox(el).click();
    await settle();
    cardsShow(el, [WAITING, WAITING, WAITING]);
    expect(text(alertLine(el))).toBe(CHANGED);
  });
});

// ── W7: refusals ─────────────────────────────────────────────────────────────

describe('V2-cash-calculators: profit refusals through errorText (W7; profit-summary.js:13)', () => {
  for (const [code, message] of [
    ['profitIncomplete', 'กรอกข้อมูลกำไรและเงินสำรองให้ครบก่อนคำนวณ'],
    ['profitNegative', 'ต้นทุน ทุน และเงินสำรองต้องไม่ติดลบ'],
    ['originRejected', 'ORIGIN_REJECTED'],
  ] as const) {
    it(`${code}: the cards go back to รอข้อมูล and the alert shows the shared sentence`, async () => {
      let n = 0;
      server.answer = () => {
        n += 1;
        // the server's message differs from the shared table: the panel shows errorText's
        return n === 1 ? { status: 200, body: answered() } : refusal(400, code, `SERVER SAYS ${message}`);
      };
      const el = await mount();
      await submit(el);
      cardsShow(el, [`${formatMoney('1234567.89')} THB`, `${formatMoney('-200.01')} THB`, `${formatMoney('42.00')} THB`]);
      button(el, SUBMIT).click();
      await settle();
      cardsShow(el, [WAITING, WAITING, WAITING]);
      expect(shared[`error.${code}`], `error.${code} is a shared key`).toBeTruthy();
      expect(text(alertLine(el))).toBe(shared[`error.${code}`]);
    });
  }

  it('a code the shared table lacks shows the server message', async () => {
    server.answer = () => refusal(400, 'oddCode', 'An odd refusal');
    const el = await mount();
    await submit(el);
    cardsShow(el, [WAITING, WAITING, WAITING]);
    expect(text(alertLine(el))).toBe('An odd refusal');
  });
});

// ── W6: stale answers ────────────────────────────────────────────────────────

describe('V2-cash-calculators: a stale profit answer is dropped (W6; §9 row 3)', () => {
  it('an answer to a submit sent before the latest input change is not shown', async () => {
    const el = await mount();
    server.hold = true;
    await submit(el);
    expect(server.computeCalls().length).toBe(1);
    type(field(el, DEFINITIONS[2][1]), '5100');
    await settle();
    server.release(1);
    await settle();
    cardsShow(el, [WAITING, WAITING, WAITING]);
    expect(text(alertLine(el))).toBe(CHANGED);
  });

  it('a stale refusal is not shown either', async () => {
    server.answer = () => refusal(400, 'oddCode', 'A stale refusal');
    const el = await mount();
    server.hold = true;
    await submit(el);
    type(field(el, DEFINITIONS[2][1]), '5100');
    await settle();
    server.release(1);
    await settle();
    expect(text(alertLine(el))).toBe(CHANGED);
  });

  it('of two submits, only the later answer is shown, whichever arrives first', async () => {
    server.answer = (_body, n) => ({ status: 200, body: answered(n === 1 ? '111.11' : '222.22') });
    const el = await mount();
    server.hold = true;
    await submit(el);
    button(el, SUBMIT).click();
    await settle();
    expect(server.computeCalls().length, 'a second submit while the first is pending').toBe(2);
    server.release(1);
    await settle();
    expect(spaced(el)).not.toContain(formatMoney('111.11'));
    server.release(2);
    await settle();
    expect(spaced(el)).toContain(`กำไรที่เกิดขึ้นแล้ว ${formatMoney('222.22')} THB`);
  });

  it('of two submits, a late earlier answer does not replace the later one', async () => {
    server.answer = (_body, n) => ({ status: 200, body: answered(n === 1 ? '111.11' : '222.22') });
    const el = await mount();
    server.hold = true;
    await submit(el);
    button(el, SUBMIT).click();
    await settle();
    server.release(2);
    await settle();
    server.release(1);
    await settle();
    const all = spaced(el);
    expect(all).toContain(`กำไรที่เกิดขึ้นแล้ว ${formatMoney('222.22')} THB`);
    expect(all).not.toContain(formatMoney('111.11'));
  });
});
