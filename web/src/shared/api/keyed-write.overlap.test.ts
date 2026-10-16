import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './client';
import { keyedWrite } from './keyed-write';

interface DeferredCall {
  url: string;
  method: string;
  key: string | null;
  body: string;
  resolve: (status: number, payload: unknown) => void;
  rejectNetwork: () => void;
}

describe('keyedWrite overlapping sends (#2327)', () => {
  let calls: DeferredCall[];
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    calls = [];
    fetchSpy = vi.fn((input: string, init?: RequestInit) => {
      const headers = new Headers(init?.headers);
      return new Promise<Response>((resolve, reject) => {
        calls.push({
          url: String(input),
          method: String(init?.method ?? 'POST'),
          key: headers.get('idempotency-key'),
          body: String(init?.body ?? ''),
          resolve: (status: number, payload: unknown) =>
            resolve(
              new Response(JSON.stringify(payload), {
                status,
                headers: { 'content-type': 'application/json' },
              }),
            ),
          rejectNetwork: () => reject(new TypeError('Failed to fetch')),
        });
      });
    });
  });

  it('1. send(A) and send(B) overlap; A succeeds; B fails 503; retry B reuses B first key', async () => {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');

    const pA = w.send({ code: 'A' });
    const pB = w.send({ code: 'B' });
    expect(calls).toHaveLength(2);
    const keyA = calls[0]!.key;
    const keyB = calls[1]!.key;
    expect(keyA).toBeTruthy();
    expect(keyB).toBeTruthy();
    expect(keyB).not.toBe(keyA);

    // A succeeds while B is still in flight
    calls[0]!.resolve(200, { ok: true });
    await expect(pA).resolves.toEqual({ ok: true });

    // B fails 503
    calls[1]!.resolve(503, { error: { code: 'unavailable', message: 'Service Unavailable' } });
    await expect(pB).rejects.toMatchObject({ status: 503 });

    // Retry B: must reuse B's first key
    const pBRetry = w.send({ code: 'B' });
    expect(calls).toHaveLength(3);
    expect(calls[2]!.key).toBe(keyB);
    calls[2]!.resolve(200, { ok: true });
    await expect(pBRetry).resolves.toEqual({ ok: true });
  });

  it('2. send(A) and send(B) overlap; B succeeds first; A fails 503; retry A reuses A first key', async () => {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');

    const pA = w.send({ code: 'A' });
    const pB = w.send({ code: 'B' });
    expect(calls).toHaveLength(2);
    const keyA = calls[0]!.key;
    const keyB = calls[1]!.key;
    expect(keyA).toBeTruthy();
    expect(keyB).toBeTruthy();
    expect(keyB).not.toBe(keyA);

    // B succeeds first
    calls[1]!.resolve(200, { ok: true });
    await expect(pB).resolves.toEqual({ ok: true });

    // A fails 503
    calls[0]!.resolve(503, { error: { code: 'unavailable', message: 'Service Unavailable' } });
    await expect(pA).rejects.toMatchObject({ status: 503 });

    // Retry A: must reuse A's first key
    const pARetry = w.send({ code: 'A' });
    expect(calls).toHaveLength(3);
    expect(calls[2]!.key).toBe(keyA);
    calls[2]!.resolve(200, { ok: true });
    await expect(pARetry).resolves.toEqual({ ok: true });
  });

  it('3. One send succeeds, then the same body is sent again: uses a new key without erasing an overlapping send key', async () => {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');

    const pA1 = w.send({ code: 'A' });
    const pB1 = w.send({ code: 'B' });
    const keyA1 = calls[0]!.key;
    const keyB1 = calls[1]!.key;

    calls[0]!.resolve(200, { ok: true });
    await expect(pA1).resolves.toEqual({ ok: true });

    calls[1]!.resolve(503, { error: { code: 'unavailable', message: 'Service Unavailable' } });
    await expect(pB1).rejects.toMatchObject({ status: 503 });

    // Sending A again after A succeeded gets a new key
    const pA2 = w.send({ code: 'A' });
    expect(calls[2]!.key).toBeTruthy();
    expect(calls[2]!.key).not.toBe(keyA1);
    calls[2]!.resolve(200, { ok: true });
    await expect(pA2).resolves.toEqual({ ok: true });

    // Retrying B still reuses B's original key
    const pB2 = w.send({ code: 'B' });
    expect(calls[3]!.key).toBe(keyB1);
    calls[3]!.resolve(200, { ok: true });
    await expect(pB2).resolves.toEqual({ ok: true });
  });

  it('4. One send fails 5xx, then the same body is retried: keeps the same key even when an overlapping send failed 4xx', async () => {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');

    const pA = w.send({ code: 'A' });
    const pB = w.send({ code: 'B' });
    const keyB = calls[1]!.key;

    // A fails 409 (4xx), which clears only A's key
    calls[0]!.resolve(409, { error: { code: 'conflict', message: 'Conflict' } });
    await expect(pA).rejects.toMatchObject({ status: 409 });

    // B fails 503 (5xx), which must keep B's key
    calls[1]!.resolve(503, { error: { code: 'unavailable', message: 'Service Unavailable' } });
    await expect(pB).rejects.toMatchObject({ status: 503 });

    const pBRetry = w.send({ code: 'B' });
    expect(calls[2]!.key).toBe(keyB);
    calls[2]!.resolve(200, { ok: true });
    await expect(pBRetry).resolves.toEqual({ ok: true });
  });

  it('5. One send fails 4xx, then the same body is sent again: mints a new key for the 4xx body while preserving an overlapping 5xx body key', async () => {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');

    const pA = w.send({ code: 'A' });
    const pB = w.send({ code: 'B' });
    const keyA = calls[0]!.key;
    const keyB = calls[1]!.key;

    // B fails 422 (4xx)
    calls[1]!.resolve(422, { error: { code: 'invalid', message: 'Invalid' } });
    await expect(pB).rejects.toMatchObject({ status: 422 });

    // A fails 502 (5xx)
    calls[0]!.resolve(502, { error: { code: 'badGateway', message: 'Bad Gateway' } });
    await expect(pA).rejects.toMatchObject({ status: 502 });

    // Sending B again after 422 mints a new key
    const pB2 = w.send({ code: 'B' });
    expect(calls[2]!.key).not.toBe(keyB);
    calls[2]!.resolve(200, { ok: true });
    await expect(pB2).resolves.toEqual({ ok: true });

    // Retrying A after 502 reuses A's first key
    const pA2 = w.send({ code: 'A' });
    expect(calls[3]!.key).toBe(keyA);
    calls[3]!.resolve(200, { ok: true });
    await expect(pA2).resolves.toEqual({ ok: true });
  });

  it('6. Edge cases: overlapping sends with the SAME body share the key, and reset() clears pending unconditionally', async () => {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');

    // Two overlapping sends with the SAME body A share the key, while B overlaps with a different body
    const pA1 = w.send({ code: 'A' });
    const pA2 = w.send({ code: 'A' });
    const pB1 = w.send({ code: 'B' });
    expect(calls[0]!.key).toBe(calls[1]!.key);
    const keyB1 = calls[2]!.key;
    expect(keyB1).not.toBe(calls[0]!.key);

    calls[0]!.resolve(200, { ok: true });
    calls[1]!.resolve(200, { ok: true });
    await expect(pA1).resolves.toEqual({ ok: true });
    await expect(pA2).resolves.toEqual({ ok: true });

    // B fails 503; retry B reuses keyB1, and if that retry also fails 503 and reset() is called, the next send gets a fresh key
    calls[2]!.resolve(503, { error: { code: 'unavailable', message: 'Service Unavailable' } });
    await expect(pB1).rejects.toMatchObject({ status: 503 });

    const pB2 = w.send({ code: 'B' });
    expect(calls[3]!.key).toBe(keyB1);
    calls[3]!.resolve(503, { error: { code: 'unavailable', message: 'Service Unavailable' } });
    await expect(pB2).rejects.toMatchObject({ status: 503 });

    w.reset();
    const pB3 = w.send({ code: 'B' });
    expect(calls[4]!.key).not.toBe(keyB1);
    calls[4]!.resolve(200, { ok: true });
    await expect(pB3).resolves.toEqual({ ok: true });
  });
});
