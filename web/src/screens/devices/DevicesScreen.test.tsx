// V1-devices-screen (#404): the device-jobs screen at /devices (ticket §7, §8, §9, §10).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// The screen runs against a fake `ApiClient` that behaves as the views of #400 do: per branch,
// a version; `view` with `since` equal to it answers `changed: false`, otherwise the data; `page`
// answers a first page, or the older rows after `before`. `post` records what it is sent.
// The screen's own text is read through its table (`th.<key>`, W10; `i18n.test.ts` pins v2's
// Thai); the status and kind labels are pinned here, as v2's Thai (D14, D15) and V7's.
//
// Money (W7): the balances and the count are the `devices` view's; the picker's rows add up to
// something else, so a screen that sums or counts in the browser shows the wrong figure, and the
// THB balance is one a JavaScript number cannot hold to the satang.
//
// Amended by SB-2 (design docs/design/blueprint/org-roles-branches.md §14.0 R2, §14.4 "Devices"), by
// its ticket author claude/dell-claude-3: the session has one branch, switched from the account bar,
// so the screen keeps no branch select. Each change:
//   - D4 "the branch select lists every branch…" → the screen has no branch select, and shows the
//     branchId the devices view answers in its links;
//   - D19 "choosing another branch…" → it reads only the branch it is given and names it in every
//     write; the switch, the URL rewrite and the B1 → B2 move go with the select.
// Where the branch comes from (`me.branch`, or `?branch=`) is pinned in main.branch.test.tsx.
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { render } from 'solid-js/web';
import {
  ApiError,
  type ApiClient,
  type PageParams,
  type ViewAnswer,
  type ViewParams,
} from '../../shared/api/client';
import type { DeviceEventRow } from '../../shared/api/generated/device_events';
import type { DeviceJobRow } from '../../shared/api/generated/device_jobs';
import type { DeviceTransactionRow } from '../../shared/api/generated/device_transactions';
import type { DevicesOverview } from '../../shared/api/generated/devices';
import { errorText, fill, setLang } from '../../shared/i18n';
import { eventsCsv } from './csv';
import { DevicesScreen } from './DevicesScreen';
import { en, th } from './i18n';

// ── v2's labels, verbatim (D14, D15) and V7's ────────────────────────────────

const STATUS_THAI: Record<string, string> = {
  QUEUED: 'รอดำเนินการ',
  SIMULATED_PRINTED: 'จำลองพิมพ์สำเร็จ',
  COMMAND_SIMULATED: 'จำลองส่งคำสั่ง · ไม่ยืนยันเปิดจริง',
  FAILED: 'จำลองการเชื่อมต่อล้มเหลว',
  LEASED: 'กำลังส่ง',
  EXPIRED: 'หมดเวลา',
  DRAWER_OPENED: 'เปิดลิ้นชักแล้ว',
};

const KIND_THAI: Record<string, string> = {
  SCAN_IMPORTED: 'นำเข้าเอกสารสแกน',
  COUNT_CONFIRMED: 'พนักงานยืนยันนับเงิน',
  EXCHANGE_LINKED: 'บันทึกรายการและเชื่อมเอกสาร',
  PRINT_RECEIPT: 'คำขอพิมพ์สลิป',
  DRAWER_REQUEST: 'คำขอเปิดลิ้นชัก',
  DRAWER_MANUAL_OBSERVATION: 'พนักงานรายงานสถานะลิ้นชัก',
  SIMULATED_PRINTED: 'จำลองพิมพ์เสร็จ',
  COMMAND_SIMULATED: 'จำลองส่งคำสั่งเปิด',
  JOB_RETRY: 'ลองงานที่ล้มเหลวอีกครั้ง',
  FAILED: 'งานจำลองล้มเหลว',
  SIMULATED_PRINTER_CONNECTION: 'เปลี่ยนสถานะเครื่องพิมพ์จำลอง',
  RECEIPT_IMAGE_ARCHIVED: 'เก็บภาพรายละเอียดใบเสร็จ',
  KIOSK_REQUEST: 'ลูกค้าส่งคำขอจาก kiosk',
  KIOSK_ACCEPTED: 'พนักงานรับคิว kiosk',
  KIOSK_ID_CHECKED: 'ตรวจพาสปอร์ตคิว kiosk',
  KIOSK_COMPLETED: 'บันทึกรายการจากคิว kiosk',
  KIOSK_CURRENCY_CONFIGURED: 'ตั้งค่าสกุลเงิน kiosk',
  LEASED: 'กำลังส่ง',
  EXPIRED: 'หมดเวลา',
  DRAWER_OPENED: 'เปิดลิ้นชักแล้ว',
};

const REFUSED = 'บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้';
const THAI = /[฀-๿]/;

// ── the data the views answer ────────────────────────────────────────────────

const BRANCHES = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
  { id: 'B2', name: 'สาขาสอง' },
];

function overview(branchId: string, over: Partial<DevicesOverview> = {}): DevicesOverview {
  return {
    branchId,
    branches: BRANCHES,
    printerOnline: true,
    printerKind: 'SIMULATED',
    printerModel: 'NEOCAL NTP-8220BT',
    hasDrawer: null,
    drawerAgentActive: false,
    // not representable to the satang as a JavaScript number (it would read …409.94)
    balanceThb: '90071992547409.93',
    balanceUsd: '33250.00',
    completedCount: 7,
    ...over,
  };
}

function job(id: string, kind: string, ref: string, status: string, actor = 'front1', created = '2026-10-08T03:04:05.000Z'): DeviceJobRow {
  return { id, kind, ref, status, actor, created };
}

function event(seq: string, kind: string, ref = '', detail = '{}', actor = 'backend1', at = '2026-10-08T02:00:00.000Z'): DeviceEventRow {
  return { seq, kind, ref, at, actor, detail };
}

