import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal, type Accessor } from 'solid-js';
import type { ApiClient, ViewAnswer, ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import type { SessionStore } from '../session/session';
import type { KioskNotifications } from './wire';
import { queueStore, useQueueStore, type QueueStore } from './queue-store';

const T0 = Date.parse('2026-10-08T03:00:00.000Z');
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

let branchNo = 1000;
function freshBranch(): string {
  branchNo += 1;
  return `QS-GAP-${branchNo}`;
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

class FakeServer {
  readonly calls: ViewParams[] = [];
  readonly names: string[] = [];
  version = 7;
  data: KioskNotifications;

  constructor(readonly branch: string) {
    this.data = notifications(branch);
  }

  write(over: Partial<KioskNotifications> = {}): void {
    this.version += 1;
    this.data = notifications(this.branch, over);
  }

  readonly api = {
    view: async <T>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      this.names.push(name);
      this.calls.push({ ...params });
      if (params.since !== undefined && params.since === this.version) {
        return { version: this.version, changed: false };
      }
      return { version: this.version, changed: true, data: this.data as unknown as T };
    },
    post: async () => ({}),
  };
}

function fakeSession(initial?: Me) {
  const [me, setMe] = createSignal<Me | undefined>(initial);
  const signOut = vi.fn(async () => {});
  const session: SessionStore = { me, signOut, dispose: () => {} };
  return { session, setMe, signOut };
}

function setup(me?: Me) {
  const branch = freshBranch();
  const server = new FakeServer(branch);
  const { session, setMe, signOut } = fakeSession(me);
  const store = queueStore(branch, { api: server.api as unknown as ApiClient, session });
  return { branch, server, session, setMe, signOut, store };
}

const disposers: Array<() => void> = [];

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

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await vi.advanceTimersByTimeAsync(0);
}

beforeEach(() => {
  vi.useFakeTimers({ now: T0 });
});

afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose();
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe('queue-store gaps (issue #486 rework)', () => {
  // Mutant: queue-store.ts:214 `if (meNow !== undefined)` -> `if (true)`
  it('a changed answer while me() is undefined must not end the store', async () => {
    const { branch, server, store } = setup(undefined);
    retain(branch);
    await flush();

    expect(store.error()).toBeUndefined();
    expect(store.state()).toEqual(server.data);

    // Write a changed notification while me() remains undefined
    server.write();
    await vi.advanceTimersByTimeAsync(5_000);
    await flush();

    expect(store.error()).toBeUndefined();
    expect(store.version()).toBe(8);
  });

  // Mutant: queue-store.ts:292 `if (b !== prevBranch)` -> `if (true)`
  it('the store is retained once per branch, not on every effect run', async () => {
    const { branch, server } = setup(ME);
    const [trigger, setTrigger] = createSignal(0);
    const branchAccessor = () => {
      trigger();
      return branch;
    };

    retain(branchAccessor);
    await flush();
    expect(server.calls).toHaveLength(1);

    // Re-run the effect by updating a signal read by the branch accessor without changing branch
    setTrigger(1);
    await flush();
    expect(server.calls).toHaveLength(1);

    setTrigger(2);
    await flush();
    expect(server.calls).toHaveLength(1);
  });
});
