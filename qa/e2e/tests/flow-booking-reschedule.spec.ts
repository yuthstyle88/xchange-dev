// QA-F21 — flow: booking reschedule to another date, then cancel at any time
// (design docs/design/qa-e2e.md §4 row F21; blueprint/booking-slots.md Q-B1, Q-B2 revised; #4204, #5079).
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
import { expect, test, type Browser, type Page } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users } from './helpers';

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

// Reads cancel cutoff minutes: from customer's existing booking in my_bookings, or from admin_booking_slots.rs:140.
async function readCancelCutoffMinutes(page: Page, browser: Browser): Promise<number> {
  try {
    const myBookingsRes = await page.request.get('/api/v3/views/my_bookings');
    if (myBookingsRes.ok()) {
      const data = (await myBookingsRes.json()) as {
        data?: { bookings?: Array<{ startAt?: string; cancelCutoffAt?: string | null }> } | null;
      };
      const existing = (data?.data?.bookings ?? []).find((b) => b.startAt && b.cancelCutoffAt);
      if (existing?.startAt && existing?.cancelCutoffAt) {
        const startMs = new Date(existing.startAt).getTime();
        const cutoffMs = new Date(existing.cancelCutoffAt).getTime();
        const diffMins = Math.round((startMs - cutoffMs) / 60_000);
        if (diffMins >= 0) return diffMins;
      }
    }
  } catch {
    // try admin view below
  }

  if (hasAccount('admin')) {
    const adminCtx = await browser.newContext();
    try {
      const adminPage = await adminCtx.newPage();
      await signIn(adminPage, 'admin', '/');
      const res = await adminPage.request.get(
        `/api/v3/views/admin_booking_slots?branch=${encodeURIComponent(users.branch)}`,
      );
      if (res.ok()) {
        const data = (await res.json()) as { data?: { policy?: { cancelCutoffMinutes?: number } | null } | null };
        // No policy row: the app's own default, COALESCE(p.cancel_cutoff_minutes, 60) (my_bookings.rs:135).
        return data?.data?.policy?.cancelCutoffMinutes ?? 60;
      }
    } catch {
      // fallback to default
    } finally {
      await adminCtx.close().catch(() => {});
    }
  }

  throw new Error('cancel cutoff unreadable: no my_bookings booking with cancelCutoffAt and no admin_booking_slots policy');
}

test.use({ viewport: { width: 430, height: 932 } });

