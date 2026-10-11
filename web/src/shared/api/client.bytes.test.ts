// #539 (lane controller's ruling on gap 3, issuecomment on #539): ApiClient.postBytes — the raw upload the desk sends
// to `path.upload` (`api/src/presentation/http/files.rs:140`: the body is the file, its type is the Content-Type, the
// answer is `{uploadKey}`). Written before the implementation by the test-writer (claude/709ec749). Do not edit,
// delete or weaken these tests; add your own elsewhere.
//
// [required]: `postBytes<R>(path: string, body: Blob, contentType: string, idempotencyKey: string): Promise<R>`, sending
// like `command` does (same-origin credentials so the session cookie goes, the `x-xchange-request` marker, the
// `Idempotency-Key`), with the Blob as the body and the given Content-Type, and refusing a non-2xx the way the JSON
// methods do (`ApiError` with the server's code, field and status). The method is reached through a cast so a client
// without it fails an assertion here, not the typecheck.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from './client';

type PostBytes = <R>(path: string, body: Blob, contentType: string, idempotencyKey: string) => Promise<R>;

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

function postBytes(client: ApiClient): PostBytes {
  const m = (client as unknown as { postBytes?: PostBytes }).postBytes;
  expect(typeof m, 'ApiClient.postBytes exists').toBe('function');
  return m!.bind(client) as PostBytes;
}

const FILE = new Blob([new Uint8Array([0xff, 0xd8, 0xff, 0x01, 0x02])], { type: 'image/jpeg' });

beforeEach(() => {
  calls = [];
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('ApiClient.postBytes', () => {
  it('B1 posts the Blob itself as the body, with the given Content-Type', async () => {
    stubFetch(200, { uploadKey: 'k-1' });
    await postBytes(new ApiClient())('/api/upload', FILE, 'image/jpeg', 'key-1');
    expect(calls).toHaveLength(1);
    const { url, init } = calls[0]!;
    expect(url).toBe('/api/upload');
    expect(init.method).toBe('POST');
    expect(header(init, 'content-type')).toBe('image/jpeg');
    const sent = init.body as Blob;
    expect(sent instanceof Blob, 'the body is a Blob, not JSON').toBe(true);
    expect(new Uint8Array(await sent.arrayBuffer())).toEqual(new Uint8Array(await FILE.arrayBuffer()));
  });

  it('B2 passes the session cookie (same-origin credentials), the marker and the Idempotency-Key', async () => {
    stubFetch(200, { uploadKey: 'k-1' });
    await postBytes(new ApiClient())('/api/staff/upload', FILE, 'application/pdf', 'idem-77');
    const { init } = calls[0]!;
    expect(init.credentials).toBe('same-origin');
    expect(header(init, 'x-xchange-request')).toBe('1');
    expect(header(init, 'idempotency-key')).toBe('idem-77');
    expect(header(init, 'content-type')).toBe('application/pdf');
  });

  it('B3 answers the JSON the server sent', async () => {
    stubFetch(200, { uploadKey: 'passport-7f3a' });
    await expect(postBytes(new ApiClient())('/api/upload', FILE, 'image/jpeg', 'k')).resolves.toEqual({
      uploadKey: 'passport-7f3a',
    });
  });

  it('B4 a non-2xx with a v3 error is refused like the JSON methods: ApiError with code, field and status', async () => {
    stubFetch(400, { error: { code: 'validationFailed', field: 'file', message: 'ไฟล์ใหญ่เกินไป' } });
    const err = await postBytes(new ApiClient())('/api/upload', FILE, 'image/jpeg', 'k').catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('validationFailed');
    expect((err as ApiError).field).toBe('file');
    expect((err as ApiError).status).toBe(400);
    expect((err as ApiError).message).toBe('ไฟล์ใหญ่เกินไป');
  });

  it("B5 a non-2xx with v2's string error is refused too (code 'refused', the server's sentence)", async () => {
    stubFetch(415, { error: 'รองรับ JPG, PNG, WebP หรือ PDF กรุณาแปลงไฟล์ HEIC เป็น JPG ก่อน' });
    const err = await postBytes(new ApiClient())('/api/upload', FILE, 'image/heic', 'k').catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('refused');
    expect((err as ApiError).status).toBe(415);
    expect((err as ApiError).message).toBe('รองรับ JPG, PNG, WebP หรือ PDF กรุณาแปลงไฟล์ HEIC เป็น JPG ก่อน');
  });

  it('B6 a 5xx that is not JSON is an ApiError with the status', async () => {
    stubFetch(502, '<html>bad gateway</html>', 'text/html');
    const err = await postBytes(new ApiClient())('/api/upload', FILE, 'image/jpeg', 'k').catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(502);
  });

  it('B7 an empty idempotency key is refused before anything is sent, like command', async () => {
    stubFetch(200, {});
    const send = postBytes(new ApiClient());
    let thrown: unknown;
    try {
      await send('/api/upload', FILE, 'image/jpeg', '  ');
    } catch (e) {
      thrown = e;
    }
    expect(String(thrown)).toMatch(/idempotency/i);
    expect(calls).toHaveLength(0);
  });

  it('B8 uses the injected fetch when one is given', async () => {
    const injected = vi.fn(async () => new Response(JSON.stringify({ uploadKey: 'x' }), { status: 200 }));
    stubFetch(500, {});
    await postBytes(new ApiClient(injected as unknown as typeof fetch))('/api/upload', FILE, 'image/jpeg', 'k');
    expect(injected).toHaveBeenCalledTimes(1);
    expect(calls).toHaveLength(0);
  });
});
