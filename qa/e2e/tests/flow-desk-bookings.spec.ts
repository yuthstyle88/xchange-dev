// QA-F17 — flow: desk QR lookup, QR re-issue (old token finds nothing), and desk cancel on /desk/bookings
// (design docs/design/qa-e2e.md §4 row F17; blueprint/booking-slots.md:73,89,227,240,583 BK-3).
// Written by ticket #4159 (QA-F17); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - customer/book/Book.tsx:394-692
//   [data-testid="book-screen"], [data-testid="branch-select"], [data-testid="date-input"],
//   [data-testid="slot-select"], [data-testid="slot-option"], [data-testid="currency-select"],
//   [data-testid="quantity-input"], [data-testid="book-submit"], .xc-book-success
// - customer/bookings/Bookings.tsx:377-494
//   .xc-booking-card[data-booking-id="<id>"], .xc-booking-no, .xc-status-cancelled,
//   [data-qr-token], [data-action="cancel"], [data-action="confirm-cancel"]
// - desk/bookings/DeskBookingsScreen.tsx:618-794,1026-1060
//   [data-screen="desk-bookings"], input[name="qrToken"] (:622), [data-action="lookup-qr"] (:632),
//   .xc-booking-item[data-booking-id="<id>"], .xc-booking-no (:679), .xc-empty-state (:663),
//   [data-action="reissue-qr"] (:727), [data-action="cancel"] (:708),
//   input[name="reason"] (:770), [data-action="confirm-cancel"] (:787)
import { expect, test, type Page } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users, type Role } from './helpers';

const REQUIRED_ROLES: readonly Role[] = ['customer', 'teller'];

