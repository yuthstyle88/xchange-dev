// V2-shell (#460): v3's own admin page at /ui/admin/ — the layout, the branch picker, the menu list
// and the HTML entry (ticket §7, §9, §10; design §3.3, §12.3–§12.6, §12.14 in PR 474).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// The shell runs against a fake `ApiClient`: `view('me', {branch})` answers as the `me` view does
// (`Policy::branch`: staff get their own branch whatever they name, the owner what they name or
// MAIN); `view('admin_branches')` answers the caller's branches (staff: their own only); `post`
// records what it is sent. AppShell's own reads (`/health/ready`, and the shared session's
// `/api/v3/views/me`) go to the global `fetch`, which answers those two and records anything else
// as a request the shell made by itself.
//
// The menus are fixtures shaped as the served nav (design §12.3): the shell is tested with what it
// is given, never with a menu id written into it. Each fixture records its mounts and the
// `branch()`, `period()` and `shown()` it is given.
//
// "Visible" below means: in the document, with no ancestor (itself included) carrying `hidden` or
// an inline `display: none`. A part not rendered at all is not visible either. (CSS files are not
// applied in these tests, so a class alone cannot hide anything here.)
//
// The shell's own texts are v2's Thai verbatim (W6) — group headings, links, header, search,
// period toolbar, notice, footer and the add-branch dialog (xchange-v2-source@4d0bdbb,
// `admin-html:N` = line N of the decoded admin page, ticket §2; `admin-workspace.js`,
// `admin-devices.js` in src/devices-public/).
//
// The shell, its table and the HTML entry are read through eager globs, so on a branch without
// them each test fails on an assertion that the file exists, not on the import.
//
// Amended by SB-2 (design docs/design/blueprint/org-roles-branches.md §14.0 R1, R2, §14.4 "Admin"),
// by its ticket author claude/dell-claude-3. The session has one branch, switched from the account
// bar; admin reads it and no longer keeps its own. Each change:
//   - the fake `me` view answers the session branch (`world.session`, default MAIN; staff: their own),
//     whatever branch is named, as SB-1's `me` does, and carries `branchName` and `branches`;
//   - `world.meFails` makes the `me` view refuse;
//   - "links … with ?branch=<picked>" → the links carry no `?branch=`;
//   - "follows the picked branch in those links after a switch" → the sidebar has no branch select;
//   - the heading test switched with the select → it mounts on session branch B1;
//   - "the branch picker" describe → "the session branch": the menus get `me.branch`, `?branch=` and
//     `'MAIN'` are not used, staff get their own branch;
//   - add branch sends the session branch as `branchId`; the check that the new branch appears in the
//     select goes with the select;
//   - "locks the branch picker while a write is pending" is removed: there is no picker to lock.
// Added by SB-2's test writer claude/5d45b017 on the lane controller's ruling 1 (#892
// issuecomment-6104783522): the add-branch auto-switch (`AdminShell.tsx:381-389`, "Controller Decision 3"
// of #460) is dropped with nothing in its place. Creating a branch calls no switch route, and the
// session branch the shell works on stays the same; the new branch is reached from the account-bar menu.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` fixture (`kind`/`role`/`username`) → `meOf(…)` (`web/src/test-support/me.ts`; the owner is
// EXECUTIVE + ADMIN organisation-wide, staff hold their §3.3 role at their branch, each with that role's
// permissions); the fixture menus' `roles: 'all'` → `needs: 'all'`. The pin "hides a menu whose roles do
// not hold me.role …" is rewritten as #1030's web case 4 (a menu `needs` a permission: hidden when
// `me.permissions` lacks it). Added: web cases 2 and 3 with the real approvals menu (FINANCE sees it,
// TELLER does not, the owner does).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createEffect, onCleanup, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import type { Me } from '../../shared/api/generated/common';
import { lang, screenLookup, setLang } from '../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from './menus';
import { menu as approvalsMenu } from './approvals/menu';
import { meOf } from '../../test-support/me';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../shared/testing/page-wide';

type ShellModule = typeof import('./AdminShell');
type TextsModule = typeof import('./i18n');
type RefreshModule = typeof import('./parts/refresh');

