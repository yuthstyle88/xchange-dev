// V2-employees (#462): the admin page's employees menu — the account list, its create / reset /
// disable writes, the branch's custody workflow (state, entity, enable) and the link to the staff
// page (ticket §7, §9, §10 rows E1–E3; design §12.3, §12.6, §12.7 in PR 474). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests in
// a file of your own.
// Amended by #1157 (claude/08f7f46f; owner: the shop owner creates staff accounts, no invite): the create
// dialog posts `userCreate` (`POST /api/users`) with a required phone and the org's roles, instead of v2's
// `{action: "create"}`; the other writes still go to `/api/employees`, unchanged.
// Amended by CA-10-FE (claude/c4d72c, the ticket author; design `cash-shift-transfer.md` §6 Q7, §9 CA-10-FE):
// CA-10 retires `/api/employees`' `enable` and `entity` actions, so the custody workflow (its state line, the
// Enable control and dialog, the entity form) leaves the screen; a stored entity may still be shown, read-only
// (Q5). E2 is restated below; the refusal loop keeps create and reset; the state-line checks are gone.
//
// Source of truth (W9): xchange-v2-source@4d0bdbb `src/online/public/employees.js:2-12`, the
// section `admin-html:19` (decoded admin page, ticket §2) and the served regrouping
// `src/devices-public/admin-workspace.js:139-142` (the `/staff` link; the workflow folded into a
// `<details>`). Every text is v2's Thai verbatim (W6), read through this folder's own table
// `i18n.ts` (W10).
//
// The menu is mounted as the shell mounts it (`menu.Component` with `branch`, `period`, `shown`),
// beside V2-parts' one `<Dialog/>` and one `<Toast/>`, which the shell renders once per page. The
// menu gets no `ApiClient` from the shell (`AdminMenuProps`), so it talks through an `ApiClient`
// of its own, whose requests reach the global `fetch`: the fake below is the server. It answers
// - `GET /api/v3/views/admin_employees` — `{changed: true, data: {employees}}` always
//   (`VERSIONED = false`, design §12.7), or a refusal when the caller is staff;
// - `GET /api/v3/views/admin_operations?branch=B[&since=v]` — `{changed: false}` when `since` is B's
//   version, otherwise `{changed: true, data: {enabled, entity}}`;
// - `GET /api/v3/views/admin_branches` — the shell's branch list `{branches: [{id, name}]}`, which
//   gives the table its branch names and the create dialog its branch choices (v2 reads both from
//   the snapshot's `branches`, `employees.js:6-7`);
// - `POST /api/employees` `{action, payload}` — v2's door, unchanged (ticket §8): `{saved: true}`,
//   or v2's refusal `{"error": "<sentence>"}`; `entity` and `enable` move the branch's version;
// - `POST /api/users` (#1157 `userCreate`) — recorded as the action `create` with the body as payload:
//   `{userId, status, version}`, or the refusal `{"error": {"code", "message"}}`.
// Every other request is recorded and fails the test. AppShell's page-wide reads
// (`PAGE_WIDE`, `shared/testing/page-wide.ts`) are answered and never counted.
//
// `menu.ts`, the table and the menu list are read through eager globs, so on a branch without them
// each test fails on an assertion that the file exists, not on the import.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: 'all'` pin → `needs: 'all'` (#1030 §3.10: `MenuDef.roles` becomes `needs`). Nothing else changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { closeDialog, Dialog } from '../parts/Dialog';
import { requestRefresh } from '../parts/refresh';
import { Toast } from '../parts/toast';
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
  expect(m, 'web/src/screens/admin/employees/menu.ts exists').toBeDefined();
  expect(m!.menu, 'menu.ts exports `menu`').toBeDefined();
  return m!.menu;
}

function table(): Table {
  const m = tableModules['./i18n.ts'];
  expect(m, 'web/src/screens/admin/employees/i18n.ts exists').toBeDefined();
  expect(m!.th, 'i18n.ts exports `th`').toBeDefined();
  expect(m!.en, 'i18n.ts exports `en`').toBeDefined();
  return m!;
}

const THAI = /[฀-๿]/;

// ── v2's texts (employees.js, admin-html:19, admin-workspace.js:139-142) ─────

