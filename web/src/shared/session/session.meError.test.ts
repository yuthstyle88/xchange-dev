// G-ME, GME-0 (`docs/design/web-silent-failures.md` §5): the session store knows when its own `me` read failed.
// Written before the implementation by the ticket author (claude/e8020026's sub-agent). Do not edit, delete or weaken;
// add tests in a file of your own.
//
// The API this pins, in `web/src/shared/session/session.ts` (signatures from the skeleton):
//   - `meError(): ApiError | null`: the last failure of the session's `me` read; a thrown `ApiError` as is, anything
//     else (a network `TypeError`) as `ApiError{code:'network', status:0}`; cleared by the next good read
//     (`changed: true` or `false`). Reading it never sends a request.
//   - `retryMe(): void`: re-reads `me` at once through the session's own poller. No second reader: the store sends no
//     `me` request of its own besides the poller's, and after a retry the next scheduled read is one interval later
//     (no doubled schedule). It does nothing after `signOut()` or once the session has ended.
// Today's behaviour stays: `me()` keeps the last good answer through a failure; a 60 s interval; a 401 after a good
// read ends the session as `expired`.
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

/** A fake `ApiClient` (as in session.test.ts): `view` answers from a script, one entry per call (the last repeats). */
function fakeApi(script: Array<ViewAnswer<Me> | Error>) {
  const seen: Seen[] = [];
  let i = 0;
  const api = {
    post: async <B, R>(_path: string, _body: B): Promise<R> => ({ saved: true }) as unknown as R,
    view: async <U>(name: string, params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      seen.push({ at: Date.now(), name, params: { ...params } });
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      if (next instanceof Error) throw next;
      return next as never;
    },
  };
  return { api, seen };
}

const teller: Me = meOf({ id: 'e-7', role: 'TELLER', branch: 'XC-B01', name: null, branchName: 'XC-B01', branches: [] });
const ok = (version = 4) => ({ version, changed: true, data: teller });
const same = (version = 4) => ({ version, changed: false });
const down = () => new ApiError({ code: 'serverError', message: 'database down', status: 500 });
const offline = () => new TypeError('Failed to fetch');
const signIn = () => new ApiError({ code: 'signIn', message: 'กรุณาเข้าสู่ระบบ', status: 401 });

const meCalls = (seen: Seen[]) => seen.filter((s) => s.name === 'me');

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('meError(): the last failure of the session’s me read (GME-0)', () => {
  it('is null before the first answer and after a good one', async () => {
    const { api } = fakeApi([ok()]);
    const store = createSessionStore(api);
    expect(store.meError()).toBeNull();
    await vi.advanceTimersByTimeAsync(0);
    expect(store.me()).toBe(teller);
    expect(store.meError()).toBeNull();
    store.dispose();
  });

  it('a failed first read sets it to that ApiError (status and code kept)', async () => {
    const err = down();
    const { api, seen } = fakeApi([err]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(meCalls(seen)).toHaveLength(1);
    expect(store.me()).toBeUndefined();
    const got = store.meError();
    expect(got).toBeInstanceOf(ApiError);
    expect(got!.status).toBe(500);
    expect(got!.code).toBe('serverError');
    store.dispose();
  });

  it('a network failure (a raw TypeError) becomes ApiError{code:"network", status:0}', async () => {
    const { api } = fakeApi([offline()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    const got = store.meError();
    expect(got).toBeInstanceOf(ApiError);
    expect(got!.code).toBe('network');
    expect(got!.status).toBe(0);
    store.dispose();
  });

  it('a 401 on the first read is kept as meError with status 401', async () => {
    const { api } = fakeApi([signIn()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.meError()?.status).toBe(401);
    store.dispose();
  });

  it('is cleared by the next good read on the poller’s own schedule (60 s later)', async () => {
    const { api, seen } = fakeApi([down(), ok()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.meError()).not.toBeNull();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(meCalls(seen)).toHaveLength(2);
    expect(store.meError()).toBeNull();
    expect(store.me()).toBe(teller);
    store.dispose();
  });

  it('is cleared by a changed: false answer too', async () => {
    const { api } = fakeApi([ok(), down(), same()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.meError()?.status).toBe(500);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.meError()).toBeNull();
    store.dispose();
  });

  it('a failure after a good read sets it, and me() keeps the last good answer (today’s behaviour)', async () => {
    const { api } = fakeApi([ok(), down()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.meError()?.code).toBe('serverError');
    expect(store.me()).toBe(teller);
    expect(store.ended()).toBeUndefined();
    store.dispose();
  });

  it('reading it sends no request', async () => {
    const { api, seen } = fakeApi([down()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    const before = seen.length;
    for (let i = 0; i < 5; i++) store.meError();
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(before);
    store.dispose();
  });
});

describe('retryMe(): re-reads me at once through the poller, with no second reader (GME-0)', () => {
  it('sends exactly one me read at once, and a good answer clears meError and fills me()', async () => {
    const { api, seen } = fakeApi([down(), ok()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(meCalls(seen)).toHaveLength(1);

    vi.setSystemTime(5_000);
    store.retryMe();
    await vi.advanceTimersByTimeAsync(0);
    expect(meCalls(seen)).toHaveLength(2);
    expect(seen[1]!.at).toBe(5_000);
    expect(store.meError()).toBeNull();
    expect(store.me()).toBe(teller);
    store.dispose();
  });

  it('a retry that fails again keeps meError set (the new failure)', async () => {
    const { api } = fakeApi([down(), offline()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    store.retryMe();
    await vi.advanceTimersByTimeAsync(0);
    expect(store.meError()?.code).toBe('network');
    store.dispose();
  });

  it('after a retry the poller reads again one interval later, once: no doubled schedule', async () => {
    const { api, seen } = fakeApi([down(), ok(), same()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(10_000);
    store.retryMe();
    await vi.advanceTimersByTimeAsync(0);
    expect(meCalls(seen).map((s) => s.at)).toEqual([0, 10_000]);

    await vi.advanceTimersByTimeAsync(120_000);
    // One read per 60 s from the retry on: 70 000 and 130 000. The schedule set before the retry (60 000) is gone.
    expect(meCalls(seen).map((s) => s.at)).toEqual([0, 10_000, 70_000, 130_000]);
    store.dispose();
  });

  it('every me request comes from the poller: open + retry + 60 s is exactly three reads', async () => {
    const { api, seen } = fakeApi([down(), ok(), same()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    store.retryMe();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(meCalls(seen)).toHaveLength(3);
    expect(seen.every((s) => s.name === 'me')).toBe(true);
    store.dispose();
  });

  it('does nothing after signOut()', async () => {
    const { api, seen } = fakeApi([down()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await store.signOut();
    const before = seen.length;
    store.retryMe();
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(before);
    store.dispose();
  });

  it('does nothing once the session has ended', async () => {
    const { api, seen } = fakeApi([ok(), signIn()]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBe('expired');
    const before = seen.length;
    store.retryMe();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(seen).toHaveLength(before);
    store.dispose();
  });
});
