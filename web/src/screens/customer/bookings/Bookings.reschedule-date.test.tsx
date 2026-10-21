// #4152: the reschedule dialog on /app/bookings lets the customer pick another service date.
// - Defaults to the booking's own service date and passes `date` on the first `booking_slots` read.
// - Changing the date re-reads `booking_slots` with the new `date`, dropping any late answer from the old date.
// - Sets `min` to today in the branch timezone with no `max`.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { getTodayInTimezone } from '../book/Book';
import { BookingsScreen } from './Bookings';

const BOOKING_ID = 'bk-2026-10-25';
const BRANCH_ID = 'XC-B01';
const SLOT_OLD = '00000000-0000-0000-0000-0000000000a1';
const SLOT_NEW = '00000000-0000-0000-0000-0000000000b2';

function makeBookingFor20261025() {
  return makeMyBookingItemDto({
    id: BOOKING_ID,
    bookingNo: 'B261025-0001',
    status: 'CONFIRMED',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000001',
    startAt: '2026-10-25T03:00:00Z',
    endAt: '2026-10-25T03:30:00Z',
    qrToken: 'qr-bk-2026-10-25',
    quoteSummary: '',
    cancelCutoffAt: '2026-10-25T02:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  });
}

function makeSlotsWithId(slotId: string, date: string) {
  return makeBookingSlotsData({
    asOf: '2026-10-21T03:00:00Z',
    slots: [
      makeBookingSlotItemDto({
        id: slotId,
        start: `${date}T04:00:00Z`,
        end: `${date}T04:30:00Z`,
        available: 3,
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

interface PendingSlotsCall {
  branch: string | null;
  date: string | null;
  resolve: (res: Response) => void;
}

describe('Bookings.tsx — reschedule dialog date picker (#4152)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let slotsCalls: PendingSlotsCall[];

  beforeEach(() => {
    // 2026-10-20T18:30:00Z is 2026-10-21T01:30:00+07:00 in Asia/Bangkok
    vi.setSystemTime(new Date('2026-10-20T18:30:00Z'));
    slotsCalls = [];
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

  async function mountAndOpenReschedule(): Promise<HTMLInputElement> {
    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/v3/views/my_bookings') {
        return viewEnvelope(makeMyBookingsData({ bookings: [makeBookingFor20261025()] }));
      }
      if (u.pathname === '/api/v3/views/booking_slots') {
        return new Promise<Response>((resolve) => {
          slotsCalls.push({
            branch: u.searchParams.get('branch'),
            date: u.searchParams.get('date'),
            resolve,
          });
        });
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(
        host.querySelector(`[data-booking-id="${BOOKING_ID}"] button[data-action="reschedule"]`),
      ).not.toBeNull();
    });

    const rescheduleBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_ID}"] button[data-action="reschedule"]`,
    )!;
    rescheduleBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-reschedule-dialog input[type="date"]')).not.toBeNull();
    });

    return host.querySelector<HTMLInputElement>('.xc-reschedule-dialog input[type="date"]')!;
  }

  it('row 1: opening reschedule on a booking for 2026-10-25 sets the date input to 2026-10-25 and sends date=2026-10-25 on the first slots read', async () => {
    const dateInput = await mountAndOpenReschedule();

    expect(dateInput.value).toBe('2026-10-25');
    expect(slotsCalls).toHaveLength(1);
    expect(slotsCalls[0]!.branch).toBe(BRANCH_ID);
    expect(slotsCalls[0]!.date).toBe('2026-10-25');
  });

  it('row 2: changing the date triggers a new booking_slots read with the new date, and a late answer for the old date is dropped', async () => {
    const dateInput = await mountAndOpenReschedule();
    expect(slotsCalls).toHaveLength(1);
    expect(slotsCalls[0]!.date).toBe('2026-10-25');

    dateInput.value = '2026-10-28';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    expect(slotsCalls).toHaveLength(2);
    expect(slotsCalls[1]!.branch).toBe(BRANCH_ID);
    expect(slotsCalls[1]!.date).toBe('2026-10-28');

    // New date's slots resolve first, then old date's slots arrive late.
    slotsCalls[1]!.resolve(viewEnvelope(makeSlotsWithId(SLOT_NEW, '2026-10-28')));
    await vi.waitFor(() => {
      expect(host.querySelector(`[data-slot-id="${SLOT_NEW}"]`)).not.toBeNull();
    });

    slotsCalls[0]!.resolve(viewEnvelope(makeSlotsWithId(SLOT_OLD, '2026-10-25')));
    await new Promise((r) => setTimeout(r, 0));

    expect(host.querySelector(`[data-slot-id="${SLOT_NEW}"]`)).not.toBeNull();
    expect(host.querySelector(`[data-slot-id="${SLOT_OLD}"]`)).toBeNull();
  });

  it('row 3: the date input has min = today in the branch timezone and no max', async () => {
    const dateInput = await mountAndOpenReschedule();
    const expectedToday = getTodayInTimezone('Asia/Bangkok');

    expect(expectedToday).toBe('2026-10-21');
    expect(dateInput.min).toBe(expectedToday);
    expect(dateInput.getAttribute('max')).toBeNull();
  });
});
