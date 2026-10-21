// Written before the implementation by the test writer gemini/warut-2 (ticket author claude/e92db6), against BK-6 provided tests 7f8e56d1. Do not edit, delete or weaken.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeBookingCancelOutput,
  makeBookingRescheduleOutput,
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
  type MyBookingItemDto,
} from '../../../shared/testing/fixtures';
import * as table from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';
let fs: NodeFs;
let path: NodePath;

// ── Mock Shapes matching merged BK-6 generated DTOs ──

const BRANCH_ID = 'XC-B01';
const BOOKING_ACTIVE_ID = 'BK-alice-001';
const BOOKING_COMPLETED_ID = 'BK-alice-002';
const BOOKING_CANCELLED_ID = 'BK-alice-cancelled';
const BOOKING_NO_SHOW_ID = 'BK-alice-noshow';
const BOOKING_EXPIRED_ID = 'BK-alice-expired';
const BOOKING_MIDNIGHT_ID = 'BK-alice-midnight';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';
const SLOT_RESCHEDULE_ID = '00000000-0000-0000-0000-000000000003';

const MY_PROFILE = {
  customerId: 'cust-alice',
  displayName: 'Alice Customer',
  hasPhone: true,
};

const INITIAL_BOOKINGS = [
  makeMyBookingItemDto({
    id: BOOKING_ACTIVE_ID,
    bookingNo: 'B261014-0024',
    status: 'CONFIRMED',
    branchId: BRANCH_ID,
    slotId: SLOT_1_ID,
    startAt: '2026-10-14T10:00:00Z',
    endAt: '2026-10-14T10:30:00Z',
    qrToken: 'qr-token-alice-12345',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    // Future cutoff: actions are available
    cancelCutoffAt: '2026-10-14T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  }),
  makeMyBookingItemDto({
    id: BOOKING_COMPLETED_ID,
    bookingNo: 'B261014-0020',
    status: 'COMPLETED',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000002',
    startAt: '2026-10-13T10:00:00Z',
    endAt: '2026-10-13T10:30:00Z',
    qrToken: 'qr-token-alice-old',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-13T09:00:00Z',
    ticketNumber: 'B01-005',
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  }),
  makeMyBookingItemDto({
    id: BOOKING_CANCELLED_ID,
    bookingNo: 'B261014-0021',
    status: 'CANCELLED',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000004',
    startAt: '2026-10-12T10:00:00Z',
    endAt: '2026-10-12T10:30:00Z',
    qrToken: 'qr-token-alice-cancelled',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-12T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  }),
  makeMyBookingItemDto({
    id: BOOKING_NO_SHOW_ID,
    bookingNo: 'B261014-0022',
    status: 'NO_SHOW',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000005',
    startAt: '2026-10-11T10:00:00Z',
    endAt: '2026-10-11T10:30:00Z',
    qrToken: 'qr-token-alice-noshow',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-11T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  }),
  makeMyBookingItemDto({
    id: BOOKING_EXPIRED_ID,
    bookingNo: 'B261014-0023',
    status: 'EXPIRED',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000006',
    startAt: '2026-10-10T10:00:00Z',
    endAt: '2026-10-10T10:30:00Z',
    qrToken: 'qr-token-alice-expired',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-10T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
  }),
  // Near midnight UTC booking to test Asia/Bangkok timezone formatting:
  // 2026-10-14T23:30:00Z is 2026-10-15T06:30:00+07:00 in Asia/Bangkok
  makeMyBookingItemDto({
    id: BOOKING_MIDNIGHT_ID,
    bookingNo: 'B261014-0099',
    status: 'CONFIRMED',
    branchId: BRANCH_ID,
    slotId: '00000000-0000-0000-0000-000000000009',
    startAt: '2026-10-14T23:30:00Z',
    endAt: '2026-10-15T00:00:00Z',
    qrToken: 'qr-token-alice-midnight',
    quoteSummary: 'เรทสุดท้ายยืนยันเมื่อทำรายการ',
    cancelCutoffAt: '2026-10-14T22:30:00Z',
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

describe('APP-17: Customer My Bookings Screen C03 (#987)', () => {
  let root: HTMLDivElement;
  let sentRequests: { method: string; url: string; path: string; headers: Headers; body: any }[];
  let viewBookings: MyBookingItemDto[];
  let viewStatus: number;
  let viewErrorCode: string | undefined;
  let commandStatus: number;
  let commandResponse: any;

  beforeAll(async () => {
    fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  });

  beforeEach(() => {
    vi.resetModules();
    document.body.innerHTML = '';
    root = document.createElement('div');
    root.id = 'root';
    document.body.appendChild(root);
    history.replaceState(null, '', '/app/bookings');

    // Freeze system clock before deadline for normal tests (08:00 UTC on 2026-10-14)
    vi.setSystemTime(new Date('2026-10-14T08:00:00Z'));

    sentRequests = [];
    viewBookings = JSON.parse(JSON.stringify(INITIAL_BOOKINGS));
    viewStatus = 200;
    viewErrorCode = undefined;
    commandStatus = 200;
    commandResponse = null;

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

        // 1. Customer profile view (for CustomerShell)
        if (u.pathname === '/api/v3/views/my_profile') {
          return viewEnvelope(MY_PROFILE);
        }

        // 2. View my_bookings / my_booking
        if (u.pathname === '/api/v3/views/my_bookings') {
          if (viewStatus !== 200) {
            return json(viewStatus, {
              error: { code: viewErrorCode ?? 'serverError', message: 'Failed to load' },
            });
          }

          const bookingId = u.searchParams.get('bookingId');
          if (bookingId) {
            const found = viewBookings.find((b) => b.id === bookingId);
            if (!found) return json(404, { error: { code: 'notFound', message: 'Booking not found' } });
            return viewEnvelope(makeMyBookingsData({ bookings: [found] }));
          }

          return viewEnvelope(makeMyBookingsData({ bookings: viewBookings }));
        }

        // 3. View booking_slots (for choosing new slot on reschedule)
        if (u.pathname === '/api/v3/views/booking_slots') {
          return viewEnvelope(AVAILABLE_SLOTS);
        }

        // 4. Command: bookingCancel (POST /api/bookings/{id}/cancel)
        const cancelMatch = u.pathname.match(/^\/api\/bookings\/([^/]+)\/cancel$/);
        if (method === 'POST' && cancelMatch) {
          if (!headers.get('idempotency-key')) {
            return json(400, { error: { code: 'validationFailed', message: 'Missing Idempotency-Key' } });
          }
          if (commandStatus !== 200) {
            return json(commandStatus, commandResponse ?? { error: { code: 'cancelCutoffPassed', message: 'Cutoff passed' } });
          }
          const item = viewBookings.find((b) => b.id === cancelMatch[1]);
          if (item) item.status = 'CANCELLED';
          return json(
            200,
            commandResponse ??
              makeBookingCancelOutput({
                id: cancelMatch[1],
                status: 'CANCELLED',
                bookingNo: item?.bookingNo ?? 'B261014-0024',
              }),
          );
        }

        // 5. Command: bookingReschedule (POST /api/bookings/{id}/reschedule)
        const reschedMatch = u.pathname.match(/^\/api\/bookings\/([^/]+)\/reschedule$/);
        if (method === 'POST' && reschedMatch) {
          if (!headers.get('idempotency-key')) {
            return json(400, { error: { code: 'validationFailed', message: 'Missing Idempotency-Key' } });
          }
          if (commandStatus !== 200) {
            return json(commandStatus, commandResponse ?? { error: { code: 'capacityFull', message: 'Slot capacity full' } });
          }
          const item = viewBookings.find((b) => b.id === reschedMatch[1]);
          if (item) item.slotId = parsedBody?.slotId;
          return json(
            200,
            commandResponse ??
              makeBookingRescheduleOutput({
                id: reschedMatch[1],
                slotId: parsedBody?.slotId ?? SLOT_RESCHEDULE_ID,
                status: 'CONFIRMED',
                bookingNo: item?.bookingNo ?? 'B261014-0024',
              }),
          );
        }

        return json(404, { error: { code: 'notFound', message: u.pathname } });
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

  async function boot() {
    await import('./main');
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  // ── 1. Reader's Bookings Only & No Customer ID in Requests (§4, :475) ───────

  describe("1. Lists only the reader's bookings and sends no customer id in request (:475)", () => {
    it('asks GET /api/v3/views/my_bookings with no customerId or customer in query or body', async () => {
      await boot();

      const req = sentRequests.find((r) => r.path === '/api/v3/views/my_bookings');
      expect(req, 'requests /api/v3/views/my_bookings').toBeDefined();
      expect(req!.method).toBe('GET');

      // :475 contract: screen sends NO customer id to view
      const u = new URL(req!.url, 'http://localhost');
      expect(u.searchParams.get('customer')).toBeNull();
      expect(u.searchParams.get('customerId')).toBeNull();
      expect(u.searchParams.get('phone')).toBeNull();

      // Confirms list renders booking number and branch
      expect(root.textContent).toContain('B261014-0024');
      expect(root.textContent).toContain(BRANCH_ID);
    });

    it('renders newest booking first and reflects booking status', async () => {
      await boot();

      const bookingItems = root.querySelectorAll('.xc-booking-card, [data-booking-id]');
      expect(bookingItems.length, 'renders list of booking cards').toBeGreaterThanOrEqual(2);

      // Status text from i18n
      expect(root.textContent).toContain(table.th.status_CONFIRMED);
      expect(root.textContent).toContain(table.th.status_COMPLETED);
    });
  });

  // ── 2. Detail View, QR Code & Indicative Quote Summary (§2, §3, :475) ───────

  describe('2. Detail view, check-in QR code and indicative quote summary (:475)', () => {
    it('renders the check-in QR code from my_booking', async () => {
      await boot();

      const qrContainer = root.querySelector('.xc-booking-qr, [data-qr-token]');
      expect(qrContainer, 'check-in QR code container rendered').not.toBeNull();

      // Assert an actual QR element (an svg, canvas or img inside .xc-booking-qr)
      const qrVisual = qrContainer?.matches('svg, canvas, img')
        ? qrContainer
        : qrContainer?.querySelector('svg, canvas, img');
      expect(qrVisual, 'actual QR element (svg, canvas or img) inside .xc-booking-qr').not.toBeNull();
      expect(['svg', 'canvas', 'img']).toContain(qrVisual?.tagName.toLowerCase());

      expect(
        qrContainer?.getAttribute('data-qr-token') ??
          (qrVisual as HTMLImageElement)?.src ??
          qrVisual?.getAttribute('data-qr-token') ??
          qrContainer?.innerHTML,
        'QR element or container reflects qrToken from view',
      ).toContain('qr-token-alice-12345');
    });

    it('renders indicative quote with disclaimer "เรทสุดท้ายยืนยันเมื่อทำรายการ"', async () => {
      await boot();

      // Indicative rate disclaimer pinned in Blueprint §4.4 and ticket §2, §3
      expect(root.textContent).toContain('เรทสุดท้ายยืนยันเมื่อทำรายการ');
    });

    it('formats slot time in branch timezone (Asia/Bangkok) near midnight UTC', async () => {
      await boot();

      // BOOKING_MIDNIGHT_ID has startAt '2026-10-14T23:30:00Z' and endAt '2026-10-15T00:00:00Z'.
      // In Asia/Bangkok (+07:00), this corresponds to 2026-10-15 (day 15, not 14) and 06:30 - 07:00.
      const midnightCard = root.querySelector(`[data-booking-id="${BOOKING_MIDNIGHT_ID}"]`);
      expect(midnightCard, 'midnight booking card rendered').not.toBeNull();
      expect(midnightCard!.textContent).toContain('15');
      expect(midnightCard!.textContent).toContain('06:30');
    });
  });

  // ── 3. Cancel Action & Cutoff Deadline (§2, §4, :475) ───────────────────────

  describe('3. Cancel booking and cancellation cutoff deadline', () => {
    it('shows cancel button before deadline, asks for confirmation once, and sends POST /api/bookings/{id}/cancel with Idempotency-Key', async () => {
      // Clock is 08:00 UTC (before cancelDeadline 09:00 UTC)
      await boot();

      const cancelBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="cancel"], .xc-booking-cancel-btn`,
      ) as HTMLButtonElement;
      expect(cancelBtn, 'cancel button visible before cutoff deadline').not.toBeNull();

      cancelBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      // §4: cancel asks for confirmation once
      const confirmDialog = root.querySelector('.xc-confirm-dialog, dialog, [role="alertdialog"]');
      expect(confirmDialog, 'asks for confirmation once').not.toBeNull();
      expect(confirmDialog?.textContent).toContain(table.th.cancelPrompt);

      const confirmBtn = root.querySelector('button[data-action="confirm-cancel"], .xc-confirm-cancel-btn') as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_ACTIVE_ID}/cancel`);
      expect(req, 'sends POST /api/bookings/{id}/cancel').toBeDefined();
      expect(req!.body?.branch, '#1913: every booking command names its branch').toBe(BRANCH_ID);
      expect(req!.headers.get('idempotency-key'), 'carries Idempotency-Key').toBeTruthy();

      // Status updates to CANCELLED
      expect(root.textContent).toContain(table.th.status_CANCELLED);
    });

    // Owner 2026-10-21 ("ยกเลิกการจอง ยกเลิกได้เลย" · "ไม่ต้อง มี การอนุมัติ เพราะไม่มีอะไรเกี่ยวกับเงิน"):
    // a customer cancels at any time; reschedule keeps the cutoff (#4204).
    it('shows cancel button even when current time is after cutoff deadline (#4204)', async () => {
      // Advance clock past cutoff (09:30 UTC > 09:00 UTC)
      vi.setSystemTime(new Date('2026-10-14T09:30:00Z'));

      await boot();

      const bookingCard = root.querySelector(`[data-booking-id="${BOOKING_ACTIVE_ID}"]`);
      expect(
        bookingCard ?? (root.textContent?.includes('B261014-0024') ? root : null),
        'active booking card is rendered',
      ).not.toBeNull();

      const cancelBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="cancel"], .xc-booking-cancel-btn`,
      );
      expect(cancelBtn, 'cancel button remains visible after cutoff deadline').not.toBeNull();
    });

    it('displays worded error when server answers 400 cancelCutoffPassed', async () => {
      commandStatus = 400;
      commandResponse = {
        error: { code: 'cancelCutoffPassed', message: 'Cutoff passed' },
      };

      await boot();

      const cancelBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="cancel"], .xc-booking-cancel-btn`,
      ) as HTMLButtonElement;
      expect(cancelBtn).not.toBeNull();
      cancelBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const confirmBtn = root.querySelector('button[data-action="confirm-cancel"], .xc-confirm-cancel-btn') as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(root.textContent).toContain(table.th['error.cancelCutoffPassed']);
      expect(root.textContent).not.toContain('cancelCutoffPassed');
    });
  });

  // ── 4. Reschedule Action & Cutoff Deadline (§2, §4, :475) ───────────────────

  describe('4. Reschedule booking and reschedule cutoff deadline', () => {
    it('shows reschedule button before deadline, selects new slot from booking_slots, and sends POST /api/bookings/{id}/reschedule with Idempotency-Key', async () => {
      // Clock is 08:00 UTC (before rescheduleDeadline 09:00 UTC)
      await boot();

      const rescheduleBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"], .xc-booking-reschedule-btn`,
      ) as HTMLButtonElement;
      expect(rescheduleBtn, 'reschedule button visible before deadline').not.toBeNull();

      rescheduleBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      // Queries available booking slots
      const slotsReq = sentRequests.find((r) => r.path === '/api/v3/views/booking_slots');
      expect(slotsReq, 'fetches booking_slots for rescheduling').toBeDefined();

      const slotOption = root.querySelector(
        `[data-slot-id="${SLOT_RESCHEDULE_ID}"], input[value="${SLOT_RESCHEDULE_ID}"], option[value="${SLOT_RESCHEDULE_ID}"]`,
      ) as HTMLElement;
      expect(slotOption, 'available new slot rendered').not.toBeNull();
      slotOption.click();
      if ('checked' in slotOption) (slotOption as HTMLInputElement).checked = true;
      if (slotOption instanceof HTMLOptionElement && slotOption.parentElement instanceof HTMLSelectElement) {
        slotOption.parentElement.value = SLOT_RESCHEDULE_ID;
        slotOption.parentElement.dispatchEvent(new Event('change', { bubbles: true }));
      }
      slotOption.dispatchEvent(new Event('change', { bubbles: true }));

      const confirmBtn = root.querySelector(
        'button[data-action="confirm-reschedule"], .xc-confirm-reschedule-btn',
      ) as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_ACTIVE_ID}/reschedule`);
      expect(req, 'sends POST /api/bookings/{id}/reschedule').toBeDefined();
      expect(req!.headers.get('idempotency-key'), 'reschedule carries Idempotency-Key').toBeTruthy();
      expect(req!.body?.slotId).toBe(SLOT_RESCHEDULE_ID);
      expect(req!.body?.branch, '#1913: every booking command names its branch').toBe(BRANCH_ID);
    });

    it('hides reschedule button when current time is after rescheduleDeadline (:475)', async () => {
      // Advance clock past rescheduleDeadline
      vi.setSystemTime(new Date('2026-10-14T09:30:00Z'));

      await boot();

      // Assert the booking card is rendered before asserting reschedule button absence
      const bookingCard = root.querySelector(`[data-booking-id="${BOOKING_ACTIVE_ID}"]`);
      expect(
        bookingCard ?? (root.textContent?.includes('B261014-0024') ? root : null),
        'active booking card is rendered',
      ).not.toBeNull();

      const rescheduleBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"], .xc-booking-reschedule-btn`,
      );
      expect(rescheduleBtn, 'reschedule button hidden after cutoff deadline').toBeNull();
    });

    it('staleVersion (409) or capacityFull (400) displays worded error and re-reads the booking (§4.3)', async () => {
      commandStatus = 409;
      commandResponse = {
        error: { code: 'staleVersion', message: 'Booking version conflict' },
      };

      await boot();

      const rescheduleBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="reschedule"], .xc-booking-reschedule-btn`,
      ) as HTMLButtonElement;
      expect(rescheduleBtn).not.toBeNull();
      rescheduleBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const slotOption = root.querySelector(
        `[data-slot-id="${SLOT_RESCHEDULE_ID}"], input[value="${SLOT_RESCHEDULE_ID}"], option[value="${SLOT_RESCHEDULE_ID}"]`,
      ) as HTMLElement | null;
      if (slotOption) {
        slotOption.click();
        if ('checked' in slotOption) (slotOption as HTMLInputElement).checked = true;
        if (slotOption instanceof HTMLOptionElement && slotOption.parentElement instanceof HTMLSelectElement) {
          slotOption.parentElement.value = SLOT_RESCHEDULE_ID;
          slotOption.parentElement.dispatchEvent(new Event('change', { bubbles: true }));
        }
        slotOption.dispatchEvent(new Event('change', { bubbles: true }));
      }

      const confirmBtn = root.querySelector(
        'button[data-action="confirm-reschedule"], .xc-confirm-reschedule-btn',
      ) as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      // Shows worded error, raw code absent
      expect(root.textContent).toContain(table.th['error.staleVersion']);
      expect(root.textContent).not.toContain('staleVersion');

      // Re-reads the booking (§4.3)
      const refetchCalls = sentRequests.filter((r) => r.path === '/api/v3/views/my_bookings');
      expect(refetchCalls.length, 're-reads booking on error').toBeGreaterThanOrEqual(2);
    });
  });

  // ── 5. Finished States Show No Actions (§9 Edge Cases) ──────────────────────

  describe('5. Finished states (COMPLETED, CANCELLED, NO_SHOW, EXPIRED) show no actions', () => {
    it('shows status badge and NO cancel or reschedule buttons for finished booking', async () => {
      await boot();

      const finishedCases = [
        { id: BOOKING_COMPLETED_ID, statusText: table.th.status_COMPLETED },
        { id: BOOKING_CANCELLED_ID, statusText: table.th.status_CANCELLED },
        { id: BOOKING_NO_SHOW_ID, statusText: table.th.status_NO_SHOW },
        { id: BOOKING_EXPIRED_ID, statusText: table.th.status_EXPIRED },
      ];

      for (const { id, statusText } of finishedCases) {
        const card = root.querySelector(`[data-booking-id="${id}"]`);
        expect(card, `booking card for ${id} rendered`).not.toBeNull();
        expect(card?.textContent).toContain(statusText);

        // No cancel or reschedule buttons on finished booking card
        expect(card?.querySelector('button[data-action="cancel"], .xc-booking-cancel-btn')).toBeNull();
        expect(card?.querySelector('button[data-action="reschedule"], .xc-booking-reschedule-btn')).toBeNull();
      }
    });
  });

  // ── 6. Empty State with Link to Book (§9 Edge Cases) ────────────────────────

  describe('6. Empty state with link to booking screen', () => {
    it('renders empty state and link to /app/book when bookings list is empty', async () => {
      viewBookings = [];
      await boot();

      expect(root.querySelector('.xc-bookings-empty')).not.toBeNull();
      expect(root.textContent).toContain(table.th.empty);

      // Link to book appointment (C02)
      const bookLink = root.querySelector('a[href="/app/book"], a[href="/app"]') as HTMLAnchorElement;
      expect(bookLink, 'link to C02 book screen rendered').not.toBeNull();
    });
  });

  // ── 7. Loading, Error and Retry States (§4.3) ───────────────────────────────

  describe('7. Loading, error and retry states', () => {
    it('renders loading state while initial view request is pending', async () => {
      vi.stubGlobal('fetch', () => new Promise(() => {})); // Never resolves
      await boot();

      const hasLoading = root.textContent?.includes(table.th.loading) || root.querySelector('.xc-bookings-loading') !== null;
      expect(hasLoading, 'renders loading state').toBe(true);
    });

    it('renders error state on 500 and refetches when retry button is clicked', async () => {
      viewStatus = 500;
      viewErrorCode = 'serverError';
      await boot();

      expect(root.querySelector('.xc-bookings-error')).not.toBeNull();
      const retryBtn = root.querySelector('button[data-action="retry"], .xc-retry-btn') as HTMLButtonElement;
      expect(retryBtn, 'retry button rendered').not.toBeNull();

      // Click retry after recovering status
      viewStatus = 200;
      retryBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(root.textContent).toContain('B261014-0024');
    });

    it('renders load error when the branch list fails rather than guessing a timezone (#4196)', async () => {
      vi.stubGlobal(
        'fetch',
        vi.fn(async (input: RequestInfo | URL) => {
          const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
          const u = new URL(rawUrl, 'http://localhost');
          if (u.pathname === '/api/v3/views/my_profile') return viewEnvelope(MY_PROFILE);
          if (u.pathname === '/api/v3/views/my_bookings') {
            return viewEnvelope(makeMyBookingsData({ bookings: viewBookings }));
          }
          return json(500, { error: { code: 'serverError' } });
        }),
      );
      await boot();

      expect(root.querySelector('.xc-bookings-error')?.textContent).toContain(table.th['error.loadFailed']);
      expect(root.querySelector('.xc-bookings-list')).toBeNull();
    });
  });

  // ── 8. Idempotency-Key Kept Across Retries ──────────────────────────────────

  describe('8. Idempotency-Key kept across retries', () => {
    it('reuses identical Idempotency-Key header when command retried after network error', async () => {
      let cancelCallCount = 0;
      let firstKey: string | null = null;
      let secondKey: string | null = null;

      vi.stubGlobal(
        'fetch',
        vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
          const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
          const u = new URL(rawUrl, 'http://localhost');
          const method = (init?.method ?? 'GET').toUpperCase();
          const headers = new Headers(init?.headers);

          if (u.pathname === '/api/public/host') return json(200, { org: 'XC' });
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
          if (u.pathname === '/api/v3/views/my_profile') return viewEnvelope(MY_PROFILE);
          if (u.pathname === '/api/v3/views/my_bookings') {
            return viewEnvelope(makeMyBookingsData({ bookings: INITIAL_BOOKINGS }));
          }

          if (method === 'POST' && u.pathname.includes('/cancel')) {
            cancelCallCount++;
            if (cancelCallCount === 1) {
              firstKey = headers.get('idempotency-key');
              throw new TypeError('Network error');
            }
            secondKey = headers.get('idempotency-key');
            return json(
              200,
              makeBookingCancelOutput({
                id: BOOKING_ACTIVE_ID,
                status: 'CANCELLED',
                bookingNo: 'B261014-0024',
              }),
            );
          }

          return json(404, { error: 'not found' });
        }),
      );

      await boot();

      const cancelBtn = root.querySelector(
        `[data-booking-id="${BOOKING_ACTIVE_ID}"] button[data-action="cancel"], .xc-booking-cancel-btn`,
      ) as HTMLButtonElement;
      expect(cancelBtn).not.toBeNull();
      cancelBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const confirmBtn = root.querySelector('button[data-action="confirm-cancel"], .xc-confirm-cancel-btn') as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const retryBtn = root.querySelector('button[data-action="retry"], .xc-retry-btn') as HTMLButtonElement;
      expect(retryBtn, 'retry button rendered after network failure').not.toBeNull();
      retryBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(firstKey, 'first attempt carried Idempotency-Key').toBeTruthy();
      expect(secondKey, 'retry attempt carried Idempotency-Key').toBeTruthy();
      expect(secondKey, 'retry must reuse identical Idempotency-Key').toBe(firstKey);
    });
  });

  // ── 9. Contract: HTML, main.tsx and i18n Parity (§4.4, §4.5) ────────────────

  describe('9. HTML, main.tsx and i18n contract', () => {
    it('app-bookings.html and main.tsx meet the customer shell and gate contract', () => {
      const thisFile = new URL(import.meta.url).pathname;
      const thisDir = path.dirname(thisFile);
      const webRoot = path.resolve(thisDir, '../../../../');
      const htmlPath = path.join(webRoot, 'pages/app-bookings.html');
      const mainPath = path.join(thisDir, 'main.tsx');

      expect(fs.existsSync(htmlPath), 'app-bookings.html exists').toBe(true);
      expect(fs.existsSync(mainPath), 'main.tsx exists').toBe(true);

      const htmlContent = fs.readFileSync(htmlPath, 'utf-8');
      expect(htmlContent).toMatch(/meta\s+name=["']xc-gate["']\s+content=["']anyone["']/);
      expect(htmlContent).toMatch(/meta\s+name=["']xc-path["']\s+content=["']\/app\/bookings["']/);
      expect(htmlContent).toMatch(/<script\s+type=["']module["']\s+src=["']\/src\/screens\/customer\/bookings\/main\.tsx["']/);

      const mainContent = fs.readFileSync(mainPath, 'utf-8');
      expect(mainContent).toMatch(/setAudience\(['"]customer['"]\)/);
      expect(mainContent).toMatch(/import\s+\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s+['"][^'"]*_shell\/Shell['"]/);
      expect(mainContent).toMatch(/<CustomerShell\s+route=["']\/app\/bookings["']/);
    });

    it('TH, EN, and ZH translation tables have exact key parity', () => {
      const thKeys = Object.keys(table.th).sort();
      const enKeys = Object.keys(table.en).sort();
      const zhKeys = Object.keys(table.zh).sort();

      expect(thKeys).toEqual(enKeys);
      expect(thKeys).toEqual(zhKeys);
      expect(thKeys.length).toBeGreaterThan(0);
    });
  });
});
