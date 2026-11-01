// J2 booked customer journey: today slot, book, check-in, BOOKING ticket called, exchange completes the booking, shift closes
// (ticket #5796; owner order relayed by claude/608dff; design docs/design/qa-e2e.md; docs/design/walkin-queue.md §12).
// Written by ticket #5796; this file is in its File scope.
// Import only from './helpers' and './desk-exchange' (#5795, #5796). Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/:
// - shared/ui/AccountBar.tsx:153,161,170                 .account-bar, span, button (t('shell.account.branch'))
// - screens/admin/AdminShell.tsx:243-261,675-681         header.menu-heading p.subtitle ("· <me.branchName>")
// - screens/admin/booking-slots/BookingSlots.tsx:383,399-432 .booking-slots-screen, input[name="slotMinutes"], input[name="capacity"], input[name="horizonDays"], Save Rule button (i18n.ts:15,65)
// - screens/customer/book/Book.tsx:394-692               [data-testid="book-screen"], [data-testid="branch-select"], [data-testid="date-input"], [data-testid="slot-select"], [data-testid="currency-select"], [data-testid="quantity-input"], [data-testid="book-submit"], .xc-book-success
// - screens/customer/bookings/Bookings.tsx:377-494       .xc-booking-card[data-booking-id="<id>"], [data-action="cancel"], [data-action="confirm-cancel"], .xc-status-cancelled
// - screens/desk/bookings/DeskBookingsScreen.tsx:30,618-704 [data-screen="desk-bookings"], .xc-booking-item[data-booking-id="<id>"], .xc-booking-no, .xc-ticket-number, .xc-status-CHECKED_IN, [data-action="check-in"]
// - screens/desk/queue/board/Board.tsx:43,69-71          li[data-ticket="<id>"]
// - screens/desk/queue/actions/TicketActions.tsx:58-63,100-140 button[name="call"], button[name="start"], button[name="finish"], button[name="cancel"], input[name="cancelReason"], button[name="cancelConfirm"]
// - screens/desk/steps/customer/step.tsx:243,269,279,293,331,354 input[name="passport"], input[name="name"], input[name="passportNo"], select[name="nationality"], input[name="identityChecked"], form.desk-form button[type="submit"] (i18n.ts:15,46)
// - screens/desk/steps/booking/step.tsx:113              [data-row="held-booking"]
// - screens/desk/steps/receipt/step.tsx:32-34            section.desk-form strong (/^XC-\d{8}-\d{6}$/)
import { expect, test, type Page } from '@playwright/test';
import { runDeskExchange } from './desk-exchange';
import {
  closeQueueIfOpened,
  closeShift,
  cutOff,
  ensureQueueOpen,
  hasAccount,
  openShift,
  posted,
  shot,
  signIn,
  startService,
  users,
  type Role,
} from './helpers';

const REQUIRED_ROLES: readonly Role[] = ['admin', 'customer', 'teller', 'branchManager'];
const SAVE_RULE_BTN = /^(Save กฎการจอง|Save Rule)$/;

/** 1x1 transparent PNG for passport file upload on the desk customer step (flow-desk-extras.spec.ts:63-66,519-523). */
const ONE_PIXEL_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
);

interface MeViewAnswer {
  data?: {
    branch?: string;
    branchName?: string;
  };
}

interface StaffMeViewAnswer {
  data?: {
    branchId?: string;
    branchName?: string;
  };
}

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

interface QueueBoardTicketItem {
  id: string;
  displayNumber: string;
  serviceType: string;
  status?: string;
}

interface QueueBoardEnvelope {
  data?: {
    waiting?: QueueBoardTicketItem[];
    called?: QueueBoardTicketItem[];
  };
}

interface MyBookingItem {
  id: string;
  bookingNo: string;
  status: string;
}

interface MyBookingsEnvelope {
  data?: {
    bookings?: MyBookingItem[];
  };
}

interface MyShiftAnswer {
  data?: {
    id: string;
    status: string;
  } | null;
}

/** Today in Asia/Bangkok (yyyy-mm-dd), matching Book.tsx getTodayInTimezone('Asia/Bangkok'). */
function todayBangkok(): string {
  const now = new Date(Date.now() + 7 * 3600_000);
  return now.toISOString().slice(0, 10);
}

