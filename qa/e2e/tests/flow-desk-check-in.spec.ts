// QA-F22 — flow: desk check-in issues a BOOKING ticket (skip-and-log without a today slot)
// (design docs/design/qa-e2e.md §4 row F22; blueprint/booking-slots.md:74 B8, :208-209).
// Written by ticket #4240 (QA-F22); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - customer/book/Book.tsx:394-692
//   [data-testid="book-screen"], [data-testid="branch-select"], [data-testid="date-input"],
//   [data-testid="slot-select"], [data-testid="slot-option"], [data-testid="currency-select"],
//   [data-testid="quantity-input"], [data-testid="book-submit"], .xc-book-success
// - customer/bookings/Bookings.tsx:377-494
//   .xc-booking-card[data-booking-id="<id>"], [data-action="cancel"], [data-action="confirm-cancel"],
//   .xc-status-cancelled
// - desk/bookings/DeskBookingsScreen.tsx:30,618-704
//   [data-screen="desk-bookings"], .xc-booking-item[data-booking-id="<id>"],
//   .xc-booking-no (:679), .xc-ticket-number (:684), .xc-status-CHECKED_IN (:686),
//   [data-action="check-in"] (:697) (POST /api/bookings/<id>/check-in)
// - desk/queue/CounterBar.tsx:114-139
//   button[name="openQueue"], button[name="closeQueue"] (POST /api/queue/open|close)
// - desk/queue/board/Board.tsx:43-59
//   li[data-ticket="<id>"] (ticket row with displayNumber and BOOKING serviceType; board/i18n.ts:9,23)
// - desk/queue/actions/TicketActions.tsx:113-140
//   button[name="cancel"], input[name="cancelReason"], button[name="cancelConfirm"]
//   (POST /api/queue/tickets/<id>/cancel)
import { expect, test, type Page } from '@playwright/test';
import {
  closeShift,
  ensureShiftClosed,
  hasAccount,
  openShift,
  posted,
  shot,
  signIn,
  users,
  type Role,
} from './helpers';

const REQUIRED_ROLES: readonly Role[] = ['customer', 'teller', 'branchManager'];

interface BookingSlotItemDto {
  id: string;
  start: string;
  end: string;
  available: number;
}

interface BookingSlotsEnvelope {
  data?: {
    asOf?: string;
    slots?: BookingSlotItemDto[];
  };
}

interface BranchServiceStatusEnvelope {
  data?: {
    queueOpen?: boolean;
  };
}

interface QueueBoardWaitingItem {
  id: string;
  displayNumber: string;
  serviceType: string;
  sequence: number;
}

interface QueueBoardEnvelope {
  data?: {
    waiting?: QueueBoardWaitingItem[];
  };
}

// Today in Asia/Bangkok (yyyy-mm-dd), matching Book.tsx getTodayInTimezone('Asia/Bangkok').
function todayBangkok(): string {
  const now = new Date(Date.now() + 7 * 3600_000);
  return now.toISOString().slice(0, 10);
}

