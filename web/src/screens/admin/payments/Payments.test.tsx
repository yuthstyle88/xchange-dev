// V2-payments (#470): the admin page's payments menu — the per-branch payment-method settings,
// their save and their history (ticket §7, §8, §9, §10 rows Y1–Y5; design §12.3, §12.5, §12.6 in
// PR 474). Written before the implementation by the lane controller (claude/7ca80c). Do not edit,
// delete or weaken; add tests in a file of your own.
//
// Source of truth (W9): xchange-v2-source@4d0bdbb `src/devices-public/admin-payments.js:1-35` and
// the served nav row `admin-workspace.js:11-20`, `:87`, `:146`. Every text is v2's Thai verbatim
// (W6), read through this folder's own table `i18n.ts` (W10). Each channel's card is found by v2's
// own attribute `data-payment-id` (`admin-payments.js:15`) and its controls by their label words.
//
// The menu is mounted as the shell mounts it (`menu.Component` with `branch`, `period`, `shown`).
// It gets no `ApiClient` from the shell (`AdminMenuProps`), so it talks through an `ApiClient` of
// its own, whose requests reach the global `fetch`: the fake below is the server. It answers
// - `GET /api/v3/views/admin_payments?branch=B[&since=v]` — `{changed: false}` when `since` is B's
//   version, otherwise `{changed: true, data}` with `data` = `{branchId, settingsVersion, methods,
//   updatedAt, updatedBy, canEdit, history}` (the view of this ticket; the history keys are the
//   view's camelCase ones), or a refusal;
// - `GET /api/v3/views/admin_branches` — the shell's branch list `{branches: [{id, name}]}`, which
//   gives the heading its branch name (v2 reads the picker's option text, `:15`);
// - `POST /api/payment-methods` `{action: 'save', payload}` — v2's door, unchanged (ticket §7):
//   the saved list answer, or v2's refusal `{"error": "<sentence>"}` with v2's status (§5.5.2).
// Every other request is recorded and fails the test. AppShell's page-wide reads
// (`PAGE_WIDE`, `shared/testing/page-wide.ts`) are answered and never counted.
//
// `menu.ts`, the table and the menu list are read through eager globs, so on a branch without them
// each test fails on an assertion that the file exists, not on the import.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: 'all'` pin → `needs: 'all'` (#1030 §3.10: `MenuDef.roles` becomes `needs`). Nothing else changes.
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { requestRefresh } from '../parts/refresh';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

