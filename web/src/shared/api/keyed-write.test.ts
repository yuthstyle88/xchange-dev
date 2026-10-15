// KW-1 (#2019): shared idempotency-key helper `keyedWrite` (design `xchange-v3-web.md` §12.24).
// One test per §9 behaviour table row.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from './client';
import { keyedWrite } from './keyed-write';

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

type QueuedAnswer = () => Response | Promise<Response>;

let answers: QueuedAnswer[];
let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  answers = [];
  fetchSpy = vi.fn(async () => {
    const next = answers.shift();
    if (!next) {
      throw new Error('no queued response');
    }
    return next();
  });
});

function jsonResponse(status: number, body: unknown): QueuedAnswer {
  return () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
}

function callAt(index: number): {
  url: string;
  method: string;
  key: string | null;
  body: string;
} {
  const [url, init] = fetchSpy.mock.calls[index] as [string, RequestInit];
  const headers = new Headers(init.headers);
  return {
    url,
    method: String(init.method),
    key: headers.get('idempotency-key'),
    body: String(init.body),
  };
}

describe('KW-1 (#2019): keyedWrite(api, method, path)', () => {
  it('1. send(A) -> network TypeError, then send(A): both requests carry the same key with a byte-equal body', async () => {
    answers.push(() => {
      throw new TypeError('Failed to fetch');
    });
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ amount: string; branch: string }, { ok: boolean }>(
      api,
      'POST',
      '/api/cash/transfers',
    );

    const bodyA = { amount: '1000.00', branch: 'MAIN' };
    await expect(w.send(bodyA)).rejects.toThrow(TypeError);
    await expect(w.send({ amount: '1000.00', branch: 'MAIN' })).resolves.toEqual({ ok: true });

    const first = callAt(0);
    const second = callAt(1);
    expect(first.key).toBeTruthy();
    expect(second.key).toBe(first.key);
    expect(second.body).toBe(first.body);
  });

  it('2. send(A) -> 500, then send(A): keeps the same key', async () => {
    answers.push(
      jsonResponse(500, { error: { code: 'serverError', message: 'Internal Server Error' } }),
    );
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ code: string }, { ok: boolean }>(api, 'POST', '/api/branches');

    await expect(w.send({ code: 'B1' })).rejects.toMatchObject({ status: 500 });
    await expect(w.send({ code: 'B1' })).resolves.toEqual({ ok: true });

    expect(callAt(0).key).toBeTruthy();
    expect(callAt(1).key).toBe(callAt(0).key);
    expect(callAt(1).body).toBe(callAt(0).body);
  });

  it('3. send(A) -> 409 refusal, then send(A): uses a new key after a 4xx refusal', async () => {
    answers.push(
      jsonResponse(409, { error: { code: 'staleVersion', message: 'Version mismatch' } }),
    );
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ version: number }, { ok: boolean }>(api, 'PUT', '/api/cash-policy');

    await expect(w.send({ version: 1 })).rejects.toMatchObject({ status: 409 });
    await expect(w.send({ version: 1 })).resolves.toEqual({ ok: true });

    expect(callAt(0).key).toBeTruthy();
    expect(callAt(1).key).toBeTruthy();
    expect(callAt(1).key).not.toBe(callAt(0).key);
  });

  it('4. send(A) -> network error, then send(B): uses a new key when the body changes', async () => {
    answers.push(() => {
      throw new TypeError('Failed to fetch');
    });
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ limit: string }, { ok: boolean }>(
      api,
      'PUT',
      '/api/cash-transfer-policy',
    );

    await expect(w.send({ limit: '5000' })).rejects.toThrow(TypeError);
    await expect(w.send({ limit: '6000' })).resolves.toEqual({ ok: true });

    expect(callAt(0).key).toBeTruthy();
    expect(callAt(1).key).toBeTruthy();
    expect(callAt(1).key).not.toBe(callAt(0).key);
  });

  it('5. send(A) -> 2xx, then send(A): uses a new key after success', async () => {
    answers.push(jsonResponse(200, { ok: true }));
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ item: string }, { ok: boolean }>(api, 'POST', '/api/items');

    await expect(w.send({ item: 'A' })).resolves.toEqual({ ok: true });
    await expect(w.send({ item: 'A' })).resolves.toEqual({ ok: true });

    expect(callAt(0).key).toBeTruthy();
    expect(callAt(1).key).toBeTruthy();
    expect(callAt(1).key).not.toBe(callAt(0).key);
  });

  it('6. send(A) -> network error, reset(), then send(A): uses a new key after reset()', async () => {
    answers.push(() => {
      throw new TypeError('Failed to fetch');
    });
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ item: string }, { ok: boolean }>(api, 'POST', '/api/items');

    await expect(w.send({ item: 'A' })).rejects.toThrow(TypeError);
    w.reset();
    await expect(w.send({ item: 'A' })).resolves.toEqual({ ok: true });

    expect(callAt(0).key).toBeTruthy();
    expect(callAt(1).key).toBeTruthy();
    expect(callAt(1).key).not.toBe(callAt(0).key);
  });

  it('7. path given as a function of the body: builds the URL from the body', async () => {
    answers.push(jsonResponse(200, { status: 'APPROVED' }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const w = keyedWrite<{ id: string; outcome: string }, { status: string }>(
      api,
      'POST',
      (b) => `/api/approvals/${b.id}/decide`,
    );

    await expect(w.send({ id: 'req-42', outcome: 'APPROVED' })).resolves.toEqual({
      status: 'APPROVED',
    });

    expect(callAt(0).url).toBe('/api/approvals/req-42/decide');
    expect(callAt(0).method).toBe('POST');
  });

  it('8. the key is non-empty and is a UUID passed to ApiClient.command as its 4th argument', async () => {
    answers.push(jsonResponse(200, { ok: true }));

    const api = new ApiClient(fetchSpy as unknown as typeof fetch);
    const commandSpy = vi.spyOn(api, 'command');
    const w = keyedWrite<{ name: string }, { ok: boolean }>(api, 'POST', '/api/groups');

    await w.send({ name: 'USD-group' });

    expect(commandSpy).toHaveBeenCalledTimes(1);
    const [method, path, body, key] = commandSpy.mock.calls[0]!;
    expect(method).toBe('POST');
    expect(path).toBe('/api/groups');
    expect(body).toEqual({ name: 'USD-group' });
    expect(typeof key).toBe('string');
    expect(key.length).toBeGreaterThan(0);
    expect(key).toMatch(UUID_RE);
  });
});