const shells = import.meta.glob<ShellModule>('./AdminShell.tsx', { eager: true });
const texts = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const refreshes = import.meta.glob<RefreshModule>('./parts/refresh.ts', { eager: true });
const entries = import.meta.glob<string>('../../../pages/admin.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});

function shellModule(): ShellModule {
  const m = shells['./AdminShell.tsx'];
  expect(m, 'web/src/screens/admin/AdminShell.tsx exists').toBeDefined();
  return m!;
}

function textsModule(): TextsModule {
  const m = texts['./i18n.ts'];
  expect(m, 'web/src/screens/admin/i18n.ts exists').toBeDefined();
  return m!;
}

const THAI = /[฀-๿]/;

// ── the served nav (design §12.3; admin-workspace.js:11-20, :85-97, :146) ─────

interface Row {
  id: string;
  group: AdminMenu['group'];
  order: number;
  label: string;
  icon: string;
  dated: boolean;
  eyebrow: string;
  title: string;
  subtitle: string;
}

const ROWS: Row[] = [
  { id: 'overview', group: 'desk', order: 1, label: 'ภาพรวม', icon: '▦', dated: true, eyebrow: 'DAILY OVERVIEW', title: 'ภาพรวมหน้าร้าน', subtitle: 'ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก' },
  { id: 'transactions', group: 'desk', order: 2, label: 'รายการแลกเงิน', icon: '⇄', dated: true, eyebrow: 'TRANSACTIONS', title: 'รายการแลกเงิน', subtitle: 'ค้นหารายการ เปิดรายละเอียด ใบเสร็จ และเอกสารที่ผูกกับออเดอร์' },
  { id: 'bookings', group: 'desk', order: 3, label: 'การจอง', icon: '▤', dated: false, eyebrow: 'ONLINE BOOKINGS', title: 'การจอง', subtitle: 'ตรวจการจอง ล็อกเรท และส่งต่อไปทำรายการหน้าร้าน' },
  { id: 'cash', group: 'money', order: 4, label: 'เงินสดและกะ', icon: '▣', dated: false, eyebrow: 'CASH & SHIFTS', title: 'เงินสดและกะ', subtitle: 'เปิดกะ รับ–จ่าย ตรวจนับ และติดตามประวัติในสาขาเดียวกัน' },
  { id: 'rates', group: 'money', order: 5, label: 'เรทและสกุลเงิน', icon: '◎', dated: false, eyebrow: 'RATE MANAGEMENT', title: 'เรทและสกุลเงิน', subtitle: 'เรทหน้าร้านและข้อมูลอ้างอิงสำหรับสาขาที่เลือก' },
  { id: 'payments', group: 'money', order: 6, label: 'วิธีชำระเงิน', icon: '▣', dated: false, eyebrow: 'PAYMENT SETTINGS', title: 'ตั้งค่าวิธีชำระเงิน', subtitle: 'เปิด–ปิดช่องทาง กำหนดข้อความแนะนำ และดูประวัติการตั้งค่า' },
  { id: 'reports', group: 'money', order: 7, label: 'รายงาน', icon: '▥', dated: true, eyebrow: 'DAILY REPORT', title: 'รายงาน', subtitle: 'แยกดูช่องทางชำระเงิน สกุลเงิน และยอดของพนักงาน' },
  { id: 'customers', group: 'data', order: 8, label: 'ลูกค้าและสมาชิก', icon: '♙', dated: false, eyebrow: 'CUSTOMERS', title: 'ลูกค้าและสมาชิก', subtitle: 'ประวัติลูกค้าและคำขอลงทะเบียน แยกเป็นหมวดเพื่อค้นหาได้เร็ว' },
  { id: 'slips', group: 'data', order: 9, label: 'สลิปและหลักฐาน', icon: '▧', dated: false, eyebrow: 'PAYMENT SLIPS', title: 'สลิปและหลักฐาน', subtitle: 'ค้นหาหลักฐานรับ–จ่ายจากรายการแลกเงิน' },
];

// group headings admin-workspace.js:12-15; the shell's own links :32-35, :55 and admin-devices.js:3
const GROUP = { desk: 'งานหน้าร้าน', money: 'เงินและรายงาน', data: 'ข้อมูลและทีม', tools: 'เครื่องมือ' } as const;
const QUEUE = 'คิวลูกค้า';
const KIOSK = 'เปิดหน้าลูกค้า ↗';
const DEVICES = 'งานอุปกรณ์';
const GUIDE = 'คู่มือพนักงาน ↗';
const NO_MENU = 'ไม่พบเมนู ลองใช้คำอื่น'; // admin-workspace.js:45
const SEARCH_PLACEHOLDER = 'ค้นหาเมนู…'; // :43
const MENU_BUTTON = '☰ เมนู'; // :60
const MANAGE_BRANCHES = 'จัดการสาขา'; // :79
const ADD_BRANCH = '＋ สาขา'; // admin-html:13
const REFRESH = 'รีเฟรช'; // admin-html:13
const TODAY = 'วันนี้'; // admin-html:16
const ALL_DATES = 'ทั้งหมด'; // admin-html:16
const DATE_ERROR = 'วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด'; // admin-html:57
const RECEIPT_SAMPLE = 'ตัวอย่างใบเสร็จ'; // admin-html:21
const CONNECTION = 'ข้อมูลและการเชื่อมต่อ ↗'; // admin-html:12
const CONNECTION_TITLE = 'บัญชีออนไลน์'; // admin-html:135
const CONNECTION_1 = 'บัญชีนี้เก็บข้อมูลบนเซิร์ฟเวอร์ใหม่ เปิดกะและตั้งเรทก่อนเริ่มใช้งาน ข้อมูลเดิมใน Google ยังไม่ได้ย้ายเข้ามา';
const CONNECTION_2 = 'รายการทุกสาขาและไฟล์แนบใช้ร่วมกันได้เมื่อเปิดจากบัญชีเจ้าของเว็บนี้';
const LIVE_ONLINE = 'เซิร์ฟเวอร์ · ข้อมูลจริง'; // admin-html:100
const LIVE_OFFLINE = 'เซิร์ฟเวอร์ · ยังไม่พร้อมใช้งาน'; // admin-html:100
const LAST_SYNC = 'อัปเดตล่าสุด '; // admin-html:100
// add branch, admin-html:168
const ADD_TITLE = 'เพิ่มสาขา';
const ADD_NOTE = 'แต่ละสาขามีเงินสด สต็อก กะ รายการ และเรทของตัวเอง สาขาใหม่เริ่มโดยไม่มีรายการหรือกะ';
const ADD_PLACEHOLDER = 'เช่น XC-C';
const ADD_SUBMIT = 'สร้างสาขา';
const ADD_ERROR = 'กรุณาระบุชื่อสาขาที่ไม่ซ้ำ';
const SAVED = 'เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว'; // admin-html:120 (pendingWrite, V2-parts)
// DEMO texts that must never show (S1): admin-html:14, :15, :21, :13
const DEMO_TEXTS = ['พื้นที่ทดลองสำหรับแอดมิน', 'คืนค่าข้อมูลทดลอง', 'เพิ่มรายการทดลอง', 'โหมดทดลอง', 'เซิร์ฟเวอร์ จริง'];

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

// ── fixture menus ────────────────────────────────────────────────────────────

interface Probe {
  mounts: number;
  cleanups: number;
  branch: string[];
  period: { from: string; to: string }[];
  shown: boolean[];
}

let probes: Record<string, Probe> = {};

function fixtureMenu(row: Row, over: Partial<AdminMenu> = {}): AdminMenu {
  const th = () => lang() === 'th';
  const Component = (props: AdminMenuProps): JSX.Element => {
    const p = probes[row.id]!;
    p.mounts++;
    onCleanup(() => {
      p.cleanups++;
    });
    createEffect(() => p.branch.push(props.branch()));
    createEffect(() => {
      const v = props.period();
      p.period.push({ from: v.from, to: v.to });
    });
    createEffect(() => p.shown.push(props.shown()));
    return <section data-fixture-menu={row.id}>{`body of ${row.id}`}</section>;
  };
  return {
    id: row.id,
    label: () => (th() ? row.label : `EN ${row.id}`),
    icon: row.icon,
    order: row.order,
    group: row.group,
    needs: 'all',
    heading: {
      eyebrow: () => row.eyebrow,
      title: () => (th() ? row.title : `EN title ${row.id}`),
      subtitle: () => (th() ? row.subtitle : `EN subtitle ${row.id}`),
    },
    dated: row.dated,
    Component,
    ...over,
  };
}

function allMenus(): AdminMenu[] {
  return ROWS.map((r) => fixtureMenu(r));
}

function probe(id: string): Probe {
  return probes[id]!;
}

/** Menus mounted right now: mounted more times than cleaned up. */
function mounted(): string[] {
  return Object.entries(probes)
    .filter(([, p]) => p.mounts > p.cleanups)
    .map(([id]) => id);
}

// ── the fake ApiClient ───────────────────────────────────────────────────────

interface Branch {
  id: string;
  name: string;
}

interface Read {
  name: string;
  branch?: string;
}

interface Post {
  path: string;
  body: { action: string; payload: Record<string, unknown> };
}

const BRANCHES: Branch[] = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
  { id: 'B2', name: 'สาขาสอง' },
];

