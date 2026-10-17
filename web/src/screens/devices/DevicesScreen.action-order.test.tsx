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
import { STATUS_LABELS } from './labels';

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

function overview(branchId = 'B1'): DevicesOverview {
  return {
    branchId,
    branches: [{ id: 'B1', name: 'สาขาหนึ่ง' }],
    printerOnline: true,
    printerKind: 'SIMULATED',
    printerModel: 'NEOCAL NTP-8220BT',
    hasDrawer: null,
    drawerAgentActive: false,
    balanceThb: '1000.00',
    balanceUsd: '50.00',
    completedCount: 2,
  };
}

function makeJob(status: DeviceJobRow['status']): DeviceJobRow {
  return {
    id: 'J1',
    kind: 'PRINT_RECEIPT',
    ref: 'TX-1',
    status,
    actor: 'teller1',
    created: '2026-10-14T10:00:00.000Z',
  };
}

const B1_EVENTS: DeviceEventRow[] = [
  {
    seq: '10',
    kind: 'PRINT_RECEIPT',
    ref: 'TX-1',
    at: '2026-10-14T10:00:00.000Z',
    actor: 'teller1',
    detail: '{}',
  },
];

const B1_TXS: DeviceTransactionRow[] = [
  { id: 'TX-1', type: 'BUY', amount: '1000.00', currency: 'USD' },
];

class FakeServer {
  version = 1;
  devices: DevicesOverview = overview('B1');
  jobs: DeviceJobRow[] = [makeJob('QUEUED')];
  events: DeviceEventRow[] = B1_EVENTS;
  transactions: DeviceTransactionRow[] = B1_TXS;

  onView?: (name: string, q?: ViewParams) => Promise<ViewAnswer<unknown>> | undefined;
  onPost: (path: string, body: unknown) => Promise<unknown> = async () => ({ saved: true });

  get api(): Api {
    const server = this;
    return {
      view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
        const custom = server.onView?.(name, q);
        if (custom) return custom as Promise<ViewAnswer<T>>;
        let data: unknown;
        if (name === 'devices') data = server.devices;
        else if (name === 'device_jobs') data = server.jobs;
        else if (name === 'device_events') data = server.events;
        return Promise.resolve({
          version: ++server.version,
          changed: true,
          data: structuredClone(data) as T,
        });
      },
      page<R>(name: string, _q?: PageParams): Promise<R[]> {
        if (name === 'device_transactions') {
          return Promise.resolve(structuredClone(server.transactions) as R[]);
        }
        if (name === 'device_events') {
          return Promise.resolve(structuredClone(server.events) as R[]);
        }
        return Promise.resolve([]);
      },
      post<B, Res>(path: string, body: B): Promise<Res> {
        return server.onPost(path, body) as Promise<Res>;
      },
    };
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('DevicesScreen poll generation across actions (#2443)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let ids = 0;

  beforeEach(() => {
    setLang('th');
    ids = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(() => Promise.reject(new Error('offline in tests'))),
    );
    vi.spyOn(window.history, 'replaceState').mockImplementation(() => {});
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(server: FakeServer): void {
    dispose = render(
      () => <DevicesScreen api={server.api} branch="B1" newId={() => `req-${++ids}`} />,
      host,
    );
  }

  function messageText(): string {
    const m = host.querySelector<HTMLElement>('#message');
    return (m?.textContent ?? '').trim();
  }

  function findButtons(label: string): HTMLButtonElement[] {
    return [...host.querySelectorAll('button')].filter(
      (b) => (b.textContent ?? '').trim() === label,
    );
  }

  function button(label: string): HTMLButtonElement {
    const found = findButtons(label);
    if (found.length !== 1) {
      throw new Error(`expected 1 button "${label}", found ${found.length}`);
    }
    return found[0]!;
  }

  function jobStatusCells(): string[] {
    return [
      ...host.querySelectorAll<HTMLElement>(`.jobs-table td[data-label="${th.colJobStatus}"]`),
    ].map((el) => (el.textContent ?? '').trim());
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 20; i++) {
      await vi.advanceTimersByTimeAsync(0);
    }
  }

  it('row 1: Old poll in flight, Process job J1, new read says J1 done, old poll then answers J1 pending -> J1 shows done', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    mount(server);
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.QUEUED.th]);
    expect(findButtons(th.process).length).toBe(1);

