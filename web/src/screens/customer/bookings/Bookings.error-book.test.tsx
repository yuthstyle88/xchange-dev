// UX-7 (#6467): bookings load-error card offers a link to /app/book alongside retry.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeMyBookingItemDto, makeMyBookingsData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';
import * as table from './i18n';

const BRANCH_ID = 'XC-B01';
const BOOKING_ID = 'BK-alice-001';

const ACTIVE_BOOKING = makeMyBookingItemDto({
  id: BOOKING_ID,
  bookingNo: 'B261014-0024',
  status: 'CONFIRMED',
  branchId: BRANCH_ID,
  slotId: '00000000-0000-0000-0000-000000000001',
  startAt: '2026-10-14T10:00:00Z',
  endAt: '2026-10-14T10:30:00Z',
  qrToken: 'qr-token-alice-12345',
  cancelCutoffAt: '2026-10-14T09:00:00Z',
});

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function makeFetch(listResponder: () => Response) {
  return vi.fn(async (input: string) => {
    const u = new URL(input, 'http://localhost');
    if (u.pathname === '/api/public/host') {
      return jsonResponse(200, { org: 'XC' });
    }
    if (u.pathname === '/api/public/orgs/XC/branches') {
      return jsonResponse(200, {
        branches: [
          {
            id: BRANCH_ID,
            code: BRANCH_ID,
            name: 'Bangkok HQ',
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
      return listResponder();
    }
    return jsonResponse(404, { error: { code: 'notFound' } });
  });
}

describe('Bookings.tsx — UX-7 error card bookNow link (#6467)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));
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

  it('row 1: bookings load fails -> the card shows the retry button and a link to /app/book with the bookNow text', async () => {
    const fetchStub = makeFetch(() =>
      jsonResponse(500, { error: { code: 'serverError', message: 'boom' } }),
    );
    dispose = render(() => <BookingsScreen fetch={fetchStub} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-bookings-error')).not.toBeNull();
    });

    const errorCard = host.querySelector('.xc-bookings-error')!;
    expect(errorCard.querySelector('[data-action="retry"]')).not.toBeNull();
    const bookLink = errorCard.querySelector('a.xc-btn.xc-btn-secondary[href="/app/book"]');
    expect(bookLink).not.toBeNull();
    expect(bookLink?.textContent).toBe(table.th.bookNow);
  });

  it('row 2: empty list -> unchanged (empty state shows bookNow link, no error card)', async () => {
    const fetchStub = makeFetch(() =>
      jsonResponse(200, { version: 1, changed: true, data: makeMyBookingsData({ bookings: [] }) }),
    );
    dispose = render(() => <BookingsScreen fetch={fetchStub} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-bookings-empty')).not.toBeNull();
    });

    expect(host.querySelector('.xc-bookings-error')).toBeNull();
    const emptyCard = host.querySelector('.xc-bookings-empty')!;
    const bookLink = emptyCard.querySelector('a.xc-btn.xc-btn-primary[href="/app/book"]');
    expect(bookLink).not.toBeNull();
    expect(bookLink?.textContent).toBe(table.th.bookNow);
  });

  it('row 3: list loads -> no such link in an error card', async () => {
    const fetchStub = makeFetch(() =>
      jsonResponse(200, {
        version: 1,
        changed: true,
        data: makeMyBookingsData({ bookings: [ACTIVE_BOOKING] }),
      }),
    );
    dispose = render(() => <BookingsScreen fetch={fetchStub} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${BOOKING_ID}"]`)).not.toBeNull();
    });

    expect(host.querySelector('.xc-bookings-error')).toBeNull();
    expect(host.querySelector('.xc-bookings-error a[href="/app/book"]')).toBeNull();
  });
});
