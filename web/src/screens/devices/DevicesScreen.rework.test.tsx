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
import { setLang } from '../../shared/i18n';
import { en as sharedEn } from '../../shared/i18n/en';
import { th as sharedTh } from '../../shared/i18n/th';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

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

function overview(branchId: string) {
  return makeDevicesOverview({
    branchId,
    branches: [makeDeviceBranch({ id: 'B1', name: 'สาขาหนึ่ง' })],
    printerOnline: true,
    printerKind: 'SIMULATED',
    printerModel: 'NEOCAL NTP-8220BT',
    hasDrawer: null,
    drawerAgentActive: false,
    balanceThb: '1000.00',
    balanceUsd: '50.00',
    completedCount: 2,
  });
}

const B1_JOBS = [
  makeDeviceJobRow({ id: 'J1', kind: 'PRINT_RECEIPT', ref: 'TX-1', status: 'QUEUED', actor: 'teller1', created: '2026-10-14T10:00:00.000Z' }),
];

const B1_EVENTS = [
  makeDeviceEventRow({ seq: '10', kind: 'PRINT_RECEIPT', ref: 'TX-1', at: '2026-10-14T10:00:00.000Z', actor: 'teller1', detail: '{}' }),
];

const B1_TXS = [
  makeDeviceTransactionRow({ id: 'TX-1', type: 'BUY', amount: '1000.00', currency: 'USD' }),
];

class FakeServer {
  readonly branch: BranchData;
  onPageFail?: (name: string, q?: PageParams) => Promise<never> | undefined;

  constructor() {
    this.branch = {
      version: 1,
      devices: overview('B1'),
      jobs: B1_JOBS,
      events: B1_EVENTS,
      transactions: B1_TXS,
      olderEvents: {},
      olderTransactions: {},
    };
  }

  get api(): Api {
    const server = this;
    return {
      view<T>(name: string, _q?: ViewParams): Promise<ViewAnswer<T>> {
        let data: unknown;
        if (name === 'devices') data = server.branch.devices;
        else if (name === 'device_jobs') data = server.branch.jobs;
        else if (name === 'device_events') data = server.branch.events;
        return Promise.resolve({ version: server.branch.version, changed: true, data: structuredClone(data) as T });
      },
      page<R>(name: string, q?: PageParams): Promise<R[]> {
        const fail = server.onPageFail?.(name, q);
        if (fail) return fail;
        if (name === 'device_transactions') {
          return Promise.resolve(structuredClone(server.branch.transactions) as R[]);
        }
        if (name === 'device_events') {
          return Promise.resolve(structuredClone(server.branch.events) as R[]);
        }
        return Promise.resolve([]);
      },
      post<B, Res>(_path: string, _body: B): Promise<Res> {
        return Promise.resolve({ saved: true } as Res);
      },
    };
  }
}

