// W5 (#4104): pin passport-read.ts failure mapping, abort, timeout, branch encoding, and default Content-Type.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import { PassportReadError, readPassportOnServer, type ReadFailure } from './passport-read';

interface Call {
  url: string;
  init: RequestInit;
}

const IMAGE = new Blob([new Uint8Array([0xff, 0xd8, 0xff])], { type: 'image/png' });

let calls: Call[];

beforeEach(() => {
  calls = [];
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function stubFetchJson(status: number, body: unknown) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      calls.push({ url, init: init ?? {} });
      return new Response(JSON.stringify(body), {
        status,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );
}

describe('readPassportOnServer (#4104)', () => {
  it('1. when the signal is already aborted, rejects with aborted and makes no POST', async () => {
    stubFetchJson(200, { fields: null, score: 0 });
    const ac = new AbortController();
    ac.abort();

    await expect(
      readPassportOnServer(new ApiClient(), IMAGE, { branch: 'XC-B01', signal: ac.signal }),
    ).rejects.toEqual(new PassportReadError('aborted'));
    expect(calls).toHaveLength(0);
  });

  it.each([
    ['passportReaderTimeout', 504, 'timeout'],
    ['passportReaderBusy', 429, 'busy'],
    ['passportReaderNotConfigured', 503, 'notConfigured'],
    ['imageTooLarge', 413, 'imageTooLarge'],
    ['imageInvalid', 400, 'imageInvalid'],
    ['pdfNotAccepted', 415, 'imageInvalid'],
    ['imageTypeNotAccepted', 415, 'imageInvalid'],
    ['roleNotAllowedHere', 403, 'notAllowed'],
    ['signIn', 401, 'notAllowed'],
    ['somethingElse', 500, 'failed'],
  ] as const satisfies readonly [string, number, ReadFailure][])(
    '2. maps server code %s (%d) to %s',
    async (code, status, expected) => {
      stubFetchJson(status, { error: { code, message: 'refused' } });
      const ac = new AbortController();

      await expect(
        readPassportOnServer(new ApiClient(), IMAGE, { branch: 'XC-B01', signal: ac.signal }),
      ).rejects.toEqual(new PassportReadError(expected));
    },
  );

  it('3. maps a non-ApiError to failed', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        throw new TypeError('network down');
      }),
    );
    const ac = new AbortController();

    await expect(
      readPassportOnServer(new ApiClient(), IMAGE, { branch: 'XC-B01', signal: ac.signal }),
    ).rejects.toEqual(new PassportReadError('failed'));
  });

  it('4. abort while the request is in flight rejects with aborted, not failed', async () => {
    // a) postBytes stays pending: race onAbort (:63) rejects with 'aborted'
    vi.stubGlobal(
      'fetch',
      vi.fn((url: string, init?: RequestInit) => {
        calls.push({ url, init: init ?? {} });
        return new Promise<Response>(() => {});
      }),
    );
    const ac1 = new AbortController();
    const p1 = readPassportOnServer(new ApiClient(), IMAGE, {
      branch: 'XC-B01',
      signal: ac1.signal,
    });
    expect(calls).toHaveLength(1);
    ac1.abort();
    await expect(p1).rejects.toEqual(new PassportReadError('aborted'));

    // b) postBytes rejects synchronously when signal is already aborted on catch (:82)
    const ac2 = new AbortController();
    const apiWithAbortingPost = {
      postBytes: vi.fn(async () => {
        ac2.abort();
        throw new TypeError('Aborted');
      }),
    } as unknown as ApiClient;
    await expect(
      readPassportOnServer(apiWithAbortingPost, IMAGE, { branch: 'XC-B01', signal: ac2.signal }),
    ).rejects.toEqual(new PassportReadError('aborted'));
  });

  it('5. when 120 s pass, rejects with timeout', async () => {
    vi.useFakeTimers();
    vi.stubGlobal(
      'fetch',
      vi.fn(
        () =>
          new Promise<Response>((resolve) => {
            setTimeout(
              () =>
                resolve(
                  new Response(JSON.stringify({ fields: null, score: 0 }), {
                    status: 200,
                    headers: { 'Content-Type': 'application/json' },
                  }),
                ),
              125_000,
            );
          }),
      ),
    );
    const ac = new AbortController();

    const p = readPassportOnServer(new ApiClient(), IMAGE, {
      branch: 'XC-B01',
      signal: ac.signal,
    });
    const assertion = expect(p).rejects.toEqual(new PassportReadError('timeout'));
    await vi.advanceTimersByTimeAsync(125_000);
    await assertion;
  });

  it('6. branch "A B" is URI-encoded as ?branch=A%20B in the path', async () => {
    stubFetchJson(200, { fields: null, score: 0 });
    const ac = new AbortController();

    await readPassportOnServer(new ApiClient(), IMAGE, { branch: 'A B', signal: ac.signal });
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe('/api/v3/compute/passport_read?branch=A%20B');
  });

  it('7. an image with an empty type is sent with Content-Type image/jpeg', async () => {
    stubFetchJson(200, { fields: null, score: 0 });
    const ac = new AbortController();
    const untyped = new Blob([new Uint8Array([1, 2, 3])]);
    expect(untyped.type).toBe('');

    await readPassportOnServer(new ApiClient(), untyped, { branch: 'XC-B01', signal: ac.signal });
    expect(calls).toHaveLength(1);
    expect(new Headers(calls[0]!.init.headers).get('content-type')).toBe('image/jpeg');
  });
});
