import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import {
  ApiError,
  type ApiClient,
  type PageParams,
  type ViewAnswer,
  type ViewParams,
} from '../../shared/api/client';
import {
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

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

interface FakeServerOptions {
  onPageFail?: (name: string, q?: PageParams) => Promise<never> | undefined;
  onPostFail?: (path: string, body: unknown) => Promise<never> | undefined;
}

function makeServer(opts: FakeServerOptions = {}) {
  const branchData = {
    version: 1,
    devices: makeDevicesOverview({
      branchId: 'B1',
      branches: [
        makeDeviceBranch({ id: 'B1', name: 'Branch 1' }),
        makeDeviceBranch({ id: 'B2', name: 'Branch 2' }),
      ],
      printerOnline: true,
      printerKind: 'SIMULATED',
      printerModel: 'NTP-8220BT',
      hasDrawer: null,
      drawerAgentActive: false,
      balanceThb: '1000.00',
      balanceUsd: '50.00',
      completedCount: 2,
    }),
    jobs: [
      makeDeviceJobRow({
        id: 'J1',
        kind: 'PRINT_RECEIPT',
        ref: 'TX-1',
        status: 'QUEUED',
        actor: 'teller1',
        created: '2026-10-14T10:00:00.000Z',
      }),
    ],
    events: [
      makeDeviceEventRow({
        seq: '10',
        kind: 'PRINT_RECEIPT',
        ref: 'TX-1',
        at: '2026-10-14T10:00:00.000Z',
        actor: 'teller1',
        detail: '{}',
      }),
    ],
    transactions: [
      makeDeviceTransactionRow({ id: 'TX-1', type: 'BUY', amount: '1000.00', currency: 'USD' }),
    ],
  };

  const api: Api = {
    view<T>(name: string, _q?: ViewParams): Promise<ViewAnswer<T>> {
      let data: unknown;
      if (name === 'devices') data = branchData.devices;
      else if (name === 'device_jobs') data = branchData.jobs;
      else if (name === 'device_events') data = branchData.events;
      return Promise.resolve({
        version: branchData.version,
        changed: true,
        data: data as T,
      });
    },
    page<R>(name: string, q?: PageParams): Promise<R[]> {
      const fail = opts.onPageFail?.(name, q);
      if (fail) return fail;
      if (name === 'device_transactions') {
        return Promise.resolve([...branchData.transactions] as R[]);
      }
      if (name === 'device_events') {
        return Promise.resolve([...branchData.events] as R[]);
      }
      return Promise.resolve([]);
    },
    post<B, Res>(path: string, body: B): Promise<Res> {
      const fail = opts.onPostFail?.(path, body);
      if (fail) return fail;
      return Promise.resolve({ saved: true } as Res);
    },
  };

  return { branchData, api };
}

describe('WE-34 screens/devices/ worded error (#1783)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
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

  function mount(api: Api, initialBranch = 'B1') {
    const [branch, setBranch] = createSignal(initialBranch);
    dispose = render(
      () => <DevicesScreen api={api} branch={branch()} />,
      host,
    );
    return { branch, setBranch };
  }

  function messageText(): string {
    const m = host.querySelector<HTMLElement>('#message');
    return (m?.textContent ?? '').trim();
  }

  describe('Catch 1: loadTransactions (device_transactions initial read)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      let fail = true;
      const { api } = makeServer({
        onPageFail: (name) => {
          if (name === 'device_transactions' && fail) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      // Switch language to en and remount
      setLang('en');
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      mount(api);

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      const { api } = makeServer({
        onPageFail: (name) => {
          if (name === 'device_transactions') {
            return Promise.reject(
              new ApiError({ code: 'roleNotAllowedHere', message: 'x', status: 500 }),
            );
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'ไม่สามารถอ่านรายการธุรกรรมของสาขานี้ได้';
      const { api } = makeServer({
        onPageFail: (name) => {
          if (name === 'device_transactions') {
            return Promise.reject(
              new ApiError({ code: 'unmappedTxCode123', message: serverMessage, status: 400 }),
            );
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  describe('Catch 2: runAction (device actions e.g. print/refresh)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      const { api } = makeServer({
        onPostFail: () => Promise.reject(new TypeError('Failed to fetch')),
      });
      mount(api);

      await vi.waitFor(() => {
        expect(host.querySelector('.picker-group select')).not.toBeNull();
      });

      const printBtn = host.querySelector('.drawer-card button, section.card button') as HTMLButtonElement;
      expect(printBtn).not.toBeNull();
      printBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      // Switch to en and click again
      setLang('en');
      printBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      const { api } = makeServer({
        onPostFail: () =>
          Promise.reject(
            new ApiError({ code: 'roleNotAllowedHere', message: 'x', status: 500 }),
          ),
      });
      mount(api);

      await vi.waitFor(() => {
        expect(host.querySelector('.picker-group select')).not.toBeNull();
      });

      const printBtn = host.querySelector('.drawer-card button, section.card button') as HTMLButtonElement;
      expect(printBtn).not.toBeNull();
      printBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'เครื่องพิมพ์ใบเสร็จถูกล็อคโดยผู้ดูแลระบบ';
      const { api } = makeServer({
        onPostFail: () =>
          Promise.reject(
            new ApiError({ code: 'unmappedAction456', message: serverMessage, status: 400 }),
          ),
      });
      mount(api);

      await vi.waitFor(() => {
        expect(host.querySelector('.picker-group select')).not.toBeNull();
      });

      const printBtn = host.querySelector('.drawer-card button, section.card button') as HTMLButtonElement;
      expect(printBtn).not.toBeNull();
      printBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  describe('Catch 3: handleLoadMoreTransactions (device_transactions pagination)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      let failMore = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_transactions' && failMore && q?.before) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        expect(host.querySelector('.picker-group select')).not.toBeNull();
      });

      failMore = true;
      const loadMoreBtn = host.querySelector('.picker-group button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      // Switch to en and click again
      setLang('en');
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      let failMore = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_transactions' && failMore && q?.before) {
            return Promise.reject(
              new ApiError({ code: 'roleNotAllowedHere', message: 'x', status: 500 }),
            );
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        expect(host.querySelector('.picker-group select')).not.toBeNull();
      });

      failMore = true;
      const loadMoreBtn = host.querySelector('.picker-group button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'ไม่สามารถดึงธุรกรรมเพิ่มเติมได้ในขณะนี้';
      let failMore = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_transactions' && failMore && q?.before) {
            return Promise.reject(
              new ApiError({ code: 'unmappedLoadMoreTx', message: serverMessage, status: 400 }),
            );
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        expect(host.querySelector('.picker-group select')).not.toBeNull();
      });

      failMore = true;
      const loadMoreBtn = host.querySelector('.picker-group button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  describe('Catch 4: handleLoadMoreEvents (device_events pagination)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      let failMore = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_events' && failMore && q?.before) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        const cell = host.querySelector('.events-table tbody td small');
        expect(cell?.textContent).toBe('TX-1');
      });

      failMore = true;
      const loadMoreBtn = host.querySelector('.events-controls button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      // Switch to en and click again
      setLang('en');
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      let failMore = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_events' && failMore && q?.before) {
            return Promise.reject(
              new ApiError({ code: 'roleNotAllowedHere', message: 'x', status: 500 }),
            );
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        const cell = host.querySelector('.events-table tbody td small');
        expect(cell?.textContent).toBe('TX-1');
      });

      failMore = true;
      const loadMoreBtn = host.querySelector('.events-controls button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'ไม่สามารถอ่านประวัติเหตุการณ์เพิ่มเติม';
      let failMore = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_events' && failMore && q?.before) {
            return Promise.reject(
              new ApiError({ code: 'unmappedEventsMore99', message: serverMessage, status: 400 }),
            );
          }
        },
      });
      mount(api);

      await vi.waitFor(() => {
        const cell = host.querySelector('.events-table tbody td small');
        expect(cell?.textContent).toBe('TX-1');
      });

      failMore = true;
      const loadMoreBtn = host.querySelector('.events-controls button') as HTMLButtonElement;
      expect(loadMoreBtn).not.toBeNull();
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  describe('Criterion 3: stale data on branch switch', () => {
    it('clears stale data from previous branch when subsequent branch read fails', async () => {
      let failB2 = false;
      const { api } = makeServer({
        onPageFail: (name, q) => {
          if (name === 'device_transactions' && failB2 && q?.branch === 'B2') {
            return Promise.reject(
              new ApiError({ code: 'branchNotAccessible', message: 'สาขาถูกปิด', status: 400 }),
            );
          }
        },
      });

      const { setBranch } = mount(api, 'B1');

      // Verify B1 loaded with TX-1
      await vi.waitFor(() => {
        const select = host.querySelector('.picker-group select') as HTMLSelectElement;
        expect(select.options.length).toBe(2);
        expect(select.options[1]?.value).toBe('TX-1');
      });

      // Switch to B2 and fail
      failB2 = true;
      setBranch('B2');

      await vi.waitFor(() => {
        expect(messageText()).toBe('สาขาถูกปิด');
      });

      // Old transaction TX-1 must not be in picker
      const selectAfter = host.querySelector('.picker-group select') as HTMLSelectElement;
      expect(selectAfter.options.length).toBe(1);
      expect(selectAfter.textContent).not.toContain('TX-1');
    });
  });
});