function tx(id: string, type: string, amount: string, currency: string): DeviceTransactionRow {
  return { id, type, amount, currency };
}

const B1_JOBS = [
  job('J3', 'PRINT_RECEIPT', 'TX-2', 'QUEUED', 'front1', '2026-10-08T03:04:05.000Z'),
  job('J2', 'DRAWER_REQUEST', 'TX-1', 'FAILED', 'backend1', '2026-10-08T02:59:00.000Z'),
  job('J1', 'DRAWER_REQUEST', '', 'COMMAND_SIMULATED', 'manager1', '2026-10-08T01:00:00.000Z'),
];

const B1_EVENTS = [
  event('42', 'PRINT_RECEIPT', 'TX-2', '{"jobId":"J3"}', 'front1', '2026-10-08T03:04:05.000Z'),
  event('41', 'SOMETHING_NEW', '', '{"a":1}', 'backend1', '2026-10-08T03:00:00.000Z'),
  event('40', 'DRAWER_OPENED', 'TX-1', '{"jobId":"J2"}', 'agent', '2026-10-08T02:58:00.000Z'),
];

// two rows of the picker: they neither add up to the balances nor count to completedCount
const B1_TXS = [tx('TX-2', 'BUY', '1000.00', 'USD'), tx('TX-1', 'SELL', '250.50', 'EUR')];

// ── the fake ApiClient ───────────────────────────────────────────────────────

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

interface BranchData {
  version: number;
  devices: DevicesOverview;
  jobs: DeviceJobRow[];
  events: DeviceEventRow[];
  transactions: DeviceTransactionRow[];
  /** `before` → the older rows. */
  olderEvents: Record<string, DeviceEventRow[]>;
  olderTransactions: Record<string, DeviceTransactionRow[]>;
}

interface Read {
  via: 'view' | 'page';
  name: string;
  branch?: string;
  since?: number;
  before?: string;
  limit?: number;
}

const VIEWS = ['devices', 'device_jobs', 'device_events', 'device_transactions'] as const;

function branchData(id: string, over: Partial<BranchData> = {}): BranchData {
  return {
    version: 5,
    devices: overview(id),
    jobs: [],
    events: [],
    transactions: [],
    olderEvents: {},
    olderTransactions: {},
    ...over,
  };
}

class FakeServer {
  readonly reads: Read[] = [];
  readonly posts: Array<{ path: string; body: unknown }> = [];
  readonly branches: Record<string, BranchData>;
  /** When set, every read is answered for this branch, whatever is named (a staff caller). */
  pinned: string | undefined;
  /** Reads of these views never answer. */
  readonly hold = new Set<string>();
  onPost: (path: string, body: unknown) => Promise<unknown> = async () => ({ saved: true });
  readonly api: Api;

  constructor(branches: Record<string, BranchData>) {
    this.branches = branches;
    const server = this;
    this.api = {
      view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
        server.reads.push({ via: 'view', name, ...(q ?? {}) });
        if (server.hold.has(name)) return new Promise<ViewAnswer<T>>(() => {});
        const b = server.branchOf(q?.branch);
        if (!b) return Promise.reject(new ApiError({ code: 'invalidRequest', message: 'Invalid branch', status: 400 }));
        if (q?.since !== undefined && q.since === b.version) {
          return Promise.resolve({ version: b.version, changed: false });
        }
        const data = server.firstPage(b, name);
        if (data === undefined) return Promise.reject(new Error(`unknown view ${name}`));
        return Promise.resolve({ version: b.version, changed: true, data: structuredClone(data) as T });
      },
      page<R>(name: string, q?: PageParams): Promise<R[]> {
        server.reads.push({ via: 'page', name, ...(q ?? {}) });
        if (server.hold.has(name)) return new Promise<R[]>(() => {});
        const b = server.branchOf(q?.branch);
        if (!b) return Promise.reject(new ApiError({ code: 'invalidRequest', message: 'Invalid branch', status: 400 }));
        let rows: unknown[] | undefined;
        if (q?.before === undefined) {
          rows = server.firstPage(b, name) as unknown[] | undefined;
        } else if (name === 'device_events') {
          rows = b.olderEvents[q.before] ?? [];
        } else if (name === 'device_transactions') {
          rows = b.olderTransactions[q.before] ?? [];
        } else {
          rows = [];
        }
        if (rows === undefined) return Promise.reject(new Error(`unknown paged view ${name}`));
        return Promise.resolve(structuredClone(rows) as R[]);
      },
      post<B, R>(path: string, body: B): Promise<R> {
        server.posts.push({ path, body: structuredClone(body) });
        return server.onPost(path, body) as Promise<R>;
      },
    };
  }

  branchOf(named: string | undefined): BranchData | undefined {
    return this.branches[this.pinned ?? named ?? 'MAIN'];
  }

  firstPage(b: BranchData, name: string): unknown {
    switch (name) {
      case 'devices':
        return b.devices;
      case 'device_jobs':
        return b.jobs;
      case 'device_events':
        return b.events;
      case 'device_transactions':
        return b.transactions;
      default:
        return undefined;
    }
  }

  readsOf(name: string): Read[] {
    return this.reads.filter((r) => r.name === name);
  }

  counts(): Record<string, number> {
    return Object.fromEntries(VIEWS.map((n) => [n, this.readsOf(n).length]));
  }
}

