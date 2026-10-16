// #2437: keyedWrite must keep the Idempotency-Key when the server answers
// 409 idempotencyConflict ("the first request with this key is still running").
// One test per behaviour-table row (part 8) and per edge case (part 9).

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './client';
import { keyedWrite } from './keyed-write';

interface DeferredCall {
  key: string | null;
  body: string;
  resolve: (status: number, payload: unknown) => void;
  resolveText: (status: number, text: string) => void;
  rejectNetwork: () => void;
}

const IDEM_CONFLICT = {
  error: { code: 'idempotencyConflict', message: 'Request with this key is still in progress' },
};

describe('keyedWrite keeps the key on 409 idempotencyConflict (#2437)', () => {
  let calls: DeferredCall[];
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    calls = [];
    fetchSpy = vi.fn((_input: string, init?: RequestInit) => {
      const headers = new Headers(init?.headers);
      return new Promise<Response>((resolve, reject) => {
        calls.push({
          key: headers.get('idempotency-key'),
          body: String(init?.body ?? ''),
          resolve: (status: number, payload: unknown) =>
            resolve(
              new Response(JSON.stringify(payload), {
                status,
                headers: { 'content-type': 'application/json' },
              }),
            ),
          resolveText: (status: number, text: string) =>
            resolve(new Response(text, { status, headers: { 'content-type': 'text/plain' } })),
          rejectNetwork: () => reject(new TypeError('Failed to fetch')),
        });
      });
    });
  });

  function makeWriter() {
    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    return keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/items');
  }

  it('row 1: send(A) -> 409 idempotencyConflict, then send(A): the second request carries the SAME key', async () => {
    const w = makeWriter();

    const p1 = w.send({ code: 'A' });
    const key1 = calls[0]!.key;
    expect(key1).toBeTruthy();
    calls[0]!.resolve(409, IDEM_CONFLICT);
    await expect(p1).rejects.toMatchObject({ status: 409, code: 'idempotencyConflict' });

    const p2 = w.send({ code: 'A' });
    expect(calls).toHaveLength(2);
    expect(calls[1]!.key).toBe(key1);
    expect(calls[1]!.body).toBe(calls[0]!.body);
    calls[1]!.resolve(200, { ok: true });
    await expect(p2).resolves.toEqual({ ok: true });
  });

  it('row 2: overlapping send(A) x2, second 409 idempotencyConflict, first succeeds: key kept while pending, NEW key after the success', async () => {
    const w = makeWriter();

    const pFirst = w.send({ code: 'A' });
    const pSecond = w.send({ code: 'A' });
    const key1 = calls[0]!.key;
    expect(calls[1]!.key).toBe(key1);

    // The server is still running the first request: it answers the second with a conflict.
    calls[1]!.resolve(409, IDEM_CONFLICT);
    await expect(pSecond).rejects.toMatchObject({ status: 409, code: 'idempotencyConflict' });

    // While the first is still pending, a retry must still carry the same key.
    const pRetry = w.send({ code: 'A' });
    expect(calls[2]!.key).toBe(key1);

    // The first request succeeds: success still clears the key.
    calls[0]!.resolve(200, { ok: true });
    await expect(pFirst).resolves.toEqual({ ok: true });
    calls[2]!.resolve(200, { ok: true });
    await expect(pRetry).resolves.toEqual({ ok: true });

    const pNext = w.send({ code: 'A' });
    expect(calls[3]!.key).toBeTruthy();
    expect(calls[3]!.key).not.toBe(key1);
    calls[3]!.resolve(200, { ok: true });
    await expect(pNext).resolves.toEqual({ ok: true });
  });

  it('row 3: send(A) -> 409 staleVersion, then send(A): new key (only idempotencyConflict keeps the key)', async () => {
    const w = makeWriter();

    const p1 = w.send({ code: 'A' });
    const key1 = calls[0]!.key;
    calls[0]!.resolve(409, IDEM_CONFLICT);
    await expect(p1).rejects.toMatchObject({ status: 409 });

    // Conflict keeps the key ...
    const p2 = w.send({ code: 'A' });
    expect(calls[1]!.key).toBe(key1);
    // ... but a real 409 refusal (staleVersion) drops it.
    calls[1]!.resolve(409, { error: { code: 'staleVersion', message: 'Version mismatch' } });
    await expect(p2).rejects.toMatchObject({ status: 409, code: 'staleVersion' });

    const p3 = w.send({ code: 'A' });
    expect(calls[2]!.key).toBeTruthy();
    expect(calls[2]!.key).not.toBe(key1);
    calls[2]!.resolve(200, { ok: true });
    await expect(p3).resolves.toEqual({ ok: true });
  });

  it('row 4: send(A) -> 500 / network error, then send(A): same key, also after a 409 idempotencyConflict', async () => {
    const w = makeWriter();

    const p1 = w.send({ code: 'A' });
    const key1 = calls[0]!.key;
    calls[0]!.resolve(409, IDEM_CONFLICT);
    await expect(p1).rejects.toMatchObject({ status: 409 });

    const p2 = w.send({ code: 'A' });
    expect(calls[1]!.key).toBe(key1);
    calls[1]!.resolve(500, { error: { code: 'serverError', message: 'Internal Server Error' } });
    await expect(p2).rejects.toMatchObject({ status: 500 });

    const p3 = w.send({ code: 'A' });
    expect(calls[2]!.key).toBe(key1);
    calls[2]!.rejectNetwork();
    await expect(p3).rejects.toThrow(TypeError);

    const p4 = w.send({ code: 'A' });
    expect(calls[3]!.key).toBe(key1);
    calls[3]!.resolve(200, { ok: true });
    await expect(p4).resolves.toEqual({ ok: true });
  });

  it('row 5: overlap rules and reset(): a conflict on A keeps only A key, does not touch B, and reset() still clears', async () => {
    const w = makeWriter();

    const pA = w.send({ code: 'A' });
    const pB = w.send({ code: 'B' });
    const keyA = calls[0]!.key;
    const keyB = calls[1]!.key;
    expect(keyB).not.toBe(keyA);

    calls[0]!.resolve(409, IDEM_CONFLICT);
    await expect(pA).rejects.toMatchObject({ status: 409 });
    calls[1]!.resolve(200, { ok: true });
    await expect(pB).resolves.toEqual({ ok: true });

    // A keeps its key after the conflict; B (succeeded) gets a new one.
    const pA2 = w.send({ code: 'A' });
    expect(calls[2]!.key).toBe(keyA);
    const pB2 = w.send({ code: 'B' });
    expect(calls[3]!.key).not.toBe(keyB);
    calls[2]!.resolve(409, IDEM_CONFLICT);
    await expect(pA2).rejects.toMatchObject({ status: 409 });
    calls[3]!.resolve(200, { ok: true });
    await expect(pB2).resolves.toEqual({ ok: true });

    // reset() clears the kept key unconditionally.
    w.reset();
    const pA3 = w.send({ code: 'A' });
    expect(calls[4]!.key).toBeTruthy();
    expect(calls[4]!.key).not.toBe(keyA);
    calls[4]!.resolve(200, { ok: true });
    await expect(pA3).resolves.toEqual({ ok: true });
  });

  it('edge: a 409 idempotencyConflict for a key already replaced by a newer send of the same body leaves the newer key', async () => {
    const w = makeWriter();

    const pOld = w.send({ code: 'A' });
    const oldKey = calls[0]!.key;
    w.reset();
    const pNew = w.send({ code: 'A' });
    const newKey = calls[1]!.key;
    expect(newKey).not.toBe(oldKey);

    // Overlapping send with the same body shares the newer key and gets a conflict.
    const pNewDup = w.send({ code: 'A' });
    expect(calls[2]!.key).toBe(newKey);
    calls[2]!.resolve(409, IDEM_CONFLICT);
    await expect(pNewDup).rejects.toMatchObject({ status: 409 });

    // The stale request (old key) also answers conflict: it must not touch the newer key.
    calls[0]!.resolve(409, IDEM_CONFLICT);
    await expect(pOld).rejects.toMatchObject({ status: 409 });

    const pRetry = w.send({ code: 'A' });
    expect(calls[3]!.key).toBe(newKey);

    calls[1]!.resolve(200, { ok: true });
    await expect(pNew).resolves.toEqual({ ok: true });
    calls[3]!.resolve(200, { ok: true });
    await expect(pRetry).resolves.toEqual({ ok: true });
  });

  it('edge: a 409 ApiError without a code (plain-text body) keeps today behaviour and drops the key', async () => {
    const w = makeWriter();

    const p1 = w.send({ code: 'A' });
    const key1 = calls[0]!.key;
    calls[0]!.resolve(409, IDEM_CONFLICT);
    await expect(p1).rejects.toMatchObject({ status: 409 });

    const p2 = w.send({ code: 'A' });
    expect(calls[1]!.key).toBe(key1);
    // No error code in the body: the client falls back to a generic code, so the key is dropped.
    calls[1]!.resolveText(409, 'Conflict');
    await expect(p2).rejects.toMatchObject({ status: 409 });

    const p3 = w.send({ code: 'A' });
    expect(calls[2]!.key).toBeTruthy();
    expect(calls[2]!.key).not.toBe(key1);
    calls[2]!.resolve(200, { ok: true });
    await expect(p3).resolves.toEqual({ ok: true });
  });
});