const T = {
  heading: 'บัญชีพนักงาน',
  intro: 'พนักงานแต่ละคนใช้รหัสผ่านของตนเอง ผู้ส่ง ผู้รับ และผู้ตรวจต้องเป็นคนละคนตามบทบาท',
  add: 'สร้างบัญชีพนักงาน',
  refresh: 'รีเฟรช',
  staffLink: 'เปิดหน้าพนักงาน ↗',
  headers: ['รหัสบัญชี / ชื่อ', 'บทบาท', 'สาขา', 'สถานะ', ''],
  active: 'ใช้งาน',
  mustChange: 'รอเปลี่ยนรหัสผ่าน',
  disabled: 'ปิดใช้งาน',
  reset: 'รีเซ็ตรหัสผ่าน',
  disable: 'ปิดบัญชี',
  // create (employees.js:7)
  createTitle: 'สร้างบัญชีพนักงาน',
  name: 'ชื่อพนักงาน',
  username: 'รหัสบัญชี (ไม่บังคับ)',
  phone: 'เบอร์โทรศัพท์ (ใช้เข้าสู่ระบบ)',
  email: 'อีเมล (ไม่บังคับ)',
  role: 'บทบาท',
  branch: 'สาขา',
  password: 'รหัสผ่านชั่วคราว',
  handOver: 'ส่งมอบรหัสผ่านให้พนักงานโดยตรง',
  createSubmit: 'สร้างบัญชี',
  // reset (employees.js:10)
  resetTitle: 'รีเซ็ตรหัสผ่าน',
  newPassword: 'รหัสผ่านชั่วคราวใหม่',
  sessionsEnd: 'เซสชันเดิมทั้งหมดจะถูกยกเลิก',
  resetSubmit: 'ยืนยันรีเซ็ต',
  // disable (employees.js:10)
  disableConfirm: 'ปิดบัญชีนี้และยกเลิกเซสชันทั้งหมด?',
} as const;

/** The create dialog's role choices, in v2's order (employees.js:7). */
const ROLES = ['TELLER', 'BRANCH_MANAGER', 'RATE_MANAGER', 'FINANCE', 'VERIFICATION_REVIEWER', 'ADMIN'];

// ── the fake server ──────────────────────────────────────────────────────────

interface Employee {
  id: string;
  username: string;
  name: string;
  role: string;
  branchId: string;
  active: boolean;
  mustChange: boolean;
}

interface Ops {
  version: number;
  entity: { company: string; site: string; confirmedBy: string; at: string } | null;
}

interface Read {
  view: string;
  branch: string | null;
  since: string | null;
}

interface Post {
  method: string;
  marker: string | null;
  action: string;
  payload: Record<string, unknown>;
}


