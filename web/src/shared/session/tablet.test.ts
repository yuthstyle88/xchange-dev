// BT-F1 (kiosk-shared-phone §7.3 T-D6, T-D10; §7.7; §7.8 rows T6, T7, T10, T12, FE side): the branch tablet mark is
// the server's answer, never the URL or storage. Written before the implementation by the XC FE controller
// claude/e8020026. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// What these tests pin (the header of ./tablet.ts is the contract):
// - `readTabletMode(fetchFn)` reads `GET /api/branch-tablet/me` once and returns its answer when it is a well-formed
//   `paired: true`; anything else (`paired: false`, a revoked/unknown cookie, 404, 5xx, a network failure, a body that
//   is not JSON, a `paired: true` with a field missing) is `{paired: false}`, and it never throws.
// - `?shared=1` and `sessionStorage['xc.sharedTablet']` change nothing.
// - `pairFromFragment(api, loc, history)` posts `#pair=<CODE>` once through the ApiClient as `{code}`, strips the
//   fragment on success and on refusal, and answers 'paired' / 'invalid'; no `pair=` ⇒ 'none' and no request.
// - The old QW-3 API (`isSharedTablet`, `signOutAndRestart`, `armTabletIdle`) is gone.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../api/client';
import * as tabletModule from './tablet';
import { pairFromFragment, readTabletMode } from './tablet';

interface Sent {
  method: string;
  path: string;
  body: unknown;
  headers: Headers;
  credentials: RequestCredentials | undefined;
}

let sent: Sent[];
let answer: () => Promise<Response>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
  const u = new URL(input, 'http://localhost');
  sent.push({
    method: (init?.method ?? 'GET').toUpperCase(),
    path: u.pathname,
    body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
    headers: new Headers(init?.headers),
    credentials: init?.credentials,
  });
  return answer();
});

const meReads = () => sent.filter((s) => s.path === '/api/branch-tablet/me');
const pairs = () => sent.filter((s) => s.path === '/api/branch-tablet/pair');

const PAIRED = { paired: true, tabletId: 'bt-1', branchCode: 'MAIN', name: 'Front desk tablet' };

