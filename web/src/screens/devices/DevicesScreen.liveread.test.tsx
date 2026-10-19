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
  makeDeviceBranch,
  makeDeviceEventRow,
  makeDeviceJobRow,
  makeDeviceTransactionRow,
  makeDevicesOverview,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { th as sharedTh } from '../../shared/i18n/th';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

type Api = Pick<ApiClient, 'view' | 'post' | 'page'>;

function overview(branchId = 'B1') {
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
  makeDeviceJobRow({
    id: 'J1',
    kind: 'PRINT_RECEIPT',
    ref: 'TX-1',
    status: 'QUEUED',
    actor: 'teller1',
    created: '2026-10-14T10:00:00.000Z',
  }),
];

const B1_EVENTS = [
  makeDeviceEventRow({
    seq: '10',
    kind: 'PRINT_RECEIPT',
    ref: 'TX-1',
    at: '2026-10-14T10:00:00.000Z',
    actor: 'teller1',
    detail: '{}',
  }),
];

const B1_TXS = [
  makeDeviceTransactionRow({ id: 'TX-1', type: 'BUY', amount: '1000.00', currency: 'USD' }),
];

class FakeServer {
  version = 1;
  devices = overview('B1');
  jobs = B1_JOBS;
  events = B1_EVENTS;
  transactions = B1_TXS;
  readonly reads: Array<{ via: 'view' | 'page'; name: string; q?: ViewParams | PageParams }> = [];
  readonly posts: Array<{ path: string; body: unknown }> = [];

  onView?: (name: string, q?: ViewParams) => Promise<ViewAnswer<unknown>> | undefined;
  onPage?: (name: string, q?: PageParams) => Promise<unknown[]> | undefined;
  onPost: (path: string, body: unknown) => Promise<unknown> = async () => ({ saved: true });

  get api(): Api {
    const server = this;
    return {
      view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
        server.reads.push({ via: 'view', name, q });
        const custom = server.onView?.(name, q);
        if (custom) return custom as Promise<ViewAnswer<T>>;
        let data: unknown;
        if (name === 'devices') data = server.devices;
        else if (name === 'device_jobs') data = server.jobs;
        else if (name === 'device_events') data = server.events;
        return Promise.resolve({
          version: server.version,
          changed: true,
          data: structuredClone(data) as T,
        });
      },
      page<R>(name: string, q?: PageParams): Promise<R[]> {
        server.reads.push({ via: 'page', name, q });
        const custom = server.onPage?.(name, q);
        if (custom) return custom as Promise<R[]>;
        if (name === 'device_transactions') {
          return Promise.resolve(structuredClone(server.transactions) as R[]);
        }
        if (name === 'device_events') {
          return Promise.resolve(structuredClone(server.events) as R[]);
        }
        return Promise.resolve([]);
      },
      post<B, Res>(path: string, body: B): Promise<Res> {
        server.posts.push({ path, body: structuredClone(body) });
        return server.onPost(path, body) as Promise<Res>;
      },
    };
  }

  counts(): Record<string, number> {
    const names = ['devices', 'device_jobs', 'device_events', 'device_transactions'];
    return Object.fromEntries(
      names.map((n) => [n, this.reads.filter((r) => r.name === n).length]),
    );
  }
}

