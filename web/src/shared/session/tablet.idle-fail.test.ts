// #4562 (kiosk-shared-phone §5r.3 row U7a, §5r.4b): a failed idle sign-out on the branch tablet must never be dropped.
// armTabletIdle(branch, api, onFailed?) — when the idle POST /api/auth/logout fails: call onFailed() once, do not
// restart, and start the idle wait again, so the sign-out is tried again after another TABLET_IDLE_MS with no input.
// Written before the implementation by the XC FE controller claude/e8020026.
// Do not edit, delete or weaken these tests; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../api/client';
import { TABLET_IDLE_MS } from './idle';
import { armTabletIdle } from './tablet';

// The web tsconfig loads only vite/client types; the unhandled-rejection check below needs Node's process (vitest).
declare const process: {
  on(event: string, listener: (...args: unknown[]) => void): void;
  off(event: string, listener: (...args: unknown[]) => void): void;
};

let assign: ReturnType<typeof vi.fn>;
let logouts: number;
let logoutStatus: number;

function api(): ApiClient {
  return new ApiClient((async (input: string) => {
    if (new URL(String(input), 'http://localhost').pathname === '/api/auth/logout') {
      logouts += 1;
      return logoutStatus === 200
        ? new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })
        : new Response(JSON.stringify({ error: { code: 'serverError', message: 'down' } }), {
            status: logoutStatus,
            headers: { 'content-type': 'application/json' },
          });
    }
    return new Response('{}', { status: 404, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof fetch);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

async function idle(): Promise<void> {
  await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS);
  await flush();
}

beforeEach(() => {
  vi.useFakeTimers();
  assign = vi.fn();
  logouts = 0;
  logoutStatus = 503;
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked: the URL flag still drives the mark
  }
  vi.stubGlobal('location', { ...window.location, search: '?branch=MAIN&shared=1', pathname: '/app/queue', assign });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('#4562 U7a: a failed idle sign-out', () => {
  it('calls onFailed once and does not restart', async () => {
    const onFailed = vi.fn();
    const dispose = armTabletIdle('MAIN', api(), onFailed);
    await idle();
    expect(logouts).toBe(1);
    expect(onFailed).toHaveBeenCalledTimes(1);
    expect(assign).not.toHaveBeenCalled();
    dispose();
  });

  it('tries again after another TABLET_IDLE_MS, and restarts once the sign-out works', async () => {
    const onFailed = vi.fn();
    const dispose = armTabletIdle('MAIN', api(), onFailed);
    await idle();
    expect(logouts).toBe(1);
    logoutStatus = 200;
    await idle();
    expect(logouts).toBe(2);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN&shared=1');
    dispose();
  });

  it('without onFailed it still retries, and the rejection does not escape', async () => {
    const unhandled = vi.fn();
    process.on('unhandledRejection', unhandled);
    const dispose = armTabletIdle('MAIN', api());
    await idle();
    await idle();
    expect(logouts).toBe(2);
    expect(assign).not.toHaveBeenCalled();
    dispose();
    process.off('unhandledRejection', unhandled);
    expect(unhandled).not.toHaveBeenCalled();
  });

  it('the disposer stops the retry', async () => {
    const dispose = armTabletIdle('MAIN', api(), vi.fn());
    await idle();
    dispose();
    await idle();
    await idle();
    expect(logouts).toBe(1);
  });
});