interface World {
  who: { kind: 'owner' } | { kind: 'staff'; role: string; branch: string };
  /** The session branch the `me` view answers for the owner (SB-1); staff always get their own. */
  session?: string;
  /** When set, the `me` view refuses. */
  meFails?: boolean;
  branches: Branch[];
  branchesFail: boolean;
  reads: Read[];
  posts: Post[];
  /** When set, a post waits for this promise before answering. */
  hold?: Promise<void>;
}

let world: World;

type MeSB = Me & { branchName: string; branches: { id: string; code: string; name: string }[] };

function refOf(id: string): { id: string; code: string; name: string } {
  return { id, code: id, name: world.branches.find((b) => b.id === id)?.name ?? id };
}

/** The `me` view as SB-1 answers it: the session branch, whatever branch the request names. */
function meFor(): MeSB {
  if (world.who.kind === 'staff') {
    const b = world.who.branch;
    return meOf({ id: 'e-1', role: world.who.role, branch: b, name: 'Somchai', branchName: refOf(b).name, branches: [refOf(b)] });
  }
  const b = world.session ?? 'MAIN';
  return meOf({ id: 'owner', role: 'OWNER', branch: b, name: 'Owner', branchName: refOf(b).name, branches: world.branches.map((x) => refOf(x.id)) });
}

function branchesFor(): Branch[] {
  const who = world.who;
  if (who.kind === 'staff') return world.branches.filter((b) => b.id === who.branch);
  return world.branches;
}

const fakeApi = {
  async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
    world.reads.push({ name, branch: q.branch });
    if (name === 'me') {
      if (world.meFails) throw new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status: 401 });
      return { version: 3, changed: true, data: meFor() as unknown as T };
    }
    if (name === 'admin_branches') {
      if (world.branchesFail) {
        throw new ApiError({ code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง', status: 500 });
      }
      return { version: 3, changed: true, data: { branches: branchesFor().map((b) => ({ ...b })) } as T };
    }
    throw new Error(`the shell reads no view ${name}`);
  },
  async page<R>(name: string): Promise<R[]> {
    throw new Error(`the shell reads no page ${name}`);
  },
  async act<B, R>(area: string, action: string, _body: B): Promise<R> {
    throw new Error(`the shell calls no v3 action ${area}/${action}`);
  },
  post<B, R>(path: string, body: B): Promise<R> {
    const b = body as unknown as Post['body'];
    world.posts.push({ path, body: b });
    return (async () => {
      if (world.hold) await world.hold;
      if (b.action === 'adminCreateBranch') {
        const name = String(b.payload['name']);
        world.branches.push({ id: `B-${name}`, name });
        // the snapshot v2 answered (of the new branch): the shell ignores it (ticket §7)
        return { branchId: `B-${name}`, branches: world.branches, ready: true } as R;
      }
      throw new Error(`unexpected post ${b.action}`);
    })();
  },
};

const api = fakeApi as unknown as ApiClient;

function reads(name: string): Read[] {
  return world.reads.filter((r) => r.name === name);
}

// ── the global fetch (AppShell's reads only) ─────────────────────────────────

let ownFetches: string[] = [];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

// ── rendering ────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

const realSetTimeout = globalThis.setTimeout;

/** Lets the shell's promises and effects run (with or without fake timers). */
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function at(url: string): void {
  window.history.replaceState(null, '', url);
}

async function mount(menus: AdminMenu[], url = '/ui/admin/'): Promise<HTMLDivElement> {
  at(url);
  const { AdminShell } = shellModule();
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <AdminShell api={api} menus={menus} />, container);
  await settle();
  return container;
}

/** Follows a link to `hash`, as the browser does (jsdom fires `hashchange` itself). */
async function go(hash: string): Promise<void> {
  window.location.hash = hash;
  await settle();
}

function visible(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (e.hasAttribute('hidden')) return false;
    if (e instanceof HTMLElement && e.style.display === 'none') return false;
  }
  return true;
}

/** The text of an element without its `aria-hidden` parts (the icons). */
function spoken(el: Element): string {
  const copy = el.cloneNode(true) as Element;
  copy.querySelectorAll('[aria-hidden="true"]').forEach((n) => n.remove());
  return (copy.textContent ?? '').replace(/\s+/g, ' ').trim();
}

/** The innermost elements whose whole text is `text`. */
function byText(text: string, root: Element = document.body): Element[] {
  const all = Array.from(root.querySelectorAll('*'));
  return all.filter(
    (e) =>
      (e.textContent ?? '').trim() === text &&
      !Array.from(e.children).some((c) => (c.textContent ?? '').trim() === text),
  );
}

function button(text: string): HTMLButtonElement {
  const found = Array.from(document.body.querySelectorAll('button')).filter((b) => spoken(b) === text || (b.textContent ?? '').trim() === text);
  expect(found.length, `one button ${JSON.stringify(text)}`).toBeGreaterThanOrEqual(1);
  return found[0]!;
}

function link(label: string): HTMLAnchorElement {
  const found = Array.from(container.querySelectorAll('a')).filter((a) => spoken(a) === label);
  expect(found, `one link ${JSON.stringify(label)}`).toHaveLength(1);
  return found[0]!;
}

/** Every select in the shell that offers a branch id (SB-2: there must be none). */
function branchSelects(): HTMLSelectElement[] {
  return Array.from(container.querySelectorAll('select')).filter((s) =>
    Array.from(s.options).some((o) => world.branches.some((b) => b.id === o.value)),
  );
}

function dates(): [HTMLInputElement, HTMLInputElement] {
  const found = Array.from(container.querySelectorAll<HTMLInputElement>('input[type="date"]'));
  expect(found, 'the period toolbar has two date inputs, from then to').toHaveLength(2);
  return [found[0]!, found[1]!];
}

/** Is the period toolbar shown? (Not rendered at all counts as not shown.) */
function toolbarShown(): boolean {
  const inputs = Array.from(container.querySelectorAll<HTMLInputElement>('input[type="date"]'));
  const today = Array.from(container.querySelectorAll('button')).filter((b) => (b.textContent ?? '').trim() === TODAY);
  const shown = inputs.some(visible);
  expect(today.some(visible), `${TODAY} follows the date inputs`).toBe(shown);
  return shown;
}

function search(): HTMLInputElement {
  const found = Array.from(container.querySelectorAll<HTMLInputElement>('input')).filter(
    (i) => i.getAttribute('placeholder') === SEARCH_PLACEHOLDER,
  );
  expect(found, 'one menu search').toHaveLength(1);
  return found[0]!;
}