interface Table {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuModules = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const tableModules = import.meta.glob<Table>('./i18n.ts', { eager: true });
const listModules = import.meta.glob<{ menus: AdminMenu[] }>('../menus.ts', { eager: true });
const sources = import.meta.glob<string>(['./*.ts', './*.tsx'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

function theMenu(): AdminMenu {
  const m = menuModules['./menu.ts'];
  expect(m, 'web/src/screens/admin/payments/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

function table(): Table {
  const m = tableModules['./i18n.ts'];
  expect(m, 'web/src/screens/admin/payments/i18n.ts exists').toBeDefined();
  expect(m!.th, 'i18n.ts exports `th`').toBeDefined();
  expect(m!.en, 'i18n.ts exports `en`').toBeDefined();
  return m!;
}

/** Thai letters, vowels, marks and digits — not v2's baht glyph ฿ (U+0E3F), Cash's icon (`:7`). */
const THAI = /[\u0E01-\u0E3A\u0E40-\u0E5B]/;

// ── v2's texts (admin-payments.js, admin-workspace.js) ───────────────────────

const T = {
  // the menu record (admin-workspace.js:17, :87; admin-payments.js:6)
  label: 'วิธีชำระเงิน',
  eyebrow: 'PAYMENT SETTINGS',
  title: 'ตั้งค่าวิธีชำระเงิน',
  subtitle: 'เปิด–ปิดช่องทาง กำหนดข้อความแนะนำ และดูประวัติการตั้งค่า',
  // loading / error (:13)
  loading: 'กำลังโหลดวิธีชำระเงิน…',
  retry: 'โหลดอีกครั้ง',
  // intro and meta (:15)
  introEyebrow: 'PAYMENT CHANNELS',
  headingOf: 'ช่องทางของ',
  intro: 'ร้านขายเงินต่างประเทศ = ร้านรับเงินบาท · ร้านรับซื้อเงินต่างประเทศ = ร้านจ่ายเงินบาท',
  neverEdited: 'ใช้ค่าเริ่มต้นของระบบ · ยังไม่เคยแก้ไข',
  reload: 'โหลดล่าสุด',
  readOnly: 'ดูการตั้งค่าได้ · ให้เจ้าของระบบเป็นผู้แก้ไข',
  // a card (:15)
  enabled: 'เปิดใช้',
  sell: 'รับเงินบาทจากลูกค้า',
  sellNote: 'ร้านขายเงินต่างประเทศ',
  buy: 'จ่ายเงินบาทให้ลูกค้า',
  buyNote: 'ร้านรับซื้อเงินต่างประเทศ',
  receiveOnly: 'ช่องทางนี้ใช้รับเงินเท่านั้น',
  account: 'ชื่อบัญชี / จุดรับ–จ่าย',
  accountUnit: 'สำหรับพนักงาน',
  accountCash: 'เช่น เงินสดเคาน์เตอร์ 1',
  accountOther: 'เช่น บัญชีบริษัท / เครื่องรับบัตร 1',
  instruction: 'ข้อความแนะนำลูกค้า',
  instructionUnit: 'แสดงเมื่อเลือกช่องทางนี้',
  instructionPlaceholder: 'พิมพ์ภาษาไทยหรืออังกฤษสำหรับลูกค้า',
  evidenceCash: 'เงินสด: พนักงานตรวจนับก่อนยืนยัน',
  evidenceOther: 'ต้องมีเลขอ้างอิงและหลักฐานก่อนยืนยันรายการ',
  // the save bar (:15, :17, :24-25)
  clean: 'บันทึกแล้ว / ยังไม่มีการแก้ไข',
  dirty: 'มีการแก้ไขที่ยังไม่บันทึก',
  saveNote: 'ใช้กับคำขอใหม่และการยืนยันรายการ · ไม่เปลี่ยนประวัติที่บันทึกสำเร็จแล้ว',
  discard: 'ยกเลิกการแก้ไข',
  save: 'บันทึกการตั้งค่า',
  saving: 'กำลังบันทึก…',
  saved: 'บันทึกการตั้งค่าสำเร็จ',
  footnote:
    'หากปิดช่องทางที่คิวค้างเลือกไว้ พนักงานต้องตรวจสอบและให้ลูกค้าเลือกใหม่ · การตั้งค่านี้ไม่ได้เชื่อมระบบตัดบัตรหรือยืนยันยอดธนาคารอัตโนมัติ',
  // history (:11, :15)
  historyHeaders: ['วันที่ / เวลา', 'ผู้แก้ไข', 'ช่องทางที่เปลี่ยน', 'รุ่น'],
  noHistory: 'ยังไม่มีประวัติการแก้ไข',
  unchanged: 'ยืนยันค่าชุดเดิม',
  // confirms (:18, :32)
  confirmReload: 'โหลดล่าสุดและทิ้งการแก้ไขที่ยังไม่บันทึก?',
  confirmBranch: 'เปลี่ยนสาขาและทิ้งการแก้ไขวิธีชำระเงินที่ยังไม่บันทึก?',
} as const;

/** The five channels in v2's order with v2's Thai name and second line (`admin-payments.js:7`). */
const CHANNELS = [
  { id: 'Cash', th: 'เงินสด', line: 'Cash' },
  { id: 'Credit Card', th: 'บัตรเครดิต', line: 'Credit card' },
  { id: 'Alipay', th: 'Alipay', line: 'Alipay' },
  { id: 'PromptPay', th: 'พร้อมเพย์', line: 'PromptPay' },
  { id: 'Bank Transfer', th: 'โอนธนาคาร', line: 'Bank transfer' },
] as const;
const BUY_CHANNELS = ['Cash', 'Bank Transfer'];

// v2's refusals of /api/payment-methods (§5.5.2 rows 7, 12, 16)
const OWNER_ONLY = 'เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้';
const NO_DIRECTION = 'ช่องทางที่เปิดต้องเลือกการรับเงินหรือจ่ายเงินอย่างน้อยหนึ่งประเภท';
const STALE = 'มีผู้แก้ไขการตั้งค่านี้แล้ว กรุณาโหลดล่าสุดก่อนบันทึก';

// ── the fake server ──────────────────────────────────────────────────────────

interface Row {
  id: string;
  enabled: boolean;
  buy: boolean;
  sell: boolean;
  account: string;
  instruction: string;
}

interface Entry {
  version: number;
  actor: string;
  createdAt: string;
  beforeMethods: Row[];
  afterMethods: Row[];
}

interface Settings {
  version: number;
  branchId: string;
  settingsVersion: number;
  methods: Row[];
  updatedAt: string | null;
  updatedBy: string | null;
  canEdit: boolean;
  history: Entry[];
}

interface Read {
  view: string;
  branch: string | null;
  since: string | null;
}

interface Post {
  method: string;
  marker: string | null;
  path: string;
  body: unknown;
}


/** 03:30 on 8 Oct in Bangkok, still 7 Oct in UTC: a date shown without Asia/Bangkok is wrong. */
const EDITED_AT = '2026-10-07T20:30:00.000Z';
const SAVED_AT = '2026-10-08T05:15:00.000Z';

function defaults(): Row[] {
  return CHANNELS.map((c) => ({
    id: c.id,
    enabled: true,
    buy: BUY_CHANNELS.includes(c.id),
    sell: true,
    account: '',
    instruction: '',
  }));
}

function mainRows(): Row[] {
  return [
    { id: 'Cash', enabled: true, buy: true, sell: true, account: 'Counter 1', instruction: 'Pay at counter 1' },
    { id: 'Credit Card', enabled: false, buy: false, sell: true, account: 'Card machine 2', instruction: 'Cards over 500 only' },
    { id: 'Alipay', enabled: true, buy: false, sell: true, account: 'Alipay till', instruction: 'Scan the shop code' },
    { id: 'PromptPay', enabled: true, buy: false, sell: true, account: 'PromptPay 0812345678', instruction: 'Show the slip' },
    { id: 'Bank Transfer', enabled: true, buy: true, sell: false, account: 'Company account', instruction: 'Transfer then wait' },
  ];
}

function with_(rows: Row[], id: string, change: Partial<Row>): Row[] {
  return rows.map((r) => (r.id === id ? { ...r, ...change } : { ...r }));
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  branches = [
    { id: 'MAIN', name: 'Main Desk' },
    { id: 'A1', name: 'Alpha Desk' },
  ];
  settings: Record<string, Settings> = {
    MAIN: {
      version: 41,
      branchId: 'MAIN',
      settingsVersion: 3,
      methods: mainRows(),
      updatedAt: EDITED_AT,
      updatedBy: 'somchai',
      canEdit: true,
      history: [
        // v3: saved the same rows again
        { version: 3, actor: 'somchai', createdAt: EDITED_AT, beforeMethods: mainRows(), afterMethods: mainRows() },
        // v2: Cash and PromptPay changed
        {
          version: 2,
          actor: 'owner',
          createdAt: '2026-10-06T02:00:00.000Z',
          beforeMethods: with_(with_(mainRows(), 'Cash', { account: 'Old counter' }), 'PromptPay', { enabled: false }),
          afterMethods: mainRows(),
        },
        // v1: from nothing (a `before` without the rows: every channel counts as changed)
        { version: 1, actor: 'owner', createdAt: '2026-10-05T02:00:00.000Z', beforeMethods: [], afterMethods: defaults() },
      ],
    },
    A1: {
      // the same version as MAIN: a cursor carried over from MAIN would answer `changed: false`
      version: 41,
      branchId: 'A1',
      settingsVersion: 0,
      methods: defaults(),
      updatedAt: null,
      updatedBy: null,
      canEdit: true,
      history: [],
    },
  };
  /** When set, `admin_payments` answers this refusal. */
  viewRefusal: { status: number; code: string; message: string } | null = null;
  /** When set, the save is refused with v2's `{error}` body and this status. */
  saveRefusal: { status: number; sentence: string } | null = null;
  /** When set, the next save (or read) waits until `release()`. */
  holdSave = false;
  holdRead = false;
  private released: (() => void)[] = [];
  reads: Read[] = [];
  posts: Post[] = [];
  others: string[] = [];

  release(): void {
    const r = this.released;
    this.released = [];
    for (const f of r) f();
  }

  private wait(): Promise<void> {
    return new Promise((resolve) => this.released.push(resolve));
  }

  readsOf(view: string): Read[] {
    return this.reads.filter((r) => r.view === view);
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const raw = String(input);
    const url = new URL(raw, 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && PAGE_WIDE.includes(url.pathname)) {
      if (url.pathname.endsWith('/me')) {
        return json({ version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } });
      }
      return json({ version: 1, changed: false });
    }
    const view = url.pathname.startsWith('/api/v3/views/') ? url.pathname.slice('/api/v3/views/'.length) : null;
    if (method === 'GET' && view === 'admin_payments') {
      const branch = url.searchParams.get('branch');
      const since = url.searchParams.get('since');
      this.reads.push({ view, branch, since });
      if (this.holdRead) await this.wait();
      if (this.viewRefusal) {
        const r = this.viewRefusal;
        return json({ error: { code: r.code, message: r.message } }, r.status);
      }
      const s = this.settings[branch ?? 'MAIN'];
      if (!s) return json({ error: { code: 'noSuchBranch', field: 'branch', message: 'ไม่พบสาขาที่เลือก กรุณารีเฟรช' } }, 400);
      if (since !== null && since === String(s.version)) return json({ version: s.version, changed: false });
      const { version, ...data } = s;
      return json({ version, changed: true, data: JSON.parse(JSON.stringify(data)) });
    }
    if (method === 'GET' && view === 'admin_branches') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      return json({ version: 1, changed: true, data: { branches: this.branches.map((b) => ({ ...b })) } });
    }
    if (method === 'POST' && url.pathname === '/api/payment-methods') {
      const headers = new Headers(init?.headers);
      const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: { branchId: string; version: number; methods: Row[] } };
      this.posts.push({ method, marker: headers.get('x-xchange-request'), path: url.pathname, body });
      if (this.holdSave) await this.wait();
      if (this.saveRefusal) return json({ error: this.saveRefusal.sentence }, this.saveRefusal.status);
      return json(this.apply(body.payload));
    }
    this.others.push(`${method} ${raw}`);
    return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  };

  /** A save as v2's door makes it: the setting's version and the branch's version both move. */
  private apply(payload: { branchId: string; version: number; methods: Row[] }): unknown {
    const s = this.settings[payload.branchId]!;
    const entry: Entry = {
      version: s.settingsVersion + 1,
      actor: 'owner',
      createdAt: SAVED_AT,
      beforeMethods: s.methods,
      afterMethods: payload.methods,
    };
    s.version += 1;
    s.settingsVersion += 1;
    s.methods = payload.methods.map((r) => ({ ...r }));
    s.updatedAt = SAVED_AT;
    s.updatedBy = 'owner';
    s.history = [entry, ...s.history].slice(0, 20);
    return {
      branchId: s.branchId,
      version: s.settingsVersion,
      methods: s.methods,
      updatedAt: s.updatedAt,
      updatedBy: s.updatedBy,
      canEdit: true,
      history: s.history.map((h) => ({
        version: h.version,
        actor: h.actor,
        created_at: h.createdAt,
        before_methods: h.beforeMethods,
        after_methods: h.afterMethods,
      })),
    };
  }
}

let server: Server;

// ── rendering ────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let container: HTMLDivElement | undefined;
const realSetTimeout = globalThis.setTimeout;

/** Lets the menu's promises and effects run (with or without fake timers). */
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

interface Mounted {
  setBranch: (b: string) => void;
  setShown: (s: boolean) => void;
}

/** The menu as the shell hosts it. */
async function mount(opts: { branch?: string; shown?: boolean } = {}): Promise<Mounted> {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [shown, setShown] = createSignal(opts.shown ?? true);
  const period: Accessor<{ from: string; to: string }> = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  const Component = menu.Component;
  dispose = render((): JSX.Element => <Component branch={branch} period={period} shown={shown} />, container);
  await settle();
  return { setBranch, setShown };
}

// ── finding things the way a person does ─────────────────────────────────────

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function page(): HTMLElement {
  expect(container, 'the menu is mounted').toBeDefined();
  return container!;
}

function words(): string {
  return norm(page().textContent);
}

function buttons(root: ParentNode, text: string): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>('button')).filter((b) => norm(b.textContent) === text);
}

