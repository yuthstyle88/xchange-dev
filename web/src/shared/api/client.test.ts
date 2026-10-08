// V0-A: ApiClient and ApiError (docs/design/xchange-v3-web.md §4.2, §4.3, §5.2).
// Written before the implementation. Do not edit, delete or weaken.
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

function header(init: RequestInit, name: string): string | null {
  return new Headers(init.headers).get(name);
}

describe('ApiClient.view', () => {
  it('reads GET /api/v3/views/{name} with branch and since, and answers the envelope', async () => {
    const f = fakeFetch(200, { version: 8, changed: true, data: { n: 1 } });
    const client = new ApiClient(f.fn);
    const answer = await client.view<{ n: number }>('me', { branch: 'XC-B01', since: 7 });
    expect(answer).toEqual({ version: 8, changed: true, data: { n: 1 } });
    expect(f.calls).toHaveLength(1);
    const url = new URL(f.calls[0]!.url, 'http://xc.test');
    expect(url.pathname).toBe('/api/v3/views/me');
    expect(url.searchParams.get('branch')).toBe('XC-B01');
    expect(url.searchParams.get('since')).toBe('7');
    expect(f.calls[0]!.init.method ?? 'GET').toBe('GET');
    expect(f.calls[0]!.init.credentials).toBe('same-origin');
  });

  it('leaves out a parameter it was not given', async () => {
    const f = fakeFetch(200, { version: 1, changed: true, data: null });
    await new ApiClient(f.fn).view('me', {});
    const url = new URL(f.calls[0]!.url, 'http://xc.test');
    expect(url.pathname).toBe('/api/v3/views/me');
    expect(url.searchParams.has('since')).toBe(false);
    expect(url.searchParams.has('branch')).toBe(false);
  });

  it('keeps an unchanged answer without data', async () => {
    const f = fakeFetch(200, { version: 7, changed: false });
    const answer = await new ApiClient(f.fn).view('me', { since: 7 });
    expect(answer.changed).toBe(false);
    expect(answer.version).toBe(7);
    expect(answer.data).toBeUndefined();
  });
});

describe('ApiClient.page', () => {
  it('reads one older page with before and limit, and answers its rows', async () => {
    const f = fakeFetch(200, { version: 3, changed: true, data: [{ id: 'a' }, { id: 'b' }] });
    const rows = await new ApiClient(f.fn).page<{ id: string }>('transactions', {
      branch: 'MAIN',
      before: 'tx-9',
      limit: 50,
    });
    expect(rows).toEqual([{ id: 'a' }, { id: 'b' }]);
    const url = new URL(f.calls[0]!.url, 'http://xc.test');
    expect(url.pathname).toBe('/api/v3/views/transactions');
    expect(url.searchParams.get('before')).toBe('tx-9');
    expect(url.searchParams.get('limit')).toBe('50');
    expect(url.searchParams.get('branch')).toBe('MAIN');
    expect(url.searchParams.has('since')).toBe(false);
  });
});

describe('ApiClient.act', () => {
  it('POSTs JSON to /api/v3/{area}/{action} with the request marker', async () => {
    const f = fakeFetch(200, { version: 9, saved: true });
    const answer = await new ApiClient(f.fn).act<{ rate: string }, { version: number; saved: boolean }>(
      'rates',
      'save',
      { rate: '35.10' },
    );
    expect(answer).toEqual({ version: 9, saved: true });
    const { url, init } = f.calls[0]!;
    expect(new URL(url, 'http://xc.test').pathname).toBe('/api/v3/rates/save');
    expect(init.method).toBe('POST');
    expect(init.credentials).toBe('same-origin');
    expect(header(init, 'x-xchange-request')).toBe('1');
    expect(header(init, 'content-type')).toMatch(/^application\/json/);
    expect(JSON.parse(String(init.body))).toEqual({ rate: '35.10' });
  });

  it('turns a refusal into an ApiError with its code, field, message and status', async () => {
    const message = 'รหัสผ่านต้องมี 12–128 ตัวอักษร';
    const f = fakeFetch(400, { error: { code: 'employeePasswordLength', field: 'password', message } });
    const err = await new ApiClient(f.fn).act('employees', 'create', { password: 'x' }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const e = err as ApiError;
    expect(e.code).toBe('employeePasswordLength');
    expect(e.field).toBe('password');
    expect(e.message).toBe(message);
    expect(e.status).toBe(400);
  });

  it('keeps field absent when the refusal names none', async () => {
    const f = fakeFetch(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
    const e = (await new ApiClient(f.fn).act('a', 'b', {}).catch((x: unknown) => x)) as ApiError;
    expect(e).toBeInstanceOf(ApiError);
    expect(e.field).toBeUndefined();
    expect(e.status).toBe(401);
    expect(e.message).toBe('กรุณาเข้าสู่ระบบ');
  });

  it('turns an answer without the refusal shape into an ApiError with its status', async () => {
    const f = fakeFetch(503, 'ui not built', 'text/plain');
    const e = (await new ApiClient(f.fn).view('me', {}).catch((x: unknown) => x)) as ApiError;
    expect(e).toBeInstanceOf(ApiError);
    expect(e.status).toBe(503);
    expect(e.field).toBeUndefined();
  });
});
