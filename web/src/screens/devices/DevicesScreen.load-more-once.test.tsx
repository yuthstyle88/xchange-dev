import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
import { setLang } from '../../shared/i18n';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

const BRANCHES = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
];

function overview(branchId: string): DevicesOverview {
  return {
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
  };
}

function event(
  seq: string,
  kind: string,
  ref = '',
  detail = '{}',
  actor = 'backend1',
  at = '2026-10-08T02:00:00.000Z',
): DeviceEventRow {
  return { seq, kind, ref, at, actor, detail };
}

function events(from: number, to: number): DeviceEventRow[] {
  const out: DeviceEventRow[] = [];
  for (let s = from; s >= to; s--) {
    out.push(
      event(
        String(s),
        s % 2 === 0 ? 'PRINT_RECEIPT' : 'DRAWER_REQUEST',
        `TX-${s}`,
        `{"n":${s}}`,
        `actor${s}`,
      ),
    );
  }
  return out;
}

function transaction(id: string): DeviceTransactionRow {
  return {
    id,
    type: 'EXCHANGE',
    amount: '100.00',
    currency: 'USD',
  };
}

function transactions(ids: string[]): DeviceTransactionRow[] {
  return ids.map((id) => transaction(id));
}

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

interface BranchData {
  version: number;
  devices: DevicesOverview;
  jobs: DeviceJobRow[];
  events: DeviceEventRow[];
  transactions: DeviceTransactionRow[];
  olderEvents: Record<string, DeviceEventRow[]>;
  olderTransactions?: Record<string, DeviceTransactionRow[]>;
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
    olderTransactions: {},
    ...over,
  };
}

class FakeServer {
  readonly reads: Read[] = [];
  readonly branches: Record<string, BranchData>;
  readonly api: Api;
  pageHook?: (name: string, q?: PageParams) => Promise<void> | void;