/**
 * Asserts the page operates under users.branch via /api/v3/views/me, staff_me, and .account-bar on screen.
 * When `opts.adminSubtitle` is true (or `header.menu-heading p.subtitle` is present on the admin screen),
 * also checks that `header.menu-heading p.subtitle` ends with `· <me.branchName>` (AdminShell.tsx:675-681).
 */
async function assertActingBranch(
  page: Page,
  label: string,
  opts?: { adminSubtitle?: boolean },
): Promise<void> {
  const meRes = await page.request.get('/api/v3/views/me');
  expect(meRes.ok(), `[${label}] GET /api/v3/views/me: ${meRes.status()}`).toBeTruthy();
  const meBody = (await meRes.json().catch(() => ({}))) as MeViewAnswer;
  const meBranch = meBody.data?.branch;
  const branchName = meBody.data?.branchName ?? '';
  expect(
    meBranch,
    `assertActingBranch [${label}] me.branch mismatch: got "${meBranch}", expected "${users.branch}"`,
  ).toBe(users.branch);

  const staffRes = await page.request.get('/api/v3/views/staff_me');
  expect(staffRes.ok(), `[${label}] GET /api/v3/views/staff_me: ${staffRes.status()}`).toBeTruthy();
  const staffBody = (await staffRes.json().catch(() => ({}))) as StaffMeViewAnswer;
  const staffBranchId = staffBody.data?.branchId;
  expect(
    staffBranchId,
    `assertActingBranch [${label}] staff_me.branchId mismatch: got "${staffBranchId}", expected "${users.branch}"`,
  ).toBe(users.branch);

  const accountBar = page.locator('.account-bar');
  await expect(accountBar, `assertActingBranch [${label}] .account-bar visible`).toBeVisible({ timeout: 20_000 });
  const escapedName = branchName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  await expect(
    accountBar,
    `assertActingBranch [${label}] .account-bar shows branch "${branchName}"`,
  ).toContainText(new RegExp(`(สาขา|Branch|分店)\\s+${escapedName}`));

  const subtitle = page.locator('header.menu-heading p.subtitle');
  if (opts?.adminSubtitle || (await subtitle.count()) > 0) {
    await expect(
      subtitle.first(),
      `assertActingBranch [${label}] header.menu-heading p.subtitle ends with "· ${branchName}" (AdminShell.tsx:675-681)`,
    ).toHaveText(new RegExp(`·\\s*${escapedName}\\s*$`), { timeout: 20_000 });
  }
}