function setValue(input: HTMLInputElement | HTMLSelectElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function text(): string {
  return (container.textContent ?? '').replace(/\s+/g, ' ');
}

/** The sidebar in document order: group headings and link labels, as far as `known` lists them. */
function navSequence(known: string[]): string[] {
  const out: string[] = [];
  for (const el of Array.from(container.querySelectorAll('*'))) {
    if (el.tagName === 'A') {
      const s = spoken(el);
      if (known.includes(s)) out.push(s);
    } else if (
      (Object.values(GROUP) as string[]).includes((el.textContent ?? '').trim()) &&
      el.children.length === 0
    ) {
      out.push((el.textContent ?? '').trim());
    }
  }
  return out;
}

/** A fixed instant: 2026-10-08 12:00 in Bangkok. */
const NOW = new Date('2026-10-08T05:00:00Z');

beforeEach(() => {
  // Fake clocks for every test: the page-wide toast (V2-parts) is expired after each test, so no
  // test sees another's toast; the dates the period starts from are fixed.
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  probes = {};
  for (const r of ROWS) probes[r.id] = { mounts: 0, cleanups: 0, branch: [], period: [], shown: [] };
  world = { who: { kind: 'owner' }, branches: BRANCHES.map((b) => ({ ...b })), branchesFail: false, reads: [], posts: [] };
  ownFetches = [];
  setLang('th');
  try {
    window.localStorage.clear();
  } catch {
    // storage blocked
  }
  const fetchSpy = vi.fn(async (input: string) => {
    const url = String(input);
    if (url.includes('/health/ready')) {
      return jsonResponse({ version: '3.0.0', build: 'abcdef1234567', channel: 'staging', builtAt: '2026-10-08T00:00:00Z' });
    }
    if (url.startsWith('/api/v3/views/me')) {
      return jsonResponse({ version: 1, changed: true, data: meFor() });
    }
    if (url.startsWith(QUEUE_CARD_PATH)) {
      return jsonResponse(queueCardUnchanged());
    }
    ownFetches.push(url);
    return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
  });
  vi.stubGlobal('fetch', fetchSpy);
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({
      matches: true,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    })),
  );
});

afterEach(async () => {
  // Leave the page-wide parts (V2-parts' one dialog and one toast) as a new page finds them.
  setLang('th');
  await settle();
  for (const b of Array.from(document.querySelectorAll<HTMLButtonElement>('button[aria-label="ปิดหน้าต่าง"]'))) {
    if (!b.disabled) b.click();
  }
  await settle();
  if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
  dispose?.();
  dispose = undefined;
  container?.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
  at('/');
});

// ── the HTML entry (ticket §7 `admin.html`, done criterion 2) ─────────────────

describe('V2-shell: web/pages/admin.html (#460 §7)', () => {
  function parse(): Document {
    const raw = entries['../../../pages/admin.html'];
    expect(raw, 'web/pages/admin.html exists').toBeTypeOf('string');
    return new DOMParser().parseFromString(raw!, 'text/html');
  }
  function metas(doc: Document, name: string): string[] {
    return Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');
  }

  it("is Thai, titled as v2's admin (admin-html:3), at /ui/admin/ behind the branch gate", () => {
    const doc = parse();
    expect(doc.documentElement.getAttribute('lang')).toBe('th');
    expect(doc.title).toBe('XCHANGE · แอดมินหน้าร้าน');
    expect(metas(doc, 'xc-path')).toEqual(['/']);
    expect(metas(doc, 'xc-gate')).toEqual(['branch']);
  });

  it('mounts /src/screens/admin/main.tsx and carries none of v2\'s add-on scripts, no XC_BRANCH (S7, design §3.3)', () => {
    const doc = parse();
    const scripts = Array.from(doc.querySelectorAll('script'));
    expect(scripts.map((s) => ({ src: s.getAttribute('src'), type: s.getAttribute('type') }))).toContainEqual({
      src: '/src/screens/admin/main.tsx',
      type: 'module',
    });
    for (const s of scripts) {
      const src = s.getAttribute('src') ?? '';
      expect(src, 'no /_xc/ script').not.toMatch(/^\/_xc\//);
      for (const v2 of ['/account-bar.js', '/admin-devices.js', '/admin-workspace.js', '/admin-queue.js', '/transport.js']) {
        expect(src, `no ${v2}`).not.toBe(v2);
      }
      expect(s.textContent ?? '', 'no window.XC_BRANCH').not.toContain('XC_BRANCH');
    }
  });
});

// ── the shell's text table (ticket §7 `i18n.ts`, W10) ─────────────────────────

describe("V2-shell: the shell's own text table (#460 §7 i18n.ts)", () => {
  it('has th and en with the same keys, collected under the namespace admin', () => {
    const { th, en } = textsModule();
    expect(Object.keys(th).length).toBeGreaterThan(0);
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const k of Object.keys(th)) {
      expect(screenLookup.th[`admin.${k}`], `admin.${k}`).toBe((th as Record<string, string>)[k]);
    }
  });

  it("starts no key with a menu id or 'parts' followed by '.' (those namespaces are the menus' and the parts')", () => {
    const { th } = textsModule();
    for (const k of Object.keys(th)) {
      for (const owner of [...ROWS.map((r) => r.id), 'parts', 'queue', 'referrals']) {
        expect(k.startsWith(`${owner}.`), `key ${k}`).toBe(false);
      }
    }
  });
});

// ── the sidebar (ticket §7, §9; design §12.3) ─────────────────────────────────

