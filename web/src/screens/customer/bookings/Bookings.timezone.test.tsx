// #4172: /app/bookings reads each booking's branch timezone from the public branch list.
// - Row 1: a branch whose timezone is not Bangkok (e.g. Asia/Tokyo) formats slot times and sets reschedule min in that timezone.
// - Row 2: a branch with no timezone shows the screen's load error rather than guessing.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { getTodayInTimezone } from '../book/Book';
import { BookingsScreen } from './Bookings';
import * as table from './i18n';

const BOOKING_ID = 'bk-tyo-1';
const BRANCH_TYO = 'XC-TYO';
const SLOT_TYO = '00000000-0000-0000-0000-000000000101';

function makeTokyoBooking() {
  // 2026-10-24T15:30:00Z is 2026-10-25 00:30–01:00 in Asia/Tokyo (+09:00),
  // vs 2026-10-24 22:30–23:00 in Asia/Bangkok (+07:00).
  return makeMyBookingItemDto({
    id: BOOKING_ID,
    bookingNo: 'B261025-0001',
    status: 'CONFIRMED',
    branchId: BRANCH_TYO,
    slotId: '00000000-0000-0000-0000-000000000001',
    startAt: '2026-10-24T15:30:00Z',
    endAt: '2026-10-24T16:00:00Z',
    qrToken: 'qr-tyo-1',
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

function makeTokyoSlots() {
  // 2026-10-25T01:00:00Z is 10:00–10:30 in Asia/Tokyo (+09:00),
  // vs 08:00–08:30 in Asia/Bangkok (+07:00).
  return makeBookingSlotsData({
    asOf: '2026-10-20T15:30:00Z',
    slots: [
      makeBookingSlotItemDto({
        id: SLOT_TYO,
        start: '2026-10-25T01:00:00Z',
        end: '2026-10-25T01:30:00Z',
        available: 2,
      }),
    ],
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

describe('Bookings.tsx — branch timezone from public branch list (#4172)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    // 2026-10-20T15:30:00Z is 2026-10-20T22:30:00+07:00 in Asia/Bangkok (2026-10-20),
    // and 2026-10-21T00:30:00+09:00 in Asia/Tokyo (2026-10-21).
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

  it('row 1: a branch whose timezone is Asia/Tokyo uses it for booking slot times, reschedule slot times, and reschedule min', async () => {
    const slotDatesRequested: (string | null)[] = [];
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') {
        return jsonResponse(200, { org: 'XC' });
      }
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            {
              code: BRANCH_TYO,
              name: 'Tokyo Branch',
              address: null,
              openingHours: null,
              timezone: 'Asia/Tokyo',
              bookingEnabled: true,
              queueOpen: true,
            },
          ],
        });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeTokyoBooking()] }));
      }
      if (u.pathname === '/api/v3/views/booking_slots') {
        slotDatesRequested.push(u.searchParams.get('date'));
        return viewEnvelope(makeTokyoSlots());
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)).not.toBeNull();
    });

    // Booking card slot time uses Asia/Tokyo (00:30 – 01:00), not Asia/Bangkok (22:30 – 23:00).
    const cardTime = host.querySelector(`[data-booking-id="${BOOKING_ID}"] .xc-booking-time`)?.textContent ?? '';
    expect(cardTime).toContain('00:30');
    expect(cardTime).toContain('01:00');
    expect(cardTime).not.toContain('22:30');

    // Open reschedule dialog
    const rescheduleBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_ID}"] button[data-action="reschedule"]`,
    )!;
    rescheduleBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-slot-id="${SLOT_TYO}"]`)).not.toBeNull();
    });

    const dateInput = host.querySelector<HTMLInputElement>('.xc-reschedule-dialog input[type="date"]')!;
    expect(getTodayInTimezone('Asia/Bangkok')).toBe('2026-10-20');
    expect(getTodayInTimezone('Asia/Tokyo')).toBe('2026-10-21');
    expect(dateInput.min).toBe('2026-10-21');
    expect(dateInput.value).toBe('2026-10-25');
    expect(slotDatesRequested).toEqual(['2026-10-25']);

    // Reschedule dialog slot time uses Asia/Tokyo (10:00 – 10:30), not Asia/Bangkok (08:00 – 08:30).
    const slotText = host.querySelector(`[data-slot-id="${SLOT_TYO}"]`)?.textContent ?? '';
    expect(slotText).toContain('10:00');
    expect(slotText).toContain('10:30');
    expect(slotText).not.toContain('08:00');
  });

  it('row 2: a branch with no timezone shows the load error', async () => {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') {
        return jsonResponse(200, { org: 'XC' });
      }
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            {
              code: BRANCH_TYO,
              name: 'Tokyo Branch',
              address: null,
              openingHours: null,
              timezone: '',
              bookingEnabled: true,
              queueOpen: true,
            },
          ],
        });
      }
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeTokyoBooking()] }));
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