function b1(over: Partial<BranchData> = {}): BranchData {
  return branchData('B1', { jobs: B1_JOBS, events: B1_EVENTS, transactions: B1_TXS, ...over });
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// ── the page ─────────────────────────────────────────────────────────────────

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let fetchSpy: ReturnType<typeof vi.fn>;
let download: ReturnType<typeof vi.fn>;
let navigate: ReturnType<typeof vi.fn>;
let replaceState: MockInstance<History['replaceState']>;
let ids: number;

beforeEach(() => {
  setLang('th');
  ids = 0;
  // AppShell may read /health/ready; nothing on this screen fetches /api/ by itself (done criterion 4).
  fetchSpy = vi.fn(() => Promise.reject(new Error('offline in tests')));
  vi.stubGlobal('fetch', fetchSpy);
  download = vi.fn();
  navigate = vi.fn();
  replaceState = vi.spyOn(window.history, 'replaceState').mockImplementation(() => {});
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  delete (document as unknown as Record<string, unknown>).hidden;
  const own = fetchSpy.mock.calls.map((c) => String(c[0])).filter((url) => url.includes('/api/'));
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
  expect(own, 'no fetch of its own: every call goes through the ApiClient').toEqual([]);
});

function mount(server: FakeServer, branch = 'B1'): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <DevicesScreen
        api={server.api}
        branch={branch}
        newId={() => `req-${++ids}`}
        navigate={navigate as unknown as (url: string) => void}
        download={download as unknown as (name: string, blob: Blob) => void}
      />
    ),
    host,
  );
}