describe('V2-shell: the sidebar follows the served 10-06 nav (#460 §7; design §12.3)', () => {
  it('renders the four groups in order, each group\'s menus by order, and the shell\'s own links where served', async () => {
    const shuffled = [...allMenus()].reverse();
    await mount(shuffled, '/ui/admin/#overview');
    const known = [...Object.values(GROUP), ...ROWS.map((r) => r.label), QUEUE, KIOSK, DEVICES, GUIDE];
    expect(navSequence(known)).toEqual([
      GROUP.desk, 'ภาพรวม', 'รายการแลกเงิน', 'การจอง', QUEUE,
      GROUP.money, 'เงินสดและกะ', 'เรทและสกุลเงิน', 'วิธีชำระเงิน', 'รายงาน',
      GROUP.data, 'ลูกค้าและสมาชิก', 'สลิปและหลักฐาน',
      GROUP.tools, KIOSK, DEVICES,
      GUIDE,
    ]);
  });

  it("shows each menu's icon aria-hidden beside its label, linking to /#<id>'s hash", async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    for (const r of ROWS) {
      const a = link(r.label);
      const icons = Array.from(a.querySelectorAll('[aria-hidden="true"]')).map((n) => (n.textContent ?? '').trim());
      expect(icons, `${r.id} icon`).toContain(r.icon);
      expect(a.getAttribute('href') ?? '', `${r.id} href`).toMatch(new RegExp(`#${r.id}$`));
    }
  });

  it("links the queue, the customer screen, devices and the staff guide with no ?branch= (admin-workspace.js:32-35, :55, :150; admin-devices.js:3; SB-2 §14.4)", async () => {
    world.session = 'B1';
    await mount(allMenus(), '/ui/admin/?branch=B2#overview');
    expect(link(QUEUE).getAttribute('href')).toBe('/kiosk-queue.html');
    const kiosk = link(KIOSK);
    expect(kiosk.getAttribute('href')).toBe('/kiosk');
    expect(kiosk.getAttribute('target')).toBe('_blank');
    expect(kiosk.getAttribute('rel') ?? '').toContain('noopener');
    expect(link(DEVICES).getAttribute('href')).toBe('/devices');
    expect(link(GUIDE).getAttribute('href')).toBe('/staff-guide.html');
    // each of them carries its glyph, aria-hidden
    for (const [label, icon] of [[QUEUE, '≡'], [KIOSK, '↗'], [DEVICES, '⎙'], [GUIDE, '?']] as const) {
      const icons = Array.from(link(label).querySelectorAll('[aria-hidden="true"]')).map((n) => (n.textContent ?? '').trim());
      expect(icons, `${label} icon`).toContain(icon);
    }
  });

  it('has no branch select in the sidebar: the session branch is switched from the account bar (SB-2 R1, R2)', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    expect(branchSelects()).toEqual([]);
    expect(Array.from(container.querySelectorAll('a')).map((a) => a.getAttribute('href') ?? '').filter((h) => h.includes('branch='))).toEqual([]);
  });

  it("hides a menu whose needs is not in me.permissions, and shows 'all' menus to everyone (#1030 §3.10)", async () => {
    // CashManage: BRANCH_MANAGER holds it, TELLER does not (role_permissions.rs).
    const menus = allMenus().map((m) => (m.id === 'payments' ? { ...m, needs: 'CashManage' } : m));
    world.who = { kind: 'staff', role: 'FRONT', branch: 'B2' };
    await mount(menus, '/ui/admin/#overview');
    const labels = Array.from(container.querySelectorAll('a')).filter(visible).map(spoken);
    expect(labels).not.toContain('วิธีชำระเงิน');
    expect(labels).toContain('รายงาน');
    dispose?.();
    container.remove();
    world.who = { kind: 'staff', role: 'MANAGER', branch: 'B2' };
    await mount(menus, '/ui/admin/#overview');
    const managers = Array.from(container.querySelectorAll('a')).filter(visible).map(spoken);
    expect(managers).toContain('วิธีชำระเงิน');
    expect(managers).toContain('รายงาน');
  });

  // #1030 §9 (web cases 2 and 3, issuecomment-6105551159): the real approvals menu follows
  // `ApprovalsDecide`, never a role name.
  function approvalsShown(): boolean {
    return Array.from(container.querySelectorAll('a')).filter(visible).map(spoken).includes(approvalsMenu.label());
  }

  it('a FINANCE user (ApprovalsDecide, no EXECUTIVE row) sees the approvals menu', async () => {
    world.who = { kind: 'staff', role: 'FINANCE', branch: 'B2' };
    await mount([...allMenus(), approvalsMenu], '/ui/admin/#overview');
    expect(approvalsShown()).toBe(true);
  });

  it('a TELLER (QueueOperate, no ApprovalsDecide) does not see the approvals menu; the owner (EXECUTIVE) does', async () => {
    world.who = { kind: 'staff', role: 'TELLER', branch: 'B2' };
    await mount([...allMenus(), approvalsMenu], '/ui/admin/#overview');
    expect(approvalsShown()).toBe(false);
    expect(Array.from(container.querySelectorAll('a')).filter(visible).map(spoken)).toContain('รายงาน');
    dispose?.();
    container.remove();
    world.who = { kind: 'owner' };
    await mount([...allMenus(), approvalsMenu], '/ui/admin/#overview');
    expect(approvalsShown()).toBe(true);
  });

  it('renders with no menu at all: the groups\' links and header are there, no menu body, no crash (§9)', async () => {
    await mount([], '/ui/admin/#overview');
    expect(visible(link(QUEUE))).toBe(true);
    expect(visible(link(GUIDE))).toBe(true);
    expect(visible(button(REFRESH))).toBe(true);
    expect(container.querySelector('[data-fixture-menu]')).toBeNull();
    await go('#cash');
    expect(container.querySelector('[data-fixture-menu]')).toBeNull();
  });
});

// ── the menu search (admin-workspace.js:42-52, §9) ────────────────────────────

describe('V2-shell: the menu search (#460 §9)', () => {
  it('keeps the links whose text holds the query, case-insensitively, and hides groups left empty', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    setValue(search(), 'เรท');
    await settle();
    expect(visible(link('เรทและสกุลเงิน'))).toBe(true);
    for (const other of ['ภาพรวม', 'รายการแลกเงิน', 'ลูกค้าและสมาชิก', QUEUE, DEVICES]) {
      expect(visible(Array.from(container.querySelectorAll('a')).find((a) => spoken(a) === other)), other).toBe(false);
    }
    for (const g of [GROUP.desk, GROUP.data, GROUP.tools]) {
      expect(byText(g, container).some(visible), `group ${g} hidden`).toBe(false);
    }
    expect(byText(GROUP.money, container).some(visible)).toBe(true);
    expect(byText(NO_MENU, container).some(visible), 'no "no menu" while something matches').toBe(false);

    // a group that matches shows only its matching links, its other links hidden
    setValue(search(), 'ภาพรวม');
    await settle();
    expect(visible(link('ภาพรวม'))).toBe(true);
    for (const other of ['รายการแลกเงิน', 'การจอง', QUEUE]) {
      expect(visible(Array.from(container.querySelectorAll('a')).find((a) => spoken(a) === other)), other).toBe(false);
    }

    // the shell's own links are searched too, and case does not matter
    setValue(search(), 'ลูกค้า');
    await settle();
    for (const shown of ['ลูกค้าและสมาชิก', QUEUE, KIOSK]) {
      expect(visible(link(shown)), shown).toBe(true);
    }
    for (const other of ['ภาพรวม', 'รายการแลกเงิน', DEVICES, 'สลิปและหลักฐาน']) {
      expect(visible(Array.from(container.querySelectorAll('a')).find((a) => spoken(a) === other)), other).toBe(false);
    }
    const menus = allMenus().map((m) => (m.id === 'rates' ? { ...m, label: () => 'Rates & FX' } : m));
    dispose?.();
    container.remove();
    await mount(menus, '/ui/admin/#overview');
    setValue(search(), 'RATES');
    await settle();
    expect(visible(link('Rates & FX'))).toBe(true);
  });

  it(`shows "${NO_MENU}" when nothing matches, and every link again when the query is cleared`, async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    setValue(search(), 'zzz-nothing');
    await settle();
    expect(byText(NO_MENU, container).some(visible)).toBe(true);
    for (const r of ROWS) {
      expect(visible(Array.from(container.querySelectorAll('a')).find((a) => spoken(a) === r.label)), r.id).toBe(false);
    }
    setValue(search(), '');
    await settle();
    expect(byText(NO_MENU, container).some(visible)).toBe(false);
    for (const r of ROWS) expect(visible(link(r.label)), r.id).toBe(true);
  });
});

