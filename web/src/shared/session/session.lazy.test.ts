import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { warmModules } from '../testing/warm-modules';

beforeAll(() => warmModules(() => import('./session')), 60_000);

describe('sharedSession lazy creation (#451, W5)', () => {
  beforeEach(() => {
    vi.useFakeTimers({ now: 0 });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it('imports session.ts without starting a timer or fetch, then creates store on first call', async () => {
    const fetchSpy = vi.fn(async () => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchSpy);

    const { sharedSession } = await import('./session');

    // Assert no fetch and no timer has started at import
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);

    // Call sharedSession() twice
    const s1 = sharedSession();
    expect(s1).toBeDefined();

    // After first call, store is created and view poller timer has started
    expect(vi.getTimerCount()).toBeGreaterThan(0);

    const s2 = sharedSession();
    // Both calls return the exact same instance
    expect(s2).toBe(s1);

    s1.dispose();
  });
});
