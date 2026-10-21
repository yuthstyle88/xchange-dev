// QA-F21 — flow: booking reschedule to another date, then cancel at any time
// (design docs/design/qa-e2e.md §4 row F21; blueprint/booking-slots.md Q-B1, Q-B2 revised; #4204).
// Written by ticket #4239 (QA-F21); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - customer/book/Book.tsx:394-692
//   [data-testid="book-screen"], [data-testid="branch-select"], [data-testid="date-input"],
//   [data-testid="slot-select"], [data-testid="slot-option"], [data-testid="currency-select"],
//   [data-testid="quantity-input"], [data-testid="book-submit"], .xc-book-success
// - customer/bookings/Bookings.tsx:391-634
//   .xc-booking-card[data-booking-id="<id>"], .xc-booking-no (:393), .xc-booking-time (:402),
//   .xc-status-cancelled (:394), [data-action="cancel"] (:440), [data-action="reschedule"] (:450),
//   .xc-confirm-dialog (:469), [data-action="confirm-cancel"] (:503),
//   .xc-reschedule-dialog (:519), [data-field="serviceDate"] input[name="serviceDate"] (:527-537),
//   .xc-slots-loading (:575), input[name="rescheduleSlot"] (:593-596),
//   [data-action="confirm-reschedule"] (:628)
import { expect, test, type Page } from '@playwright/test';
import { posted, shot, signIn, users } from './helpers';

