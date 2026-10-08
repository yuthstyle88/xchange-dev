// V1-kiosk-currencies (#399): the write of the currency screen (ticket §7.4 `kiosk-api.ts`, §8,
// V2). Written before the implementation by the lane controller. Do not edit, delete or weaken;
// add tests in a file of your own.
//
// The write stays on the existing endpoint: `configure` on `POST /api/kiosk`, sent through
// `ApiClient.post` (#393 §7.9), rates as decimal strings (W7). No fetch of its own.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from '../../shared/api/client';
import { configure } from './kiosk-api';

const PAYLOAD = {
  branchId: 'B9',
  code: 'USD',
  buy: '33.25',
  sell: '33.50',
  enabled: true,
  previousUpdated: '2026-10-06T05:00:01.123Z',
  partnerName: 'SuperRich',
  partnerCost: '32.9000',
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('configure (#399)', () => {
  it("posts { action: 'configure', payload } to /api/kiosk through ApiClient.post, once", async () => {
    const posts: Array<{ path: string; body: unknown }> = [];
    const api: Pick<ApiClient, 'post'> = {
      post<B, R>(path: string, body: B): Promise<R> {
        posts.push({ path, body });
        return Promise.resolve({ saved: true } as unknown as R);
      },
    };
    await expect(configure(api, PAYLOAD)).resolves.toBeUndefined();
    expect(posts).toEqual([{ path: '/api/kiosk', body: { action: 'configure', payload: PAYLOAD } }]);
  });

  it('sends a partner cost of null as null', async () => {
    const posts: Array<{ path: string; body: unknown }> = [];
    const api: Pick<ApiClient, 'post'> = {
      post<B, R>(path: string, body: B): Promise<R> {
        posts.push({ path, body });
        return Promise.resolve({ saved: true } as unknown as R);
      },
    };
    const p = { ...PAYLOAD, partnerName: '', partnerCost: null };
    await configure(api, p);
    expect(posts).toEqual([{ path: '/api/kiosk', body: { action: 'configure', payload: p } }]);
  });

  it('passes a refusal on as the same ApiError', async () => {
    const refusal = new ApiError({
      code: 'refused',
      message: 'Set valid counter buy and sell rates first.',
      status: 400,
    });
    const api: Pick<ApiClient, 'post'> = {
      post<B, R>(_path: string, _body: B): Promise<R> {
        return Promise.reject(refusal);
      },
    };
    await expect(configure(api, PAYLOAD)).rejects.toBe(refusal);
  });

  it('over a real ApiClient the wire carries the rates as JSON strings, never numbers', async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    const client = new ApiClient(async (input, init) => {
      calls.push({ input, init });
      return new Response(JSON.stringify({ saved: true }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
    const globalFetch = vi.fn(() => Promise.reject(new Error('no fetch of its own')));
    vi.stubGlobal('fetch', globalFetch);

    await configure(client, PAYLOAD);

    expect(globalFetch).not.toHaveBeenCalled();
    expect(calls).toHaveLength(1);
    expect(calls[0]!.input).toBe('/api/kiosk');
    expect(calls[0]!.init?.method).toBe('POST');
    const body = JSON.parse(String(calls[0]!.init?.body));
    expect(body).toEqual({ action: 'configure', payload: PAYLOAD });
    expect(typeof body.payload.buy).toBe('string');
    expect(typeof body.payload.sell).toBe('string');
    expect(typeof body.payload.partnerCost).toBe('string');
  });

  it("over a real ApiClient v2's refusal arrives as ApiError { code: 'refused', message }", async () => {
    const client = new ApiClient(
      async () =>
        new Response(JSON.stringify({ error: 'เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน' }), {
          status: 400,
          headers: { 'content-type': 'application/json' },
        }),
    );
    const error = await configure(client, PAYLOAD).then(
      () => undefined,
      (e: unknown) => e,
    );
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).code).toBe('refused');
    expect((error as ApiError).message).toBe('เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน');
    expect((error as ApiError).status).toBe(400);
  });
});
