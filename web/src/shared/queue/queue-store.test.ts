// V1-kiosk-queue-panel (#486): the shared queue store, design docs/design/xchange-v3-web.md §12.18.3
// rows S1–S8 (v2: src/devices-public/account-bar.js:5, :43-46 of xchange-v2-source@4d0bdbb).
// Written before the implementation by the lane controller. Do not edit, delete or weaken; add
// tests in a file of your own.
//
// The store runs against a fake `ApiClient` (`view` answers like the server: a `since` equal to the
// branch version gets `changed: false`) and a fake `SessionStore` whose `me` is a signal. The deps
// object is `{ api, session }` — the names this file fixes for `queueStore(branch, deps)`.
// Every test uses a branch of its own, because the store is one per branch for the life of the
// module (like `sharedSession()`).
//
// Amended for SB-3 (#842) by the test-writer claude/5d45b017, on the FE lane controller
// claude/5edf7bb5's ruling (#842 issuecomment-6105121433, item 2): S8's "else MAIN" becomes
// "else undefined" (with no `me`, `queueBranch()` is `undefined`); the reactive `setQueueBranch`
// and `me()` cases stay. Session branch design §14.4: no page names 'MAIN' itself.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createEffect, createRoot, createSignal, type Accessor } from 'solid-js';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import type { SessionStore } from '../session/session';
import type { KioskNotifications } from './wire';
import {
  queueBranch,
  queueStore,
  setQueueBranch,
  useQueueStore,
  type QueueStore,
} from './queue-store';

// S8 reads `me().branch` of the shared session; the page has no other session to give it.
const shared = vi.hoisted(() => ({ setMe: undefined as undefined | ((me: unknown) => void) }));
vi.mock('../session/session', async (importOriginal) => {
  const original = await importOriginal<typeof import('../session/session')>();
  const { createSignal: signal } = await import('solid-js');
  const [me, setMe] = signal<Me | undefined>(undefined);
  shared.setMe = (value) => setMe(() => value as Me | undefined);
  const session: SessionStore = { me, signOut: async () => {}, dispose: () => {} };
  return { ...original, sharedSession: () => session };
});

// ── fixtures ─────────────────────────────────────────────────────────────────

const T0 = Date.parse('2026-10-08T03:00:00.000Z');
/** The server's clock runs ten minutes ahead of this browser's. */
const SKEW = 10 * 60 * 1000;

const ME: Me = {
  kind: 'employee',
  id: 'E1',
  role: 'FRONT',
  branch: 'B-HOME',
  name: 'Somchai',
  username: 'somchai',
  branchName: 'B-HOME',
  branches: [],
};

let branchNo = 0;
function freshBranch(): string {
  branchNo += 1;
  return `QS-${branchNo}`;
}

function notifications(branch: string, over: Partial<KioskNotifications> = {}): KioskNotifications {
  return {
    serverNow: new Date(Date.now() + SKEW).toISOString(),
    branchId: branch,
    branchName: 'Silom',
    count: 1,
    latest: 'kq-1',
    workingCount: 0,
    working: [],
    employee: { id: ME.id, role: ME.role },
    ...over,
  };
}

/** A fake `ApiClient` for one branch that answers `kiosk_notifications` like the server. */
class FakeServer {
  readonly calls: ViewParams[] = [];
  readonly names: string[] = [];
  readonly posts: unknown[] = [];
  version = 7;
  data: KioskNotifications;
  /** When set, every request is refused with it. */
  fail: unknown = undefined;
  private holdNext = false;
  private readonly held: Array<() => void> = [];

  constructor(readonly branch: string) {
    this.data = notifications(branch);
  }

  /** The next request stays in flight until the returned function is called. */
  hold(): () => void {
    this.holdNext = true;
    return () => {
      for (const release of this.held.splice(0)) release();
    };
  }

  /** One write to the branch: the version moves and the data is new. */
  write(over: Partial<KioskNotifications> = {}): void {
    this.version += 1;
    this.data = notifications(this.branch, over);
  }

  readonly api = {
    view: async <T>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      this.names.push(name);
      this.calls.push({ ...params });
      if (this.holdNext) {
        this.holdNext = false;
        await new Promise<void>((resolve) => this.held.push(resolve));
      }
      if (this.fail !== undefined) throw this.fail;
      if (params.since !== undefined && params.since === this.version) {
        return { version: this.version, changed: false };
      }
      return { version: this.version, changed: true, data: this.data as unknown as T };
    },
    post: async (path: string, body: unknown): Promise<unknown> => {
      this.posts.push({ path, body });
      return {};
    },
  };
}

function fakeSession(initial: Me | undefined = ME) {
  const [me, setMe] = createSignal<Me | undefined>(initial);
  const signOut = vi.fn(async () => {});
  const session: SessionStore = { me, signOut, dispose: () => {} };
  return { session, setMe, signOut };
}