// ── routing (S3, §9) ─────────────────────────────────────────────────────────

describe('V2-shell: hash routing (#460 S3)', () => {
  it('shows the menu the hash names, with its heading, and marks its link aria-current="page"', async () => {
    await mount(allMenus(), '/ui/admin/#slips');
    expect(mounted()).toEqual(['slips']);
    expect(text()).toContain('PAYMENT SLIPS');
    expect(byText('สลิปและหลักฐาน', container).some(visible), 'the title').toBe(true);
    expect(link('สลิปและหลักฐาน').getAttribute('aria-current')).toBe('page');
    expect(link('ภาพรวม').getAttribute('aria-current')).not.toBe('page');
  });

  it('shows the lowest-order menu for an empty or unknown hash, and leaves the hash as typed (admin-html:74)', async () => {
    await mount(allMenus(), '/ui/admin/');
    expect(mounted()).toEqual(['overview']);
    dispose?.();
    container.remove();
    for (const r of ROWS) probes[r.id] = { mounts: 0, cleanups: 0, branch: [], period: [], shown: [] };
    await mount(allMenus(), '/ui/admin/#referrals');
    expect(mounted()).toEqual(['overview']);
    expect(window.location.hash).toBe('#referrals');
    // the lowest order, not the first listed
    dispose?.();
    container.remove();
    for (const r of ROWS) probes[r.id] = { mounts: 0, cleanups: 0, branch: [], period: [], shown: [] };
    const withoutOverview = allMenus().filter((m) => m.id !== 'overview').reverse();
    await mount(withoutOverview, '/ui/admin/#nothing');
    expect(mounted()).toEqual(['transactions']);
  });

  it('mounts a menu only while it is shown, gives it shown() = true, and mounts it afresh when shown again', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    expect(mounted()).toEqual(['overview']);
    expect(probe('overview').shown.at(-1)).toBe(true);
    expect(probe('cash').mounts).toBe(0);

    await go('#cash');
    expect(mounted()).toEqual(['cash']);
    expect(probe('cash').mounts).toBe(1);
    expect(probe('cash').shown.at(-1)).toBe(true);

    await go('#overview');
    expect(mounted()).toEqual(['overview']);
    expect(probe('overview').mounts, 'overview is mounted again, so it reads on show').toBe(2);
    for (const r of ROWS) {
      expect(probe(r.id).shown.includes(false) && probe(r.id).mounts > probe(r.id).cleanups, `${r.id} hidden yet mounted`).toBe(false);
    }
  });

  it('follows /#payments, the link v2 already uses (ticket §7 routing)', async () => {
    await mount(allMenus(), '/ui/admin/#payments');
    expect(mounted()).toEqual(['payments']);
  });
});

// ── heading, period toolbar (S4, design §12.3, §12.14) ───────────────────────

describe('V2-shell: heading and period toolbar (#460 S4)', () => {
  it("shows the menu's eyebrow, title, and subtitle followed by ' · <branch name>' (admin-workspace.js:145)", async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    expect(text()).toContain('DAILY OVERVIEW');
    expect(byText('ภาพรวมหน้าร้าน', container).some(visible)).toBe(true);
    expect(byText('ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก · สำนักงานใหญ่', container).some(visible)).toBe(true);
    dispose?.();
    container.remove();
    world.session = 'B1';
    await mount(allMenus(), '/ui/admin/#overview');
    expect(byText('ติดตามสถานะกะและยอดรายการในช่วงวันที่ที่เลือก · สาขาหนึ่ง', container).some(visible)).toBe(true);
    await go('#slips');
    expect(text()).toContain('PAYMENT SLIPS');
    expect(byText('ค้นหาหลักฐานรับ–จ่ายจากรายการแลกเงิน · สาขาหนึ่ง', container).some(visible)).toBe(true);
  });

  it('shows the period toolbar for dated menus only (overview, transactions, reports; admin-workspace.js:146)', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    for (const r of ROWS) {
      await go(`#${r.id}`);
      expect(mounted(), r.id).toEqual([r.id]);
      expect(toolbarShown(), `${r.id}: toolbar shown`).toBe(r.dated);
    }
  });

  it('defaults the period to today in Asia/Bangkok, sets it with วันนี้ and ทั้งหมด, and gives it to the menu', async () => {
    // the test process runs in UTC, so a date taken from the local clock is not Bangkok's
    const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
    const tz = env['TZ'];
    env['TZ'] = 'UTC';
    try {
      // 2026-10-08 20:30 UTC is 2026-10-09 03:30 in Bangkok
      vi.setSystemTime(new Date('2026-10-08T20:30:00Z'));
      await mount(allMenus(), '/ui/admin/#overview');
      const [from, to] = dates();
      expect([from.value, to.value]).toEqual(['2026-10-09', '2026-10-09']);
      expect(probe('overview').period.at(-1)).toEqual({ from: '2026-10-09', to: '2026-10-09' });

      button(ALL_DATES).click();
      await settle();
      expect(probe('overview').period.at(-1)).toEqual({ from: '', to: '' });
      expect([dates()[0].value, dates()[1].value]).toEqual(['', '']);

      button(TODAY).click();
      await settle();
      expect(probe('overview').period.at(-1)).toEqual({ from: '2026-10-09', to: '2026-10-09' });

      setValue(dates()[0], '2026-10-01');
      await settle();
      expect(probe('overview').period.at(-1)).toEqual({ from: '2026-10-01', to: '2026-10-09' });
    } finally {
      env['TZ'] = tz;
    }
  });

  it('keeps one period for the whole page: another dated menu gets the same period', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    setValue(dates()[0], '2026-09-01');
    setValue(dates()[1], '2026-09-30');
    await settle();
    await go('#reports');
    expect(probe('reports').period.at(-1)).toEqual({ from: '2026-09-01', to: '2026-09-30' });
  });

  it(`shows "${DATE_ERROR}" for a range whose start is after its end, and never gives that range to a menu (admin-html:57, design §12.3)`, async () => {
    await mount(allMenus(), '/ui/admin/#transactions');
    setValue(dates()[0], '2026-10-01');
    setValue(dates()[1], '2026-10-05');
    await settle();
    expect(byText(DATE_ERROR, container).some(visible)).toBe(false);
    setValue(dates()[0], '2026-10-07');
    await settle();
    expect(byText(DATE_ERROR, container).some(visible)).toBe(true);
    for (const p of probe('transactions').period) {
      expect(p.from !== '' && p.to !== '' && p.from > p.to, `menu given ${JSON.stringify(p)}`).toBe(false);
    }
    setValue(dates()[0], '2026-10-02');
    await settle();
    expect(byText(DATE_ERROR, container).some(visible)).toBe(false);
    expect(probe('transactions').period.at(-1)).toEqual({ from: '2026-10-02', to: '2026-10-05' });
  });

  it('applies from / to found in the hash query to the period, and routes by the part before ? (design §12.14)', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    await go('#transactions?status=PENDING&from=&to=');
    expect(mounted()).toEqual(['transactions']);
    expect(probe('transactions').period.at(-1)).toEqual({ from: '', to: '' });
    await go('#transactions?from=2026-10-01&to=2026-10-05');
    expect(probe('transactions').period.at(-1)).toEqual({ from: '2026-10-01', to: '2026-10-05' });
  });
});

