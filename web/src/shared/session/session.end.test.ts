// V0-B-session-end (#492; design docs/design/xchange-v3-web.md §12.19.1-§12.19.3, rules SE1-SE7):
// a session ends without signing out, on a view refusal or an account switch, as v2's account bar
// did (xchange-v2-source@4d0bdbb:src/devices-public/account-bar.js:42, :44).
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/shared/session/session.ts` (names [proposed] in the design,
// behaviour [required]):
//
//   export type EndReason = 'expired' | 'switched';
//   createSessionStore(api) returns a store that also has
//     ended: Accessor<EndReason | undefined>;   // undefined while the session runs
//     end(reason: EndReason): void;             // SE1: the first reason wins; stops the me poll;
//                                               //      keeps me(); makes no request
//   export function endsSession(status: number, code: string | undefined): boolean;      // SE4
//   export function endingFetch(inner: Fetch, onEnd: () => void): Fetch;                  // SE5
//   export function sharedApi(): ApiClient;   // SE6: one per page; the global fetch looked up per call
//   export function sharedSession();          // built on sharedApi()
//
// The store is never typed by name here, so the implementer chooses how `SessionStore` carries
// `ended` / `end`.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixtures (`kind`/`role`/`username`) → `meOf(…)` (`web/src/test-support/me.ts`, design §3.3). Nothing else changes. "another role" → "other roles" (#1030 §3.10, ruling (a): session.ts compares the role grants); the
// rename keeps the roles and changes only `name`.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type Fetch, type ViewAnswer, type ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import { createSessionStore, endingFetch, endsSession } from './session';
import { meOf } from '../../test-support/me';
import { QUEUE_CARD_PATH, QUEUE_CARD_VIEW } from '../testing/page-wide';
import { warmModules } from '../testing/warm-modules';

// ------------------------------------------------------------------ fixtures

const owner: Me = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Somchai', branchName: 'MAIN', branches: [] });
const rates: Me = meOf({ id: 'e-7', role: 'RATES', branch: 'XC-B01', name: 'Nok', branchName: 'XC-B01', branches: [] });
const ratesOther: Me = meOf({ id: 'e-8', role: 'RATES', branch: 'XC-B01', name: 'Ploy', branchName: 'XC-B01', branches: [] });
const frontSameId: Me = meOf({ id: 'e-7', role: 'FRONT', branch: 'XC-B01', name: 'Nok', branchName: 'XC-B01', branches: [] });
const ratesRenamed: Me = { ...rates, name: 'Nok K.' };

const refusal = (status: number, code: string, message = 'refused') =>
  new ApiError({ code, message, status });

function fakeApi(script: Array<ViewAnswer<Me> | Error>) {
  const seen: ViewParams[] = [];
  const posted: Array<{ path: string; body: unknown }> = [];
  let i = 0;
  const api = {
    post: async <B, R>(path: string, body: B): Promise<R> => {
      posted.push({ path, body });
      return { saved: true } as unknown as R;
    },
    view: async <U>(_name: string, params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      seen.push({ ...params });
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      if (next instanceof Error) throw next;
      return next as unknown as ViewAnswer<U>;
    },
  };
  return { api, seen, posted };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const errorBody = (code: string, message = 'refused') => ({ error: { code, message } });

interface Call {
  url: string;
  method: string;
  body: unknown;
}

/** A global `fetch` that answers by path and records every call. */
function router(answer: (url: string, method: string, n: number) => Response) {
  const calls: Call[] = [];
  const counts = new Map<string, number>();
  const fn = vi.fn(async (input: string, init?: RequestInit) => {
    const url = String(input);
    const method = (init?.method ?? 'GET').toUpperCase();
    const path = url.split('?')[0]!;
    const n = counts.get(path) ?? 0;
    counts.set(path, n + 1);
    calls.push({ url, method, body: init?.body === undefined ? undefined : JSON.parse(String(init.body)) });
    return answer(url, method, n);
  });
  const meReads = () => calls.filter((c) => c.url.startsWith('/api/v3/views/me')).length;
  const posts = () => calls.filter((c) => c.method === 'POST');
  return { fn, calls, meReads, posts };
}

const meAnswer = (version: number, data: Me) => json(200, { version, changed: true, data });

/** Fresh module instances, so `sharedApi()` / `sharedSession()` are new in every test. */
async function fresh() {
  vi.resetModules();
  const session = await import('./session');
  const client = await import('../api/client');
  return { ...session, ...client };
}

beforeAll(
  () =>
    warmModules(
      () => import('./session'),
      () => import('../api/client'),
    ),
  60_000,
);

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// ------------------------------------------------------------------ SE4

describe('endsSession (SE4)', () => {
  it('is true for any 401 and for 400 kioskStaff, and false for everything else', () => {
    expect(endsSession(401, 'signIn')).toBe(true);
    expect(endsSession(401, 'employeeAuthRequired')).toBe(true);
    expect(endsSession(401, undefined)).toBe(true);
    expect(endsSession(400, 'kioskStaff')).toBe(true);

    expect(endsSession(400, 'invalidRequest')).toBe(false);
    expect(endsSession(400, 'kioskOwnerOrRates')).toBe(false);
    expect(endsSession(400, undefined)).toBe(false);
    expect(endsSession(403, 'kioskStaff')).toBe(false);
    expect(endsSession(403, 'ownerOnly')).toBe(false);
    expect(endsSession(404, 'notFound')).toBe(false);
    expect(endsSession(500, 'serverError')).toBe(false);
    expect(endsSession(200, undefined)).toBe(false);
  });
});

// ------------------------------------------------------------------ SE1

describe('createSessionStore: ended and end (SE1)', () => {
  it('ended is undefined while the session runs, from the first answer on', async () => {
    const { api, seen } = fakeApi([
      { version: 4, changed: true, data: rates },
      { version: 4, changed: false },
    ]);
    const store = createSessionStore(api);
    expect(store.ended()).toBeUndefined();
    await vi.advanceTimersByTimeAsync(0);
    expect(store.me()).toBe(rates);
    expect(store.ended()).toBeUndefined();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(seen).toHaveLength(3);
    expect(store.ended()).toBeUndefined();
    store.dispose();
  });

  it('end keeps me, stops the me poll and makes no request; the first reason wins', async () => {
    const { api, seen, posted } = fakeApi([{ version: 4, changed: true, data: owner }]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);

    store.end('expired');
    expect(store.ended()).toBe('expired');
    expect(store.me()).toBe(owner);

    store.end('switched');
    expect(store.ended()).toBe('expired');

    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(1);
    expect(posted).toEqual([]);
    expect(store.me()).toBe(owner);
    store.dispose();
  });

  it('end called before the first answer stops the poll before it asks', async () => {
    const { api, seen } = fakeApi([{ version: 4, changed: true, data: owner }]);
    const store = createSessionStore(api);
    store.end('switched');
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(0);
    expect(store.ended()).toBe('switched');
    expect(store.me()).toBeUndefined();
    store.dispose();
  });

  it('signOut after end still posts logout to /api/auth/logout and clears me', async () => {
    const { api, posted } = fakeApi([{ version: 4, changed: true, data: owner }]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(0);
    store.end('expired');

    await store.signOut();

    expect(posted).toEqual([{ path: '/api/auth/logout', body: {} }]);
    expect(store.me()).toBeUndefined();
  });
});

// ------------------------------------------------------------------ SE2, SE3

describe('createSessionStore: the me poll ends the session (SE2, SE3)', () => {
  it('a changed answer with another id ends it as switched, keeps the shown account and stops the poll', async () => {
    const { api, seen } = fakeApi([
      { version: 4, changed: true, data: rates },
      { version: 9, changed: true, data: ratesOther },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(seen).toHaveLength(2);
    expect(store.ended()).toBe('switched');
    expect(store.me()).toBe(rates);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(2);
    store.dispose();
  });

  it('a changed answer with the same id and other roles ends it as switched', async () => {
    const { api } = fakeApi([
      { version: 4, changed: true, data: rates },
      { version: 9, changed: true, data: frontSameId },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBe('switched');
    expect(store.me()).toBe(rates);
    store.dispose();
  });

  it('a changed answer with the same id and roles (a rename) replaces me and does not end it', async () => {
    const { api, seen } = fakeApi([
      { version: 4, changed: true, data: rates },
      { version: 5, changed: true, data: ratesRenamed },
      { version: 5, changed: false },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.me()).toBe(ratesRenamed);
    expect(store.ended()).toBeUndefined();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(seen).toHaveLength(3);
    expect(seen[2]!.since).toBe(5);
    expect(store.ended()).toBeUndefined();
    store.dispose();
  });

  it('a 401 after me was seen ends it as expired, keeps me and stops the poll', async () => {
    const { api, seen, posted } = fakeApi([
      { version: 4, changed: true, data: owner },
      refusal(401, 'signIn', 'กรุณาเข้าสู่ระบบ'),
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBe('expired');
    expect(store.me()).toBe(owner);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen).toHaveLength(2);
    expect(posted).toEqual([]);
    store.dispose();
  });

  it('a 400 kioskStaff after me was seen ends it as expired', async () => {
    const { api } = fakeApi([
      { version: 4, changed: true, data: rates },
      refusal(400, 'kioskStaff'),
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.ended()).toBe('expired');
    expect(store.me()).toBe(rates);
    store.dispose();
  });

  it('a 400 with another code, a 403 or a 500 does not end it, and the poll goes on', async () => {
    const { api, seen } = fakeApi([
      { version: 4, changed: true, data: owner },
      refusal(400, 'invalidRequest'),
      refusal(403, 'ownerOnly'),
      refusal(500, 'serverError'),
      { version: 4, changed: false },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen.length).toBeGreaterThanOrEqual(5);
    expect(store.ended()).toBeUndefined();
    expect(store.me()).toBe(owner);
    store.dispose();
  });

  it('a 401 before any me was seen does not end it, and the poll keeps retrying (/login)', async () => {
    const { api, seen } = fakeApi([refusal(401, 'signIn', 'กรุณาเข้าสู่ระบบ')]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(seen.length).toBeGreaterThan(1);
    expect(store.ended()).toBeUndefined();
    expect(store.me()).toBeUndefined();
    store.dispose();
  });

  it('after a 401 before sign-in, the first answer is taken and is not a switch', async () => {
    const { api } = fakeApi([
      refusal(401, 'signIn', 'กรุณาเข้าสู่ระบบ'),
      { version: 4, changed: true, data: rates },
    ]);
    const store = createSessionStore(api);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(store.me()).toBe(rates);
    expect(store.ended()).toBeUndefined();
    store.dispose();
  });
});

// ------------------------------------------------------------------ SE5

describe('endingFetch (SE5)', () => {
  it('passes the request through unchanged and returns the same, unread response', async () => {
    const res = json(200, { version: 4, changed: false });
    const inner = vi.fn<Fetch>(async () => res);
    const onEnd = vi.fn();
    const wrapped = endingFetch(inner, onEnd);
    const init: RequestInit = { method: 'GET', credentials: 'same-origin' };

    const got = await wrapped('/api/v3/views/me?since=4', init);

    expect(inner).toHaveBeenCalledTimes(1);
    expect(inner.mock.calls[0]![0]).toBe('/api/v3/views/me?since=4');
    expect(inner.mock.calls[0]![1]).toBe(init);
    expect(got).toBe(res);
    expect(await got.json()).toEqual({ version: 4, changed: false });
    expect(onEnd).not.toHaveBeenCalled();
  });

  it('calls onEnd for a view read answering 401, before the response is handed back, body still readable', async () => {
    const res = json(401, errorBody('signIn', 'กรุณาเข้าสู่ระบบ'));
    const onEnd = vi.fn();
    const wrapped = endingFetch(async () => res, onEnd);

    const got = await wrapped('/api/v3/views/rates?branch=XC-B01', { method: 'GET', credentials: 'same-origin' });

    expect(onEnd).toHaveBeenCalledTimes(1);
    expect(got).toBe(res);
    expect(await got.json()).toEqual(errorBody('signIn', 'กรุณาเข้าสู่ระบบ'));
  });

  it('calls onEnd for a view read answering 401 with no body', async () => {
    const onEnd = vi.fn();
    const wrapped = endingFetch(async () => new Response('', { status: 401 }), onEnd);
    await wrapped('/api/v3/views/me', { method: 'GET' });
    expect(onEnd).toHaveBeenCalledTimes(1);
  });

  it('calls onEnd for a view read answering 400 kioskStaff', async () => {
    const onEnd = vi.fn();
    const wrapped = endingFetch(async () => json(400, errorBody('kioskStaff')), onEnd);
    const got = await wrapped(QUEUE_CARD_PATH, { method: 'GET' });
    expect(onEnd).toHaveBeenCalledTimes(1);
    expect(await got.json()).toEqual(errorBody('kioskStaff'));
  });

  it('does not call onEnd for a view read answering 400 with another code, 400 with no JSON, 403 or 500', async () => {
    const onEnd = vi.fn();
    for (const res of [
      json(400, errorBody('invalidRequest')),
      new Response('not json', { status: 400 }),
      json(403, errorBody('kioskStaff')),
      json(403, errorBody('ownerOnly')),
      json(500, errorBody('serverError')),
      json(200, { version: 1, changed: false }),
    ]) {
      const wrapped = endingFetch(async () => res, onEnd);
      const got = await wrapped('/api/v3/views/me', { method: 'GET' });
      expect(got).toBe(res);
    }
    expect(onEnd).not.toHaveBeenCalled();
  });

  it('does not call onEnd outside GET /api/v3/views/ (POST /api/kiosk kioskStaff, actions, other paths)', async () => {
    const onEnd = vi.fn();
    const cases: Array<[string, RequestInit, Response]> = [
      ['/api/kiosk', { method: 'POST', body: '{}' }, json(400, errorBody('kioskStaff'))],
      ['/api/kiosk', { method: 'POST', body: '{}' }, json(401, errorBody('signIn'))],
      ['/api/v3/desk/save', { method: 'POST', body: '{}' }, json(401, errorBody('employeeAuthRequired'))],
      ['/api/v3/views/me', { method: 'POST', body: '{}' }, json(401, errorBody('signIn'))],
      ['/api/auth', { method: 'POST', body: '{}' }, json(401, errorBody('signIn'))],
      ['/health/ready', { method: 'GET' }, json(401, errorBody('signIn'))],
      ['/api/v3/viewsx/me', { method: 'GET' }, json(401, errorBody('signIn'))],
    ];
    for (const [url, init, res] of cases) {
      const wrapped = endingFetch(async () => res, onEnd);
      expect(await wrapped(url, init)).toBe(res);
    }
    expect(onEnd).not.toHaveBeenCalled();
  });
});

// ------------------------------------------------------------------ SE6, SE7

describe('sharedApi and sharedSession (SE6, SE7)', () => {
  it('importing session.ts fetches nothing; sharedApi() is one ApiClient that starts no timer and no fetch', async () => {
    const net = router(() => json(200, {}));
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    expect(net.fn).not.toHaveBeenCalled();

    const a = m.sharedApi();
    const b = m.sharedApi();
    expect(a).toBe(b);
    expect(a).toBeInstanceOf(m.ApiClient);
    expect(vi.getTimerCount()).toBe(0);
    expect(net.fn).not.toHaveBeenCalled();
  });

  it('a read through sharedApi() made after a second fetch stub reaches the second stub (done criterion 2)', async () => {
    const first = vi.fn<Fetch>(async () => json(200, { version: 1, changed: false }));
    const second = vi.fn<Fetch>(async () => json(200, { version: 2, changed: false }));
    vi.stubGlobal('fetch', first);
    const m = await fresh();
    const api = m.sharedApi();
    expect(await api.view('rates')).toEqual({ version: 1, changed: false });
    expect(first).toHaveBeenCalledTimes(1);

    vi.stubGlobal('fetch', second);
    expect(await m.sharedApi().view('rates')).toEqual({ version: 2, changed: false });
    expect(await api.view('rates')).toEqual({ version: 2, changed: false });
    expect(second).toHaveBeenCalledTimes(2);
    expect(first).toHaveBeenCalledTimes(1);
  });

  it('the shared session’s me poll uses the fetch stubbed after it was created', async () => {
    const first = router(() => meAnswer(3, owner));
    vi.stubGlobal('fetch', first.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    expect(first.meReads()).toBe(1);

    const second = router(() => json(200, { version: 3, changed: false }));
    vi.stubGlobal('fetch', second.fn);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(second.meReads()).toBe(1);
    expect(first.meReads()).toBe(1);
    s.dispose();
  });

  it('a view read through sharedApi() answering 401 ends the shared session as expired; nothing is posted', async () => {
    const net = router((url) =>
      url.startsWith('/api/v3/views/me') ? meAnswer(3, owner) : json(401, errorBody('signIn', 'กรุณาเข้าสู่ระบบ')),
    );
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    expect(s.me()).toEqual(owner);
    expect(s.ended()).toBeUndefined();

    await expect(m.sharedApi().view('rates')).rejects.toMatchObject({ status: 401 });

    expect(s.ended()).toBe('expired');
    expect(s.me()).toEqual(owner);
    const reads = net.meReads();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(net.meReads()).toBe(reads);
    expect(net.posts()).toEqual([]);
  });

  it('a view read through sharedApi() answering 400 kioskStaff ends the shared session as expired', async () => {
    const net = router((url) =>
      url.startsWith('/api/v3/views/me') ? meAnswer(3, rates) : json(400, errorBody('kioskStaff')),
    );
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    await expect(m.sharedApi().view(QUEUE_CARD_VIEW)).rejects.toMatchObject({ status: 400, code: 'kioskStaff' });
    expect(s.ended()).toBe('expired');
    expect(s.me()).toEqual(rates);
  });

  it('a view read through sharedApi() answering 400 invalidRequest does not end it', async () => {
    const net = router((url) =>
      url.startsWith('/api/v3/views/me') ? meAnswer(3, owner) : json(400, errorBody('invalidRequest')),
    );
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    await expect(m.sharedApi().view('rates')).rejects.toMatchObject({ status: 400, code: 'invalidRequest' });
    expect(s.ended()).toBeUndefined();
    s.dispose();
  });

  it('a 400 kioskStaff from POST /api/kiosk through sharedApi() does not end it (only view reads count)', async () => {
    const net = router((url, method) =>
      url.startsWith('/api/v3/views/me')
        ? meAnswer(3, rates)
        : method === 'POST'
          ? json(400, errorBody('kioskStaff'))
          : json(200, {}),
    );
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    await expect(
      m.sharedApi().post('/api/kiosk', { action: 'claim', payload: {} }),
    ).rejects.toMatchObject({ status: 400, code: 'kioskStaff' });
    expect(s.ended()).toBeUndefined();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(net.meReads()).toBe(2);
    s.dispose();
  });

  it('a 401 through sharedApi() before me was seen does not end it, and the me poll keeps retrying', async () => {
    const net = router(() => json(401, errorBody('signIn', 'กรุณาเข้าสู่ระบบ')));
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    expect(s.me()).toBeUndefined();
    await expect(m.sharedApi().view('rates')).rejects.toMatchObject({ status: 401 });
    expect(s.ended()).toBeUndefined();
    const reads = net.meReads();
    await vi.advanceTimersByTimeAsync(600_000);
    expect(net.meReads()).toBeGreaterThan(reads);
    expect(s.ended()).toBeUndefined();
    s.dispose();
  });

  it('a 401 through sharedApi() with no shared session creates none and starts no poll', async () => {
    const net = router(() => json(401, errorBody('signIn', 'กรุณาเข้าสู่ระบบ')));
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    await expect(m.sharedApi().view('rates')).rejects.toMatchObject({ status: 401 });
    expect(net.fn).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(net.fn).toHaveBeenCalledTimes(1);

    const s = m.sharedSession();
    expect(s.ended()).toBeUndefined();
    s.dispose();
  });

  it('a 401 from a client that is not sharedApi() does not end it; the next me poll does (SE7)', async () => {
    const net = router((url, _method, n) => {
      if (url.startsWith('/api/v3/views/me')) {
        return n === 0 ? meAnswer(3, owner) : json(401, errorBody('signIn', 'กรุณาเข้าสู่ระบบ'));
      }
      return json(401, errorBody('signIn', 'กรุณาเข้าสู่ระบบ'));
    });
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    await expect(new m.ApiClient().view('rates')).rejects.toMatchObject({ status: 401 });
    expect(s.ended()).toBeUndefined();

    await vi.advanceTimersByTimeAsync(60_000);
    expect(net.meReads()).toBe(2);
    expect(s.ended()).toBe('expired');
    expect(s.me()).toEqual(owner);
  });

  it('a me answer for another account ends the shared session as switched and keeps the shown account', async () => {
    const net = router((url, _method, n) =>
      url.startsWith('/api/v3/views/me') ? (n === 0 ? meAnswer(3, owner) : meAnswer(8, rates)) : json(200, {}),
    );
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(net.meReads()).toBe(2);
    expect(s.ended()).toBe('switched');
    expect(s.me()).toEqual(owner);
    await vi.advanceTimersByTimeAsync(600_000);
    expect(net.meReads()).toBe(2);
    expect(net.posts()).toEqual([]);
  });

  it('signOut on an ended shared session posts logout and clears me', async () => {
    const net = router((url) => (url.startsWith('/api/v3/views/me') ? meAnswer(3, owner) : json(200, { saved: true })));
    vi.stubGlobal('fetch', net.fn);
    const m = await fresh();
    const s = m.sharedSession();
    await vi.advanceTimersByTimeAsync(0);
    s.end('expired');
    await s.signOut();
    expect(net.posts()).toEqual([{ url: '/api/auth/logout', method: 'POST', body: {} }]);
    expect(s.me()).toBeUndefined();
  });
});
