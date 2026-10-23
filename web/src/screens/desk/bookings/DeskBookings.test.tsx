// Written before the implementation by the test writer gemini/warut-2 (ticket author claude/e92db6), against BK-6 provided tests 7f8e56d1. Do not edit, delete or weaken.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  type DeskBookingsData,
  makeBookingCancelOutput,
  makeBookingCheckInOutput,
  makeBookingCreateOutput,
  makeBookingRescheduleOutput,
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
  type MeView,
} from '../../../shared/testing/fixtures';
import { warmModules } from '../../../shared/testing/warm-modules';
import * as texts from './i18n';

// ── Mock data matching merged BK-6 generated DTOs ──

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';
const SLOT_2_ID = '00000000-0000-0000-0000-000000000002';
const BOOKING_1_ID = 'BK-test-0001';
const BOOKING_2_ID = 'BK-test-0002';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const STAFF_MANAGER = makeMe({
  id: '00000000-0000-4000-8000-000000000002',
  name: 'Somying Manager',
  roles: [makeMeRole({ role: 'manager', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BranchesManage', 'OverviewRead'], // lacks BookingsDesk
});

const OWNER_WITH_DESK = makeMe({
  id: '00000000-0000-4000-8000-000000000003',
  name: 'Owner User',
  roles: [makeMeRole({ role: 'owner', branchId: null })],
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
      id: BOOKING_2_ID,
      bookingNo: 'B261014-0022',
      status: 'CHECKED_IN',
      customerDisplayName: 'Bob Customer',
      phoneLast4: '8901',
      readiness: 'unknown',
      ticketNumber: 'B01-001',
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    }),
    makeDeskBookingItemDto({
      id: 'BK-test-0003',
      bookingNo: 'B261014-0023',
      status: 'CONFIRMED',
      customerDisplayName: 'Carol Customer',
      phoneLast4: '3456',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_2_ID,
      startAt: '2026-10-14T09:30:00Z',
      endAt: '2026-10-14T10:00:00Z',
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

describe('BK-8: B02 Teller Desk Bookings Screen (#1409)', () => {
  let host: HTMLDivElement;
  let sentRequests: { method: string; url: string; path: string; headers: Headers; body: any }[];
  let currentStaff: MeView;
  let bookingsData: DeskBookingsData;
  let bookingsVersion: number;
  let bookingsChanged: boolean;
  let commandStatus: number;
  let commandResponse: any;

  beforeAll(
    () =>
      warmModules(
        () => import('./main'),
        () => import('./main.tsx?raw'),
        () => import('./DeskBookingsScreen.tsx?raw'),
      ),
    60_000,
  );

  beforeEach(() => {
    vi.resetModules();
    document.body.innerHTML = '';
    host = document.createElement('div');
    host.id = 'app';
    document.body.appendChild(host);
    history.replaceState(null, '', '/desk/bookings');

    sentRequests = [];
    currentStaff = STAFF_TELLER;
    bookingsData = JSON.parse(JSON.stringify(INITIAL_DESK_BOOKINGS));
    bookingsVersion = 1;
    bookingsChanged = true;
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

        // 1. Session / Me (staff_me returns 401 employeeAuthRequired as on real server for owner)
        if (u.pathname === '/api/v3/views/me') {
          return viewEnvelope(currentStaff);
        }
        if (u.pathname === '/api/v3/views/staff_me') {
          return json(401, {
            error: {
              code: 'employeeAuthRequired',
              message: 'Employee authentication required',
            },
          });
        }

        // 2. View desk_bookings
        if (u.pathname === '/api/v3/views/desk_bookings') {
          if (!currentStaff.permissions.includes('BookingsDesk')) {
            return json(400, {
              error: {
                code: 'roleNotAllowed',
                message: 'Desk bookings requires BookingsDesk permission',
              },
            });
          }
          const roleBranch = currentStaff.roles.find((r) => r.branchId)?.branchId ?? null;
          const requestedBranch = u.searchParams.get('branch') ?? roleBranch;
          if (!requestedBranch) {
            return json(400, {
              error: {
                code: 'validationFailed',
                message: 'branch is required when caller has no assigned branch',
              },
            });
          }

          const since = u.searchParams.get('since');
          if (since && Number(since) === bookingsVersion && !bookingsChanged) {
            return json(200, { version: bookingsVersion, changed: false });
          }

          const qr = u.searchParams.get('qr');
          if (qr) {
            const filtered: DeskBookingsData = JSON.parse(JSON.stringify(bookingsData));
            filtered.items = filtered.items.filter((b) => b.id === BOOKING_1_ID);
            return viewEnvelope(filtered, bookingsVersion, true);
          }

          return viewEnvelope(bookingsData, bookingsVersion, true);
        }

        // 3. Command: bookingCreate (POST /api/bookings)
        if (method === 'POST' && u.pathname === '/api/bookings') {
          if (!headers.get('idempotency-key')) {
            return json(400, { error: { code: 'validationFailed', message: 'Missing Idempotency-Key' } });
          }
          if (commandStatus !== 200) {
            return json(commandStatus, commandResponse ?? { error: { code: 'capacityFull', message: 'Slot capacity full' } });
          }
          return json(
            200,
            commandResponse ??
              makeBookingCreateOutput({
                id: 'BK-new-001',
                bookingNo: 'B261014-0099',
                status: 'CONFIRMED',
                sourceChannel: 'DESK',
                branchId: BRANCH_ID,
                slotId: parsedBody?.slotId ?? SLOT_1_ID,
                customerId: 'cust-new',
                shopSide: parsedBody?.shopSide ?? 'SELL',
                currency: parsedBody?.currency ?? 'USD',
                quantity: parsedBody?.quantity ?? 250,
                qrToken: null,
              }),
          );
        }

        // 4. Command: bookingCheckIn (POST /api/bookings/{id}/check-in)
        const checkInMatch = u.pathname.match(/^\/api\/bookings\/([^/]+)\/check-in$/);
        if (method === 'POST' && checkInMatch) {
          if (!headers.get('idempotency-key')) {
            return json(400, { error: { code: 'validationFailed', message: 'Missing Idempotency-Key' } });
          }
          if (commandStatus !== 200) {
            return json(commandStatus, commandResponse ?? { error: { code: 'qrMismatch', message: 'QR mismatch' } });
          }
          return json(
            200,
            commandResponse ??
              makeBookingCheckInOutput({
                id: checkInMatch[1],
                status: 'CHECKED_IN',
                ticketNumber: 'B01-009',
                bookingNo: 'B261014-0021',
              }),
          );
        }

        // 5. Command: bookingCancel (POST /api/bookings/{id}/cancel)
        const cancelMatch = u.pathname.match(/^\/api\/bookings\/([^/]+)\/cancel$/);
        if (method === 'POST' && cancelMatch) {
          if (!headers.get('idempotency-key')) {
            return json(400, { error: { code: 'validationFailed', message: 'Missing Idempotency-Key' } });
          }
          if (!parsedBody?.reason || String(parsedBody.reason).trim() === '') {
            return json(400, { error: { code: 'validationFailed', message: 'Reason required for staff cancellation' } });
          }
          if (commandStatus !== 200) {
            return json(commandStatus, commandResponse ?? { error: { code: 'cancelCutoffPassed', message: 'Cutoff passed' } });
          }
          return json(
            200,
            commandResponse ??
              makeBookingCancelOutput({
                id: cancelMatch[1],
                status: 'CANCELLED',
                bookingNo: 'B261014-0021',
              }),
          );
        }

        // 6. Command: bookingReschedule (POST /api/bookings/{id}/reschedule)
        const reschedMatch = u.pathname.match(/^\/api\/bookings\/([^/]+)\/reschedule$/);
        if (method === 'POST' && reschedMatch) {
          if (!headers.get('idempotency-key')) {
            return json(400, { error: { code: 'validationFailed', message: 'Missing Idempotency-Key' } });
          }
          if (commandStatus !== 200) {
            return json(commandStatus, commandResponse ?? { error: { code: 'capacityFull', message: 'Slot capacity full' } });
          }
          return json(
            200,
            commandResponse ??
              makeBookingRescheduleOutput({
                id: reschedMatch[1],
                slotId: parsedBody?.slotId ?? SLOT_2_ID,
                status: 'CONFIRMED',
                bookingNo: 'B261014-0021',
              }),
          );
        }

        return json(404, { error: { code: 'notFound', message: u.pathname } });
      }),
    );
  });

  afterEach(() => {
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function boot() {
    await import('./main');
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  // ── 1. Permission Gate (§4, §9 Row 2) ──────────────────────────────────────

  describe('1. Permission Gate (BookingsDesk, no role names)', () => {
    it('user with BookingsDesk (TELLER) sees B02 screen inside AppShell', async () => {
      currentStaff = STAFF_TELLER;
      await boot();

      const shell = host.querySelector('.xc-shell');
      expect(shell, 'renders inside AppShell').not.toBeNull();

      // B02 desk bookings container rendered
      const screen = host.querySelector('.xc-desk-bookings, [data-screen="desk-bookings"]');
      expect(screen, 'B02 bookings screen rendered for user with BookingsDesk').not.toBeNull();
      expect(host.textContent).toContain('B261014-0021');
      expect(sentRequests.some((r) => r.path === '/api/v3/views/staff_me')).toBe(false);
      const viewReq = sentRequests.find((r) => r.path === '/api/v3/views/desk_bookings');
      expect(viewReq?.url).toContain(`branch=${BRANCH_ID}`);
    });

    it('owner on /desk/bookings?branch=B with BookingsDesk loads bookings of B with no staff_me request (#3950)', async () => {
      currentStaff = OWNER_WITH_DESK;
      history.replaceState(null, '', '/desk/bookings?branch=XC-B02');
      await boot();

      expect(sentRequests.some((r) => r.path === '/api/v3/views/staff_me')).toBe(false);
      const viewReq = sentRequests.find((r) => r.path === '/api/v3/views/desk_bookings');
      expect(viewReq, 'requests desk_bookings view for branch XC-B02').toBeDefined();
      expect(new URL(viewReq!.url, 'http://localhost').searchParams.get('branch')).toBe('XC-B02');

      const screen = host.querySelector('.xc-desk-bookings, [data-screen="desk-bookings"]');
      expect(screen, 'B02 bookings screen rendered for owner with BookingsDesk').not.toBeNull();
      expect(host.textContent).toContain('B261014-0021');
    });

    it('user without BookingsDesk (BRANCH_MANAGER) does NOT see B02 screen', async () => {
      currentStaff = STAFF_MANAGER;
      await boot();

      const screen = host.querySelector('.xc-desk-bookings, [data-screen="desk-bookings"]');
      expect(screen, 'B02 bookings screen NOT rendered for user without BookingsDesk').toBeNull();

      // Renders worded refusal from i18n, raw roleNotAllowed code absent
      expect(host.textContent).toContain(texts.th['error.roleNotAllowed']);
      expect(host.textContent).not.toContain('roleNotAllowed');
      expect(host.textContent).not.toContain('B261014-0021');
    });

    it('gate uses permission BookingsDesk and names no role string in screen source', async () => {
      const mainSrc = (await import('./main.tsx?raw')).default;
      const screenSrc = (await import('./DeskBookingsScreen.tsx?raw')).default;
      const combined = `${mainSrc}\n${screenSrc}`;

      // No role strings in gate logic
      const roleRegex = /\b(TELLER|BRANCH_MANAGER|FINANCE|FRONT|BACKEND|MANAGER|EXECUTIVE|OWNER)\b/;
      expect(roleRegex.test(combined), 'no role string used in screen logic').toBe(false);
    });
  });

  // ── 2. Day-by-Slot List & Phone Privacy (§2, §4.3, §4.4) ───────────────────

  describe("2. Day-by-slot list over desk_bookings and phone last 4 privacy", () => {
    it('renders slots, booking numbers, customer display name, readiness, ticket number and phone last 4 only', async () => {
      await boot();

      // Check request was made
      const viewReq = sentRequests.find((r) => r.path === '/api/v3/views/desk_bookings');
      expect(viewReq, 'requests desk_bookings view').toBeDefined();
      expect(viewReq!.method).toBe('GET');

      // Assert booking content rendered
      expect(host.textContent).toContain('B261014-0021');
      expect(host.textContent).toContain('Alice Customer');
      expect(host.textContent).toContain('B261014-0022');
      expect(host.textContent).toContain('Bob Customer');
      expect(host.textContent).toContain('B01-001'); // ticket number
      expect(host.textContent).toContain('unknown'); // readiness unknown per BK-6

      // Phone last 4 rendered
      expect(host.textContent).toContain('4567');
      expect(host.textContent).toContain('8901');

      // Privacy: full phone numbers must NEVER appear anywhere in the DOM
      expect(host.textContent).not.toContain('+66891234567');
      expect(host.textContent).not.toContain('0891234567');
      expect(host.textContent).not.toContain('+66819876543');
      expect(host.textContent).not.toContain('0819876543');
    });
  });

  // ── 3. Since Polling (§2, §9 Row 4) ────────────────────────────────────────

  describe('3. since polling ({ changed: false } leaves DOM untouched)', () => {
    it('polls desk_bookings with since version; when changed is false nothing is redrawn', async () => {
      await boot();

      const bookingElBefore = host.querySelector('[data-booking-id="BK-test-0001"], .xc-booking-item');
      expect(bookingElBefore, 'booking element exists').not.toBeNull();

      // Trigger refresh with since unchanged
      bookingsChanged = false;
      const refreshBtn = host.querySelector(
        'button[data-action="refresh"], .xc-bookings-refresh',
      ) as HTMLButtonElement;
      expect(refreshBtn, 'refresh control exists').not.toBeNull();
      refreshBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      // Assert desk_bookings request with since=1 was sent
      const sinceReq = sentRequests.find(
        (r) => r.path === '/api/v3/views/desk_bookings' && r.url.includes('since='),
      );
      expect(sinceReq, 'desk_bookings request with since=1 was sent').toBeDefined();
      expect(sinceReq!.url).toContain('since=1');

      // Assert that booking row is the exact same DOM node after changed:false
      const bookingElAfter = host.querySelector('[data-booking-id="BK-test-0001"], .xc-booking-item');
      expect(bookingElAfter, 'booking row is same DOM node after changed:false').toBe(bookingElBefore);
    });
  });

  // ── 4. QR Lookup & Filter (§2, §4.4) ────────────────────────────────────────

  describe('4. QR scan and lookup', () => {
    it('queries desk_bookings with qr token and shows matched booking', async () => {
      await boot();

      const qrInput = host.querySelector('input[name="qrToken"], input[data-action="scan-qr"], .xc-qr-input') as HTMLInputElement;
      expect(qrInput, 'QR scan/search input exists').not.toBeNull();

      qrInput.value = 'scanned-qr-token-12345';
      qrInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = host.querySelector('button[data-action="lookup-qr"], .xc-qr-submit') as HTMLButtonElement;
      if (submitBtn) {
        submitBtn.click();
      } else {
        qrInput.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
      }

      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const qrReq = sentRequests.find(
        (r) => r.path === '/api/v3/views/desk_bookings' && r.url.includes('qr='),
      );
      expect(qrReq, 'queries desk_bookings with qr parameter').toBeDefined();
      expect(qrReq!.url).toContain('qr=scanned-qr-token-12345');
    });
  });

  // ── 5. Check-In & Ticket Issuance (§2, §4.4, §8) ───────────────────────────

  describe('5. Check-in (bookingCheckIn with Idempotency-Key and ticket issuance)', () => {
    it('sends POST /api/bookings/{id}/check-in with Idempotency-Key, updates status to CHECKED_IN and renders ticket', async () => {
      await boot();

      const checkInBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="check-in"], .xc-checkin-btn',
      ) as HTMLButtonElement;
      expect(checkInBtn, 'check-in button rendered').not.toBeNull();

      checkInBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_1_ID}/check-in`);
      expect(req, 'sends POST /api/bookings/{id}/check-in').toBeDefined();
      expect(req!.method).toBe('POST');

      // [required] Idempotency-Key present
      const idempKey = req!.headers.get('idempotency-key');
      expect(idempKey, 'check-in carries Idempotency-Key').toBeTruthy();

      // Body carries identityChecked or qrToken
      expect(
        req!.body?.identityChecked === true || typeof req!.body?.qrToken === 'string',
        'body carries identityChecked: true or qrToken',
      ).toBe(true);
      expect(req!.body?.branch, '#1913: every booking command names its branch').toBe(BRANCH_ID);

      // DOM updates with CHECKED_IN status and issued ticket number
      expect(
        host.textContent?.includes(texts.th['status.CHECKED_IN']) ||
          host.textContent?.includes('CHECKED_IN'),
        'shows CHECKED_IN status',
      ).toBe(true);
      expect(host.textContent).toContain('B01-009');
    });

    it('refusals (qrMismatch, checkInWindow) display worded errors and absent raw codes', async () => {
      commandStatus = 400;
      commandResponse = {
        error: { code: 'qrMismatch', message: 'QR mismatch' },
      };

      await boot();

      const checkInBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="check-in"], .xc-checkin-btn',
      ) as HTMLButtonElement;
      expect(checkInBtn).not.toBeNull();
      checkInBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(host.textContent).toContain(texts.th['error.qrMismatch']);
      expect(host.textContent).not.toContain('qrMismatch');
    });
  });

  // ── 6. Cancel by Staff with Reason (§2, §4.4, §7 [required]) ───────────────

  describe('6. Cancel by staff requires reason', () => {
    it('submits cancel with reason and Idempotency-Key; empty reason is prevented', async () => {
      await boot();

      const cancelBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="cancel"], .xc-cancel-btn',
      ) as HTMLButtonElement;
      expect(cancelBtn, 'cancel button rendered').not.toBeNull();

      cancelBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const reasonInput = host.querySelector(
        'input[name="reason"], textarea[name="reason"], .xc-cancel-reason',
      ) as HTMLInputElement;
      expect(reasonInput, 'cancellation reason input rendered').not.toBeNull();

      const confirmCancelBtn = host.querySelector(
        'button[data-action="confirm-cancel"], .xc-confirm-cancel',
      ) as HTMLButtonElement;
      expect(confirmCancelBtn, 'confirm cancel button rendered').not.toBeNull();

      // 1. Attempt empty reason submission -> prevented
      reasonInput.value = '   ';
      reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
      confirmCancelBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(
        sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_1_ID}/cancel`),
        'empty reason cancel must not be dispatched',
      ).toBeUndefined();

      // 2. Submit with valid reason
      reasonInput.value = 'Customer called to cancel due to emergency';
      reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
      confirmCancelBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_1_ID}/cancel`);
      expect(req, 'sends POST /api/bookings/{id}/cancel').toBeDefined();
      expect(req!.headers.get('idempotency-key'), 'cancel carries Idempotency-Key').toBeTruthy();
      expect(req!.body).toEqual({ reason: 'Customer called to cancel due to emergency', branch: BRANCH_ID });

      // Booking status updates to CANCELLED
      expect(
        host.textContent?.includes(texts.th['status.CANCELLED']) ||
          host.textContent?.includes('CANCELLED'),
        'shows CANCELLED status',
      ).toBe(true);
    });
  });

  // ── 7. Reschedule to Another Slot (§2, §4.4, §8) ───────────────────────────

  describe('7. Reschedule (bookingReschedule)', () => {
    it('sends POST /api/bookings/{id}/reschedule with target slotId and Idempotency-Key', async () => {
      await boot();

      const rescheduleBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="reschedule"], .xc-reschedule-btn',
      ) as HTMLButtonElement;
      expect(rescheduleBtn, 'reschedule button rendered').not.toBeNull();

      rescheduleBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const slotSelect = host.querySelector('select[name="targetSlotId"]') as HTMLSelectElement;
      expect(slotSelect, 'targetSlotId selection control exists').not.toBeNull();
      slotSelect.value = SLOT_2_ID;
      slotSelect.dispatchEvent(new Event('input', { bubbles: true }));
      slotSelect.dispatchEvent(new Event('change', { bubbles: true }));

      // Trigger reschedule to SLOT_2_ID
      const confirmBtn = host.querySelector(
        'button[data-action="confirm-reschedule"], .xc-confirm-reschedule',
      ) as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.path === `/api/bookings/${BOOKING_1_ID}/reschedule`);
      expect(req, 'sends POST /api/bookings/{id}/reschedule').toBeDefined();
      expect(req!.headers.get('idempotency-key'), 'reschedule carries Idempotency-Key').toBeTruthy();
      expect(req!.body).toEqual({ slotId: SLOT_2_ID, branch: BRANCH_ID });
    });

    it('capacityFull refusal shows worded error and absent raw code', async () => {
      commandStatus = 400;
      commandResponse = {
        error: { code: 'capacityFull', message: 'Slot capacity full' },
      };

      await boot();

      const rescheduleBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="reschedule"], .xc-reschedule-btn',
      ) as HTMLButtonElement;
      expect(rescheduleBtn).not.toBeNull();
      rescheduleBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const slotSelect = host.querySelector('select[name="targetSlotId"]') as HTMLSelectElement;
      if (slotSelect) {
        slotSelect.value = SLOT_2_ID;
        slotSelect.dispatchEvent(new Event('input', { bubbles: true }));
        slotSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }

      const confirmBtn = host.querySelector(
        'button[data-action="confirm-reschedule"], .xc-confirm-reschedule',
      ) as HTMLButtonElement;
      expect(confirmBtn).not.toBeNull();
      confirmBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(host.textContent).toContain(texts.th['error.capacityFull']);
      expect(host.textContent).not.toContain('capacityFull');
    });
  });

  // ── 8. New Booking Form & Referral Code (§4, §9 Row 1) ─────────────────────

  describe('8. New booking form with optional referralCode field', () => {
    function fillRequiredBookingFields(container: HTMLElement) {
      // Optional branchId input if present in form
      const branchEl = container.querySelector(
        'select[name="branchId"], input[name="branchId"]',
      ) as HTMLSelectElement | HTMLInputElement | null;
      if (branchEl) {
        branchEl.value = BRANCH_ID;
        branchEl.dispatchEvent(new Event('input', { bubbles: true }));
        branchEl.dispatchEvent(new Event('change', { bubbles: true }));
      }

      // Slot
      const slotEl = container.querySelector(
        'select[name="slotId"], input[name="slotId"]',
      ) as HTMLSelectElement | HTMLInputElement | null;
      expect(slotEl, 'slotId field exists in new booking form').not.toBeNull();
      slotEl!.value = SLOT_1_ID;
      slotEl!.dispatchEvent(new Event('input', { bubbles: true }));
      slotEl!.dispatchEvent(new Event('change', { bubbles: true }));

      // Side
      const sideEl = container.querySelector(
        'select[name="shopSide"], input[name="shopSide"]',
      ) as HTMLSelectElement | HTMLInputElement | null;
      expect(sideEl, 'shopSide field exists in new booking form').not.toBeNull();
      sideEl!.value = 'SELL';
      sideEl!.dispatchEvent(new Event('input', { bubbles: true }));
      sideEl!.dispatchEvent(new Event('change', { bubbles: true }));

      // Currency
      const currencyEl = container.querySelector(
        'input[name="currency"], select[name="currency"]',
      ) as HTMLInputElement | HTMLSelectElement | null;
      expect(currencyEl, 'currency field exists in new booking form').not.toBeNull();
      currencyEl!.value = 'USD';
      currencyEl!.dispatchEvent(new Event('input', { bubbles: true }));
      currencyEl!.dispatchEvent(new Event('change', { bubbles: true }));

      // Quantity
      const quantityEl = container.querySelector(
        'input[name="quantity"]',
      ) as HTMLInputElement | null;
      expect(quantityEl, 'quantity field exists in new booking form').not.toBeNull();
      quantityEl!.value = '250';
      quantityEl!.dispatchEvent(new Event('input', { bubbles: true }));
      quantityEl!.dispatchEvent(new Event('change', { bubbles: true }));

      // Customer displayName
      const nameEl = container.querySelector(
        'input[name="displayName"]',
      ) as HTMLInputElement | null;
      expect(nameEl, 'displayName field exists in new booking form').not.toBeNull();
      nameEl!.value = 'Charlie Customer';
      nameEl!.dispatchEvent(new Event('input', { bubbles: true }));
      nameEl!.dispatchEvent(new Event('change', { bubbles: true }));

      // Customer phone
      const phoneEl = container.querySelector(
        'input[name="phone"]',
      ) as HTMLInputElement | null;
      expect(phoneEl, 'phone field exists in new booking form').not.toBeNull();
      phoneEl!.value = '+66812345678';
      phoneEl!.dispatchEvent(new Event('input', { bubbles: true }));
      phoneEl!.dispatchEvent(new Event('change', { bubbles: true }));
    }

    it('sends referralCode key when referral field is filled', async () => {
      await boot();

      const newBookingBtn = host.querySelector(
        'button[data-action="new-booking"], .xc-new-booking-btn',
      ) as HTMLButtonElement;
      expect(newBookingBtn, 'new booking button rendered').not.toBeNull();
      newBookingBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      fillRequiredBookingFields(host);

      const referralInput = host.querySelector('input[name="referralCode"], .xc-referral-input') as HTMLInputElement;
      expect(referralInput, 'referralCode input exists in new booking form').not.toBeNull();

      referralInput.value = 'BROKER789';
      referralInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = host.querySelector(
        'button[type="submit"], button[data-action="submit-booking"], .xc-submit-booking',
      ) as HTMLButtonElement;
      expect(submitBtn, 'submit new booking button rendered').not.toBeNull();
      submitBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.method === 'POST' && r.path === '/api/bookings');
      expect(req, 'sends POST /api/bookings').toBeDefined();
      expect(req!.headers.get('idempotency-key'), 'bookingCreate carries Idempotency-Key').toBeTruthy();
      expect(req!.body).toEqual({
        branch: BRANCH_ID,
        branchId: BRANCH_ID,
        slotId: SLOT_1_ID,
        shopSide: 'SELL',
        currency: 'USD',
        quantity: 250,
        displayName: 'Charlie Customer',
        phone: '+66812345678',
        referralCode: 'BROKER789',
      });
    });

    it('omits referralCode key when referral field is empty or whitespace', async () => {
      await boot();

      const newBookingBtn = host.querySelector(
        'button[data-action="new-booking"], .xc-new-booking-btn',
      ) as HTMLButtonElement;
      expect(newBookingBtn).not.toBeNull();
      newBookingBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      fillRequiredBookingFields(host);

      const referralInput = host.querySelector('input[name="referralCode"], .xc-referral-input') as HTMLInputElement;
      expect(referralInput).not.toBeNull();

      referralInput.value = '   ';
      referralInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = host.querySelector(
        'button[type="submit"], button[data-action="submit-booking"], .xc-submit-booking',
      ) as HTMLButtonElement;
      expect(submitBtn).not.toBeNull();
      submitBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const req = sentRequests.find((r) => r.method === 'POST' && r.path === '/api/bookings');
      expect(req, 'sends POST /api/bookings').toBeDefined();
      expect(req!.headers.get('idempotency-key'), 'bookingCreate carries Idempotency-Key').toBeTruthy();

      expect(req!.body).toEqual({
        branch: BRANCH_ID,
        branchId: BRANCH_ID,
        slotId: SLOT_1_ID,
        shopSide: 'SELL',
        currency: 'USD',
        quantity: 250,
        displayName: 'Charlie Customer',
        phone: '+66812345678',
      });
      // [required] referralCode key must be omitted completely when empty
      expect('referralCode' in (req!.body ?? {}), 'referralCode key must NOT be present in body').toBe(false);
    });
  });

  // ── 9. Idempotency-Key Reused Across Retries (§4, §9 Row 3) ────────────────

  describe('9. Idempotency-Key kept across retries', () => {
    it('keeps identical Idempotency-Key header when retrying after network failure', async () => {
      let callCount = 0;
      let firstKey: string | null = null;
      let secondKey: string | null = null;

      vi.stubGlobal(
        'fetch',
        vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
          const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
          const u = new URL(rawUrl, 'http://localhost');
          const method = (init?.method ?? 'GET').toUpperCase();
          const headers = new Headers(init?.headers);

          if (u.pathname === '/api/v3/views/me') return viewEnvelope(STAFF_TELLER);
          if (u.pathname === '/api/v3/views/desk_bookings') return viewEnvelope(INITIAL_DESK_BOOKINGS);

          if (method === 'POST' && u.pathname.includes('/check-in')) {
            callCount++;
            if (callCount === 1) {
              firstKey = headers.get('idempotency-key');
              throw new TypeError('Network error');
            }
            secondKey = headers.get('idempotency-key');
            return json(200, { id: BOOKING_1_ID, status: 'CHECKED_IN', ticketNumber: 'B01-010' });
          }

          return json(404, { error: 'not found' });
        }),
      );

      await boot();

      const checkInBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="check-in"], .xc-checkin-btn',
      ) as HTMLButtonElement;
      expect(checkInBtn).not.toBeNull();
      checkInBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      const retryBtn = host.querySelector('button[data-action="retry"], .xc-retry-btn') as HTMLButtonElement;
      expect(retryBtn, 'retry button rendered after network failure').not.toBeNull();
      retryBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(firstKey, 'first attempt carried Idempotency-Key').toBeTruthy();
      expect(secondKey, 'retry attempt carried Idempotency-Key').toBeTruthy();
      expect(secondKey, 'retry must reuse identical Idempotency-Key').toBe(firstKey);
    });
  });

  // ── 10. Refusal Codes & Worded Errors (§8) ──────────────────────────────────

  describe('10. Worded errors (staleVersion, queueClosed)', () => {
    it('staleVersion (409) displays worded error from i18n and absent raw code', async () => {
      commandStatus = 409;
      commandResponse = {
        error: { code: 'staleVersion', message: 'Stale version conflict' },
      };

      await boot();

      const checkInBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="check-in"], .xc-checkin-btn',
      ) as HTMLButtonElement;
      expect(checkInBtn).not.toBeNull();
      checkInBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(host.textContent).toContain(texts.th['error.staleVersion']);
      expect(host.textContent).not.toContain('staleVersion');
    });

    it('queueClosed (400) displays worded error from i18n and absent raw code', async () => {
      commandStatus = 400;
      commandResponse = {
        error: { code: 'queueClosed', message: 'Queue is closed for today' },
      };

      await boot();

      const checkInBtn = host.querySelector(
        '[data-booking-id="BK-test-0001"] button[data-action="check-in"], .xc-checkin-btn',
      ) as HTMLButtonElement;
      expect(checkInBtn).not.toBeNull();
      checkInBtn.click();
      for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));

      expect(host.textContent).toContain(texts.th['error.queueClosed']);
      expect(host.textContent).not.toContain('queueClosed');
    });
  });

  // ── 11. Page Entry & i18n Contract Tests ────────────────────────────────────

  describe('11. Entry, HTML and i18n Contract', () => {
    it('main.tsx renders inside the shared AppShell without crashing', async () => {
      await boot();
      const shell = host.querySelector('.xc-shell');
      expect(shell, 'the entry renders AppShell').not.toBeNull();
      expect(shell?.textContent).toContain(texts.th.title);
    });

    it('i18n has matching keys in both languages and no extra languages', () => {
      const allTexts = texts as unknown as Record<string, Record<string, string> | undefined>;
      expect(allTexts['zh']).toBeUndefined();
      const thKeys = Object.keys(texts.th).sort();
      const enKeys = Object.keys(texts.en).sort();
      expect(thKeys).toEqual(enKeys);
    });
  });
});