// Tomorrow in Asia/Bangkok, as the date input wants it (yyyy-mm-dd), matching flow-booking.spec.ts:7-10.
function tomorrowBangkok(): string {
  const now = new Date(Date.now() + 7 * 3600_000 + 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}

// Lookup a QR token on /desk/bookings and wait for the desk_bookings view response.
async function lookupQrOnDesk(teller: Page, token: string): Promise<void> {
  const viewLoaded = teller.waitForResponse(
    (r) => r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/desk_bookings',
    { timeout: 15_000 },
  );
  await teller.locator('input[name="qrToken"]').fill(token);
  await teller.locator('[data-action="lookup-qr"]').click();
  const res = await viewLoaded;
  expect(res.ok(), `GET /api/v3/views/desk_bookings?qr=${token} answered ${res.status()}`).toBeTruthy();
}

test.describe.serial('F17 desk bookings', () => {
  test('desk looks up booking by QR token, re-issues QR so old token finds nothing, and cancels', async ({ browser }) => {
    const missing = REQUIRED_ROLES.filter((role) => !hasAccount(role));
    if (missing.length > 0) {
      const reason = `missing account in users.json for: ${missing.join(', ')} (run scripts/reseed.sh)`;
      console.log(`[F17] skip: ${reason}`);
      test.skip(true, reason);
    }

    const customer = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
    const teller = await browser.newPage({ baseURL: users.baseUrl });

    const runReason = `e2e-F17-${Date.now()}`;
    let bookingId = '';
    let bookingNo = '';
    let deskCancelled = false;
    const cleanupFailures: string[] = [];

    try {
      // Step 1 (customer): book MAIN for tomorrow, as F1 does; on /app/bookings read that booking's [data-qr-token].
      await signIn(customer, 'customer', '/app/book');
      await expect(customer.getByTestId('book-screen')).toBeVisible({ timeout: 15_000 });
      await customer.getByTestId('branch-select').selectOption(users.branch);

      const tomorrow = tomorrowBangkok();
      const slotsLoaded = customer
        .waitForResponse(
          (r) => r.url().includes('/api/v3/views/booking_slots') && r.url().includes(`date=${tomorrow}`),
          { timeout: 15_000 },
        )
        .catch(() => null);
      await customer.getByTestId('date-input').fill(tomorrow);
      await slotsLoaded;
      await expect(customer.locator('.xc-book-slots-loading')).toBeHidden({ timeout: 15_000 });

      const openSlots = customer.locator('[data-testid="slot-option"]:not([disabled])');
      if ((await openSlots.count()) === 0) {
        const reason = `no open booking slot on ${users.branch} for ${tomorrow} (seed not run)`;
        console.log(`[F17] skip: ${reason}`);
        test.skip(true, reason);
      }

      const slotValue = await openSlots.first().getAttribute('value');
      expect(slotValue, 'first open slot id').toBeTruthy();
      await customer.getByTestId('slot-select').selectOption(slotValue!);
      await customer.getByTestId('currency-select').selectOption(users.currency ?? 'USD');
      await customer.getByTestId('quantity-input').fill(users.quantity ?? '100');

      const bookRes = await posted(customer, /^\/api\/bookings$/, () => customer.getByTestId('book-submit').click());
      console.log(`[F17] POST /api/bookings ${bookRes.text}`);
      expect(bookRes.ok, `POST /api/bookings: ${bookRes.text}`).toBeTruthy();
      await expect(customer.locator('.xc-book-success')).toBeVisible({ timeout: 20_000 });

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

      await customer.goto('/app/bookings');
      const custCard = bookingId
        ? customer.locator(`.xc-booking-card[data-booking-id="${bookingId}"]`)
        : customer.locator('.xc-booking-card').first();
      await expect(custCard, 'booked card on /app/bookings').toBeVisible({ timeout: 15_000 });
      if (!bookingId) {
        bookingId = ((await custCard.getAttribute('data-booking-id')) ?? '').trim();
      }
      bookingNo = (await custCard.locator('.xc-booking-no').innerText()).trim();
      expect(bookingId, 'booked bookingId').not.toBe('');
      expect(bookingNo, 'bookingNo read from /app/bookings').not.toBe('');

      const qrEl = custCard.locator('[data-qr-token]');
      await expect(qrEl, '[data-qr-token] on customer booking card').toBeVisible({ timeout: 15_000 });
      const oldToken = ((await qrEl.getAttribute('data-qr-token')) ?? '').trim();
      expect(oldToken, 'non-empty QR token on /app/bookings').not.toBe('');
      await shot(customer, 'desk', 'f17-1-customer-booked');
      console.log(`[F17] booked id=${bookingId} bookingNo=${bookingNo}`);

      // Step 2 (teller on /desk/bookings): qrToken = the token, lookup-qr -> exactly that booking shows.
      await signIn(teller, 'teller', '/desk/bookings');
      await expect(teller.locator('[data-screen="desk-bookings"]')).toBeVisible({ timeout: 20_000 });
      await lookupQrOnDesk(teller, oldToken);

      const deskItems = teller.locator('.xc-booking-item');
      await expect(deskItems, 'lookup-qr shows exactly one booking').toHaveCount(1, { timeout: 15_000 });
      const deskRow = teller.locator(`.xc-booking-item[data-booking-id="${bookingId}"]`);
      await expect(deskRow, `desk row for ${bookingId}`).toBeVisible({ timeout: 15_000 });
      await expect(deskRow.locator('.xc-booking-no')).toHaveText(bookingNo);
      await shot(teller, 'desk', 'f17-2-desk-qr-found');

      // Step 3 (teller): reissue-qr on it -> ok.
      const reissueRes = await posted(
        teller,
        new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/qr$`),
        () => deskRow.locator('[data-action="reissue-qr"]').click(),
      );
      console.log(`[F17] POST /api/bookings/${bookingId}/qr ${reissueRes.text}`);
      expect(reissueRes.ok, `POST /api/bookings/${bookingId}/qr: ${reissueRes.text}`).toBeTruthy();

      let newToken = '';
      try {
        const parsed = JSON.parse(reissueRes.body) as {
          qrToken?: string;
          data?: { qrToken?: string };
        };
        newToken = String((parsed.data ?? parsed).qrToken ?? '').trim();
      } catch {
        // fallback to unfiltered list if needed
      }
      expect(newToken, 're-issued qrToken is non-empty and rotated').not.toBe('');
      expect(newToken, 're-issued qrToken differs from oldToken').not.toBe(oldToken);

      const qrModal = teller.locator('.xc-modal-overlay');
      await expect(qrModal, 'QR reissue dialog visible').toBeVisible({ timeout: 15_000 });
      await shot(teller, 'desk', 'f17-3-desk-qr-reissued');
      await qrModal.locator('button').click();
      await expect(qrModal, 'QR reissue dialog closed').toBeHidden({ timeout: 15_000 });

      // Step 4 (teller): look up the old token again -> nothing found (the empty or not-found state).
      await lookupQrOnDesk(teller, oldToken);
      await expect(teller.locator('.xc-booking-item'), 'old QR token finds no booking items').toHaveCount(0, {
        timeout: 15_000,
      });
      await expect(teller.locator('.xc-empty-state'), 'empty state shown for old QR token').toBeVisible({
        timeout: 15_000,
      });
      await shot(teller, 'desk', 'f17-4-old-qr-empty');

      // Step 5 (teller): cancel it: reason = a run-unique reason, confirm-cancel -> ok.
      await lookupQrOnDesk(teller, newToken);
      await expect(deskRow, `desk row for ${bookingId} found with new QR token`).toBeVisible({ timeout: 15_000 });
      await deskRow.locator('[data-action="cancel"]').click();

      const cancelModal = teller.locator('.xc-modal-overlay');
      await expect(cancelModal, 'desk cancel modal visible').toBeVisible({ timeout: 15_000 });
      await cancelModal.locator('input[name="reason"]').fill(runReason);

      const cancelRes = await posted(
        teller,
        new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/cancel$`),
        () => cancelModal.locator('[data-action="confirm-cancel"]').click(),
      );
      console.log(`[F17] POST /api/bookings/${bookingId}/cancel ${cancelRes.text}`);
      expect(cancelRes.ok, `POST /api/bookings/${bookingId}/cancel: ${cancelRes.text}`).toBeTruthy();
      await expect(cancelModal, 'desk cancel modal closed after confirm').toBeHidden({ timeout: 15_000 });
      await expect(deskRow.locator('.xc-status-CANCELLED'), 'desk row shows CANCELLED').toBeVisible({
        timeout: 15_000,
      });
      deskCancelled = true;
      await shot(teller, 'desk', 'f17-5-desk-cancelled');

      // Step 6 (customer): reload /app/bookings -> that booking shows CANCELLED.
      await customer.reload();
      await expect(custCard, `customer booking card ${bookingId} after reload`).toBeVisible({ timeout: 15_000 });
      await expect(custCard.locator('.xc-status-cancelled'), 'customer card has .xc-status-cancelled').toBeVisible({
        timeout: 15_000,
      });
      await expect(
        custCard.getByText(/ยกเลิกแล้ว|Cancelled|已取消|CANCELLED/i),
        `${bookingNo} shows CANCELLED on /app/bookings`,
      ).toBeVisible({ timeout: 15_000 });
      await shot(customer, 'desk', 'f17-6-customer-cancelled');
    } finally {
      // finally (customer): if step 1 booked and step 5 did not cancel, cancel it from /app/bookings so no booking is left open.
      try {
        if (bookingId && !deskCancelled) {
          await signIn(customer, 'customer', '/app/bookings');
          await customer.goto('/app/bookings');
          const card = customer.locator(`.xc-booking-card[data-booking-id="${bookingId}"]`);
          await expect(card, `cleanup: booking card ${bookingId} on /app/bookings`).toBeVisible({ timeout: 15_000 });
          const cancelBtn = card.locator('[data-action="cancel"]');
          if (await cancelBtn.isVisible().catch(() => false)) {
            await cancelBtn.click();
            const confirmBtn = customer.locator('[data-action="confirm-cancel"]');
            await expect(confirmBtn, 'cleanup: customer confirm-cancel button').toBeVisible({ timeout: 15_000 });
            const cleanRes = await posted(
              customer,
              new RegExp(`^/api/bookings/${encodeURIComponent(bookingId)}/cancel$`),
              () => confirmBtn.click(),
            );
            console.log(`[F17] finally: customer POST /api/bookings/${bookingId}/cancel ${cleanRes.text}`);
            expect(cleanRes.ok, `cleanup cancel ${bookingId}: ${cleanRes.text}`).toBeTruthy();
          }
          await expect(card.locator('.xc-status-cancelled'), `cleanup: ${bookingId} is CANCELLED`).toBeVisible({
            timeout: 15_000,
          });
        }
      } catch (e) {
        cleanupFailures.push(e instanceof Error ? e.message : String(e));
      } finally {
        await customer.close().catch(() => {});
        await teller.close().catch(() => {});
      }
      expect(cleanupFailures, 'F17 finally cleanup errors').toEqual([]);
    }
  });
});
