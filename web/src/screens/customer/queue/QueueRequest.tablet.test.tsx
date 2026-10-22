// BT-F2 (kiosk-shared-phone §7.3 T-D7, T-D8, T-D11; §7.8 rows T12, T13, T15, T17, T18, T21): the /app/queue page on
// a paired branch tablet and on a phone. The page is the same on both (R-T5); pairing switches only where it goes when
// the request ends, and that the first page starts at sign-in. Written before the implementation by the XC FE
// controller claude/e8020026. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// The fake API answers `GET /api/branch-tablet/me` from `tabletMe`; everything else as QueueRequest.test.tsx.
// - Paired, the page ends a request by `location.assign('/app/queue?branch=<tablet branch>')`, once, with no press
//   and no `/api/auth/logout`: when the status leaves QUOTED (the scan), on a 401 poll after submit (the server
//   deleted the tablet's session, T-D7), on ยกเลิกคิว, on expiry, and on เสร็จ.
// - Paired, the first page shows the sign-in link and no form, even when `my_profile` answers 200 (T-D8). The test
//   follows that link's `next` back to the page (as the sign-in screen does after OTP), and there the form shows.
// - Not paired (no cookie, a phone, the BE not deployed, `?shared=1`): today's flow, no loop (T12).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueRequestScreen } from './QueueRequest';

interface Sent {
  method: string;
  path: string;
  search: string;
  body: any;
}

const RATES = {
  branchId: 'b-id-7',
  branchName: 'XC-B',
  open: true,
  rates: [{ code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 }],
  paymentMethods: [],
};

const PAIRED_MAIN = { paired: true, tabletId: 'bt-1', branchCode: 'MAIN', name: 'Front desk tablet' };

let sent: Sent[];
let tabletMe: () => Response;
let profileStatus: number;
let views: Array<() => Response>;
let assign: ReturnType<typeof vi.fn>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
  const u = new URL(input, 'http://localhost');
  const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
  sent.push({ method: (init?.method ?? 'GET').toUpperCase(), path: u.pathname, search: u.search, body });
  if (u.pathname === '/api/branch-tablet/me') return tabletMe();
  if (u.pathname === '/api/auth/logout') return json(200, {});
  if (u.pathname === '/api/public/host') return json(200, { org: 'ORG_1' });
  if (u.pathname === '/api/public/orgs/ORG_1/branches/MAIN/queue-rates') return json(200, RATES);
  if (u.pathname === '/api/public/orgs/ORG_1/branches/OTHER/queue-rates') return json(200, { ...RATES, branchId: 'b-id-9' });
  if (u.pathname === '/api/v3/views/my_profile') {
    return profileStatus === 200
      ? json(200, { version: 1, changed: true, data: { customerId: 'c-1', displayName: 'Somchai', emailCopy: { available: false, to: null } } })
      : json(profileStatus, { error: { code: 'signIn', message: 'sign in' } });
  }
  if (u.pathname === '/api/app/web-requests' && init?.method === 'POST') {
    return json(200, { requestId: 'k-1', requestQrUrl: '/q#tok-1', expiresAt: '2026-10-21T21:00:00Z' });
  }
  if (/^\/api\/app\/web-requests\/[^/]+\/cancel$/.test(u.pathname)) return json(200, { requestId: 'k-1', status: 'CANCELLED' });
  if (u.pathname === '/api/v3/views/my_web_request') {
    const next = views.shift();
    return next ? next() : json(200, { version: 5, changed: false });
  }
  return json(404, { error: { code: 'notFound', message: u.pathname } });
});

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function polls(n: number): Promise<void> {
  for (let i = 0; i < n; i += 1) {
    await vi.advanceTimersByTimeAsync(3_000);
    await flush();
  }
}

