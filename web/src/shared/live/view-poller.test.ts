// V0-A: ViewPoller (docs/design/xchange-v3-web.md §5.2 and §9).
// Written before the implementation. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ViewAnswer, ViewParams } from '../api/client';
import { ViewPoller } from './view-poller';

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
      return next as unknown as ViewAnswer<U>;
    },
  };
  return { client, seen };
}

let hidden = false;

function setHidden(value: boolean) {
  hidden = value;
  document.dispatchEvent(new Event('visibilitychange'));
}

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
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

describe('ViewPoller', () => {
  it('sends the version of the last answer as since on the next request', async () => {
    const { client, seen } = fakeClient<{ n: number }>([
      { version: 7, changed: true, data: { n: 1 } },
      { version: 7, changed: false },
    ]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', {
      params: { branch: 'XC-B01' },
      intervalMs: 1000,
      maxIntervalMs: 8000,
    });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    expect(seen[0]!.name).toBe('rates');
    expect(seen[0]!.params.since).toBeUndefined();
    expect(seen[0]!.params.branch).toBe('XC-B01');

    await vi.advanceTimersByTimeAsync(1000);
    expect(seen).toHaveLength(2);
    expect(seen[1]!.params.since).toBe(7);
    expect(seen[1]!.params.branch).toBe('XC-B01');
    expect(poller.version()).toBe(7);
    poller.stop();
  });

  it('keeps the same data object when the answer is changed: false', async () => {
    const first = { n: 1 };
    const { client } = fakeClient<{ n: number }>([
      { version: 3, changed: true, data: first },
      { version: 3, changed: false },
      { version: 3, changed: false },
    ]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.data()).toBe(first);
    await vi.advanceTimersByTimeAsync(2000);
    expect(poller.data()).toBe(first);
    poller.stop();
  });

  it('replaces the data when the answer is changed: true', async () => {
    const second = { n: 2 };
    const { client } = fakeClient<{ n: number }>([
      { version: 3, changed: true, data: { n: 1 } },
      { version: 4, changed: true, data: second },
    ]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(1000);
    expect(poller.data()).toBe(second);
    expect(poller.version()).toBe(4);
    poller.stop();
  });

  it('makes no request while the document is hidden, and resumes when it is shown', async () => {
    const { client, seen } = fakeClient<{ n: number }>([{ version: 1, changed: true, data: { n: 1 } }]);
    setHidden(true);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(seen).toHaveLength(0);

    setHidden(false);
    await vi.advanceTimersByTimeAsync(1000);
    expect(seen.length).toBeGreaterThanOrEqual(1);

    const before = seen.length;
    setHidden(true);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(seen.length).toBeLessThanOrEqual(before + 1);
    poller.stop();
  });

  it('doubles the interval after each error, up to the cap', async () => {
    const { client, seen } = fakeClient<{ n: number }>([new Error('down')]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 4000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(14_000);
    // after the 1st error 2 s, the 2nd 4 s, then capped at 4 s
    expect(seen.map((s) => s.at)).toEqual([0, 2000, 6000, 10_000, 14_000]);
    poller.stop();
  });

  it('goes back to its own interval after an answer', async () => {
    const { client, seen } = fakeClient<{ n: number }>([
      new Error('down'),
      new Error('down'),
      { version: 1, changed: true, data: { n: 1 } },
      { version: 1, changed: false },
    ]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(7000);
    expect(seen.map((s) => s.at)).toEqual([0, 2000, 6000, 7000]);
    poller.stop();
  });

  it('makes no request after stop', async () => {
    const { client, seen } = fakeClient<{ n: number }>([{ version: 1, changed: true, data: { n: 1 } }]);
    const poller = new ViewPoller<{ n: number }>(client, 'rates', { intervalMs: 1000, maxIntervalMs: 8000 });
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    poller.stop();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(seen).toHaveLength(1);
  });
});
