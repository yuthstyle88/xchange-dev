// xchange#2459: closing the cancel or reschedule dialog while its command is in flight
// must not let the late answer close or error another booking's dialog or retarget Retry.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import {
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
  type MyBookingItemDto,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';

const BOOKING_A = 'bk-alice-a';
const BOOKING_B = 'bk-alice-b';
const BRANCH_A = 'bkk-a';
const BRANCH_B = 'bkk-b';
const SLOT_A = 'slot-a-1';
const SLOT_B = 'slot-b-1';

function makeBooking(id: string, bookingNo: string, branchId: string, slotId: string) {
  return makeMyBookingItemDto({
    id,
    bookingNo,
    branchId,
    slotId,
    startAt: '2026-10-15T10:00:00Z',
    endAt: '2026-10-15T10:30:00Z',
    status: 'CONFIRMED',
    cancelCutoffAt: '2026-10-15T09:00:00Z',
    qrToken: `QR-${id}`,
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    quoteSummary: '',
    readiness: 'unknown',
  });
}

const INITIAL_BOOKINGS = [
  makeBooking(BOOKING_A, 'B261015-0001', BRANCH_A, SLOT_A),
  makeBooking(BOOKING_B, 'B261015-0002', BRANCH_B, SLOT_B),
];

function slotsFor(branch: string | null) {
  const slotId = branch === BRANCH_B ? SLOT_B : SLOT_A;
  return makeBookingSlotsData({
    asOf: '2026-10-15T08:00:00Z',
    slots: [
      makeBookingSlotItemDto({
        id: slotId,
        start: '2026-10-15T11:00:00Z',
        end: '2026-10-15T11:30:00Z',
        available: 4,
      }),
    ],
  });
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

interface PendingCommandCall {
  kind: 'cancel' | 'reschedule';
  bookingId: string;
  path: string;
  key: string | null;
  body: string;
  resolve: (res: Response) => void;
  reject: (err: unknown) => void;
}

function makeDeferredHarness() {
  const commands: PendingCommandCall[] = [];
  let currentBookings: MyBookingItemDto[] = INITIAL_BOOKINGS.map((b) => ({ ...b }));

  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');

    if (method === 'GET' && u.pathname === '/api/public/host') {
      return jsonResponse(200, { org: 'XC' });
    }
    if (method === 'GET' && u.pathname === '/api/public/orgs/XC/branches') {
      return jsonResponse(200, {
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
    if (method === 'GET' && u.pathname === '/api/v3/views/my_bookings') {
      const body = makeMyBookingsData({ bookings: currentBookings.map((b) => ({ ...b })) });
      return jsonResponse(200, { version: 1, changed: true, data: body });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/booking_slots') {
      return jsonResponse(200, {
        version: 1,
        changed: true,
        data: slotsFor(u.searchParams.get('branch')),
      });
    }

    const cancelMatch = /^\/api\/bookings\/([^/]+)\/cancel$/.exec(u.pathname);
    if (method === 'POST' && cancelMatch) {
      const bookingId = cancelMatch[1]!;
      const headers = new Headers(init?.headers);
      const rawBody = typeof init?.body === 'string' ? init.body : '';
      return new Promise<Response>((resolve, reject) => {
        commands.push({
          kind: 'cancel',
          bookingId,
          path: u.pathname,
          key: headers.get('idempotency-key'),
          body: rawBody,
          resolve,
          reject,
        });
      });
    }

    const rescheduleMatch = /^\/api\/bookings\/([^/]+)\/reschedule$/.exec(u.pathname);
    if (method === 'POST' && rescheduleMatch) {
      const bookingId = rescheduleMatch[1]!;
      const headers = new Headers(init?.headers);
      const rawBody = typeof init?.body === 'string' ? init.body : '';
      return new Promise<Response>((resolve, reject) => {
        commands.push({
          kind: 'reschedule',
          bookingId,
          path: u.pathname,
          key: headers.get('idempotency-key'),
          body: rawBody,
          resolve,
          reject,
        });
      });
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);

  return {
    api: new ApiClient(fetchFn),
    commands,
    setBookingsState(next: MyBookingItemDto[]) {
      currentBookings = next;
    },
  };
}

async function flush(n = 8): Promise<void> {
  for (let i = 0; i < n; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('Bookings.tsx — closing cancel/reschedule dialog while command is in flight (#2459)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date('2026-10-15T08:00:00Z'));
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function mountScreen(api: ApiClient): Promise<void> {
    dispose = render(() => <BookingsScreen client={api} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector(`[data-booking-id="${BOOKING_A}"]`)).not.toBeNull();
      expect(host.querySelector(`[data-booking-id="${BOOKING_B}"]`)).not.toBeNull();
    });
  }

  function openCancel(bookingId: string): void {
    const btn = host.querySelector(
      `[data-booking-id="${bookingId}"] button[data-action="cancel"]`,
    ) as HTMLButtonElement | null;
    expect(btn, `cancel button for ${bookingId}`).not.toBeNull();
    btn!.click();
  }

  function confirmCancel(): void {
    const btn = host.querySelector('button[data-action="confirm-cancel"]') as HTMLButtonElement | null;
    expect(btn, 'confirm-cancel button').not.toBeNull();
    btn!.click();
  }

  function closeCancelViaBackdrop(): void {
    const backdrop = host.querySelector('.xc-modal-backdrop') as HTMLElement | null;
    expect(backdrop, 'cancel modal backdrop').not.toBeNull();
    backdrop!.click();
  }

  function closeCancelViaBack(): void {
    const backBtn = host.querySelector('.xc-confirm-dialog .xc-btn-secondary') as HTMLButtonElement | null;
    expect(backBtn, 'cancel dialog back button').not.toBeNull();
    backBtn!.click();
  }

  async function openRescheduleAndWaitSlots(bookingId: string, expectedSlotId: string): Promise<void> {
    const btn = host.querySelector(
      `[data-booking-id="${bookingId}"] button[data-action="reschedule"]`,
    ) as HTMLButtonElement | null;
    expect(btn, `reschedule button for ${bookingId}`).not.toBeNull();
    btn!.click();
    await vi.waitFor(() => {
      expect(host.querySelector(`.xc-slot-option[data-slot-id="${expectedSlotId}"] input[type="radio"]`)).not.toBeNull();
    });
  }

  function selectSlot(slotId: string): void {
    const radio = host.querySelector(
      `.xc-slot-option[data-slot-id="${slotId}"] input[type="radio"]`,
    ) as HTMLInputElement | null;
    expect(radio, `radio for ${slotId}`).not.toBeNull();
    radio!.click();
    radio!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function confirmReschedule(): void {
    const btn = host.querySelector('button[data-action="confirm-reschedule"]') as HTMLButtonElement | null;
    expect(btn, 'confirm-reschedule button').not.toBeNull();
    btn!.click();
  }

  function closeRescheduleViaBack(): void {
    const backBtn = host.querySelector('.xc-reschedule-dialog .xc-btn-secondary') as HTMLButtonElement | null;
    expect(backBtn, 'reschedule dialog back button').not.toBeNull();
    backBtn!.click();
  }

  it("row 1: Cancel A, close while pending, open Cancel on B, A is refused -> B's dialog shows no error from A; no Retry targets B", async () => {
    const { api, commands } = makeDeferredHarness();
    await mountScreen(api);

    // 1. Open Cancel on A and click Confirm
    openCancel(BOOKING_A);
    confirmCancel();
    expect(commands).toHaveLength(1);
    expect(commands[0]!.bookingId).toBe(BOOKING_A);

    // 2. Close A's dialog via backdrop while A's cancel is pending, then open Cancel on B
    closeCancelViaBackdrop();
    expect(host.querySelector('.xc-confirm-dialog')).toBeNull();

    openCancel(BOOKING_B);
    expect(host.querySelector('.xc-confirm-dialog')).not.toBeNull();

    // 3. A's cancel answers with a 409 refusal (cancelCutoffPassed)
    commands[0]!.resolve(
      jsonResponse(409, {
        error: { code: 'cancelCutoffPassed', message: 'Cancel cutoff has passed' },
      }),
    );
    await flush();

    // B's dialog must stay open with NO error banner and NO Retry button targeting B
    expect(host.querySelector('.xc-confirm-dialog')).not.toBeNull();
    expect(host.querySelector('.xc-dialog-error')).toBeNull();
    expect(host.querySelector('.xc-command-error')).toBeNull();
    expect(host.querySelector('button[data-action="retry"]')).toBeNull();
    expect(commands).toHaveLength(1);
  });

  it("row 2: Cancel A, close while pending, open Cancel on B, A succeeds -> B's dialog stays open; A shows cancelled in the list", async () => {
    const { api, commands } = makeDeferredHarness();
    await mountScreen(api);

    // 1. Open Cancel on A and click Confirm
    openCancel(BOOKING_A);
    confirmCancel();
    expect(commands).toHaveLength(1);
    expect(commands[0]!.bookingId).toBe(BOOKING_A);

    // 2. Close A's dialog via Back while A's cancel is pending, then open Cancel on B
    closeCancelViaBack();
    openCancel(BOOKING_B);
    expect(host.querySelector('.xc-confirm-dialog')).not.toBeNull();

    // 3. A's cancel succeeds
    commands[0]!.resolve(
      jsonResponse(200, {
        id: BOOKING_A,
        bookingNo: 'B261015-0001',
        status: 'CANCELLED',
      }),
    );
    await flush();

    // B's dialog must remain open
    expect(host.querySelector('.xc-confirm-dialog')).not.toBeNull();

    // A shows cancelled in the list
    const cardA = host.querySelector(`[data-booking-id="${BOOKING_A}"]`)!;
    expect(cardA.querySelector('.xc-status-cancelled')).not.toBeNull();

    // Confirming inside B's still-open dialog cancels B
    confirmCancel();
    expect(commands).toHaveLength(2);
    expect(commands[1]!.bookingId).toBe(BOOKING_B);
  });

  it("row 3a: Reschedule A, close while pending, open Reschedule on B, A is refused -> B's dialog unaffected", async () => {
    const { api, commands } = makeDeferredHarness();
    await mountScreen(api);

    await openRescheduleAndWaitSlots(BOOKING_A, SLOT_A);
    selectSlot(SLOT_A);
    confirmReschedule();
    expect(commands).toHaveLength(1);
    expect(commands[0]!.kind).toBe('reschedule');
    expect(commands[0]!.bookingId).toBe(BOOKING_A);

    // Close A while pending, open Reschedule on B
    closeRescheduleViaBack();
    await openRescheduleAndWaitSlots(BOOKING_B, SLOT_B);
    selectSlot(SLOT_B);

    // A's reschedule fails with 409 capacityFull
    commands[0]!.resolve(
      jsonResponse(409, {
        error: { code: 'capacityFull', message: 'Slot is full' },
      }),
    );
    await flush();

    // B's reschedule dialog is open and shows no error from A
    expect(host.querySelector('.xc-reschedule-dialog')).not.toBeNull();
    expect(host.querySelector('.xc-dialog-error')).toBeNull();
    expect(host.querySelector('.xc-command-error')).toBeNull();
    expect(host.querySelector('button[data-action="retry"]')).toBeNull();
    expect(commands).toHaveLength(1);
  });

  it("row 3b: Reschedule A, close while pending, open Reschedule on B, A succeeds -> B's dialog stays open and unaffected", async () => {
    const { api, commands } = makeDeferredHarness();
    await mountScreen(api);

    await openRescheduleAndWaitSlots(BOOKING_A, SLOT_A);
    selectSlot(SLOT_A);
    confirmReschedule();
    expect(commands).toHaveLength(1);
    expect(commands[0]!.bookingId).toBe(BOOKING_A);

    // Close A while pending, open Reschedule on B
    closeRescheduleViaBack();
    await openRescheduleAndWaitSlots(BOOKING_B, SLOT_B);
    selectSlot(SLOT_B);

    // A's reschedule succeeds
    commands[0]!.resolve(
      jsonResponse(200, {
        id: BOOKING_A,
        bookingNo: 'B261015-0001',
        slotId: SLOT_A,
        status: 'CONFIRMED',
      }),
    );
    await flush();

    // B's reschedule dialog must remain open and able to submit for B
    expect(host.querySelector('.xc-reschedule-dialog')).not.toBeNull();
    confirmReschedule();
    expect(commands).toHaveLength(2);
    expect(commands[1]!.bookingId).toBe(BOOKING_B);
  });

  it("edge case (§9): Retry on A's own dialog (not closed) still retries A with the same key", async () => {
    const { api, commands } = makeDeferredHarness();
    await mountScreen(api);

    // Start a cancel on B and close B while pending so a stale in-flight command exists
    openCancel(BOOKING_B);
    confirmCancel();
    expect(commands).toHaveLength(1);
    expect(commands[0]!.bookingId).toBe(BOOKING_B);
    closeCancelViaBack();

    // Now open Cancel on A (and keep A's dialog open)
    openCancel(BOOKING_A);
    confirmCancel();
    expect(commands).toHaveLength(2);
    expect(commands[1]!.bookingId).toBe(BOOKING_A);

    // A's first attempt fails with 500 server error -> shows Retry on A's open dialog
    commands[1]!.resolve(
      jsonResponse(500, {
        error: { code: 'internalError', message: 'Temporary error' },
      }),
    );
    await flush();

    // Meanwhile B's stale cancel resolves 200 OK -> must NOT close A's open dialog
    commands[0]!.resolve(
      jsonResponse(200, {
        id: BOOKING_B,
        bookingNo: 'B261015-0002',
        status: 'CANCELLED',
      }),
    );
    await flush();

    expect(host.querySelector('.xc-confirm-dialog')).not.toBeNull();
    const retryBtn = host.querySelector('.xc-confirm-dialog button[data-action="retry"]') as HTMLButtonElement | null;
    expect(retryBtn, 'Retry button on A dialog').not.toBeNull();

    // Clicking Retry on A's own dialog retries A with the exact same Idempotency-Key
    retryBtn!.click();
    await flush();

    expect(commands).toHaveLength(3);
    expect(commands[2]!.bookingId).toBe(BOOKING_A);
    expect(commands[1]!.key).toBeTruthy();
    expect(commands[2]!.key).toBe(commands[1]!.key);
    expect(commands[2]!.body).toBe(commands[1]!.body);
  });
});
