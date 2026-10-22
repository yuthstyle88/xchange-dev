// QW-2h (kiosk-shared-phone §5r.3 rows U8a, U7b; §5r.4a QW-2h note, §5r.4b): how the /app/queue page ends.
// - U8a: on a personal phone, เสร็จ leaves for the customer's own pages: location.assign('/app/bookings') once,
//   and no /api/auth/logout.
// - U7b: on the branch tablet (?shared=1), as soon as the open request's status leaves QUOTED (the staff scan issued
//   the number), the page signs out (POST /api/auth/logout once) and starts over at /app/queue?branch=MAIN&shared=1,
//   with no press. On a phone the same status change signs nothing out and the page stays, showing the number.
// Written before the implementation by the XC FE controller claude/e8020026 (owner lines on #4562).
// Do not edit, delete or weaken these tests; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueRequestScreen } from './QueueRequest';

interface Sent {
  method: string;
  path: string;
  search: string;
  body: any;
  headers: Headers;
}

const RATES = {
  branchId: 'b-id-7',
  branchName: 'XC-B',
  open: true,
  rates: [{ code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 }],
  paymentMethods: [],
};

let sent: Sent[];
let profile: unknown;
let profileStatus: number;
let createAnswer: () => Response;
let views: Array<() => Response>;
let logoutStatus: number;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
  const u = new URL(input, 'http://localhost');
  const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
  sent.push({ method: (init?.method ?? 'GET').toUpperCase(), path: u.pathname, search: u.search, body, headers: new Headers(init?.headers) });
  if (u.pathname === '/api/auth/logout') return json(logoutStatus, logoutStatus === 200 ? {} : { error: { code: 'serverError', message: 'down' } });
  if (u.pathname === '/api/public/host') return json(200, { org: 'ORG_1' });
  if (u.pathname === '/api/public/orgs/ORG_1/branches/MAIN/queue-rates') return json(200, RATES);
  if (u.pathname === '/api/v3/views/my_profile') return json(profileStatus, profileStatus === 200 ? { version: 1, changed: true, data: profile } : { error: { code: 'signIn', message: 'sign in' } });
  if (u.pathname === '/api/app/web-requests' && init?.method === 'POST') return createAnswer();
  if (/^\/api\/app\/web-requests\/[^/]+\/qr$/.test(u.pathname)) {
    return json(200, { requestId: 'k-old', requestQrUrl: '/q#tok-fresh', expiresAt: '2026-10-21T21:00:00Z' });
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

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  sent = [];
  profileStatus = 200;
  profile = { customerId: 'c-1', displayName: 'Somchai', emailCopy: { available: false, to: null } };
  createAnswer = () => json(200, { requestId: 'k-1', requestQrUrl: '/q#tok-1', expiresAt: '2026-10-21T21:00:00Z' });
  views = [view('QUOTED', null, 5)];
  logoutStatus = 200;
  fetchFn.mockClear();
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

async function mount(search = '?branch=MAIN'): Promise<void> {
  dispose = render(() => <QueueRequestScreen fetch={fetchFn} search={search} intervalMs={3_000} />, host);
  await flush();
}

async function submit(amount = '100'): Promise<void> {
  const type = q<HTMLSelectElement>('select[name="type"]')!;
  type.value = 'BUY';
  type.dispatchEvent(new Event('change', { bubbles: true }));
  const cur = q<HTMLSelectElement>('select[name="currency"]')!;
  cur.value = 'USD';
  cur.dispatchEvent(new Event('change', { bubbles: true }));
  const amt = q<HTMLInputElement>('input[name="amount"]')!;
  amt.value = amount;
  amt.dispatchEvent(new Event('input', { bubbles: true }));
  q<HTMLButtonElement>('button[data-action="submit-request"]')!.click();
  await flush();
}



const logouts = () => sent.filter((s) => s.path === '/api/auth/logout' && s.method === 'POST');

function at(search: string): ReturnType<typeof vi.fn> {
  try {
    window.sessionStorage.clear(); // the tablet mark (shared/session/tablet.ts) must not leak between tests
  } catch {
    // storage blocked: the URL flag still drives the mark
  }
  const assign = vi.fn();
  vi.stubGlobal('location', { ...window.location, assign, search, pathname: '/app/queue' });
  return assign;
}

describe('QW-2h: how /app/queue ends', () => {
  it('U8a: on a personal phone, done leaves for /app/bookings once and signs nothing out', async () => {
    const assign = at('?branch=MAIN');
    await mount('?branch=MAIN');
    await submit();
    q<HTMLButtonElement>('button[data-action="done"]')!.click();
    await flush();
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/bookings');
    expect(logouts()).toHaveLength(0);
  });

  it('U7b: on the branch tablet, the scan (QUOTED → WAITING) signs out once and starts over with no press', async () => {
    const assign = at('?branch=MAIN&shared=1');
    views = [view('QUOTED', null, 5), view('WAITING', 'A012', 6)];
    await mount('?branch=MAIN&shared=1');
    await submit();
    expect(logouts(), 'nothing before the scan').toHaveLength(0);
    for (let i = 0; i < 3; i += 1) {
      await vi.advanceTimersByTimeAsync(3_000);
      await flush();
    }
    expect(logouts()).toHaveLength(1);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith('/app/queue?branch=MAIN&shared=1');
  });

  it('U7b + U7a: on the branch tablet, a failed sign-out after the scan shows the worded error and does not restart', async () => {
    const assign = at('?branch=MAIN&shared=1');
    logoutStatus = 503;
    views = [view('QUOTED', null, 5), view('WAITING', 'A012', 6)];
    await mount('?branch=MAIN&shared=1');
    await submit();
    for (let i = 0; i < 3; i += 1) {
      await vi.advanceTimersByTimeAsync(3_000);
      await flush();
    }
    expect(logouts().length).toBeGreaterThanOrEqual(1);
    expect(assign).not.toHaveBeenCalled();
    expect(q('[role="alert"]'), 'a worded error').not.toBeNull();
  });

  it('U7b on a phone: the scan signs nothing out, and the page stays showing the number', async () => {
    const assign = at('?branch=MAIN');
    views = [view('QUOTED', null, 5), view('WAITING', 'A012', 6)];
    await mount('?branch=MAIN');
    await submit();
    for (let i = 0; i < 3; i += 1) {
      await vi.advanceTimersByTimeAsync(3_000);
      await flush();
    }
    expect(logouts()).toHaveLength(0);
    expect(assign).not.toHaveBeenCalled();
    expect(q<HTMLElement>('[data-display-number]')?.textContent).toContain('A012');
  });
});
