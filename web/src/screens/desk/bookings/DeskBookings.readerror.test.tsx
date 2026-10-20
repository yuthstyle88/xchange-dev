// #2195: Desk bookings (B02) — read error recovery and write error preservation (§8, §9).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCheckInOutput,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeStaffMeView,
  makeStaffRoleDto,
} from '../../../shared/testing/fixtures';
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

const STAFF_TELLER = makeStaffMeView({
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeStaffRoleDto({ role: 'teller', branchId: 'XC-B01' })],
  branchId: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const INITIAL_BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
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
    }),
  ],
});

const UPDATED_BOOKINGS = makeDeskBookingsData({
  items: [
    ...INITIAL_BOOKINGS.items,
    makeDeskBookingItemDto({
      id: 'bk-2',
      bookingNo: 'B261016-0002',
      status: 'CONFIRMED',
      customerDisplayName: 'Bob Customer',
      phoneLast4: '5678',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    }),
  ],
});

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen (B02) — §8 & §9 read error recovery (#2195)', () => {
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

  it('row 1: Refresh fails -> Banner with the worded error', async () => {
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
            return jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS });
          }
          return jsonResponse(500, { error: { code: 'serverError', message: 'raw server failure' } });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(host.querySelector('.xc-error-banner')).toBeNull();

    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner[role="alert"]');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.serverError']);
    expect(banner!.textContent).not.toContain('raw server failure');
  });

  it('row 2: Then Refresh succeeds with changed:false -> Banner gone', async () => {
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
          seenSince.push(u.searchParams.get('since'));
          if (bookingsCall === 1) {
            return jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS });
          }
          if (bookingsCall === 2) {
            return jsonResponse(500, { error: { code: 'serverError', message: 'db down' } });
          }
          return jsonResponse(200, { version: 1, changed: false });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')).not.toBeNull();

    refreshBtn.click();
    await flush();
    expect(seenSince).toEqual([null, '1', '1']);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');
  });

  it('row 3: Then Refresh succeeds with new items -> Banner gone; items updated', async () => {
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
            return jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS });
          }
          if (bookingsCall === 2) {
            return jsonResponse(500, { error: { code: 'serverError', message: 'db down' } });
          }
          return jsonResponse(200, { version: 2, changed: true, data: UPDATED_BOOKINGS });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).not.toContain('B261016-0002');

    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')).not.toBeNull();

    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).toContain('B261016-0002');
    expect(host.textContent).toContain('Bob Customer');
  });

  it('row 4: A write fails -> Banner with Retry, kept across a successful read until the next action', async () => {
    let checkInCalls = 0;
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
          if (u.searchParams.get('since') === '1') {
            return jsonResponse(200, { version: 1, changed: false });
          }
          return jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS });
        }
        if (method === 'POST' && u.pathname === '/api/bookings/bk-1/check-in') {
          checkInCalls += 1;
          if (checkInCalls === 1) {
            return jsonResponse(409, { error: { code: 'qrMismatch', message: 'QR mismatch' } });
          }
          return jsonResponse(
            200,
            makeBookingCheckInOutput({
              id: 'bk-1',
              status: 'CHECKED_IN',
              ticketNumber: 'B01-009',
              bookingNo: 'B261016-0001',
            }),
          );
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const checkInBtn = host.querySelector<HTMLButtonElement>(
      '[data-booking-id="bk-1"] button[data-action="check-in"]',
    )!;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner[role="alert"]');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(th['error.qrMismatch']);
    const retryBtn = banner!.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(retryBtn).not.toBeNull();

    // A successful read keeps the write error until the next write/dialog action
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(th['error.qrMismatch']);

    // Clicking Retry (the next action) succeeds and clears the banner
    retryBtn!.click();
    await flush();
    expect(checkInCalls).toBe(2);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
  });

  it('row 5: roleNotAllowed -> The refusal block (kept)', async () => {
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
  });

  it('edge case (§9): a read error and then a write error -> the write error stays after a later successful read only if it came after that read', async () => {
    let bookingsCall = 0;
    let resolveCheckIn: ((res: Response) => void) | undefined;

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
            return jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS });
          }
          if (bookingsCall === 2 || bookingsCall === 4) {
            return jsonResponse(500, { error: { code: 'serverError', message: 'db down' } });
          }
          return jsonResponse(200, { version: 1, changed: false });
        }
        if (method === 'POST' && u.pathname === '/api/bookings/bk-1/check-in') {
          return new Promise<Response>((resolve) => {
            resolveCheckIn = resolve;
          });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    // 1. Start a check-in write (in flight), then a read fails (#2), then the write fails (write error came AFTER the read error)
    const checkInBtn = host.querySelector<HTMLButtonElement>(
      '[data-booking-id="bk-1"] button[data-action="check-in"]',
    )!;
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;

    checkInBtn.click();
    await flush();

    // Read #2 fails while write is in flight -> read error shown
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(sharedTh['error.serverError']);

    // Now the write fails after the read error -> write error replaces read error
    resolveCheckIn!(jsonResponse(409, { error: { code: 'qrMismatch', message: 'QR mismatch' } }));
    await flush();
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(th['error.qrMismatch']);

    // Later read #3 succeeds -> write error MUST stay because it came after the failed read
    refreshBtn.click();
    await flush();
    expect(bookingsCall).toBe(3);
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(th['error.qrMismatch']);

    // 2. Conversely, if a read fails (#4) AFTER that write error, then a later successful read (#5) clears the banner
    refreshBtn.click();
    await flush();
    expect(bookingsCall).toBe(4);
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(sharedTh['error.serverError']);

    refreshBtn.click();
    await flush();
    expect(bookingsCall).toBe(5);
    expect(host.querySelector('.xc-error-banner')).toBeNull();
  });

  it('edge case (§9 validationFailed): check-in in flight, Refresh fails, check-in answers 200 with malformed body, then Refresh answers changed:false -> validationFailed banner stays', async () => {
    let bookingsCall = 0;
    let resolveCheckIn: ((res: Response) => void) | undefined;

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
            return jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS });
          }
          if (bookingsCall === 2) {
            return jsonResponse(500, { error: { code: 'serverError', message: 'db down' } });
          }
          return jsonResponse(200, { version: 1, changed: false });
        }
        if (method === 'POST' && u.pathname === '/api/bookings/bk-1/check-in') {
          return new Promise<Response>((resolve) => {
            resolveCheckIn = resolve;
          });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
      }),
    );

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const checkInBtn = host.querySelector<HTMLButtonElement>(
      '[data-booking-id="bk-1"] button[data-action="check-in"]',
    )!;
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;

    // Check-in starts and stays in flight
    checkInBtn.click();
    await flush();

    // Refresh fails while check-in is in flight -> read error banner shown
    refreshBtn.click();
    await flush();
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(sharedTh['error.serverError']);

    // Check-in answers 200 with malformed body (no status string) -> validationFailed banner shown
    resolveCheckIn!(jsonResponse(200, {}));
    await flush();
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(th['error.validationFailed']);

    // Refresh answers {changed: false} -> validationFailed banner must still be shown
    refreshBtn.click();
    await flush();
    expect(bookingsCall).toBe(3);
    expect(host.querySelector('.xc-error-banner')?.textContent).toContain(th['error.validationFailed']);
  });
});