function people(): Employee[] {
  return [
    { id: 'e-1', username: 'front01', name: 'Somchai Front', role: 'FRONT', branchId: 'MAIN', active: true, mustChange: false },
    { id: 'e-2', username: 'fin01', name: 'Fah Finance', role: 'FINANCE', branchId: 'A1', active: true, mustChange: true },
    { id: 'e-3', username: 'old01', name: 'Old Backend', role: 'BACKEND', branchId: 'GONE', active: false, mustChange: false },
  ];
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  employees: Employee[] = people();
  branches = [
    { id: 'MAIN', name: 'Main Desk' },
    { id: 'A1', name: 'Alpha Desk' },
    { id: 'B2', name: 'Bravo Desk' },
  ];
  ops: Record<string, Ops> = {
    MAIN: { version: 7, entity: null },
    A1: { version: 7, entity: { company: 'Alpha Co.', site: 'Alpha Site', confirmedBy: 'owner', at: '2026-10-01T00:00:00.000Z' } },
    B2: { version: 4, entity: null },
  };
  /** When set, `admin_employees` answers this refusal (a staff caller, ticket §9 row 1). */
  employeesRefusal: { status: number; code: string; message: string } | null = null;
  /** action → v2's refusal sentence `/api/employees` answers instead of saving. */
  refuse: Record<string, string> = {};
  reads: Read[] = [];
  posts: Post[] = [];
  others: string[] = [];
  /** Called when a POST arrives, before it is answered. */
  onPost: ((action: string) => void) | undefined;

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
    if (method === 'GET' && view === 'admin_employees') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      if (this.employeesRefusal) {
        const r = this.employeesRefusal;
        return json({ error: { code: r.code, message: r.message } }, r.status);
      }
      return json({ version: 3, changed: true, data: { employees: this.employees.map((e) => ({ ...e })) } });
    }
    if (method === 'GET' && view === 'admin_operations') {
      const branch = url.searchParams.get('branch');
      const since = url.searchParams.get('since');
      this.reads.push({ view, branch, since });
      const ops = this.ops[branch ?? 'MAIN'];
      if (!ops) return json({ error: { code: 'noSuchBranch', field: 'branch', message: 'ไม่พบสาขาที่เลือก กรุณารีเฟรช' } }, 400);
      if (since !== null && Number(since) === ops.version) return json({ version: ops.version, changed: false });
      return json({ version: ops.version, changed: true, data: { entity: ops.entity && { ...ops.entity } } });
    }
    if (method === 'GET' && view === 'admin_branches') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      return json({ version: 1, changed: true, data: { branches: this.branches.map((b) => ({ ...b })) } });
    }
    if (method === 'POST' && url.pathname === '/api/employees') {
      const headers = new Headers(init?.headers);
      const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: Record<string, unknown> };
      this.posts.push({ method, marker: headers.get('x-xchange-request'), action: body.action, payload: body.payload });
      this.onPost?.(body.action);
      const sentence = this.refuse[body.action];
      if (sentence !== undefined) return json({ error: sentence }, 400);
      return json(this.apply(body.action, body.payload));
    }
    if (method === 'POST' && url.pathname === '/api/users') {
      const headers = new Headers(init?.headers);
      const body = JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>;
      this.posts.push({ method, marker: headers.get('x-xchange-request'), action: 'create', payload: body });
      this.onPost?.('create');
      const sentence = this.refuse.create;
      if (sentence !== undefined) return json({ error: { code: 'validationFailed', message: sentence } }, 400);
      const roles = body.roles as { role: string; branch: string }[];
      this.employees.push({ id: `e-${this.employees.length + 1}`, username: String(body.username), name: String(body.name), role: roles[0]!.role, branchId: roles[0]!.branch, active: true, mustChange: false });
      return json({ userId: `u-${this.employees.length}`, status: 'ACTIVE', version: 2 });
    }
    this.others.push(`${method} ${raw}`);
    return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  };

  private apply(action: string, payload: Record<string, unknown>): unknown {
    const p = payload as Record<string, string>;
    if (action === 'create') {
      this.employees.push({ id: `e-${this.employees.length + 1}`, username: p.username!, name: p.name!, role: p.role!, branchId: p.branchId!, active: true, mustChange: true });
      return { saved: true };
    }
    if (action === 'reset') {
      const e = this.employees.find((x) => x.id === p.id);
      if (e) e.mustChange = true;
      return { saved: true };
    }
    if (action === 'disable') {
      const e = this.employees.find((x) => x.id === p.id);
      if (e) e.active = false;
      return { saved: true };
    }
    const ops = this.ops[p.branchId ?? 'MAIN']!;
    ops.version += 1;
    if (action === 'entity') ops.entity = { company: p.company!, site: p.site!, confirmedBy: 'owner', at: '2026-10-08T00:00:00.000Z' };
    return { ok: true };
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

/** The menu as the shell hosts it, beside the page's one dialog and one toast. */
async function mount(opts: { branch?: string; shown?: boolean } = {}): Promise<Mounted> {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [shown, setShown] = createSignal(opts.shown ?? true);
  const period: Accessor<{ from: string; to: string }> = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  const Component = menu.Component;
  dispose = render(
    (): JSX.Element => (
      <>
        <Component branch={branch} period={period} shown={shown} />
        <Dialog />
        <Toast />
      </>
    ),
    container,
  );
  await settle();
  return { setBranch, setShown };
}

// ── finding things the way a person does ─────────────────────────────────────

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function openDialogEl(): HTMLDialogElement | null {
  return document.querySelector<HTMLDialogElement>('dialog[open]');
}

function theDialog(): HTMLDialogElement {
  const d = openDialogEl();
  expect(d, 'a dialog is open').not.toBeNull();
  return d!;
}

function dialogTitle(): string {
  return norm(theDialog().querySelector('h2')?.textContent);
}

/** The menu's own part of the page (not the dialog or the toast). */
function page(): HTMLElement {
  expect(container, 'the menu is mounted').toBeDefined();
  return container!;
}

function menuArea(): Element {
  const own = Array.from(page().children).filter((c) => c.tagName !== 'DIALOG' && !c.classList.contains('toast'));
  const wrap = document.createElement('div');
  for (const c of own) wrap.appendChild(c.cloneNode(true));
  return wrap;
}

/** The menu's own buttons, in page order (not the dialog's). */
function menuButtons(): HTMLButtonElement[] {
  return Array.from(page().querySelectorAll<HTMLButtonElement>('button')).filter((b) => !b.closest('dialog'));
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

/** The control a label with exactly these words names, inside `root`. */
function field<T extends HTMLElement = HTMLInputElement>(root: ParentNode, words: string): T {
  const labels = Array.from(root.querySelectorAll<HTMLLabelElement>('label')).filter((l) => labelWords(l) === words);
  expect(labels.length, `one label "${words}"`).toBe(1);
  const label = labels[0]!;
  const control =
    (label.htmlFor ? document.getElementById(label.htmlFor) : null) ??
    label.querySelector('input, select, textarea');
  expect(control, `label "${words}" names a control`).not.toBeNull();
  return control as unknown as T;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}


/** The account table: the `<table>` whose header row is v2's. */
function accountTable(): HTMLTableElement {
  const t = Array.from(page().querySelectorAll('table')).find((x) =>
    Array.from(x.querySelectorAll('th')).some((th) => norm(th.textContent) === T.headers[0]),
  );
  expect(t, 'the account table is shown').toBeDefined();
  return t!;
}

function rowOf(username: string): HTMLTableRowElement {
  const rows = Array.from(accountTable().querySelectorAll<HTMLTableRowElement>('tbody tr')).filter(
    (r) => norm(r.cells[0]?.textContent).includes(username),
  );
  expect(rows.length, `one row for ${username}`).toBe(1);
  return rows[0]!;
}


function toastText(): string {
  return norm(document.querySelector('.toast')?.textContent);
}

/** Reads made after `from` posts/reads were recorded. */
function readsSince(mark: number, view: string): Read[] {
  return server.reads.slice(mark).filter((r) => r.view === view);
}

// ── set-up ───────────────────────────────────────────────────────────────────

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  await settle();
  closeDialog();
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

describe('V2-employees: the menu record (#462 §7)', () => {
  it("is v2's `employees` page in group data at order 10, undated, for every role", () => {
    const menu = theMenu();
    expect(menu.id).toBe('employees');
    expect(menu.group).toBe('data');
    expect(menu.order).toBe(10);
    expect(menu.icon).toBe('♙');
    expect(menu.dated).toBe(false);
    expect(menu.needs).toBe('all');
    expect(menu.label()).toBe('พนักงานและสิทธิ์');
    expect(menu.heading.eyebrow()).toBe('TEAM & OPERATIONS');
    expect(menu.heading.title()).toBe('พนักงานและสิทธิ์');
    expect(menu.heading.subtitle()).toBe('บัญชีพนักงานและการตั้งค่าขั้นตอนรับ–ส่งเงิน');
  });

  it("reaches the page through menus.ts's glob, with no shell file edited", () => {
    const list = listModules['../menus.ts'];
    expect(list, 'web/src/screens/admin/menus.ts exists').toBeDefined();
    const found = list!.menus.filter((m) => m.id === 'employees');
    expect(found).toHaveLength(1);
    expect(found[0]).toBe(theMenu());
  });
});

// ── reading (design §12.6, §12.7; ticket §9 row 3) ───────────────────────────

describe('V2-employees: when the menu reads (#462 §7, §9; design §12.6, §12.7)', () => {
  it('reads nothing while hidden, then both views once when shown: the whole list, and the branch’s operations without a cursor', async () => {
    const m = await mount({ shown: false });
    expect(server.readsOf('admin_employees'), 'no read while hidden').toEqual([]);
    expect(server.readsOf('admin_operations'), 'no read while hidden').toEqual([]);
    m.setShown(true);
    await settle();
    expect(server.readsOf('admin_employees')).toHaveLength(1);
    const ops = server.readsOf('admin_operations');
    expect(ops).toHaveLength(1);
    expect(ops[0]).toEqual({ view: 'admin_operations', branch: 'MAIN', since: null });
    expect(server.others, 'no request but the views and /api/employees').toEqual([]);
    expect(norm(rowOf('front01').textContent)).toContain('Somchai Front');
  });

  it('re-reads both on the page’s refresh: the whole list again, the operations with the version it holds', async () => {
    await mount();
    const mark = server.reads.length;
    server.employees.push({ id: 'e-9', username: 'other01', name: 'Made In Another Tab', role: 'RATES', branchId: 'MAIN', active: true, mustChange: true });
    requestRefresh();
    await settle();
    expect(readsSince(mark, 'admin_employees')).toHaveLength(1);
    expect(readsSince(mark, 'admin_operations')).toEqual([{ view: 'admin_operations', branch: 'MAIN', since: '7' }]);
    expect(norm(rowOf('other01').textContent), 'a write by another tab shows (VERSIONED = false)').toContain('Made In Another Tab');
  });

  it("re-reads the list on v2's own `รีเฟรช` button (employees.js:8)", async () => {
    await mount();
    const mark = server.reads.length;
    server.employees[0]!.name = 'Renamed Elsewhere';
    button(page(), T.refresh).click();
    await settle();
    expect(readsSince(mark, 'admin_employees')).toHaveLength(1);
    expect(norm(rowOf('front01').textContent)).toContain('Renamed Elsewhere');
  });

  it('on a branch switch reads the new branch’s operations without the old branch’s cursor', async () => {
    const m = await mount();
    // both branches are at version 7: a cursor carried over would answer `changed: false`
    const mark = server.reads.length;
    m.setBranch('A1');
    await settle();
    const ops = readsSince(mark, 'admin_operations');
    expect(ops.length).toBeGreaterThanOrEqual(1);
    expect(ops[0]).toEqual({ view: 'admin_operations', branch: 'A1', since: null });
  });

  it('never re-reads on a timer (design §12.6: no admin menu polls)', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval', 'Date'] });
    await mount();
    const mark = server.reads.length;
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(readsSince(mark, 'admin_employees')).toEqual([]);
    expect(readsSince(mark, 'admin_operations')).toEqual([]);
  });
});

// ── E1: heading, table (employees.js:2-6, admin-html:19) ─────────────────────

describe("V2-employees E1: v2's account card and table", () => {
  it('shows the heading, the sentence, and the create and refresh buttons', async () => {
    await mount();
    const words = norm(page().textContent);
    expect(Array.from(page().querySelectorAll('h2, h3')).map((h) => norm(h.textContent))).toContain(T.heading);
    expect(words).toContain(T.intro);
    button(page(), T.add);
    button(page(), T.refresh);
  });

  it("lists every account with v2's columns: account / name, role, branch name (else its id), status, and the two actions on active accounts only", async () => {
    await mount();
    const heads = Array.from(accountTable().querySelectorAll('thead th')).map((th) => norm(th.textContent));
    expect(heads).toEqual(T.headers);
    const cells = (u: string) => Array.from(rowOf(u).cells).map((c) => norm(c.textContent));

    const front = cells('front01');
    expect(front[0]).toContain('front01');
    expect(front[0]).toContain('Somchai Front');
    expect(front[1]).toBe('FRONT');
    expect(front[2]).toBe('Main Desk');
    expect(front[3]).toBe(T.active);
    expect(buttons(rowOf('front01'), T.reset)).toHaveLength(1);
    expect(buttons(rowOf('front01'), T.disable)).toHaveLength(1);

    const fin = cells('fin01');
    expect(fin[2]).toBe('Alpha Desk');
    expect(fin[3]).toBe(T.mustChange);

    const old = cells('old01');
    expect(old[2], 'a branch the list does not know is shown by its id').toBe('GONE');
    expect(old[3]).toBe(T.disabled);
    expect(rowOf('old01').querySelectorAll('button'), 'no action on a closed account').toHaveLength(0);
    expect(accountTable().querySelectorAll('tbody tr')).toHaveLength(3);
  });

  it("shows the server's refusal where the list would be when the caller may not list (staff, ticket §9 row 1)", async () => {
    server.employeesRefusal = { status: 403, code: 'ownerOnly', message: 'refused by the server' };
    await mount();
    expect(norm(page().textContent)).toContain(errorText('ownerOnly', 'refused by the server'));
    expect(page().querySelectorAll('tbody tr'), 'no account row').toHaveLength(0);
  });
});

// ── E1: the writes (employees.js:7, :10) ─────────────────────────────────────

/** Both views were read again after `mark`. */
function expectBothReRead(mark: number, what: string): void {
  expect(readsSince(mark, 'admin_employees').length, `${what}: the list is read again`).toBeGreaterThanOrEqual(1);
  expect(readsSince(mark, 'admin_operations').length, `${what}: the operations are read again`).toBeGreaterThanOrEqual(1);
}

describe("V2-employees E1: create, reset and disable through v2's door", () => {
  it("creates an account from v2's dialog and posts exactly the userCreate payload to /api/users (#1157)", async () => {
    await mount({ branch: 'A1' });
    button(page(), T.add).click();
    await settle();
    const d = theDialog();
    expect(dialogTitle()).toBe(T.createTitle);
    expect(norm(d.textContent)).toContain(T.handOver);
    const role = field<HTMLSelectElement>(d, T.role);
    expect(Array.from(role.options).map((o) => o.value)).toEqual(ROLES);
    const branch = field<HTMLSelectElement>(d, T.branch);
    expect(Array.from(branch.options).map((o) => [o.value, norm(o.textContent)])).toEqual([
      ['MAIN', 'Main Desk'],
      ['A1', 'Alpha Desk'],
      ['B2', 'Bravo Desk'],
    ]);
    expect(branch.value, 'the branch picked in the header').toBe('A1');
    const password = field(d, T.password);
    expect(password.type, 'the temporary password is concealed').toBe('password');

    type(field(d, T.name), 'New Person');
    type(field(d, T.phone), '0812345678');
    type(field(d, T.username), 'new01');
    choose(role, 'TELLER');
    choose(branch, 'MAIN');
    type(password, 'temporary-pass-01');
    const mark = server.reads.length;
    button(d, T.createSubmit).click();
    await settle();

    expect(server.posts).toEqual([
      {
        method: 'POST',
        marker: '1',
        action: 'create',
        payload: {
          name: 'New Person',
          username: 'new01',
          phone: '0812345678',
          email: null,
          password: 'temporary-pass-01',
          roles: [{ role: 'TELLER', branch: 'MAIN' }],
        },
      },
    ]);
    expect(openDialogEl(), 'the dialog closes once saved').toBeNull();
    expectBothReRead(mark, 'create');
    expect(norm(rowOf('new01').textContent)).toContain('New Person');
    expect(server.others).toEqual([]);
  });

  it('resets a password from a PasswordField that its submit conceals, and posts {id, password}', async () => {
    await mount();
    button(rowOf('front01'), T.reset).click();
    await settle();
    const d = theDialog();
    expect(dialogTitle()).toBe(T.resetTitle);
    expect(norm(d.textContent)).toContain(T.sessionsEnd);
    const password = field(d, T.newPassword);
    expect(password.type).toBe('password');
    const eye = password.parentElement?.querySelector<HTMLButtonElement>('button[aria-pressed]');
    expect(eye, "PasswordField's eye sits inside the field's wrapper (#378)").toBeTruthy();
    eye!.click();
    await settle();
    expect(password.type, 'the eye shows it').toBe('text');
    type(password, 'brand-new-pass-02');

    // a refused submit: the dialog stays, and the password is concealed by the submit itself
    const short = 'รหัสผ่านต้องมี 12–128 ตัวอักษร';
    server.refuse.reset = short;
    button(d, T.resetSubmit).click();
    expect(password.type, 'concealed within the submit').toBe('password');
    await settle();
    expect(openDialogEl(), 'refused: the dialog stays open').not.toBeNull();
    expect(norm(theDialog().textContent)).toContain(short);
    expect(password.type, 'still concealed after the refusal').toBe('password');

    delete server.refuse.reset;
    const mark = server.reads.length;
    button(d, T.resetSubmit).click();
    await settle();
    expect(server.posts.map((p) => [p.method, p.marker, p.action, p.payload])).toEqual([
      ['POST', '1', 'reset', { id: 'e-1', password: 'brand-new-pass-02' }],
      ['POST', '1', 'reset', { id: 'e-1', password: 'brand-new-pass-02' }],
    ]);
    expect(openDialogEl()).toBeNull();
    expectBothReRead(mark, 'reset');
    expect(norm(rowOf('front01').cells[3]?.textContent)).toBe(T.mustChange);
  });

  it("disables an account only after v2's confirm, and posts {id}", async () => {
    await mount();
    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);
    button(rowOf('front01'), T.disable).click();
    await settle();
    expect(confirm).toHaveBeenCalledWith(T.disableConfirm);
    expect(server.posts, 'cancelled: nothing is sent').toEqual([]);

    confirm.mockReturnValue(true);
    const mark = server.reads.length;
    button(rowOf('front01'), T.disable).click();
    await settle();
    expect(server.posts.map((p) => [p.method, p.marker, p.action, p.payload])).toEqual([['POST', '1', 'disable', { id: 'e-1' }]]);
    expectBothReRead(mark, 'disable');
    expect(norm(rowOf('front01').cells[3]?.textContent)).toBe(T.disabled);
  });

  it("toasts the server's sentence when a disable is refused (employees.js:10)", async () => {
    await mount();
    vi.stubGlobal('confirm', vi.fn(() => true));
    server.refuse.disable = 'ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน';
    button(rowOf('front01'), T.disable).click();
    await settle();
    expect(toastText()).toBe(server.refuse.disable);
    expect(norm(rowOf('front01').cells[3]?.textContent), 'still open').toBe(T.active);
  });

  it("keeps a dialog open and shows the server's sentence inside it when create or reset is refused", async () => {
    const sentence = 'สร้างบัญชีให้ครบ Front, Backend, Manager, Rates, Monitor และ Finance ในสาขาก่อน';
    for (const action of ['create', 'reset'] as const) {
      server = new Server();
      vi.stubGlobal('fetch', vi.fn(server.fetch));
      server.refuse[action] = sentence;
      await mount();
      if (action === 'create') {
        button(page(), T.add).click();
        await settle();
        type(field(theDialog(), T.name), 'Somebody');
        type(field(theDialog(), T.phone), '0812345678');
        type(field(theDialog(), T.username), 'some01');
        type(field(theDialog(), T.password), 'temporary-pass-01');
        button(theDialog(), T.createSubmit).click();
      } else {
        button(rowOf('front01'), T.reset).click();
        await settle();
        type(field(theDialog(), T.newPassword), 'temporary-pass-01');
        button(theDialog(), T.resetSubmit).click();
      }
      await settle();
      expect(server.posts.map((p) => p.action), action).toEqual([action]);
      expect(openDialogEl(), `${action}: the dialog stays open`).not.toBeNull();
      expect(norm(theDialog().textContent), `${action}: the sentence is shown in the dialog`).toContain(sentence);
      const submitWords: string[] = [T.createSubmit, T.resetSubmit];
      const submits = Array.from(theDialog().querySelectorAll<HTMLButtonElement>('button')).filter((b) =>
        submitWords.includes(norm(b.textContent)),
      );
      expect(submits.length, action).toBe(1);
      expect(submits[0]!.disabled, `${action}: the submit can be pressed again`).toBe(false);
      closeDialog();
      dispose?.();
      dispose = undefined;
      container?.remove();
      container = undefined;
    }
  });
});

