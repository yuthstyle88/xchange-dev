import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeBookingRescheduleOutput,
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { warmModules } from '../../../shared/testing/warm-modules';

const BRANCH_ID = 'XC-B01';
const BOOKING_ACTIVE_ID = 'BK-alice-001';
const SLOT_FULL_ID = '00000000-0000-0000-0000-0000000000f0';
const SLOT_AVAILABLE_ID = '00000000-0000-0000-0000-0000000000a1';

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
      id: SLOT_FULL_ID,
      start: '2026-10-14T11:00:00Z',
      end: '2026-10-14T11:30:00Z',
      available: 0,
    }),
    makeBookingSlotItemDto({
      id: SLOT_AVAILABLE_ID,
      start: '2026-10-14T11:30:00Z',
      end: '2026-10-14T12:00:00Z',
      available: 2,
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

describe('xchange#2607: full slot cannot be selected in reschedule dialog', () => {
  let root: HTMLDivElement;
  let sentRequests: { method: string; url: string; path: string; headers: Headers; body: any }[];

  const flush = async (n = 10) => {
    for (let i = 0; i < n; i++) await new Promise((r) => setTimeout(r, 0));
  };

  beforeEach(() => {
    vi.resetModules();
    document.body.innerHTML = '';
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
    history.replaceState(null, '', '/app/bookings');
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));

    sentRequests = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        const headers = new Headers(init?.headers);

        let parsedBody: any = null;
        if (init?.body && typeof init.body === 'string') {
          try {
            parsedBody = JSON.parse(init.body);
          } catch {
            parsedBody = init.body;
          }
        }

        sentRequests.push({
          method,
          url: rawUrl,
          path: u.pathname,
          headers,
          body: parsedBody,
        });

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
          return viewEnvelope(AVAILABLE_SLOTS);
        }

        const reschedMatch = u.pathname.match(/^\/api\/bookings\/([^/]+)\/reschedule$/);
        if (method === 'POST' && reschedMatch) {
          return json(
            200,
            makeBookingRescheduleOutput({
              id: reschedMatch[1],
              slotId: parsedBody?.slotId,
              status: 'CONFIRMED',
              bookingNo: 'B261014-0024',
            }),
          );
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

  async function openRescheduleDialog() {
    await import('./main');
    await flush(20);

    const rescheduleBtn = root.querySelector(
      `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement;
    expect(rescheduleBtn, 'reschedule button present').not.toBeNull();
    rescheduleBtn.click();
    await flush(10);
  }

  it('row 1: slot with available: 0 has disabled radio', async () => {
    await openRescheduleDialog();

    const fullSlotOption = root.querySelector(`[data-slot-id="${SLOT_FULL_ID}"]`) as HTMLElement;
    expect(fullSlotOption, 'full slot option rendered').not.toBeNull();

    const fullRadio = fullSlotOption.querySelector('input[type="radio"]') as HTMLInputElement;
    expect(fullRadio, 'full slot radio element exists').not.toBeNull();
    expect(fullRadio.disabled, 'radio must be disabled when available === 0').toBe(true);
  });

  it('row 1: slot with available: 0 clicking row leaves no slot selected and Confirm stays disabled', async () => {
    await openRescheduleDialog();

    const fullSlotOption = root.querySelector(`[data-slot-id="${SLOT_FULL_ID}"]`) as HTMLElement;
    expect(fullSlotOption, 'full slot option rendered').not.toBeNull();

    const fullRadio = fullSlotOption.querySelector('input[type="radio"]') as HTMLInputElement;
    const confirmBtn = root.querySelector('button[data-action="confirm-reschedule"]') as HTMLButtonElement;
    expect(confirmBtn.disabled, 'confirm button disabled initially').toBe(true);

    fullSlotOption.click();
    await flush();

    expect(fullSlotOption.classList.contains('selected'), 'full slot row must not become selected on click').toBe(false);
    expect(fullRadio.checked, 'full slot radio must not be checked on row click').toBe(false);
    expect(confirmBtn.disabled, 'confirm button remains disabled after row click').toBe(true);
  });

  it('row 1: slot with available: 0 clicking radio leaves no slot selected and Confirm stays disabled', async () => {
    await openRescheduleDialog();

    const fullSlotOption = root.querySelector(`[data-slot-id="${SLOT_FULL_ID}"]`) as HTMLElement;
    expect(fullSlotOption, 'full slot option rendered').not.toBeNull();

    const fullRadio = fullSlotOption.querySelector('input[type="radio"]') as HTMLInputElement;
    const confirmBtn = root.querySelector('button[data-action="confirm-reschedule"]') as HTMLButtonElement;
    expect(confirmBtn.disabled, 'confirm button disabled initially').toBe(true);

    fullRadio.click();
    await flush();

    expect(fullSlotOption.classList.contains('selected'), 'full slot row must not become selected on radio click').toBe(false);
    expect(fullRadio.checked, 'full slot radio must not be checked on radio click').toBe(false);
    expect(confirmBtn.disabled, 'confirm button remains disabled after radio click').toBe(true);
  });

  it('row 2: slot with available > 0 is selectable and Confirm posts its id (kept)', async () => {
    await openRescheduleDialog();

    const availableSlotOption = root.querySelector(`[data-slot-id="${SLOT_AVAILABLE_ID}"]`) as HTMLElement;
    expect(availableSlotOption, 'available slot option rendered').not.toBeNull();

    const radio = availableSlotOption.querySelector('input[type="radio"]') as HTMLInputElement;
    expect(radio, 'available slot radio exists').not.toBeNull();
    expect(radio.disabled, 'radio must not be disabled when available > 0').toBe(false);

    const confirmBtn = root.querySelector('button[data-action="confirm-reschedule"]') as HTMLButtonElement;
    expect(confirmBtn, 'confirm button exists').not.toBeNull();
    expect(confirmBtn.disabled, 'confirm button disabled before selection').toBe(true);

    // Click available slot
    availableSlotOption.click();
    await flush();

    expect(availableSlotOption.classList.contains('selected'), 'available slot becomes selected').toBe(true);
    expect(confirmBtn.disabled, 'confirm button enabled after selecting available slot').toBe(false);

    confirmBtn.click();
    await flush(10);

    const reschedReq = sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_ACTIVE_ID}/reschedule`);
    expect(reschedReq, 'sends POST /api/bookings/{id}/reschedule').toBeDefined();
    expect(reschedReq!.body?.slotId).toBe(SLOT_AVAILABLE_ID);
    expect(reschedReq!.body?.branch).toBe(BRANCH_ID);
  });
});
