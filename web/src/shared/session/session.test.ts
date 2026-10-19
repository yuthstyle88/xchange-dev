// V0-B: SessionStore (docs/design/xchange-v3-web.md §5.2; ticket #393 §7.8).
// Written before the implementation. Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/shared/session/session.ts`:
//
//   export interface SessionStore {
//     me: Accessor<Me | undefined>;   // the `me` view's data; undefined until the first answer and after signOut
//     signOut(): Promise<void>;       // api.post('/api/auth/logout', {}), then me() is undefined and polling stops
//     dispose(): void;                // stops polling
//   }
//   export function createSessionStore(api: Pick<ApiClient, 'view' | 'post'>): SessionStore;
//
// It polls the `me` view through V0-A's `ViewPoller` from the moment it is created: no `branch`
// (the server answers staff for their own branch and the owner for MAIN), every 60 s. `Me` changes
// only when someone signs in or out, or a role or branch is reassigned; a session lasts 12 h
// (`SESSION_SECONDS` = 43 200, api/src/presentation/extractors/actor.rs:27). Once a minute notices a
// sign-out from another tab within a minute and costs one indexed read of `branch_versions` per tab.
//
// Sign-out goes to the Identity BFF's `/api/auth/logout`, which ends the `__Host-xc_session` row and clears it plus
// the old owner and staff cookies (api/src/presentation/http/auth.rs:181-189, :477-490 @ 8234e77). The old
// `/api/auth` action `logout` clears only the owner and staff cookies (:556-563), so it would leave a person signed
// in through the new login screen still signed in (amended by the OR-3 web follow-up #762, claude/b060e9). It goes through `ApiClient.post`, which sends
// `X-Xchange-Request: 1` (pinned in ../api/client.post.test.ts).
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixtures (`kind`/`role`/`username`) → `meOf(…)` (`web/src/test-support/me.ts`, design §3.3). Nothing else changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type ViewAnswer, type ViewParams } from '../api/client';
import type { Me } from '../testing/fixtures';
import { createSessionStore } from './session';
import { meOf } from '../../test-support/me';

interface Seen {
  at: number;
  name: string;
  params: ViewParams;
}

interface Posted {
  path: string;
  body: unknown;
}

/**
 * A fake `ApiClient`: `view` answers from a script, one entry per call (the last repeats);
 * `post` records its call and answers `{ saved: true }`.
 */
function fakeApi(script: Array<ViewAnswer<Me> | Error>) {
  const seen: Seen[] = [];
  const posted: Posted[] = [];
  let i = 0;
  const api = {
    post: async <B, R>(path: string, body: B): Promise<R> => {
      posted.push({ path, body });
      return { saved: true } as unknown as R;
    },
    view: async <U>(name: string, params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      seen.push({ at: Date.now(), name, params: { ...params } });
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      if (next instanceof Error) throw next;
      return next as never;
    },
  };
  return { api, seen, posted };
}

const owner: Me = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: null, branchName: 'MAIN', branches: [] });
const rates: Me = meOf({ id: 'e-7', role: 'RATES', branch: 'XC-B01', name: null, branchName: 'XC-B01', branches: [] });

const signIn = () =>
  new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status: 401 });

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('createSessionStore', () => {
  it('is undefined before the first answer, then is the me view’s data', async () => {
    const { api, seen } = fakeApi([{ version: 4, changed: true, data: owner }]);
    const store = createSessionStore(api);
    expect(store.me()).toBeUndefined();

    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    expect(store.me()).toBe(owner);
    store.dispose();
  });

  it('polls the view named me, with no branch, every 60 s, sending the last version as since', async () => {
    const { api, seen } = fakeApi([
      { version: 4, changed: true, data: rates },
      { version: 4, changed: false },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    expect(seen[0]!.name).toBe('me');
    expect(seen[0]!.params.branch).toBeUndefined();
    expect(seen[0]!.params.since).toBeUndefined();

    await vi.advanceTimersByTimeAsync(59_999);
    expect(seen).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(seen).toHaveLength(2);
    expect(seen[1]!.name).toBe('me');
    expect(seen[1]!.params.since).toBe(4);

    await vi.advanceTimersByTimeAsync(60_000);
    expect(seen.map((s) => s.at)).toEqual([0, 60_000, 120_000]);
    store.dispose();
  });

  it('keeps the same object when the answer is changed: false', async () => {
    const first = { ...owner };
    const { api } = fakeApi([
      { version: 4, changed: true, data: first },
      { version: 4, changed: false },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.me()).toBe(first);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(store.me()).toBe(first);
    store.dispose();
  });

  it('ends as switched, keeping the shown account, when a changed: true answer is another account', async () => {
    // Amended by #492 (design §12.19.3 SE2, D-S3): an owner → RATES answer no longer replaces me().
    const { api } = fakeApi([
      { version: 4, changed: true, data: owner },
      { version: 5, changed: true, data: rates },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.me()).toBe(owner);
    expect(store.ended()).toBe('switched');
    store.dispose();
  });

  it('leaves me undefined on a 401 and does not throw', async () => {
    // An unhandled rejection fails the vitest run on its own.
    const { api, seen } = fakeApi([signIn()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    expect(store.me()).toBeUndefined();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen.length).toBeGreaterThan(1);
    expect(store.me()).toBeUndefined();
    store.dispose();
  });

  it('signOut posts logout to /api/auth/logout through api.post, clears me and stops polling', async () => {
    const { api, seen, posted } = fakeApi([{ version: 4, changed: true, data: owner }]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.me()).toBe(owner);

    await store.signOut();

    expect(posted).toEqual([{ path: '/api/auth/logout', body: {} }]);

    expect(store.me()).toBeUndefined();
    const before = seen.length;
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(before);
    expect(store.me()).toBeUndefined();
  });

  it('makes no request after dispose', async () => {
    const { api, seen } = fakeApi([{ version: 4, changed: true, data: owner }]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    store.dispose();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(1);
  });
});