/** A branch with its fake server and session, its store created with them. */
function setup(me: Me | undefined = ME) {
  const branch = freshBranch();
  const server = new FakeServer(branch);
  const { session, setMe, signOut } = fakeSession(me);
  const store = queueStore(branch, { api: server.api as unknown as ApiClient, session });
  return { branch, server, session, setMe, signOut, store };
}

const disposers: Array<() => void> = [];

/** A component that retains the store of `branch` while it is mounted. */
function retain(branch: Accessor<string> | string): { store: Accessor<QueueStore>; dispose: () => void } {
  const read = typeof branch === 'string' ? () => branch : branch;
  let store!: Accessor<QueueStore>;
  const dispose = createRoot((d) => {
    store = useQueueStore(read);
    return d;
  });
  disposers.push(dispose);
  return { store, dispose };
}

/** Runs what is due now: pending promises and zero-delay timers, nothing later. */
async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await vi.advanceTimersByTimeAsync(0);
}

let hidden = false;
function setHidden(value: boolean): void {
  hidden = value;
  document.dispatchEvent(new Event('visibilitychange'));
}

beforeEach(() => {
  vi.useFakeTimers({ now: T0 });
  hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => (hidden ? 'hidden' : 'visible'),
  });
});

afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose();
  vi.clearAllTimers();
  vi.useRealTimers();
  document.body.innerHTML = '';
  window.history.replaceState(null, '', '/');
});

// ── S1, S4: one store per branch, one request every 5 s while retained ───────

describe('S1: polling kiosk_notifications every 5 s while retained', () => {
  it('queueStore is lazy and one per branch: the same object for the same branch, and creating it sends nothing', async () => {
    const { branch, server, store } = setup();
    expect(queueStore(branch)).toBe(store);
    expect(queueStore(freshBranch(), { api: server.api as unknown as ApiClient, session: fakeSession().session })).not.toBe(store);

    await vi.advanceTimersByTimeAsync(20_000);
    expect(server.calls).toHaveLength(0);
  });

  it('polls at once when retained, then every 5 s with the last version as since, and stops when released', async () => {
    const { branch, server } = setup();
    const { dispose } = retain(branch);
    await flush();
    expect(server.names).toEqual(['kiosk_notifications']);
    expect(server.calls[0]).toEqual({ branch });

    await vi.advanceTimersByTimeAsync(4_999);
    expect(server.calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(server.calls).toHaveLength(2);
    expect(server.calls[1]).toEqual({ branch, since: 7 });

    server.write();
    await vi.advanceTimersByTimeAsync(5_000);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls.map((c) => c.since)).toEqual([undefined, 7, 7, 8]);

    dispose();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(server.calls).toHaveLength(4);
  });

  it('two components on one branch share one store and one request every 5 s; polling stops when the last one goes', async () => {
    const { branch, server, store } = setup();
    const a = retain(branch);
    const b = retain(branch);
    expect(a.store()).toBe(store);
    expect(b.store()).toBe(store);
    await flush();
    expect(server.calls).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(2);

    a.dispose();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(3);

    b.dispose();
    await vi.advanceTimersByTimeAsync(20_000);
    expect(server.calls).toHaveLength(3);
  });

  it('an unchanged answer keeps state and version and re-runs nothing that reads them (O(1)); a write is picked up', async () => {
    const { branch, server } = setup();
    const { store } = retain(branch);
    let stateRuns = 0;
    let versionRuns = 0;
    createRoot((d) => {
      disposers.push(d);
      createEffect(() => {
        store().state();
        stateRuns += 1;
      });
      createEffect(() => {
        store().version();
        versionRuns += 1;
      });
    });
    await flush();
    const first = store().state();
    expect(first).toEqual(server.data);
    expect(store().version()).toBe(7);
    const runs = [stateRuns, versionRuns];

    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls[1]!.since).toBe(7);
    expect(store().state()).toBe(first);
    expect(store().version()).toBe(7);
    expect([stateRuns, versionRuns]).toEqual(runs);

    server.write({ count: 2, latest: 'kq-2' });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store().version()).toBe(8);
    expect(store().state()?.count).toBe(2);
    expect(store().state()?.latest).toBe('kq-2');
  });

  it('runs on the server clock: now() and lastUpdated add the offset of the last changed answer; an unchanged answer keeps the offset', async () => {
    const { branch, server } = setup();
    const { store } = retain(branch);
    expect(store().now()).toBe(T0);
    expect(store().lastUpdated()).toBeUndefined();
    expect(store().version()).toBeUndefined();

    await flush();
    expect(store().now()).toBe(T0 + SKEW);
    expect(store().lastUpdated()).toBe(T0 + SKEW);

    await vi.advanceTimersByTimeAsync(5_000); // unchanged: no serverNow, the offset stays
    expect(server.calls[1]!.since).toBe(7);
    expect(store().lastUpdated()).toBe(T0 + 5_000 + SKEW);
    expect(store().now()).toBe(T0 + 5_000 + SKEW);

    await vi.advanceTimersByTimeAsync(1_234);
    expect(store().now()).toBe(T0 + 6_234 + SKEW);
    expect(store().lastUpdated()).toBe(T0 + 5_000 + SKEW);

    // the server's clock is now set right: the next changed answer brings the offset to 0
    server.write();
    server.data = { ...server.data, serverNow: new Date(T0 + 10_000).toISOString() };
    await vi.advanceTimersByTimeAsync(10_000 - 6_234);
    expect(store().version()).toBe(8);
    expect(store().now()).toBe(T0 + 10_000);
    expect(store().lastUpdated()).toBe(T0 + 10_000);
  });
});