function textOf(el: Element | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

/** The innermost elements under `root` whose text is `text`. */
function byText(root: ParentNode, text: string): HTMLElement[] {
  const all = [...root.querySelectorAll<HTMLElement>('*')].filter((el) => textOf(el) === text);
  return all.filter((el) => ![...el.children].some((child) => textOf(child) === text));
}

function buttons(text: string, root: ParentNode = host): HTMLButtonElement[] {
  return [...root.querySelectorAll('button')].filter((b) => textOf(b) === text);
}

function button(text: string, root: ParentNode = host): HTMLButtonElement {
  const found = buttons(text, root);
  if (found.length !== 1) throw new Error(`expected one button ${text}, found ${found.length}`);
  return found[0]!;
}

/** The `<section>` holding the heading `heading`. */
function sectionOf(heading: string): HTMLElement {
  const h = byText(host, heading)[0];
  const s = h?.closest('section');
  if (!s) throw new Error(`no section holding ${heading}`);
  return s;
}

/** A label's own text, without the text of the control it wraps. */
function labelText(l: HTMLLabelElement): string {
  const copy = l.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('select, input, textarea').forEach((c) => c.remove());
  return textOf(copy);
}

function control<T extends HTMLElement>(label: string, root: ParentNode = host): T {
  const l = [...root.querySelectorAll('label')].find((x) => labelText(x) === label);
  if (!l) throw new Error(`no label ${label}`);
  const c = l.htmlFor ? document.getElementById(l.htmlFor) : l.querySelector('select, input, textarea');
  if (!c) throw new Error(`no control for ${label}`);
  return c as T;
}

function headerCells(table: Element): string[] {
  return [...table.querySelectorAll('th')].map(textOf);
}

function tableWith(headers: string[]): HTMLTableElement {
  const t = [...host.querySelectorAll('table')].find((x) => headerCells(x).join('|') === headers.join('|'));
  if (!t) throw new Error(`no table headed ${headers.join(' | ')}`);
  return t;
}

function bodyRows(table: Element): HTMLTableRowElement[] {
  return [...table.querySelectorAll('tr')].filter((tr) => tr.querySelector('td'));
}

const JOB_HEADERS = () => [th.colJobTime, th.colJobKind, th.colJobActor, th.colJobStatus, th.colJobActions];
const EVENT_HEADERS = () => [th.colTime, th.colActor, th.colEvent, th.colRef, th.colDetail];

function jobsTable(): HTMLTableElement {
  return tableWith(JOB_HEADERS());
}

function eventsTable(): HTMLTableElement {
  return tableWith(EVENT_HEADERS());
}

function jobRow(id: string): HTMLTableRowElement {
  const r = bodyRows(jobsTable()).find((tr) => textOf(tr.cells[0]?.querySelector('small')) === id);
  if (!r) throw new Error(`no job row ${id}`);
  return r;
}

function shownJobIds(): string[] {
  return bodyRows(jobsTable())
    .map((tr) => textOf(tr.cells[0]?.querySelector('small')))
    .filter((x) => x !== '');
}

function picker(): HTMLSelectElement {
  return control<HTMLSelectElement>(th.transaction);
}

function options(select: HTMLSelectElement): Array<[string, string]> {
  return [...select.options].map((o) => [o.value, textOf(o)]);
}

function choose(select: HTMLSelectElement, value: string): void {
  select.value = value;
  select.dispatchEvent(new Event('input', { bubbles: true }));
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function message(): HTMLElement {
  const m = host.querySelector<HTMLElement>('#message');
  if (!m) throw new Error('no #message');
  return m;
}

/** The value shown beside a metric's label (D6). */
function metric(label: string): string {
  const el = byText(host, label)[0];
  const box = el?.parentElement;
  if (!box) throw new Error(`no metric ${label}`);
  const labels = [th.metricThb, th.metricUsd, th.metricCompleted].filter((l) => byText(box, l).length > 0);
  expect(labels, `the box of ${label} holds only its own metric`).toEqual([label]);
  return textOf(box).replace(label, '').trim();
}

function stamp(iso: string, locale = 'th-TH'): string {
  return new Date(iso).toLocaleString(locale, { timeZone: 'Asia/Bangkok' });
}

async function loaded(jobs = B1_JOBS.length): Promise<void> {
  await vi.waitFor(() => expect(shownJobIds()).toHaveLength(jobs));
}

/** The screen's own action buttons (not the language switch of AppShell). */
function actionButtons(): HTMLButtonElement[] {
  return [
    button(th.refresh),
    button(th.print),
    button(th.drawer),
    button(th.drawerOpen),
    button(th.drawerClose),
    button(th.exportCsv),
    button(th.process, jobRow('J3')),
    button(th.retry, jobRow('J2')),
  ];
}

/** Waits until the action just sent is over: the views read again and the buttons free. */
async function settled(server: FakeServer, devicesReads: number): Promise<void> {
  await vi.waitFor(() => expect(server.readsOf('devices').length).toBeGreaterThan(devicesReads));
  await vi.waitFor(() => {
    for (const b of actionButtons()) expect(b.disabled, textOf(b)).toBe(false);
  });
}

/** Lets fake timers run what is due now, and the promises it starts, a few rounds over. */
async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await vi.advanceTimersByTimeAsync(0);
}

// ── reading ──────────────────────────────────────────────────────────────────

describe('DevicesScreen reading (#404 §7 Reading, V2, V5)', () => {
  it('reads the four views of #400 for the branch it is given: devices, and the first 100 jobs, events and transactions', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(3));
    expect(server.posts).toEqual([]);
    for (const name of VIEWS) {
      const first = server.readsOf(name)[0];
      expect(first, `${name} is read`).toBeDefined();
      expect(first!.branch, name).toBe('B1');
      expect(first!.before, name).toBeUndefined();
    }
    expect(server.readsOf('devices')[0]!.via).toBe('view');
    expect(server.readsOf('device_jobs')[0]).toMatchObject({ via: 'view', limit: 100 });
    expect(server.readsOf('device_events')[0]).toMatchObject({ via: 'view', limit: 100 });
    expect(server.readsOf('device_transactions')[0]!.limit).toBe(100);
    expect(server.reads.every((r) => (VIEWS as readonly string[]).includes(r.name)), 'no other view').toBe(true);
  });

  it('polls devices, jobs and events every 10 s with the last version as since; nothing earlier (D20, V5)', async () => {
    vi.useFakeTimers();
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await settle();
    expect(shownJobIds()).toEqual(['J3', 'J2', 'J1']);
    const before = server.counts();

    await vi.advanceTimersByTimeAsync(9_000);
    await settle();
    expect(server.counts(), 'nothing is read again within 9 s').toEqual(before);

    await vi.advanceTimersByTimeAsync(1_100);
    await settle();
    for (const name of ['devices', 'device_jobs', 'device_events']) {
      const reads = server.readsOf(name);
      expect(reads.length, name).toBe(before[name]! + 1);
      expect(reads.at(-1), name).toMatchObject({ via: 'view', branch: 'B1', since: 5 });
    }
    expect(server.readsOf('device_jobs').at(-1)!.limit).toBe(100);
    expect(server.readsOf('device_events').at(-1)!.limit).toBe(100);
  });

  it('a poll that answers changed shows the new rows', async () => {
    vi.useFakeTimers();
    const data = b1();
    const server = new FakeServer({ B1: data });
    mount(server, 'B1');
    await settle();
    expect(shownJobIds()).toEqual(['J3', 'J2', 'J1']);
    data.version = 6;
    data.jobs = [job('J4', 'PRINT_RECEIPT', 'TX-1', 'QUEUED'), ...B1_JOBS];
    await vi.advanceTimersByTimeAsync(10_100);
    await settle();
    expect(shownJobIds()).toEqual(['J4', 'J3', 'J2', 'J1']);
  });

  it('reads nothing while the page is hidden, and polls again once it is shown (V5)', async () => {
    vi.useFakeTimers();
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await settle();
    expect(shownJobIds()).toEqual(['J3', 'J2', 'J1']);
    const before = server.counts();
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(120_000);
    await settle();
    expect(server.counts()).toEqual(before);

    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(10_100);
    await settle();
    for (const name of ['devices', 'device_jobs', 'device_events']) {
      expect(server.readsOf(name).length, name).toBeGreaterThan(before[name]!);
    }
  });

  it("shows v2's page texts through its table, and every button to every role (D3-D5, D7-D9, D11-D13, D16, V1, V9)", async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    for (const key of [
      'heading',
      'eyebrow',
      'intro',
      'printerTag',
      'printerName',
      'printerText',
      'scanTag',
      'scanName',
      'scanText',
      'counterTag',
      'counterName',
      'counterText',
      'drawerNote',
      'requestEyebrow',
      'requestHeading',
      'eventsEyebrow',
      'eventsHeading',
      'eventsNote',
      'footer',
    ] as const) {
      expect(byText(host, th[key]).length, key).toBeGreaterThan(0);
    }
    expect(textOf(host)).toContain(th.counterNoteCount);
    expect(textOf(host)).toContain(th.counterNoteCheck);

    const brand = [...host.querySelectorAll('a')].find((a) => a.getAttribute('href') === '/devices');
    expect(textOf(brand)).toContain('XCHANGE');

    const m = message();
    expect(m.getAttribute('role')).toBe('status');
    expect(m.getAttribute('aria-live')).toBe('polite');
    expect(textOf(m)).toBe('');

    const reason = control<HTMLInputElement>(th.reason);
    expect(reason.maxLength).toBe(200);
    expect(reason.placeholder).toBe(th.reasonPlaceholder);

    expect(actionButtons()).toHaveLength(8);
    for (const b of actionButtons()) expect(b.disabled, textOf(b)).toBe(false);
  });

  it('the nav links carry the branch (D3)', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    const link = (text: string) => [...host.querySelectorAll('a')].find((a) => textOf(a) === text);
    expect(link(th.adminLink)?.getAttribute('href')).toBe('/?branch=B1');
    expect(link(th.deskLink)?.getAttribute('href')).toBe('/desk?branch=B1');
  });

  it('has no branch select, and its links follow the branchId the devices view answers (D4, §9; SB-2 §14.4)', async () => {
    const server = new FakeServer({ B1: b1(), B2: branchData('B2') });
    server.pinned = 'B1'; // a staff member of B1 naming B2 is answered for B1
    mount(server, 'B2');
    await loaded();
    expect(() => control<HTMLSelectElement>(th.branch)).toThrow();
    const branchSelects = [...host.querySelectorAll('select')].filter((s) =>
      [...s.options].some((o) => BRANCHES.some((b) => b.id === o.value)),
    );
    expect(branchSelects).toEqual([]);
    const link = (text: string) => [...host.querySelectorAll('a')].find((a) => textOf(a) === text);
    await vi.waitFor(() => expect(link(th.adminLink)?.getAttribute('href')).toBe('/?branch=B1'));
  });
});

