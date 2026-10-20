import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
//
// Amended for QU-6 (#1354; this directory is the ticket's) by the implementer claude/709ec749: the store polls
// `queue_board`, so the fixture is `QueueBoardData` (one waiting ticket kq-1). Fixture only; no assertion changes.
import { createRoot, createSignal, type Accessor } from 'solid-js';
import type { ApiClient, ViewParams } from '../api/client';
import type { SessionStore } from '../session/session';
import {
  makeQueueBoardData,
  makeQueueBoardWaiting,
  makeViewAnswer,
  type Me,
  type QueueBoardData,
  type ViewAnswer,
} from '../testing/fixtures';
import { queueStore, useQueueStore, type QueueStore } from './queue-store';
import { meOf } from '../../test-support/me';

const T0 = Date.parse('2026-10-08T03:00:00.000Z');

const ME: Me = meOf({
  id: 'E1',
  role: 'FRONT',
  branch: 'B-HOME',
  name: 'Somchai',
  branchName: 'B-HOME',
  branches: [],
});

let branchNo = 1000;
function freshBranch(): string {
  branchNo += 1;
  return `QS-GAP-${branchNo}`;
}

function board(over: Partial<QueueBoardData> = {}) {
  return makeQueueBoardData({
    waiting: [
      makeQueueBoardWaiting({
        id: 'kq-1',
        displayNumber: 'W001',
        serviceType: 'WALK_IN',
        sequence: 1,
        issuedAt: new Date(T0).toISOString(),
      }),
    ],
    called: [],
    myCounter: null,
    ...over,
  });
}

class FakeServer {
  readonly calls: ViewParams[] = [];
  readonly names: string[] = [];
  version = 7;
  data: QueueBoardData;

  constructor(readonly branch: string) {
    this.data = board();
  }

  write(over: Partial<QueueBoardData> = {}): void {
    this.version += 1;
    this.data = board(over);
  }

  readonly api = {
    view: async <T>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      this.names.push(name);
      this.calls.push({ ...params });
      if (params.since !== undefined && params.since === this.version) {
        return makeViewAnswer(undefined as unknown as T, { version: this.version, changed: false });
      }
      return makeViewAnswer(this.data as unknown as T, { version: this.version, changed: true });
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
