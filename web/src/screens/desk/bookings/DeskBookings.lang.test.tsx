// #2149: DeskBookingsScreen (B02) language switching across all 18 render sites (§2, §9 rows 1–4).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
  type DeskBookingsData,
  type MeView,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { en, th } from './i18n';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const STAFF_NO_DESK = makeMe({
  id: '00000000-0000-4000-8000-000000000002',
  name: 'Somying Manager',
  roles: [makeMeRole({ role: 'manager', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['RatesRead'],
});

const ONE_BOOKING = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261016-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Alice Customer',
      phoneLast4: '1234',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    }),
  ],
});

function stubBookingsFetch(opts?: {
  staff?: MeView;
  bookings?: DeskBookingsData;
  checkInStatus?: number;
  checkInBody?: unknown;
}) {
  const staff = opts?.staff ?? STAFF_TELLER;
  const bookings = opts?.bookings ?? ONE_BOOKING;
  const checkInStatus = opts?.checkInStatus ?? 400;
  const checkInBody = opts?.checkInBody ?? {
    error: { code: 'checkInWindow', message: 'Outside of valid check-in window' },
  };

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/me') {
      return jsonResponse(200, { version: 1, changed: true, data: staff });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: bookings });
    }
    if (method === 'POST' && u.pathname === '/api/bookings/bk-1/check-in') {
      return jsonResponse(checkInStatus, checkInBody);
    }
    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen (B02) — §9 language switching (#2149)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
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

  it('row 1 (lang = th): renders the Thai texts across toolbar, booking card, dialogs, empty state, and refusal, including after switching back from en to th', async () => {
    setLang('th');
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const qrInput = host.querySelector<HTMLInputElement>('input[data-action="scan-qr"]')!;
    const qrBtn = host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!;
    const newBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!;
    expect(qrInput.placeholder).toBe(th.scanQr);
    expect(qrBtn.textContent?.trim()).toBe(th.scanQr);
    expect(newBookingBtn.textContent?.trim()).toBe(th.newBooking);

    const statusTag = host.querySelector<HTMLElement>('.xc-status-tag')!;
    const checkInBtn = host.querySelector<HTMLButtonElement>('button[data-action="check-in"]')!;
    const cancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="cancel"]')!;
    const rescheduleBtn = host.querySelector<HTMLButtonElement>('button[data-action="reschedule"]')!;
    expect(statusTag.textContent?.trim()).toBe(th['status.CONFIRMED']);
    expect(checkInBtn.textContent?.trim()).toBe(th.checkIn);
    expect(cancelBtn.textContent?.trim()).toBe(th.cancel);
    expect(rescheduleBtn.textContent?.trim()).toBe(th.reschedule);

    checkInBtn.click();
    await flush();
    const retryBtn = host.querySelector<HTMLButtonElement>('button[data-action="retry"]')!;
    expect(retryBtn.textContent?.trim()).toBe(th.retry);

    // Switching to en and back to th updates the reactive screenT() labels back to Thai
    setLang('en');
    expect(qrBtn.textContent?.trim()).toBe(en.scanQr);
    setLang('th');
    expect(qrBtn.textContent?.trim()).toBe(th.scanQr);
  });

  it('row 2 (lang = en): renders the English text at all 18 render sites listed in §2', async () => {
    setLang('en');
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    // Toolbar: scanQr (:447, :458) and newBooking (:477)
    const qrInput = host.querySelector<HTMLInputElement>('input[data-action="scan-qr"]')!;
    const qrBtn = host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!;
    const newBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!;
    expect(qrInput.placeholder).toBe(en.scanQr);
    expect(qrBtn.textContent?.trim()).toBe(en.scanQr);
    expect(newBookingBtn.textContent?.trim()).toBe(en.newBooking);

    // Booking row: status (:509), checkIn (:520), cancel (:528), reschedule (:536)
    const statusTag = host.querySelector<HTMLElement>('.xc-status-tag')!;
    const checkInBtn = host.querySelector<HTMLButtonElement>('button[data-action="check-in"]')!;
    const cancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="cancel"]')!;
    const rescheduleBtn = host.querySelector<HTMLButtonElement>('button[data-action="reschedule"]')!;
    expect(statusTag.textContent?.trim()).toBe(en['status.CONFIRMED']);
    expect(checkInBtn.textContent?.trim()).toBe(en.checkIn);
    expect(cancelBtn.textContent?.trim()).toBe(en.cancel);
    expect(rescheduleBtn.textContent?.trim()).toBe(en.reschedule);

    // Retry button (:435)
    checkInBtn.click();
    await flush();
    const retryBtn = host.querySelector<HTMLButtonElement>('button[data-action="retry"]')!;
    expect(retryBtn.textContent?.trim()).toBe(en.retry);

    // Cancel modal: title (:553), reasonRequired label (:555), placeholder (:562), confirm (:579)
    cancelBtn.click();
    await flush();
    const cancelModalTitle = host.querySelector<HTMLElement>('.xc-modal-title')!;
    const reasonLabel = host.querySelector<HTMLLabelElement>('label[for="cancel-reason-input"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('#cancel-reason-input')!;
    const confirmCancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel"]')!;
    expect(cancelModalTitle.textContent?.trim()).toBe(en.cancel);
    expect(reasonLabel.textContent?.trim()).toBe(en.reasonRequired);
    expect(reasonInput.placeholder).toBe(en.reasonRequired);
    expect(confirmCancelBtn.textContent?.trim()).toBe(en.cancel);
    host.querySelector<HTMLButtonElement>('.xc-modal-actions .xc-btn-secondary')!.click();
    await flush();

    // Reschedule modal: title (:590), confirm (:626)
    rescheduleBtn.click();
    await flush();
    const rescheduleModalTitle = host.querySelector<HTMLElement>('.xc-modal-title')!;
    const confirmRescheduleBtn = host.querySelector<HTMLButtonElement>(
      'button[data-action="confirm-reschedule"]',
    )!;
    expect(rescheduleModalTitle.textContent?.trim()).toBe(en.reschedule);
    expect(confirmRescheduleBtn.textContent?.trim()).toBe(en.reschedule);
    host.querySelector<HTMLButtonElement>('.xc-modal-actions .xc-btn-secondary')!.click();
    await flush();

    // New booking modal: title (:637), submit (:758)
    newBookingBtn.click();
    await flush();
    const newBookingModalTitle = host.querySelector<HTMLElement>('.xc-modal-title')!;
    const submitBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="submit-booking"]')!;
    expect(newBookingModalTitle.textContent?.trim()).toBe(en.newBooking);
    expect(submitBookingBtn.textContent?.trim()).toBe(en.newBooking);

    // Empty state (:485)
    dispose();
    host.innerHTML = '';
    stubBookingsFetch({ bookings: { items: [] } });
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    const emptyState = host.querySelector<HTMLElement>('.xc-empty-state')!;
    expect(emptyState.textContent?.trim()).toBe(en.empty);

    // Role refusal (:419 / :140 / :153)
    dispose();
    host.innerHTML = '';
    stubBookingsFetch({ staff: STAFF_NO_DESK });
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    const refusal = host.querySelector<HTMLElement>('.xc-refusal')!;
    expect(refusal.textContent?.trim()).toBe(en['error.roleNotAllowed']);
  });

  it('row 3 (lang = zh): uses screenT zh lookup with its en fallback instead of raw Thai th', async () => {
    setAudience('customer');
    setLang('zh');
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const qrInput = host.querySelector<HTMLInputElement>('input[data-action="scan-qr"]')!;
    const qrBtn = host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!;
    const newBookingBtn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!;
    const statusTag = host.querySelector<HTMLElement>('.xc-status-tag')!;
    const checkInBtn = host.querySelector<HTMLButtonElement>('button[data-action="check-in"]')!;
    const cancelBtn = host.querySelector<HTMLButtonElement>('button[data-action="cancel"]')!;
    const rescheduleBtn = host.querySelector<HTMLButtonElement>('button[data-action="reschedule"]')!;

    expect(qrInput.placeholder).toBe(en.scanQr);
    expect(qrBtn.textContent?.trim()).toBe(en.scanQr);
    expect(newBookingBtn.textContent?.trim()).toBe(en.newBooking);
    expect(statusTag.textContent?.trim()).toBe(en['status.CONFIRMED']);
    expect(checkInBtn.textContent?.trim()).toBe(en.checkIn);
    expect(cancelBtn.textContent?.trim()).toBe(en.cancel);
    expect(rescheduleBtn.textContent?.trim()).toBe(en.reschedule);
  });

  it('row 4 (unknown status): falls back to statusUnknown in the active language', async () => {
    const unknownBookings = makeDeskBookingsData({
      items: [
        makeDeskBookingItemDto({
          ...ONE_BOOKING.items[0],
          status: 'UNMAPPED_FUTURE_STATUS',
        }),
      ],
    });

    setLang('en');
    stubBookingsFetch({ bookings: unknownBookings });
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(host.querySelector<HTMLElement>('.xc-status-tag')!.textContent?.trim()).toBe(
      en.statusUnknown,
    );

    dispose();
    host.innerHTML = '';
    setLang('th');
    stubBookingsFetch({ bookings: unknownBookings });
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(host.querySelector<HTMLElement>('.xc-status-tag')!.textContent?.trim()).toBe(
      th.statusUnknown,
    );
  });
});
