// V1-login (#396 §7 `auth-api.ts` [required], §8): the login screen's two calls to `/api/auth`,
// made through V0-B's `ApiClient.post` (#393 §7.9) and never through a `fetch` of its own.
// v2: every call is `POST /api/auth` with body `{action, payload}`
// (xchange-v2-source@4d0bdbb:src/devices-public/login.html:2). v3's endpoint:
// api/src/presentation/http/auth.rs:76-132 (`status` :93-100, `login` :101-120).
// v3 has no setup mode (#396 R7), so `login` sends no `name`.
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/auth-api.ts`:
//
//   export interface AuthUser { id: string; username: string; name: string; role: string; branchId: string | null }
//   export interface AuthApi {
//     status(): Promise<{ user: AuthUser | null }>;
//     login(p: { username: string; password: string }): Promise<{ user: AuthUser; mustChange?: boolean }>;
//   }
//   export function authApi(api: Pick<ApiClient, 'post'>): AuthApi;
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from '../../shared/api/client';
import { authApi, type AuthUser } from './auth-api';

interface Posted {
  path: string;
  body: unknown;
}

const NOK: AuthUser = { id: 'employee-7', username: 'nok', name: 'Nok', role: 'FRONT', branchId: 'XC-B01' };

/** A fake `ApiClient` with only `post`: records each call and answers from a script. */
function fakePost(answers: unknown[]) {
  const posted: Posted[] = [];
  let i = 0;
  const api = {
    post: async <B, R>(path: string, body: B): Promise<R> => {
      posted.push({ path, body });
      const a = answers[Math.min(i, answers.length - 1)];
      i += 1;
      if (a instanceof Error) throw a;
      return a as R;
    },
  };
  return { api, posted };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('authApi (#396)', () => {
  it('status posts {action:"status", payload:{}} to /api/auth and answers the user', async () => {
    const { api, posted } = fakePost([{ setupRequired: false, user: NOK }]);
    const answer = await authApi(api).status();
    expect(posted).toEqual([{ path: '/api/auth', body: { action: 'status', payload: {} } }]);
    expect(answer.user).toEqual(NOK);
  });

  it('status answers user null when nobody is signed in', async () => {
    const { api } = fakePost([{ setupRequired: false, user: null }]);
    const answer = await authApi(api).status();
    expect(answer.user).toBeNull();
  });

  it('login posts {action:"login", payload:{username, password}} with no name field', async () => {
    const { api, posted } = fakePost([{ user: NOK, mustChange: true }]);
    const answer = await authApi(api).login({ username: 'nok', password: 'secret-pw-1' });
    expect(posted).toEqual([
      { path: '/api/auth', body: { action: 'login', payload: { username: 'nok', password: 'secret-pw-1' } } },
    ]);
    expect(answer.user).toEqual(NOK);
    expect(answer.mustChange).toBe(true);
  });

  it('login answers the owner, who has no mustChange', async () => {
    const owner: AuthUser = { id: 'owner', username: 'owner1', name: 'Somchai', role: 'OWNER', branchId: null };
    const { api } = fakePost([{ user: owner }]);
    const answer = await authApi(api).login({ username: 'owner1', password: 'secret-pw-1' });
    expect(answer.user).toEqual(owner);
    expect(answer.mustChange).toBeUndefined();
  });

  it('passes a refusal through unchanged', async () => {
    const refusal = new ApiError({ code: 'refused', message: 'ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง', status: 400 });
    const { api } = fakePost([refusal]);
    await expect(authApi(api).login({ username: 'nok', password: 'wrong-pw' })).rejects.toBe(refusal);
  });

  it('makes no fetch of its own: every call goes through post', async () => {
    const fetch = vi.fn(async () => {
      throw new Error('authApi called fetch itself');
    });
    vi.stubGlobal('fetch', fetch);
    const { api, posted } = fakePost([{ setupRequired: false, user: null }, { user: NOK, mustChange: false }]);
    const auth = authApi(api);
    await auth.status();
    await auth.login({ username: 'nok', password: 'secret-pw-1' });
    expect(fetch).not.toHaveBeenCalled();
    expect(posted).toHaveLength(2);
  });

  it('over the real ApiClient: a v2 refusal {"error": sentence} reaches the caller as ApiError refused, verbatim', async () => {
    const seen: Array<{ url: string; init: RequestInit }> = [];
    const fakeFetch = async (url: string, init?: RequestInit): Promise<Response> => {
      seen.push({ url, init: init ?? {} });
      return new Response(JSON.stringify({ error: 'ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' },
      });
    };
    const auth = authApi(new ApiClient(fakeFetch));
    const err = await auth.login({ username: 'nok', password: 'secret-pw-1' }).then(
      () => undefined,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('refused');
    expect((err as ApiError).message).toBe('ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที');
    expect(seen).toHaveLength(1);
    expect(seen[0]!.url).toBe('/api/auth');
    expect(seen[0]!.init.method).toBe('POST');
    expect(new Headers(seen[0]!.init.headers).get('x-xchange-request')).toBe('1');
    expect(JSON.parse(String(seen[0]!.init.body))).toEqual({
      action: 'login',
      payload: { username: 'nok', password: 'secret-pw-1' },
    });
  });
});
