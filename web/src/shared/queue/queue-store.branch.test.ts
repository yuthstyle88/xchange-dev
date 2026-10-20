// SB-3 (#842): `queueBranch()` picks its branch the way every page does, and names no branch of its
// own (design docs/design/blueprint/org-roles-branches.md §14.4; #842 §8 row `queueBranch()`).
// Written before the implementation by the test-writer claude/5d45b017, on the FE lane controller
// claude/5edf7bb5's order. Do not edit, delete or weaken; add tests in a file of your own.
//
// The rule pinned here ([required]):
//   1. `setQueueBranch(b)`;
//   2. else `?branch=`;
//   3. else `me().branch` of the shared session;
//   4. else `undefined`, never 'MAIN'.
// While it is `undefined`, a queue store following it (`useQueueStore(queueBranch)`, as the shell's
// queue card does) reads nothing; once `me` answers it reads that branch.
//
// The shared session is faked with a `me` signal, as in `queue-store.test.ts`; the store's own
// `ApiClient` goes through the global `fetch`, which is stubbed to record the URLs read.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)`
// (`web/src/test-support/me.ts`, design §3.3), after main's SB-3 tests reached this branch. Nothing else changes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Accessor } from 'solid-js';
import type { SessionStore } from '../session/session';
import { meOf } from '../../test-support/me';
import { makeBranchRef, type Me } from '../testing/fixtures';
import { warmModules } from '../testing/warm-modules';

const shared = vi.hoisted(() => ({ setMe: undefined as undefined | ((me: Me | undefined) => void) }));
vi.mock('../session/session', async (importOriginal) => {
  const original = await importOriginal<typeof import('../session/session')>();
  const { createSignal: signal } = await import('solid-js');
  const [me, setMe] = signal<Me | undefined>(undefined);
  shared.setMe = (value) => setMe(() => value);
  const session: SessionStore = { me, signOut: async () => {}, dispose: () => {} };
  return { ...original, sharedSession: () => session };
});

const ME: Me = meOf({
  id: 'E1',
  role: 'FRONT',
  branch: 'B-ME',
  name: 'Somchai',
  branchName: 'B-ME',
  branches: [makeBranchRef({ id: 'B-ME', code: 'B-ME', name: 'B-ME' })],
});

let urls: string[];
const disposers: Array<() => void> = [];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function queueReadBranches(): string[] {
  return urls
    .filter((u) => u.startsWith('/api/v3/views/queue_board'))
    .map((u) => new URL(u, 'http://localhost').searchParams.get('branch') ?? '<none>');
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function load() {
  vi.resetModules();
  const mod = await import('./queue-store');
  return mod as unknown as {
    queueBranch: Accessor<string | undefined>;
    setQueueBranch: (b: string) => void;
    useQueueStore: (b: Accessor<string | undefined>) => Accessor<unknown>;
  };
}

beforeAll(() => warmModules(() => import('./queue-store')), 60_000);

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-10T05:00:00Z') });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
  urls = [];
  shared.setMe?.(undefined);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown) => {
      const u = String(input);
      urls.push(u);
      return json({
        version: 7,
        changed: true,
        data: {
          waiting: [],
          called: [],
          myCounter: null,
        },
      });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete (document as unknown as Record<string, unknown>).hidden;
  delete (document as unknown as Record<string, unknown>).visibilityState;
  window.history.replaceState(null, '', '/');
});

describe('SB-3 queueBranch', () => {
  it("nothing names a branch (no setQueueBranch, no ?branch=, no me): undefined, never 'MAIN'", async () => {
    window.history.replaceState(null, '', '/kiosk-queue.html');
    const { queueBranch } = await load();
    shared.setMe!(undefined);
    expect(queueBranch()).toBeUndefined();
  });

  it('the order: setQueueBranch, else ?branch=, else me().branch', async () => {
    window.history.replaceState(null, '', '/');
    const { queueBranch, setQueueBranch } = await load();
    shared.setMe!({ ...ME, branch: 'B-ME' });
    expect(queueBranch()).toBe('B-ME');
    window.history.replaceState(null, '', '/?branch=B-URL');
    expect(queueBranch()).toBe('B-URL');
    setQueueBranch('B-SET');
    expect(queueBranch()).toBe('B-SET');
  });

  it('while queueBranch() is undefined the store following it reads nothing; once me answers it reads that branch', async () => {
    window.history.replaceState(null, '', '/');
    const { queueBranch, useQueueStore } = await load();
    shared.setMe!(undefined);
    createRoot((d) => {
      disposers.push(d);
      useQueueStore(queueBranch);
    });
    await flush();
    await vi.advanceTimersByTimeAsync(15_000);
    expect(queueReadBranches(), 'no read before any branch is known').toEqual([]);

    shared.setMe!({ ...ME, branch: 'B-HOME' });
    await flush();
    expect(queueReadBranches().length, 'the store read once me answered').toBeGreaterThan(0);
    expect(new Set(queueReadBranches())).toEqual(new Set(['B-HOME']));
  });
});
