// Up-front tests for QW-2g (kiosk-shared-phone §5r.4a). Do not edit, delete or weaken; add tests in a file of your own.
//
// useQueueTablet, the /app/queue page's one call into QW-3's TabletSession (shared/session/tablet.ts; §5r F9).
// Rows: U6, U7, U8 and the failed sign-out follow-up. Only useQueueTablet is under test; the network is an injected
// fetch and `location` is stubbed.
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { TABLET_IDLE_MS } from '../../../shared/session/idle';
import { th } from './i18n';
import { useQueueTablet, type QueueTablet } from './queue-tablet';

let assign: ReturnType<typeof vi.fn>;
let logouts: number;
let logoutStatus: number;
let disposeRoot: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function api(): ApiClient {
  return new ApiClient(async (input: string) => {
    if (new URL(input, 'http://localhost').pathname === '/api/auth/logout') {
      logouts += 1;
      return logoutStatus === 200 ? json(200, {}) : json(logoutStatus, { error: { code: 'serverError', message: 'down' } });
    }
    return json(404, { error: { code: 'notFound', message: input } });
  });
}

function openAt(search: string): void {
  vi.stubGlobal('location', { ...window.location, search, pathname: '/app/queue', assign });
}

function mount(): QueueTablet {
  return createRoot((dispose) => {
    disposeRoot = dispose;
    return useQueueTablet('MAIN', api());
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  assign = vi.fn();
  logouts = 0;
  logoutStatus = 200;
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked here: the tests still drive the ?shared=1 flag
  }
});

afterEach(() => {
  disposeRoot?.();
  disposeRoot = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('QW-2g: useQueueTablet on a shared tablet', () => {
  it('U6: done() signs out once, then restarts /app/queue for the branch on the shared tablet', async () => {
    openAt('?branch=MAIN&shared=1');
    const tablet = mount();
    expect(tablet.shared).toBe(true);
    await tablet.done();
    expect(logouts).toBe(1);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN&shared=1');
    expect(tablet.error()).toBeNull();
  });

  it('U7: no input for TABLET_IDLE_MS while mounted signs out and restarts', async () => {
    vi.useFakeTimers();
    openAt('?branch=MAIN&shared=1');
    mount();
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS - 1);
    expect(logouts).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(logouts).toBe(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN&shared=1');
  });

  it('U7: unmounting disposes the idle timer', async () => {
    vi.useFakeTimers();
    openAt('?branch=MAIN&shared=1');
    const tablet = mount();
    expect(tablet.shared).toBe(true);
    disposeRoot?.();
    disposeRoot = undefined;
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS * 2);
    await flush();
    expect(logouts).toBe(0);
    expect(assign).not.toHaveBeenCalled();
  });

  it('a failed sign-out: the worded signOutFailed error, and no restart signed in', async () => {
    openAt('?branch=MAIN&shared=1');
    logoutStatus = 503;
    const tablet = mount();
    await tablet.done();
    expect(logouts).toBe(1);
    expect(assign).not.toHaveBeenCalled();
    expect(tablet.error()).toBe(th.signOutFailed);
  });
});

describe('QW-2g: useQueueTablet on a personal phone', () => {
  it('U8: done() signs nothing out and leaves the page alone', async () => {
    openAt('?branch=MAIN');
    const tablet = mount();
    expect(tablet.shared).toBe(false);
    await tablet.done();
    expect(logouts).toBe(0);
    expect(assign).not.toHaveBeenCalled();
    expect(tablet.error()).toBeNull();
  });

  it('U8: no idle sign-out on a phone', async () => {
    vi.useFakeTimers();
    openAt('?branch=MAIN');
    const tablet = mount();
    expect(tablet.shared).toBe(false);
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS * 2);
    await flush();
    expect(logouts).toBe(0);
    expect(assign).not.toHaveBeenCalled();
  });
});
