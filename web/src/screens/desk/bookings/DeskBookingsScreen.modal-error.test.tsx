// #2592: Desk bookings (B02) — surface Cancel / Reschedule / New booking errors inside the open dialog
// and show reasonRequired when Cancel is pressed with an empty reason (§8, §9).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCancelOutput,
  makeBookingCreateOutput,
  makeBookingRescheduleOutput,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { th } from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';
const SLOT_2_ID = '00000000-0000-0000-0000-000000000002';
const BOOKING_1_ID = 'BK-test-0001';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const INITIAL_DESK_BOOKINGS = makeDeskBookingsData({
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
  ],
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

describe('DeskBookingsScreen (B02) — modal error display & empty cancel reason (#2592 §8, §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let writeResponder: (req: RecordedRequest) => Response | Promise<Response>;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    writeResponder = (req) => {
      if (req.path.endsWith('/cancel')) {
        return jsonResponse(
          200,
          makeBookingCancelOutput({
            id: BOOKING_1_ID,
            status: 'CANCELLED',
            bookingNo: 'B261016-0021',
          }),
        );
      }
      if (req.path.endsWith('/reschedule')) {
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
      if (req.path === '/api/bookings') {
        return jsonResponse(
          200,
          makeBookingCreateOutput({
            id: 'BK-new-001',
            bookingNo: 'B261016-0099',
            status: 'CONFIRMED',
            sourceChannel: 'DESK',
            branchId: BRANCH_ID,
            slotId: SLOT_1_ID,
            customerId: 'cust-new',
            shopSide: 'SELL',
            currency: 'USD',
            quantity: 100,
            qrToken: null,
          }),
        );
      }
      return jsonResponse(404, { error: { code: 'notFound', message: req.path } });
    };

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
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
          return jsonResponse(200, { version: 1, changed: true, data: INITIAL_DESK_BOOKINGS });
        }
        if (method === 'POST') {
          return writeResponder(rec);
        }
        return jsonResponse(404, { error: { code: 'notFound', message: u.pathname } });
      }),
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

  it('row 1: Cancel fails (500) with the dialog open -> the error text and a Retry button are inside the cancel dialog', async () => {
    let attempts = 0;
    writeResponder = () => {
      attempts += 1;
      if (attempts === 1) {
        return jsonResponse(500, {
          error: { code: 'INTERNAL_ERROR', message: 'database timeout' },
        });
      }
      return jsonResponse(
        200,
        makeBookingCancelOutput({
          id: BOOKING_1_ID,
          status: 'CANCELLED',
          bookingNo: 'B261016-0021',
        }),
      );
    };

    await mountScreen();

    const cancelBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="cancel"]`,
    )!;
    cancelBtn.click();
    await flush();

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    reasonInput.value = 'Customer requested cancellation';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const confirmCancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel"]')!;
    confirmCancelBtn.click();
    await flush();

    const modal = host.querySelector<HTMLElement>('.xc-modal');
    expect(modal, 'cancel dialog stays open on failure').not.toBeNull();

    const modalAlert = modal!.querySelector<HTMLElement>('[role="alert"]');
    expect(modalAlert, 'error alert is rendered inside the cancel dialog').not.toBeNull();
    expect(modalAlert!.textContent).toContain(sharedTh['error.serverError']);

    const modalRetryBtn = modal!.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(modalRetryBtn, 'Retry button is rendered inside the cancel dialog').not.toBeNull();
    expect(modalRetryBtn!.textContent?.trim()).toBe(th.retry);

    modalRetryBtn!.click();
    await flush();

    expect(attempts).toBe(2);
    expect(host.querySelector('.xc-modal')).toBeNull();
  });

  it('row 2: Reschedule fails -> error inside the reschedule dialog', async () => {
    let attempts = 0;
    writeResponder = () => {
      attempts += 1;
      if (attempts === 1) {
        return jsonResponse(400, {
          error: { code: 'capacityFull', message: 'Slot capacity full' },
        });
      }
      return jsonResponse(
        200,
        makeBookingRescheduleOutput({
          id: BOOKING_1_ID,
          slotId: SLOT_2_ID,
          status: 'CONFIRMED',
          bookingNo: 'B261016-0021',
        }),
      );
    };

    await mountScreen();

    const rescheduleBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="reschedule"]`,
    )!;
    rescheduleBtn.click();
    await flush();

    const confirmRescheduleBtn = host.querySelector<HTMLButtonElement>(
      'button[data-action="confirm-reschedule"]',
    )!;
    confirmRescheduleBtn.click();
    await flush();

    const modal = host.querySelector<HTMLElement>('.xc-modal');
    expect(modal, 'reschedule dialog stays open on failure').not.toBeNull();

    const modalAlert = modal!.querySelector<HTMLElement>('[role="alert"]');
    expect(modalAlert, 'error alert is rendered inside the reschedule dialog').not.toBeNull();
    expect(modalAlert!.textContent).toContain(th['error.capacityFull']);

    const modalRetryBtn = modal!.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(modalRetryBtn, 'Retry button is rendered inside the reschedule dialog').not.toBeNull();

    modalRetryBtn!.click();
    await flush();

    expect(attempts).toBe(2);
    expect(host.querySelector('.xc-modal')).toBeNull();
  });

  it('row 3: New booking fails -> error inside the create dialog', async () => {
    let attempts = 0;
    writeResponder = () => {
      attempts += 1;
      if (attempts === 1) {
        return jsonResponse(400, {
          error: { code: 'capacityFull', message: 'Slot capacity full' },
        });
      }
      return jsonResponse(
        200,
        makeBookingCreateOutput({
          id: 'BK-new-001',
          bookingNo: 'B261016-0099',
          status: 'CONFIRMED',
          sourceChannel: 'DESK',
          branchId: BRANCH_ID,
          slotId: SLOT_1_ID,
          customerId: 'cust-new',
          shopSide: 'SELL',
          currency: 'USD',
          quantity: 100,
          qrToken: null,
        }),
      );
    };

    await mountScreen();

    const newBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!;
    newBookingBtn.click();
    await flush();

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    const nameInput = host.querySelector<HTMLInputElement>('input[name="displayName"]')!;
    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    nameInput.value = 'Walk-in Guest';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput.value = '+66812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="submit-booking"]')!;
    submitBookingBtn.click();
    await flush();

    const modal = host.querySelector<HTMLElement>('.xc-modal');
    expect(modal, 'create dialog stays open on failure').not.toBeNull();

    const modalAlert = modal!.querySelector<HTMLElement>('[role="alert"]');
    expect(modalAlert, 'error alert is rendered inside the create dialog').not.toBeNull();
    expect(modalAlert!.textContent).toContain(th['error.capacityFull']);

    const modalRetryBtn = modal!.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(modalRetryBtn, 'Retry button is rendered inside the create dialog').not.toBeNull();

    modalRetryBtn!.click();
    await flush();

    expect(attempts).toBe(2);
    expect(host.querySelector('.xc-modal')).toBeNull();
  });

  it('row 4: Cancel pressed with empty reason -> reasonRequired text shown in the dialog; nothing sent', async () => {
    await mountScreen();

    const cancelBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="cancel"]`,
    )!;
    cancelBtn.click();
    await flush();

    const modal = host.querySelector<HTMLElement>('.xc-modal');
    expect(modal).not.toBeNull();
    expect(modal!.querySelector('[role="alert"]')).toBeNull();

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    reasonInput.value = '   ';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const confirmCancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel"]')!;
    confirmCancelBtn.click();
    await flush();

    const cancelPosts = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === `/api/bookings/${BOOKING_1_ID}/cancel`,
    );
    expect(cancelPosts).toHaveLength(0);

    const modalAlert = modal!.querySelector<HTMLElement>('[role="alert"]');
    expect(modalAlert, 'reasonRequired alert is shown inside the cancel dialog').not.toBeNull();
    expect(modalAlert!.textContent).toContain(th.reasonRequired);
  });

  it('edge case (§9): typing a reason after the empty-reason message clears it on the next submit', async () => {
    await mountScreen();

    const cancelBtn = host.querySelector<HTMLButtonElement>(
      `[data-booking-id="${BOOKING_1_ID}"] button[data-action="cancel"]`,
    )!;
    cancelBtn.click();
    await flush();

    const modal = host.querySelector<HTMLElement>('.xc-modal')!;
    const confirmCancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel"]')!;

    // 1. Submit with empty reason -> shows reasonRequired in dialog, nothing sent
    confirmCancelBtn.click();
    await flush();

    expect(modal.querySelector('[role="alert"]')?.textContent).toContain(th.reasonRequired);
    expect(sentRequests.filter((r) => r.method === 'POST')).toHaveLength(0);

    // 2. Type a valid reason and submit -> clears message and sends cancel POST
    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    reasonInput.value = 'Customer changed plans';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    confirmCancelBtn.click();
    await flush();

    const cancelPosts = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === `/api/bookings/${BOOKING_1_ID}/cancel`,
    );
    expect(cancelPosts).toHaveLength(1);
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('.xc-modal')).toBeNull();
  });
});