function button(root: ParentNode, text: string): HTMLButtonElement {
  const found = buttons(root, text);
  expect(found.length, `one button "${text}"`).toBe(1);
  return found[0]!;
}

/** A label's own words, without the text of the control inside it. */
function labelWords(label: HTMLLabelElement): string {
  const copy = label.cloneNode(true) as HTMLElement;
  for (const c of Array.from(copy.querySelectorAll('input, select, textarea, button'))) c.remove();
  return norm(copy.textContent);
}

type Control = HTMLInputElement | HTMLTextAreaElement;

/** The controls whose label's own words start with `start`, inside `root`. */
function controls(root: ParentNode, start: string): Control[] {
  const labels = Array.from(root.querySelectorAll<HTMLLabelElement>('label')).filter((l) => labelWords(l).startsWith(start));
  return labels.map((label) => {
    const control =
      (label.htmlFor ? document.getElementById(label.htmlFor) : null) ?? label.querySelector('input, textarea');
    expect(control, `label "${start}" names a control`).not.toBeNull();
    return control as Control;
  });
}

function control<T extends Control = HTMLInputElement>(root: ParentNode, start: string): T {
  const found = controls(root, start);
  expect(found.length, `one control labelled "${start}…"`).toBe(1);
  return found[0] as T;
}

