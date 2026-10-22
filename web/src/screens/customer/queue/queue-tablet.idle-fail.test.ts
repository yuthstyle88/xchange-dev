// #4562 (kiosk-shared-phone §5r.3 row U7a, §5r.4b): on the /app/queue page, a failed idle sign-out on the branch
// tablet shows the same worded signOutFailed error as a failed เสร็จ, and does not restart.
// Written before the implementation by the XC FE controller claude/e8020026.
// Do not edit, delete or weaken these tests; add tests in a file of your own.
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { TABLET_IDLE_MS } from '../../../shared/session/idle';
import { th } from './i18n';
import { useQueueTablet, type QueueTablet } from './queue-tablet';

let assign: ReturnType<typeof vi.fn>;
let disposeRoot: (() => void) | undefined;

function api(): ApiClient {
  return new ApiClient((async (input: string) => {
    if (new URL(String(input), 'http://localhost').pathname === '/api/auth/logout') {
      return new Response(JSON.stringify({ error: { code: 'serverError', message: 'down' } }), {
        status: 503,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response('{}', { status: 404, headers: { 'content-type': 'application/json' } });
  }) as unknown as typeof fetch);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  assign = vi.fn();
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked: the URL flag still drives the mark
  }
  vi.stubGlobal('location', { ...window.location, search: '?branch=MAIN&shared=1', pathname: '/app/queue', assign });
});

afterEach(() => {
  disposeRoot?.();
  disposeRoot = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('#4562 U7a on the page: useQueueTablet', () => {
  it('a failed idle sign-out sets the worded signOutFailed error and does not restart', async () => {
    const tablet: QueueTablet = createRoot((dispose) => {
      disposeRoot = dispose;
      return useQueueTablet('MAIN', api());
    });
    expect(tablet.error()).toBeNull();
    await vi.advanceTimersByTimeAsync(TABLET_IDLE_MS);
    await flush();
    expect(tablet.error()).toBe(th.signOutFailed);
    expect(assign).not.toHaveBeenCalled();
  });
});