describe('DevicesScreen worded errors for failed reads (#1693)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    setLang('th');
    fetchSpy = vi.fn(() => Promise.reject(new Error('offline in tests')));
    vi.stubGlobal('fetch', fetchSpy);
    vi.spyOn(window.history, 'replaceState').mockImplementation(() => {});
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(server: FakeServer): void {
    dispose = render(
      () => <DevicesScreen api={server.api} branch="B1" />,
      host,
    );
  }

  function messageText(): string {
    const m = host.querySelector<HTMLElement>('#message');
    return (m?.textContent ?? '').trim();
  }

  // Row 1: web/src/screens/devices/DevicesScreen.tsx:148 — transactions fail: empty list, no message -> worded error
  describe('Row 1: initial transactions load failure shows worded error', () => {
    it('shows worded error when transactions read fails with 500 (network failure)', async () => {
      const server = new FakeServer();
      server.onPageFail = (name) => {
        if (name === 'device_transactions') {
          return Promise.reject(new Error('Network connection failed'));
        }
      };

      mount(server);
      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      // Picker shows no transactions
      const select = host.querySelector('.picker-group select') as HTMLSelectElement;
      expect(select.options.length).toBe(1);
      expect(select.options[0]?.text).toBe(th.noTransactions);
    });

    it('shows worded error when transactions read fails with known refusal code', async () => {
      const server = new FakeServer();
      server.onPageFail = (name) => {
        if (name === 'device_transactions') {
          return Promise.reject(
            new ApiError({ code: 'roleNotAllowedHere', message: 'raw refusal', status: 403 }),
          );
        }
      };

      mount(server);
      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);
      });
      expect(messageText()).not.toContain('raw refusal');
      expect(messageText()).not.toContain('roleNotAllowedHere');
    });

    it('shows generic worded error when transactions read fails with unknown code, never raw code or err.message', async () => {
      const server = new FakeServer();
      server.onPageFail = (name) => {
        if (name === 'device_transactions') {
          return Promise.reject(
            new ApiError({ code: 'unknownRefusal999', message: 'RAW_UNWORDED_MSG', status: 400 }),
          );
        }
      };

      mount(server);
      await vi.waitFor(() => {
        expect(messageText()).toBe('RAW_UNWORDED_MSG');
      });
      expect(messageText()).not.toContain('unknownRefusal999');
    });

    it('shows English worded error when language is en and read fails', async () => {
      setLang('en');
      const server = new FakeServer();
      server.onPageFail = (name) => {
        if (name === 'device_transactions') {
          return Promise.reject(
            new ApiError({ code: 'roleNotAllowedHere', message: 'raw', status: 403 }),
          );
        }
      };

      mount(server);
      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.roleNotAllowedHere']);
      });
    });
  });

  // Row 2: web/src/screens/devices/DevicesScreen.tsx:351 — load more transactions does nothing -> worded error
  describe('Row 2: load more transactions failure shows worded error', () => {
    it('shows worded error when load-more transactions fails, keeping existing rows visible', async () => {
      const server = new FakeServer();
      let failLoadMore = false;
      server.onPageFail = (name, q) => {
        if (name === 'device_transactions' && failLoadMore && q?.before) {
          return Promise.reject(
            new ApiError({ code: 'roleNotAllowedHere', message: 'refused', status: 403 }),
          );
        }
      };

      mount(server);
      // Wait for initial load
      await vi.waitFor(() => {
        const select = host.querySelector('.picker-group select') as HTMLSelectElement;
        expect(select.options.length).toBe(2); // placeholder + TX-1
      });
      expect(messageText()).toBe('');

      // Now trigger load more and simulate failure
      failLoadMore = true;
      const loadMoreBtn = host.querySelector('.picker-group button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);
      });

      // Existing transaction is still in picker (stale data remains visible)
      const select = host.querySelector('.picker-group select') as HTMLSelectElement;
      expect(select.options.length).toBe(2);
      expect(select.options[1]?.value).toBe('TX-1');
    });
  });

  // Row 3: web/src/screens/devices/DevicesScreen.tsx:368 — load more events does nothing -> worded error
  describe('Row 3: load more events failure shows worded error', () => {
    it('shows worded error when load-more events fails, keeping existing rows visible', async () => {
      const server = new FakeServer();
      let failLoadMore = false;
      server.onPageFail = (name, q) => {
        if (name === 'device_events' && failLoadMore && q?.before) {
          return Promise.reject(
            new ApiError({ code: 'serverError', message: 'raw 500', status: 500 }),
          );
        }
      };

      mount(server);
      // Wait for initial load
      await vi.waitFor(() => {
        const cell = host.querySelector('.events-table tbody td small');
        expect(cell?.textContent).toBe('TX-1');
      });
      expect(messageText()).toBe('');

      // Now trigger load more events and simulate failure
      failLoadMore = true;
      const loadMoreBtn = host.querySelector('.events-controls button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });

      // Existing events remain visible
      const rows = host.querySelectorAll('.events-table tbody tr');
      expect(rows.length).toBe(1);
    });
  });
});