function card(id: string): HTMLElement {
  const found = Array.from(page().querySelectorAll<HTMLElement>('[data-payment-id]')).filter((e) => e.dataset.paymentId === id);
  expect(found.length, `one card data-payment-id="${id}"`).toBe(1);
  return found[0]!;
}

function cardIds(): string[] {
  return Array.from(page().querySelectorAll<HTMLElement>('[data-payment-id]')).map((e) => e.dataset.paymentId ?? '');
}

/** Disabled itself, or inside a disabled <fieldset> (v2 locks the form that way, `:15`, `:24`). */
function isDisabled(el: HTMLInputElement | HTMLTextAreaElement | HTMLButtonElement): boolean {
  return el.disabled || el.closest('fieldset[disabled]') !== null;
}

function type(input: Control, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

/** v2's feedback line (`#payment-feedback`, `role="status"`). */
function feedback(): string {
  return Array.from(page().querySelectorAll('[role="status"]'))
    .map((e) => norm(e.textContent))
    .join(' | ');
}

function historyDetails(): HTMLDetailsElement {
  // found by its table (four headers), so it is found in either language
  const all = Array.from(page().querySelectorAll<HTMLDetailsElement>('details')).filter(
    (d) => d.querySelector('summary') !== null && d.querySelectorAll('table thead th').length === 4,
  );
  expect(all.length, "one <details> holding v2's four-column history table").toBe(1);
  return all[0]!;
}

function historyRows(): string[][] {
  return Array.from(historyDetails().querySelectorAll<HTMLTableRowElement>('tbody tr')).map((r) =>
    Array.from(r.cells).map((c) => norm(c.textContent)),
  );
}

/** The date as v2 shows it (`admin-payments.js:10`) in `locale`. */
function dateText(iso: string, locale: 'th-TH' | 'en-GB'): string {
  return norm(new Date(iso).toLocaleString(locale, { timeZone: 'Asia/Bangkok' }));
}

function readsSince(mark: number, view: string): Read[] {
  return server.reads.slice(mark).filter((r) => r.view === view);
}

/** The body of the one save that was posted. */
function onlySave(): { action: string; payload: { branchId: string; version: number; methods: Row[] } } {
  expect(server.posts.length, 'one save was posted').toBe(1);
  const p = server.posts[0]!;
  expect(p.path).toBe('/api/payment-methods');
  expect(p.marker, 'v2\'s marker header').toBe('1');
  return p.body as { action: string; payload: { branchId: string; version: number; methods: Row[] } };
}

/** Untick a checkbox the way a person does. */
function untick(box: Control): void {
  if ((box as HTMLInputElement).checked) (box as HTMLInputElement).click();
}

function tick(box: Control): void {
  if (!(box as HTMLInputElement).checked) (box as HTMLInputElement).click();
}

// ── set-up ───────────────────────────────────────────────────────────────────

// The dates must be shown in Asia/Bangkok whatever the machine's zone (`admin-payments.js:10`), so
// the file runs in a zone where Bangkok's wall clock differs (restored afterwards).
const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
let zone: string | undefined;
beforeAll(() => {
  zone = env.TZ;
  env.TZ = 'America/New_York';
});
afterAll(() => {
  if (zone === undefined) delete env.TZ;
  else env.TZ = zone;
});

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  server.release();
  await settle();
  if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
  dispose?.();
  dispose = undefined;
  container?.remove();
  container = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

// ── the menu record (ticket §7; design §12.3) ────────────────────────────────

describe('V2-payments: the menu record (#470 §7)', () => {
  it("is v2's `payments` page in group money at order 6, undated, for every role", () => {
    const menu = theMenu();
    expect(menu.id).toBe('payments');
    expect(menu.group).toBe('money');
    expect(menu.order).toBe(6);
    expect(menu.icon).toBe('▣');
    expect(menu.dated).toBe(false);
    expect(menu.needs).toBe('all');
    expect(menu.label()).toBe(T.label);
    expect(menu.heading.eyebrow()).toBe(T.eyebrow);
    expect(menu.heading.title()).toBe(T.title);
    expect(menu.heading.subtitle()).toBe(T.subtitle);
    expect(typeof menu.Component).toBe('function');
  });

  it("reaches the page through menus.ts's glob, with no shell file edited", () => {
    const list = listModules['../menus.ts'];
    expect(list, 'web/src/screens/admin/menus.ts exists').toBeDefined();
    const found = list!.menus.filter((m) => m.id === 'payments');
    expect(found).toHaveLength(1);
    expect(found[0]).toBe(theMenu());
  });

  it('follows setLang: the label and the heading switch to the en table', () => {
    const menu = theMenu();
    const { th, en } = table();
    const keyOf = (text: string): string => {
      const found = Object.entries(th).find(([, v]) => v === text);
      expect(found, `${text} is a value of the menu's th table`).toBeDefined();
      return found![0];
    };
    setLang('en');
    expect(menu.label()).toBe(en[keyOf(T.label)]);
    expect(menu.heading.eyebrow()).toBe(en[keyOf(T.eyebrow)]);
    expect(menu.heading.title()).toBe(en[keyOf(T.title)]);
    expect(menu.heading.subtitle()).toBe(en[keyOf(T.subtitle)]);
  });
});

// ── reading (design §12.6) ───────────────────────────────────────────────────

describe('V2-payments: when the menu reads (#470 §7; design §12.6)', () => {
  it('reads nothing while hidden, then the branch’s settings once when shown, without a cursor', async () => {
    const m = await mount({ shown: false });
    expect(server.readsOf('admin_payments'), 'no read while hidden').toEqual([]);
    m.setShown(true);
    await settle();
    expect(server.readsOf('admin_payments')).toEqual([{ view: 'admin_payments', branch: 'MAIN', since: null }]);
    expect(server.others, 'no request but the views and /api/payment-methods').toEqual([]);
    expect(server.posts).toEqual([]);
    expect(cardIds()).toEqual(CHANNELS.map((c) => c.id));
  });

  it('re-reads on the page’s refresh with the version it holds, and a changed answer shows', async () => {
    await mount();
    const mark = server.reads.length;
    requestRefresh();
    await settle();
    expect(readsSince(mark, 'admin_payments')).toEqual([{ view: 'admin_payments', branch: 'MAIN', since: '41' }]);
    expect(control(card('Cash'), T.account).value, 'changed: false keeps what is shown').toBe('Counter 1');

    // another tab saved: the branch's version moved
    server.settings.MAIN!.version = 42;
    server.settings.MAIN!.methods = with_(mainRows(), 'Cash', { account: 'Saved elsewhere' });
    const mark2 = server.reads.length;
    requestRefresh();
    await settle();
    expect(readsSince(mark2, 'admin_payments')).toEqual([{ view: 'admin_payments', branch: 'MAIN', since: '41' }]);
    expect(control(card('Cash'), T.account).value).toBe('Saved elsewhere');
  });

  it('on a branch switch reads the new branch without the old branch’s cursor, and shows it', async () => {
    const m = await mount();
    const mark = server.reads.length;
    m.setBranch('A1');
    await settle();
    const reads = readsSince(mark, 'admin_payments');
    expect(reads.length).toBeGreaterThanOrEqual(1);
    expect(reads[0]).toEqual({ view: 'admin_payments', branch: 'A1', since: null });
    expect(words()).toContain(`${T.headingOf} Alpha Desk`);
    expect(words()).toContain(T.neverEdited);
    expect(control(card('Cash'), T.account).value).toBe('');
  });

  it('never re-reads on a timer (design §12.6: no admin menu polls)', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval', 'Date'] });
    await mount();
    const mark = server.reads.length;
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(readsSince(mark, 'admin_payments')).toEqual([]);
  });

  it("shows v2's loading line while the read is on its way, then the error and v2's retry when it is refused (:13)", async () => {
    server.holdRead = true;
    server.viewRefusal = { status: 500, code: 'serverError', message: 'server says no' };
    await mount();
    expect(words()).toContain(T.loading);
    server.release();
    await settle();
    expect(words()).toContain(errorText('serverError', 'server says no'));
    expect(cardIds(), 'no card without settings').toEqual([]);
    server.holdRead = false;
    server.viewRefusal = null;
    const mark = server.reads.length;
    button(page(), T.retry).click();
    await settle();
    expect(readsSince(mark, 'admin_payments').length).toBe(1);
    expect(cardIds()).toEqual(CHANNELS.map((c) => c.id));
  });
});

