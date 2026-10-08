// V0-B: ApiClient.post (ticket #393, the controller's decision for SessionStore.signOut).
// Written before the implementation. Do not edit, delete or weaken.
//
// The API this test pins, added to `web/src/shared/api/client.ts`:
//
//   post<B, R>(path: string, body: B): Promise<R>
//
// It POSTs `body` as JSON to a non-v3 route of this server (`/api/auth` for sign-out) with
// `X-Xchange-Request: 1` (api/src/presentation/extractors/origin.rs:41-50) and same-origin
// credentials, answers the parsed JSON of a 2xx, and turns anything else into `ApiError` through
// the same mapping as `act`. That mapping also reads v2's string refusal `{"error": "<sentence>"}`
// (api/src/presentation/extractors/actor.rs:64-66 `refuse`) as code `refused` with the sentence as
// the message (ticket #393 §7.9). It throws synchronously, before any request, for a path that
// does not start with `/api/` or that starts with `/api/v3/` (v3's own routes go through `view`,
// `page` and `act`).
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

describe('ApiClient.post', () => {
  it('POSTs the body as JSON to the path with the request marker and same-origin credentials', async () => {
    stubFetch(200, { saved: true });
    await new ApiClient().post('/api/auth', { action: 'logout' });
    expect(calls).toHaveLength(1);
    const { url, init } = calls[0]!;
    expect(new URL(url, 'http://xc.test').pathname).toBe('/api/auth');
    expect(init.method).toBe('POST');
    expect(init.credentials).toBe('same-origin');
    expect(header(init, 'x-xchange-request')).toBe('1');
    expect(header(init, 'content-type')).toMatch(/^application\/json/);
    expect(JSON.parse(String(init.body))).toEqual({ action: 'logout' });
  });

  it('answers the JSON of a 2xx', async () => {
    stubFetch(200, { saved: true, user: { id: 'owner' } });
    const answer = await new ApiClient().post<
      { action: string; payload: Record<string, never> },
      { saved: boolean; user: { id: string } }
    >('/api/auth', { action: 'status', payload: {} });
    expect(answer).toEqual({ saved: true, user: { id: 'owner' } });
  });

  it('turns a refusal into an ApiError with its code, field, message and status', async () => {
    const message = 'รหัสผ่านต้องมี 12–128 ตัวอักษร';
    stubFetch(400, { error: { code: 'employeePasswordLength', field: 'password', message } });
    const err = await new ApiClient().post('/api/staff/password', { password: 'x' }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const e = err as ApiError;
    expect(e.code).toBe('employeePasswordLength');
    expect(e.field).toBe('password');
    expect(e.message).toBe(message);
    expect(e.status).toBe(400);
  });

  it('turns v2\'s string refusal into an ApiError with code refused and the sentence as message', async () => {
    const sentence = 'ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง';
    stubFetch(400, { error: sentence });
    const err = await new ApiClient()
      .post('/api/auth', { action: 'login', payload: {} })
      .catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const e = err as ApiError;
    expect(e.code).toBe('refused');
    expect(e.message).toBe(sentence);
    expect(e.status).toBe(400);
    expect(e.field).toBeUndefined();
  });

  it('turns a non-JSON error answer into an ApiError with its status', async () => {
    stubFetch(502, 'Bad Gateway', 'text/html');
    const err = await new ApiClient().post('/api/auth', { action: 'logout' }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const e = err as ApiError;
    expect(e.status).toBe(502);
    expect(e.field).toBeUndefined();
  });

  it('throws before any request for a path outside /api/', async () => {
    stubFetch(200, {});
    const client = new ApiClient();
    expect(() => client.post('/x', {})).toThrow();
    expect(() => client.post('/login', {})).toThrow();
    expect(() => client.post('api/auth', {})).toThrow();
    expect(calls).toHaveLength(0);
    // the guard is the path check, not a missing method: an /api/ path is sent
    await expect(client.post('/api/auth', { action: 'status' })).resolves.toEqual({});
    expect(calls).toHaveLength(1);
  });

  it('throws before any request for a v3 path', async () => {
    stubFetch(200, {});
    const client = new ApiClient();
    expect(() => client.post('/api/v3/views/me', {})).toThrow();
    expect(() => client.post('/api/v3/rates/save', {})).toThrow();
    expect(() => client.post('/api/v3/x/y', {})).toThrow();
    expect(calls).toHaveLength(0);
    await expect(client.post('/api/staff/logout', {})).resolves.toEqual({});
    expect(calls).toHaveLength(1);
  });
});
