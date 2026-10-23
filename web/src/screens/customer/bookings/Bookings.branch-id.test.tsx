// #4831 (CB-Bookings): /app/bookings keys branch names and timezones by branch id, never code.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCancelOutput,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';
import * as table from './i18n';

const BOOKING_ID = 'bk-cb-1';

const BRANCH_SIAM = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: null,
  openingHours: null,
  timezone: 'Asia/Tokyo',
  bookingEnabled: true,
  queueOpen: true,
};

function makeBooking() {
  // 2026-10-24T15:30:00Z is 00:30–01:00 in Asia/Tokyo (+09:00) vs 22:30–23:00 in Asia/Bangkok (+07:00).
  return makeMyBookingItemDto({
    id: BOOKING_ID,
    bookingNo: 'B261025-0001',
    status: 'CONFIRMED',
    branchId: 'br-1',
    slotId: '00000000-0000-0000-0000-000000000001',
    startAt: '2026-10-24T15:30:00Z',
    endAt: '2026-10-24T16:00:00Z',
    qrToken: 'qr-cb-1',
    quoteSummary: '',
    cancelCutoffAt: '2026-10-24T14:30:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  });
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown): Response {
  return jsonResponse(200, { version: 1, changed: true, data });
}

describe('Bookings.tsx — branch maps keyed by id (#4831)', () => {
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

  it('K1: when the list holds br-1 / MAIN, the row shows Siam, formats slot time in Asia/Tokyo, and shows no load error', async () => {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, { branches: [BRANCH_SIAM] });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeBooking()] }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)).not.toBeNull();
    });

    const card = host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)!;
    expect(card.querySelector('.xc-booking-branch')?.textContent).toBe('Siam');
    const cardTime = card.querySelector('.xc-booking-time')?.textContent ?? '';
    expect(cardTime).toContain('00:30');
    expect(cardTime).toContain('01:00');
    expect(cardTime).not.toContain('22:30');
    expect(host.querySelector('.xc-bookings-error')).toBeNull();
  });

  it('K2: cancelling the booking sends branch: br-1 in the cancel body', async () => {
    let cancelBody: unknown = null;
    const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
      const u = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, { branches: [BRANCH_SIAM] });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeBooking()] }));
      }
      if (method === 'POST' && u.pathname === `/api/bookings/${BOOKING_ID}/cancel`) {
        cancelBody = typeof init?.body === 'string' ? JSON.parse(init.body) : null;
        return jsonResponse(
          200,
          makeBookingCancelOutput({
            id: BOOKING_ID,
            status: 'CANCELLED',
            bookingNo: 'B261025-0001',
          }),
        );
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${BOOKING_ID}"] [data-action="cancel"]`)).not.toBeNull();
    });

    host.querySelector<HTMLButtonElement>(`[data-booking-id="${BOOKING_ID}"] [data-action="cancel"]`)!.click();

    await vi.waitFor(() => {
      expect(host.querySelector('[data-action="confirm-cancel"]')).not.toBeNull();
    });

    host.querySelector<HTMLButtonElement>('[data-action="confirm-cancel"]')!.click();

    await vi.waitFor(() => {
      expect(cancelBody).not.toBeNull();
    });

    expect(cancelBody).toMatchObject({ branch: 'br-1' });
  });

  it('K3: when another branch has code br-1, the row still shows Siam with Tokyo time (keyed by id, never code)', async () => {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            BRANCH_SIAM,
            {
              id: 'br-2',
              code: 'br-1',
              name: 'Other',
              address: null,
              openingHours: null,
              timezone: 'Asia/Bangkok',
              bookingEnabled: true,
              queueOpen: true,
            },
          ],
        });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeBooking()] }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)).not.toBeNull();
    });

    const card = host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)!;
    expect(card.querySelector('.xc-booking-branch')?.textContent).toBe('Siam');
    const cardTime = card.querySelector('.xc-booking-time')?.textContent ?? '';
    expect(cardTime).toContain('00:30');
    expect(cardTime).toContain('01:00');
    expect(cardTime).not.toContain('22:30');
  });

  it('K4: when the list holds only { id: br-9, code: br-1 }, the row has no branch and the load error shows', async () => {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            {
              ...BRANCH_SIAM,
              id: 'br-9',
              code: 'br-1',
            },
          ],
        });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeBooking()] }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-bookings-error')).not.toBeNull();
    });

    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(table.th['error.loadFailed']);
    expect(host.querySelector('.xc-bookings-list')).toBeNull();
  });
});
