// V2-cash-calculators (#511): the valuation panel of the shift detail (ticket §8–§10 rows W1–W4,
// W6–W8, §9 edge cases; design §12.20.1 V1–V7, §12.20.4, §12.20.6). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`, `src/devices-public/shift-valuation.js`): the panel `:9`, the
// submit `:11-12`, the result `:13-14`, the refusal `:15`. `<money>` is v2's `money()`
// (`admin-html:45`, en-US grouping, two places), here `formatMoney` of the server's decimal string.
// The result table's headers are v2's `table()` headers with its `~` marker (a numeric column,
// `admin-html:62`) stripped, as #467's shift table does.
//
// The panel computes nothing (W7, W8): the fixtures below carry figures that do not add up, so a
// panel that computed them would show something else. It sends each rate as typed, trimmed
// (controller, #511 issuecomment-6070066755), never the THB rate.
//
// The panel is mounted with the ticket's [proposed] props `{ api, branch, shiftId }` (plain
// values). Its requests go through `ApiClient.act('compute', …)`; whether the panel uses the `api`
// prop or builds its own `new ApiClient()` in its folder, the request reaches the global `fetch`,
// which the fake server below replaces and records.
//
// The panel is read through an eager glob, so on a branch without it each test fails on an
// assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import type { ShiftValuation, ShiftValuationCurrency } from '../../../../shared/api/generated/shift_valuation';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { formatMoney } from '../../../../shared/ui/MoneyText';

interface PanelProps {
  api: ApiClient;
  branch: string;
  shiftId: string;
}
interface PanelModule {
  ShiftValuationPanel: (props: PanelProps) => JSX.Element;
}

const panelFiles = import.meta.glob<PanelModule>('./ShiftValuationPanel.tsx', { eager: true });

function thePanel(): PanelModule['ShiftValuationPanel'] {
  const m = panelFiles['./ShiftValuationPanel.tsx'];
  expect(m, 'web/src/screens/admin/cash/calculators/ShiftValuationPanel.tsx exists').toBeDefined();
  expect(typeof m!.ShiftValuationPanel, 'it exports ShiftValuationPanel').toBe('function');
  return m!.ShiftValuationPanel;
}

const VALUATION = '/api/v3/compute/shift_valuation';
const BRANCH = 'B2';
const SHIFT = 'S-0042';
/** The shared table's sentence for a code (`errorText`'s source), read loosely so a missing key fails by assertion. */
const shared = sharedTh as Record<string, string>;

// ── the answers ──────────────────────────────────────────────────────────────

/**
 * The currencies the server lists, in the server's order. Deliberately not THB-then-A–Z (the
 * order the real server answers): a panel that sorted them by any rule would show another order.
 * EUR moved in and out back to zero and still needs a rate (§12.20.4).
 */
function listed(): ShiftValuationCurrency[] {
  return [
    { currency: 'THB', opening: '10000.00', current: '6600.00', needsRate: false },
    { currency: 'USD', opening: '100.00', current: '200.00', needsRate: true },
    { currency: 'EUR', opening: '0.00', current: '0.00', needsRate: true },
    { currency: 'AUD', opening: '1234567.00', current: '50.00', needsRate: true },
  ];
}

