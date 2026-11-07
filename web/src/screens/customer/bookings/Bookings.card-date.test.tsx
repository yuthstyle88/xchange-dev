import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown): Response {
  return jsonResponse(200, { version: 1, changed: true, data });
}

describe('Bookings.tsx — booking card date in branch time zone (#6788)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  it('a Bangkok booking card reads 04/11/2026 09:00 – 09:30', async () => {
    const booking = makeMyBookingItemDto({
      id: 'bk-bkk-1',
      bookingNo: 'B261104-0001',
      status: 'CONFIRMED',
      branchId: 'bkk-main',
      startAt: '2026-11-04T02:00:00Z',
      endAt: '2026-11-04T02:30:00Z',
      qrToken: 'qr-1',
    });

    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            {
              id: 'bkk-main',
              code: 'BKK',
              name: 'Bangkok Main',
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
        return viewEnvelope(makeMyBookingsData({ bookings: [booking] }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-bkk-1"]')).not.toBeNull();
    });

    const cardTime = host.querySelector('[data-booking-id="bk-bkk-1"] .xc-booking-time')?.textContent ?? '';
    expect(cardTime).toBe('04/11/2026 09:00 – 09:30');
  });

  it("a Tokyo branch's card shows the Tokyo date and times", async () => {
    // 2026-11-04T15:30:00Z is 2026-11-05 00:30–01:00 in Tokyo (+09:00),
    // but 2026-11-04 22:30–23:00 in Bangkok (+07:00).
    const booking = makeMyBookingItemDto({
      id: 'bk-tyo-1',
      bookingNo: 'B261105-0001',
      status: 'CONFIRMED',
      branchId: 'tyo-main',
      startAt: '2026-11-04T15:30:00Z',
      endAt: '2026-11-04T16:00:00Z',
      qrToken: 'qr-2',
    });

    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            {
              id: 'tyo-main',
              code: 'TYO',
              name: 'Tokyo Main',
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
        return viewEnvelope(makeMyBookingsData({ bookings: [booking] }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-tyo-1"]')).not.toBeNull();
    });

    const cardTime = host.querySelector('[data-booking-id="bk-tyo-1"] .xc-booking-time')?.textContent ?? '';
    expect(cardTime).toBe('05/11/2026 00:30 – 01:00');
  });

  it('two bookings on different days show different dates', async () => {
    const booking1 = makeMyBookingItemDto({
      id: 'bk-day1',
      bookingNo: 'B261104-0001',
      status: 'CONFIRMED',
      branchId: 'bkk-main',
      startAt: '2026-11-04T02:00:00Z',
      endAt: '2026-11-04T02:30:00Z',
      qrToken: 'qr-day1',
    });
    const booking2 = makeMyBookingItemDto({
      id: 'bk-day2',
      bookingNo: 'B261105-0001',
      status: 'CONFIRMED',
      branchId: 'bkk-main',
      startAt: '2026-11-05T02:00:00Z',
      endAt: '2026-11-05T02:30:00Z',
      qrToken: 'qr-day2',
    });

    const fetchFn = vi.fn(async (input: string) => {
      const u = new URL(input, 'http://localhost');
      if (u.pathname === '/api/public/host') return jsonResponse(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') {
        return jsonResponse(200, {
          branches: [
            {
              id: 'bkk-main',
              code: 'BKK',
              name: 'Bangkok Main',
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
        return viewEnvelope(makeMyBookingsData({ bookings: [booking1, booking2] }));
      }
      return jsonResponse(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-day1"]')).not.toBeNull();
      expect(host.querySelector('[data-booking-id="bk-day2"]')).not.toBeNull();
    });

    const cardTime1 = host.querySelector('[data-booking-id="bk-day1"] .xc-booking-time')?.textContent ?? '';
    const cardTime2 = host.querySelector('[data-booking-id="bk-day2"] .xc-booking-time')?.textContent ?? '';

    expect(cardTime1).toBe('04/11/2026 09:00 – 09:30');
    expect(cardTime2).toBe('05/11/2026 09:00 – 09:30');
    expect(cardTime1).not.toBe(cardTime2);
  });
});
