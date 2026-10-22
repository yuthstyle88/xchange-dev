// BT-F2 (kiosk-shared-phone §7.3 T-D7, T-D8, T-D10, T-D11; §7.8 rows T12, T13, T15, T17, T18, T21): useQueueTablet,
// the /app/queue page's one call into BT-F1's shared/session/tablet.ts. Written before the implementation by the XC FE
// controller claude/e8020026. Do not edit, delete or weaken these tests; add tests in a file of your own.
// Only useQueueTablet is under test (the header of ./queue-tablet.ts is the contract); the network is an injected
// fetch that answers `/api/branch-tablet/me` and `/api/branch-tablet/pair`, and `location` is stubbed.
// The whole page is pinned in QueueRequest.tablet.test.tsx.
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { useQueueTablet, type QueueTablet } from './queue-tablet';

interface Sent {
  method: string;
  path: string;
  body: unknown;
}

const PAIRED_MAIN = { paired: true, tabletId: 'bt-1', branchCode: 'MAIN', name: 'Front desk tablet' };

let sent: Sent[];
let meAnswer: () => Response;
let pairAnswer: () => Response;
let assign: ReturnType<typeof vi.fn>;
let reload: ReturnType<typeof vi.fn>;
let disposeRoot: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
  const u = new URL(input, 'http://localhost');
  sent.push({
    method: (init?.method ?? 'GET').toUpperCase(),
    path: u.pathname,
    body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
  });
  if (u.pathname === '/api/branch-tablet/me') return meAnswer();
  if (u.pathname === '/api/branch-tablet/pair') return pairAnswer();
  if (u.pathname === '/api/auth/logout') return json(200, {});
  return json(404, { error: { code: 'notFound', message: u.pathname } });
});

const meReads = () => sent.filter((s) => s.path === '/api/branch-tablet/me');
const pairs = () => sent.filter((s) => s.path === '/api/branch-tablet/pair');
const logouts = () => sent.filter((s) => s.path === '/api/auth/logout');

function openAt(search: string, hash = ''): void {
  vi.stubGlobal('location', { ...window.location, search, hash, pathname: '/app/queue', assign, reload });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function mount(search: string, hash = ''): Promise<QueueTablet> {
  openAt(search, hash);
  const tablet = createRoot((dispose) => {
    disposeRoot = dispose;
    return useQueueTablet(search, fetchFn);
  });
  await flush();
  return tablet;
}

function unmount(): void {
  disposeRoot?.();
  disposeRoot = undefined;
}

/** The search part of the sign-in link's `next` (must be the /app/queue page). */
function searchOf(next: string): string {
  const u = new URL(next, 'http://localhost');
  expect(u.pathname).toBe('/app/queue');
  return u.search;
}

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  sent = [];
  fetchFn.mockClear();
  meAnswer = () => json(200, { paired: false });
  pairAnswer = () => json(200, { tabletId: 'bt-1', branchCode: 'MAIN', name: 'Front desk tablet' });
  assign = vi.fn();
  reload = vi.fn();
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked here: the rows below do not need it
  }
});

