// SB-2 (session branch, web half): the session store carries the session branch, notices a switch
// made in another tab, and sends the switch command (design
// docs/design/blueprint/org-roles-branches.md §14.4 "The session store", §14.3 the command route).
// Written before the implementation by the ticket author (claude/dell-claude-3). Do not edit,
// delete or weaken; add tests in a file of your own.
//
// The API this file pins, in `web/src/shared/session/session.ts`:
//
//   export type EndReason = 'expired' | 'switched' | 'branchChanged';            [required] the new value
//   createSessionStore(api) returns a store that also has
//     switchBranch(branch: string): Promise<void>                                [proposed] name
//       api.post('/api/auth/session/branch', { branch }) — `ApiClient.post` sends the
//       `X-Xchange-Request: 1` marker the route requires; it rejects with the post's error.
//
// A `me` answer with `changed: true` whose `id` and `roles` are the held ones but whose `branch`
// differs ends the store as 'branchChanged' (switched in another tab), keeping the held `me()`, as
// 'switched' does (session.end.test.ts SE2). Another account wins: a new `id` ends it as 'switched'.
//
// `MeSB` spells out SB-1's `Me` fields so this file type-checks before SB-1 regenerates `common.ts`.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixtures (`kind`/`role`/`username`) → `meOf(…)` (`web/src/test-support/me.ts`, design §3.3). Nothing else changes. The ADMIN's row is organisation-wide; `otherAtB2` drops `username`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type ViewAnswer, type ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import { createSessionStore } from './session';
import { meOf } from '../../test-support/me';

type BranchRef = { id: string; code: string; name: string };
type MeSB = Me & { branchName: string; branches: BranchRef[] };

const HQ: BranchRef = { id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' };
const B2: BranchRef = { id: 'B2', code: 'XC-02', name: 'สาขาสอง' };

const atMain: MeSB = meOf({
  id: 'u-1',
  role: 'ADMIN',
  orgWide: true,
  branch: 'MAIN',
  name: 'Somchai',
  branchName: 'สำนักงานใหญ่',
  branches: [HQ, B2],
});
const atB2: MeSB = { ...atMain, branch: 'B2', branchName: 'สาขาสอง' };
const otherAtB2: MeSB = { ...atB2, id: 'u-2' };
const moreBranches: MeSB = { ...atMain, branches: [HQ, B2, { id: 'B3', code: 'XC-03', name: 'สาขาสาม' }] };

type Switching = { switchBranch(branch: string): Promise<void> };

function switcher(store: object): Switching['switchBranch'] {
  const fn = (store as Partial<Switching>).switchBranch;
  expect(typeof fn, 'the store has switchBranch').toBe('function');
  return fn!.bind(store);
}

function fakeApi(script: Array<ViewAnswer<Me> | Error>, postFails?: Error) {
  const seen: ViewParams[] = [];
  const posted: Array<{ path: string; body: unknown }> = [];
  let i = 0;
  const api = {
    post: async <B, R>(path: string, body: B): Promise<R> => {
      posted.push({ path, body });
      if (postFails) throw postFails;
      return atB2 as unknown as R;
    },
    view: async <U>(_name: string, params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      seen.push({ ...params });
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      if (next instanceof Error) throw next;
      return next as unknown as ViewAnswer<U>;
    },
  };
  return { api, seen, posted };
}

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('SB-2 session store: switched in another tab', () => {
  it("ends as 'branchChanged' when the same account answers with another branch, keeping the held me()", async () => {
    const { api } = fakeApi([
      { version: 4, changed: true, data: atMain },
      { version: 5, changed: true, data: atB2 },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.ended()).toBeUndefined();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBe('branchChanged');
    expect(store.me()).toBe(atMain);
    store.dispose();
  });

  it('stops polling once ended that way', async () => {
    const { api, seen } = fakeApi([
      { version: 4, changed: true, data: atMain },
      { version: 5, changed: true, data: atB2 },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    const after = seen.length;
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(after);
    store.dispose();
  });

  it("another account with another branch ends as 'switched', not 'branchChanged'", async () => {
    const { api } = fakeApi([
      { version: 4, changed: true, data: atMain },
      { version: 5, changed: true, data: otherAtB2 },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBe('switched');
    store.dispose();
  });

  it('the same branch with a longer branch list (a grant added) does not end the session, and me() follows it', async () => {
    const { api } = fakeApi([
      { version: 4, changed: true, data: atMain },
      { version: 5, changed: true, data: moreBranches },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBeUndefined();
    expect(store.me()).toBe(moreBranches);
    store.dispose();
  });

  it('the first answer never ends the session, whatever its branch', async () => {
    const { api } = fakeApi([{ version: 4, changed: true, data: atB2 }]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.ended()).toBeUndefined();
    expect(store.me()).toBe(atB2);
    store.dispose();
  });
});

describe('SB-2 session store: switchBranch (§14.3 POST /api/auth/session/branch)', () => {
  it('posts { branch } to /api/auth/session/branch through api.post, once', async () => {
    const { api, posted } = fakeApi([{ version: 4, changed: true, data: atMain }]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await switcher(store)('B2');
    expect(posted).toEqual([{ path: '/api/auth/session/branch', body: { branch: 'B2' } }]);
    store.dispose();
  });

  it('rejects with the refusal (403 outOfScope) and does not end the session', async () => {
    const refusal = new ApiError({ code: 'outOfScope', message: 'refused', status: 403 });
    const { api } = fakeApi([{ version: 4, changed: true, data: atMain }], refusal);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await expect(switcher(store)('B9')).rejects.toBe(refusal);
    expect(store.ended()).toBeUndefined();
    expect(store.me()).toBe(atMain);
    store.dispose();
  });
});