// ── money and the page text that comes from the view (W7, V3, V6, V13) ─────

describe('DevicesScreen figures and current-behaviour text (#404 V3, V6)', () => {
  it("shows the view's balances formatted and its count, never a figure of its own (D6, V3, §9)", async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(metric(th.metricThb)).toMatch(/^90,071,992,547,409\.93(\s|$)/));
    expect(metric(th.metricUsd)).toMatch(/^33,250\.00(\s|$)/);
    expect(metric(th.metricCompleted)).toBe('7');
  });

  it("the picker lists the view's rows as `id · type amount currency`, the amount formatted as sent (D11, V8)", async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() =>
      expect(options(picker())).toEqual([
        ['', th.choose],
        ['TX-2', 'TX-2 · BUY 1,000.00 USD'],
        ['TX-1', 'TX-1 · SELL 250.50 EUR'],
      ]),
    );
  });

  it('before the first load the picker says there is nothing yet; an empty first page leaves only the empty choice (D11)', async () => {
    const waiting = new FakeServer({ B1: b1() });
    waiting.hold.add('device_transactions');
    mount(waiting, 'B1');
    await loaded();
    expect(options(picker())).toEqual([['', th.noTransactions]]);
    dispose?.();
    document.body.innerHTML = '';

    const empty = new FakeServer({ B1: b1({ transactions: [] }) });
    mount(empty, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toEqual([['', th.choose]]));
  });

  it("a simulated printer: the printer banner, and the drawer card says the drawer is not connected; v2's simulator texts are gone (V6)", async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(byText(host, th.bannerSimulated)).toHaveLength(1));
    expect(byText(host, th.drawerNone)).toHaveLength(1);
    const page = textOf(host);
    for (const retired of [
      'งานอุปกรณ์ยังเป็นการจำลอง · ไม่ส่งคำสั่งไปเครื่องพิมพ์หรือลิ้นชักจริง',
      'เซนเซอร์ยังไม่ผ่านการทดสอบ',
      'ลิ้นชัก one',
      'ผลจริงที่ทดสอบ',
      'ข้อมูลล่าสุด 200 เหตุการณ์',
    ]) {
      expect(page, retired).not.toContain(retired);
    }
  });

  it("a network printer: its banner, and the drawer card names the view's printer model (V6)", async () => {
    const server = new FakeServer({
      B1: b1({ devices: overview('B1', { printerKind: 'NETWORK', printerModel: 'EPSON TM-T82X', hasDrawer: true }) }),
    });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(byText(host, th.bannerNetwork)).toHaveLength(1));
    expect(byText(host, fill(th.drawerNetwork, { printerModel: 'EPSON TM-T82X' }))).toHaveLength(1);
  });

  it('an active device agent: the drawer card says the command goes to the agent (V6)', async () => {
    const server = new FakeServer({
      B1: b1({ devices: overview('B1', { printerKind: 'NETWORK', drawerAgentActive: true, hasDrawer: false }) }),
    });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(byText(host, th.drawerAgent)).toHaveLength(1));
  });

  it('the printer switch shows printerOnline and its state line (D7, V13)', async () => {
    const online = new FakeServer({ B1: b1() });
    mount(online, 'B1');
    await loaded();
    const box = control<HTMLInputElement>(th.printerOnline);
    expect(box.type).toBe('checkbox');
    await vi.waitFor(() => expect(box.checked).toBe(true));
    expect(byText(host, th.printerReady)).toHaveLength(1);
    expect(byText(host, th.printerOffline)).toEqual([]);
    dispose?.();
    document.body.innerHTML = '';

    const offline = new FakeServer({ B1: b1({ devices: overview('B1', { printerOnline: false }) }) });
    mount(offline, 'B1');
    await loaded();
    await vi.waitFor(() => expect(control<HTMLInputElement>(th.printerOnline).checked).toBe(false));
    expect(byText(host, th.printerOffline)).toHaveLength(1);
    expect(byText(host, th.printerReady)).toEqual([]);
  });
});

// ── the tables (D12, D13, D14, D15, D21, V7, V10) ─────────────────────────────