// ── Y1: intro, count pill, meta line (:15) ───────────────────────────────────

describe("V2-payments Y1: v2's intro, count and meta line", () => {
  it('shows the eyebrow, the branch’s heading, the sentence, the count of channels on, who saved and when, the reload button and the footnote', async () => {
    await mount();
    const w = words();
    expect(w).toContain(T.introEyebrow);
    expect(Array.from(page().querySelectorAll('h2')).map((h) => norm(h.textContent))).toContain(`${T.headingOf} Main Desk`);
    expect(w).toContain(T.intro);
    expect(w).toContain('เปิด 4 / 5 ช่องทาง');
    expect(w).toContain(`แก้ไข ${dateText(EDITED_AT, 'th-TH')} โดย somchai`);
    expect(w).not.toContain(T.neverEdited);
    expect(w).toContain(T.footnote);
    expect(w).not.toContain(T.readOnly);
    expect(isDisabled(button(page(), T.reload))).toBe(false);
  });

  it("says the branch uses the system's defaults when it was never edited (§9 `updatedAt` null)", async () => {
    await mount({ branch: 'A1' });
    expect(words()).toContain(T.neverEdited);
    expect(words()).toContain('เปิด 5 / 5 ช่องทาง');
    expect(words()).not.toContain('แก้ไข ');
  });
});

// ── Y2: a card per channel (:7, :15-16) ──────────────────────────────────────

describe("V2-payments Y2: v2's card per channel", () => {
  it("shows the five channels in the server's order, each with v2's name, second line and evidence note", async () => {
    await mount();
    expect(cardIds()).toEqual(CHANNELS.map((c) => c.id));
    for (const c of CHANNELS) {
      const el = card(c.id);
      const text = norm(el.textContent);
      expect(Array.from(el.querySelectorAll('h3')).map((h) => norm(h.textContent)), c.id).toEqual([c.th]);
      expect(text, c.id).toContain(c.line);
      expect(text, c.id).toContain(c.id === 'Cash' ? T.evidenceCash : T.evidenceOther);
      expect(text, c.id).toContain(T.sellNote);
    }
  });

  it("fills each card from the server's rows: on/off, both directions, the account and the instruction", async () => {
    await mount();
    for (const r of mainRows()) {
      const el = card(r.id);
      expect((control(el, T.enabled) as HTMLInputElement).checked, `${r.id}.enabled`).toBe(r.enabled);
      expect((control(el, T.sell) as HTMLInputElement).checked, `${r.id}.sell`).toBe(r.sell);
      const buy = controls(el, T.buy);
      if (BUY_CHANNELS.includes(r.id)) {
        expect(buy.length, `${r.id} offers BUY`).toBe(1);
        expect((buy[0] as HTMLInputElement).checked, `${r.id}.buy`).toBe(r.buy);
      }
      expect(control(el, T.account).value, `${r.id}.account`).toBe(r.account);
      expect(control<HTMLTextAreaElement>(el, T.instruction).value, `${r.id}.instruction`).toBe(r.instruction);
    }
  });

  it('limits the account to 100 and the instruction to 400 characters, with v2’s placeholders and units', async () => {
    await mount();
    for (const c of CHANNELS) {
      const el = card(c.id);
      const account = control(el, T.account);
      const instruction = control(el, T.instruction);
      expect(account.maxLength, `${c.id} account`).toBe(100);
      expect(instruction.maxLength, `${c.id} instruction`).toBe(400);
      expect(account.placeholder, c.id).toBe(c.id === 'Cash' ? T.accountCash : T.accountOther);
      expect(instruction.placeholder, c.id).toBe(T.instructionPlaceholder);
      const text = norm(el.textContent);
      expect(text, c.id).toContain(T.accountUnit);
      expect(text, c.id).toContain(T.instructionUnit);
    }
  });

  it('offers BUY only on Cash and Bank Transfer; the other three say they only receive (§9)', async () => {
    await mount();
    for (const c of CHANNELS) {
      const el = card(c.id);
      const text = norm(el.textContent);
      const enabledBuy = controls(el, T.buy).filter((b) => !isDisabled(b));
      if (BUY_CHANNELS.includes(c.id)) {
        expect(enabledBuy.length, `${c.id}: BUY offered`).toBe(1);
        expect(text, c.id).toContain(T.buyNote);
        expect(text, c.id).not.toContain(T.receiveOnly);
      } else {
        expect(enabledBuy, `${c.id}: no BUY to tick`).toEqual([]);
        expect(text, c.id).toContain(T.receiveOnly);
      }
    }
  });
});