describe('DevicesScreen live read failures and recovery (#2190)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let fetchSpy: ReturnType<typeof vi.fn>;
  let ids = 0;

  beforeEach(() => {
    setLang('th');
    ids = 0;
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

  function button(label: string): HTMLButtonElement {
    const found = [...host.querySelectorAll('button')].filter(
      (b) => (b.textContent ?? '').trim() === label,
    );
    if (found.length !== 1) {
      throw new Error(`expected 1 button "${label}", found ${found.length}`);
    }
    return found[0]!;
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 20; i++) {
      await vi.advanceTimersByTimeAsync(0);
    }
  }

  // Row 1: All reads OK | Empty message (kept)
  it('Row 1: keeps message empty when all reads succeed initially and after a transient live failure clears', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    let failJobs = false;
    server.onView = (name) => {
      if (name === 'device_jobs' && failJobs) {
        return Promise.reject(
          new ApiError({ code: 'serverError', message: 'jobs down', status: 500 }),
        );
      }
      return undefined;
    };

    mount(server);
    await settle();

    expect(messageText()).toBe('');
    expect(button(th.refresh).disabled).toBe(false);
    expect(button(th.print).disabled).toBe(false);

    // Transient failure on next poll shows message, then all reads OK again leaves empty message
    failJobs = true;
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.serverError']);

    failJobs = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();
    expect(messageText()).toBe('');
  });

  // Row 2: `devices` read fails at mount | Message = `wordedError` text
  it('Row 2: shows wordedError text and backs off when devices read fails at mount', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    server.onView = (name) => {
      if (name === 'devices') {
        return Promise.reject(
          new ApiError({ code: 'roleNotAllowedHere', message: 'raw refusal', status: 403 }),
        );
      }
      return undefined;
    };

    mount(server);
    await settle();

    expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);

    // Verify poller rethrow kept backoff (20s instead of 10s)
    const afterMountCount = server.reads.filter((r) => r.name === 'devices').length;
    expect(afterMountCount).toBe(1);

    await vi.advanceTimersByTimeAsync(10_000);
    expect(server.reads.filter((r) => r.name === 'devices').length).toBe(1);

    await vi.advanceTimersByTimeAsync(10_000);
    expect(server.reads.filter((r) => r.name === 'devices').length).toBe(2);
  });

  // Row 3: `device_events` keeps failing; user presses Refresh or Print | `busy` is released once that read has failed; buttons enabled again; the message shown
  it('Row 3: releases busy and re-enables buttons with wordedError message when device_events keeps failing on Refresh or Print', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    server.onView = (name) => {
      if (name === 'device_events') {
        return Promise.reject(
          new ApiError({ code: 'serverError', message: 'events down', status: 500 }),
        );
      }
      return undefined;
    };

    mount(server);
    await settle();

    // Press Refresh while device_events keeps failing
    button(th.refresh).click();
    await settle();

    expect(button(th.refresh).disabled).toBe(false);
    expect(button(th.print).disabled).toBe(false);
    expect(button(th.drawer).disabled).toBe(false);
    expect(messageText()).toBe(sharedTh['error.serverError']);

    // Press Print while device_events keeps failing
    button(th.print).click();
    await settle();

    expect(server.posts).toHaveLength(1);
    expect(button(th.refresh).disabled).toBe(false);
    expect(button(th.print).disabled).toBe(false);
    expect(button(th.drawer).disabled).toBe(false);
    expect(messageText()).toBe(sharedTh['error.serverError']);
  });

  // Row 4: A view fails once, then its next poll succeeds | The message set by that failure is cleared
  it('Row 4: clears the message set by a failed view when its next poll succeeds', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    let failDevices = true;
    server.onView = (name) => {
      if (name === 'devices' && failDevices) {
        return Promise.reject(
          new ApiError({ code: 'serverError', message: 'temporary failure', status: 500 }),
        );
      }
      return undefined;
    };

    mount(server);
    await settle();
    expect(messageText()).toBe(sharedTh['error.serverError']);

    // Next poll for failed devices view runs after 20s backoff and succeeds
    failDevices = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();

    expect(messageText()).toBe('');
  });

  it('Row 4b: clears the message when a view fails while an action is in flight, the action succeeds, and the view next read succeeds', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    let failDevices = false;
    let releasePost!: () => void;
    const postHeld = new Promise<{ saved: boolean }>((r) => {
      releasePost = () => r({ saved: true });
    });

    server.onView = (name) => {
      if (name === 'devices' && failDevices) {
        return Promise.reject(new Error('Network connection failed'));
      }
      return undefined;
    };
    server.onPost = () => postHeld;

    mount(server);
    await settle();
    expect(messageText()).toBe('');

    // Start Print action (held in flight)
    button(th.print).click();
    await settle();

    // While the action is in flight, devices poll ticks and fails
    failDevices = true;
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.network']);

    // Now the action succeeds and the post-action devices read succeeds
    failDevices = false;
    releasePost();
    await settle();

    expect(messageText()).toBe('');
  });

  // Row 5: `device_transactions` read or load-more fails | Message as today (kept)
  it('Row 5: preserves device_transactions read and load-more error messages even when live views succeed or recover', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    let failInitialTx = true;
    let failLoadMoreTx = false;
    let failDevices = false;
    server.onView = (name) => {
      if (name === 'devices' && failDevices) {
        return Promise.reject(
          new ApiError({ code: 'roleNotAllowedHere', message: 'devices refused', status: 403 }),
        );
      }
      return undefined;
    };
    server.onPage = (name, q) => {
      if (name === 'device_transactions' && failInitialTx && !q?.before) {
        return Promise.reject(
          new ApiError({ code: 'roleNotAllowedHere', message: 'tx refused', status: 403 }),
        );
      }
      if (name === 'device_transactions' && failLoadMoreTx && q?.before) {
        return Promise.reject(
          new ApiError({ code: 'serverError', message: 'load more failed', status: 500 }),
        );
      }
      return undefined;
    };

    mount(server);
    await settle();
    expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);

    // Subsequent live view polls must not clear the device_transactions failure message
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);

    // Allow initial transactions on Refresh, then fail devices on poll and load-more transactions
    failInitialTx = false;
    button(th.refresh).click();
    await settle();
    expect(messageText()).toBe('');

    failDevices = true;
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);

    failLoadMoreTx = true;
    const loadMoreBtn = host.querySelector('.picker-group button') as HTMLButtonElement;
    loadMoreBtn.click();
    await settle();
    expect(messageText()).toBe(sharedTh['error.serverError']);

    // When devices recovers at +20s, it must NOT clear the load-more error message set after it
    failDevices = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.serverError']);
  });

  it('Row 5b: keeps transactions and load-more network error when a failing view with the same worded text recovers', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    let failViews = true;
    let failInitialTx = true;
    let failLoadMoreTx = false;
    server.onView = (name) => {
      if (failViews && (name === 'devices' || name === 'device_jobs' || name === 'device_events')) {
        return Promise.reject(new Error('Network connection failed'));
      }
      return undefined;
    };
    server.onPage = (name, q) => {
      if (name === 'device_transactions' && failInitialTx && !q?.before) {
        return Promise.reject(new Error('Network connection failed'));
      }
      if (name === 'device_transactions' && failLoadMoreTx && q?.before) {
        return Promise.reject(new Error('Network connection failed'));
      }
      return undefined;
    };

    // Probe B: all reads fail with network error at mount, then views recover at +20s
    mount(server);
    await settle();
    expect(messageText()).toBe(sharedTh['error.network']);

    failViews = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.network']);

    // Probe C: Refresh succeeds, then a view fails with network error, then load-more fails with the same network error, then view recovers
    failInitialTx = false;
    button(th.refresh).click();
    await settle();
    expect(messageText()).toBe('');

    failViews = true;
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.network']);

    failLoadMoreTx = true;
    const loadMoreBtn = host.querySelector('.picker-group button') as HTMLButtonElement;
    loadMoreBtn.click();
    await settle();
    expect(messageText()).toBe(sharedTh['error.network']);

    failViews = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.network']);
  });

  // Row 6: A refused action | Reads no view (kept)
  it('Row 6: reads no view when an action is refused, and re-reads views when the action succeeds', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    mount(server);
    await settle();

    server.onPost = () =>
      Promise.reject(
        new ApiError({ code: 'roleNotAllowedHere', message: 'refused', status: 403 }),
      );
    const before = server.counts();

    button(th.print).click();
    await settle();

    expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);
    expect(button(th.print).disabled).toBe(false);
    expect(server.posts).toHaveLength(1);
    expect(server.counts()).toEqual(before);

    // Positive control: when action succeeds and a re-read fails, views are read, busy releases, and message is shown
    server.onPost = async () => ({ saved: true });
    server.onView = (name) => {
      if (name === 'device_jobs') {
        return Promise.reject(
          new ApiError({ code: 'serverError', message: 'jobs failed', status: 500 }),
        );
      }
      return undefined;
    };
    button(th.print).click();
    await settle();

    expect(server.counts().devices).toBe(before.devices! + 1);
    expect(button(th.print).disabled).toBe(false);
    expect(messageText()).toBe(sharedTh['error.serverError']);
  });

  // Edge case: Two views fail: the latest failure's text is shown; clearing one view's failure does not hide the other's
  it('Edge case: shows the latest failure when two views fail, and clearing one does not hide the other', async () => {
    vi.useFakeTimers();
    const server = new FakeServer();
    let failDevices = true;
    let failJobs = true;
    server.onView = (name) => {
      if (name === 'devices' && failDevices) {
        return Promise.reject(
          new ApiError({ code: 'roleNotAllowedHere', message: 'devices fail', status: 403 }),
        );
      }
      if (name === 'device_jobs' && failJobs) {
        return Promise.reject(
          new ApiError({ code: 'serverError', message: 'jobs fail', status: 500 }),
        );
      }
      return undefined;
    };

    mount(server);
    await settle();

    // devices failed first, device_jobs failed second -> latest failure's text is shown
    expect(messageText()).toBe(sharedTh['error.serverError']);

    // Now device_jobs recovers on the next poll (at 20s), while devices still fails on a delayed/prior state:
    // First test when devices recovers at 20s while device_jobs still fails -> device_jobs error stays shown
    failDevices = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.serverError']);

    // Next test when devices fails again at 30s (now latest is roleNotAllowedHere), and at 40s devices recovers while device_jobs is still failing:
    failDevices = true;
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.roleNotAllowedHere']);

    // At t=40s, device_jobs ticks (20s + 20s*2 = 60s, not yet) — wait: at t=50s (30s + 20s backoff), devices ticks and recovers!
    // Even though devices was the latest failure, device_jobs is still in failed state, so device_jobs's error is shown!
    failDevices = false;
    await vi.advanceTimersByTimeAsync(20_000);
    await settle();
    expect(messageText()).toBe(sharedTh['error.serverError']);

    // Finally at t=60s, device_jobs ticks and recovers -> message is cleared
    failJobs = false;
    await vi.advanceTimersByTimeAsync(10_000);
    await settle();
    expect(messageText()).toBe('');
  });
});
