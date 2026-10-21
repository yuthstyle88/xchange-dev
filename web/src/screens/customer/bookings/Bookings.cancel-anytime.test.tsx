// #4204: Owner 2026-10-21 ("ยกเลิกการจอง ยกเลิกได้เลย" · "ไม่ต้อง มี การอนุมัติ เพราะไม่มีอะไรเกี่ยวกับเงิน"):
// a customer cancels at any time; reschedule keeps the cutoff, and the deadline line words the reschedule cutoff only.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeMyBookingItemDto, makeMyBookingsData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';

const BRANCH_ID = 'XC-BKK';
const PAST_CUTOFF_ID = 'bk-past-cutoff';
const FUTURE_CUTOFF_ID = 'bk-future-cutoff';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function makeFetch() {
  return vi.fn(async (input: string) => {
    const u = new URL(input, 'http://localhost');
    if (u.pathname === '/api/public/host') {
      return jsonResponse(200, { org: 'XC' });
    }
    if (u.pathname === '/api/public/orgs/XC/branches') {
      return jsonResponse(200, {
        branches: [
          {
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
      return jsonResponse(200, {
        version: 1,
        changed: true,
        data: makeMyBookingsData({
          bookings: [
            makeMyBookingItemDto({
              id: PAST_CUTOFF_ID,
              bookingNo: 'B261021-0001',
              status: 'CONFIRMED',
              branchId: BRANCH_ID,
              slotId: 'slot-1',
              startAt: '2026-10-21T10:00:00Z',
              endAt: '2026-10-21T10:30:00Z',
              qrToken: 'qr-1',
              quoteSummary: '',
              cancelCutoffAt: '2026-10-21T09:00:00Z',
              ticketNumber: null,
              ticketsAhead: null,
              estimateSeconds: null,
              estimateLowSeconds: null,
              estimateHighSeconds: null,
              estimateAsOf: null,
              readiness: 'unknown',
            }),
            makeMyBookingItemDto({
              id: FUTURE_CUTOFF_ID,
              bookingNo: 'B261021-0002',
              status: 'CONFIRMED',
              branchId: BRANCH_ID,
              slotId: 'slot-2',
              startAt: '2026-10-21T12:00:00Z',
              endAt: '2026-10-21T12:30:00Z',
              qrToken: 'qr-2',
              quoteSummary: '',
              cancelCutoffAt: '2026-10-21T11:00:00Z',
              ticketNumber: null,
              ticketsAhead: null,
              estimateSeconds: null,
              estimateLowSeconds: null,
              estimateHighSeconds: null,
              estimateAsOf: null,
              readiness: 'unknown',
            }),
          ],
        }),
      });
    }
    return jsonResponse(404, { error: { code: 'notFound' } });
  });
}

describe('Bookings.tsx — cancel anytime & reschedule deadline wording (#4204)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    vi.setSystemTime(new Date('2026-10-21T09:30:00Z'));
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

  it('row 1: a booking past its cutoff still shows [data-action=cancel] and hides [data-action=reschedule]', async () => {
    dispose = render(() => <BookingsScreen fetch={makeFetch()} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${PAST_CUTOFF_ID}"]`)).not.toBeNull();
    });

    const pastCard = host.querySelector(`[data-booking-id="${PAST_CUTOFF_ID}"]`)!;
    expect(pastCard.querySelector('[data-action="cancel"]')).not.toBeNull();
    expect(pastCard.querySelector('[data-action="reschedule"]')).toBeNull();
  });

  it('row 2: the deadline line uses the reschedule wording', async () => {
    dispose = render(() => <BookingsScreen fetch={makeFetch()} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${FUTURE_CUTOFF_ID}"]`)).not.toBeNull();
    });

    const futureCard = host.querySelector(`[data-booking-id="${FUTURE_CUTOFF_ID}"]`)!;
    const deadline = futureCard.querySelector('[data-testid="cancel-deadline"]');
    expect(deadline?.textContent).toContain('เลื่อนนัดได้ถึง');
    expect(deadline?.textContent).not.toContain('ยกเลิกได้ถึง');
  });
});