    // Next 10s poll of device_jobs starts and stays in flight before Process is clicked
    const stalePoll = deferred<ViewAnswer<DeviceJobRow[]>>();
    let interceptOnce = true;
    server.onView = (name) => {
      if (name === 'device_jobs' && interceptOnce) {
        interceptOnce = false;
        return stalePoll.promise;
      }
      return undefined;
    };

    await vi.advanceTimersByTimeAsync(10_000);

    // User clicks Process on J1; server transitions J1 to SIMULATED_PRINTED and post-action read finishes
    server.onPost = async () => {
      server.jobs = [makeJob('SIMULATED_PRINTED')];
      return { saved: true };
    };
    button(th.process).click();
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);
    expect(findButtons(th.process).length).toBe(0);

    // Now the old pre-action poll lands with stale QUEUED state
    stalePoll.resolve({
      version: 99,
      changed: true,
      data: [makeJob('QUEUED')],
    });
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);
    expect(findButtons(th.process).length).toBe(0);
  });

  it('row 2: Old poll errors after the new read succeeded -> no error shown', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    mount(server);
    await settle();

    const stalePoll = deferred<ViewAnswer<DeviceJobRow[]>>();
    let interceptOnce = true;
    server.onView = (name) => {
      if (name === 'device_jobs' && interceptOnce) {
        interceptOnce = false;
        return stalePoll.promise;
      }
      return undefined;
    };

    await vi.advanceTimersByTimeAsync(10_000);

    server.onPost = async () => {
      server.jobs = [makeJob('SIMULATED_PRINTED')];
      return { saved: true };
    };
    button(th.process).click();
    await settle();

    expect(messageText()).toBe('');
    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);

    // Old pre-action poll rejects after the post-action read succeeded
    stalePoll.reject(
      new ApiError({ code: 'serverError', message: 'stale poll failed', status: 500 }),
    );
    await settle();

    expect(messageText()).toBe('');
    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);
  });

  it('row 3: Normal polling with no action applies answers as today, and a late pre-Retry poll does not revert the post-Retry read', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    mount(server);
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.QUEUED.th]);

    // Normal poll with no action updates J1 from QUEUED to FAILED
    server.jobs = [makeJob('FAILED')];
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.FAILED.th]);
    expect(findButtons(th.retry).length).toBe(1);

    // Next poll starts and stays in flight while user clicks Retry
    const stalePoll = deferred<ViewAnswer<DeviceJobRow[]>>();
    let interceptOnce = true;
    server.onView = (name) => {
      if (name === 'device_jobs' && interceptOnce) {
        interceptOnce = false;
        return stalePoll.promise;
      }
      return undefined;
    };

    await vi.advanceTimersByTimeAsync(10_000);

    server.onPost = async () => {
      server.jobs = [makeJob('SIMULATED_PRINTED')];
      return { saved: true };
    };
    button(th.retry).click();
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);
    expect(findButtons(th.retry).length).toBe(0);

    // Late pre-Retry poll answers with FAILED
    stalePoll.resolve({
      version: 99,
      changed: true,
      data: [makeJob('FAILED')],
    });
    await settle();

    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);
    expect(findButtons(th.retry).length).toBe(0);
  });

  it('edge case: runAction keeps busy() until the current generation first reads complete, ignoring a stale pre-action poll', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    mount(server);
    await settle();

    const stalePoll = deferred<ViewAnswer<DeviceJobRow[]>>();
    const postActionRead = deferred<ViewAnswer<DeviceJobRow[]>>();
    let jobReadCall = 0;
    server.onView = (name) => {
      if (name === 'device_jobs') {
        jobReadCall += 1;
        if (jobReadCall === 1) return stalePoll.promise;
        if (jobReadCall === 2) return postActionRead.promise;
      }
      return undefined;
    };

    // Start pre-action poll (jobReadCall === 1)
    await vi.advanceTimersByTimeAsync(10_000);

    // Click Process -> starts post-action initBranch (jobReadCall === 2)
    button(th.process).click();
    await settle();

    expect(button(th.refresh).disabled).toBe(true);

    // Stale pre-action poll finishes while post-action read is still in flight
    stalePoll.resolve({
      version: 50,
      changed: true,
      data: [makeJob('QUEUED')],
    });
    await settle();

    // busy() must still be true until postActionRead completes
    expect(button(th.refresh).disabled).toBe(true);

    postActionRead.resolve({
      version: 51,
      changed: true,
      data: [makeJob('SIMULATED_PRINTED')],
    });
    await settle();

    expect(button(th.refresh).disabled).toBe(false);
    expect(jobStatusCells()).toEqual([STATUS_LABELS.SIMULATED_PRINTED.th]);
  });
});
