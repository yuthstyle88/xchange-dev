// OR-7.7 (#1030 §3.10; lane controller's ruling (a), issuecomment-6105567502): the session counts as
// "switched" when the `me` answer names another id, or another SET of role grants. One helper shared with
// the queue store (♻️). The same grants in another order are not a switch; the same role at another
// branch is (added after claude/dell-claude-6's review on #1030, gap 1).
// Written before the implementation by the test-writer claude/5d45b017. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ViewAnswer, ViewParams } from '../api/client';
import { makeBranchRef, makeMe, makeMeRole, type Me, type MeRole } from '../testing/fixtures';
import { createSessionStore } from './session';

function me(roles: MeRole[], id = 'u-7') {
  return makeMe({
    id,
    name: 'Nok',
    roles,
    permissions: [],
    branch: 'B1',
    branchName: 'B1',
    branches: [makeBranchRef({ id: 'B1', code: 'B1', name: 'B1' })],
  });
}

const TELLER = makeMeRole({ role: 'TELLER', branchId: 'B1' });
const FINANCE = makeMeRole({ role: 'FINANCE', branchId: 'B1' });

function fakeApi(script: Array<ViewAnswer<Me>>) {
  let i = 0;
  const api = {
    post: async <B, R>(_path: string, _body: B): Promise<R> => ({}) as R,
    view: async <U>(_name: string, _params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      return next as never;
    },
  };
  return api;
}

beforeEach(() => {
  vi.useFakeTimers({ now: Date.parse('2026-10-11T03:00:00.000Z') });
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe('OR-7.7 session: switched follows id and the set of role grants', () => {
  it('a changed answer with the same id and a role granted ends it as switched', async () => {
    const first = me([TELLER]);
    const api = fakeApi([
      { version: 4, changed: true, data: first },
      { version: 9, changed: true, data: me([TELLER, FINANCE]) },
    ]);
    const store = createSessionStore(api as never);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended?.()).toBe('switched');
    expect(store.me()).toBe(first);
    store.dispose();
  });

  it('the same role at another branch (same id) ends it as switched', async () => {
    const first = me([TELLER]);
    const api = fakeApi([
      { version: 4, changed: true, data: first },
      { version: 9, changed: true, data: me([makeMeRole({ role: 'TELLER', branchId: 'B2' })]) },
    ]);
    const store = createSessionStore(api as never);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended?.()).toBe('switched');
    expect(store.me()).toBe(first);
    store.dispose();
  });

  it('the same grants in another order replace me and do not end it', async () => {
    const reordered = me([FINANCE, TELLER]);
    const api = fakeApi([
      { version: 4, changed: true, data: me([TELLER, FINANCE]) },
      { version: 5, changed: true, data: reordered },
    ]);
    const store = createSessionStore(api as never);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended?.()).toBeUndefined();
    expect(store.me()).toBe(reordered);
    store.dispose();
  });
});
