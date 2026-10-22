// xchange#2418: the reschedule dialog must only list slots read for the
// booking whose dialog is open now. A late answer (or error) for a booking
// whose dialog was closed must be dropped, and the list must be cleared on
// open, on close and when the read fails.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { th } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';

const BRANCH_A = 'XC-B01';
const BRANCH_B = 'XC-B02';
const BOOKING_A = 'BK-alice-001';
const BOOKING_B = 'BK-alice-002';

const SLOT_A = '00000000-0000-0000-0000-0000000000a1';
const SLOT_A_FRESH = '00000000-0000-0000-0000-0000000000a2';
const SLOT_B = '00000000-0000-0000-0000-0000000000b1';

function booking(id: string, bookingNo: string, branchId: string) {
  return makeMyBookingItemDto({
    id,
    bookingNo,
    status: 'CONFIRMED',
    branchId,
    slotId: '00000000-0000-0000-0000-000000000001',
    startAt: '2026-10-14T10:00:00Z',
    endAt: '2026-10-14T10:30:00Z',
    qrToken: `qr-${id}`,
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-14T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  });
}

const INITIAL_BOOKINGS = [
  booking(BOOKING_A, 'B261014-0024', BRANCH_A),
  booking(BOOKING_B, 'B261014-0025', BRANCH_B),
];

function slots(id: string) {
  return makeBookingSlotsData({
    asOf: '2026-10-14T08:00:00Z',
    slots: [
      makeBookingSlotItemDto({
        id,
        start: '2026-10-14T11:00:00Z',
        end: '2026-10-14T11:30:00Z',
        available: 3,
      }),
    ],
  });
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown, version = 1, changed = true): Response {
  return json(200, { version, changed, data });
}

interface SlotsCall {
  branch: string | null;
  resolve: (r: Response) => void;
}

beforeAll(() => warmModules(() => import('./main')), 60_000);

describe('xchange#2418: reschedule dialog lists only the open booking’s slots', () => {
  let root: HTMLDivElement;
  let slotsCalls: SlotsCall[] = [];

  const flush = async (n = 10) => {
    for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
  };

  const answer = async (call: SlotsCall, slotId: string) => {
    call.resolve(viewEnvelope(slots(slotId)));
    await flush();
  };

  const fail = async (call: SlotsCall) => {
    call.resolve(json(500, { error: { code: 'loadFailed', message: 'Failed to load slots' } }));
    await flush();
  };

  const openReschedule = async (bookingId: string) => {
    const btn = root.querySelector(
      `[data-booking-id="${bookingId}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement | null;
    expect(btn, `reschedule button for ${bookingId}`).not.toBeNull();
    btn!.click();
    await flush();
  };

  const closeReschedule = async () => {
    const back = root.querySelector('.xc-reschedule-dialog .xc-btn-secondary') as HTMLButtonElement | null;
    expect(back, 'reschedule dialog back button').not.toBeNull();
    back!.click();
    await flush();
  };

  const listed = (slotId: string) => root.querySelector(`[data-slot-id="${slotId}"]`) !== null;
  const dialogError = () => root.querySelector('.xc-reschedule-dialog .xc-dialog-error');

  beforeEach(async () => {
    vi.resetModules();
    document.body.innerHTML = '';
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
    history.replaceState(null, '', '/app/bookings');
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));

    slotsCalls = [];

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
            branches: [BRANCH_A, BRANCH_B].map((code) => ({
              id: code,
              code,
              name: code,
              address: null,
              openingHours: null,
              timezone: 'Asia/Bangkok',
              bookingEnabled: true,
              queueOpen: true,
            })),
          });
        }
        if (u.pathname === '/api/v3/views/my_profile') {
          return viewEnvelope({ customerId: 'cust-alice', displayName: 'Alice' });
        }
        if (u.pathname === '/api/v3/views/my_bookings') {
          return viewEnvelope(makeMyBookingsData({ bookings: INITIAL_BOOKINGS }));
        }
        if (u.pathname === '/api/v3/views/booking_slots') {
          return new Promise<Response>((resolve) => {
            slotsCalls.push({ branch: u.searchParams.get('branch'), resolve });
          });
        }
        return json(404, { error: { code: 'notFound' } });
      }),
    );

    await import('./main');
    await flush(20);
  });

  afterEach(() => {
    root.remove();
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('open A, close, open B: before B’s slots answer, no slot from A is listed', async () => {
    await openReschedule(BOOKING_A);
    await closeReschedule();
    await openReschedule(BOOKING_B);
    expect(slotsCalls.map((c) => c.branch)).toEqual([BRANCH_A, BRANCH_B]);

    // A's slow answer lands while B's read is still pending.
    await answer(slotsCalls[0], SLOT_A);

    expect(listed(SLOT_A), 'A slot must not be listed in B’s dialog').toBe(false);
    expect(root.querySelector('.xc-slots-loading'), 'B is still loading').not.toBeNull();
  });

  it('open A, close, open B; A’s slots answer after B’s: B’s slots are listed', async () => {
    await openReschedule(BOOKING_A);
    await closeReschedule();
    await openReschedule(BOOKING_B);

    await answer(slotsCalls[1], SLOT_B);
    await answer(slotsCalls[0], SLOT_A);

    expect(listed(SLOT_B), 'B slot listed').toBe(true);
    expect(listed(SLOT_A), 'A slot not listed').toBe(false);
  });

  it('open B; B’s slots read fails: no old slots listed, the worded error and Retry show', async () => {
    await openReschedule(BOOKING_A);
    await answer(slotsCalls[0], SLOT_A);
    expect(listed(SLOT_A)).toBe(true);
    await closeReschedule();

    await openReschedule(BOOKING_B);
    await fail(slotsCalls[1]);

    expect(listed(SLOT_A), 'old A slot not listed after B fails').toBe(false);
    const err = dialogError();
    expect(err, 'dialog error shown').not.toBeNull();
    expect(err?.textContent).toContain(th['error.serverError']);
    expect(err?.querySelector('button[data-action="retry"]'), 'Retry shown').not.toBeNull();
  });

  it('edge: re-opening the same booking reads again and shows the fresh list', async () => {
    await openReschedule(BOOKING_A);
    await closeReschedule();
    await openReschedule(BOOKING_A);
    expect(slotsCalls.map((c) => c.branch)).toEqual([BRANCH_A, BRANCH_A]);

    // Fresh read answers first; the first (stale) read lands last.
    await answer(slotsCalls[1], SLOT_A_FRESH);
    await answer(slotsCalls[0], SLOT_A);

    expect(listed(SLOT_A_FRESH), 'fresh slot listed').toBe(true);
    expect(listed(SLOT_A), 'stale slot not listed').toBe(false);
  });

  it('edge: Retry reads the current booking only, and a late answer for the old booking is dropped', async () => {
    await openReschedule(BOOKING_A);
    await closeReschedule();
    await openReschedule(BOOKING_B);
    await fail(slotsCalls[1]);

    const retry = dialogError()?.querySelector('button[data-action="retry"]') as HTMLButtonElement | null;
    expect(retry, 'Retry shown').not.toBeNull();
    retry!.click();
    await flush();

    expect(slotsCalls).toHaveLength(3);
    expect(slotsCalls[2].branch, 'Retry reads B’s branch').toBe(BRANCH_B);

    await answer(slotsCalls[2], SLOT_B);
    await answer(slotsCalls[0], SLOT_A);

    expect(listed(SLOT_B), 'B slot listed').toBe(true);
    expect(listed(SLOT_A), 'late A slot not listed').toBe(false);
    expect(dialogError(), 'no error after B retry succeeded').toBeNull();
  });
});