test.describe.serial('F21 booking reschedule', () => {
  test('customer books tomorrow, reschedules to the day after tomorrow, and cancels at any time', async ({
    page,
    browser,
  }) => {
    let bookingId = '';
    let bookingNo = '';
    let cancelled = false;
    const cleanupFailures: string[] = [];

    try {
      // Step 1: book MAIN for the first open slot whose start - cancel_cutoff_minutes > now + 10 min.
      await signIn(page, 'customer', '/app/book');
      await expect(page.getByTestId('book-screen')).toBeVisible({ timeout: 15_000 });
      await page.getByTestId('branch-select').selectOption(users.branch);

      const cancelCutoffMinutes = await readCancelCutoffMinutes(page, browser);
      console.log(`[F21] cancel cutoff: ${cancelCutoffMinutes} min`);

      let bookedDayOffset = 1;
      let bookedDate = '';
      let bookedSlotId = '';

      for (let dayOffset = 1; dayOffset <= 7; dayOffset++) {
        const candidateDate = bangkokDate(dayOffset);
        const slotsPromise = page
          .waitForResponse(
            (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${candidateDate}`),
            { timeout: 15_000 },
          )
          .catch(() => null);
        await page.getByTestId('date-input').fill(candidateDate);
        const slotsRes = await slotsPromise;
        await expect(page.locator('.xc-book-slots-loading')).toBeHidden({ timeout: 15_000 });

        if (slotsRes && slotsRes.ok()) {
          const data = (await slotsRes.json().catch(() => ({}))) as {
            data?: { slots?: Array<{ id: string; start: string; available: number }> } | null;
          };
          const slots = data.data?.slots ?? [];
          const now = Date.now();
          const qualifying = slots.find((s) => {
            if (s.available <= 0) return false;
            const startMs = new Date(s.start).getTime();
            return startMs - cancelCutoffMinutes * 60_000 > now + 10 * 60_000;
          });

          if (qualifying) {
            bookedDayOffset = dayOffset;
            bookedDate = candidateDate;
            bookedSlotId = qualifying.id;
            break;
          }
        }
      }

      if (!bookedSlotId) {
        const reason = `no qualifying open slot within 7 days (cutoff ${cancelCutoffMinutes} min)`;
        console.log(`[F21] skip: ${reason}`);
        test.skip(true, reason);
      }

      await page.getByTestId('slot-select').selectOption(bookedSlotId);
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

      // Step 2: on /app/bookings, open the reschedule dialog and verify serviceDate defaults to booked date.
      const myBookingsPromise = page
        .waitForResponse((r) => r.url().includes('/api/v3/views/my_bookings'), { timeout: 15_000 })
        .catch(() => null);
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

      // Before clicking reschedule, assert cancelCutoffAt is in the future, so a wrong pick fails at the cause, not by timeout.
      let cancelCutoffAt: string | null = null;
      const myBookingsRes = await myBookingsPromise;
      if (myBookingsRes && myBookingsRes.ok()) {
        const data = (await myBookingsRes.json().catch(() => ({}))) as {
          data?: { bookings?: Array<{ id?: string; bookingNo?: string; cancelCutoffAt?: string | null }> } | null;
        };
        const currentBooking = (data.data?.bookings ?? []).find(
          (b) => b.id === bookingId || (bookingNo && b.bookingNo === bookingNo),
        );
        cancelCutoffAt = currentBooking?.cancelCutoffAt ?? null;
      }
      if (!cancelCutoffAt) {
        const viewRes = await page.request.get('/api/v3/views/my_bookings');
        if (viewRes.ok()) {
          const data = (await viewRes.json().catch(() => ({}))) as {
            data?: { bookings?: Array<{ id?: string; bookingNo?: string; cancelCutoffAt?: string | null }> } | null;
          };
          const currentBooking = (data.data?.bookings ?? []).find(
            (b) => b.id === bookingId || (bookingNo && b.bookingNo === bookingNo),
          );
          cancelCutoffAt = currentBooking?.cancelCutoffAt ?? null;
        }
      }
      expect(cancelCutoffAt, `cancelCutoffAt for ${bookingId}`).toBeTruthy();
      expect(
        new Date(cancelCutoffAt!).getTime(),
        `cancelCutoffAt (${cancelCutoffAt}) must be in the future before clicking reschedule`,
      ).toBeGreaterThan(Date.now());

      const initialSlotsLoaded = page
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${bookedDate}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await card.locator('[data-action="reschedule"]').click();
      const dialog = page.locator('.xc-reschedule-dialog');
      await expect(dialog, 'reschedule dialog visible').toBeVisible({ timeout: 15_000 });
      const dateInput = dialog.locator('[data-field="serviceDate"] input[name="serviceDate"]');
      await expect(dateInput, 'reschedule date input defaults to booked date').toHaveValue(bookedDate);
      await initialSlotsLoaded;
      await expect(dialog.locator('.xc-slots-loading')).toBeHidden({ timeout: 15_000 });

      // Step 3: change the date control to a subsequent date and pick an open slot.
      let rescheduleDate = bangkokDate(bookedDayOffset + 1);
      const dayAfterSlotsLoaded = page
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${rescheduleDate}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await dateInput.fill(rescheduleDate);
      await dayAfterSlotsLoaded;
      await expect(dialog.locator('.xc-slots-loading')).toBeHidden({ timeout: 15_000 });

      let openRescheduleSlots = dialog.locator('input[name="rescheduleSlot"]:not([disabled])');
      if ((await openRescheduleSlots.count()) === 0) {
        for (let nextOffset = bookedDayOffset + 2; nextOffset <= bookedDayOffset + 7; nextOffset++) {
          const tryDate = bangkokDate(nextOffset);
          const nextLoaded = page
            .waitForResponse(
              (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${tryDate}`),
              { timeout: 15_000 },
            )
            .catch(() => null);
          await dateInput.fill(tryDate);
          await nextLoaded;
          await expect(dialog.locator('.xc-slots-loading')).toBeHidden({ timeout: 15_000 });
          openRescheduleSlots = dialog.locator('input[name="rescheduleSlot"]:not([disabled])');
          if ((await openRescheduleSlots.count()) > 0) {
            rescheduleDate = tryDate;
            break;
          }
        }
      }

      if ((await openRescheduleSlots.count()) === 0) {
        const reason = `no open slot on ${rescheduleDate} (seed)`;
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

      // Assert the card shows the new slot time and its serviceDate is now rescheduleDate.
      await expect(card.locator('.xc-booking-time'), 'card shows new slot time').toHaveText(newSlotTime, {
        timeout: 15_000,
      });
      await card.locator('[data-action="reschedule"]').click();
      await expect(dialog, 'reschedule dialog reopened to check new date').toBeVisible({ timeout: 15_000 });
      await expect(
        dialog.locator('[data-field="serviceDate"] input[name="serviceDate"]'),
        'booking serviceDate updated to rescheduleDate',
      ).toHaveValue(rescheduleDate);
      await dialog.locator('button.xc-btn-secondary').click();
      await expect(dialog, 'reschedule dialog closed').toBeHidden({ timeout: 15_000 });
      await shot(page, 'app', 'f21-3-rescheduled');

      // Step 4: cancel the booking (shown at any time, Q-B2 revised) -> card shows CANCELLED.
      await cancelBookingCard(page, bookingId);
      cancelled = true;
      await shot(page, 'app', 'f21-4-cancelled');
    } finally {
      // finally: if step 1 created a booking and it is not yet CANCELLED, cancel it through a fresh page or API so reruns stay clean.
      try {
        if (bookingId && !cancelled) {
          const cleanContext = await browser.newContext();
          try {
            const cleanPage = await cleanContext.newPage();
            await signIn(cleanPage, 'customer', '/app/bookings');
            try {
              await cancelBookingCard(cleanPage, bookingId);
            } catch (cardErr) {
              const res = await cleanPage.request.post(`/api/bookings/${encodeURIComponent(bookingId)}/cancel`, {
                // command.rs:259 same_origin needs the marker header and Origin (origin.rs:33-48).
                headers: { 'x-xchange-request': '1', Origin: new URL(users.baseUrl).origin },
              });
              if (!res.ok()) {
                throw cardErr;
              }
            }
            cancelled = true;
          } finally {
            await cleanContext.close().catch(() => {});
          }
        }
      } catch (e) {
        cleanupFailures.push(e instanceof Error ? e.message : String(e));
      }
      expect(cleanupFailures, 'F21 finally cleanup errors').toEqual([]);
    }
  });
});
