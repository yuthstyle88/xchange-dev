// #5711: desk bookings: reschedule confirm button is disabled until a target slot is picked.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingRescheduleOutput,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { withBookingSlots } from './booking-slots.testing';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';
const SLOT_2_ID = '00000000-0000-0000-0000-000000000002';
const SLOT_3_ID = '00000000-0000-0000-0000-000000000003';
const BOOKING_1_ID = 'BK-test-0001';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

interface RecordedRequest {
  method: string;
  path: string;
  rawBody: string | undefined;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen — reschedule confirm disabled until target slot picked (#5711)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let bookingsData: ReturnType<typeof makeDeskBookingsData>;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    vi.stubGlobal(
      'fetch',
      withBookingSlots(vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        const rawBody = typeof init?.body === 'string' ? init.body : undefined;

        const rec: RecordedRequest = {
          method,
          path: u.pathname,
          rawBody,
        };
        sentRequests.push(rec);

        if (method === 'GET' && u.pathname === '/api/v3/views/me') {
          return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          return jsonResponse(200, { version: 1, changed: true, data: bookingsData });
        }
        if (method === 'POST' && u.pathname.endsWith('/reschedule')) {
          return jsonResponse(
            200,
            makeBookingRescheduleOutput({
              id: BOOKING_1_ID,
              slotId: SLOT_2_ID,
              status: 'CONFIRMED',
              bookingNo: 'B261016-0021',
            }),
          );
        }
        return jsonResponse(404, { error: { code: 'notFound', message: u.pathname } });
      })),
    );
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

  async function mountScreen(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
  }

  it("open the reschedule dialog on a booking: the target select is '', and button[data-action=\"confirm-reschedule\"] is disabled", async () => {
    bookingsData = makeDeskBookingsData({
      items: [
        makeDeskBookingItemDto({
          id: BOOKING_1_ID,
          bookingNo: 'B261016-0021',
          status: 'CONFIRMED',
          customerDisplayName: 'Alice Customer',
          phoneLast4: '4567',
          readiness: 'ready',
          ticketNumber: null,
          slotId: SLOT_1_ID,
          startAt: '2026-10-16T09:00:00Z',
          endAt: '2026-10-16T09:30:00Z',
        }),
      ],
    });

    await mountScreen();

    const rescheduleBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="reschedule"]`,
    )!;
    rescheduleBtn.click();
    await flush();

    const targetSelect = host.querySelector<HTMLSelectElement>('select[name="targetSlotId"]')!;
    expect(targetSelect.value).toBe('');

    const confirmBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-reschedule"]')!;
    expect(confirmBtn.disabled).toBe(true);
  });

  it('pick an open slot: the button is enabled, a click sends one POST /api/bookings/{id}/reschedule with that targetSlotId', async () => {
    bookingsData = makeDeskBookingsData({
      items: [
        makeDeskBookingItemDto({
          id: BOOKING_1_ID,
          bookingNo: 'B261016-0021',
          status: 'CONFIRMED',
          customerDisplayName: 'Alice Customer',
          phoneLast4: '4567',
          readiness: 'ready',
          ticketNumber: null,
          slotId: SLOT_1_ID,
          startAt: '2026-10-16T09:00:00Z',
          endAt: '2026-10-16T09:30:00Z',
        }),
        makeDeskBookingItemDto({
          id: 'BK-test-0002',
          bookingNo: 'B261016-0022',
          status: 'CONFIRMED',
          customerDisplayName: 'Bob Customer',
          phoneLast4: '8901',
          readiness: 'ready',
          ticketNumber: null,
          slotId: SLOT_2_ID,
          startAt: '2026-10-16T09:30:00Z',
          endAt: '2026-10-16T10:00:00Z',
        }),
        makeDeskBookingItemDto({
          id: 'BK-test-0003',
          bookingNo: 'B261016-0023',
          status: 'CONFIRMED',
          customerDisplayName: 'Carol Customer',
          phoneLast4: '2345',
          readiness: 'ready',
          ticketNumber: null,
          slotId: SLOT_3_ID,
          startAt: '2026-10-16T10:00:00Z',
          endAt: '2026-10-16T10:30:00Z',
        }),
      ],
    });

    await mountScreen();

    const rescheduleBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="reschedule"]`,
    )!;
    rescheduleBtn.click();
    await flush();

    const targetSelect = host.querySelector<HTMLSelectElement>('select[name="targetSlotId"]')!;
    const confirmBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-reschedule"]')!;

    targetSelect.value = SLOT_3_ID;
    targetSelect.dispatchEvent(new Event('input', { bubbles: true }));
    targetSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    expect(confirmBtn.disabled).toBe(false);

    confirmBtn.click();
    await flush();

    const rescheduleReqs = sentRequests.filter(
      (r) => r.path === `/api/bookings/${BOOKING_1_ID}/reschedule`,
    );
    expect(rescheduleReqs).toHaveLength(1);
    expect(rescheduleReqs[0].method).toBe('POST');
    expect(JSON.parse(rescheduleReqs[0].rawBody!)).toEqual({
      slotId: SLOT_3_ID,
      branch: BRANCH_ID,
    });
  });
});
