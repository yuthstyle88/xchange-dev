import { describe, expect, it } from 'vitest';
import { ApiClient, ApiError } from './client';

interface Call {
  url: string;
  init: RequestInit;
}

function fakeFetch(status: number, body: unknown, contentType = 'application/json') {
  const calls: Call[] = [];
  const text = typeof body === 'string' ? body : JSON.stringify(body);
  const fn = async (url: string, init?: RequestInit): Promise<Response> => {
    calls.push({ url, init: init ?? {} });
    return new Response(text, { status, headers: { 'Content-Type': contentType } });
  };
  return { fn, calls };
}

describe('ApiClient.viewBare (#1892)', () => {
  it('1. 200 bare object resolves to that exact object (deep-equal) without unwrapping .data', async () => {
    const bare = { tier: 'GOLD', points: { available: '10' } };
    const f = fakeFetch(200, bare);
    const answer = await new ApiClient(f.fn).viewBare<typeof bare>('my_membership');
    expect(answer).toEqual({ tier: 'GOLD', points: { available: '10' } });
  });

  it('2. branch: "B2" requests URL /api/v3/views/my_membership?branch=B2', async () => {
    const f = fakeFetch(200, { tier: 'GOLD' });
    await new ApiClient(f.fn).viewBare('my_membership', { branch: 'B2' });
    expect(f.calls).toHaveLength(1);
    expect(f.calls[0]!.url).toBe('/api/v3/views/my_membership?branch=B2');
  });

  it('3. no params requests URL without ?', async () => {
    const f = fakeFetch(200, { tier: 'GOLD' });
    await new ApiClient(f.fn).viewBare('my_membership');
    expect(f.calls).toHaveLength(1);
    expect(f.calls[0]!.url).toBe('/api/v3/views/my_membership');
    expect(f.calls[0]!.url.includes('?')).toBe(false);
  });

  it('4. 403 {error:{code:"outOfScope",message:"x"}} rejects with ApiError code outOfScope and status 403', async () => {
    const f = fakeFetch(403, { error: { code: 'outOfScope', message: 'x' } });
    const err = await new ApiClient(f.fn).viewBare('my_membership').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const apiErr = err as ApiError;
    expect(apiErr.code).toBe('outOfScope');
    expect(apiErr.status).toBe(403);
    expect(apiErr.message).toBe('x');
  });

  it('5. 500 with a body that is not JSON rejects with ApiError status 500 (same as view)', async () => {
    const f = fakeFetch(500, 'upstream gateway crashed', 'text/plain');
    const err = await new ApiClient(f.fn).viewBare('my_membership').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const apiErr = err as ApiError;
    expect(apiErr.status).toBe(500);
    expect(apiErr.code).toBe('httpError');
  });

  it('6. request init sets method: "GET" and credentials: "same-origin"', async () => {
    const f = fakeFetch(200, { tier: 'GOLD' });
    await new ApiClient(f.fn).viewBare('my_membership');
    expect(f.calls).toHaveLength(1);
    expect(f.calls[0]!.init.method).toBe('GET');
    expect(f.calls[0]!.init.credentials).toBe('same-origin');
  });
});
