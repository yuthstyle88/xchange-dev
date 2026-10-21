import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { th } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';

const BRANCH_ID = 'XC-B01';
const BOOKING_ACTIVE_ID = 'BK-alice-001';

const INITIAL_BOOKINGS = [
  makeMyBookingItemDto({
    id: BOOKING_ACTIVE_ID,
    bookingNo: 'B261014-0024',
    status: 'CONFIRMED',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000001',
    startAt: '2026-10-14T10:00:00Z',
    endAt: '2026-10-14T10:30:00Z',
    qrToken: 'qr-token-alice-12345',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-14T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  }),
];

const AVAILABLE_SLOTS = makeBookingSlotsData({
  asOf: '2026-10-14T08:00:00Z',
  slots: [
    makeBookingSlotItemDto({
      id: '00000000-0000-0000-0000-000000000003',
      start: '2026-10-14T11:00:00Z',
      end: '2026-10-14T11:30:00Z',
      available: 3,
    }),
  ],
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown, version = 1, changed = true): Response {
  return json(200, { version, changed, data });
}

beforeAll(() => warmModules(() => import('./main')), 60_000);

describe('APP-17: booking_slots error handling in reschedule dialog', () => {
  let root: HTMLDivElement;
  let slotsStatus = 500;
  let slotsCalls = 0;

  beforeEach(() => {
    vi.resetModules();
    document.body.innerHTML = '';
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
    history.replaceState(null, '', '/app/bookings');
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));

    slotsStatus = 500;
    slotsCalls = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, _init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');

        if (u.pathname === '/api/public/host') {
          return json(200, { org: 'XC' });
        }
        if (u.pathname === '/api/public/orgs/XC/branches') {
          return json(200, {
            branches: [
              {
                code: BRANCH_ID,
                name: BRANCH_ID,
                address: null,
                openingHours: null,
                timezone: 'Asia/Bangkok',
                bookingEnabled: true,
                queueOpen: true,
              },
            ],
          });
        }

        if (u.pathname === '/api/v3/views/my_profile') {
          return viewEnvelope({ customerId: 'cust-alice', displayName: 'Alice' });
        }

        if (u.pathname === '/api/v3/views/my_bookings') {
          return viewEnvelope(makeMyBookingsData({ bookings: INITIAL_BOOKINGS }));
        }

        if (u.pathname === '/api/v3/views/booking_slots') {
          slotsCalls++;
          if (slotsStatus !== 200) {
            return json(slotsStatus, {
              error: { code: 'loadFailed', message: 'Failed to load slots' },
            });
          }
          return viewEnvelope(AVAILABLE_SLOTS);
        }

        return json(404, { error: { code: 'notFound' } });
      }),
    );
  });

  afterEach(() => {
    root.remove();
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('displays worded error when booking_slots fails, and retries when retry button is clicked', async () => {
    await import('./main');
    for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

    const rescheduleBtn = root.querySelector(
      `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement;
    expect(rescheduleBtn).not.toBeNull();

    rescheduleBtn.click();
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

    expect(slotsCalls).toBe(1);

    // Dialog renders error message
    const errorEl = root.querySelector('.xc-dialog-error, [role="alert"]');
    expect(errorEl, 'dialog error is displayed').not.toBeNull();
    expect(errorEl?.textContent).toContain(th['error.serverError']);

    // Now make slots request succeed on retry
    slotsStatus = 200;
    const retryBtn = errorEl?.querySelector('button[data-action="retry"], .xc-retry-btn') as HTMLButtonElement;
    expect(retryBtn, 'retry button in dialog error').not.toBeNull();

    retryBtn.click();
    for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

    expect(slotsCalls).toBe(2);
    // After retry succeeds, slot option should be rendered
    const slotOption = root.querySelector('[data-slot-id="00000000-0000-0000-0000-000000000003"]');
    expect(slotOption, 'slot option rendered after retry').not.toBeNull();
  });
});