// ── Y3: dirty, reload, discard, save (:17-25, :32-33) ────────────────────────

describe("V2-payments Y3: v2's save bar", () => {
  it('starts clean: the clean line, discard disabled, save enabled, the note', async () => {
    await mount();
    expect(words()).toContain(T.clean);
    expect(words()).not.toContain(T.dirty);
    expect(words()).toContain(T.saveNote);
    expect(isDisabled(button(page(), T.discard))).toBe(true);
    expect(isDisabled(button(page(), T.save))).toBe(false);
  });

  it('turns dirty on the first edit and enables discard', async () => {
    await mount();
    type(control(card('Alipay'), T.account), 'Alipay till 2');
    await settle();
    expect(words()).toContain(T.dirty);
    expect(words()).not.toContain(T.clean);
    expect(isDisabled(button(page(), T.discard))).toBe(false);
  });

  it('turns dirty on a ticked box too', async () => {
    await mount();
    untick(control(card('PromptPay'), T.enabled));
    await settle();
    expect(words()).toContain(T.dirty);
  });

  it("discard puts back the server's rows without a request", async () => {
    await mount();
    type(control(card('Cash'), T.account), 'Typed');
    untick(control(card('Cash'), T.sell));
    await settle();
    const mark = server.reads.length;
    button(page(), T.discard).click();
    await settle();
    expect(control(card('Cash'), T.account).value).toBe('Counter 1');
    expect((control(card('Cash'), T.sell) as HTMLInputElement).checked).toBe(true);
    expect(words()).toContain(T.clean);
    expect(isDisabled(button(page(), T.discard))).toBe(true);
    expect(server.posts).toEqual([]);
    expect(readsSince(mark, 'admin_payments')).toEqual([]);
  });

  it("saves exactly v2's payload: the branch, the setting's own version, five rows in order with trimmed texts", async () => {
    await mount();
    type(control(card('Cash'), T.account), '  New counter  ');
    type(control<HTMLTextAreaElement>(card('Alipay'), T.instruction), '\n Scan here \n');
    tick(control(card('Credit Card'), T.enabled));
    untick(control(card('Bank Transfer'), T.buy));
    tick(control(card('Bank Transfer'), T.sell));
    await settle();
    button(page(), T.save).click();
    await settle();
    let want = with_(mainRows(), 'Cash', { account: 'New counter' });
    want = with_(want, 'Alipay', { instruction: 'Scan here' });
    want = with_(want, 'Credit Card', { enabled: true });
    want = with_(want, 'Bank Transfer', { buy: false, sell: true });
    expect(onlySave()).toEqual({ action: 'save', payload: { branchId: 'MAIN', version: 3, methods: want } });
    expect(server.others).toEqual([]);
  });

  it("while saving shows v2's line and locks the form, then shows success, re-reads the view and is clean", async () => {
    await mount();
    type(control(card('Cash'), T.account), 'Counter 9');
    await settle();
    server.holdSave = true;
    const mark = server.reads.length;
    button(page(), T.save).click();
    await settle();
    expect(feedback()).toContain(T.saving);
    expect(isDisabled(button(page(), T.save)), 'save locked').toBe(true);
    expect(isDisabled(button(page(), T.discard)), 'discard locked').toBe(true);
    expect(isDisabled(button(page(), T.reload)), 'reload locked').toBe(true);
    expect(isDisabled(control(card('Cash'), T.account)), 'the form is locked').toBe(true);
    // a second press, or a submit by any other way, while saving sends nothing more (:21)
    button(page(), T.save).click();
    for (const form of Array.from(page().querySelectorAll('form'))) {
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    }
    await settle();
    expect(server.posts).toHaveLength(1);

    server.release();
    await settle();
    expect(feedback()).toContain(T.saved);
    expect(readsSince(mark, 'admin_payments').length, 'after save the view re-reads').toBeGreaterThanOrEqual(1);
    expect(control(card('Cash'), T.account).value).toBe('Counter 9');
    expect(isDisabled(control(card('Cash'), T.account))).toBe(false);
    expect(words()).toContain(T.clean);
    expect(words()).toContain(`แก้ไข ${dateText(SAVED_AT, 'th-TH')} โดย owner`);
    expect(isDisabled(button(page(), T.discard))).toBe(true);
  });

  it('a second save after the first sends the new setting version', async () => {
    await mount();
    type(control(card('Cash'), T.account), 'First');
    await settle();
    button(page(), T.save).click();
    await settle();
    type(control(card('Cash'), T.account), 'Second');
    await settle();
    button(page(), T.save).click();
    await settle();
    const versions = server.posts.map((p) => (p.body as { payload: { version: number } }).payload.version);
    expect(versions).toEqual([3, 4]);
  });

  it("sends a channel on with neither direction and shows the server's sentence: no client rule of its own (§9)", async () => {
    server.saveRefusal = { status: 400, sentence: NO_DIRECTION };
    await mount();
    untick(control(card('Cash'), T.buy));
    untick(control(card('Cash'), T.sell));
    await settle();
    button(page(), T.save).click();
    await settle();
    const sent = onlySave().payload.methods.find((r) => r.id === 'Cash');
    expect(sent).toEqual({ id: 'Cash', enabled: true, buy: false, sell: false, account: 'Counter 1', instruction: 'Pay at counter 1' });
    expect(feedback()).toContain(errorText('refused', NO_DIRECTION));
    expect(feedback()).toContain(NO_DIRECTION);
  });

  it("shows the owner-only refusal (403) when the server refuses the save (§8)", async () => {
    server.saveRefusal = { status: 403, sentence: OWNER_ONLY };
    await mount();
    type(control(card('Cash'), T.account), 'Mine');
    await settle();
    button(page(), T.save).click();
    await settle();
    expect(feedback()).toContain(OWNER_ONLY);
  });

  it('on a 409 keeps the user’s edits and offers reload, which asks v2’s confirm and then reads the latest (§8)', async () => {
    server.saveRefusal = { status: 409, sentence: STALE };
    await mount();
    type(control(card('Cash'), T.account), 'Mine');
    await settle();
    button(page(), T.save).click();
    await settle();
    expect(feedback()).toContain(STALE);
    expect(control(card('Cash'), T.account).value, 'the edit is kept').toBe('Mine');
    expect(isDisabled(control(card('Cash'), T.account))).toBe(false);
    expect(words()).toContain(T.dirty);
    expect(isDisabled(button(page(), T.save))).toBe(false);
    expect(isDisabled(button(page(), T.discard))).toBe(false);
    expect(isDisabled(button(page(), T.reload))).toBe(false);

    // someone else's save is what made it stale
    server.saveRefusal = null;
    server.settings.MAIN!.version = 50;
    server.settings.MAIN!.settingsVersion = 7;
    server.settings.MAIN!.methods = with_(mainRows(), 'Cash', { account: 'Theirs' });
    const confirm = vi.fn(() => true);
    vi.stubGlobal('confirm', confirm);
    const mark = server.reads.length;
    button(page(), T.reload).click();
    await settle();
    expect(confirm).toHaveBeenCalledWith(T.confirmReload);
    expect(readsSince(mark, 'admin_payments').length).toBe(1);
    expect(control(card('Cash'), T.account).value).toBe('Theirs');
    expect(words()).toContain(T.clean);

    // and the next save carries the new setting version
    type(control(card('Cash'), T.account), 'Mine again');
    await settle();
    button(page(), T.save).click();
    await settle();
    expect((server.posts.at(-1)!.body as { payload: { version: number } }).payload.version).toBe(7);
  });

  it('reload with unsaved edits asks first; cancelled, nothing is read and the edits stay (§9)', async () => {
    await mount();
    type(control(card('Cash'), T.account), 'Unsaved');
    await settle();
    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);
    const mark = server.reads.length;
    button(page(), T.reload).click();
    await settle();
    expect(confirm).toHaveBeenCalledWith(T.confirmReload);
    expect(readsSince(mark, 'admin_payments')).toEqual([]);
    expect(control(card('Cash'), T.account).value).toBe('Unsaved');
    expect(words()).toContain(T.dirty);
  });

  it('reload without edits reads at once, asking nothing', async () => {
    await mount();
    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);
    server.settings.MAIN!.version = 42;
    server.settings.MAIN!.methods = with_(mainRows(), 'Alipay', { instruction: 'Newer' });
    const mark = server.reads.length;
    button(page(), T.reload).click();
    await settle();
    expect(confirm).not.toHaveBeenCalled();
    expect(readsSince(mark, 'admin_payments').length).toBe(1);
    expect(control<HTMLTextAreaElement>(card('Alipay'), T.instruction).value).toBe('Newer');
  });

  it('a branch switch with unsaved edits asks v2’s confirm; accepted, the new branch shows clean (§9)', async () => {
    const m = await mount();
    type(control(card('Cash'), T.account), 'Unsaved');
    await settle();
    const confirm = vi.fn(() => true);
    vi.stubGlobal('confirm', confirm);
    m.setBranch('A1');
    await settle();
    expect(confirm).toHaveBeenCalledWith(T.confirmBranch);
    expect(words()).toContain(`${T.headingOf} Alpha Desk`);
    expect(control(card('Cash'), T.account).value).toBe('');
    expect(words()).toContain(T.clean);
  });

  it('a branch switch without edits asks nothing', async () => {
    const m = await mount();
    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);
    m.setBranch('A1');
    await settle();
    expect(confirm).not.toHaveBeenCalled();
    expect(words()).toContain(`${T.headingOf} Alpha Desk`);
  });

  it('the page’s refresh never throws away unsaved edits without asking', async () => {
    await mount();
    type(control(card('Cash'), T.account), 'Unsaved');
    await settle();
    vi.stubGlobal('confirm', vi.fn(() => false));
    server.settings.MAIN!.version = 42;
    server.settings.MAIN!.methods = with_(mainRows(), 'Cash', { account: 'Saved elsewhere' });
    requestRefresh();
    await settle();
    expect(control(card('Cash'), T.account).value).toBe('Unsaved');
    expect(words()).toContain(T.dirty);
  });

  it('guards leaving the page while there are unsaved edits (:33)', async () => {
    await mount();
    const leave = (): boolean => {
      const e = new Event('beforeunload', { cancelable: true });
      const prevent = vi.spyOn(e, 'preventDefault');
      window.dispatchEvent(e);
      return prevent.mock.calls.length > 0;
    };
    expect(leave(), 'clean: leaving is not held').toBe(false);
    type(control(card('Cash'), T.account), 'Unsaved');
    await settle();
    expect(leave(), 'dirty: leaving is held').toBe(true);
    button(page(), T.discard).click();
    await settle();
    expect(leave(), 'discarded: not held').toBe(false);
  });
});

