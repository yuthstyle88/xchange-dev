import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCancelOutput,
  makeBookingCheckInOutput,
  makeBookingCreateOutput,
  makeBookingRescheduleOutput,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeStaffMeView,
  makeStaffRoleDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { en as screenEn, th as screenTh } from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';
const SLOT_2_ID = '00000000-0000-0000-0000-000000000002';
const BOOKING_1_ID = 'BK-test-0001';

const STAFF_TELLER = makeStaffMeView({
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeStaffRoleDto({ role: 'teller', branchId: BRANCH_ID })],
  branchId: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const INITIAL_DESK_BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: BOOKING_1_ID,
      bookingNo: 'B261014-0021',
      status: 'CONFIRMED',
      customerDisplayName: 'Alice Customer',
      phoneLast4: '4567',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    }),
    makeDeskBookingItemDto({
      id: 'BK-test-0002',
      bookingNo: 'B261014-0022',
      status: 'CONFIRMED',
      customerDisplayName: 'Bob Customer',
      phoneLast4: '8901',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_2_ID,
      startAt: '2026-10-14T09:30:00Z',
      endAt: '2026-10-14T10:00:00Z',
    }),
  ],
});

interface RecordedRequest {
  method: string;
  url: string;
  path: string;
  headers: Headers;
  rawBody: string | undefined;
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

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('B02 DeskBookingsScreen shared ApiClient & wordedError (#1891 §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let deskBookingsResponder: () => Response | Promise<Response>;
  let writeResponder: (req: RecordedRequest) => Response | Promise<Response>;

  beforeAll(() => warmModules(() => import('./DeskBookingsScreen')), 60_000);

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    deskBookingsResponder = () => viewEnvelope(INITIAL_DESK_BOOKINGS);
    writeResponder = (req) => {
      if (req.path.endsWith('/check-in')) {
        return json(200, makeBookingCheckInOutput({
          id: BOOKING_1_ID,
          status: 'CHECKED_IN',
          ticketNumber: 'B01-009',
          bookingNo: 'B261014-0021',
        }));
      }
      if (req.path.endsWith('/cancel')) {
        return json(200, makeBookingCancelOutput({
          id: BOOKING_1_ID,
          status: 'CANCELLED',
          bookingNo: 'B261014-0021',
        }));
      }
      if (req.path.endsWith('/reschedule')) {
        return json(200, makeBookingRescheduleOutput({
          id: BOOKING_1_ID,
          slotId: SLOT_2_ID,
          status: 'CONFIRMED',
          bookingNo: 'B261014-0021',
        }));
      }
      if (req.path === '/api/bookings') {
        return json(200, makeBookingCreateOutput({
          id: 'BK-new-001',
          bookingNo: 'B261014-0099',
          status: 'CONFIRMED',
          sourceChannel: 'DESK',
          branchId: BRANCH_ID,
          slotId: SLOT_1_ID,
          customerId: 'cust-new',
          shopSide: 'SELL',
          currency: 'USD',
          quantity: 100,
          qrToken: null,
        }));
      }
      return json(404, { error: { code: 'notFound', message: req.path } });
    };

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        const headers = new Headers(init?.headers);
        const rawBody = typeof init?.body === 'string' ? init.body : undefined;

        const rec: RecordedRequest = {
          method,
          url: rawUrl,
          path: u.pathname,
          headers,
          rawBody,
        };
        sentRequests.push(rec);

        if (u.pathname === '/api/v3/views/staff_me') {
          return viewEnvelope(STAFF_TELLER);
        }
        if (u.pathname === '/api/v3/views/desk_bookings') {
          return deskBookingsResponder();
        }
        if (method === 'POST') {
          return writeResponder(rec);
        }
        return json(404, { error: { code: 'notFound', message: u.pathname } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('staff');
    setLang('th');
  });

  async function mountScreen(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
  }

  it('row 1: check-in, cancel, reschedule, create — each request has x-xchange-request: 1 and an Idempotency-Key', async () => {
    const origResponder = writeResponder;
    writeResponder = (req) => {
      if (req.path.endsWith('/check-in')) {
        return json(200, makeBookingCheckInOutput({
          id: BOOKING_1_ID,
          status: 'CONFIRMED',
          ticketNumber: 'B01-009',
          bookingNo: 'B261014-0021',
        }));
      }
      if (req.path.endsWith('/cancel')) {
        return json(200, makeBookingCancelOutput({
          id: BOOKING_1_ID,
          status: 'CONFIRMED',
          bookingNo: 'B261014-0021',
        }));
      }
      return origResponder(req);
    };

    await mountScreen();

    // 1. Check-in
    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    expect(checkInBtn).not.toBeNull();
    checkInBtn.click();
    await flush();

    // 2. Cancel
    const cancelBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="cancel"]`,
    ) as HTMLButtonElement;
    expect(cancelBtn).not.toBeNull();
    cancelBtn.click();
    await flush();

    const reasonInput = host.querySelector('input[name="reason"]') as HTMLInputElement;
    expect(reasonInput).not.toBeNull();
    reasonInput.value = 'Customer requested cancellation';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const confirmCancelBtn = host.querySelector('button[data-action="confirm-cancel"]') as HTMLButtonElement;
    expect(confirmCancelBtn).not.toBeNull();
    confirmCancelBtn.click();
    await flush();

    // 3. Reschedule
    const rescheduleBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement;
    expect(rescheduleBtn).not.toBeNull();
    rescheduleBtn.click();
    await flush();

    const slotSelect = host.querySelector('select[name="targetSlotId"]') as HTMLSelectElement;
    expect(slotSelect).not.toBeNull();
    slotSelect.value = SLOT_2_ID;
    slotSelect.dispatchEvent(new Event('input', { bubbles: true }));
    slotSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const confirmRescheduleBtn = host.querySelector('button[data-action="confirm-reschedule"]') as HTMLButtonElement;
    expect(confirmRescheduleBtn).not.toBeNull();
    confirmRescheduleBtn.click();
    await flush();

    // 4. Create
    const newBookingBtn = host.querySelector('button[data-action="new-booking"]') as HTMLButtonElement;
    expect(newBookingBtn).not.toBeNull();
    newBookingBtn.click();
    await flush();

    const qtyInput = host.querySelector('input[name="quantity"]') as HTMLInputElement;
    const nameInput = host.querySelector('input[name="displayName"]') as HTMLInputElement;
    const phoneInput = host.querySelector('input[name="phone"]') as HTMLInputElement;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    nameInput.value = 'Walk-in Guest';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput.value = '+66812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBookingBtn = host.querySelector('button[data-action="submit-booking"]') as HTMLButtonElement;
    expect(submitBookingBtn).not.toBeNull();
    submitBookingBtn.click();
    await flush();

    const expectedPaths = [
      `/api/bookings/${BOOKING_1_ID}/check-in`,
      `/api/bookings/${BOOKING_1_ID}/cancel`,
      `/api/bookings/${BOOKING_1_ID}/reschedule`,
      '/api/bookings',
    ];

    for (const path of expectedPaths) {
      const req = sentRequests.find((r) => r.method === 'POST' && r.path === path);
      expect(req, `POST ${path} was sent`).toBeDefined();
      expect(req!.headers.get('x-xchange-request'), `${path} sends x-xchange-request: 1`).toBe('1');
      expect(req!.headers.get('idempotency-key')?.trim(), `${path} sends non-empty Idempotency-Key`).toBeTruthy();
    }
  });

  it('row 2: retry after a network failure reuses the same idempotency key and byte-equal body', async () => {
    let attempts = 0;
    writeResponder = () => {
      attempts++;
      if (attempts === 1) {
        throw new TypeError('Failed to fetch');
      }
      return json(200, makeBookingCancelOutput({
        id: BOOKING_1_ID,
        status: 'CANCELLED',
        bookingNo: 'B261014-0021',
      }));
    };

    await mountScreen();

    const cancelBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="cancel"]`,
    ) as HTMLButtonElement;
    cancelBtn.click();
    await flush();

    const reasonInput = host.querySelector('input[name="reason"]') as HTMLInputElement;
    reasonInput.value = 'Flight delayed';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const confirmCancelBtn = host.querySelector('button[data-action="confirm-cancel"]') as HTMLButtonElement;
    confirmCancelBtn.click();
    await flush();

    const retryBtn = host.querySelector('button[data-action="retry"]') as HTMLButtonElement;
    expect(retryBtn, 'retry button shown after network error').not.toBeNull();
    retryBtn.click();
    await flush();

    const cancelPosts = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === `/api/bookings/${BOOKING_1_ID}/cancel`,
    );
    expect(cancelPosts).toHaveLength(2);
    const [first, second] = cancelPosts;
    expect(first.headers.get('idempotency-key')).toBeTruthy();
    expect(second.headers.get('idempotency-key')).toBe(first.headers.get('idempotency-key'));
    expect(second.rawBody).toBe(first.rawBody);
  });

  it("row 3: a write answering 500 {code:'roleNotAllowedHere'} shows th['error.serverError']", async () => {
    writeResponder = () =>
      json(500, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Role is not allowed on this route',
        },
      });

    await mountScreen();

    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.serverError']);
    expect(banner!.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(banner!.textContent).not.toContain('Role is not allowed on this route');
  });

  it("row 3 (screen key): a write answering 500 {code:'capacityFull'} shows th['error.serverError'] and not screen th['error.capacityFull']", async () => {
    writeResponder = () =>
      json(500, {
        error: {
          code: 'capacityFull',
          message: 'Capacity is full for this slot on the server',
        },
      });

    await mountScreen();

    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.serverError']);
    expect(banner!.textContent).not.toContain(screenTh['error.capacityFull']);
    expect(banner!.textContent).not.toContain('Capacity is full for this slot on the server');
  });

  it('row 4: a write answering 409 with an unknown code and a sentence shows that sentence (W10 (4))', async () => {
    const sentence = 'Booking slot is currently locked by branch supervisor';
    writeResponder = () =>
      json(409, {
        error: {
          code: 'slotLockedBySupervisor',
          message: sentence,
        },
      });

    await mountScreen();

    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sentence);
    expect(banner!.textContent).not.toContain(sharedTh['error.serverError']);
    expect(banner!.textContent).not.toContain(screenTh['error.validationFailed']);
  });

  it('row 5: a read answering roleNotAllowed flips the screen to the not-authorized state', async () => {
    deskBookingsResponder = () =>
      json(403, {
        error: {
          code: 'roleNotAllowed',
          message: 'Desk bookings requires BookingsDesk permission',
        },
      });

    await mountScreen();

    expect(host.querySelector('[data-screen="desk-bookings"]')).toBeNull();
    const refusal = host.querySelector('.xc-refusal[role="alert"]');
    expect(refusal).not.toBeNull();
    expect(refusal!.textContent).toContain(screenTh['error.roleNotAllowed']);
  });

  it("row 6: in language 'en', a write answering 500 {code:'roleNotAllowedHere'} shows en['error.serverError']", async () => {
    setLang('en');
    writeResponder = () =>
      json(500, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Role is not allowed on this route',
        },
      });

    await mountScreen();

    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedEn['error.serverError']);
    expect(banner!.textContent).not.toContain(sharedTh['error.serverError']);
  });

  it("row 6 (screen key): in language 'en', a write answering 500 {code:'capacityFull'} shows en['error.serverError'] and not screen en['error.capacityFull']", async () => {
    setLang('en');
    writeResponder = () =>
      json(500, {
        error: {
          code: 'capacityFull',
          message: 'Capacity is full for this slot on the server',
        },
      });

    await mountScreen();

    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedEn['error.serverError']);
    expect(banner!.textContent).not.toContain(screenEn['error.capacityFull']);
    expect(banner!.textContent).not.toContain(sharedTh['error.serverError']);
  });

  it('row 7: a network TypeError on a write shows error.network', async () => {
    writeResponder = () => {
      throw new TypeError('Failed to fetch');
    };

    await mountScreen();

    const checkInBtn = host.querySelector(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="check-in"]`,
    ) as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const banner = host.querySelector('.xc-error-banner');
    expect(banner).not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.network']);
    expect(banner!.textContent).not.toContain('Failed to fetch');
  });
});