describe('DevicesScreen tables (#404 D12-D15, D21)', () => {
  it("jobs: time and id, kind label and ref, actor, status label, and each status's actions (D12, §9)", async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    expect(shownJobIds(), "the view's order").toEqual(['J3', 'J2', 'J1']);

    for (const j of B1_JOBS) {
      const cells = [...jobRow(j.id).cells];
      expect(cells, j.id).toHaveLength(5);
      expect(textOf(cells[0]).startsWith(stamp(j.created)), `${j.id} time`).toBe(true);
      expect(textOf(cells[0]!.querySelector('small')), `${j.id} id`).toBe(j.id);
      expect(textOf(cells[1]).startsWith(KIND_THAI[j.kind]!), `${j.id} kind`).toBe(true);
      expect(textOf(cells[1]!.querySelector('small')), `${j.id} ref`).toBe(j.ref || th.noRef);
      expect(textOf(cells[2]), `${j.id} actor`).toBe(j.actor);
      expect(textOf(cells[3]), `${j.id} status`).toBe(STATUS_THAI[j.status]);
    }

    const actions = (id: string) => [...jobRow(id).cells[4]!.querySelectorAll('button')].map(textOf);
    expect(actions('J3')).toEqual([th.process]);
    expect(actions('J2')).toEqual([th.retry]);
    expect(actions('J1')).toEqual([]);
  });

  it('a print job links its 80 mm sample for the branch and ref, in a new tab; other jobs do not (D12, V10)', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    const links = (id: string) => [...jobRow(id).cells[4]!.querySelectorAll('a')];
    const sample = links('J3');
    expect(sample.map(textOf)).toEqual([th.receipt]);
    expect(sample[0]!.getAttribute('href')).toBe('/devices/receipt?branch=B1&ref=TX-2');
    expect(sample[0]!.getAttribute('target')).toBe('_blank');
    expect(sample[0]!.getAttribute('rel') ?? '').toContain('noopener');
    expect(links('J2')).toEqual([]);
    expect(links('J1')).toEqual([]);
  });

  it("every status label: v2's Thai and V7's for v3's own statuses", async () => {
    const jobs = Object.keys(STATUS_THAI).map((s, i) => job(`S${i}`, 'PRINT_RECEIPT', `TX-${i}`, s));
    const server = new FakeServer({ B1: b1({ jobs }) });
    mount(server, 'B1');
    await loaded(jobs.length);
    for (const [i, s] of Object.keys(STATUS_THAI).entries()) {
      expect(textOf(jobRow(`S${i}`).cells[3]), s).toBe(STATUS_THAI[s]);
    }
  });

  it("events: time, actor, kind label (or the raw kind), ref or —, detail as sent (D13, §9)", async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    const rows = bodyRows(eventsTable());
    expect(rows).toHaveLength(3);
    const expected = [
      [stamp('2026-10-08T03:04:05.000Z'), 'front1', KIND_THAI.PRINT_RECEIPT!, 'TX-2', '{"jobId":"J3"}'],
      [stamp('2026-10-08T03:00:00.000Z'), 'backend1', 'SOMETHING_NEW', '—', '{"a":1}'],
      [stamp('2026-10-08T02:58:00.000Z'), 'agent', KIND_THAI.DRAWER_OPENED!, 'TX-1', '{"jobId":"J2"}'],
    ];
    expect(rows.map((tr) => [...tr.cells].map(textOf))).toEqual(expected);
    for (const [i, tr] of rows.entries()) {
      expect(textOf(tr.cells[3]!.querySelector('small')), `row ${i} ref`).toBe(expected[i]![3]);
      expect(textOf(tr.cells[4]!.querySelector('code')), `row ${i} detail`).toBe(expected[i]![4]);
    }
  });

  it("every kind label: v2's Thai, V7's for v3's event kinds", async () => {
    const events = Object.keys(KIND_THAI).map((k, i) => event(String(100 - i), k, `R${i}`));
    const server = new FakeServer({ B1: b1({ events }) });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(events.length));
    expect(bodyRows(eventsTable()).map((tr) => textOf(tr.cells[2]))).toEqual(Object.values(KIND_THAI));
  });

  it("empty tables say v2's empty lines (D12, D13)", async () => {
    const server = new FakeServer({ B1: b1({ jobs: [], events: [] }) });
    mount(server, 'B1');
    await vi.waitFor(() => expect(byText(jobsTable(), th.noJobs)).toHaveLength(1));
    expect(bodyRows(jobsTable())).toHaveLength(1);
    expect(byText(eventsTable(), th.noEvents)).toHaveLength(1);
    expect(bodyRows(eventsTable())).toHaveLength(1);
  });

  it('every cell carries its column title as data-label, for phones (D21)', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(3));
    for (const [table, headers] of [
      [jobsTable(), JOB_HEADERS()],
      [eventsTable(), EVENT_HEADERS()],
    ] as const) {
      for (const tr of bodyRows(table)) {
        expect([...tr.cells].map((td) => td.getAttribute('data-label'))).toEqual(headers);
      }
    }
  });
});

// ── writing (§7 Writing, D18, V1, V2, V5) ────────────────────────────────────

