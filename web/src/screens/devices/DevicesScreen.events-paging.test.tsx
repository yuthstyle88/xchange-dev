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
  makeDevicesOverview,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { eventsCsv } from './csv';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

const BRANCHES = [
  makeDeviceBranch({ id: 'MAIN', name: 'สำนักงานใหญ่' }),
  makeDeviceBranch({ id: 'B1', name: 'สาขาหนึ่ง' }),
];

function overview(branchId: string) {
  return makeDevicesOverview({
    branchId,
    branches: BRANCHES,
    printerOnline: true,
    printerKind: 'SIMULATED',
    printerModel: 'NEOCAL NTP-8220BT',
    hasDrawer: null,
    drawerAgentActive: false,
    balanceThb: '1000.00',
    balanceUsd: '100.00',
    completedCount: 1,
  });
}

function event(seq: string, kind: string, ref = '', detail = '{}', actor = 'backend1', at = '2026-10-08T02:00:00.000Z') {
  return makeDeviceEventRow({ seq, kind, ref, at, actor, detail });
}

function events(from: number, to: number) {
  const out = [];
  for (let s = from; s >= to; s--) {
    out.push(event(String(s), s % 2 === 0 ? 'PRINT_RECEIPT' : 'DRAWER_REQUEST', `TX-${s}`, `{"n":${s}}`, `actor${s}`));
  }
  return out;
}

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

interface BranchData {
  version: number;
  devices: DevicesOverview;
  jobs: DeviceJobRow[];
  events: DeviceEventRow[];
  transactions: DeviceTransactionRow[];
  olderEvents: Record<string, DeviceEventRow[]>;
}

interface Read {
  via: 'view' | 'page';
  name: string;
  branch?: string;
  since?: number;
  before?: string;
  limit?: number;
}

function branchData(id: string, over: Partial<BranchData> = {}): BranchData {
  return {
    version: 5,
    devices: overview(id),
    jobs: [],
    events: [],
    transactions: [],
    olderEvents: {},
    ...over,
  };
}

class FakeServer {
  readonly reads: Read[] = [];
  readonly branches: Record<string, BranchData>;
  readonly api: Api;

  constructor(branches: Record<string, BranchData>) {
    this.branches = branches;
    const server = this;
    this.api = {
      view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
        server.reads.push({ via: 'view', name, ...(q ?? {}) });
        const b = server.branches[q?.branch ?? 'MAIN'];
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
        const b = server.branches[q?.branch ?? 'MAIN'];
        if (!b) return Promise.reject(new ApiError({ code: 'invalidRequest', message: 'Invalid branch', status: 400 }));
        let rows: unknown[] | undefined;
        if (q?.before === undefined) {
          rows = server.firstPage(b, name) as unknown[] | undefined;
        } else if (name === 'device_events') {
          rows = b.olderEvents[q.before] ?? [];
        } else {
          rows = [];
        }
        return Promise.resolve(structuredClone(rows ?? []) as R[]);
      },
      post<B, R>(_path: string, _body: B): Promise<R> {
        return Promise.resolve({ saved: true } as unknown as R);
      },
    };
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
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let download: ReturnType<typeof vi.fn>;

beforeEach(() => {
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
  download = vi.fn();
  vi.spyOn(window.history, 'replaceState').mockImplementation(() => {});
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

function mount(server: FakeServer, branch = 'B1'): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <DevicesScreen
        api={server.api}
        branch={branch}
        download={download as unknown as (name: string, blob: Blob) => void}
      />
    ),
    host,
  );
}

function textOf(el: Element | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

function byText(root: ParentNode, text: string): HTMLElement[] {
  const all = [...root.querySelectorAll<HTMLElement>('*')].filter((el) => textOf(el) === text);
  return all.filter((el) => ![...el.children].some((child) => textOf(child) === text));
}

function button(text: string, root: ParentNode = host): HTMLButtonElement {
  const found = [...root.querySelectorAll('button')].filter((b) => textOf(b) === text);
  if (found.length !== 1) throw new Error(`expected one button ${text}, found ${found.length}`);
  return found[0]!;
}

function sectionOf(heading: string): HTMLElement {
  const h = byText(host, heading)[0];
  const s = h?.closest('section');
  if (!s) throw new Error(`no section holding ${heading}`);
  return s;
}

function eventsTable(): HTMLTableElement {
  const headers = [th.colTime, th.colActor, th.colEvent, th.colRef, th.colDetail].join('|');
  const t = [...host.querySelectorAll('table')].find(
    (x) => [...x.querySelectorAll('th')].map(textOf).join('|') === headers,
  );
  if (!t) throw new Error('no events table');
  return t;
}

function bodyRows(table: Element): HTMLTableRowElement[] {
  return [...table.querySelectorAll('tr')].filter((tr) => tr.querySelector('td') && !tr.querySelector('td.empty'));
}

function shownEventActors(): string[] {
  return bodyRows(eventsTable()).map((tr) => textOf(tr.cells[1]));
}

describe('DevicesScreen events paging after new events arrive (#2595)', () => {
  it('Page 200..101 + older 100..1, then a poll returns 201..102: no seq is missing between the shown rows (older pages dropped; Load more available)', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const first = events(200, 101);
    const olderFrom101 = events(100, 1);
    const newer = events(201, 102);
    const olderFrom102 = events(101, 2);

    const server = new FakeServer({
      B1: branchData('B1', {
        version: 5,
        events: first,
        olderEvents: {
          '101': olderFrom101,
          '102': olderFrom102,
        },
      }),
    });

    mount(server, 'B1');
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));

    button(th.loadMore, sectionOf(th.eventsHeading)).click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(200));

    // Edge case: a poll answer identical to the current page keeps the loaded older pages
    server.branches.B1!.version = 6;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(bodyRows(eventsTable())).toHaveLength(200);
    expect(shownEventActors()).toEqual([...first, ...olderFrom101].map((e) => e.actor));

    server.branches.B1!.events = newer;
    server.branches.B1!.version = 7;
    await vi.advanceTimersByTimeAsync(10_000);

    await vi.waitFor(() => expect(shownEventActors()[0]).toBe('actor201'));
    expect(shownEventActors()).toEqual(newer.map((e) => e.actor));

    button(th.loadMore, sectionOf(th.eventsHeading)).click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(200));
    expect(shownEventActors()).toEqual([...newer, ...olderFrom102].map((e) => e.actor));
  });

  it('CSV after a poll changes the newest page while older pages were loaded equals the rows shown without a gap', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const first = events(200, 101);
    const olderFrom101 = events(100, 1);
    const newer = events(201, 102);

    const server = new FakeServer({
      B1: branchData('B1', {
        version: 5,
        events: first,
        olderEvents: {
          '101': olderFrom101,
        },
      }),
    });

    mount(server, 'B1');
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));

    button(th.loadMore, sectionOf(th.eventsHeading)).click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(200));

    server.branches.B1!.events = newer;
    server.branches.B1!.version = 6;
    await vi.advanceTimersByTimeAsync(10_000);

    await vi.waitFor(() => expect(shownEventActors()[0]).toBe('actor201'));

    button(th.exportCsv).click();
    await vi.waitFor(() => expect(download).toHaveBeenCalledTimes(1));
    const [, blob] = download.mock.calls[0]! as [string, Blob];
    const text = new TextDecoder('utf-8', { ignoreBOM: true }).decode(await blob.arrayBuffer());
    expect(text).toBe(eventsCsv(newer));
  });
});
