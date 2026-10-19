import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiError } from './client';

function refusal(status: number, code: string): Response {
  return new Response(JSON.stringify({ error: { code, message: `${code} (test)` } }), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubLocation(pathname: string, search = '', hash = '') {
  const assign = vi.fn<(url: string) => void>();
  vi.stubGlobal('location', { pathname, search, hash, assign });
  return assign;
}

async function freshClient() {
  vi.resetModules();
  return import('./client');
}

describe('W5: ApiClient default fetch redirects on 403 passwordChangeRequired', () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('a view answered 403 passwordChangeRequired on /admin navigates to /login?next=%2Fadmin once and still rejects with the refusal', async () => {
    const assign = stubLocation('/admin');
    vi.stubGlobal('fetch', vi.fn(async () => refusal(403, 'passwordChangeRequired')));
    const { ApiClient } = await freshClient();

    const err = (await new ApiClient().view('admin_overview').catch((e: unknown) => e)) as ApiError;

    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/login?next=%2Fadmin');
    expect(err.status).toBe(403);
    expect(err.code).toBe('passwordChangeRequired');
  });

  it('a view answered 403 passwordChangeRequired on /login does not navigate', async () => {
    const assign = stubLocation('/login');
    vi.stubGlobal('fetch', vi.fn(async () => refusal(403, 'passwordChangeRequired')));
    const { ApiClient } = await freshClient();

    const err = (await new ApiClient().view('admin_overview').catch((e: unknown) => e)) as ApiError;

    expect(assign).not.toHaveBeenCalled();
    expect(err.status).toBe(403);
    expect(err.code).toBe('passwordChangeRequired');
  });

  it('three 403 passwordChangeRequired answers in a row navigate only once', async () => {
    const assign = stubLocation('/admin', '?branch=MAIN', '#rates');
    vi.stubGlobal('fetch', vi.fn(async () => refusal(403, 'passwordChangeRequired')));
    const { ApiClient } = await freshClient();
    const client = new ApiClient();

    for (let i = 0; i < 3; i++) {
      await expect(client.view('admin_overview')).rejects.toMatchObject({
        status: 403,
        code: 'passwordChangeRequired',
      });
    }

    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/login?next=%2Fadmin%3Fbranch%3DMAIN%23rates');
  });

  it('new ApiClient(fake) with a fetchFn given does not navigate from the client', async () => {
    const assign = stubLocation('/admin');
    const fake = vi.fn(async () => refusal(403, 'passwordChangeRequired'));
    const { ApiClient } = await freshClient();

    await expect(new ApiClient(fake).view('admin_overview')).rejects.toMatchObject({
      status: 403,
      code: 'passwordChangeRequired',
    });

    expect(fake).toHaveBeenCalledTimes(1);
    expect(assign).not.toHaveBeenCalled();
  });

  it('a 403 whose body is not JSON does not navigate and still surfaces the refusal', async () => {
    const assign = stubLocation('/admin');
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('plain forbidden', { status: 403, headers: { 'content-type': 'text/plain' } })),
    );
    const { ApiClient } = await freshClient();

    const err = (await new ApiClient().view('admin_overview').catch((e: unknown) => e)) as ApiError;

    expect(assign).not.toHaveBeenCalled();
    expect(err.status).toBe(403);
    expect(err.code).toBe('httpError');
  });
});
