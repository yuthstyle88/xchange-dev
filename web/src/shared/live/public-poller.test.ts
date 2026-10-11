// RD-2: PublicPoller (docs/design/blueprint/rate-display.md §4.2, D4: fleet rule 🔁).
// Written before the implementation by the test-writer claude/07d3765f, on the FE lane controller
// claude/5edf7bb5's order. Do not edit, delete or weaken.
//
// One test per §4.2 row. `fetch` is injected and answers from a script; time is vitest's fake clock;
// the tab's visibility is `document.hidden` / `document.visibilityState`, as in `view-poller.test.ts`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PublicPoller } from './public-poller';

const ROUTE = '/api/public/branches/XC-B01/rates';

interface Seen {
  at: number;
  url: string;
  ifNoneMatch: string | null;
}

type Step = { status: number; body?: unknown; etag?: string } | Error;

/** A fake `fetch` answering from `script`, one entry per call (the last one repeats). */
function fakeFetch(script: Step[]) {
  const seen: Seen[] = [];
  let i = 0;
  const fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    seen.push({
      at: Date.now(),
      url: input,
      ifNoneMatch: new Headers(init?.headers).get('If-None-Match'),
    });
    const step = script[Math.min(i, script.length - 1)]!;
    i += 1;
    if (step instanceof Error) throw step;
    const headers = new Headers();
    if (step.etag !== undefined) headers.set('ETag', step.etag);
    const body =
      step.status === 304 || step.body === undefined ? null : JSON.stringify(step.body);
    if (body !== null) headers.set('Content-Type', 'application/json');
    return new Response(body, { status: step.status, headers });
  };
  return { fetch, seen };
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

describe('PublicPoller (§4.2)', () => {
  it('first tick: GET without If-None-Match; state loading, then ready', async () => {
    const { fetch, seen } = fakeFetch([{ status: 200, body: { n: 1 }, etag: '"p1-s0"' }]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 8000);
    expect(poller.state()).toBe('loading');
    poller.start();
    expect(poller.state()).toBe('loading');
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    expect(seen[0]!.url).toBe(ROUTE);
    expect(seen[0]!.ifNoneMatch).toBeNull();
    expect(poller.state()).toBe('ready');
    poller.stop();
  });

  it('200 with ETag: data() is the body; the next tick sends If-None-Match with that ETag', async () => {
    const { fetch, seen } = fakeFetch([
      { status: 200, body: { n: 1 }, etag: '"p1-s0"' },
      { status: 304 },
    ]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 8000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.data()).toEqual({ n: 1 });

    await vi.advanceTimersByTimeAsync(1000);
    expect(seen).toHaveLength(2);
    expect(seen[1]!.at).toBe(1000);
    expect(seen[1]!.ifNoneMatch).toBe('"p1-s0"');
    poller.stop();
  });

  it('304: data() is the same object, lastOkAt moves, and the interval is back to intervalMs', async () => {
    const { fetch, seen } = fakeFetch([
      { status: 200, body: { n: 1 }, etag: '"p1-s0"' }, // t=0
      { status: 500 }, // t=1000 → interval doubles to 2000
      { status: 304 }, // t=3000 → interval back to 1000
      { status: 304 }, // t=4000
    ]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 8000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    const first = poller.data();
    expect(first).toEqual({ n: 1 });
    expect(poller.lastOkAt()).toBe(0);

    await vi.advanceTimersByTimeAsync(1000); // the 500
    await vi.advanceTimersByTimeAsync(2000); // the 304 at t=3000
    expect(seen.map((s) => s.at)).toEqual([0, 1000, 3000]);
    expect(poller.data()).toBe(first);
    expect(poller.state()).toBe('ready');
    expect(poller.lastOkAt()).toBe(3000);

    await vi.advanceTimersByTimeAsync(1000);
    expect(seen.map((s) => s.at)).toEqual([0, 1000, 3000, 4000]);
    expect(seen[3]!.ifNoneMatch).toBe('"p1-s0"');
    expect(poller.data()).toBe(first);
    poller.stop();
  });

  it('404: state notFound and no further request', async () => {
    const { fetch, seen } = fakeFetch([{ status: 404 }]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 8000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(poller.state()).toBe('notFound');

    await vi.advanceTimersByTimeAsync(60_000);
    expect(seen).toHaveLength(1);
    poller.stop();
  });

  it('5xx after data: state error, data() kept, and the interval doubles up to maxIntervalMs', async () => {
    const { fetch, seen } = fakeFetch([
      { status: 200, body: { n: 1 }, etag: '"p1-s0"' },
      { status: 503 },
    ]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 4000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    const first = poller.data();

    await vi.advanceTimersByTimeAsync(20_000);
    // 0 (200), then errors at 1000, +2000, +4000 (capped), +4000, +4000 …
    expect(seen.map((s) => s.at)).toEqual([0, 1000, 3000, 7000, 11000, 15000, 19000]);
    expect(poller.state()).toBe('error');
    expect(poller.data()).toBe(first);
    expect(poller.lastOkAt()).toBe(0);
    poller.stop();
  });

  it('network error after data: state error, data() kept, and the interval doubles up to maxIntervalMs', async () => {
    const { fetch, seen } = fakeFetch([
      { status: 200, body: { n: 1 }, etag: '"p1-s0"' },
      new TypeError('Failed to fetch'),
    ]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 4000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    const first = poller.data();

    await vi.advanceTimersByTimeAsync(12_000);
    expect(seen.map((s) => s.at)).toEqual([0, 1000, 3000, 7000, 11000]);
    expect(poller.state()).toBe('error');
    expect(poller.data()).toBe(first);
    poller.stop();
  });

  it('tab hidden: no request; visible again: one tick at once', async () => {
    const { fetch, seen } = fakeFetch([
      { status: 200, body: { n: 1 }, etag: '"p1-s0"' },
      { status: 304 },
    ]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 8000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);

    setHidden(true);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(seen).toHaveLength(1);

    setHidden(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(2);
    expect(seen[1]!.at).toBe(30_000);
    expect(seen[1]!.ifNoneMatch).toBe('"p1-s0"');
    poller.stop();
  });

  it('stop(): no further request', async () => {
    const { fetch, seen } = fakeFetch([{ status: 200, body: { n: 1 }, etag: '"p1-s0"' }]);
    const poller = new PublicPoller<{ n: number }>(ROUTE, fetch, 1000, 8000);
    poller.start();
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);

    poller.stop();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(seen).toHaveLength(1);
  });
});
