import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { th } from './i18n';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER: StaffMeView = {
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [{ role: 'teller', branchId: 'XC-B01' }],
  branchId: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
};

const STAFF_NO_DESK: StaffMeView = {
  userId: '00000000-0000-4000-8000-000000000002',
  name: 'Somying Manager',
  roles: [{ role: 'manager', branchId: 'XC-B01' }],
  branchId: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['RatesRead'],
};

const ONE_BOOKING: DeskBookingsData = {
  items: [
    {
      id: 'bk-1',
      bookingNo: 'B261016-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Alice Customer',
      phoneLast4: '1234',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    },
  ],
};

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen (B02) — §9 load-error behaviour (#2163)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('staff');
    setLang('th');
  });

  it('row 1: the first desk_bookings read fails 500 -> an error banner with Retry is shown; not blank', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          return jsonResponse(500, { error: { code: 'INTERNAL_ERROR', message: 'db down' } });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    expect(host.textContent?.trim()).not.toBe('');
    const banner = host.querySelector('.xc-error-banner[role="alert"]');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.serverError']);

    const retryBtn = banner!.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(retryBtn).not.toBeNull();
    expect(retryBtn!.textContent?.trim()).toBe(th.retry);
  });

  it('row 2: then Retry succeeds -> the bookings show; the error is gone', async () => {
    let bookingsCall = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          bookingsCall += 1;
          if (bookingsCall === 1) {
            return jsonResponse(500, { error: { code: 'INTERNAL_ERROR', message: 'db down' } });
          }
          return jsonResponse(200, { version: 2, changed: true, data: ONE_BOOKING });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const retryBtn = host.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();
    await flush();

    expect(bookingsCall).toBe(2);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).toContain('Alice Customer');
  });

  it('row 3: a later read fails after a good load -> the error is shown; the last bookings stay', async () => {
    let bookingsCall = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          bookingsCall += 1;
          if (bookingsCall === 1) {
            return jsonResponse(200, { version: 1, changed: true, data: ONE_BOOKING });
          }
          return jsonResponse(500, { error: { code: 'INTERNAL_ERROR', message: 'db down' } });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    expect(host.textContent).toContain('B261016-0001');
    expect(host.querySelector('.xc-error-banner')).toBeNull();

    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]');
    expect(refreshBtn).not.toBeNull();
    refreshBtn!.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.serverError']);
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).toContain('Alice Customer');
  });

  it('row 4: then a read succeeds -> that error is gone', async () => {
    let bookingsCall = 0;
    const seenSince: Array<string | null> = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          bookingsCall += 1;
          const since = u.searchParams.get('since');
          seenSince.push(since);
          if (bookingsCall === 2 || bookingsCall === 4) {
            return jsonResponse(500, { error: { code: 'INTERNAL_ERROR', message: 'db down' } });
          }
          if (since === '1') {
            return jsonResponse(200, { version: 1, changed: false });
          }
          return jsonResponse(200, { version: 1, changed: true, data: ONE_BOOKING });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    // Later read #1 fails -> Refresh succeeds with {version: 1, changed: false} -> banner gone
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')).not.toBeNull();

    refreshBtn.click();
    await flush();
    expect(bookingsCall).toBe(3);
    expect(seenSince).toEqual([null, '1', '1']);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');

    // Later read #2 fails -> Retry button succeeds with {version: 1, changed: false} -> banner gone
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')).not.toBeNull();
    const retryBtn = host.querySelector<HTMLButtonElement>('button[data-action="retry"]')!;
    expect(retryBtn).not.toBeNull();

    retryBtn.click();
    await flush();
    expect(bookingsCall).toBe(5);
    expect(seenSince).toEqual([null, '1', '1', '1', '1']);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');
  });

  it('row 5: the first read is refused roleNotAllowed -> the refusal, as today', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          return jsonResponse(403, { error: { code: 'roleNotAllowed', message: 'Refused' } });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const refusal = host.querySelector('.xc-refusal[role="alert"]');
    expect(refusal).not.toBeNull();
    expect(refusal!.textContent).toContain(th['error.roleNotAllowed']);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.querySelector('[data-screen="desk-bookings"]')).toBeNull();
  });

  it('row 6: no DESK_PERMISSION in staff_me -> the refusal, as today', async () => {
    let bookingsCall = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_NO_DESK });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          bookingsCall += 1;
          return jsonResponse(200, { version: 1, changed: true, data: ONE_BOOKING });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    expect(bookingsCall).toBe(0);
    const refusal = host.querySelector('.xc-refusal[role="alert"]');
    expect(refusal).not.toBeNull();
    expect(refusal!.textContent).toContain(th['error.roleNotAllowed']);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.querySelector('[data-screen="desk-bookings"]')).toBeNull();
  });
});