function q<T extends Element>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function view(status: string, displayNumber: string | null, version: number) {
  return () =>
    json(200, {
      version,
      changed: true,
      data: {
        requestId: 'k-1', status, expiresAt: '2026-10-21T21:00:00Z', branch: { code: 'MAIN', name: 'XC-B' },
        type: 'BUY', currency: 'USD', amount: '100', displayNumber, ticketsAhead: displayNumber ? 2 : null, counter: null,
      },
    });
}

const signedOutPoll = () => json(401, { error: { code: 'signIn', message: 'sign in' } });
const expiredPoll = () => json(404, { error: { code: 'notFound', message: 'notFound' } });

async function mount(search: string): Promise<void> {
  vi.stubGlobal('location', { ...window.location, assign, reload: vi.fn(), search, hash: '', pathname: '/app/queue' });
  dispose = render(() => <QueueRequestScreen fetch={fetchFn} search={search} intervalMs={3_000} />, host);
  await flush();
}

function unmount(): void {
  dispose?.();
  dispose = undefined;
  host.innerHTML = '';
}

async function submit(): Promise<void> {
  const type = q<HTMLSelectElement>('select[name="type"]')!;
  type.value = 'BUY';
  type.dispatchEvent(new Event('change', { bubbles: true }));
  const cur = q<HTMLSelectElement>('select[name="currency"]')!;
  cur.value = 'USD';
  cur.dispatchEvent(new Event('change', { bubbles: true }));
  const amt = q<HTMLInputElement>('input[name="amount"]')!;
  amt.value = '100';
  amt.dispatchEvent(new Event('input', { bubbles: true }));
  q<HTMLButtonElement>('button[data-action="submit-request"]')!.click();
  await flush();
}

/** The sign-in link's `next`, as the search of the /app/queue page it returns to. */
function signInReturn(): string {
  const link = q<HTMLAnchorElement>('a[href^="/app/sign-in"]');
  expect(link, 'the sign-in link').not.toBeNull();
  const next = new URL(link!.getAttribute('href')!, 'http://localhost').searchParams.get('next');
  expect(next, 'the link carries next').toBeTruthy();
  const back = new URL(next!, 'http://localhost');
  expect(back.pathname).toBe('/app/queue');
  return back.search;
}

/** A paired tablet: open the first page at `first`, follow the sign-in link back, as the person who just signed in. */
async function pairedAndSignedIn(first = '?branch=MAIN'): Promise<void> {
  await mount(first);
  const back = signInReturn();
  unmount();
  sent = [];
  await mount(back);
}

const creates = () => sent.filter((s) => s.path === '/api/app/web-requests' && s.method === 'POST');
const logouts = () => sent.filter((s) => s.path === '/api/auth/logout');
const ratesReads = (code: string) => sent.filter((s) => s.path === `/api/public/orgs/ORG_1/branches/${code}/queue-rates`);

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  sent = [];
  tabletMe = () => json(200, { paired: false });
  profileStatus = 200;
  views = [view('QUOTED', null, 5)];
  assign = vi.fn();
  fetchFn.mockClear();
  try {
    window.sessionStorage.clear();
  } catch {
    // storage blocked here: the rows below do not need it
  }
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('BT-F2: /app/queue not paired (T12)', () => {
  it('?shared=1 and the old storage mark do nothing: the form, and after the scan the page stays on the number', async () => {
    try {
      window.sessionStorage.setItem('xc.sharedTablet', '1');
    } catch {
      // storage blocked: the URL flag alone is still checked
    }
    views = [view('QUOTED', null, 5), view('WAITING', 'A012', 6)];
    await mount('?branch=MAIN&shared=1');
    expect(sent.filter((s) => s.path === '/api/branch-tablet/me'), 'me is read once').toHaveLength(1);
    expect(q('button[data-action="submit-request"]'), 'the form, no sign-in first').not.toBeNull();
    await submit();
    await polls(3);
    expect(assign).not.toHaveBeenCalled();
    expect(logouts()).toHaveLength(0);
    expect(q<HTMLElement>('[data-display-number]')?.textContent).toContain('A012');
  });

  it('fail-open: me answering 500 is the normal flow', async () => {
    tabletMe = () => json(500, { error: { code: 'serverError', message: 'down' } });
    await mount('?branch=MAIN');
    expect(sent.filter((s) => s.path === '/api/branch-tablet/me'), 'me is read once').toHaveLength(1);
    expect(q('button[data-action="submit-request"]')).not.toBeNull();
    expect(ratesReads('MAIN')).toHaveLength(1);
  });
});