// ── the branch (§7 Branch, §9; design §12.4) ─────────────────────────────────

describe('V2-shell: the session branch (#460 §7 Branch; SB-2 design §14.4 Admin)', () => {
  it('gives the menu me.branch, and nothing before it; the heading names it from admin_branches', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    expect(reads('admin_branches').length).toBeGreaterThanOrEqual(1);
    expect(probe('overview').branch, 'the menu is given MAIN, and nothing before it').toEqual(['MAIN']);
    expect(branchSelects()).toEqual([]);
  });

  it('a session on B2: the menu is given B2, and nothing before it', async () => {
    world.session = 'B2';
    await mount(allMenus(), '/ui/admin/#cash');
    expect(probe('cash').branch).toEqual(['B2']);
    expect(mounted()).toEqual(['cash']);
  });

  it('ignores ?branch=: the menu is given me.branch, not the named one (§14.4: no ?branch=)', async () => {
    world.session = 'B2';
    await mount(allMenus(), '/ui/admin/?branch=B1#overview');
    expect(probe('overview').branch, 'the menu is given B2, and nothing before it').toEqual(['B2']);
  });

  it('gives staff their own branch, whatever ?branch= names', async () => {
    world.who = { kind: 'staff', role: 'FRONT', branch: 'B2' };
    await mount(allMenus(), '/ui/admin/?branch=B1#overview');
    expect(probe('overview').branch, 'the menu is given B2, and nothing before it').toEqual(['B2']);
    expect(link(QUEUE).getAttribute('href')).toBe('/kiosk-queue.html');
  });

  it("with no me answer it names no branch: no 'MAIN' fallback reaches a menu or a read", async () => {
    world.meFails = true;
    await mount(allMenus(), '/ui/admin/#overview');
    expect(probe('overview').branch).not.toContain('MAIN');
    expect(world.reads.filter((r) => r.name !== 'me').map((r) => r.branch)).not.toContain('MAIN');
  });

  it('re-reads admin_branches on รีเฟรช and moves refreshTick (design §12.4, §12.6)', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    const refresh = refreshes['./parts/refresh.ts'];
    expect(refresh, 'web/src/screens/admin/parts/refresh.ts (V2-parts #476) exists').toBeDefined();
    const before = reads('admin_branches').length;
    const tick = refresh!.refreshTick();
    button(REFRESH).click();
    await settle();
    expect(reads('admin_branches').length).toBe(before + 1);
    expect(refresh!.refreshTick()).not.toBe(tick);
  });
});

// ── add branch (§7 Add branch; admin-html:168) ────────────────────────────────

describe('V2-shell: add branch (#460 §7 Add branch)', () => {
  async function openAdd(): Promise<HTMLInputElement> {
    expect(byText(MANAGE_BRANCHES, container).length, `"${MANAGE_BRANCHES}" (admin-workspace.js:79)`).toBeGreaterThanOrEqual(1);
    button(ADD_BRANCH).click();
    await settle();
    expect(text() + (document.body.textContent ?? '')).toContain(ADD_TITLE);
    expect(document.body.textContent ?? '').toContain(ADD_NOTE);
    const inputs = Array.from(document.body.querySelectorAll<HTMLInputElement>('input')).filter(
      (i) => i.getAttribute('placeholder') === ADD_PLACEHOLDER,
    );
    expect(inputs, 'the branch name input').toHaveLength(1);
    expect(inputs[0]!.maxLength).toBe(60);
    return inputs[0]!;
  }

  it('refuses an empty name and a name already used (any case) without sending anything', async () => {
    world.branches = [
      { id: 'MAIN', name: 'XC-B' },
      { id: 'B1', name: 'XC-C' },
    ];
    await mount(allMenus(), '/ui/admin/#overview');
    const input = await openAdd();
    expect(document.body.textContent ?? '').not.toContain(ADD_ERROR);
    setValue(input, '   ');
    button(ADD_SUBMIT).click();
    await settle();
    expect(document.body.textContent ?? '').toContain(ADD_ERROR);
    setValue(input, 'xc-c');
    button(ADD_SUBMIT).click();
    await settle();
    expect(document.body.textContent ?? '').toContain(ADD_ERROR);
    expect(world.posts).toEqual([]);
    // a new name is sent, so the refusals above were the name's
    setValue(input, 'XC-D');
    button(ADD_SUBMIT).click();
    await settle();
    expect(world.posts.map((p) => p.body.payload['name'])).toEqual(['XC-D']);
  });

  it("sends adminCreateBranch {name} through /api/rpc with the session branch and a request id, then re-reads the branches (rpc.rs:277)", async () => {
    world.session = 'B1';
    await mount(allMenus(), '/ui/admin/#overview');
    const before = reads('admin_branches').length;
    const input = await openAdd();
    setValue(input, '  XC-C  ');
    button(ADD_SUBMIT).click();
    await settle();
    expect(world.posts).toHaveLength(1);
    const post = world.posts[0]!;
    expect(post.path).toBe('/api/rpc');
    expect(post.body.action).toBe('adminCreateBranch');
    expect(post.body.payload['name']).toBe('XC-C');
    expect(post.body.payload['branchId']).toBe('B1');
    expect(String(post.body.payload['requestId'])).toMatch(UUID);
    expect(reads('admin_branches').length).toBeGreaterThan(before);
    expect(document.body.textContent ?? '').toContain(SAVED);
  });

  it('creating a branch switches nothing: no switch route is called and the session branch stays (SB-2 ruling 1)', async () => {
    world.session = 'B1';
    await mount(allMenus(), '/ui/admin/#overview');
    expect(probe('overview').branch.at(-1), 'the menu starts on the session branch').toBe('B1');
    const readsBefore = world.reads.length;
    const input = await openAdd();
    setValue(input, 'XC-NEW');
    button(ADD_SUBMIT).click();
    await settle();
    // the branch was created
    expect(world.posts.map((p) => p.body.action)).toEqual(['adminCreateBranch']);
    expect(world.branches.some((b) => b.id === 'B-XC-NEW')).toBe(true);
    // no switch: neither through the shell's api nor through the session's own fetch
    expect(world.posts.filter((p) => p.path.includes('/api/auth/session/branch'))).toEqual([]);
    expect(ownFetches.filter((u) => u.includes('/api/auth/session/branch'))).toEqual([]);
    // the shell keeps working on the session branch: the menu never receives the new branch
    expect(probe('overview').branch).not.toContain('B-XC-NEW');
    expect(probe('overview').branch.at(-1)).toBe('B1');
    // and nothing it reads afterwards names the new branch
    expect(world.reads.slice(readsBefore).filter((r) => r.branch === 'B-XC-NEW')).toEqual([]);
  });
});