// Date in Asia/Bangkok offset by `daysAhead` days (yyyy-mm-dd), matching flow-booking.spec.ts:7-10.
function bangkokDate(daysAhead: number): string {
  const now = new Date(Date.now() + 7 * 3600_000 + daysAhead * 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}

// Cancels the booking card on /app/bookings and waits for POST /api/bookings/<id>/cancel + CANCELLED badge.
async function cancelBookingCard(page: Page, bookingId: string): Promise<void> {
  const card = page.locator(`.xc-booking-card[data-booking-id="${bookingId}"]`);
  await expect(card, `booking card ${bookingId} on /app/bookings`).toBeVisible({ timeout: 15_000 });
  const cancelBtn = card.locator('[data-action="cancel"]');
  await expect(cancelBtn, `[data-action="cancel"] on ${bookingId}`).toBeVisible({ timeout: 15_000 });
  await cancelBtn.click();

  const dialog = page.locator('.xc-confirm-dialog');
  await expect(dialog, 'cancel confirm dialog').toBeVisible({ timeout: 15_000 });
  const confirmBtn = dialog.locator('[data-action="confirm-cancel"]');
  const cancelRes = await posted(
    page,
    new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/cancel$`),
    () => confirmBtn.click(),
  );
  console.log(`[F21] POST /api/bookings/${bookingId}/cancel ${cancelRes.text}`);
  expect(cancelRes.ok, `POST /api/bookings/${bookingId}/cancel: ${cancelRes.text}`).toBeTruthy();
  await expect(dialog, 'cancel confirm dialog closed').toBeHidden({ timeout: 15_000 });
  await expect(card.locator('.xc-status-cancelled'), `${bookingId} has .xc-status-cancelled`).toBeVisible({
    timeout: 15_000,
  });
  await expect(
    card.getByText(/ยกเลิกแล้ว|Cancelled|已取消|CANCELLED/i),
    `${bookingId} shows CANCELLED on /app/bookings`,
  ).toBeVisible({ timeout: 15_000 });
}

test.use({ viewport: { width: 430, height: 932 } });

test.describe.serial('F21 booking reschedule', () => {
  test('customer books tomorrow, reschedules to the day after tomorrow, and cancels at any time', async ({ page }) => {
    const tomorrow = bangkokDate(1);
    const dayAfter = bangkokDate(2);
    let bookingId = '';
    let bookingNo = '';
    let cancelled = false;
    const cleanupFailures: string[] = [];

    try {
      // Step 1: book MAIN for tomorrow, as in flow-booking.spec.ts.
      await signIn(page, 'customer', '/app/book');
      await expect(page.getByTestId('book-screen')).toBeVisible({ timeout: 15_000 });
      await page.getByTestId('branch-select').selectOption(users.branch);

      const slotsTomorrow = page
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${tomorrow}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await page.getByTestId('date-input').fill(tomorrow);
      await slotsTomorrow;
      await expect(page.locator('.xc-book-slots-loading')).toBeHidden({ timeout: 15_000 });

      const openTomorrow = page.locator('[data-testid="slot-option"]:not([disabled])');
      if ((await openTomorrow.count()) === 0) {
        const reason = `no open slot on ${tomorrow} (seed)`;
        console.log(`[F21] skip: ${reason}`);
        test.skip(true, reason);
      }

      const slotValue = await openTomorrow.first().getAttribute('value');
      expect(slotValue, 'first open slot id on tomorrow').toBeTruthy();
      await page.getByTestId('slot-select').selectOption(slotValue!);
      await page.getByTestId('currency-select').selectOption(users.currency ?? 'USD');
      await page.getByTestId('quantity-input').fill(users.quantity ?? '100');

      const bookRes = await posted(page, /^\/api\/bookings$/, () => page.getByTestId('book-submit').click());
      console.log(`[F21] POST /api/bookings ${bookRes.text}`);
      expect(bookRes.ok, `POST /api/bookings: ${bookRes.text}`).toBeTruthy();
      await expect(page.locator('.xc-book-success')).toBeVisible({ timeout: 20_000 });

      try {
        const parsed = JSON.parse(bookRes.body) as {
          id?: string;
          bookingNo?: string;
          data?: { id?: string; bookingNo?: string };
        };
        const data = parsed.data ?? parsed;
        bookingId = String(data.id ?? '').trim();
        bookingNo = String(data.bookingNo ?? '').trim();
      } catch {
        // read from /app/bookings below
      }

      // Step 2: on /app/bookings, open the reschedule dialog and verify serviceDate defaults to tomorrow.
      await page.goto('/app/bookings');
      const card = bookingId
        ? page.locator(`.xc-booking-card[data-booking-id="${bookingId}"]`)
        : page.locator('.xc-booking-card').first();
      await expect(card, 'booked card on /app/bookings').toBeVisible({ timeout: 15_000 });
      if (!bookingId) {
        bookingId = ((await card.getAttribute('data-booking-id')) ?? '').trim();
      }
      bookingNo = (await card.locator('.xc-booking-no').innerText()).trim();
      expect(bookingId, 'booked bookingId').not.toBe('');
      expect(bookingNo, 'bookingNo on /app/bookings').not.toBe('');
      await shot(page, 'app', 'f21-1-booked');

      const initialSlotsLoaded = page
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${tomorrow}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await card.locator('[data-action="reschedule"]').click();
      const dialog = page.locator('.xc-reschedule-dialog');
      await expect(dialog, 'reschedule dialog visible').toBeVisible({ timeout: 15_000 });
      const dateInput = dialog.locator('[data-field="serviceDate"] input[name="serviceDate"]');
      await expect(dateInput, 'reschedule date input defaults to tomorrow').toHaveValue(tomorrow);
      await initialSlotsLoaded;
      await expect(dialog.locator('.xc-slots-loading')).toBeHidden({ timeout: 15_000 });

      // Step 3: change the date control to the day after tomorrow and pick an open slot.
      const dayAfterSlotsLoaded = page
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${dayAfter}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await dateInput.fill(dayAfter);
      await dayAfterSlotsLoaded;
      await expect(dialog.locator('.xc-slots-loading')).toBeHidden({ timeout: 15_000 });

      const openRescheduleSlots = dialog.locator('input[name="rescheduleSlot"]:not([disabled])');
      if ((await openRescheduleSlots.count()) === 0) {
        const reason = `no open slot on ${dayAfter} (seed)`;
        console.log(`[F21] skip: ${reason}`);
        await dialog.locator('button.xc-btn-secondary').click();
        await expect(dialog, 'reschedule dialog closed before skip').toBeHidden({ timeout: 15_000 });
        await cancelBookingCard(page, bookingId);
        cancelled = true;
        test.skip(true, reason);
      }

      const chosenRadio = openRescheduleSlots.first();
      const chosenOption = dialog
        .locator('.xc-slot-option')
        .filter({ has: page.locator('input[name="rescheduleSlot"]:not([disabled])') })
        .first();
      const newSlotTime = (await chosenOption.locator('.xc-slot-label span').innerText()).trim();
      expect(newSlotTime, 'chosen slot time label').not.toBe('');
      await chosenRadio.check();
      await shot(page, 'app', 'f21-2-reschedule-selected');

      const rescheduleRes = await posted(
        page,
        new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/reschedule$`),
        () => dialog.locator('[data-action="confirm-reschedule"]').click(),
      );
      console.log(`[F21] POST /api/bookings/${bookingId}/reschedule ${rescheduleRes.text}`);
      expect(rescheduleRes.ok, `POST /api/bookings/${bookingId}/reschedule: ${rescheduleRes.text}`).toBeTruthy();
      await expect(dialog, 'reschedule dialog closed after confirm').toBeHidden({ timeout: 15_000 });

      // Assert the card shows the new slot time and its serviceDate is now the day after tomorrow.
      await expect(card.locator('.xc-booking-time'), 'card shows new slot time').toHaveText(newSlotTime, {
        timeout: 15_000,
      });
      await card.locator('[data-action="reschedule"]').click();
      await expect(dialog, 'reschedule dialog reopened to check new date').toBeVisible({ timeout: 15_000 });
      await expect(
        dialog.locator('[data-field="serviceDate"] input[name="serviceDate"]'),
        'booking serviceDate updated to day after tomorrow',
      ).toHaveValue(dayAfter);
      await dialog.locator('button.xc-btn-secondary').click();
      await expect(dialog, 'reschedule dialog closed').toBeHidden({ timeout: 15_000 });
      await shot(page, 'app', 'f21-3-rescheduled');

      // Step 4: cancel the booking (shown at any time, Q-B2 revised) -> card shows CANCELLED.
      await cancelBookingCard(page, bookingId);
      cancelled = true;
      await shot(page, 'app', 'f21-4-cancelled');
    } finally {
      // finally: if step 1 created a booking and it is not yet CANCELLED, cancel it so reruns stay clean.
      try {
        if (bookingId && !cancelled) {
          await signIn(page, 'customer', '/app/bookings');
          await page.goto('/app/bookings');
          await cancelBookingCard(page, bookingId);
          cancelled = true;
        }
      } catch (e) {
        cleanupFailures.push(e instanceof Error ? e.message : String(e));
      }
      expect(cleanupFailures, 'F21 finally cleanup errors').toEqual([]);
    }
  });
});
