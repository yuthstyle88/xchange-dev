// Up-front tests for SF-a (docs/design/web-silent-failures.md §2 SF-1, B1). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// ViewPoller reports its own failures: `error()` is the last failed read as an ApiError (cleared by the next good
// read), `lastOkAt()` is `Date.now()` at the last good read. A 401 is kept as `error()` with status 401. Backoff and
// polling do not change. The harness is view-poller.test.ts's, copied.
import { createMemo, createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type ViewParams } from '../api/client';
import { makeViewAnswer, type ViewAnswer } from '../testing/fixtures';
import { ViewPoller } from './view-poller';

const T0 = 1_000_000;

interface Seen {
  at: number;
  name: string;
  params: ViewParams;
}

/** A fake `ApiClient` whose `view` answers from a script, one entry per call. */
function fakeClient<T>(script: Array<ViewAnswer<T> | Error>) {
  const seen: Seen[] = [];
  let i = 0;
  const client = {
    view: async <U>(name: string, params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      seen.push({ at: Date.now(), name, params: { ...params } });
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      if (next instanceof Error) throw next;
      return makeViewAnswer(next.data as U, { version: next.version, changed: next.changed });
    },
  };
  return { client, seen };
}

function refusal(status: number, code: string): ApiError {
  return new ApiError({ code, message: code, status });
}

const ok = (n: number, version: number) => makeViewAnswer({ n }, { version, changed: true });
const same = (version: number) => makeViewAnswer({ n: 0 }, { version, changed: false, data: undefined });

let hidden = false;

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
  vi.useRealTimers();
});

describe('SF-a: ViewPoller.error()', () => {
  it('is null before any read and after a good first read', async () => {
    const { client } = fakeClient<{ n: number }>([ok(1, 1)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    expect(poller.error()).toBeNull();
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.error()).toBeNull();
    poller.stop();
  });

  it('is the thrown ApiError after a failed read, and is cleared by the next good read', async () => {
    const down = refusal(503, 'serverError');
    const { client } = fakeClient<{ n: number }>([down, ok(1, 1)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.error()).toBe(down);
    expect(poller.error()?.status).toBe(503);
    await vi.advanceTimersByTimeAsync(2000);
    expect(poller.data()).toEqual({ n: 1 });
    expect(poller.error()).toBeNull();
    poller.stop();
  });

  it('a failure after a good read is reported, and the data of the good read stays', async () => {
    const first = { n: 1 };
    const { client } = fakeClient<{ n: number }>([makeViewAnswer(first, { version: 1, changed: true }), refusal(500, 'serverError')]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.error()).toBeNull();
    await vi.advanceTimersByTimeAsync(1000);
    expect(poller.error()?.status).toBe(500);
    expect(poller.data()).toBe(first);
    poller.stop();
  });

  it('a 401 is kept as error() with status 401, and polling goes on', async () => {
    const { client, seen } = fakeClient<{ n: number }>([refusal(401, 'signIn')]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.error()).toBeInstanceOf(ApiError);
    expect(poller.error()?.status).toBe(401);
    expect(poller.error()?.code).toBe('signIn');
    await vi.advanceTimersByTimeAsync(2000);
    expect(seen, 'a 401 does not stop the poller').toHaveLength(2);
    expect(poller.error()?.status).toBe(401);
    poller.stop();
  });

  it('a thrown non-ApiError (network) is reported as ApiError {code: network, status: 0}', async () => {
    const { client } = fakeClient<{ n: number }>([new TypeError('Failed to fetch')]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    const err = poller.error();
    expect(err).toBeInstanceOf(ApiError);
    expect(err?.code).toBe('network');
    expect(err?.status).toBe(0);
    poller.stop();
  });

  it('is reactive: a computation that read it re-runs on a failure and on the next good read', async () => {
    const { client } = fakeClient<{ n: number }>([ok(1, 1), refusal(500, 'serverError'), ok(2, 2)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    let dispose = () => {};
    const status = createRoot((d) => {
      dispose = d;
      return createMemo(() => poller.error()?.status ?? null);
    });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(status()).toBeNull();
    await vi.advanceTimersByTimeAsync(1000);
    expect(status()).toBe(500);
    await vi.advanceTimersByTimeAsync(2000);
    expect(status()).toBeNull();
    poller.stop();
    dispose();
  });
});

describe('SF-a: ViewPoller.lastOkAt()', () => {
  it('is null before the first good read', async () => {
    const { client } = fakeClient<{ n: number }>([refusal(500, 'serverError')]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    expect(poller.lastOkAt()).toBeNull();
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.lastOkAt(), 'a failed read is not a good read').toBeNull();
    poller.stop();
  });

  it('moves on every good read (changed true or false) and only then', async () => {
    const { client, seen } = fakeClient<{ n: number }>([ok(1, 1), same(1), refusal(500, 'serverError'), ok(2, 2)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.lastOkAt()).toBe(T0);
    await vi.advanceTimersByTimeAsync(1000);
    expect(poller.lastOkAt(), 'changed: false is a good read too').toBe(T0 + 1000);
    await vi.advanceTimersByTimeAsync(1000);
    expect(seen).toHaveLength(3);
    expect(poller.lastOkAt(), 'the failed read leaves it').toBe(T0 + 1000);
    await vi.advanceTimersByTimeAsync(2000);
    expect(seen).toHaveLength(4);
    expect(poller.lastOkAt()).toBe(T0 + 4000);
    poller.stop();
  });

  it('is reactive', async () => {
    const { client } = fakeClient<{ n: number }>([ok(1, 1)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    let dispose = () => {};
    const at = createRoot((d) => {
      dispose = d;
      return createMemo(() => poller.lastOkAt());
    });
    expect(at()).toBeNull();
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(at()).toBe(T0);
    poller.stop();
    dispose();
  });
});

describe('SF-a: backoff does not change (guards, green today)', () => {
  it('one failure: the next read is 2 intervals later, then back to one interval', async () => {
    const { client, seen } = fakeClient<{ n: number }>([ok(1, 1), refusal(500, 'serverError'), ok(2, 2), same(2)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(5000);
    expect(seen.map((s) => s.at - T0)).toEqual([0, 1000, 3000, 4000, 5000]);
    poller.stop();
  });

  it('a 401 backs off like any other failure', async () => {
    const { client, seen } = fakeClient<{ n: number }>([refusal(401, 'signIn')]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 4000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(seen.map((s) => s.at - T0)).toEqual([0, 2000, 6000, 10_000]);
    poller.stop();
  });

  it('after a failure the next read still carries since=<the last good version>', async () => {
    const { client, seen } = fakeClient<{ n: number }>([ok(1, 7), refusal(500, 'serverError'), same(7)]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(3000);
    expect(seen).toHaveLength(3);
    expect(seen[1]!.params.since).toBe(7);
    expect(seen[2]!.params.since).toBe(7);
    poller.stop();
  });
});
