// XC Counter CA-5 (docs/design/xc-counter-app.md §3, §5.4, §8.4, §8.6): the bridge wrapper.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//
// | # | Case | Required |
// |---|---|---|
// | B1 | no `window.xcCounter` | `getXcCounter()` is undefined |
// | B2 | `window.xcCounter` set | `getXcCounter()` is that object |
// | B3 | `bindCounter`, the server accepts | `bindingNonce` once, then one POST desk_bind_counter `{nonce}` with an idempotency key; answers true |
// | B4 | each refusal of §8.4 | answers false, does not throw |
// | B5 | the network fails | answers false, does not throw |
// | B6 | `bindingNonce` rejects | answers false, no request |
// | B7 | no `window.xcCounter` | answers false, no request |
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, type Fetch } from '../api/client';
import { bindCounter, getXcCounter, type XcCounter } from './bridge';

function counter(over: Partial<XcCounter> = {}): XcCounter {
  return {
    info: vi.fn(async () => {
      throw new Error('info is not part of this test');
    }),
    scanPassport: vi.fn(async () => ({ ok: false as const, reason: 'noScanner' as const })),
    bindingNonce: vi.fn(async () => 'v1.nonce'),
    ...over,
  };
}

function server(answer: () => Response | Promise<Response>) {
  const calls: Array<{ url: string; method?: string; body: unknown; key: string | null }> = [];
  const fetchFn: Fetch = async (input, init) => {
    calls.push({
      url: input,
      method: init?.method,
      body: JSON.parse(String(init?.body ?? 'null')),
      key: new Headers(init?.headers).get('idempotency-key'),
    });
    return answer();
  };
  return { api: new ApiClient(fetchFn), calls };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

afterEach(() => {
  delete window.xcCounter;
});

describe('getXcCounter', () => {
  it('B1: is undefined in a plain browser', () => {
    expect(getXcCounter()).toBeUndefined();
  });

  it('B2: is the app bridge inside the app', () => {
    const bridge = counter();
    window.xcCounter = bridge;
    expect(getXcCounter()).toBe(bridge);
  });
});

describe('bindCounter', () => {
  it('B3: takes one nonce, sends deskBindCounter {nonce}, and answers true', async () => {
    const bridge = counter();
    window.xcCounter = bridge;
    const { api, calls } = server(() => json(200, { counterAgentId: 'agent-1' }));

    await expect(bindCounter(api)).resolves.toBe(true);

    expect(bridge.bindingNonce).toHaveBeenCalledTimes(1);
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe('/api/v3/commands/desk_bind_counter');
    expect(calls[0]!.method).toBe('POST');
    expect(calls[0]!.body).toEqual({ nonce: 'v1.nonce' });
    expect(calls[0]!.key, 'sent through keyedWrite').toBeTruthy();
  });

  it.each(['NONCE_INVALID', 'NONCE_EXPIRED', 'COUNTER_REVOKED', 'COUNTER_OTHER_BRANCH'])(
    'B4: the refusal %s answers false and does not throw',
    async (code) => {
      window.xcCounter = counter();
      const { api, calls } = server(() => json(409, { error: { code, message: code } }));
      await expect(bindCounter(api)).resolves.toBe(false);
      expect(calls, 'the request was sent and refused').toHaveLength(1);
    },
  );

  it('B5: a network failure answers false and does not throw', async () => {
    window.xcCounter = counter();
    const { api, calls } = server(() => Promise.reject(new TypeError('Failed to fetch')));
    await expect(bindCounter(api)).resolves.toBe(false);
    expect(calls, 'the request was attempted').toHaveLength(1);
  });

  it('B6: a bindingNonce that rejects answers false with no request', async () => {
    const bridge = counter({ bindingNonce: vi.fn(async () => Promise.reject(new Error('revoked'))) });
    window.xcCounter = bridge;
    const { api, calls } = server(() => json(200, {}));
    await expect(bindCounter(api)).resolves.toBe(false);
    expect(bridge.bindingNonce).toHaveBeenCalledTimes(1);
    expect(calls).toHaveLength(0);
  });

  it('B7: with no xcCounter answers false with no request', async () => {
    const { api, calls } = server(() => json(200, {}));
    await expect(bindCounter(api)).resolves.toBe(false);
    expect(calls).toHaveLength(0);
  });
});