// ── canEdit false (§9, §8) ───────────────────────────────────────────────────

describe('V2-payments: an employee reads their branch and cannot change it', () => {
  it('shows v2’s read-only line, locks every control, offers no save and sends nothing', async () => {
    const s = server.settings.MAIN!;
    s.canEdit = false;
    s.history = [];
    await mount();
    expect(words()).toContain(T.readOnly);
    for (const c of CHANNELS) {
      for (const el of Array.from(card(c.id).querySelectorAll<Control>('input, textarea'))) {
        expect(isDisabled(el), `${c.id} ${el.name}`).toBe(true);
      }
    }
    const saves = buttons(page(), T.save);
    expect(saves.every((b) => isDisabled(b)), 'no save button, or a disabled one').toBe(true);
    for (const form of Array.from(page().querySelectorAll('form'))) {
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    }
    await settle();
    expect(server.posts).toEqual([]);
    expect(norm(historyDetails().querySelector('summary')?.textContent)).toBe('ประวัติการตั้งค่า 0 รายการล่าสุด');
    expect(historyRows()).toEqual([[T.noHistory]]);
  });
});

// ── Y4: history (:11, :15) ───────────────────────────────────────────────────

describe("V2-payments Y4: v2's history of the settings", () => {
  it("lists the entries under v2's summary and headers: date, who, which channels changed, version", async () => {
    await mount();
    const d = historyDetails();
    expect(norm(d.querySelector('summary')?.textContent)).toBe('ประวัติการตั้งค่า 3 รายการล่าสุด');
    expect(Array.from(d.querySelectorAll('thead th')).map((th) => norm(th.textContent))).toEqual(T.historyHeaders);
    expect(historyRows()).toEqual([
      [dateText(EDITED_AT, 'th-TH'), 'somchai', T.unchanged, '3'],
      [dateText('2026-10-06T02:00:00.000Z', 'th-TH'), 'owner', 'เงินสด, พร้อมเพย์', '2'],
      [dateText('2026-10-05T02:00:00.000Z', 'th-TH'), 'owner', 'เงินสด, บัตรเครดิต, Alipay, พร้อมเพย์, โอนธนาคาร', '1'],
    ]);
  });

  it("shows v2's empty row when there is no history", async () => {
    await mount({ branch: 'A1' });
    expect(norm(historyDetails().querySelector('summary')?.textContent)).toBe('ประวัติการตั้งค่า 0 รายการล่าสุด');
    expect(historyRows()).toEqual([[T.noHistory]]);
  });

  it('shows the new entry after a save', async () => {
    await mount();
    untick(control(card('Alipay'), T.enabled));
    await settle();
    button(page(), T.save).click();
    await settle();
    expect(historyRows()[0]).toEqual([dateText(SAVED_AT, 'th-TH'), 'owner', 'Alipay', '4']);
  });
});