beforeEach(() => {
  sent = [];
  fetchFn.mockClear();
  answer = async () => json(200, { paired: false });
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked here: the rows below do not need it
  }
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('BT-F1: readTabletMode', () => {
  it('T-D6: reads GET /api/branch-tablet/me once, same-origin, and returns the paired answer', async () => {
    answer = async () => json(200, PAIRED);
    const mode = await readTabletMode(fetchFn);
    expect(mode).toEqual(PAIRED);
    expect(meReads()).toHaveLength(1);
    expect(meReads()[0]!.method).toBe('GET');
    expect(meReads()[0]!.credentials).toBe('same-origin');
    expect(sent).toHaveLength(1);
  });

  it('T6/T7: a revoked or unknown cookie ({paired: false}) is not paired', async () => {
    answer = async () => json(200, { paired: false });
    await expect(readTabletMode(fetchFn)).resolves.toEqual({ paired: false });
    expect(meReads()).toHaveLength(1);
  });

  it.each([
    ['404 (the BE not deployed yet)', async () => json(404, { error: { code: 'notFound', message: 'nope' } })],
    ['500', async () => json(500, { error: { code: 'serverError', message: 'down' } })],
    ['503 with a non-JSON body', async () => new Response('Service Unavailable', { status: 503 })],
    ['a network failure', async () => Promise.reject(new TypeError('Failed to fetch'))],
    ['a 200 that is not JSON', async () => new Response('<html>oops</html>', { status: 200 })],
    ['a 200 with paired: true but no branchCode', async () => json(200, { paired: true, tabletId: 'bt-1', name: 'x' })],
    ['a 200 with paired: true and an empty branchCode', async () => json(200, { ...PAIRED, branchCode: '' })],
    ['a 200 with paired: "yes"', async () => json(200, { ...PAIRED, paired: 'yes' })],
    ['a 200 with null', async () => json(200, null)],
  ])('T12 fail-open: %s ⇒ {paired: false}, no throw', async (_label, a) => {
    answer = a as () => Promise<Response>;
    await expect(readTabletMode(fetchFn)).resolves.toEqual({ paired: false });
    expect(meReads()).toHaveLength(1);
  });

  it('T12: ?shared=1 and the old sessionStorage mark have no effect', async () => {
    vi.stubGlobal('location', { ...window.location, search: '?branch=MAIN&shared=1', pathname: '/app/queue' });
    try {
      window.sessionStorage.setItem('xc.sharedTablet', '1');
    } catch {
      // storage blocked: the URL flag alone is still checked
    }
    answer = async () => json(200, { paired: false });
    await expect(readTabletMode(fetchFn)).resolves.toEqual({ paired: false });
    expect(meReads()).toHaveLength(1);
  });

  it('the QW-3 API is gone: the module exports no isSharedTablet, signOutAndRestart or armTabletIdle', () => {
    const names = Object.keys(tabletModule);
    expect(names).not.toContain('isSharedTablet');
    expect(names).not.toContain('signOutAndRestart');
    expect(names).not.toContain('armTabletIdle');
  });
});

describe('BT-F1: pairFromFragment (T-D10)', () => {
  function at(hash: string) {
    const loc = { hash, pathname: '/app/queue', search: '?branch=MAIN' };
    const history = { replaceState: vi.fn() };
    return { loc, history };
  }

  it('#pair=<CODE>: one POST /api/branch-tablet/pair {code} through ApiClient; the fragment is stripped; "paired"', async () => {
    answer = async () => json(200, { tabletId: 'bt-1', branchCode: 'MAIN', name: 'Front desk tablet' });
    const { loc, history } = at('#pair=ABCDE-12345');
    const outcome = await pairFromFragment(new ApiClient(fetchFn), loc, history);
    expect(outcome).toBe('paired');
    expect(pairs()).toHaveLength(1);
    expect(pairs()[0]!.method).toBe('POST');
    expect(pairs()[0]!.body).toEqual({ code: 'ABCDE-12345' });
    // ApiClient's write header: proof it went through the client, not a raw fetch.
    expect(pairs()[0]!.headers.get('x-xchange-request')).toBe('1');
    expect(sent).toHaveLength(1);
    expect(history.replaceState).toHaveBeenCalledTimes(1);
    expect(history.replaceState).toHaveBeenCalledWith(null, '', '/app/queue?branch=MAIN');
  });

  it('T10: 400 PAIRING_CODE_INVALID ⇒ "invalid", and the fragment is stripped all the same', async () => {
    answer = async () => json(400, { error: { code: 'PAIRING_CODE_INVALID', message: 'invalid code' } });
    const { loc, history } = at('#pair=ABCDE-12345');
    const outcome = await pairFromFragment(new ApiClient(fetchFn), loc, history);
    expect(outcome).toBe('invalid');
    expect(pairs()).toHaveLength(1);
    expect(history.replaceState).toHaveBeenCalledWith(null, '', '/app/queue?branch=MAIN');
  });

  it('429 RATE_LIMITED or a network failure ⇒ "invalid", no throw, the fragment stripped', async () => {
    for (const a of [
      async () => json(429, { error: { code: 'RATE_LIMITED', message: 'slow down' } }),
      async () => Promise.reject(new TypeError('Failed to fetch')),
    ]) {
      answer = a as () => Promise<Response>;
      const { loc, history } = at('#pair=ABCDE-12345');
      await expect(pairFromFragment(new ApiClient(fetchFn), loc, history)).resolves.toBe('invalid');
      expect(history.replaceState).toHaveBeenCalledWith(null, '', '/app/queue?branch=MAIN');
    }
  });

  it('no #pair= ⇒ "none": no request, and the URL is left alone', async () => {
    for (const hash of ['', '#', '#other=1']) {
      const { loc, history } = at(hash);
      await expect(pairFromFragment(new ApiClient(fetchFn), loc, history)).resolves.toBe('none');
      expect(history.replaceState).not.toHaveBeenCalled();
    }
    expect(sent).toHaveLength(0);
  });
});