describe('BT-F2: /app/queue on a paired tablet', () => {
  beforeEach(() => {
    tabletMe = () => json(200, PAIRED_MAIN);
  });

  it('T17 / T-D8: the first page starts at sign-in even with a live session cookie; nothing of the last customer', async () => {
    profileStatus = 200;
    await mount('?branch=MAIN');
    expect(q('a[href^="/app/sign-in"]'), 'the sign-in link').not.toBeNull();
    expect(q('button[data-action="submit-request"]'), 'no form').toBeNull();
    expect(q('[data-request-qr]')).toBeNull();
    expect(host.textContent).not.toContain('Somchai');
    expect(creates()).toHaveLength(0);
    expect(new URLSearchParams(signInReturn()).get('branch')).toBe('MAIN');
  });

  it('T-D8: back from sign-in, the form shows', async () => {
    await pairedAndSignedIn();
    expect(q('button[data-action="submit-request"]')).not.toBeNull();
  });

  it("T13 / T-D11: opened at ?branch=OTHER, the page uses the tablet's branch", async () => {
    await mount('?branch=OTHER');
    expect(new URLSearchParams(signInReturn()).get('branch')).toBe('MAIN');
    await pairedAndSignedIn('?branch=OTHER');
    expect(ratesReads('MAIN')).toHaveLength(1);
    expect(ratesReads('OTHER')).toHaveLength(0);
    await submit();
    expect(creates()[0]!.body.branch).toBe('b-id-7');
  });

  it('T15: the scan (QUOTED → WAITING) goes straight to the first page once, with no press and no sign-out', async () => {
    views = [view('QUOTED', null, 5), view('WAITING', 'A012', 6)];
    await pairedAndSignedIn();
    await submit();
    expect(assign, 'nothing before the scan').not.toHaveBeenCalled();
    await polls(3);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
    expect(logouts()).toHaveLength(0);
  });

  it('T15 / T-D7: a poll answering 401 after submit (the server ended the session) goes to the first page once', async () => {
    views = [view('QUOTED', null, 5), signedOutPoll, signedOutPoll];
    await pairedAndSignedIn();
    await submit();
    await polls(4);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
    expect(logouts()).toHaveLength(0);
  });

  it('T18: ยกเลิกคิว goes to the first page once', async () => {
    await pairedAndSignedIn();
    await submit();
    q<HTMLButtonElement>('button[data-action="cancel-web-request"]')!.click();
    await flush();
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
    expect(logouts()).toHaveLength(0);
  });

  it('expiry (a 404 after expiresAt) goes to the first page once', async () => {
    views = [view('QUOTED', null, 5), expiredPoll];
    await pairedAndSignedIn();
    await submit();
    await polls(3);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
  });

  it('เสร็จ on the tablet goes to the first page, not /app/bookings', async () => {
    await pairedAndSignedIn();
    await submit();
    q<HTMLButtonElement>('button[data-action="done"]')!.click();
    await flush();
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN');
    expect(logouts()).toHaveLength(0);
  });

  it('T21: no input for 10 minutes, on the form or on a QUOTED request, changes nothing', async () => {
    await pairedAndSignedIn();
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    expect(q('button[data-action="submit-request"]')).not.toBeNull();
    await submit();
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    await flush();
    expect(assign).not.toHaveBeenCalled();
    expect(logouts()).toHaveLength(0);
  });
});