// ── S2: not ViewPoller — the chime is for a tab in the background ────────────

describe('S2: the store keeps polling while the tab is hidden', () => {
  it('a hidden tab still sends one request every 5 s', async () => {
    const { branch, server } = setup();
    retain(branch);
    await flush();
    setHidden(true);
    await flush();
    const before = server.calls.length;

    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(before + 1);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(before + 2);
  });
});

// ── S3: extra polls ──────────────────────────────────────────────────────────

describe('S3: it also polls at once on focus, on becoming visible, 100 ms after a change, and on refresh()', () => {
  it('window focus polls at once', async () => {
    const { branch, server } = setup();
    retain(branch);
    await flush();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(server.calls).toHaveLength(1);

    window.dispatchEvent(new Event('focus'));
    await flush();
    expect(server.calls).toHaveLength(2);
    expect(server.calls[1]).toEqual({ branch, since: 7 });
  });

  it('becoming visible polls at once; becoming hidden does not', async () => {
    const { branch, server } = setup();
    retain(branch);
    await flush();
    await vi.advanceTimersByTimeAsync(1_000);

    setHidden(true);
    await flush();
    expect(server.calls).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1_000);
    setHidden(false);
    await flush();
    expect(server.calls).toHaveLength(2);
  });

  it('a change event anywhere in the document polls 100 ms later, not before', async () => {
    const { branch, server } = setup();
    retain(branch);
    await flush();
    await vi.advanceTimersByTimeAsync(1_000);

    const select = document.createElement('select');
    document.body.append(select);
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.advanceTimersByTimeAsync(99);
    expect(server.calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(server.calls).toHaveLength(2);
  });

  it('refresh() polls at once and resolves once the answer is in', async () => {
    const { branch, server } = setup();
    const { store } = retain(branch);
    await flush();
    await vi.advanceTimersByTimeAsync(1_000);

    server.write({ count: 4 });
    await store().refresh();
    expect(server.calls).toHaveLength(2);
    expect(store().state()?.count).toBe(4);
    expect(store().version()).toBe(8);
  });
});

// ── S4: single flight ────────────────────────────────────────────────────────

describe('S4: one request in flight per store', () => {
  it('a tick, a focus, a change and refresh() while a request runs are skipped, not queued', async () => {
    const { branch, server } = setup();
    const release = server.hold();
    const { store } = retain(branch);
    await flush();
    expect(server.calls).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(5_000);
    window.dispatchEvent(new Event('focus'));
    document.body.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.advanceTimersByTimeAsync(100);
    void store().refresh();
    await flush();
    expect(server.calls).toHaveLength(1);

    release();
    await flush();
    expect(store().version()).toBe(7);
    expect(server.calls).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(2);
  });
});

// ── S5: offline ──────────────────────────────────────────────────────────────

describe('S5: any other error is offline, and the timer keeps trying', () => {
  it.each([
    ['a 502', new ApiError({ code: 'httpError', message: 'Bad gateway', status: 502 })],
    ['a network failure', new TypeError('Failed to fetch')],
    ['a refusal other than kioskStaff', new ApiError({ code: 'invalidRequest', message: 'x', status: 400 })],
  ])('%s sets error {kind: offline}; the next success clears it', async (_name, failure) => {
    const { branch, server } = setup();
    server.fail = failure;
    const { store } = retain(branch);
    await flush();
    expect(store().error()).toEqual({ kind: 'offline' });

    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(2);
    expect(store().error()).toEqual({ kind: 'offline' });

    server.fail = undefined;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls).toHaveLength(3);
    expect(store().error()).toBeUndefined();
    expect(store().state()).toEqual(server.data);
  });
});

// ── S6, S7: the store ends, and never signs out ──────────────────────────────