/** The answer to a submit: fresh figures (USD current 210.00, not the 200.00 listed on show). */
function computed(difference = '77.77'): ShiftValuation {
  const currencies = listed();
  currencies[1] = { ...currencies[1]!, current: '210.00' };
  return {
    currencies,
    result: {
      rows: [
        { currency: 'THB', rate: '1.000000', value: '6600.00' },
        { currency: 'USD', rate: '34.500000', value: '7245.00' },
        { currency: 'EUR', rate: '40.000000', value: '0.00' },
        { currency: 'AUD', rate: '22.123457', value: '1106.17' },
      ],
      // none of these follows from the rows: the panel shows what is answered
      opening: '27469123.45',
      current: '14951.17',
      movements: '500.00',
      difference,
      adjusted: '-1234.56',
    },
  };
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
  /** How the valuation answers a request: the list call has no `rates`. */
  answer: (body: Record<string, unknown>, n: number) => Reply = (body) =>
    body.rates == null
      ? { status: 200, body: { currencies: listed() } }
      : { status: 200, body: computed() };
  /** While set, every answer waits until `release(i)`. */
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

  /** Submits: valuation calls that carry rates. */
  submits(): Call[] {
    return this.calls.filter((c) => c.path === VALUATION && c.body.rates != null);
  }

  /** List-only calls: valuation calls without rates. */
  lists(): Call[] {
    return this.calls.filter((c) => c.path === VALUATION && c.body.rates == null);
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
    const call: Call = { path: url.pathname, method, headers, body };
    this.calls.push(call);
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
    if (method === 'POST' && url.pathname === VALUATION) {
      const reply = this.answer(body, this.calls.filter((c) => c.path === VALUATION).length - 1);
      if (this.hold) {
        const index = this.calls.filter((c) => c.path === VALUATION).length - 1;
        await new Promise<void>((go) => this.held.push({ index, go }));
      }
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

async function mount(shiftId = SHIFT): Promise<HTMLElement> {
  const Panel = thePanel();
  root = document.createElement('div');
  document.body.appendChild(root);
  const api = new ApiClient();
  dispose = render(() => <Panel api={api} branch={BRANCH} shiftId={shiftId} />, root);
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

/** Every text node, in order, joined by one space: markup-agnostic reading of "label value unit". */
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

function buttonsNamed(el: ParentNode, name: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = buttonsNamed(el, name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

/** The label's own text: without its control and its `<small>` hint. */
function ownText(l: HTMLLabelElement): string {
  const clone = l.cloneNode(true) as HTMLElement;
  clone.querySelectorAll('input,select,textarea,small').forEach((n) => n.remove());
  return norm(clone.textContent ?? '');
}

/** The control of the one label whose own text is `label`: nested in it, or named by `for`. */
function field<T extends HTMLElement = HTMLInputElement>(within: ParentNode, label: string): T {
  const labels = [...within.querySelectorAll<HTMLLabelElement>('label')].filter((l) => ownText(l) === label);
  expect(labels.length, `one field labelled ${JSON.stringify(label)}`).toBe(1);
  const l = labels[0]!;
  const nested = l.querySelector<T>('input,select,textarea');
  const named = l.htmlFor ? document.getElementById(l.htmlFor) : null;
  const control = (nested ?? named) as T | null;
  expect(control, `the control of ${JSON.stringify(label)}`).not.toBeNull();
  return control!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

/**
 * Types a value a number input's own sanitizer would turn into '' (" 34.5"): the control keeps a
 * valid value for the browser's checks, and its `value` reads back exactly what was typed.
 */
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

function chooseText(select: HTMLSelectElement, optionText: string): void {
  const option = [...select.options].find((o) => text(o) === optionText);
  expect(option, `an option ${JSON.stringify(optionText)}`).toBeDefined();
  select.value = option!.value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

const RATE_PLACEHOLDER = 'กรอกเรทอ้างอิง ธปท.';
const rateLabel = (c: string): string => `${c} → THB ต่อ 1 ${c}`;

function resultArea(el: ParentNode): HTMLElement {
  const areas = el.querySelectorAll<HTMLElement>('[aria-live="polite"]');
  expect(areas.length, 'one result area with aria-live="polite"').toBe(1);
  return areas[0]!;
}

/** The rate inputs, in the order shown. */
function rateInputs(el: ParentNode): HTMLInputElement[] {
  return [...el.querySelectorAll<HTMLInputElement>('input[type="number"]')];
}

/** Fills the date, the kind and the rates (th labels) and presses the button. */
async function submit(
  el: HTMLElement,
  rates: Record<string, string>,
  opts: { date?: string; kind?: string } = {},
): Promise<void> {
  type(field(el, 'วันที่เรทที่ ธปท. เผยแพร่'), opts.date ?? '2026-10-28');
  chooseText(field<HTMLSelectElement>(el, 'ประเภทเรท'), opts.kind ?? 'ซื้อตั๋วเงิน');
  for (const [c, v] of Object.entries(rates)) type(field(el, rateLabel(c)), v);
  button(el, 'คำนวณส่วนต่าง').click();
  await settle();
}

const RATES = { USD: '34.5', EUR: '40', AUD: '22.1234567' };

/** The table whose header row holds `header`, as rows of cell texts. */
function tableWith(el: ParentNode, header: string): { headers: string[]; rows: string[][] } {
  const tables = [...el.querySelectorAll('table')].filter((t) =>
    [...t.querySelectorAll('th')].some((th) => text(th) === header),
  );
  expect(tables.length, `one table with the header ${JSON.stringify(header)}`).toBe(1);
  return {
    headers: [...tables[0]!.querySelectorAll('thead th')].map((th) => text(th)),
    rows: [...tables[0]!.querySelectorAll('tbody tr')].map((tr) => [...tr.querySelectorAll('td')].map((td) => text(td))),
  };
}

// ── W1, W2: what the panel shows and asks on show ───────────────────────────

describe('V2-cash-calculators: the valuation panel on show (W1, W2; shift-valuation.js:9)', () => {
  it("asks the server once, through act('compute', 'shift_valuation'), with the branch and the shift only", async () => {
    await mount();
    const calls = server.computeCalls();
    expect(calls.length, 'one request on show').toBe(1);
    const c = calls[0]!;
    expect(c.path).toBe(VALUATION);
    expect(c.method).toBe('POST');
    expect(c.headers['x-xchange-request'], 'sent by ApiClient.act').toBe('1');
    expect(c.body.branch).toBe(BRANCH);
    expect(c.body.shiftId).toBe(SHIFT);
    expect(c.body.rates == null, 'no rates on show: the list-only call (§12.20.4 point 2)').toBe(true);
    expect(Object.keys(c.body).filter((k) => k !== 'rates').sort()).toEqual(['branch', 'shiftId']);
  });

  it("v2's texts, verbatim: heading, explanation, the note on hand-typed rates, the BOT link", async () => {
    const el = await mount();
    const all = spaced(el);
    for (const t of [
      'กำไรส่วนต่างจากการตีราคา (ประมาณการ)',
      'รวมเงินคงเหลือทุกสกุลเป็น THB แล้วเทียบเงินตั้งต้นด้วยเรทชุดเดียวกัน ไม่บวกเงินตั้งต้นซ้ำ',
      'กรอกเรทจาก ธปท. ด้วยตนเอง ยังไม่ดึงข้อมูลอัตโนมัติ ตัวเลขนี้ไม่ใช่กำไรสุทธิหรือกำไรที่รับรู้แล้ว และไม่รวมบัญชีธนาคาร/หนี้สิน',
      'กรอกเป็นบาทต่อ 1 หน่วยเสมอ หากแหล่งข้อมูลระบุ 100 JPY ให้หาร 100 ก่อน',
    ]) {
      expect(all, t).toContain(t);
    }
    const headings = [...el.querySelectorAll('h1,h2,h3,h4,h5,h6')].map((h) => text(h));
    expect(headings, 'the heading is a heading').toContain('กำไรส่วนต่างจากการตีราคา (ประมาณการ)');
    const links = [...el.querySelectorAll<HTMLAnchorElement>('a')].filter((a) => text(a) === 'เปิดอัตราแลกเปลี่ยน ธปท. ↗');
    expect(links.length, 'one BOT link').toBe(1);
    const a = links[0]!;
    expect(a.getAttribute('href')).toBe('https://www.bot.or.th/th/statistics/exchange-rate.html');
    expect(a.getAttribute('target')).toBe('_blank');
    expect((a.getAttribute('rel') ?? '').split(/\s+/).sort()).toEqual(['noopener', 'noreferrer']);
  });

  it('the required date and kind inputs, one rate input per currency needing a rate (server order, no THB), the button and the live result area', async () => {
    const el = await mount();
    const date = field(el, 'วันที่เรทที่ ธปท. เผยแพร่');
    expect(date.getAttribute('type')).toBe('date');
    expect(date.required).toBe(true);
    const kind = field<HTMLSelectElement>(el, 'ประเภทเรท');
    expect(kind.tagName).toBe('SELECT');
    expect([...kind.options].map((o) => text(o))).toEqual(['ซื้อเงินโอน', 'ซื้อตั๋วเงิน', 'ขาย']);

    // USD, EUR, AUD: the listed order, THB skipped (needsRate false)
    const inputs = rateInputs(el);
    expect(inputs.length, 'one rate input per needsRate currency').toBe(3);
    for (const [i, c] of ['USD', 'EUR', 'AUD'].entries()) {
      const input = field(el, rateLabel(c));
      expect(input, `${c} is the input number ${i + 1}`).toBe(inputs[i]);
      expect(input.getAttribute('type')).toBe('number');
      expect(input.getAttribute('min')).toBe('0.000001');
      expect(input.getAttribute('max')).toBe('1000000');
      expect(input.getAttribute('step')).toBe('any');
      expect(input.required, `${c} is required`).toBe(true);
      expect(input.getAttribute('placeholder')).toBe(RATE_PLACEHOLDER);
    }
    expect(
      [...el.querySelectorAll<HTMLLabelElement>('label')].some((l) => ownText(l) === rateLabel('THB')),
      'no rate input for THB',
    ).toBe(false);
    button(el, 'คำนวณส่วนต่าง');
    expect(text(resultArea(el)), 'the result area starts empty').toBe('');
  });

  it("renders the inputs the server's list names — not a fixed register", async () => {
    server.answer = () => ({
      status: 200,
      body: {
        currencies: [
          { currency: 'THB', opening: '1.00', current: '1.00', needsRate: false },
          { currency: 'JPY', opening: '0.00', current: '9.00', needsRate: true },
        ],
      },
    });
    const el = await mount();
    expect(rateInputs(el).length).toBe(1);
    field(el, rateLabel('JPY'));
  });

  it('a refused first call (notFound) shows the shared sentence in the result area (§9 row 1, W7)', async () => {
    server.answer = () => refusal(404, 'notFound', 'Not found', 'shiftId');
    const el = await mount();
    expect(text(resultArea(el))).toBe(shared['error.notFound']);
  });

  it('a refused first call with a code the shared table lacks shows the server message (errorText)', async () => {
    server.answer = () => refusal(400, 'somethingElse', 'Something else went wrong');
    const el = await mount();
    expect(text(resultArea(el))).toBe('Something else went wrong');
  });

  it('polls nothing: no request after the first while time passes (§12.6)', async () => {
    const el = await mount();
    expect(el).toBeDefined();
    vi.useFakeTimers();
    await vi.advanceTimersByTimeAsync(60_000);
    vi.useRealTimers();
    await settle();
    expect(server.computeCalls().length).toBe(1);
  });
});

// ── W3: the submit and the result ────────────────────────────────────────────

describe('V2-cash-calculators: the valuation submit and its result (W3; shift-valuation.js:11-14)', () => {
  it('sends the branch, the shift and each needed rate as a typed string — never THB', async () => {
    const el = await mount();
    await submit(el, RATES);
    const sent = server.submits();
    expect(sent.length).toBe(1);
    const c = sent[0]!;
    expect(c.headers['x-xchange-request']).toBe('1');
    expect(c.body).toEqual({ branch: BRANCH, shiftId: SHIFT, rates: { USD: '34.5', EUR: '40', AUD: '22.1234567' } });
  });

  it('trims each typed rate before sending: " 34.5 " is sent as "34.5" (controller, issuecomment-6070066755)', async () => {
    const el = await mount();
    type(field(el, 'วันที่เรทที่ ธปท. เผยแพร่'), '2026-10-28');
    typeRaw(field(el, rateLabel('USD')), ' 34.5 ');
    typeRaw(field(el, rateLabel('EUR')), '40 ');
    typeRaw(field(el, rateLabel('AUD')), ' 1');
    button(el, 'คำนวณส่วนต่าง').click();
    await settle();
    const sent = server.submits();
    expect(sent.length).toBe(1);
    expect(sent[0]!.body.rates).toEqual({ USD: '34.5', EUR: '40', AUD: '1' });
  });

  it("shows the answer: the date and kind as typed, ผู้ใช้กรอกเอง, คำนวณเมื่อ with the browser's clock (th-TH)", async () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    const now = new Date('2026-10-28T05:06:07.000Z');
    vi.setSystemTime(now);
    const el = await mount();
    await submit(el, RATES, { date: '2026-10-27', kind: 'ซื้อตั๋วเงิน' });
    const all = spaced(resultArea(el));
    expect(all).toContain('เรทวันที่ 2026-10-27 · ซื้อตั๋วเงิน · ผู้ใช้กรอกเอง');
    expect(all).toContain(`คำนวณเมื่อ ${norm(now.toLocaleString('th-TH'))}`);
  });

  it("the table: one row per answered currency, in the answer's order, with the answer's figures and the server's rate string", async () => {
    const el = await mount();
    await submit(el, RATES);
    const t = tableWith(resultArea(el), 'เรท THB');
    expect(t.headers).toEqual(['สกุล', 'ตั้งต้น', 'คงเหลือตามระบบ', 'เรท THB', 'มูลค่าคงเหลือ THB']);
    expect(t.rows).toEqual([
      ['THB', formatMoney('10000.00'), formatMoney('6600.00'), '1.000000', formatMoney('6600.00')],
      // 210.00: the submit's fresh figure, not the 200.00 listed on show
      ['USD', formatMoney('100.00'), formatMoney('210.00'), '34.500000', formatMoney('7245.00')],
      ['EUR', '0.00', '0.00', '40.000000', '0.00'],
      ['AUD', formatMoney('1234567.00'), formatMoney('50.00'), '22.123457', formatMoney('1106.17')],
    ]);
  });

  it('the five figures, each `<money> THB` as answered, the last one bold, and the closing note', async () => {
    const el = await mount();
    await submit(el, RATES);
    const area = resultArea(el);
    const all = spaced(area);
    for (const [label, value] of [
      ['เงินคงเหลือรวมเป็นบาท', '14951.17'],
      ['เงินตั้งต้นรวมเป็นบาท (เรทเดียวกัน)', '27469123.45'],
      ['ส่วนต่างก่อนปรับรับ–จ่ายอื่น', '77.77'],
      ['รับ–จ่ายเงินอื่นสุทธิ (รับเข้า − จ่ายออก)', '500.00'],
      ['ส่วนต่างหลังตัดผลรับ–จ่ายอื่น', '-1234.56'],
    ] as const) {
      expect(all, label).toContain(`${label} ${formatMoney(value)} THB`);
    }
    const bold = [...area.querySelectorAll('strong, b')].map((b) => spaced(b));
    expect(bold, 'the adjusted figure is bold').toContain(`${formatMoney('-1234.56')} THB`);
    expect(all).toContain(
      'ค่าบวก = มูลค่าเพิ่ม / ค่าลบ = มูลค่าลด ไม่ใช่กำไรสุทธิ รายการรับ–จ่ายอื่นอาจรวมค่าใช้จ่าย จึงแสดงการปรับแยก ไม่รวมส่วนต่างตรวจนับเงินจริง (Cash Variance) และยังไม่บันทึกผลคำนวณลงบัญชี',
    );
  });

  it('a result is not cleared when an input changes afterwards (V7; §9 row 5)', async () => {
    const el = await mount();
    await submit(el, RATES);
    type(field(el, rateLabel('USD')), '35');
    await settle();
    expect(spaced(resultArea(el))).toContain(`ส่วนต่างก่อนปรับรับ–จ่ายอื่น ${formatMoney('77.77')} THB`);
    expect(server.submits().length, 'typing sends nothing').toBe(1);
  });
});

// ── W4, W7: refusals on submit ───────────────────────────────────────────────

describe('V2-cash-calculators: valuation refusals (W4, W7; shift-valuation.js:15)', () => {
  it('valuationRate: re-reads the currencies, keeps what was typed, renders a new one, then shows the shared sentence (§9 row 2)', async () => {
    let refused = false;
    server.answer = (body) => {
      if (body.rates != null) {
        refused = true;
        // the server's message differs from the shared table: the panel shows errorText's
        return refusal(400, 'valuationRate', 'SERVER SENTENCE', 'rates');
      }
      const currencies = listed();
      // the shift gained GBP since the panel was shown
      if (refused) currencies.push({ currency: 'GBP', opening: '0.00', current: '5.00', needsRate: true });
      return { status: 200, body: { currencies } };
    };
    const el = await mount();
    await submit(el, RATES);
    expect(server.calls.filter((c) => c.path === VALUATION).map((c) => (c.body.rates == null ? 'list' : 'submit'))).toEqual([
      'list',
      'submit',
      'list',
    ]);
    const reread = server.lists()[1]!;
    expect(reread.body.branch).toBe(BRANCH);
    expect(reread.body.shiftId).toBe(SHIFT);
    expect(field(el, rateLabel('USD')).value, 'the typed USD rate is kept').toBe('34.5');
    expect(field(el, rateLabel('EUR')).value).toBe('40');
    expect(field(el, rateLabel('AUD')).value).toBe('22.1234567');
    expect(field(el, rateLabel('GBP')).value, 'the new currency asks for its rate').toBe('');
    expect(text(resultArea(el))).toBe('กรอกเรทให้ครบทุกสกุลที่มียอด');
    expect(text(resultArea(el))).not.toContain('SERVER SENTENCE');
  });

  it('a refusal replaces a previous result with the sentence', async () => {
    let n = 0;
    server.answer = (body) => {
      if (body.rates == null) return { status: 200, body: { currencies: listed() } };
      n += 1;
      return n === 1 ? { status: 200, body: computed() } : refusal(400, 'originRejected', 'ORIGIN_REJECTED');
    };
    const el = await mount();
    await submit(el, RATES);
    expect(spaced(resultArea(el))).toContain(formatMoney('77.77'));
    button(el, 'คำนวณส่วนต่าง').click();
    await settle();
    const area = resultArea(el);
    expect(area.querySelector('table'), 'no table left').toBeNull();
    expect(text(area)).not.toContain(formatMoney('77.77'));
    expect(text(area)).toBe(shared['error.originRejected']);
    expect(shared['error.originRejected'], 'error.originRejected is a shared key').toBeTruthy();
  });

  it('a code the shared table lacks shows the server message', async () => {
    server.answer = (body) =>
      body.rates == null ? { status: 200, body: { currencies: listed() } } : refusal(400, 'oddCode', 'An odd refusal');
    const el = await mount();
    await submit(el, RATES);
    expect(text(resultArea(el))).toBe('An odd refusal');
  });
});

// ── W6: stale answers ────────────────────────────────────────────────────────

describe('V2-cash-calculators: a stale valuation answer is dropped (W6; §9 row 3)', () => {
  it('an answer to a submit sent before the latest input change is not shown', async () => {
    const el = await mount();
    server.hold = true;
    await submit(el, RATES);
    expect(server.submits().length).toBe(1);
    type(field(el, rateLabel('USD')), '36');
    await settle();
    server.release(1);
    await settle();
    const area = resultArea(el);
    expect(area.querySelector('table'), 'the stale result is dropped').toBeNull();
    expect(text(area)).not.toContain(formatMoney('77.77'));
  });

  it('a stale refusal is not shown either', async () => {
    server.answer = (body) =>
      body.rates == null ? { status: 200, body: { currencies: listed() } } : refusal(400, 'oddCode', 'A stale refusal');
    const el = await mount();
    server.hold = true;
    await submit(el, RATES);
    type(field(el, rateLabel('EUR')), '41');
    await settle();
    server.release(1);
    await settle();
    expect(text(resultArea(el))).not.toContain('A stale refusal');
  });

  it('of two submits, only the later answer is shown, whichever arrives first', async () => {
    server.answer = (body, n) =>
      body.rates == null ? { status: 200, body: { currencies: listed() } } : { status: 200, body: computed(n === 1 ? '111.11' : '222.22') };
    const el = await mount();
    server.hold = true;
    await submit(el, RATES);
    button(el, 'คำนวณส่วนต่าง').click();
    await settle();
    expect(server.submits().length, 'a second submit while the first is pending').toBe(2);
    // the earlier answer arrives first: dropped
    server.release(1);
    await settle();
    expect(text(resultArea(el))).not.toContain(formatMoney('111.11'));
    server.release(2);
    await settle();
    expect(spaced(resultArea(el))).toContain(`ส่วนต่างก่อนปรับรับ–จ่ายอื่น ${formatMoney('222.22')} THB`);
  });

  it('of two submits, a late earlier answer does not replace the later one', async () => {
    server.answer = (body, n) =>
      body.rates == null ? { status: 200, body: { currencies: listed() } } : { status: 200, body: computed(n === 1 ? '111.11' : '222.22') };
    const el = await mount();
    server.hold = true;
    await submit(el, RATES);
    button(el, 'คำนวณส่วนต่าง').click();
    await settle();
    server.release(2);
    await settle();
    server.release(1);
    await settle();
    const all = spaced(resultArea(el));
    expect(all).toContain(`ส่วนต่างก่อนปรับรับ–จ่ายอื่น ${formatMoney('222.22')} THB`);
    expect(all).not.toContain(formatMoney('111.11'));
  });
});
