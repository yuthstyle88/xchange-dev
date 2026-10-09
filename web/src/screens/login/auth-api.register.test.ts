// #763 (login follow-up of #762): the invite step registers an Identity account, then claims.
// Server side (api/src/presentation/http/auth.rs @ abb7ea1):
//   POST /api/auth/register  {phone, password, email?}  → 201 {registered: true}, no cookie   (:370-404)
//   a refusal is {"error": {"code", "message"}}; a refused claim also carries
//   "reason": "inviteInvalid" | "alreadyLinked" | "userNotFound" | "userDisabled"   (#763 [required])
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
//
// The API this test pins, added to `web/src/screens/login/auth-api.ts` beside #762's three calls:
//
//   AuthApi.register(p: { phone: string; password: string; email?: string }): Promise<unknown>;
//
// and that a refusal's `reason` reaches the caller as `ApiError.reason` (a string, or undefined when absent).
// The module is reached through import.meta.glob so this file compiles before the new API exists.
import { describe, expect, it } from 'vitest';
import { ApiClient, ApiError } from '../../shared/api/client';

interface RegisterParams {
  phone: string;
  password: string;
  email?: string;
}
interface RegisterApi {
  register(p: RegisterParams): Promise<unknown>;
  claim(p: { identifier: string; password: string; inviteCode: string }): Promise<unknown>;
}
type AuthApiFn = (api: Pick<ApiClient, 'post'>) => RegisterApi;

const modules = import.meta.glob<{ authApi?: unknown }>('./auth-api.ts', { eager: true });

function authApi(api: Pick<ApiClient, 'post'>): RegisterApi {
  const fn = modules['./auth-api.ts']?.authApi;
  expect(fn, 'web/src/screens/login/auth-api.ts exports authApi').toBeTypeOf('function');
  const made = (fn as AuthApiFn)(api);
  expect(made.register, 'authApi(...).register is a function').toBeTypeOf('function');
  return made;
}

function fakePost(answers: unknown[]) {
  const posted: Array<{ path: string; body: unknown }> = [];
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

function reasonOf(err: unknown): unknown {
  return (err as { reason?: unknown }).reason;
}

describe('authApi.register (#763)', () => {
  it('posts exactly {phone, password, email} to /api/auth/register', async () => {
    const { api, posted } = fakePost([{ registered: true }]);
    await authApi(api).register({ phone: '0812345678', password: 'secret-pw-1', email: 'nok@example.com' });
    expect(posted).toEqual([
      {
        path: '/api/auth/register',
        body: { phone: '0812345678', password: 'secret-pw-1', email: 'nok@example.com' },
      },
    ]);
  });

  it.each([
    ['an empty string', ''],
    ['only spaces', '   '],
    ['absent', undefined],
  ])('an e-mail that is %s is not sent at all', async (_n, email) => {
    const { api, posted } = fakePost([{ registered: true }]);
    await authApi(api).register({ phone: '0812345678', password: 'secret-pw-1', email });
    expect(posted).toHaveLength(1);
    expect(posted[0]!.path).toBe('/api/auth/register');
    expect(posted[0]!.body).toEqual({ phone: '0812345678', password: 'secret-pw-1' });
    expect(Object.prototype.hasOwnProperty.call(posted[0]!.body, 'email')).toBe(false);
  });

  it('passes a refusal through unchanged', async () => {
    const refusal = new ApiError({ code: 'alreadyRegistered', message: 'x', status: 409 });
    const { api } = fakePost([refusal]);
    await expect(authApi(api).register({ phone: '0812345678', password: 'pw' })).rejects.toBe(refusal);
  });
});

describe('over the real ApiClient (#763)', () => {
  function client(status: number, body: unknown, seen: Array<{ url: string; init: RequestInit }>) {
    return new ApiClient(async (url: string, init?: RequestInit) => {
      seen.push({ url, init: init ?? {} });
      return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
    });
  }

  it('register answers 201 {registered: true} without throwing, through ApiClient.post', async () => {
    const seen: Array<{ url: string; init: RequestInit }> = [];
    await authApi(client(201, { registered: true }, seen)).register({ phone: '0812345678', password: 'secret-pw-1' });
    expect(seen).toHaveLength(1);
    expect(seen[0]!.url).toBe('/api/auth/register');
    expect(seen[0]!.init.method).toBe('POST');
    expect(new Headers(seen[0]!.init.headers).get('x-xchange-request')).toBe('1');
    expect(JSON.parse(String(seen[0]!.init.body))).toEqual({ phone: '0812345678', password: 'secret-pw-1' });
  });

  it.each(['inviteInvalid', 'alreadyLinked', 'userNotFound', 'userDisabled'])(
    'a refused claim with reason %s reaches the caller as ApiError {code: inviteRefused, reason}',
    async (reason) => {
      const seen: Array<{ url: string; init: RequestInit }> = [];
      const auth = authApi(
        client(400, { error: { code: 'inviteRefused', reason, message: 'ข้อความของเซิร์ฟเวอร์' } }, seen),
      );
      const err = await auth.claim({ identifier: '0812345678', password: 'secret-pw-1', inviteCode: 'INV-1' }).then(
        () => undefined,
        (e: unknown) => e,
      );
      expect(err).toBeInstanceOf(ApiError);
      expect((err as ApiError).code).toBe('inviteRefused');
      expect((err as ApiError).status).toBe(400);
      expect(reasonOf(err)).toBe(reason);
      expect(seen[0]!.url).toBe('/api/auth/invite/claim');
    },
  );

  it('a refusal without a reason has reason undefined', async () => {
    const seen: Array<{ url: string; init: RequestInit }> = [];
    const auth = authApi(client(409, { error: { code: 'alreadyRegistered', message: 'x' } }, seen));
    const err = await auth.register({ phone: '0812345678', password: 'secret-pw-1' }).then(
      () => undefined,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('alreadyRegistered');
    expect(reasonOf(err)).toBeUndefined();
  });
});
