// #6313 up-front tests (behaviour table rows B1–B2). Written before the implementation by claude/37e371ca on the
// controller's order. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// Pinned: a booking whose branch is not in the public branch list (for example a deactivated branch) still renders.
// Its branch line shows the branch id the booking carries (`MyBookingItemDto` has no branch code), and it shows no
// slot hours (`.xc-booking-time`), because the branch's timezone is unknown and a time in the device's zone would be wrong. The other
// bookings render as before. The page error stays for a failed read of `my_bookings`.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeMyBookingItemDto, makeMyBookingsData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';

const SIAM = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: null,
  openingHours: null,
  timezone: 'Asia/Tokyo',
  bookingEnabled: true,
  queueOpen: true,
};

const SILOM = {
  id: 'br-2',
  code: 'SILOM',
  name: 'Silom',
  address: null,
  openingHours: null,
  timezone: 'Asia/Bangkok',
  bookingEnabled: true,
  queueOpen: true,
};

// Not in the public list.
const GONE_BRANCH_ID = 'br-gone';

function booking(id: string, branchId: string, bookingNo: string) {
  // 2026-10-24T15:30:00Z is 00:30–01:00 in Asia/Tokyo and 22:30–23:00 in Asia/Bangkok.
  return makeMyBookingItemDto({
    id,
    bookingNo,
    status: 'CONFIRMED',
    branchId,
    slotId: '00000000-0000-0000-0000-000000000001',
    startAt: '2026-10-24T15:30:00Z',
    endAt: '2026-10-24T16:00:00Z',
    qrToken: null,
    quoteSummary: '',
    cancelCutoffAt: '2026-10-24T14:30:00Z',
    readiness: 'unknown',
  });
}

const BOOKINGS = [
  booking('bk-siam', SIAM.id, 'B261025-0001'),
  booking('bk-gone', GONE_BRANCH_ID, 'B261025-0002'),
  booking('bk-silom', SILOM.id, 'B261025-0003'),
];

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown): Response {
  return jsonResponse(200, { version: 1, changed: true, data });
}

function card(host: HTMLElement, id: string): Element | null {
  return host.querySelector(`[data-booking-id="${id}"]`);
}

describe('Bookings.tsx — a booking on a branch missing from the public list (#6313)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    vi.setSystemTime(new Date('2026-10-20T15:30:00Z'));
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('B1: 3 bookings, 1 on a branch not in the public list — all 3 render; the odd one shows its branch, with no hours', async () => {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, { branches: [SIAM, SILOM] });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: BOOKINGS }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-bookings-loading')).toBeNull();
    });

    expect(host.querySelector('.xc-bookings-error')).toBeNull();
    expect(host.querySelectorAll('.xc-booking-card')).toHaveLength(3);

    const siam = card(host, 'bk-siam');
    expect(siam).not.toBeNull();
    expect(siam!.querySelector('.xc-booking-branch')?.textContent).toBe('Siam');
    const siamTime = siam!.querySelector('.xc-booking-time')?.textContent ?? '';
    expect(siamTime).toContain('00:30');
    expect(siamTime).toContain('01:00');

    const silom = card(host, 'bk-silom');
    expect(silom).not.toBeNull();
    expect(silom!.querySelector('.xc-booking-branch')?.textContent).toBe('Silom');
    const silomTime = silom!.querySelector('.xc-booking-time')?.textContent ?? '';
    expect(silomTime).toContain('22:30');
    expect(silomTime).toContain('23:00');

    const gone = card(host, 'bk-gone');
    expect(gone).not.toBeNull();
    expect(gone!.textContent).toContain('B261025-0002');
    expect(gone!.querySelector('.xc-booking-branch')?.textContent).toBe(GONE_BRANCH_ID);
    // No hours: the slot-time line is absent or carries no HH:MM.
    expect(gone!.querySelector('.xc-booking-time')?.textContent ?? '').not.toMatch(/\d{1,2}:\d{2}/);
  });

  it('B2: the my_bookings read fails — the page error shows, as today', async () => {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, { branches: [SIAM, SILOM] });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return jsonResponse(500, { error: { code: 'serverError' } });
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-bookings-error')).not.toBeNull();
    });

    expect(host.querySelector('.xc-bookings-error [data-action="retry"]')).not.toBeNull();
    expect(host.querySelector('.xc-bookings-list')).toBeNull();
    expect(host.querySelectorAll('.xc-booking-card')).toHaveLength(0);
  });
});
