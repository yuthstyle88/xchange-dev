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

interface Read {
  via: 'view' | 'page';
  name: string;
  branch?: string;
  since?: number;
  before?: string;
  limit?: number;
}

interface BranchData {
  version: number;
  devices: DevicesOverview;
  jobs: DeviceJobRow[];
  events: DeviceEventRow[];
  transactions: DeviceTransactionRow[];
  olderEvents: Record<string, DeviceEventRow[]>;
  olderTransactions: Record<string, DeviceTransactionRow[]>;
}

function overview(branchId: string): DevicesOverview {
  return {
    branchId,
    branches: [{ id: branchId, name: branchId }],
    printerOnline: true,
    printerKind: 'SIMULATED',
    printerModel: 'NEOCAL NTP-8220BT',
    hasDrawer: null,
    drawerAgentActive: false,
    balanceThb: '100.00',
    balanceUsd: '50.00',
    completedCount: 1,
  };
}

class FakeServer {
  readonly reads: Read[] = [];
  readonly posts: Array<{ path: string; body: unknown }> = [];
  readonly branches: Record<string, BranchData>;
  readonly api: Pick<ApiClient, 'view' | 'post' | 'page'>;

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
        const data = name === 'devices' ? b.devices : name === 'device_jobs' ? b.jobs : name === 'device_events' ? b.events : undefined;
        if (data === undefined) return Promise.reject(new Error(`unknown view ${name}`));
        return Promise.resolve({ version: b.version, changed: true, data: structuredClone(data) as T });
      },
      page<R>(name: string, q?: PageParams): Promise<R[]> {
        server.reads.push({ via: 'page', name, ...(q ?? {}) });
        const b = server.branches[q?.branch ?? 'MAIN'];
        if (!b) return Promise.reject(new ApiError({ code: 'invalidRequest', message: 'Invalid branch', status: 400 }));
        const rows = name === 'device_transactions' ? b.transactions : [];
        return Promise.resolve(structuredClone(rows) as R[]);
      },
      post<B, R>(path: string, body: B): Promise<R> {
        server.posts.push({ path, body: structuredClone(body) });
        return Promise.resolve({ saved: true } as R);
      },
    };
  }

  readsOf(name: string): Read[] {
    return this.reads.filter((r) => r.name === name);
  }
}

function b1(): BranchData {
  return {
    version: 1,
    devices: overview('B1'),
    jobs: [],
    events: [],
    transactions: [{ id: 'TX-1', type: 'BUY', amount: '100.00', currency: 'USD' }],
    olderEvents: {},
    olderTransactions: {},
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchSpy = vi.fn().mockImplementation(async (input: unknown) => {
    const url = String(input);
    if (url.includes('/health/ready')) {
      return {
        ok: true,
        json: async () => ({ version: '1.0.0', build: 'abcd1234', channel: 'stable', builtAt: '2026-10-08T00:00:00Z' }),
      };
    }
    return { ok: true, json: async () => ({}) };
  });
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  delete (document as unknown as Record<string, unknown>).hidden;
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
      />
    ),
    host,
  );
}

describe('DevicesScreen hidden-at-mount (#404 §7, D20)', () => {
  it('reads device_transactions exactly once when hidden at mount then shown', async () => {
    vi.useFakeTimers();
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });

    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');

    // While hidden at mount, no reads occur
    await vi.advanceTimersByTimeAsync(10_100);
    expect(server.readsOf('device_transactions')).toHaveLength(0);

    // Shown
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
    document.dispatchEvent(new Event('visibilitychange'));

    await vi.advanceTimersByTimeAsync(10_100);

    // Exactly once on becoming visible
    expect(server.readsOf('device_transactions'), 'hidden->shown reads device_transactions exactly once').toHaveLength(1);

    // Advance further timer to verify no second poll
    await vi.advanceTimersByTimeAsync(20_000);
    expect(server.readsOf('device_transactions'), 'no second read of device_transactions').toHaveLength(1);
  });

  it('reads device_transactions exactly once when visible at mount', async () => {
    vi.useFakeTimers();
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });

    const server = new FakeServer({ B1: b1() });
    mount(server, 'B1');

    await vi.advanceTimersByTimeAsync(10_100);

    // Exactly once on initial visible load
    expect(server.readsOf('device_transactions'), 'visible reads device_transactions exactly once').toHaveLength(1);

    // Advance further timer to verify no second poll
    await vi.advanceTimersByTimeAsync(20_000);
    expect(server.readsOf('device_transactions'), 'no second read of device_transactions').toHaveLength(1);
  });
});