// ── Y5: dates per language (W10 point 5) ─────────────────────────────────────

describe('V2-payments Y5: dates follow the language, in Asia/Bangkok', () => {
  it('shows the meta line and the history dates in en-GB once the language is English', async () => {
    setLang('en');
    await mount();
    const w = words();
    expect(w).toContain(dateText(EDITED_AT, 'en-GB'));
    expect(w).not.toContain(dateText(EDITED_AT, 'th-TH'));
    expect(historyRows()[1]![0]).toBe(dateText('2026-10-06T02:00:00.000Z', 'en-GB'));
  });

  it('follows a language switch while shown', async () => {
    await mount();
    expect(words()).toContain(dateText(EDITED_AT, 'th-TH'));
    setLang('en');
    await settle();
    expect(words()).toContain(dateText(EDITED_AT, 'en-GB'));
    expect(historyRows()[0]![0]).toBe(dateText(EDITED_AT, 'en-GB'));
  });
});

// ── W10: every text through this folder's table ──────────────────────────────

describe("V2-payments W10: the menu's texts come from payments/i18n.ts", () => {
  it('holds every v2 text above, verbatim, in `th`, with the same keys in `en` and no Thai there', () => {
    const { th, en } = table();
    const values = new Set(Object.values(th));
    const expected: string[] = [
      ...Object.values(T).flat(),
      ...CHANNELS.map((c) => c.th).filter((s) => THAI.test(s)),
    ];
    const bare = (v: string): string => v.replace(/\{\w+\}/g, '').trim();
    for (const s of expected) {
      expect([...values].some((v) => v === s || bare(v) === s), `th holds "${s}"`).toBe(true);
    }
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(en)) {
      expect(THAI.test(v), `en.${k} has no Thai: ${v}`).toBe(false);
      expect(v.trim(), `en.${k} is not empty`).not.toBe('');
    }
  });

  it('shows no Thai anywhere in the menu once the language is English, labels, placeholders and history included', async () => {
    setLang('en');
    await mount();
    expect(THAI.test(page().textContent ?? ''), words()).toBe(false);
    const attrs = Array.from(page().querySelectorAll('[aria-label], [title], [placeholder]')).map(
      (e) => `${e.getAttribute('aria-label') ?? ''} ${e.getAttribute('title') ?? ''} ${e.getAttribute('placeholder') ?? ''}`,
    );
    expect(attrs.filter((l) => THAI.test(l))).toEqual([]);

    type(control(card('Cash'), controlLabelEn('account')), 'Edited');
    await settle();
    expect(THAI.test(page().textContent ?? ''), words()).toBe(false);
    const confirm = vi.fn((_message?: string) => false);
    vi.stubGlobal('confirm', confirm);
    reloadButtonEn().click();
    await settle();
    expect(confirm).toHaveBeenCalledTimes(1);
    expect(THAI.test(String(confirm.mock.calls[0]?.[0])), 'the confirm is English').toBe(false);
  });
});

/** The en label of a table key whose th value is `T[key]`. */
function controlLabelEn(key: keyof typeof T): string {
  const { th, en } = table();
  const found = Object.entries(th).find(([, v]) => v === T[key]);
  expect(found, `${String(key)} is in the th table`).toBeDefined();
  return en[found![0]]!;
}

function reloadButtonEn(): HTMLButtonElement {
  return button(page(), controlLabelEn('reload'));
}

// ── done criteria 3 and 4, in the source ─────────────────────────────────────

describe('V2-payments: the folder makes no request and no money arithmetic of its own (done criteria 3, 4)', () => {
  it('has menu.ts, Payments.tsx and i18n.ts, none calling fetch( or doing money arithmetic', () => {
    const files = Object.keys(sources).filter((f) => !f.includes('.test.'));
    for (const want of ['./menu.ts', './Payments.tsx', './i18n.ts']) expect(files, want).toContain(want);
    for (const f of files) {
      const text = sources[f]!;
      expect(/fetch\(/.test(text), `${f}: every call goes through ApiClient`).toBe(false);
      expect(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/.test(text), `${f}: no money arithmetic`).toBe(false);
    }
  });
});
