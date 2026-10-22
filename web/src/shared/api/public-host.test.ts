// PH-0 up-front tests (docs/design/web-public-get.md §3, §4 rows C1–C10). Written before the implementation by
// the XC FE controller's skeleton session (claude/e8020026). Do not edit, delete or weaken these tests; add tests in
// a file of your own.
//
// Pinned:
// - `ApiClient.publicGet<T>(path)`: one `GET` of `path` with `credentials: 'same-origin'` and no body; a 2xx answers
//   the parsed JSON; anything else rejects through the same `parseError` as `view` / `post` (an `ApiError` with the
//   server's `code`, `message` and `status`; a non-JSON body is `httpError`). A path that does not start with `/api/`,
//   or that starts with `/api/v3/`, rejects with an `Error` (not an `ApiError`) and sends no request.
// - `readHostOrg(api)`: one `publicGet('/api/public/host')`; a non-empty `org` answers it; a missing, non-string or
//   empty `org` rejects with `ApiError { code: 'notFound', status: 404 }`; a refusal or a network failure rejects
//   with what `publicGet` rejected with.
import { describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from './client';
import { readHostOrg } from './public-host';

interface Call {
  url: string;
  init: RequestInit | undefined;
}

function server(reply: () => Response | Promise<Response>): { api: ApiClient; calls: Call[] } {
  const calls: Call[] = [];
  const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
    calls.push({ url, init });
    return reply();
  });
  return { api: new ApiClient(fetchFn), calls };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function rejection(p: Promise<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (err) {
    return err;
  }
  throw new Error('expected the promise to reject');
}

describe('ApiClient.publicGet (PH-0)', () => {
  it('C1: a 200 answers the parsed JSON, from one same-origin GET of the path, with no body', async () => {
    const s = server(() => json(200, { org: 'XC', extra: [1, 2] }));
    const out = await s.api.publicGet<{ org: string; extra: number[] }>('/api/public/host');
    expect(out).toEqual({ org: 'XC', extra: [1, 2] });
    expect(s.calls).toHaveLength(1);
    expect(s.calls[0]!.url).toBe('/api/public/host');
    expect((s.calls[0]!.init?.method ?? 'GET').toUpperCase()).toBe('GET');
    expect(s.calls[0]!.init?.credentials).toBe('same-origin');
    expect(s.calls[0]!.init?.body ?? null).toBeNull();
  });

  it('C2: a 5xx with an error body rejects with ApiError carrying its code, message and status', async () => {
    const s = server(() => json(500, { error: { code: 'serverError', message: 'boom' } }));
    const err = await rejection(s.api.publicGet('/api/public/host'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: 'serverError', message: 'boom', status: 500 });
    expect(s.calls).toHaveLength(1);
  });

  it('C3: a 4xx with an error body rejects with ApiError carrying its code, field and status', async () => {
    const s = server(() => json(404, { error: { code: 'notFound', message: 'no such org', field: 'org' } }));
    const err = await rejection(s.api.publicGet('/api/public/orgs/NOPE/branches'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: 'notFound', message: 'no such org', field: 'org', status: 404 });
  });

  it('C4: a non-JSON error body rejects with ApiError httpError and the text as the message', async () => {
    const s = server(() => new Response('Bad Gateway', { status: 502, headers: { 'content-type': 'text/plain' } }));
    const err = await rejection(s.api.publicGet('/api/public/host'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: 'httpError', message: 'Bad Gateway', status: 502 });
  });

  it('C5: a v2 string refusal {"error": "<sentence>"} rejects with code refused, like post', async () => {
    const s = server(() => json(403, { error: 'ไม่มีสิทธิ์' }));
    const err = await rejection(s.api.publicGet('/api/public/host'));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: 'refused', message: 'ไม่มีสิทธิ์', status: 403 });
  });

  it('C6: a network failure rejects with the fetch error itself, not an ApiError', async () => {
    const s = server(() => {
      throw new TypeError('Failed to fetch');
    });
    const err = await rejection(s.api.publicGet('/api/public/host'));
    expect(err).toBeInstanceOf(TypeError);
    expect(err).not.toBeInstanceOf(ApiError);
  });

  it.each(['/api/v3/views/me', 'api/public/host', '/health/ready', 'https://evil.example/api/public/host'])(
    'C7: the path %s rejects with an Error before any request',
    async (path) => {
      const s = server(() => json(200, {}));
      const err = await rejection(s.api.publicGet(path));
      expect(err).toBeInstanceOf(Error);
      expect(err).not.toBeInstanceOf(ApiError);
      expect(String((err as Error).message)).not.toBe('PH-0');
      expect(s.calls).toHaveLength(0);
    },
  );
});

describe('readHostOrg (PH-0)', () => {
  it('C8: answers the org from one GET of /api/public/host', async () => {
    const s = server(() => json(200, { org: 'ORG_1' }));
    await expect(readHostOrg(s.api)).resolves.toBe('ORG_1');
    expect(s.calls.map((c) => c.url)).toEqual(['/api/public/host']);
  });

  it.each<[string, unknown]>([
    ['missing', {}],
    ['empty', { org: '' }],
    ['not a string', { org: 42 }],
    ['a null body', null],
  ])('C9: an org that is %s rejects with ApiError notFound (status 404), after one request', async (_why, body) => {
    const s = server(() => json(200, body));
    const err = await rejection(readHostOrg(s.api));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: 'notFound', status: 404 });
    expect(s.calls).toHaveLength(1);
  });

  it('C10: a refusal rejects with publicGet\'s ApiError unchanged; a network failure with the fetch error', async () => {
    const refused = server(() => json(503, { error: { code: 'serverError', message: 'down' } }));
    const err = await rejection(readHostOrg(refused.api));
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ code: 'serverError', message: 'down', status: 503 });
    expect(refused.calls).toHaveLength(1);

    const offline = server(() => {
      throw new TypeError('Failed to fetch');
    });
    const netErr = await rejection(readHostOrg(offline.api));
    expect(netErr).toBeInstanceOf(TypeError);
    expect(offline.calls).toHaveLength(1);
  });
});