/** Reads the teller's active shift from GET /api/v3/views/my_shift. */
async function readMyShift(page: Page): Promise<NonNullable<MyShiftAnswer['data']> | null> {
  const res = await page.request.get(`/api/v3/views/my_shift?branch=${encodeURIComponent(users.branch)}`);
  expect(res.ok(), `GET /api/v3/views/my_shift: ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as MyShiftAnswer;
  return body.data ?? null;
}

/** Reads a booking by id from the customer's GET /api/v3/views/my_bookings. */
async function readMyBooking(customer: Page, bookingId: string): Promise<MyBookingItem | undefined> {
  const res = await customer.request.get('/api/v3/views/my_bookings');
  expect(
    res.ok(),
    `GET /api/v3/views/my_bookings: ${res.status()} ${await res.text().catch(() => '')}`,
  ).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as MyBookingsEnvelope;
  return (body.data?.bookings ?? []).find((b) => b.id === bookingId);
}

/** Passport number fixed per customer account so reruns against the same seed match (#5862). */
function passportNoFor(phone: string): string {
  const digits = phone.replace(/\D/g, '');
  return `J2-${digits}`.slice(0, 30);
}

test.describe.serial('J2 booked customer', () => {
  test('J2 booked customer: books a today slot, arrives, desk check-in, BOOKING ticket called, exchange completes the booking, shift closes', async ({
    browser,
  }) => {
    test.setTimeout(300_000);

    for (const role of REQUIRED_ROLES) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[J2] skip: ${reason}`);
        test.skip(true, reason);
      }
    }

    const admin = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const customer = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    const today = todayBangkok();
    let bookingId = '';
    let bookingNo = '';
    let checkedIn = false;
    let ticketId = '';
    let ticketNumber = '';
    let ticketEnded = false;
    let openedShift = false;
    let openedQueue = false;
    let started: 'started' | 'alreadyOpen' | undefined;
    const cleanupFailures: string[] = [];

    try {
      // Step 1: Role guard done above; sign in all four roles and assertActingBranch on admin, teller and manager.
      await signIn(admin, 'admin', '/#booking-slots');
      await signIn(customer, 'customer', '/app/book');
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');

      await expect(admin.locator('.booking-slots-screen')).toBeVisible({ timeout: 20_000 });
      await assertActingBranch(admin, 'admin on /#booking-slots', { adminSubtitle: true });
      await assertActingBranch(teller, 'teller on /staff');
      await assertActingBranch(manager, 'manager on /staff');

      // Step 2: Today slot — on /#booking-slots, read the three rule inputs (BookingSlots.tsx:399-428)
      // and save them unchanged with Save Rule (PUT /api/branches/<MAIN>/booking-slot-rule, expect ok).
      await expect(admin.locator('.booking-slots-screen .loading')).toHaveCount(0, { timeout: 15_000 });
      const saveRuleBtn = admin.getByRole('button', { name: SAVE_RULE_BTN });
      const slotMinInput = admin.locator('input[name="slotMinutes"]');
      const capInput = admin.locator('input[name="capacity"]');
      const horizonInput = admin.locator('input[name="horizonDays"]');
      await expect(saveRuleBtn).toBeEnabled({ timeout: 15_000 });
      await expect(slotMinInput).not.toHaveValue('', { timeout: 15_000 });
      await expect(capInput).not.toHaveValue('', { timeout: 15_000 });
      await expect(horizonInput).not.toHaveValue('', { timeout: 15_000 });

      const slotMinutes = await slotMinInput.inputValue();
      const capacity = await capInput.inputValue();
      const horizonDays = await horizonInput.inputValue();
      await slotMinInput.fill(slotMinutes);
      await capInput.fill(capacity);
      await horizonInput.fill(horizonDays);

      const ruleSaveRes = await posted(
        admin,
        new RegExp(`^/api/branches/${encodeURIComponent(users.branch)}/booking-slot-rule$`),
        () => saveRuleBtn.click(),
        'PUT',
      );
      console.log(`[J2] PUT /api/branches/${users.branch}/booking-slot-rule ${ruleSaveRes.text}`);
      expect(ruleSaveRes.ok, `PUT /api/branches/${users.branch}/booking-slot-rule: ${ruleSaveRes.text}`).toBeTruthy();
      await expect(admin.locator('.booking-slots-screen .loading')).toHaveCount(0, { timeout: 15_000 });

      const slotsRes = await customer.request.get(
        `/api/v3/views/booking_slots?branch=${encodeURIComponent(users.branch)}&date=${encodeURIComponent(today)}`,
      );
      const slotsText = await slotsRes.text().catch(() => '');
      expect(slotsRes.ok(), `GET /api/v3/views/booking_slots: ${slotsRes.status()} ${slotsText}`).toBeTruthy();
      const slotsBody = JSON.parse(slotsText) as BookingSlotsEnvelope;
      const allSlots = slotsBody.data?.slots ?? [];
      const nowMs = slotsBody.data?.asOf ? Date.parse(slotsBody.data.asOf) : Date.now();
      const openLaterSlots = allSlots.filter((s) => s.available > 0 && Date.parse(s.start) > nowMs);
      expect(
        openLaterSlots.length,
        `expected an OPEN slot later than now (${slotsBody.data?.asOf ?? new Date(nowMs).toISOString()}) with available > 0 on ${users.branch} ${today}; got slots: ${JSON.stringify(allSlots)}`,
      ).toBeGreaterThan(0);
      const targetSlot = openLaterSlots[0]!;
      console.log(`[J2] today slot id=${targetSlot.id} start=${targetSlot.start} available=${targetSlot.available}`);

      // Step 3: openShift, startService, ensureQueueOpen.
      await openShift(teller, manager);
      openedShift = true;
      started = await startService(manager);
      openedQueue = await ensureQueueOpen(manager);

      // Step 4: Book — customer books that slot on /app/book (flow-desk-check-in.spec.ts:186-221).
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
      console.log(`[J2] POST /api/bookings ${bookRes.text}`);
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
      await shot(customer, 'desk', 'j2-1-customer-booked');
      console.log(`[J2] booked id=${bookingId} bookingNo=${bookingNo}`);

      // Step 5: Arrive and check in — teller checks in on /desk/bookings (flow-desk-check-in.spec.ts:223-258).
      await teller.goto('/desk/bookings');
      await expect(teller.locator('[data-screen="desk-bookings"]')).toBeVisible({ timeout: 20_000 });
      await assertActingBranch(teller, 'teller on /desk/bookings');

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
      console.log(`[J2] POST /api/bookings/${bookingId}/check-in ${checkInRes.text}`);
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
      await shot(teller, 'desk', 'j2-2-desk-checked-in');

      // Step 6: Call and start — on /desk/queue, the BOOKING ticket's row gets button[name="call"], then button[name="start"].
      // The customer's my_bookings view then shows the booking IN_SERVICE (B9).
      await teller.goto('/desk/queue');
      await assertActingBranch(teller, 'teller on /desk/queue before call/start');

      const boardRes = await teller.request.get(
        `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
      );
      expect(
        boardRes.ok(),
        `GET /api/v3/views/queue_board after check-in: ${boardRes.status()} ${await boardRes.text().catch(() => '')}`,
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

      const callBtn = ticketRow.locator('button[name="call"]');
      await expect(callBtn, 'call button on BOOKING ticket (TicketActions.tsx:58-59)').toBeVisible({
        timeout: 20_000,
      });
      const calledRes = await posted(
        teller,
        new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketId)}/call$`),
        () => callBtn.click(),
      );
      console.log(`[J2] POST /api/queue/tickets/${ticketId}/call ${calledRes.text}`);
      expect(calledRes.ok, `call BOOKING ticket ${ticketId}: ${calledRes.text}`).toBeTruthy();
      await expect(ticketRow, `board shows ${ticketNumber} as CALLED`).toContainText('CALLED', {
        timeout: 20_000,
      });

      const startBtn = ticketRow.locator('button[name="start"]');
      await expect(startBtn, 'start button on CALLED BOOKING ticket (TicketActions.tsx:59,100-107)').toBeVisible({
        timeout: 20_000,
      });
      const startedRes = await posted(
        teller,
        new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketId)}/start$`),
        () => startBtn.click(),
      );
      console.log(`[J2] POST /api/queue/tickets/${ticketId}/start ${startedRes.text}`);
      expect(startedRes.ok, `start BOOKING ticket ${ticketId}: ${startedRes.text}`).toBeTruthy();
      await expect(ticketRow, `board shows ${ticketNumber} as SERVING`).toContainText(/SERVING|Serving/, {
        timeout: 20_000,
      });

      const bookingInService = await readMyBooking(customer, bookingId);
      expect(bookingInService, `booking ${bookingId} found in customer my_bookings`).toBeTruthy();
      expect(
        bookingInService?.status,
        `customer my_bookings status for ${bookingId} after ticket start (B9)`,
      ).toBe('IN_SERVICE');

      // Step 7: Exchange tied to the booking (#5799, walkin-queue.md §12).
      await assertActingBranch(teller, 'teller before booked BUY sale');

      let reservedCustomerId = '';
      const createTxPromise = teller.waitForRequest(
        (req) => {
          try {
            const url = new URL(req.url());
            return req.method() === 'POST' && /^\/api\/transactions$/.test(url.pathname);
          } catch {
            return false;
          }
        },
        { timeout: 120_000 },
      );

      const receipt = await runDeskExchange(
        teller,
        manager,
        'BUY',
        (opened) => {
          if (opened) openedQueue = true;
        },
        {
          customer: async (tPage) => {
            await tPage.locator('input[name="passport"]').setInputFiles({
              name: 'passport.png',
              mimeType: 'image/png',
              buffer: ONE_PIXEL_PNG,
            });
            await tPage.locator('input[name="name"]').fill('J2 BOOKED');
            const passportNo = passportNoFor(users.customer.phone ?? '');
            await tPage.locator('input[name="passportNo"]').fill(passportNo);
            await tPage.locator('select[name="nationality"]').selectOption('THA');
            await tPage.locator('input[name="identityChecked"]').check();

            const reserveReqPromise = tPage.waitForRequest(
              (req) => {
                try {
                  const url = new URL(req.url());
                  if (req.method() !== 'POST' || !/^\/api\/rpc$/.test(url.pathname)) return false;
                  const body = req.postDataJSON() as { action?: string } | null;
                  return body?.action === 'reserveCustomer';
                } catch {
                  return false;
                }
              },
              { timeout: 30_000 },
            );

            await tPage.locator('form.desk-form button[type="submit"]').click();
            const reserveReq = await reserveReqPromise;
            const reserveSent = reserveReq.postDataJSON() as {
              action?: string;
              payload?: { bookingId?: string };
            };
            expect(
              reserveSent.payload?.bookingId,
              `reserveCustomer payload.bookingId matches bookingId (${bookingId})`,
            ).toBe(bookingId);

            const reserveRes = await reserveReq.response();
            expect(reserveRes, 'reserveCustomer response received').not.toBeNull();
            const reserveText = await reserveRes!.text().catch(() => '');
            console.log(`[J2] POST /api/rpc (reserveCustomer) ${reserveRes!.status()} ${reserveText}`);
            const isPassportMismatch =
              reserveRes!.status() === 400 &&
              reserveText.includes('พาสปอร์ตนี้ไม่ใช่ของลูกค้าที่จองคิวนี้');
            expect(
              reserveRes!.ok(),
              isPassportMismatch
                ? `customer ${users.customer.phone} already holds another passport from a run before #5862: run scripts/reseed.sh for a fresh customer`
                : `reserveCustomer refused: ${reserveRes!.status()} ${reserveText}`,
            ).toBeTruthy();

            const reserveParsed = JSON.parse(reserveText) as {
              customerId?: string;
              existing?: boolean;
              data?: { customerId?: string; existing?: boolean };
            };
            const reserveData = reserveParsed.data ?? reserveParsed;
            expect(
              reserveData.existing,
              `reserveCustomer existing: true (C1.2): ${reserveText}`,
            ).toBe(true);
            reservedCustomerId = String(reserveData.customerId ?? '').trim();
            expect(reservedCustomerId, `reserveCustomer customerId: ${reserveText}`).not.toBe('');

            const heldBookingRow = tPage.locator('[data-row="held-booking"]');
            await expect(
              heldBookingRow,
              `[data-row="held-booking"] contains ticket number ${ticketNumber} (steps/booking/step.tsx:113)`,
            ).toContainText(ticketNumber, { timeout: 15_000 });
          },
        },
      );

      const createReq = await createTxPromise;
      const createPostData = createReq.postDataJSON() as {
        bookingId?: string | null;
        customerId?: string | null;
        queueTicketId?: string | null;
      };
      expect(
        createPostData.bookingId,
        `POST /api/transactions request bookingId matches ${bookingId}`,
      ).toBe(bookingId);
      expect(
        createPostData.customerId,
        'POST /api/transactions request customerId matches the booking customer from reserveCustomer',
      ).toBe(reservedCustomerId);
      expect(receipt, 'receipt documentRef matches /^XC-\\d{8}-\\d{6}$/').toMatch(/^XC-\d{8}-\d{6}$/);

      const bookingCompleted = await readMyBooking(customer, bookingId);
      expect(bookingCompleted, `booking ${bookingId} found in customer my_bookings after sale`).toBeTruthy();
      expect(
        bookingCompleted?.status,
        `customer my_bookings status for ${bookingId} after exchange completion (lifecycle.rs:141)`,
      ).toBe('COMPLETED');

      // Step 8: The BOOKING ticket leaves the board. Finish the ticket on /desk/queue before closing the shift.
      await teller.goto('/desk/queue');
      await assertActingBranch(teller, 'teller on /desk/queue before finishing BOOKING ticket');

      const servingRow = teller.locator(`li[data-ticket="${ticketId}"]`);
      await expect(servingRow, `SERVING BOOKING ticket ${ticketNumber} row on /desk/queue`).toBeVisible({
        timeout: 20_000,
      });
      const finishBtn = servingRow.locator('button[name="finish"]');
      await expect(finishBtn, 'finish button on SERVING BOOKING ticket (flow-queue.spec.ts:152)').toBeVisible({
        timeout: 20_000,
      });
      const finishedRes = await posted(
        teller,
        new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketId)}/finish$`),
        () => finishBtn.click(),
      );
      console.log(`[J2] POST /api/queue/tickets/${ticketId}/finish ${finishedRes.text}`);
      expect(finishedRes.ok, `finish BOOKING ticket ${ticketId}: ${finishedRes.text}`).toBeTruthy();
      await expect(
        servingRow,
        `finished BOOKING ticket ${ticketNumber} leaves the called/serving list`,
      ).toHaveCount(0, { timeout: 20_000 });
      ticketEnded = true;
      await shot(teller, 'desk', 'j2-3-ticket-finished');

      const bookingStillCompleted = await readMyBooking(customer, bookingId);
      expect(
        bookingStillCompleted?.status,
        `booking ${bookingId} stays COMPLETED after finishing BOOKING ticket`,
      ).toBe('COMPLETED');
    } finally {
      // If the test failed after check-in, cancel the BOOKING ticket and the booking (flow-desk-check-in.spec.ts:296-366),
      // then close the queue (if opened), cut off service (if started), and close the shift (step 9).
      try {
        if ((ticketId || ticketNumber) && !ticketEnded) {
          await teller.goto('/desk/queue');
          if (!ticketId && ticketNumber) {
            const bRes = await teller.request.get(
              `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
            );
            if (bRes.ok()) {
              const bBody = (await bRes.json()) as QueueBoardEnvelope;
              const allTickets = [...(bBody.data?.waiting ?? []), ...(bBody.data?.called ?? [])];
              ticketId = allTickets.find((t) => t.displayNumber === ticketNumber)?.id ?? '';
            }
          }
          if (ticketId) {
            const row = teller.locator(`li[data-ticket="${ticketId}"]`);
            if (await row.isVisible({ timeout: 10_000 }).catch(() => false)) {
              const cancelBtn = row.locator('button[name="cancel"]');
              if (await cancelBtn.isVisible().catch(() => false)) {
                await cancelBtn.click();
                await row.locator('input[name="cancelReason"]').fill(`e2e-J2-cleanup-${Date.now()}`);
                const cleanTicketRes = await posted(
                  teller,
                  new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketId)}/cancel$`),
                  () => row.locator('button[name="cancelConfirm"]').click(),
                );
                console.log(`[J2] finally: POST /api/queue/tickets/${ticketId}/cancel ${cleanTicketRes.text}`);
                expect(cleanTicketRes.ok, `cleanup cancel ticket ${ticketId}: ${cleanTicketRes.text}`).toBeTruthy();
                await expect(row).toHaveCount(0, { timeout: 15_000 });
              }
            }
          }
        }
      } catch (e) {
        cleanupFailures.push(`ticket cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

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
            console.log(`[J2] finally: POST /api/bookings/${bookingId}/cancel ${cleanBookingRes.text}`);
            expect(cleanBookingRes.ok, `cleanup cancel booking ${bookingId}: ${cleanBookingRes.text}`).toBeTruthy();
          }
          await expect(card.locator('.xc-status-cancelled')).toBeVisible({ timeout: 15_000 });
        }
      } catch (e) {
        cleanupFailures.push(`booking cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      try {
        await closeQueueIfOpened(manager, openedQueue);
      } catch (e) {
        cleanupFailures.push(`queue cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      try {
        if (started === 'started') await cutOff(manager);
      } catch (e) {
        cleanupFailures.push(`cutOff cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      // Step 9: Close — closeShift, then my_shift is null.
      try {
        if (openedShift) {
          await teller.goto('/staff');
          await manager.goto('/staff');
          await closeShift(teller, manager);
          const closedShift = await readMyShift(teller);
          expect(closedShift, 'GET my_shift is null after closeShift').toBeNull();
        }
      } catch (e) {
        cleanupFailures.push(`shift cleanup: ${e instanceof Error ? e.message : String(e)}`);
      }

      await Promise.all([
        admin.close().catch(() => {}),
        customer.close().catch(() => {}),
        teller.close().catch(() => {}),
        manager.close().catch(() => {}),
      ]);
      expect(cleanupFailures, 'J2 finally cleanup errors').toEqual([]);
    }
  });
});