afterEach(() => {
  unmount();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('BT-F2: useQueueTablet on a phone (not paired)', () => {
  it('T12: me {paired: false} even with ?shared=1 and the old storage mark: the page branch, no sign-in-first', async () => {
    try {
      window.sessionStorage.setItem('xc.sharedTablet', '1');
    } catch {
      // storage blocked: the URL flag alone is still checked
    }
    const tablet = await mount('?branch=MAIN&shared=1');
    expect(tablet.mode()).toEqual({ paired: false });
    expect(tablet.branch()).toBe('MAIN');
    expect(tablet.signInFirst()).toBe(false);
    expect(tablet.signInNext()).toBe('/app/queue?branch=MAIN&shared=1');
    expect(meReads()).toHaveLength(1);
  });

  it('T12: when the request ends the page stays (no assign, no sign-out)', async () => {
    const tablet = await mount('?branch=MAIN');
    tablet.requestEnded();
    await flush();
    expect(assign).not.toHaveBeenCalled();
    expect(logouts()).toHaveLength(0);
  });

  it('U8a: done() on a phone leaves for /app/bookings once, and signs nothing out', async () => {
    const tablet = await mount('?branch=MAIN');
    tablet.done();
    await flush();
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/bookings');
    expect(logouts()).toHaveLength(0);
  });

  it('fail-open: me answering 404 (the BE not deployed) is a phone', async () => {
    meAnswer = () => json(404, { error: { code: 'notFound', message: 'nope' } });
    const tablet = await mount('?branch=MAIN');
    expect(tablet.mode()).toEqual({ paired: false });
    expect(tablet.branch()).toBe('MAIN');
    expect(tablet.signInFirst()).toBe(false);
  });
});

describe('BT-F2: useQueueTablet on a paired tablet', () => {
  beforeEach(() => {
    meAnswer = () => json(200, PAIRED_MAIN);
  });

  it('mode() is null until me answers, then the paired mode', async () => {
    openAt('?branch=MAIN');
    const tablet = createRoot((dispose) => {
      disposeRoot = dispose;
      return useQueueTablet('?branch=MAIN', fetchFn);
    });
    expect(tablet.mode()).toBeNull();
    await flush();
    expect(tablet.mode()).toEqual(PAIRED_MAIN);
  });

  it("T13 / T-D11: the tablet's branch replaces ?branch=", async () => {
    const tablet = await mount('?branch=OTHER');
    expect(tablet.branch()).toBe('MAIN');
  });

  it('T17 / T-D8: the first page starts at sign-in; the link returns to the tablet branch, where sign-in is not first again', async () => {
    const first = await mount('?branch=OTHER');
    expect(first.signInFirst()).toBe(true);
    const back = searchOf(first.signInNext());
    expect(new URLSearchParams(back).get('branch')).toBe('MAIN');
    unmount();
    const returned = await mount(back);
    expect(returned.signInFirst()).toBe(false);
    expect(returned.branch()).toBe('MAIN');
  });

  it('T15 / T18: requestEnded() goes straight to the first page, once, with no sign-out', async () => {
    const tablet = await mount('?branch=OTHER');
    tablet.requestEnded();
    tablet.requestEnded();
    await flush();
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
    expect(logouts()).toHaveLength(0);
    expect(reload).not.toHaveBeenCalled();
  });

  it('done() (เสร็จ) on the tablet goes to the first page too, not /app/bookings', async () => {
    const tablet = await mount('?branch=MAIN');
    tablet.done();
    await flush();
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
    expect(logouts()).toHaveLength(0);
  });

  it('T21: no input for 10 minutes changes nothing; me is read once, never polled', async () => {
    await mount('?branch=MAIN');
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    document.dispatchEvent(new Event('pointerdown'));
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    await vi.advanceTimersByTimeAsync(5 * 60_000);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    expect(reload).not.toHaveBeenCalled();
    expect(logouts()).toHaveLength(0);
    expect(meReads()).toHaveLength(1);
  });
});

describe('BT-F2: pairing on the page (T-D10)', () => {
  it('#pair=<CODE>: posts the code once, then reloads the page once', async () => {
    meAnswer = () => json(200, { paired: false });
    await mount('?branch=MAIN', '#pair=ABCDE-12345');
    expect(pairs()).toHaveLength(1);
    expect(pairs()[0]!.method).toBe('POST');
    expect(pairs()[0]!.body).toEqual({ code: 'ABCDE-12345' });
    expect(reload).toHaveBeenCalledTimes(1);
    expect(assign).not.toHaveBeenCalled();
  });

  it('an invalid code: no reload; the page reads me and goes on as that answer says', async () => {
    pairAnswer = () => json(400, { error: { code: 'PAIRING_CODE_INVALID', message: 'invalid code' } });
    meAnswer = () => json(200, { paired: false });
    const tablet = await mount('?branch=MAIN', '#pair=ABCDE-12345');
    expect(pairs()).toHaveLength(1);
    expect(reload).not.toHaveBeenCalled();
    expect(meReads()).toHaveLength(1);
    expect(tablet.mode()).toEqual({ paired: false });
  });

  it('no #pair=: no pair request', async () => {
    await mount('?branch=MAIN');
    expect(pairs()).toHaveLength(0);
    expect(reload).not.toHaveBeenCalled();
  });
});
