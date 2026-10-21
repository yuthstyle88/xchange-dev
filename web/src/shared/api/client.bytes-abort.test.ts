// #4096: cancelling a passport read aborts the in-flight image upload (`postBytes` forwards `signal` to `fetch`).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readPassportOnServer, PassportReadError } from '../../screens/desk/ocr/passport-read';
import { ApiClient } from './client';

const FILE = new Blob([new Uint8Array([0xff, 0xd8, 0xff])], { type: 'image/jpeg' });

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('ApiClient.postBytes abort signal (#4096)', () => {
  it('1. aborting the signal aborts the stubbed fetch (init.signal is aborted), including via readPassportOnServer', async () => {
    const calls: RequestInit[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn((_url: string, init?: RequestInit) => {
        calls.push(init ?? {});
        return new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('The operation was aborted.', 'AbortError'));
          });
        });
      }),
    );

    const client = new ApiClient();
    const ac1 = new AbortController();
    const p1 = client.postBytes('/api/upload', FILE, 'image/jpeg', 'key-1', ac1.signal);
    expect(calls).toHaveLength(1);
    expect(calls[0]!.signal).toBe(ac1.signal);
    expect(calls[0]!.signal?.aborted).toBe(false);

    ac1.abort();
    expect(calls[0]!.signal?.aborted).toBe(true);
    await expect(p1).rejects.toThrow();

    const ac2 = new AbortController();
    const p2 = readPassportOnServer(client, FILE, { branch: 'XC-B01', signal: ac2.signal });
    expect(calls).toHaveLength(2);
    expect(calls[1]!.signal).toBe(ac2.signal);
    expect(calls[1]!.signal?.aborted).toBe(false);

    ac2.abort();
    expect(calls[1]!.signal?.aborted).toBe(true);
    await expect(p2).rejects.toEqual(new PassportReadError('aborted'));
  });

  it('2. with no signal, postBytes behaves as before and leaves init.signal undefined', async () => {
    const calls: RequestInit[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init?: RequestInit) => {
        calls.push(init ?? {});
        return new Response(JSON.stringify({ uploadKey: 'k-2' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }),
    );

    const client = new ApiClient();
    await expect(client.postBytes('/api/upload', FILE, 'image/jpeg', 'key-2')).resolves.toEqual({
      uploadKey: 'k-2',
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]!.signal).toBeUndefined();
  });
});
