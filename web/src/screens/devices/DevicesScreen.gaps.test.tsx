import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  ApiError,
  type ApiClient,
  type PageParams,
  type ViewAnswer,
  type ViewParams,
} from '../../shared/api/client';
import {
  type DeviceEventRow,
  type DeviceJobRow,
  type DeviceTransactionRow,
  type DevicesOverview,
  makeDeviceBranch,
  makeDeviceEventRow,
  makeDeviceJobRow,
  makeDeviceTransactionRow,
  makeDevicesOverview,
} from '../../shared/testing/fixtures';
import { errorText, setLang } from '../../shared/i18n';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

const REFUSED = 'บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้';

const BRANCHES = [
  makeDeviceBranch({ id: 'MAIN', name: 'สำนักงานใหญ่' }),
  makeDeviceBranch({ id: 'B1', name: 'สาขาหนึ่ง' }),
  makeDeviceBranch({ id: 'B2', name: 'สาขาสอง' }),
];

function overview(branchId: string, over: Partial<DevicesOverview> = {}) {
  return makeDevicesOverview({
    branchId,
    branches: BRANCHES,
    printerOnline: true,
    printerKind: 'SIMULATED',
    printerModel: 'NEOCAL NTP-8220BT',
    hasDrawer: null,
    drawerAgentActive: false,
    balanceThb: '90071992547409.93',
    balanceUsd: '33250.00',
    completedCount: 7,
    ...over,
  });
}

function job(id: string, kind: string, ref: string, status: string, actor = 'front1', created = '2026-10-08T03:04:05.000Z') {
  return makeDeviceJobRow({ id, kind, ref, status, actor, created });
}

function event(seq: string, kind: string, ref = '', detail = '{}', actor = 'backend1', at = '2026-10-08T02:00:00.000Z') {
  return makeDeviceEventRow({ seq, kind, ref, at, actor, detail });
}

function tx(id: string, type: string, amount: string, currency: string) {
  return makeDeviceTransactionRow({ id, type, amount, currency });
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

const B1_TXS = [tx('TX-2', 'BUY', '1000.00', 'USD'), tx('TX-1', 'SELL', '250.50', 'EUR')];

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

interface BranchData {
  version: number;
  devices: DevicesOverview;
  jobs: DeviceJobRow[];
  events: DeviceEventRow[];
  transactions: DeviceTransactionRow[];
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
  pinned: string | undefined;
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

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let fetchSpy: ReturnType<typeof vi.fn>;
let download: ReturnType<typeof vi.fn>;
let navigate: ReturnType<typeof vi.fn>;
let ids: number;

beforeEach(() => {
  setLang('th');
  ids = 0;
  fetchSpy = vi.fn(() => Promise.reject(new Error('offline in tests')));
  vi.stubGlobal('fetch', fetchSpy);
  download = vi.fn();
  navigate = vi.fn();
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

function buttons(text: string, root: ParentNode = host): HTMLButtonElement[] {
  return [...root.querySelectorAll('button')].filter((b) => textOf(b) === text);
}

function button(text: string, root: ParentNode = host): HTMLButtonElement {
  const found = buttons(text, root);
  if (found.length !== 1) throw new Error(`expected one button ${text}, found ${found.length}`);
  return found[0]!;
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

function jobsTable(): HTMLTableElement {
  return tableWith(JOB_HEADERS());
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

function message(): HTMLElement {
  const m = host.querySelector<HTMLElement>('#message');
  if (!m) throw new Error('no #message');
  return m;
}

async function loaded(jobs = B1_JOBS.length): Promise<void> {
  await vi.waitFor(() => expect(shownJobIds()).toHaveLength(jobs));
}

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

async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await vi.advanceTimersByTimeAsync(0);
}

describe('DevicesScreen gaps (#488)', () => {
  it('(a) a refused action reads no view (§1 (a), §3 (a), §8 (a))', async () => {
    vi.useFakeTimers();
    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');
    await settle();
    await loaded();

    server.onPost = () => Promise.reject(new ApiError({ code: 'refused', message: REFUSED, status: 400 }));
    const before = server.counts();

    button(th.print).click();
    await settle();
    expect(textOf(message())).toBe(errorText('refused', REFUSED));
    for (const b of actionButtons()) {
      expect(b.disabled, textOf(b)).toBe(false);
    }
    await settle();

    expect(server.posts).toHaveLength(1);
    expect(server.counts(), 'counts equal before the click for all four views').toEqual(before);

    // positive control: a successful action re-reads the views
    server.onPost = async () => ({ saved: true });
    button(th.print).click();
    await settle();
    for (const b of actionButtons()) {
      expect(b.disabled, textOf(b)).toBe(false);
    }
    await settle();

    expect(server.posts).toHaveLength(2);
    for (const name of VIEWS) {
      expect(server.readsOf(name).length, `${name} re-read on success`).toBeGreaterThan(before[name]!);
    }
  });

  it('(b) hidden at mount, then shown: polls every 10 s (§1 (b), §3 (b), §8 (b))', async () => {
    vi.useFakeTimers();
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });

    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');

    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    for (const name of ['devices', 'device_jobs', 'device_events']) {
      expect(server.readsOf(name), `${name} not read while hidden`).toHaveLength(0);
    }

    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
    document.dispatchEvent(new Event('visibilitychange'));
    await settle();

    for (const name of ['devices', 'device_jobs', 'device_events']) {
      expect(server.readsOf(name), `${name} read once on shown`).toHaveLength(1);
    }

    await vi.advanceTimersByTimeAsync(9_000);
    await settle();
    for (const name of ['devices', 'device_jobs', 'device_events']) {
      expect(server.readsOf(name), `${name} not read again within 9 s`).toHaveLength(1);
    }

    await vi.advanceTimersByTimeAsync(1_100);
    await settle();
    for (const name of ['devices', 'device_jobs', 'device_events']) {
      const reads = server.readsOf(name);
      expect(reads, `${name} read exactly twice at 10.1 s`).toHaveLength(2);
      expect(reads[1], `${name} second read carries since and branch`).toMatchObject({
        via: 'view',
        branch: 'B1',
        since: 5,
      });
    }
    expect(server.readsOf('device_jobs')[1]).toMatchObject({ limit: 100 });
    expect(server.readsOf('device_events')[1]).toMatchObject({ limit: 100 });
  });
});
