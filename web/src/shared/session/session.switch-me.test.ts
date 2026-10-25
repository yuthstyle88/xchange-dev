// #5087 (P19) row 3: `switchBranch` applies the `Me` that `POST /api/auth/session/branch` answers
// (the route answers the new `Me` as the whole body, api/src/presentation/http/auth.rs `v3_session_branch`).
// Up-front test written before the fix by the test writer claude/studio-claude-5 on the order of the lane
// controller claude/8be3a451. Do not edit, delete or weaken; add tests in a file of your own.
//
// Root cause (main 2bd6d41a): `switchBranch` (session.ts:191-193) awaits the post and drops the answer, so
// `me()` keeps the old branch until a reload that, on a hash-routed screen, never comes.
//
// What this file pins: after `await store.switchBranch('B2')` resolves, `store.me()` is the answered `Me`
// (the store's `updateMe(answer)`), and the switch does not end the session. Checked through `me()`, not by
// spying on `updateMe`, so the fix may call `updateMe` or its signal directly.
//
// Store set-up follows session.branch.test.ts.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type ViewAnswer } from '../api/client';
import { makeBranchRef, type BranchRef, type Me } from '../testing/fixtures';
import { createSessionStore } from './session';
import { meOf } from '../../test-support/me';

type MeSB = Me & { branchName: string; branches: BranchRef[] };

const HQ = makeBranchRef({ id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' });
const B2 = makeBranchRef({ id: 'B2', code: 'XC-02', name: 'สาขาสอง' });

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

function fakeApi(answer: Me) {
  const posted: Array<{ path: string; body: unknown }> = [];
  const api = {
    post: async <B, R>(path: string, body: B): Promise<R> => {
      posted.push({ path, body });
      return answer as unknown as R;
    },
    view: async <U>(): Promise<ViewAnswer<U>> => ({ version: 4, changed: true, data: atMain }) as never,
  };
  return { api, posted };
}

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('#5087 session store: switchBranch applies the answered Me', () => {
  it('row 3: after the switch resolves, me() is the Me the route answered (updateMe), and the session has not ended', async () => {
    const { api, posted } = fakeApi(atB2);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.me()).toBe(atMain);

    await store.switchBranch!('B2');

    expect(posted).toEqual([{ path: '/api/auth/session/branch', body: { branch: 'B2' } }]);
    expect(store.me(), 'me() is the answered Me, not the held one').toBe(atB2);
    expect(store.me()?.branch).toBe('B2');
    expect(store.ended()).toBeUndefined();
    store.dispose();
  });
});