// ── E2: CA-10-FE — the retired custody workflow is gone (cash-shift-transfer.md §6 Q7) ─────

/** v2's workflow words the screen no longer shows (CA-10-FE). */
const RETIRED = {
  stateOn: 'สาขานี้เปิดใช้ขั้นตอนรับ–ส่งเงินแล้ว',
  stateOff: 'สาขานี้ยังใช้ขั้นตอนเดิม สร้างบัญชีครบ 6 บทบาทและปิดกะเดิมก่อนเปิดใช้',
  entityButton: 'ยืนยันกิจการและจุดบริการ',
  enableButton: 'เปิดใช้สำหรับสาขานี้',
} as const;

describe('CA-10-FE: the employees screen drops the retired enable / entity actions', () => {
  it('shows no Enable control, no entity control and no enabled/not-enabled line, on a branch with or without the flow on', async () => {
    const m = await mount();
    for (const branch of ['MAIN', 'A1']) {
      if (branch !== 'MAIN') {
        m.setBranch(branch);
        await settle();
      }
      const words = Array.from(page().querySelectorAll('button')).map((b) => norm(b.textContent));
      expect(words, `${branch}: no Enable button`).not.toContain(RETIRED.enableButton);
      expect(words, `${branch}: no entity button`).not.toContain(RETIRED.entityButton);
      const text = norm(page().textContent);
      expect(text, `${branch}: no state line`).not.toContain(RETIRED.stateOn);
      expect(text, `${branch}: no state line`).not.toContain(RETIRED.stateOff);
    }
  });

  /** Presses every enabled button of the menu once, ticking every checkbox and submitting each dialog it opens;
   *  gives back the values of every input and textarea the page or a dialog showed on the way. */
  async function pressEverything(): Promise<string[]> {
    vi.stubGlobal('confirm', vi.fn(() => false));
    const seen: string[] = [];
    const look = () => {
      for (const e of Array.from(document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>('input, textarea'))) seen.push(e.value);
    };
    look();
    const count = menuButtons().length;
    for (let i = 0; i < count; i++) {
      const b = menuButtons()[i];
      if (!b || b.disabled) continue;
      b.click();
      await settle();
      const d = openDialogEl();
      if (d) {
        look();
        for (const box of Array.from(d.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'))) if (!box.checked) box.click();
        d.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();
        await settle();
        if (openDialogEl()) closeDialog();
        await settle();
      }
    }
    return seen;
  }

  it('shows the stored entity as read-only text, `company · site`, and nothing on a branch with none', async () => {
    const m = await mount({ branch: 'A1' });
    expect(norm(menuArea().textContent)).toContain('Alpha Co. · Alpha Site');
    m.setBranch('MAIN');
    await settle();
    expect(norm(menuArea().textContent)).not.toContain('Alpha Co.');
    m.setBranch('A1');
    await settle();
    const values = await pressEverything();
    expect(values, 'no field is prefilled with the stored company').not.toContain('Alpha Co.');
    expect(values, 'no field is prefilled with the stored site').not.toContain('Alpha Site');
  });

  it('posts neither enable nor entity, whatever is pressed, on a branch with the flow off', async () => {
    await mount({ branch: 'MAIN' });
    await pressEverything();
    const actions = server.posts.map((p) => p.action);
    expect(actions).not.toContain('enable');
    expect(actions).not.toContain('entity');
  });
});

// ── E3: the staff page link (admin-workspace.js:139) ─────────────────────────

describe('V2-employees E3: the link to the staff page', () => {
  it("points `เปิดหน้าพนักงาน ↗` at /staff in a new tab, as the served page does", async () => {
    await mount();
    const links = Array.from(page().querySelectorAll<HTMLAnchorElement>('a')).filter((a) => norm(a.textContent) === T.staffLink);
    expect(links).toHaveLength(1);
    expect(links[0]!.getAttribute('href')).toBe('/staff');
    expect(links[0]!.getAttribute('target')).toBe('_blank');
    expect(links[0]!.getAttribute('rel') ?? '').toContain('noopener');
  });
});

// ── W10: every text through this folder's table ──────────────────────────────

describe("V2-employees W10: the menu's texts come from employees/i18n.ts", () => {
  it("holds every v2 sentence above, verbatim, in `th`, with the same keys in `en` and no Thai there", () => {
    const { th, en } = table();
    const values = new Set(Object.values(th));
    const expected = [...Object.values(T).flat().filter((s) => s !== ''), 'พนักงานและสิทธิ์', 'บัญชีพนักงานและการตั้งค่าขั้นตอนรับ–ส่งเงิน'];
    for (const s of expected) {
      const found = values.has(s);
      expect(found, `th holds "${s}"`).toBe(true);
    }
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(en)) expect(THAI.test(v), `en.${k} has no Thai: ${v}`).toBe(false);
  });

  it('shows no Thai anywhere in the menu or its dialogs once the language is English', async () => {
    setLang('en');
    await mount();
    vi.stubGlobal('confirm', vi.fn(() => false));
    expect(THAI.test(menuArea().textContent ?? ''), norm(menuArea().textContent)).toBe(false);
    const labels = Array.from(page().querySelectorAll('[aria-label], [title], [placeholder]')).map(
      (e) => `${e.getAttribute('aria-label') ?? ''} ${e.getAttribute('title') ?? ''} ${e.getAttribute('placeholder') ?? ''}`,
    );
    expect(labels.filter((l) => THAI.test(l))).toEqual([]);

    const count = menuButtons().length;
    expect(count).toBeGreaterThanOrEqual(6);
    for (let i = 0; i < count; i++) {
      // looked up again each time: a re-read may draw the rows anew
      const b = menuButtons()[i];
      if (!b || b.disabled) continue;
      b.click();
      await settle();
      const d = openDialogEl();
      if (d) {
        expect(THAI.test(d.textContent ?? ''), `dialog of "${norm(b.textContent)}": ${norm(d.textContent)}`).toBe(false);
        const attrs = Array.from(d.querySelectorAll('[aria-label], [placeholder]')).map(
          (e) => `${e.getAttribute('aria-label') ?? ''} ${e.getAttribute('placeholder') ?? ''}`,
        );
        expect(attrs.filter((l) => THAI.test(l)), `dialog of "${norm(b.textContent)}"`).toEqual([]);
        closeDialog();
        await settle();
      }
    }
    const asked = (globalThis.confirm as unknown as { mock: { calls: unknown[][] } }).mock.calls;
    expect(asked.length, 'the disable confirm was asked').toBeGreaterThanOrEqual(1);
    for (const call of asked) expect(THAI.test(String(call[0])), String(call[0])).toBe(false);
    expect(server.posts, 'no write was sent').toEqual([]);
  });
});

// ── done criteria 3 and 4, in the source ─────────────────────────────────────

describe('V2-employees: the folder makes no request and no money arithmetic of its own (done criteria 3, 4)', () => {
  it('has menu.ts, Employees.tsx and i18n.ts, none calling fetch( or doing money arithmetic', () => {
    const files = Object.keys(sources).filter((f) => !f.includes('.test.'));
    for (const want of ['./menu.ts', './Employees.tsx', './i18n.ts']) expect(files, want).toContain(want);
    for (const f of files) {
      const text = sources[f]!;
      expect(/fetch\(/.test(text), `${f}: every call goes through ApiClient`).toBe(false);
      expect(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/.test(text), `${f}: no money arithmetic`).toBe(false);
    }
  });
});