test.describe.serial('F22 desk check-in', () => {
  test('desk check-in issues a BOOKING ticket on the queue board (skip-and-log without a today slot)', async ({
    browser,
  }) => {
    const missing = REQUIRED_ROLES.filter((role) => !hasAccount(role));
    if (missing.length > 0) {
      const reason = `missing account in users.json for: ${missing.join(', ')} (run scripts/reseed.sh)`;
      console.log(`[F22] skip: ${reason}`);
      test.skip(true, reason);
    }

    const customer: Page = await browser.newPage({
      baseURL: users.baseUrl,
      viewport: { width: 430, height: 932 },
    });
    const teller: Page = await browser.newPage({
      baseURL: users.baseUrl,
      viewport: { width: 1280, height: 900 },
    });
    const manager: Page = await browser.newPage({
      baseURL: users.baseUrl,
      viewport: { width: 1280, height: 900 },
    });

    const today = todayBangkok();
    let bookingId = '';
    let bookingNo = '';
    let checkedIn = false;
    let ticketId = '';
    let ticketNumber = '';
    let ticketCancelled = false;
    let openedShift = false;
    let openedQueue = false;
    const cleanupFailures: string[] = [];

    try {
      // 1. Precondition: read booking_slots for MAIN (users.branch) for today.
      //    Seed step 4 makes slots from tomorrow on, so skip and log if no OPEN slot starts later than now.
      const slotsRes = await customer.request.get(
        `/api/v3/views/booking_slots?branch=${encodeURIComponent(users.branch)}&date=${encodeURIComponent(today)}`,
      );
      expect(
        slotsRes.ok(),
        `GET /api/v3/views/booking_slots: ${slotsRes.status()} ${await slotsRes.text().catch(() => '')}`,
      ).toBeTruthy();
      const slotsBody = (await slotsRes.json()) as BookingSlotsEnvelope;
      const nowMs = slotsBody.data?.asOf ? Date.parse(slotsBody.data.asOf) : Date.now();
      const openLaterSlots = (slotsBody.data?.slots ?? []).filter(
        (s) => s.available > 0 && Date.parse(s.start) > nowMs,
      );
      if (openLaterSlots.length === 0) {
        const reason = 'no MAIN slot today later than now (seed makes slots from tomorrow)';
        console.log(`[F22] skip: ${reason}`);
        test.skip(true, reason);
      }
      const targetSlot = openLaterSlots[0]!;
      console.log(`[F22] found today slot id=${targetSlot.id} start=${targetSlot.start} available=${targetSlot.available}`);

      // Sign in staff and customer.
      await signIn(manager, 'branchManager', '/staff');
      await signIn(teller, 'teller', '/staff');
      await signIn(customer, 'customer', '/app/book');

      // 2. Leftover pre-check from QA-F8 (flow-queue-many.spec.ts:80-92): fail if WAITING tickets are left over.
      const preBoardRes = await teller.request.get(
        `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
      );
      expect(
        preBoardRes.ok(),
        `GET /api/v3/views/queue_board: ${preBoardRes.status()} ${await preBoardRes.text().catch(() => '')}`,
      ).toBeTruthy();
      const preBoardBody = (await preBoardRes.json()) as QueueBoardEnvelope;
      const leftoverNumbers = (preBoardBody.data?.waiting ?? []).map((w) => w.displayNumber);
      expect(
        leftoverNumbers,
        `waiting queue must be empty before test (leftover display numbers: ${leftoverNumbers.join(', ')})`,
      ).toEqual([]);

      // Open teller shift so the teller is on duty in the branch (needed to cancel the queue ticket in finally).
      await ensureShiftClosed(teller, manager);
      await openShift(teller, manager);
      openedShift = true;

      // 3. Open the queue only if it is closed, and remember that this spec opened it.
      await manager.goto('/desk/queue');
      const openQueueBtn = manager.locator('button[name="openQueue"]');
      await expect(openQueueBtn, 'branch manager openQueue button (CounterBar.tsx:114-126)').toBeVisible({
        timeout: 20_000,
      });

      const statusRes = await manager.request.get(
        `/api/v3/views/branch_service_status?branch=${encodeURIComponent(users.branch)}`,
      );
      expect(
        statusRes.ok(),
        `GET /api/v3/views/branch_service_status: ${statusRes.status()} ${await statusRes.text().catch(() => '')}`,
      ).toBeTruthy();
      const statusBody = (await statusRes.json()) as BranchServiceStatusEnvelope;
      const wasQueueOpen = Boolean(statusBody.data?.queueOpen);
      console.log(`[F22] branch=${users.branch} initial queueOpen=${wasQueueOpen}`);

      if (!wasQueueOpen) {
        const openRes = await posted(manager, /^\/api\/queue\/open$/, () => openQueueBtn.click());
        console.log(`[F22] POST /api/queue/open ${openRes.text}`);
        expect(openRes.ok, `open queue accepted: ${openRes.text}`).toBeTruthy();
        openedQueue = true;
      }

      // 4a. Customer books that slot on /app/book.
      await expect(customer.getByTestId('book-screen')).toBeVisible({ timeout: 15_000 });
      const slotsLoaded = customer
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${today}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await customer.getByTestId('branch-select').selectOption(users.branch);
      await customer.getByTestId('date-input').fill(today);
      await slotsLoaded;
      await expect(customer.locator('.xc-book-slots-loading')).toBeHidden({ timeout: 15_000 });

      await customer.getByTestId('slot-select').selectOption(targetSlot.id);
      await customer.getByTestId('currency-select').selectOption(users.currency ?? 'USD');
      await customer.getByTestId('quantity-input').fill(users.quantity ?? '100');

      const bookRes = await posted(customer, /^\/api\/bookings$/, () => customer.getByTestId('book-submit').click());
      console.log(`[F22] POST /api/bookings ${bookRes.text}`);
      expect(bookRes.ok, `POST /api/bookings: ${bookRes.text}`).toBeTruthy();
      await expect(customer.locator('.xc-book-success')).toBeVisible({ timeout: 20_000 });

      const bookParsed = JSON.parse(bookRes.body) as {
        id?: string;
        bookingNo?: string;
        data?: { id?: string; bookingNo?: string };
      };
      const bookData = bookParsed.data ?? bookParsed;
      bookingId = String(bookData.id ?? '').trim();
      bookingNo = String(bookData.bookingNo ?? '').trim();
      expect(bookingId, 'booked bookingId').not.toBe('');
      expect(bookingNo, 'booked bookingNo').not.toBe('');
      await shot(customer, 'desk', 'f22-1-customer-booked');
      console.log(`[F22] booked id=${bookingId} bookingNo=${bookingNo}`);

      // 4b. Teller on /desk/bookings finds the booking and presses [data-action="check-in"].
      await teller.goto('/desk/bookings');
      await expect(teller.locator('[data-screen="desk-bookings"]')).toBeVisible({ timeout: 20_000 });

      const deskRow = teller.locator(`.xc-booking-item[data-booking-id="${bookingId}"]`);
      await expect(deskRow, `desk row for ${bookingId} (${bookingNo})`).toBeVisible({ timeout: 20_000 });
      await expect(deskRow.locator('.xc-booking-no')).toHaveText(bookingNo);

      const checkInBtn = deskRow.locator('[data-action="check-in"]');
      await expect(checkInBtn, 'check-in button on CONFIRMED booking (DeskBookingsScreen.tsx:697)').toBeVisible({
        timeout: 15_000,
      });

      const checkInRes = await posted(
        teller,
        new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/check-in$`),
        () => checkInBtn.click(),
      );
      console.log(`[F22] POST /api/bookings/${bookingId}/check-in ${checkInRes.text}`);
      expect(checkInRes.ok, `POST /api/bookings/${bookingId}/check-in: ${checkInRes.text}`).toBeTruthy();
      checkedIn = true;

      const checkInParsed = JSON.parse(checkInRes.body) as {
        id?: string;
        status?: string;
        ticketNumber?: string;
        bookingNo?: string;
        data?: { id?: string; status?: string; ticketNumber?: string; bookingNo?: string };
      };
      const checkInData = checkInParsed.data ?? checkInParsed;
      ticketNumber = String(checkInData.ticketNumber ?? '').trim();
      expect(ticketNumber, 'ticketNumber returned by check-in').not.toBe('');
      await expect(deskRow.locator('.xc-status-CHECKED_IN'), 'desk row shows CHECKED_IN').toBeVisible({
        timeout: 15_000,
      });
      await expect(deskRow.locator('.xc-ticket-number')).toHaveText(ticketNumber);
      await shot(teller, 'desk', 'f22-2-desk-checked-in');

      // 4c. A BOOKING ticket appears on the board (/desk/queue).
      await teller.goto('/desk/queue');
      const boardRes = await teller.request.get(
        `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
      );
      expect(
        boardRes.ok(),
        `GET /api/v3/views/queue_board after check-in: ${boardRes.status()}`,
      ).toBeTruthy();
      const boardBody = (await boardRes.json()) as QueueBoardEnvelope;
      const waitingTicket = (boardBody.data?.waiting ?? []).find((w) => w.displayNumber === ticketNumber);
      expect(waitingTicket, `BOOKING ticket ${ticketNumber} in queue_board.waiting`).toBeTruthy();
      expect(waitingTicket!.serviceType, `ticket ${ticketNumber} serviceType`).toBe('BOOKING');
      ticketId = waitingTicket!.id;

      const ticketRow = teller.locator(`li[data-ticket="${ticketId}"]`);
      await expect(ticketRow, `BOOKING ticket ${ticketNumber} row on /desk/queue board`).toBeVisible({
        timeout: 20_000,
      });
      await expect(ticketRow).toContainText(ticketNumber);
      await expect(ticketRow).toContainText('BOOKING');
      await shot(teller, 'desk', 'f22-3-board-booking-ticket');

      // Cancel the ticket with a reason (TicketActions.tsx:113-140) and verify it leaves the board.
      const cancelBtn = ticketRow.locator('button[name="cancel"]');
      await expect(cancelBtn, 'cancel button on BOOKING ticket (TicketActions.tsx:113)').toBeVisible({
        timeout: 20_000,
      });
      await cancelBtn.click();

      const reasonInput = ticketRow.locator('input[name="cancelReason"]');
      await expect(reasonInput, 'cancelReason input (TicketActions.tsx:128)').toBeVisible({ timeout: 20_000 });
      await reasonInput.fill(`e2e-F22-${Date.now()}`);

      const confirmBtn = ticketRow.locator('button[name="cancelConfirm"]');
      await expect(confirmBtn, 'cancelConfirm button (TicketActions.tsx:137)').toBeVisible({ timeout: 20_000 });
      const cancelTicketRes = await posted(
        teller,
        new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketId)}/cancel$`),
        () => confirmBtn.click(),
      );
      console.log(`[F22] POST /api/queue/tickets/${ticketId}/cancel ${cancelTicketRes.text}`);
      expect(cancelTicketRes.ok, `cancel BOOKING ticket accepted: ${cancelTicketRes.text}`).toBeTruthy();
      await expect(ticketRow, `cancelled BOOKING ticket ${ticketNumber} leaves the board`).toHaveCount(0, {
        timeout: 20_000,
      });
      ticketCancelled = true;
      await shot(teller, 'desk', 'f22-4-board-ticket-cancelled');
    } finally {
      // 5. finally:
      //    (a) cancel the BOOKING ticket with a reason if check-in issued one and it is not yet cancelled;
      try {
        if ((ticketId || ticketNumber) && !ticketCancelled) {
          await teller.goto('/desk/queue');
          if (!ticketId && ticketNumber) {
            const bRes = await teller.request.get(
              `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
            );
            if (bRes.ok()) {
              const bBody = (await bRes.json()) as QueueBoardEnvelope;
              ticketId = (bBody.data?.waiting ?? []).find((w) => w.displayNumber === ticketNumber)?.id ?? '';
            }
          }
          if (ticketId) {
            const row = teller.locator(`li[data-ticket="${ticketId}"]`);
            if (await row.isVisible({ timeout: 10_000 }).catch(() => false)) {
              await row.locator('button[name="cancel"]').click();
              await row.locator('input[name="cancelReason"]').fill(`e2e-F22-cleanup-${Date.now()}`);
              const cleanTicketRes = await posted(
                teller,
                new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketId)}/cancel$`),
                () => row.locator('button[name="cancelConfirm"]').click(),
              );
              console.log(`[F22] finally: POST /api/queue/tickets/${ticketId}/cancel ${cleanTicketRes.text}`);
              expect(cleanTicketRes.ok, `cleanup cancel ticket ${ticketId}: ${cleanTicketRes.text}`).toBeTruthy();
              await expect(row).toHaveCount(0, { timeout: 15_000 });
            }
          }
        }
      } catch (e) {
        cleanupFailures.push(`ticket cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      //    (b) cancel the booking if it was created and not checked in;
      try {
        if (bookingId && !checkedIn) {
          await customer.goto('/app/bookings');
          const card = customer.locator(`.xc-booking-card[data-booking-id="${bookingId}"]`);
          await expect(card, `cleanup: booking card ${bookingId} on /app/bookings`).toBeVisible({ timeout: 15_000 });
          const cancelBtn = card.locator('[data-action="cancel"]');
          if (await cancelBtn.isVisible().catch(() => false)) {
            await cancelBtn.click();
            const confirmBtn = customer.locator('[data-action="confirm-cancel"]');
            await expect(confirmBtn, 'cleanup: customer confirm-cancel button').toBeVisible({ timeout: 15_000 });
            const cleanBookingRes = await posted(
              customer,
              new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/cancel$`),
              () => confirmBtn.click(),
            );
            console.log(`[F22] finally: POST /api/bookings/${bookingId}/cancel ${cleanBookingRes.text}`);
            expect(cleanBookingRes.ok, `cleanup cancel booking ${bookingId}: ${cleanBookingRes.text}`).toBeTruthy();
          }
          await expect(card.locator('.xc-status-cancelled')).toBeVisible({ timeout: 15_000 });
        }
      } catch (e) {
        cleanupFailures.push(`booking cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      //    (c) close the teller shift if opened;
      try {
        if (openedShift) {
          await teller.goto('/staff');
          await manager.goto('/staff');
          await closeShift(teller, manager);
        }
      } catch (e) {
        cleanupFailures.push(`shift cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      //    (d) close the queue ONLY if this spec opened it.
      try {
        if (openedQueue) {
          await manager.goto('/desk/queue');
          const closeQueueBtn = manager.locator('button[name="closeQueue"]');
          await expect(closeQueueBtn, 'branch manager closeQueue button (CounterBar.tsx:127-139)').toBeVisible({
            timeout: 20_000,
          });
          const closeRes = await posted(manager, /^\/api\/queue\/close$/, () => closeQueueBtn.click());
          console.log(`[F22] finally: POST /api/queue/close ${closeRes.text}`);
          expect(closeRes.ok, `close queue accepted: ${closeRes.text}`).toBeTruthy();
        }
      } catch (e) {
        cleanupFailures.push(`queue cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      await Promise.all([
        customer.close().catch(() => {}),
        teller.close().catch(() => {}),
        manager.close().catch(() => {}),
      ]);
      expect(cleanupFailures, 'F22 finally cleanup errors').toEqual([]);
    }
  });
});
