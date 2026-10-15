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

function makeThreeFieldForm(): FormData {
  const form = new FormData();
  form.set('documentType', 'NATIONAL_ID');
  form.set('documentNumber', '1100700123456');
  form.set('file', new Blob(['img'], { type: 'image/jpeg' }), 'id.jpg');
  return form;
}

beforeEach(() => {
  calls = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('ApiClient.postForm', () => {
  it('R1 sends one POST to the path with same-origin credentials, the FormData body, x-xchange-request: 1, and idempotency-key', async () => {
    stubFetch(201, { caseId: 'c-1' });
    const form = makeThreeFieldForm();
    await new ApiClient().postForm('/api/verify/branch-intake', form, 'idem-key-1');

    expect(calls).toHaveLength(1);
    const { url, init } = calls[0]!;
    expect(url).toBe('/api/verify/branch-intake');
    expect(init.method).toBe('POST');
    expect(init.credentials).toBe('same-origin');
    expect(init.body).toBe(form);
    expect(header(init, 'x-xchange-request')).toBe('1');
    expect(header(init, 'idempotency-key')).toBe('idem-key-1');
  });

  it('R2 does not set content-type so fetch sets the multipart boundary', async () => {
    stubFetch(201, { caseId: 'c-1' });
    await new ApiClient().postForm('/api/verify/branch-intake', makeThreeFieldForm(), 'idem-key-2');

    expect(calls).toHaveLength(1);
    const { init } = calls[0]!;
    expect(header(init, 'content-type')).toBeNull();
  });

  it('R3 throws before fetch is called when idempotencyKey is empty or whitespace', async () => {
    stubFetch(201, { caseId: 'c-1' });
    const client = new ApiClient();
    const form = makeThreeFieldForm();

    expect(() => client.postForm('/api/verify/branch-intake', form, '')).toThrow(/idempotency/i);
    expect(() => client.postForm('/api/verify/branch-intake', form, '   ')).toThrow(/idempotency/i);
    expect(calls).toHaveLength(0);
  });

  it('R4 rejects with ApiError code roleNotAllowedHere and status 403 on 403 {error:{code:"roleNotAllowedHere"}}', async () => {
    stubFetch(403, { error: { code: 'roleNotAllowedHere' } });
    const err = await new ApiClient()
      .postForm('/api/verify/branch-intake', makeThreeFieldForm(), 'idem-key-4')
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('roleNotAllowedHere');
    expect((err as ApiError).status).toBe(403);
  });

  it('R5 rejects with ApiError status 500 on a 500 with a non-JSON body', async () => {
    stubFetch(500, '<html>internal server error</html>', 'text/html');
    const err = await new ApiClient()
      .postForm('/api/verify/branch-intake', makeThreeFieldForm(), 'idem-key-5')
      .catch((e: unknown) => e);

    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(500);
  });

  it('R6 resolves to {caseId:"c-1"} on 201 {caseId:"c-1"}', async () => {
    stubFetch(201, { caseId: 'c-1' });
    const result = await new ApiClient().postForm<{ caseId: string }>(
      '/api/verify/branch-intake',
      makeThreeFieldForm(),
      'idem-key-6',
    );

    expect(result).toEqual({ caseId: 'c-1' });
  });
});