describe('DevicesScreen actions (#404 §7 Writing, §8, §9)', () => {
  it('เพิ่มงานพิมพ์ queues a receipt for the chosen transaction, then reads all four views again and shows the job', async () => {
    const data = b1();
    const server = new FakeServer({ B1: data });
    server.onPost = async () => {
      data.version = 6;
      data.jobs = [job('J4', 'PRINT_RECEIPT', 'TX-1', 'QUEUED'), ...B1_JOBS];
      return { saved: true };
    };
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(3));
    choose(picker(), 'TX-1');
    const before = server.counts();
    button(th.print).click();

    await vi.waitFor(() => expect(shownJobIds()).toEqual(['J4', 'J3', 'J2', 'J1']));
    expect(server.posts).toEqual([
      {
        path: '/api/devices',
        body: { action: 'queue', payload: { kind: 'PRINT_RECEIPT', ref: 'TX-1', requestId: 'req-1', branchId: 'B1' } },
      },
    ]);
    for (const name of VIEWS) {
      expect(server.readsOf(name).length, `${name} is read again`).toBeGreaterThan(before[name]!);
    }
  });

  it('เพิ่มคำสั่งเปิดลิ้นชัก queues a drawer request with the reason and a new request id each time', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(3));
    choose(picker(), 'TX-2');
    type(control<HTMLInputElement>(th.reason), 'ส่งมอบเงินให้ลูกค้า');
    let reads = server.readsOf('devices').length;
    button(th.drawer).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    await settled(server, reads);
    reads = server.readsOf('devices').length;
    button(th.drawer).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(2));
    const payload = { kind: 'DRAWER_REQUEST', ref: 'TX-2', reason: 'ส่งมอบเงินให้ลูกค้า', branchId: 'B1' };
    expect(server.posts).toEqual([
      { path: '/api/devices', body: { action: 'queue', payload: { ...payload, requestId: 'req-1' } } },
      { path: '/api/devices', body: { action: 'queue', payload: { ...payload, requestId: 'req-2' } } },
    ]);
  });

  it('จำลองดำเนินการ processes a queued job and ลองใหม่ retries a failed one, by id', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    const reads = server.readsOf('devices').length;
    button(th.process, jobRow('J3')).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    await settled(server, reads);
    button(th.retry, jobRow('J2')).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(2));
    expect(server.posts).toEqual([
      { path: '/api/devices', body: { action: 'process', payload: { id: 'J3', branchId: 'B1' } } },
      { path: '/api/devices', body: { action: 'retry', payload: { id: 'J2', branchId: 'B1' } } },
    ]);
  });

  it('the printer switch sends printerOnline with its new state (D7)', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    const box = control<HTMLInputElement>(th.printerOnline);
    await vi.waitFor(() => expect(box.checked).toBe(true));
    box.click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    expect(server.posts[0]).toEqual({
      path: '/api/devices',
      body: { action: 'printerOnline', payload: { online: false, branchId: 'B1' } },
    });
  });

  it('the two observation buttons record OPEN and CLOSED against the chosen transaction (D10)', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(3));
    choose(picker(), 'TX-2');
    const reads = server.readsOf('devices').length;
    button(th.drawerOpen).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    await settled(server, reads);
    button(th.drawerClose).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(2));
    expect(server.posts).toEqual([
      { path: '/api/devices', body: { action: 'drawerObservation', payload: { state: 'OPEN', ref: 'TX-2', branchId: 'B1' } } },
      { path: '/api/devices', body: { action: 'drawerObservation', payload: { state: 'CLOSED', ref: 'TX-2', branchId: 'B1' } } },
    ]);
  });

  it('อัปเดตข้อมูล reads all four views again and sends nothing (D11)', async () => {
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(server.readsOf('device_transactions').length).toBeGreaterThan(0));
    const before = server.counts();
    button(th.refresh).click();
    for (const name of VIEWS) {
      await vi.waitFor(() => expect(server.readsOf(name).length, name).toBeGreaterThan(before[name]!));
    }
    expect(server.posts).toEqual([]);
  });

  it("a refusal shows the server's sentence in #message and frees the buttons; the next action clears it (§9, D18, V1)", async () => {
    const server = new FakeServer({ B1: b1() });
    server.onPost = () => Promise.reject(new ApiError({ code: 'refused', message: REFUSED, status: 400 }));
    mount(server, 'B1');
    await loaded();
    button(th.print).click();
    await vi.waitFor(() => expect(textOf(message())).toBe(errorText('refused', REFUSED)));
    await vi.waitFor(() => {
      for (const b of actionButtons()) expect(b.disabled, textOf(b)).toBe(false);
    });

    const pending = deferred<unknown>();
    server.onPost = () => pending.promise;
    button(th.drawer).click();
    await vi.waitFor(() => expect(textOf(message())).toBe(''));
    pending.resolve({ saved: true });
  });

  it('while an action is pending every button is disabled and a second click sends nothing (§9, D18)', async () => {
    const pending = deferred<unknown>();
    const server = new FakeServer({ B1: b1() });
    server.onPost = () => pending.promise;
    mount(server, 'B1');
    await loaded();
    button(th.print).click();
    await vi.waitFor(() => {
      for (const b of actionButtons()) expect(b.disabled, textOf(b)).toBe(true);
    });
    button(th.print).click();
    button(th.drawer).click();
    button(th.process, jobRow('J3')).click();
    expect(server.posts).toHaveLength(1);

    pending.resolve({ saved: true });
    await vi.waitFor(() => {
      for (const b of actionButtons()) expect(b.disabled, textOf(b)).toBe(false);
    });
    expect(server.posts).toHaveLength(1);
  });

  it('the chosen transaction is kept while the picker still lists it, and dropped, also from what is sent, when it does not (§9)', async () => {
    const data = b1();
    const server = new FakeServer({ B1: data });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(3));
    choose(picker(), 'TX-1');

    let reads = server.readsOf('device_transactions').length;
    const devicesReads = server.readsOf('devices').length;
    button(th.refresh).click();
    await vi.waitFor(() => expect(server.readsOf('device_transactions').length).toBeGreaterThan(reads));
    await settled(server, devicesReads);
    expect(picker().value).toBe('TX-1');

    data.version = 6;
    data.transactions = [tx('TX-3', 'BUY', '20.00', 'JPY'), B1_TXS[0]!];
    reads = server.readsOf('device_transactions').length;
    button(th.refresh).click();
    await vi.waitFor(() => expect(server.readsOf('device_transactions').length).toBeGreaterThan(reads));
    await vi.waitFor(() =>
      expect(options(picker())).toEqual([
        ['', th.choose],
        ['TX-3', 'TX-3 · BUY 20.00 JPY'],
        ['TX-2', 'TX-2 · BUY 1,000.00 USD'],
      ]),
    );
    expect(picker().value).toBe('');

    // what is sent is what is shown: the dropped row is not sent either
    await settled(server, server.readsOf('devices').length - 1);
    button(th.print).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    expect((server.posts[0]!.body as { payload: { ref: string } }).payload.ref).toBe('');
  });
});

// ── branches (D19, §9) ───────────────────────────────────────────────────────

describe('DevicesScreen one branch (#404 D19; SB-2 §14.4)', () => {
  it('reads only the branch it is given, even after polls, and names it in every write', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const B2_JOBS = [job('J9', 'DRAWER_REQUEST', 'TX-9', 'QUEUED', 'front2')];
    const server = new FakeServer({
      B1: b1(),
      B2: branchData('B2', {
        version: 5,
        devices: overview('B2', { balanceThb: '10.00', balanceUsd: '0.00', completedCount: 1 }),
        jobs: B2_JOBS,
        events: [event('7', 'PRINT_RECEIPT', 'TX-9', '{}', 'front2')],
        transactions: [tx('TX-9', 'BUY', '5.00', 'USD')],
      }),
    });
    mount(server, 'B2');
    await vi.waitFor(() => expect(shownJobIds()).toEqual(['J9']));
    await vi.waitFor(() => expect(metric(th.metricThb)).toMatch(/^10\.00(\s|$)/));

    await vi.advanceTimersByTimeAsync(30_000);
    expect(server.reads.length).toBeGreaterThan(0);
    expect(server.reads.filter((r) => r.branch !== 'B2')).toEqual([]);
    expect(replaceState.mock.calls.map((c) => String(c[2])).filter((u) => u.includes('branch='))).toEqual([]);

    button(th.process, jobRow('J9')).click();
    await vi.waitFor(() => expect(server.posts).toHaveLength(1));
    expect(server.posts[0]).toEqual({
      path: '/api/devices',
      body: { action: 'process', payload: { id: 'J9', branchId: 'B2' } },
    });
  });
});

