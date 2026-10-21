// QW-3 (kiosk-shared-phone §5r F9, rows U6–U8): the borrowed branch tablet signs the customer out on done and on
// idle; a personal phone never does. Written before the implementation by the sub-controller claude/36d9dc6a.
// Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// What these tests pin:
// - `isSharedTablet()` is true when the page URL has `?shared=1`, and stays true for the tab via
//   sessionStorage['xc.sharedTablet'] after the parameter is gone.
// - `signOutAndRestart(branch, api)` posts `POST /api/auth/logout` once (§4.3a W12), then calls
//   `location.assign('/app/queue?branch=<branch>&shared=1')`.
// - `armTabletIdle(branch, api)` on a shared tablet signs out after `TABLET_IDLE_MS` (120 000 ms) with no input;
//   a `pointerdown` or `keydown` restarts the wait; the returned disposer stops it. On a personal phone it does nothing.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../api/client';
import { TABLET_IDLE_MS } from './idle';
import { armTabletIdle, isSharedTablet, signOutAndRestart } from './tablet';

let assign: ReturnType<typeof vi.fn>;
let calls: { url: string; method: string }[];

function fakeApi(): ApiClient {
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), method: init?.method ?? 'GET' });
    return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } });
  });
  return new ApiClient(fetchFn as unknown as typeof fetch);
}

function openAt(search: string): void {
  vi.stubGlobal('location', { ...window.location, search, pathname: '/app/queue', assign });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
}

beforeEach(() => {
  calls = [];
  assign = vi.fn();
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked in this environment: the tests below still drive the URL flag
  }
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('QW-3: isSharedTablet', () => {
  it('is false on a personal phone (no ?shared=1)', () => {
    openAt('?branch=MAIN');
    expect(isSharedTablet()).toBe(false);
  });

  it('is true with ?shared=1, and stays true for the tab after the parameter is gone', () => {
    openAt('?branch=MAIN&shared=1');
    expect(isSharedTablet()).toBe(true);
    openAt('?branch=MAIN');
    expect(isSharedTablet()).toBe(true);
  });
});

describe('QW-3: signOutAndRestart (U6)', () => {
  it('posts /api/auth/logout once, then restarts the tablet page for the branch', async () => {
    openAt('?branch=MAIN&shared=1');
    await signOutAndRestart('MAIN', fakeApi());
    const logouts = calls.filter((c) => c.url.endsWith('/api/auth/logout'));
    expect(logouts).toHaveLength(1);
    expect(logouts[0]!.method).toBe('POST');
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN&shared=1');
  });
});

describe('QW-3: armTabletIdle (U7, U8)', () => {
  it('U7: on a shared tablet, no input for TABLET_IDLE_MS signs out', async () => {
    expect(TABLET_IDLE_MS).toBe(120_000);
    vi.useFakeTimers();
    openAt('?branch=MAIN&shared=1');
    const dispose = armTabletIdle('MAIN', fakeApi());
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS - 1);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(calls.filter((c) => c.url.endsWith('/api/auth/logout'))).toHaveLength(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN&shared=1');
    dispose();
  });

  it('U7: a pointerdown or keydown restarts the wait', async () => {
    vi.useFakeTimers();
    openAt('?branch=MAIN&shared=1');
    const dispose = armTabletIdle('MAIN', fakeApi());
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS - 1_000);
    document.dispatchEvent(new Event('pointerdown'));
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS - 1_000);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS - 1_000);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    dispose();
  });

  it('the disposer stops the timer', async () => {
    vi.useFakeTimers();
    openAt('?branch=MAIN&shared=1');
    const dispose = armTabletIdle('MAIN', fakeApi());
    dispose();
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS * 2);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    expect(calls).toHaveLength(0);
  });

  it('U8: on a personal phone it never signs out', async () => {
    vi.useFakeTimers();
    openAt('?branch=MAIN');
    const dispose = armTabletIdle('MAIN', fakeApi());
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS * 3);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    expect(calls).toHaveLength(0);
    dispose();
  });
});
