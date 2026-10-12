// OR-7.7 (#1030 §3.10, §9): the queue store's "switched" check compares the caller's id and role
// LIST, not the old single role. Spec: #1030 issuecomment-6105551159 (web half, case 1).
// Written before the implementation by the test-writer claude/5d45b017, on the lane controller
// claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//
// The rule pinned here ([required]):
//   - `kiosk_notifications.employee` is `{ id, roles: {role, branchId}[] }` and `me` carries
//     `roles` (#1030 §7);
//   - an answer whose `employee.id` differs from `me().id`, or whose `roles` differ from
//     `me().roles` as sets, ends the store with `{kind: 'ended', reason: 'switched'}`;
//   - the same roles in another order is not a switch; the same role at another branch is one.
//   (The branch case added after claude/dell-claude-6's review, #1030 issuecomment of 05:19Z, gap 1.)
//
// The store runs against a fake `ApiClient` answering `kiosk_notifications` like the server (a
// `since` equal to the version gets `changed: false`), and a fake session whose `me` is a signal,
// as in `queue-store.test.ts`. The fixtures are built untyped (`as unknown as`) so this file runs
// against today's generated types as well as #1030's.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import type { ApiClient, ViewAnswer, ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import type { SessionStore } from '../session/session';
import type { KioskNotifications } from './wire';
import { queueStore, useQueueStore } from './queue-store';

type Role = { role: string; branchId: string | null };

const TELLER_B1: Role = { role: 'TELLER', branchId: 'B1' };
const CASHIER_B1: Role = { role: 'CASHIER', branchId: 'B1' };

function me(roles: Role[]): Me {
  return {
    id: 'U1',
    name: 'Nok',
    roles,
    permissions: ['QueueOperate'],
    branch: 'B1',
    branchName: 'B1',
    branches: [{ id: 'B1', code: 'B1', name: 'B1' }],
  } as unknown as Me;
}

let branchNo = 0;

function notifications(branch: string, employee: { id: string; roles: Role[] }): KioskNotifications {
  return {
    serverNow: new Date().toISOString(),
    branchId: branch,
    branchName: branch,
    count: 0,
    latest: null,
    workingCount: 0,
    working: [],
    employee,
  } as unknown as KioskNotifications;
}

class FakeServer {
  readonly calls: ViewParams[] = [];
  version = 7;
  data: KioskNotifications;

  constructor(readonly branch: string, employee: { id: string; roles: Role[] }) {
    this.data = notifications(branch, employee);
  }

  write(employee: { id: string; roles: Role[] }): void {
    this.version += 1;
    this.data = notifications(this.branch, employee);
  }

  readonly api = {
    view: async <T>(_name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      this.calls.push({ ...params });
      if (params.since !== undefined && params.since === this.version) {
        return { version: this.version, changed: false };
      }
      return { version: this.version, changed: true, data: this.data as unknown as T };
    },
  };
}

const disposers: Array<() => void> = [];

function setup(roles: Role[]) {
  branchNo += 1;
  const branch = `QR-${branchNo}`;
  const server = new FakeServer(branch, { id: 'U1', roles });
  const [meSig, setMe] = createSignal<Me | undefined>(me(roles));
  const session: SessionStore = { me: meSig, signOut: async () => {}, dispose: () => {} };
  const store = queueStore(branch, { api: server.api as unknown as ApiClient, session });
  createRoot((d) => {
    disposers.push(d);
    useQueueStore(() => branch);
  });
  return { branch, server, setMe, store };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await vi.advanceTimersByTimeAsync(0);
}

beforeEach(() => {
  vi.useFakeTimers({ now: Date.parse('2026-10-11T03:00:00.000Z') });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  vi.clearAllTimers();
  vi.useRealTimers();
  delete (document as unknown as Record<string, unknown>).hidden;
  delete (document as unknown as Record<string, unknown>).visibilityState;
});

describe('OR-7.7 queue store: switched follows id and roles', () => {
  it('the roles change between two polls (same id): the queue ends with "switched"', async () => {
    const { server, store } = setup([TELLER_B1]);
    await flush();
    expect(store.error(), 'the first answer matches me').toBeUndefined();
    expect(store.state()).toBeDefined();

    server.write({ id: 'U1', roles: [TELLER_B1, CASHIER_B1] });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'switched' });
  });

  it("me()'s roles change (same id) while the server still answers the old list: switched", async () => {
    const { setMe, store } = setup([TELLER_B1]);
    await flush();
    expect(store.error()).toBeUndefined();

    setMe(me([CASHIER_B1]));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'switched' });
  });

  it('the same roles in another order is not a switch', async () => {
    const { server, store } = setup([TELLER_B1, CASHIER_B1]);
    await flush();
    server.write({ id: 'U1', roles: [CASHIER_B1, TELLER_B1] });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toBeUndefined();
    expect(store.state()?.employee).toEqual({ id: 'U1', roles: [CASHIER_B1, TELLER_B1] });
  });

  it('the same role moved to another branch (same id) is a switch', async () => {
    const { server, store } = setup([TELLER_B1]);
    await flush();
    expect(store.error()).toBeUndefined();
    server.write({ id: 'U1', roles: [{ role: 'TELLER', branchId: 'B2' }] });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'switched' });
  });

  it('another id is a switch', async () => {
    const { server, store } = setup([TELLER_B1]);
    await flush();
    server.write({ id: 'U2', roles: [TELLER_B1] });
    await vi.advanceTimersByTimeAsync(5_000);
    expect(store.error()).toEqual({ kind: 'ended', reason: 'switched' });
  });
});
