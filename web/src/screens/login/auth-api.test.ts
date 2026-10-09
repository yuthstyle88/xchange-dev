// OR-3 web follow-up (#762 §7 `auth-api.ts` [required], §8): the login screen's three calls to the
// Identity sign-in BFF that OR-3 (#752, merged 8234e77) added, made through `ApiClient.post`
// and never through a `fetch` of its own.
// Server side (api/src/presentation/http/auth.rs @ 8234e77):
//   POST /api/auth/session        {}                                   → 200 {user: SessionUser | null}  (:406-425)
//   POST /api/auth/login          {identifier, password}               → 200 {user: SessionUser}         (:196-236)
//   POST /api/auth/invite/claim   {identifier, password, inviteCode}   → 200 {user: SessionUser}         (:238-285)
//   a refusal is {"error": {"code", "message"}} (:145-154)
// SessionUser: api/crates/xc-auth/src/application/session/session_service.rs:27-41.
// Replaces the #396 pins of the old `/api/auth {action}` calls (the retired file's :49-97, :99-124).
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/auth-api.ts`:
//
//   export interface SessionUserRole { role: string; branchId: string | null }
//   export interface SessionUser { id: string; name: string; organizationId: string; roles: SessionUserRole[] }
//   export interface AuthApi {
//     session(): Promise<{ user: SessionUser | null }>;
//     login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }>;
//     claim(p: { identifier: string; password: string; inviteCode: string }): Promise<{ user: SessionUser }>;
//   }
//   export function authApi(api: Pick<ApiClient, 'post'>): AuthApi;
//
// The module is reached through import.meta.glob so this file compiles before the new API exists.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from '../../shared/api/client';

interface SessionUser {
  id: string;
  name: string;
  organizationId: string;
  roles: Array<{ role: string; branchId: string | null }>;
}
interface NewAuthApi {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: { identifier: string; password: string }): Promise<{ user: SessionUser }>;
  claim(p: { identifier: string; password: string; inviteCode: string }): Promise<{ user: SessionUser }>;
}
type AuthApiFn = (api: Pick<ApiClient, 'post'>) => NewAuthApi;

const modules = import.meta.glob<{ authApi?: unknown }>('./auth-api.ts', { eager: true });

function authApi(api: Pick<ApiClient, 'post'>): NewAuthApi {
  const fn = modules['./auth-api.ts']?.authApi;
  expect(fn, 'web/src/screens/login/auth-api.ts exports authApi').toBeTypeOf('function');
  const made = (fn as AuthApiFn)(api);
  for (const name of ['session', 'login', 'claim'] as const) {
    expect(made[name], `authApi(...).${name} is a function`).toBeTypeOf('function');
  }
  return made;
}

interface Posted {
  path: string;
  body: unknown;
}

const NOK: SessionUser = {
  id: 'user-7',
  name: 'Nok',
  organizationId: 'org-1',
  roles: [{ role: 'TELLER', branchId: 'XC-B01' }],
};

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

describe('authApi over the Identity BFF (OR-3 web)', () => {
  it('session posts {} to /api/auth/session and answers the user', async () => {
    const { api, posted } = fakePost([{ user: NOK }]);
    const answer = await authApi(api).session();
    expect(posted).toEqual([{ path: '/api/auth/session', body: {} }]);
    expect(answer.user).toEqual(NOK);
  });

  it('session answers user null when nobody is signed in', async () => {
    const { api } = fakePost([{ user: null }]);
    const answer = await authApi(api).session();
    expect(answer.user).toBeNull();
  });

  it('login posts exactly {identifier, password} to /api/auth/login', async () => {
    const { api, posted } = fakePost([{ user: NOK }]);
    const answer = await authApi(api).login({ identifier: '0812345678', password: 'secret-pw-1' });
    expect(posted).toEqual([
      { path: '/api/auth/login', body: { identifier: '0812345678', password: 'secret-pw-1' } },
    ]);
    expect(answer).toEqual({ user: NOK });
  });

  it('claim posts exactly {identifier, password, inviteCode} to /api/auth/invite/claim', async () => {
    const { api, posted } = fakePost([{ user: NOK }]);
    const answer = await authApi(api).claim({
      identifier: 'nok@example.com',
      password: 'secret-pw-1',
      inviteCode: 'INV-7Q2K',
    });
    expect(posted).toEqual([
      {
        path: '/api/auth/invite/claim',
        body: { identifier: 'nok@example.com', password: 'secret-pw-1', inviteCode: 'INV-7Q2K' },
      },
    ]);
    expect(answer).toEqual({ user: NOK });
  });

  it('passes a refusal through unchanged', async () => {
    const refusal = new ApiError({ code: 'invalidCredentials', message: 'x', status: 401 });
    const { api } = fakePost([refusal]);
    await expect(authApi(api).login({ identifier: 'nok', password: 'wrong-pw' })).rejects.toBe(refusal);
    const again = fakePost([refusal]);
    await expect(
      authApi(again.api).claim({ identifier: 'nok', password: 'pw', inviteCode: 'C' }),
    ).rejects.toBe(refusal);
  });

  it('never calls the old /api/auth {action} endpoint and makes no fetch of its own', async () => {
    const fetch = vi.fn(async () => {
      throw new Error('authApi called fetch itself');
    });
    vi.stubGlobal('fetch', fetch);
    const { api, posted } = fakePost([{ user: null }, { user: NOK }, { user: NOK }]);
    const auth = authApi(api);
    await auth.session();
    await auth.login({ identifier: 'nok', password: 'secret-pw-1' });
    await auth.claim({ identifier: 'nok', password: 'secret-pw-1', inviteCode: 'C1' });
    expect(fetch).not.toHaveBeenCalled();
    expect(posted.map((p) => p.path)).toEqual(['/api/auth/session', '/api/auth/login', '/api/auth/invite/claim']);
    for (const p of posted) expect(JSON.stringify(p.body)).not.toContain('action');
  });

  it("over the real ApiClient: the BFF's {error:{code,message}} reaches the caller as ApiError with that code", async () => {
    const seen: Array<{ url: string; init: RequestInit }> = [];
    const fakeFetch = async (url: string, init?: RequestInit): Promise<Response> => {
      seen.push({ url, init: init ?? {} });
      return new Response(
        JSON.stringify({ error: { code: 'accountLocked', message: 'บัญชีถูกล็อกชั่วคราว กรุณาลองใหม่ภายหลัง' } }),
        { status: 423, headers: { 'Content-Type': 'application/json' } },
      );
    };
    const auth = authApi(new ApiClient(fakeFetch));
    const err = await auth.login({ identifier: 'nok', password: 'secret-pw-1' }).then(
      () => undefined,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('accountLocked');
    expect((err as ApiError).status).toBe(423);
    expect(seen).toHaveLength(1);
    expect(seen[0]!.url).toBe('/api/auth/login');
    expect(seen[0]!.init.method).toBe('POST');
    expect(new Headers(seen[0]!.init.headers).get('x-xchange-request')).toBe('1');
    expect(JSON.parse(String(seen[0]!.init.body))).toEqual({ identifier: 'nok', password: 'secret-pw-1' });
  });
});