// ── notice, connection dialog, footer, mobile menu (S1, S2, S7, S8) ───────────

describe('V2-shell: notice, LIVE only, connection, footer (#460 S1, S2, S7, S8)', () => {
  it(`shows the LIVE connection state "${LIVE_ONLINE}" with the last update, never a DEMO text (S1, S2)`, async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    const all = text();
    expect(all).toContain(LIVE_ONLINE);
    expect(all).toContain(LAST_SYNC);
    for (const demo of DEMO_TEXTS) expect(all, demo).not.toContain(demo);
    const status = Array.from(container.querySelectorAll('[role="status"]')).filter((e) =>
      (e.textContent ?? '').includes(LIVE_ONLINE),
    );
    expect(status.length, 'the notice is role="status" (admin-workspace.js:82)').toBeGreaterThanOrEqual(1);
  });

  it(`shows "${LIVE_OFFLINE}" and the refusal when the branches cannot be read, disables ${ADD_BRANCH}, and recovers on รีเฟรช`, async () => {
    world.branchesFail = true;
    await mount(allMenus(), '/ui/admin/#overview');
    expect(text()).toContain(LIVE_OFFLINE);
    expect(text()).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(button(ADD_BRANCH).disabled).toBe(true);
    world.branchesFail = false;
    button(REFRESH).click();
    await settle();
    expect(text()).toContain(LIVE_ONLINE);
    expect(text()).not.toContain(LIVE_OFFLINE);
    expect(button(ADD_BRANCH).disabled).toBe(false);
  });

  it('keeps nothing of DEMO in the browser: no data-mode select, no demo branch store (S1)', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    for (const s of Array.from(container.querySelectorAll('select'))) {
      for (const o of Array.from(s.options)) {
        expect(['LIVE', 'DEMO']).not.toContain(o.value);
      }
    }
    const keys: string[] = [];
    for (let i = 0; i < window.localStorage.length; i++) keys.push(window.localStorage.key(i)!);
    expect(keys.filter((k) => k !== 'xc.lang')).toEqual([]);
  });

  it(`opens "${CONNECTION_TITLE}" with its two sentences from ${CONNECTION} (S8, admin-html:135)`, async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    button(CONNECTION).click();
    await settle();
    const page = document.body.textContent ?? '';
    expect(page).toContain(CONNECTION_TITLE);
    expect(page).toContain(CONNECTION_1);
    expect(page).toContain(CONNECTION_2);
  });

  it(`links the footer's "${RECEIPT_SAMPLE}" to /receipt-sample in a new tab (admin-html:21), with AppShell's version line (S7)`, async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    const a = link(RECEIPT_SAMPLE);
    expect(a.getAttribute('href')).toBe('/receipt-sample');
    expect(a.getAttribute('target')).toBe('_blank');
    expect(a.getAttribute('rel') ?? '').toContain('noopener');
    expect(text()).toContain('v3.0.0 · abcdef1 · staging');
  });

  it(`opens and closes the mobile menu with ${MENU_BUTTON} and Escape (admin-workspace.js:59-77)`, async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    const b = button(MENU_BUTTON);
    const controls = b.getAttribute('aria-controls');
    expect(controls, 'aria-controls names the sidebar').toBeTruthy();
    const sidebar = document.getElementById(controls!);
    expect(sidebar, 'the controlled element exists').not.toBeNull();
    expect(sidebar!.textContent ?? '').toContain('ภาพรวม');
    expect(b.getAttribute('aria-expanded')).toBe('false');
    b.click();
    await settle();
    expect(b.getAttribute('aria-expanded')).toBe('true');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle();
    expect(b.getAttribute('aria-expanded')).toBe('false');
  });
});

// ── language (§9 setLang) ─────────────────────────────────────────────────────

describe("V2-shell: setLang('en') (#460 §9, W10)", () => {
  it("switches the shell's texts and the menus' labels and headings: no Thai left in the shell", async () => {
    world.branches = [
      { id: 'MAIN', name: 'HQ' },
      { id: 'B1', name: 'Branch one' },
    ];
    await mount(allMenus(), '/ui/admin/#overview');
    const thai = text();
    for (const s of [...Object.values(GROUP), QUEUE, KIOSK, DEVICES, GUIDE, REFRESH, RECEIPT_SAMPLE, LIVE_ONLINE]) {
      expect(thai, s).toContain(s);
    }
    setLang('en');
    await settle();
    const english = text();
    expect(english.match(THAI), `Thai left: ${english.match(new RegExp(`.{0,30}${THAI.source}.{0,30}`))?.[0]}`).toBeNull();
    expect(english).toContain('EN overview');
    expect(english).toContain('EN title overview');
    for (const el of Array.from(container.querySelectorAll('[placeholder], [aria-label], [title]'))) {
      if (el.tagName === 'BUTTON' && (el.textContent ?? '').trim() === 'TH') continue; // LangSwitch names Thai
      for (const attr of ['placeholder', 'aria-label', 'title']) {
        const v = el.getAttribute(attr);
        if (v) expect(v.match(THAI), `${el.tagName} ${attr}="${v}"`).toBeNull();
      }
    }
    setLang('th');
    await settle();
    expect(text()).toContain(GROUP.desk);
  });
});

// ── no poller, no request of its own (S9, design §12.6, done criterion 4) ─────

describe('V2-shell: no polling (#460 S9; design §12.6)', () => {
  it('reads nothing on a timer: ten minutes later the same reads, and no fetch of its own', async () => {
    await mount(allMenus(), '/ui/admin/#overview');
    await vi.advanceTimersByTimeAsync(1000);
    const before = world.reads.length;
    expect(before).toBeGreaterThan(0);
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(world.reads.length, JSON.stringify(world.reads.slice(before))).toBe(before);
    expect(world.posts).toEqual([]);
    // AppShell's /health/ready and the shared session's /api/v3/views/me are not the shell's
    expect(ownFetches).toEqual([]);
  });
});