async function expectStopped(server: FakeServer, store: QueueStore): Promise<void> {
  const sent = server.calls.length;
  await vi.advanceTimersByTimeAsync(30_000);
  window.dispatchEvent(new Event('focus'));
  setHidden(true);
  setHidden(false);
  document.body.dispatchEvent(new Event('change', { bubbles: true }));
  await vi.advanceTimersByTimeAsync(200);
  await store.refresh();
  await flush();
  expect(server.calls).toHaveLength(sent);
}

describe('S6: a 401 or kioskStaff ends the store as expired', () => {
  it.each([
    ['a 401', new ApiError({ code: 'signIn', message: 'Please sign in', status: 401 })],
    ['kioskStaff', new ApiError({ code: 'kioskStaff', message: 'Staff access required.', status: 400 })],
  ])('%s sets error {kind: ended, reason: expired}, stops polling, and signs nobody out', async (_name, failure) => {
    const { branch, server, signOut, store } = setup();
    retain(branch);
    await flush();
    server.fail = failure;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'expired' });

    await expectStopped(server, store);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'expired' });
    expect(signOut).not.toHaveBeenCalled();
    expect(server.posts).toEqual([]);
  });
});

describe('S7: an account switch ends the store; a new me drops since', () => {
  it.each([
    ['another id', { id: 'E2', role: 'FRONT' }],
    ['the same id with another role', { id: 'E1', role: 'OWNER' }],
  ])('an answer naming %s sets error {kind: ended, reason: switched} and stops polling', async (_name, employee) => {
    const { branch, server, signOut, store } = setup();
    retain(branch);
    await flush();
    expect(store.error()).toBeUndefined();

    server.write({ employee });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'switched' });

    await expectStopped(server, store);
    expect(signOut).not.toHaveBeenCalled();
    expect(server.posts).toEqual([]);
  });

  it("when me()'s id or role changes the next poll has no since, so its answer carries employee; a me() with the same id and role keeps since", async () => {
    const { branch, server, setMe, store } = setup();
    retain(branch);
    await flush();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls.map((c) => c.since)).toEqual([undefined, 7]);

    setMe({ ...ME, name: 'Somchai J.' });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls[2]!.since).toBe(7);

    setMe({ ...ME, id: 'E2', username: 'malee' });
    server.data = { ...server.data, employee: { id: 'E2', role: 'FRONT' } };
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls[3]!.since).toBeUndefined();
    expect(store.error()).toBeUndefined();

    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls[4]!.since).toBe(7);

    setMe({ ...ME, id: 'E2', role: 'MANAGER' });
    server.data = { ...server.data, employee: { id: 'E2', role: 'MANAGER' } };
    await vi.advanceTimersByTimeAsync(5_000);
    expect(server.calls[5]!.since).toBeUndefined();
    expect(store.error()).toBeUndefined();
  });
});

// ── useQueueStore follows its branch ─────────────────────────────────────────

describe('useQueueStore', () => {
  it('follows its branch accessor: the new branch is retained and polled, the old one is released', async () => {
    const one = setup();
    const two = setup();
    const [branch, setBranch] = createSignal(one.branch);
    const { store } = retain(branch);
    await flush();
    expect(store()).toBe(one.store);
    expect(one.server.calls).toHaveLength(1);

    setBranch(two.branch);
    await flush();
    expect(store()).toBe(two.store);
    expect(two.server.calls).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(10_000);
    expect(one.server.calls).toHaveLength(1);
    expect(two.server.calls).toHaveLength(3);
  });
});

// ── S8: which branch the card follows ────────────────────────────────────────

describe('S8: queueBranch follows v2 currentBranch() (account-bar.js:5)', () => {
  it('setQueueBranch, else ?branch=, else me().branch, else undefined — and it is reactive to setQueueBranch', () => {
    window.history.replaceState(null, '', '/kiosk-queue.html');
    shared.setMe!(undefined);
    expect(queueBranch()).toBeUndefined();

    shared.setMe!({ ...ME, branch: 'B-ME' });
    expect(queueBranch()).toBe('B-ME');

    window.history.replaceState(null, '', '/kiosk-queue.html?branch=B-URL');
    expect(queueBranch()).toBe('B-URL');

    const seen: Array<string | undefined> = [];
    createRoot((d) => {
      disposers.push(d);
      createEffect(() => seen.push(queueBranch()));
    });
    setQueueBranch('B-SET');
    expect(queueBranch()).toBe('B-SET');
    setQueueBranch('B-SET2');
    expect(queueBranch()).toBe('B-SET2');
    expect(seen).toEqual(['B-URL', 'B-SET', 'B-SET2']);

    window.history.replaceState(null, '', '/kiosk-queue.html?branch=B-URL2');
    shared.setMe!({ ...ME, branch: 'B-ME2' });
    expect(queueBranch()).toBe('B-SET2');
  });
});
