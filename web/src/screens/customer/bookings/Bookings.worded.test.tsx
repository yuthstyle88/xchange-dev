import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeBookingCancelOutput,
  makeBookingRescheduleOutput,
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { zh as sharedZh } from '../../../shared/i18n/zh';
import { BookingsScreen } from './Bookings';
import * as table from './i18n';

const BRANCH_ID = 'XC-B01';
const BOOKING_ACTIVE_ID = 'BK-alice-001';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';
const SLOT_RESCHEDULE_ID = '00000000-0000-0000-0000-000000000003';

const ACTIVE_BOOKING = makeMyBookingItemDto({
  id: BOOKING_ACTIVE_ID,
  bookingNo: 'B261014-0024',
  status: 'CONFIRMED',
  branchId: BRANCH_ID,
  slotId: SLOT_1_ID,
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
});

const AVAILABLE_SLOTS = makeBookingSlotsData({
  asOf: '2026-10-14T08:00:00Z',
  slots: [
    makeBookingSlotItemDto({
      id: SLOT_RESCHEDULE_ID,
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

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('Customer My Bookings (C03): wordedError handling (#2194)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let listResponder: () => Response | Promise<Response>;
  let slotsResponder: () => Response | Promise<Response>;
  let cancelResponder: () => Response | Promise<Response>;
  let rescheduleResponder: () => Response | Promise<Response>;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));

    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);

    listResponder = () =>
      viewEnvelope(makeMyBookingsData({ bookings: [{ ...ACTIVE_BOOKING }] }));
    slotsResponder = () => viewEnvelope(AVAILABLE_SLOTS);
    cancelResponder = () =>
      json(
        200,
        makeBookingCancelOutput({
          id: BOOKING_ACTIVE_ID,
          status: 'CANCELLED',
          bookingNo: ACTIVE_BOOKING.bookingNo,
        }),
      );
    rescheduleResponder = () =>
      json(
        200,
        makeBookingRescheduleOutput({
          id: BOOKING_ACTIVE_ID,
          slotId: SLOT_RESCHEDULE_ID,
          status: 'CONFIRMED',
          bookingNo: ACTIVE_BOOKING.bookingNo,
        }),
      );

    fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (u.pathname === '/api/v3/views/my_bookings') {
        return listResponder();
      }
      if (u.pathname === '/api/v3/views/booking_slots') {
        return slotsResponder();
      }
      if (method === 'POST' && /^\/api\/bookings\/[^/]+\/cancel$/.test(u.pathname)) {
        return cancelResponder();
      }
      if (method === 'POST' && /^\/api\/bookings\/[^/]+\/reschedule$/.test(u.pathname)) {
        return rescheduleResponder();
      }
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  async function mountScreen(): Promise<void> {
    dispose?.();
    host.innerHTML = '';
    dispose = render(() => <BookingsScreen fetch={fetchStub} />, host);
    await flush();
  }

  it('row 1: 400 cancelCutoffPassed / staleVersion / capacityFull use the screen table, while a 500 with a screen-table code uses shared serverError', async () => {
    // 400 cancelCutoffPassed -> screen table text
    cancelResponder = () =>
      json(400, {
        error: { code: 'cancelCutoffPassed', message: 'Cutoff passed' },
      });
    await mountScreen();

    const cancelBtn = host.querySelector(
      `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="cancel"]`,
    ) as HTMLButtonElement;
    expect(cancelBtn).not.toBeNull();
    cancelBtn.click();
    await flush();

    const confirmCancelBtn = host.querySelector(
      'button[data-action="confirm-cancel"]',
    ) as HTMLButtonElement;
    expect(confirmCancelBtn).not.toBeNull();
    confirmCancelBtn.click();
    await flush();

    expect(host.textContent).toContain(table.th['error.cancelCutoffPassed']);
    expect(host.textContent).not.toContain('Cutoff passed');

    // Close cancel dialog, then test 409 staleVersion and 400 capacityFull on reschedule
    const closeCancelBtn = host.querySelector(
      '.xc-confirm-dialog .xc-btn-secondary',
    ) as HTMLButtonElement;
    closeCancelBtn.click();
    await flush();

    rescheduleResponder = () =>
      json(409, {
        error: { code: 'staleVersion', message: 'Booking version conflict' },
      });

    const rescheduleBtn = host.querySelector(
      `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement;
    rescheduleBtn.click();
    await flush();

    const slotOption = host.querySelector(
      `[data-slot-id="${SLOT_RESCHEDULE_ID}"]`,
    ) as HTMLElement;
    slotOption.click();
    await flush();

    const confirmRescheduleBtn = host.querySelector(
      'button[data-action="confirm-reschedule"]',
    ) as HTMLButtonElement;
    confirmRescheduleBtn.click();
    await flush();

    expect(host.textContent).toContain(table.th['error.staleVersion']);
    expect(host.textContent).not.toContain('Booking version conflict');

    // 400 capacityFull -> screen table text
    rescheduleResponder = () =>
      json(400, {
        error: { code: 'capacityFull', message: 'Slot capacity full' },
      });
    confirmRescheduleBtn.click();
    await flush();

    expect(host.textContent).toContain(table.th['error.capacityFull']);
    expect(host.textContent).not.toContain('Slot capacity full');

    // 500 carrying a screen-table code (cancelCutoffPassed) must NOT use the screen table
    rescheduleResponder = () =>
      json(500, {
        error: { code: 'cancelCutoffPassed', message: 'Internal failure' },
      });
    confirmRescheduleBtn.click();
    await flush();

    const alertEl = host.querySelector('.xc-dialog-error');
    expect(alertEl?.textContent).toContain(sharedTh['error.serverError']);
    expect(alertEl?.textContent).not.toContain(table.th['error.cancelCutoffPassed']);
    expect(alertEl?.textContent).not.toContain('Internal failure');
  });

  it('row 2: network TypeError on the list read shows shared network text in TH, EN, and ZH, never "Failed to fetch"', async () => {
    listResponder = () => Promise.reject(new TypeError('Failed to fetch'));

    setLang('th');
    await mountScreen();
    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedTh['error.network'],
    );
    expect(host.textContent).not.toContain('Failed to fetch');

    setLang('en');
    await mountScreen();
    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedEn['error.network'],
    );
    expect(host.textContent).not.toContain('Failed to fetch');

    setLang('zh');
    await mountScreen();
    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedZh['error.network'],
    );
    expect(host.textContent).not.toContain('Failed to fetch');
  });

  it('row 3: 500 serverError with message "Failed to load" shows shared server-error text on list, slots, cancel, and reschedule, never the raw message', async () => {
    // 1. List read 500
    listResponder = () =>
      json(500, {
        error: { code: 'serverError', message: 'Failed to load' },
      });
    await mountScreen();

    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(host.textContent).not.toContain('Failed to load');

    // 2. Cancel 500, Slots 500, and Reschedule 500
    listResponder = () =>
      viewEnvelope(makeMyBookingsData({ bookings: [{ ...ACTIVE_BOOKING }] }));
    cancelResponder = () =>
      json(500, {
        error: { code: 'serverError', message: 'Failed to load' },
      });
    await mountScreen();

    const cancelBtn = host.querySelector(
      `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="cancel"]`,
    ) as HTMLButtonElement;
    cancelBtn.click();
    await flush();

    const confirmCancelBtn = host.querySelector(
      'button[data-action="confirm-cancel"]',
    ) as HTMLButtonElement;
    confirmCancelBtn.click();
    await flush();

    expect(host.querySelector('.xc-dialog-error')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(host.textContent).not.toContain('Failed to load');

    const closeCancelBtn = host.querySelector(
      '.xc-confirm-dialog .xc-btn-secondary',
    ) as HTMLButtonElement;
    closeCancelBtn.click();
    await flush();

    // Slots 500
    slotsResponder = () =>
      json(500, {
        error: { code: 'serverError', message: 'Failed to load' },
      });
    const rescheduleBtn = host.querySelector(
      `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement;
    rescheduleBtn.click();
    await flush();

    expect(host.querySelector('.xc-dialog-error')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(host.textContent).not.toContain('Failed to load');

    // Reschedule 500
    const closeRescheduleBtn = host.querySelector(
      '.xc-reschedule-dialog .xc-btn-secondary',
    ) as HTMLButtonElement;
    closeRescheduleBtn.click();
    await flush();

    slotsResponder = () => viewEnvelope(AVAILABLE_SLOTS);
    rescheduleResponder = () =>
      json(500, {
        error: { code: 'serverError', message: 'Failed to load' },
      });
    rescheduleBtn.click();
    await flush();

    const slotOption = host.querySelector(
      `[data-slot-id="${SLOT_RESCHEDULE_ID}"]`,
    ) as HTMLElement;
    slotOption.click();
    await flush();

    const confirmRescheduleBtn = host.querySelector(
      'button[data-action="confirm-reschedule"]',
    ) as HTMLButtonElement;
    confirmRescheduleBtn.click();
    await flush();

    expect(host.querySelector('[role="alert"]')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(host.textContent).not.toContain('Failed to load');
  });

  it('row 4: 502 with an HTML body shows the shared server-error text, never the raw HTML', async () => {
    listResponder = () =>
      new Response('<html><body><h1>502 Bad Gateway</h1></body></html>', {
        status: 502,
        headers: { 'content-type': 'text/html' },
      });
    await mountScreen();

    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(host.textContent).not.toContain('502 Bad Gateway');
    expect(host.textContent).not.toContain('<html>');
  });

  it('row 5: 500 on the list shows shared server-error text, then Retry -> 200 renders the booking list', async () => {
    listResponder = () =>
      json(500, {
        error: { code: 'serverError', message: 'Failed to load' },
      });
    await mountScreen();

    const errBox = host.querySelector('.xc-bookings-error');
    expect(errBox).not.toBeNull();
    expect(errBox?.textContent).toContain(sharedTh['error.serverError']);
    expect(errBox?.textContent).not.toContain('Failed to load');

    listResponder = () =>
      viewEnvelope(makeMyBookingsData({ bookings: [{ ...ACTIVE_BOOKING }] }));
    const retryBtn = host.querySelector(
      'button[data-action="retry"]',
    ) as HTMLButtonElement;
    expect(retryBtn).not.toBeNull();
    retryBtn.click();
    await flush();

    expect(host.querySelector('.xc-bookings-error')).toBeNull();
    expect(host.textContent).toContain('B261014-0024');
  });

  it('edge case: a 4xx with a code not in the screen table uses shared wordedError text, not the raw message', async () => {
    listResponder = () =>
      json(400, {
        error: { code: 'invalidRequest', message: 'Raw invalid request payload' },
      });
    await mountScreen();

    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedTh['error.invalidRequest'],
    );
    expect(host.textContent).not.toContain('Raw invalid request payload');

    // A 4xx with an unknown code whose message is just the bare code falls back to shared serverError
    listResponder = () =>
      json(422, {
        error: { code: 'unmappedBookingRefusal', message: 'unmappedBookingRefusal' },
      });
    await mountScreen();

    expect(host.querySelector('.xc-bookings-error')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    expect(host.textContent).not.toContain('unmappedBookingRefusal');
  });
});