  constructor(branches: Record<string, BranchData>) {
    this.branches = branches;
    const server = this;
    this.api = {
      view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
        server.reads.push({ via: 'view', name, ...(q ?? {}) });
        const b = server.branches[q?.branch ?? 'MAIN'];
        if (!b)
          return Promise.reject(
            new ApiError({ code: 'invalidRequest', message: 'Invalid branch', status: 400 }),
          );
        if (q?.since !== undefined && q.since === b.version) {
          return Promise.resolve({ version: b.version, changed: false });
        }
        const data = server.firstPage(b, name);
        if (data === undefined) return Promise.reject(new Error(`unknown view ${name}`));
        return Promise.resolve({
          version: b.version,
          changed: true,
          data: structuredClone(data) as T,
        });
      },
      async page<R>(name: string, q?: PageParams): Promise<R[]> {
        server.reads.push({ via: 'page', name, ...(q ?? {}) });
        if (server.pageHook) {
          await server.pageHook(name, q);
        }
        const b = server.branches[q?.branch ?? 'MAIN'];
        if (!b)
          return Promise.reject(
            new ApiError({ code: 'invalidRequest', message: 'Invalid branch', status: 400 }),
          );
        let rows: unknown[] | undefined;
        if (q?.before === undefined) {
          rows = server.firstPage(b, name) as unknown[] | undefined;
        } else if (name === 'device_events') {
          rows = b.olderEvents[q.before] ?? [];
        } else if (name === 'device_transactions') {
          rows = b.olderTransactions?.[q.before] ?? [];
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
  return [...table.querySelectorAll('tr')].filter(
    (tr) => tr.querySelector('td') && !tr.querySelector('td.empty'),
  );
}

describe('Devices: load more in-flight guards (#2664)', () => {
  it('L1: Events first page 200..101; double click "Load more" -> 1 page("device_events") read with before: "101"; 200 rows, each seq once', async () => {
    const first = events(200, 101);
    const olderFrom101 = events(100, 1);

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

    const loadMoreBtn = button(th.loadMore, sectionOf(th.eventsHeading));
    loadMoreBtn.click();
    loadMoreBtn.click();

    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(200));

    const eventPageReads = server.reads.filter(
      (r) => r.via === 'page' && r.name === 'device_events',
    );
    expect(eventPageReads).toHaveLength(1);
    expect(eventPageReads[0]?.before).toBe('101');
    expect(bodyRows(eventsTable())).toHaveLength(200);

    const refs = bodyRows(eventsTable()).map((tr) => textOf(tr.cells[3]));
    expect(refs).toHaveLength(new Set(refs).size);
  });

  it('L2: Transactions: double click "Load more" -> 1 page("device_transactions") read; no duplicated transaction id in the picker', async () => {
    const initialTx = transactions(['tx-20', 'tx-19', 'tx-18']);
    const olderTx = transactions(['tx-17', 'tx-16', 'tx-15']);

    const server = new FakeServer({
      B1: branchData('B1', {
        version: 5,
        transactions: initialTx,
        olderTransactions: {
          'tx-18': olderTx,
        },
      }),
    });

    mount(server, 'B1');
    const txSection = sectionOf(th.requestHeading);
    await vi.waitFor(() => {
      const select = txSection.querySelector('select')!;
      expect(select.querySelectorAll('option').length).toBe(4); // placeholder + 3 tx
    });

    const loadMoreBtn = button(th.loadMore, txSection);
    loadMoreBtn.click();
    loadMoreBtn.click();

    await vi.waitFor(() => {
      const select = txSection.querySelector('select')!;
      expect(select.querySelectorAll('option').length).toBe(7); // placeholder + 6 tx
    });

    const olderTxReads = server.reads.filter(
      (r) => r.via === 'page' && r.name === 'device_transactions' && r.before !== undefined,
    );
    expect(olderTxReads).toHaveLength(1);
    expect(olderTxReads[0]?.before).toBe('tx-18');

    const select = txSection.querySelector('select')!;
    const ids = [...select.querySelectorAll('option')].map((o) => o.value).filter(Boolean);
    expect(ids).toHaveLength(new Set(ids).size);
  });

  it('L3: Events "Load more" while the read is in flight -> that button is disabled until it answers', async () => {
    const first = events(200, 101);
    const olderFrom101 = events(100, 1);

    let resolveHold: (() => void) | undefined;
    const holdPromise = new Promise<void>((resolve) => {
      resolveHold = resolve;
    });

    const server = new FakeServer({
      B1: branchData('B1', {
        version: 5,
        events: first,
        olderEvents: {
          '101': olderFrom101,
        },
      }),
    });

    server.pageHook = (name) => {
      if (name === 'device_events') return holdPromise;
    };

    mount(server, 'B1');
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));

    const loadMoreBtn = button(th.loadMore, sectionOf(th.eventsHeading));
    expect(loadMoreBtn.disabled).toBe(false);

    loadMoreBtn.click();

    await vi.waitFor(() => expect(loadMoreBtn.disabled).toBe(true));

    resolveHold!();
    await vi.waitFor(() => expect(loadMoreBtn.disabled).toBe(false));
    expect(bodyRows(eventsTable())).toHaveLength(200);
  });

  it('L4: the older-events read fails (500) -> the worded error shows, and the button is enabled again so a retry sends one read', async () => {
    const first = events(200, 101);
    const olderFrom101 = events(100, 1);

    let fail = true;
    const server = new FakeServer({
      B1: branchData('B1', {
        version: 5,
        events: first,
        olderEvents: {
          '101': olderFrom101,
        },
      }),
    });

    server.pageHook = (name) => {
      if (name === 'device_events' && fail) {
        return Promise.reject(
          new ApiError({ code: 'httpError', status: 500, message: 'Server error' }),
        );
      }
    };

    mount(server, 'B1');
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));

    const loadMoreBtn = button(th.loadMore, sectionOf(th.eventsHeading));
    loadMoreBtn.click();

    await vi.waitFor(() => {
      const msg = host.querySelector('#message');
      expect(textOf(msg)).not.toBe('');
    });

    expect(loadMoreBtn.disabled).toBe(false);

    fail = false;
    loadMoreBtn.click();

    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(200));
    const eventPageReads = server.reads.filter(
      (r) => r.via === 'page' && r.name === 'device_events',
    );
    expect(eventPageReads).toHaveLength(2);
  });

  it('L5: two clicks with the first answer back in between -> 2 reads, the second with the new last seq', async () => {
    const first = events(200, 101);
    const olderFrom101 = events(100, 51);
    const olderFrom51 = events(50, 1);

    const server = new FakeServer({
      B1: branchData('B1', {
        version: 5,
        events: first,
        olderEvents: {
          '101': olderFrom101,
          '51': olderFrom51,
        },
      }),
    });

    mount(server, 'B1');
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(100));

    const loadMoreBtn = button(th.loadMore, sectionOf(th.eventsHeading));
    loadMoreBtn.click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(150));

    loadMoreBtn.click();
    await vi.waitFor(() => expect(bodyRows(eventsTable())).toHaveLength(200));

    const eventPageReads = server.reads.filter(
      (r) => r.via === 'page' && r.name === 'device_events',
    );
    expect(eventPageReads).toHaveLength(2);
    expect(eventPageReads[0]?.before).toBe('101');
    expect(eventPageReads[1]?.before).toBe('51');
  });
});
