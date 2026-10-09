// Ticket #597 (CM-1): ApiClient.command.
// Pins the client method for v3 commands.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from './client';

interface Call {
  url: string;
  init: RequestInit;
}

let calls: Call[];

function stubFetch(status: number, body: unknown, contentType = 'application/json') {
  const text = typeof body === 'string' ? body : JSON.stringify(body);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      calls.push({ url, init: init ?? {} });
      return new Response(text, { status, headers: { 'Content-Type': contentType } });
    }),
  );
}

function header(init: RequestInit, name: string): string | null {
  return new Headers(init.headers).get(name);
}

beforeEach(() => {
  calls = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('ApiClient.command', () => {
  it('throws synchronously when idempotency key is empty or whitespace', () => {
    stubFetch(200, { ok: true });
    const client = new ApiClient();

    expect(() => client.command('POST', '/api/v3/test', {}, '')).toThrow(/idempotency/i);
    expect(() => client.command('POST', '/api/v3/test', {}, '   ')).toThrow(/idempotency/i);
    expect(calls).toHaveLength(0);
  });

  it('sends Idempotency-Key and x-xchange-request headers with same-origin credentials', async () => {
    stubFetch(200, { success: true });
    const client = new ApiClient();
    await client.command('POST', '/api/v3/orders/1/submit', { amount: '100' }, 'idem-key-001');

    expect(calls).toHaveLength(1);
    const { url, init } = calls[0]!;
    expect(url).toBe('/api/v3/orders/1/submit');
    expect(init.method).toBe('POST');
    expect(init.credentials).toBe('same-origin');
    expect(header(init, 'x-xchange-request')).toBe('1');
    expect(header(init, 'idempotency-key')).toBe('idem-key-001');
    expect(header(init, 'content-type')).toMatch(/^application\/json/);
    expect(JSON.parse(String(init.body))).toEqual({ amount: '100' });
  });

  it('respects the HTTP method and path as given', async () => {
    stubFetch(200, { updated: true });
    const client = new ApiClient();

    await client.command('PUT', '/api/v3/drafts/42', { title: 'draft' }, 'key-put');
    expect(calls[0]!.init.method).toBe('PUT');
    expect(calls[0]!.url).toBe('/api/v3/drafts/42');

    await client.command('PATCH', 'api/v3/drafts/42', { patch: true }, 'key-patch');
    expect(calls[1]!.init.method).toBe('PATCH');
    expect(calls[1]!.url).toBe('/api/v3/drafts/42');
  });

  it('returns the 2xx payload as parsed JSON', async () => {
    stubFetch(200, { orderId: 'ord-123', status: 'confirmed' });
    const client = new ApiClient();

    const result = await client.command<
      { customerId: string },
      { orderId: string; status: string }
    >('POST', '/api/v3/orders', { customerId: 'cust-1' }, 'key-order');

    expect(result).toEqual({ orderId: 'ord-123', status: 'confirmed' });
  });

  it('preserves 409 idempotencyConflict as an ApiError', async () => {
    stubFetch(409, {
      error: {
        code: 'idempotencyConflict',
        message: 'idempotency key already used for a different request',
      },
    });
    const client = new ApiClient();

    const err = await client
      .command('POST', '/api/v3/orders', { amount: '200' }, 'key-conflict')
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(ApiError);
    const apiErr = err as ApiError;
    expect(apiErr.status).toBe(409);
    expect(apiErr.code).toBe('idempotencyConflict');
    expect(apiErr.message).toBe('idempotency key already used for a different request');
  });

  it('preserves 400 validationFailed with field as an ApiError', async () => {
    stubFetch(400, {
      error: {
        code: 'validationFailed',
        field: 'idempotencyKey',
        message: 'missing or invalid idempotency key',
      },
    });
    const client = new ApiClient();

    const err = await client
      .command('POST', '/api/v3/orders', {}, 'invalid-key')
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(ApiError);
    const apiErr = err as ApiError;
    expect(apiErr.status).toBe(400);
    expect(apiErr.code).toBe('validationFailed');
    expect(apiErr.field).toBe('idempotencyKey');
  });

  it('turns non-JSON 5xx responses into ApiError', async () => {
    stubFetch(502, 'Bad Gateway', 'text/html');
    const client = new ApiClient();

    const err = await client
      .command('POST', '/api/v3/orders', {}, 'server-err-key')
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(ApiError);
    const apiErr = err as ApiError;
    expect(apiErr.status).toBe(502);
    expect(apiErr.code).toBe('httpError');
  });
});
