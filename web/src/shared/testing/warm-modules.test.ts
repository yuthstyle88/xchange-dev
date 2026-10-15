import { describe, expect, it } from 'vitest';
import { warmModules } from './warm-modules';

describe('warmModules (#1874)', () => {
  it('stubs fetch with 401 unauthenticated during warm-up, then restores fetch and clears document.body', async () => {
    const originalFetch = globalThis.fetch;
    let statusDuringWarmup = 0;
    let bodyDuringWarmup: unknown;

    await warmModules(async () => {
      document.body.innerHTML = '<div id="warm-side-effect">mounted</div>';
      const res = await fetch('/api/v3/views/me');
      statusDuringWarmup = res.status;
      bodyDuringWarmup = await res.json();
    });

    expect(statusDuringWarmup).toBe(401);
    expect(bodyDuringWarmup).toEqual({ error: { code: 'unauthenticated' } });
    expect(globalThis.fetch).toBe(originalFetch);
    expect(document.body.innerHTML).toBe('');
  });

  it('restores fetch and clears document.body even when a loader throws', async () => {
    const originalFetch = globalThis.fetch;

    await expect(
      warmModules(async () => {
        document.body.innerHTML = '<div id="leaked">leaked</div>';
        throw new Error('loader failed');
      }),
    ).rejects.toThrow('loader failed');

    expect(globalThis.fetch).toBe(originalFetch);
    expect(document.body.innerHTML).toBe('');
  });

  it('awaits each loader sequentially in the given order', async () => {
    const order: string[] = [];

    await warmModules(
      async () => {
        order.push('first:start');
        await new Promise((r) => setTimeout(r, 5));
        order.push('first:end');
      },
      async () => {
        order.push('second:start');
        await Promise.resolve();
        order.push('second:end');
      },
      async () => {
        order.push('third');
      },
    );

    expect(order).toEqual(['first:start', 'first:end', 'second:start', 'second:end', 'third']);
  });
});