// ── older pages and the CSV (§7, V8, V9, D17) ────────────────────────────────

describe('DevicesScreen older pages and CSV (#404 V8, V9, D17)', () => {
  function events(from: number, to: number): DeviceEventRow[] {
    const out: DeviceEventRow[] = [];
    for (let s = from; s >= to; s--) {
      out.push(event(String(s), s % 2 === 0 ? 'PRINT_RECEIPT' : 'DRAWER_REQUEST', `TX-${s}`, `{"n":${s}}`, `actor${s}`));
    }
    return out;
  }

  function txs(from: number, to: number): DeviceTransactionRow[] {
    const out: DeviceTransactionRow[] = [];
    for (let i = from; i >= to; i--) out.push(tx(`TX-${i}`, 'BUY', '1.00', 'USD'));
    return out;
  }

  it('โหลดเพิ่ม under the events reads the next 100 before the last seq shown and adds them below', async () => {
    const first = events(200, 101);
    const older = events(100, 99);
    const server = new FakeServer({ B1: b1({ events: first, olderEvents: { '101': older } }) });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));
    button(th.loadMore, sectionOf(th.eventsHeading)).click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(102));
    const paged = server.reads.filter((r) => r.before !== undefined);
    expect(paged).toEqual([{ via: 'page', name: 'device_events', branch: 'B1', before: '101', limit: 100 }]);
    expect(bodyRows(eventsTable()).map((tr) => textOf(tr.cells[1]))).toEqual(
      [...first, ...older].map((e) => e.actor),
    );
  });

  it('โหลดเพิ่ม under the picker reads the next 100 transactions before the last id listed and adds them below', async () => {
    const first = txs(200, 101);
    const server = new FakeServer({ B1: b1({ transactions: first, olderTransactions: { 'TX-101': txs(100, 100) } }) });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(101));
    button(th.loadMore, sectionOf(th.requestHeading)).click();
    await vi.waitFor(() => expect(options(picker())).toHaveLength(102));
    const paged = server.reads.filter((r) => r.before !== undefined);
    expect(paged).toEqual([{ via: 'page', name: 'device_transactions', branch: 'B1', before: 'TX-101', limit: 100 }]);
    expect(options(picker()).at(-1)).toEqual(['TX-100', 'TX-100 · BUY 1.00 USD']);
  });

  it('ดาวน์โหลดบันทึก CSV hands over XCHANGE-DEVICE-EVENTS.csv of exactly the rows shown, and sends nothing', async () => {
    const first = events(200, 101);
    const older = events(100, 99);
    const server = new FakeServer({ B1: b1({ events: first, olderEvents: { '101': older } }) });
    mount(server, 'B1');
    await loaded();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));
    button(th.loadMore, sectionOf(th.eventsHeading)).click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(102));

    const before = server.reads.length;
    button(th.exportCsv).click();
    await vi.waitFor(() => expect(download).toHaveBeenCalledTimes(1));
    const [name, blob] = download.mock.calls[0]! as [string, Blob];
    expect(name).toBe('XCHANGE-DEVICE-EVENTS.csv');
    expect(blob.type).toBe('text/csv;charset=utf-8');
    // ignoreBOM keeps the BOM in the decoded text, so it is compared too
    const text = new TextDecoder('utf-8', { ignoreBOM: true }).decode(await blob.arrayBuffer());
    expect(text.startsWith('\uFEFF')).toBe(true);
    expect(text).toBe(eventsCsv([...first, ...older]));
    expect(server.posts).toEqual([]);
    expect(server.reads.length, 'the CSV is the rows shown, not a new read').toBe(before);
  });
});

// ── language (W10) ───────────────────────────────────────────────────────────

describe('DevicesScreen language (#404 W10)', () => {
  it('every text follows the language: English when chosen, Thai again after the switch', async () => {
    setLang('en');
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await vi.waitFor(() => expect(tableWith([en.colJobTime, en.colJobKind, en.colJobActor, en.colJobStatus, en.colJobActions])).toBeDefined());
    expect(byText(host, en.heading).length).toBeGreaterThan(0);
    expect(buttons(en.print)).toHaveLength(1);
    expect(buttons(th.print)).toEqual([]);
    await vi.waitFor(() => expect(byText(host, en.bannerSimulated)).toHaveLength(1));
    const jobs = tableWith([en.colJobTime, en.colJobKind, en.colJobActor, en.colJobStatus, en.colJobActions]);
    await vi.waitFor(() => expect(bodyRows(jobs)).toHaveLength(3));
    for (const tr of bodyRows(jobs)) {
      expect(THAI.test(textOf(tr.cells[1])), `kind ${textOf(tr.cells[1])}`).toBe(false);
      expect(THAI.test(textOf(tr.cells[3])), `status ${textOf(tr.cells[3])}`).toBe(false);
      expect(textOf(tr.cells[3]).length).toBeGreaterThan(0);
    }

    setLang('th');
    await vi.waitFor(() => expect(buttons(th.print)).toHaveLength(1));
    expect(buttons(en.print)).toEqual([]);
    await loaded();
    expect(textOf(jobRow('J3').cells[3])).toBe(STATUS_THAI.QUEUED);
  });
});
