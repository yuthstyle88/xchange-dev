import { expect, test, type Page } from '@playwright/test';
import {
  clockIn,
  closeShift,
  cutOff,
  ensureShiftClosed,
  hasAccount,
  HTML_LANG,
  openShift,
  posted,
  shot,
  signIn,
   STAFF_LANGS,
  startService,
  users,
  withLang,
  type Lang,
  type Role,
} from './helpers';

// Flow Validation (Staff & Desk, Round 9 — #4296, docs/design/qa-e2e.md §9.1–§9.5):
// Invalid input on the staff and desk forms the earlier rounds walk (3–5 cases per form),
// refused and worded in both `th` and `en` (`withLang`), input kept, nothing created:
//   - desk amount/rate & payment step (`/desk?branch=MAIN`): `quoteCreate`, `transactionCreate`, `transactionConfirm`
//     (0, negative, too many decimals, over the branch limit, empty submit, incomplete payment);
//   - refunds (`/desk/transactions/{id}`, `/desk?branch=MAIN`): `reversalRequest`, `reversalPost`
//     (empty reason, whitespace reason, >500-char reason; post without open shift, cancelled confirm);
//   - shift counts (`/staff`): `countSubmit`, `countVerify`
//     (empty lines, negative, not a number, decimal quantity);
//   - holds (`/staff`): `holdCreate`
//     (no customer picked, empty reason, whitespace-only reason);
//   - cash transfers (`/staff`): `transferCreate`
//     (0, negative, not a number, over the limit);
//   - floor day target (`/staff`): `dayTarget`
//     (negative, not a number, too many decimals);
//   - walk-in ticket (`/desk/queue`): `ticketIssueWalkIn`
//     (issue when branch queue is closed);
//   - desk booking cancel, reschedule & new booking (`/desk/bookings`): `bookingCancel`, `bookingReschedule`
//     (empty cancel reason, whitespace cancel reason, unselected reschedule slot, invalid new-booking quantities).
//
// Selector citations (file:line on main):
//   - web/src/screens/desk/DeskPage.tsx:28 (`.desk-page`)
//   - web/src/screens/desk/steps/rate/step.tsx:54-98,126-137 (`form.desk-form`, `select[name="type"]`, `select[name="currency"]`, `input[name="amount"]`, `.desk-error`, `[role="alert"].desk-error`, `button[type="submit"]`)
//   - web/src/screens/desk/steps/customer/step.tsx:169 (`button[name="skipCustomer"]`)
//   - web/src/screens/desk/steps/booking/step.tsx:54 (`button` confirm booking)
//   - web/src/screens/desk/steps/payment/step.tsx:135,168,209,224,230 (`select[name="method"]`, `input[name="account"]`, `input[name="paymentChecked"]`, `[role="alert"].desk-error`, `button[type="submit"]`)
//   - web/src/screens/desk/steps/receipt/step.tsx:32-34 (`section.desk-form strong`)
//   - web/src/screens/desk-transaction/TransactionScreen.tsx:181,215,225-230 (`.xc-dtx-message[role="status"]`, `button[data-action="request-reversal"]`, `dialog.xc-dialog`)
//   - web/src/screens/staff/parts/ReasonDialog.tsx:32-50 (`dialog.xc-dialog`, `input[type="text"]`, `.xc-dialog-actions button`)
//   - web/src/screens/desk/steps/refunds/task.tsx:53 (`button[data-desk-task="refunds"]`)
//   - web/src/screens/desk/steps/refunds/RefundsStep.tsx:14,54-55,76,87-96,128-136 (`[data-section="refunds"]`, `.desk-refunds-error-msg`, `[data-reversal]`, `button[data-action="post"]`, `dialog.xc-dialog`)
//   - web/src/screens/staff/shift/ShiftCard.tsx:382-485,558-585 (`.xc-shift-card`, `[data-testid="shift-error"]`, `[data-testid="shift-open-count"]`, `[data-testid="shift-count-form"]`, `[data-denomination-row]`, `input[data-denomination-id]`, `[data-testid="shift-count-save"]`, `[data-testid="shift-count-cancel"]`, `[data-testid="shift-verify"]`)
//   - web/src/screens/staff/holds/HoldsCard.tsx:153-208 (`[data-card="holds"]`, `[data-testid="holds-error"]`, `[data-testid="holds-form"]`, `[data-testid="holds-key"]`, `[data-testid="holds-find"]`, `[data-customer-id]`, `[data-testid="holds-reason"]`, `[data-testid="holds-place"]`, `[data-hold-id]`)
//   - web/src/screens/staff/transfers/TransfersCard.tsx:361-493 (`[data-card="transfers"]`, `[data-testid="transfer-new"]`, `[data-testid="transfer-error"]`, `[data-testid="transfer-form"]`, `[data-testid="transfer-kind"]`, `[data-testid="transfer-note"]`, `[data-denomination-row]`, `input[data-denomination-id]`, `[data-testid="transfer-save"]`)
//   - web/src/screens/staff/floor/FloorCard.tsx:242-248,293-326 (`.xc-floor-card`, `[data-testid="floor-error"]`, `[data-testid="floor-target-form"]`, `input[data-currency="USD"]`, `[data-testid="floor-target-save"]`)
//   - web/src/screens/desk/queue/counter/CounterBar.tsx:68-74,114-139,151-157 (`button[name="issueWalkIn"]`, `button[name="openQueue"]`, `button[name="closeQueue"]`, `.desk-error[role="alert"]`)
//   - web/src/screens/desk/bookings/DeskBookingsScreen.tsx:368-380,522-535,601-615,618-860,863-1023 (`[data-screen="desk-bookings"]`, `input[name="date"]`, `[data-action="new-booking"]`, `.xc-modal-overlay`, `select[name="currency"]`, `input[name="quantity"]`, `[data-action="submit-booking"]`, `[role="alert"].field-error`, `[data-booking-id]`, `[data-action="cancel"]`, `input[name="reason"]`, `[data-action="confirm-cancel"]`, `[data-action="reschedule"]`, `select[name="targetSlotId"]`, `[data-action="confirm-reschedule"]`, `[role="alert"].xc-error-banner`)

const TINY_PNG_DATA_URL =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';

const EXCHANGE_AMOUNT = '1';

interface TransactionAnswer {
  id?: string;
  status?: string;
}

interface ReversalRequestAnswer {
  id?: string;
  approvalRequestId?: string;
}

interface BookingSlotsViewAnswer {
  data?: {
    slots?: Array<{
      id: string;
      status: string;
      remaining: number;
    }>;
  };
}

interface BookingCreateAnswer {
  id?: string;
  bookingNo?: string;
  status?: string;
}

/** Switches `page` to `lang` via `withLang` and reloads `path`, asserting `<html lang>` matches. */
async function openInLang(page: Page, lang: Lang, path: string): Promise<void> {
  await withLang(page, lang);
  await page.goto(path);
  await expect(page.locator('html'), `${path} renders in ${lang}`).toHaveAttribute('lang', HTML_LANG[lang]);
}

/** Asserts an error message is human-worded: non-empty, not a raw camelCase error code, not a generic 500 fallback. */
function expectWordedRefusal(msg: string, label: string): void {
  const trimmed = msg.trim();
  expect(trimmed.length, `${label} refusal is non-empty`).toBeGreaterThan(2);
  expect(trimmed, `${label} refusal is not raw validationFailed code`).not.toBe('validationFailed');
  expect(trimmed, `${label} refusal is not generic 500 fallback`).not.toMatch(/^(เซิร์ฟเวอร์ขัดข้อง|Server error)$/);
}

/** Returns tomorrow's YYYY-MM-DD in Asia/Bangkok so booking cancel/reschedule stays outside the cutoff window. */
function tomorrowBangkokIsoDate(): string {
  const bkkToday = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Bangkok',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(new Date());
  const [y, m, d] = bkkToday.split('-').map((n) => Number(n));
  const next = new Date(Date.UTC(y, m - 1, d + 1));
  return next.toISOString().slice(0, 10);
}

test.describe.serial('Flow Validation — staff and desk forms (#4296, §9.1–§9.5)', () => {
  test('shift count, hold, transfer, and floor day-target forms refuse invalid input in th and en, keep input, and create nothing', async ({
    browser,
  }) => {
    test.setTimeout(240_000);
    const requiredRoles: readonly Role[] = ['teller', 'branchManager', 'finance'];
    for (const role of requiredRoles) {
      if (!hasAccount(role)) {
        test.skip(true, `no ${role} account in users.json: run scripts/reseed.sh`);
      }
    }

    const runMarker = `qa-r9-12-${Date.now()}`;
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const finance = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    try {
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await signIn(finance, 'finance', '/staff');
      await ensureShiftClosed(teller, manager);

      try {
        // ── 1. Shift count form (`/staff` — `countSubmit`, `countVerify`) ──
        // Citations: ShiftCard.tsx:221-225,382-485,558-585; commands/cash.rs:125-135,181-192,250-342.
        await clockIn(teller);

        for (const lang of STAFF_LANGS) {
          await openInLang(teller, lang, '/staff');
          const shiftCard = teller.locator('.xc-shift-card');
          await expect(shiftCard, 'ShiftCard visible (ShiftCard.tsx:382)').toBeVisible({ timeout: 20_000 });

          const openCountBtn = teller.locator('[data-testid="shift-open-count"]');
          await expect(openCountBtn, 'shift-open-count visible (ShiftCard.tsx:419)').toBeVisible({ timeout: 15_000 });
          await openCountBtn.click();

          const countForm = teller.locator('[data-testid="shift-count-form"]');
          await expect(countForm, 'shift-count-form open (ShiftCard.tsx:464)').toBeVisible({ timeout: 15_000 });

          const denomInputs = countForm.locator('[data-denomination-row] input[data-denomination-id]');
          const inputCount = await denomInputs.count();
          expect(inputCount, 'at least one denomination input on shift-count-form').toBeGreaterThan(0);

          // Case 1: all denomination inputs empty ('') -> client-side refusal (`noCountLines`), nothing submitted.
          for (let i = 0; i < inputCount; i++) {
            await denomInputs.nth(i).fill('');
          }
          await teller.locator('[data-testid="shift-count-save"]').click();
          const shiftErr = teller.locator('[data-testid="shift-error"]');
          await expect(shiftErr, `shift-error visible on empty count (${lang}, ShiftCard.tsx:386)`).toBeVisible({
            timeout: 10_000,
          });
          expectWordedRefusal(await shiftErr.innerText(), `shift count empty lines (${lang})`);
          await expect(denomInputs.first(), 'first denomination input stays empty').toHaveValue('');
          await expect(teller.locator('[data-testid="shift-verify"]')).toHaveCount(0);

          // Cases 2–4: negative ('-1'), not a number ('abc'), decimal ('1.5') -> `countSubmit` 400 validationFailed.
          const firstInput = denomInputs.first();
          for (const badQty of ['-1', 'abc', '1.5']) {
            await firstInput.fill(badQty);
            const countRes = await posted(teller, /^\/api\/cash-counts$/, () =>
              teller.locator('[data-testid="shift-count-save"]').click(),
            );
            expect(countRes.ok, `POST /api/cash-counts (${badQty}, ${lang}) must be refused`).toBe(false);
            expect(countRes.text, `POST /api/cash-counts (${badQty}, ${lang}) status`).toMatch(/^400\b/);

            await expect(shiftErr, `shift-error visible for ${badQty} (${lang})`).toBeVisible({ timeout: 10_000 });
            expectWordedRefusal(await shiftErr.innerText(), `shift count ${badQty} (${lang})`);
            await expect(firstInput, `shift count input keeps '${badQty}' (${lang})`).toHaveValue(badQty);
            await expect(
              teller.locator('[data-testid="shift-verify"]'),
              'no count created for countVerify after refused countSubmit',
            ).toHaveCount(0);
          }

          await teller.locator('[data-testid="shift-count-cancel"]').click();
        }
        await shot(teller, 'staff', 'validation-shift-count');

        // ── 2. Holds form (`/staff` — `holdCreate`) ──
        // Citations: HoldsCard.tsx:97-123,153-208; holds/i18n.ts:7,16; commands/branch_day.rs:344-375.
        const reserveRes = await manager.request.post('/api/rpc', {
          headers: { 'Idempotency-Key': `${runMarker}-reserve-hold-cust` },
          data: {
            method: 'reserveCustomer',
            params: {
              branch: users.branch,
              docType: 'PASSPORT',
              identityKey: `WALK-IN-${runMarker}`,
              fullName: `Hold Validation ${runMarker}`,
              nationality: 'TH',
              country: 'TH',
              imageBase64: TINY_PNG_DATA_URL,
            },
          },
        });
        expect(reserveRes.ok(), `reserveCustomer for hold validation: ${reserveRes.status()}`).toBeTruthy();
        const reservedJson = (await reserveRes.json()) as { result?: { id?: string }; id?: string };
        const customerId = String(reservedJson.result?.id ?? reservedJson.id ?? '');
        expect(customerId, 'reserved customerId').not.toBe('');

        for (const lang of STAFF_LANGS) {
          await openInLang(manager, lang, '/staff');
          const holdsCard = manager.locator('[data-card="holds"]');
          await expect(holdsCard, 'HoldsCard visible (HoldsCard.tsx:153)').toBeVisible({ timeout: 20_000 });

          const holdsErr = holdsCard.locator('[data-testid="holds-error"]');
          const holdsReason = holdsCard.locator('[data-testid="holds-reason"]');
          const holdsPlace = holdsCard.locator('[data-testid="holds-place"]');

          // Case 1: no customer selected, reason filled with runMarker -> client-side refusal (`pickCustomer`).
          await holdsReason.fill(runMarker);
          await holdsPlace.click();
          await expect(holdsErr, `holds-error visible without customer (${lang}, HoldsCard.tsx:157)`).toBeVisible({
            timeout: 10_000,
          });
          expectWordedRefusal(await holdsErr.innerText(), `hold no customer (${lang})`);
          await expect(holdsReason, `holds-reason keeps '${runMarker}' (${lang})`).toHaveValue(runMarker);

          // Search and pick the customer so cases 2–3 test `holdCreate` reason validation.
          await holdsCard.locator('[data-testid="holds-key"]').fill(`WALK-IN-${runMarker}`);
          await holdsCard.locator('[data-testid="holds-find"]').click();
          const custBtn = holdsCard.locator(`[data-customer-id="${customerId}"]`);
          await expect(custBtn, 'customer search result visible (HoldsCard.tsx:180)').toBeVisible({ timeout: 15_000 });
          await custBtn.click();

          // Cases 2–3: empty reason ('') and whitespace-only reason ('   ') -> `POST /api/customer-holds` 400.
          for (const badReason of ['', '   ']) {
            await holdsReason.fill(badReason);
            const holdRes = await posted(manager, /^\/api\/customer-holds$/, () => holdsPlace.click());
            expect(holdRes.ok, `POST /api/customer-holds (${JSON.stringify(badReason)}, ${lang}) refused`).toBe(false);
            expect(holdRes.text, 'POST /api/customer-holds status').toMatch(/^400\b/);

            await expect(holdsErr, `holds-error visible for reason ${JSON.stringify(badReason)}`).toBeVisible({
              timeout: 10_000,
            });
            expectWordedRefusal(await holdsErr.innerText(), `hold reason ${JSON.stringify(badReason)} (${lang})`);
            await expect(holdsReason, 'holds-reason keeps typed input').toHaveValue(badReason);
            await expect(
              holdsCard.locator('[data-hold-id]', { hasText: customerId }),
              'no hold row created for customer',
            ).toHaveCount(0);
          }
        }
        await shot(manager, 'staff', 'validation-holds');

        // ── 3. Cash transfer form (`/staff` — `transferCreate`) ──
        // Citations: TransfersCard.tsx:361-493; commands/cash/transfers.rs:23-54.
        for (const lang of STAFF_LANGS) {
          await openInLang(manager, lang, '/staff');
          const transfersCard = manager.locator('[data-card="transfers"]');
          await expect(transfersCard, 'TransfersCard visible (TransfersCard.tsx:361)').toBeVisible({ timeout: 20_000 });

          await transfersCard.locator('[data-testid="transfer-new"]').click();
          const transferForm = transfersCard.locator('[data-testid="transfer-form"]');
          await expect(transferForm, 'transfer-form open (TransfersCard.tsx:381)').toBeVisible({ timeout: 15_000 });

          await transferForm.locator('[data-testid="transfer-kind"]').selectOption('EXTERNAL_OUT');
          const noteInput = transferForm.locator('[data-testid="transfer-note"]');
          await noteInput.fill(runMarker);

          const denomInputs = transferForm.locator('[data-denomination-row] input[data-denomination-id]');
          const count = await denomInputs.count();
          expect(count, 'at least one denomination input on transfer-form').toBeGreaterThan(0);
          for (let i = 0; i < count; i++) {
            await denomInputs.nth(i).fill('');
          }

          const firstDenomInput = denomInputs.first();
          const transferErr = transfersCard.locator('[data-testid="transfer-error"]');

          // Cases 1–4: '0' (zero), '-5' (negative), 'abc' (not a number), '999999999999' (over the limit).
          for (const badQty of ['0', '-5', 'abc', '999999999999']) {
            await firstDenomInput.fill(badQty);
            const txRes = await posted(manager, /^\/api\/cash-transfers$/, () =>
              transferForm.locator('[data-testid="transfer-save"]').click(),
            );
            expect(txRes.ok, `POST /api/cash-transfers (${badQty}, ${lang}) must be refused`).toBe(false);
            expect(txRes.text, `POST /api/cash-transfers (${badQty}, ${lang}) status`).toMatch(/^400\b/);

            await expect(
              transferErr,
              `transfer-error visible for ${badQty} (${lang}, TransfersCard.tsx:373)`,
            ).toBeVisible({ timeout: 10_000 });
            expectWordedRefusal(await transferErr.innerText(), `transfer ${badQty} (${lang})`);
            await expect(firstDenomInput, `transfer input keeps '${badQty}' (${lang})`).toHaveValue(badQty);
            await expect(noteInput, `transfer-note keeps '${runMarker}' (${lang})`).toHaveValue(runMarker);
            await expect(
              transfersCard.locator('[data-transfer-id]', { hasText: runMarker }),
              'no transfer row created with runMarker',
            ).toHaveCount(0);
          }
        }
        await shot(manager, 'staff', 'validation-transfers');

        // ── 4. Floor day target form (`/staff` — `dayTarget`) ──
        // Citations: FloorCard.tsx:242-248,293-326; commands/branch_day.rs:235-280.
        for (const lang of STAFF_LANGS) {
          await openInLang(finance, lang, '/staff');
          const floorCard = finance.locator('.xc-floor-card');
          await expect(floorCard, 'FloorCard visible (FloorCard.tsx:242)').toBeVisible({ timeout: 20_000 });

          const targetForm = floorCard.locator('[data-testid="floor-target-form"]');
          await expect(targetForm, 'floor-target-form visible (FloorCard.tsx:293)').toBeVisible({ timeout: 15_000 });

          const usdInput = targetForm.locator('input[data-currency="USD"]');
          await expect(usdInput, 'USD target input visible (FloorCard.tsx:303)').toBeVisible({ timeout: 10_000 });
          const floorErr = floorCard.locator('[data-testid="floor-error"]');

          // Cases 1–3: '-100' (negative), 'abc' (not a number), '10.12345' (too many decimals).
          for (const badTarget of ['-100', 'abc', '10.12345']) {
            await usdInput.fill(badTarget);
            const targetRes = await posted(finance, /^\/api\/branch-day\/target$/, () =>
              targetForm.locator('[data-testid="floor-target-save"]').click(),
            );
            expect(targetRes.ok, `POST /api/branch-day/target (${badTarget}, ${lang}) refused`).toBe(false);
            expect(targetRes.text, `POST /api/branch-day/target (${badTarget}, ${lang}) status`).toMatch(/^400\b/);

            await expect(floorErr, `floor-error visible for ${badTarget} (${lang}, FloorCard.tsx:246)`).toBeVisible({
              timeout: 10_000,
            });
            expectWordedRefusal(await floorErr.innerText(), `floor target ${badTarget} (${lang})`);
            await expect(usdInput, `USD target input keeps '${badTarget}' (${lang})`).toHaveValue(badTarget);
          }
        }
        await shot(finance, 'staff', 'validation-floor-target');
      } finally {
        await ensureShiftClosed(teller, manager);
      }
    } finally {
      await teller.close();
      await manager.close();
      await finance.close();
    }
  });

  test('desk amount/rate, payment, refunds, walk-in queue, and desk bookings forms refuse invalid input in th and en, keep input, and create nothing', async ({
    browser,
  }) => {
    test.setTimeout(300_000);
    const requiredRoles: readonly Role[] = ['teller', 'branchManager', 'finance', 'customer'];
    for (const role of requiredRoles) {
      if (!hasAccount(role)) {
        test.skip(true, `no ${role} account in users.json: run scripts/reseed.sh`);
      }
    }

    const runMarker = `qa-r9-12-${Date.now()}`;
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const finance = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const customer = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    try {
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await signIn(finance, 'finance', '/staff');
      await signIn(customer, 'customer', '/bookings');

      await openShift(teller, manager);
      let shiftOpened = true;
      let started: 'started' | 'alreadyOpen' | undefined;
      let createdBookingId: string | null = null;

      try {
        started = await startService(manager);
        const currency = users.currency ?? 'USD';

        // ── 1. Desk amount/rate & payment step (`/desk?branch=MAIN` — `quoteCreate`, `transactionCreate`, `transactionConfirm`) ──
        // Citations: DeskPage.tsx:28; steps/rate/step.tsx:54-98,126-137; steps/rate/i18n.ts:5-7,22-24;
        // desk-store.ts:153-162,252-300; steps/payment/step.tsx:98-121,135,168,209,224,230.
        for (const lang of STAFF_LANGS) {
          await openInLang(teller, lang, '/desk?branch=' + users.branch);
          await expect(teller.locator('.desk-page'), 'desk page visible (DeskPage.tsx:28)').toBeVisible({
            timeout: 20_000,
          });

          const typeSelect = teller.locator('select[name="type"]');
          const currencySelect = teller.locator('select[name="currency"]');
          const amountInput = teller.locator('input[name="amount"]');
          const submitBtn = teller.locator('form.desk-form button[type="submit"]');

          await expect(
            teller.locator(`select[name="currency"] option[value="${currency}"]`),
            `rate for ${currency} loaded in desk_config (steps/rate/step.tsx:81)`,
          ).toHaveCount(1, { timeout: 20_000 });
          await typeSelect.selectOption('BUY');
          await currencySelect.selectOption(currency);

          // Case 1: empty amount ('') on submit -> client-side refusal (`needShiftAndRate`), stays on step 0.
          await amountInput.fill('');
          await submitBtn.click();
          const alertErr = teller.locator('[role="alert"].desk-error');
          await expect(
            alertErr,
            `desk alert error visible on empty submit (${lang}, steps/rate/step.tsx:128)`,
          ).toBeVisible({ timeout: 10_000 });
          expectWordedRefusal(await alertErr.innerText(), `desk empty amount (${lang})`);
          await expect(amountInput, 'amount input stays empty').toHaveValue('');
          await expect(teller.locator('button[name="skipCustomer"]')).toHaveCount(0);

          // Cases 2–5: '0' (zero), '-50' (negative), '10.12345' (too many decimals), '999999999999999' (over limit).
          for (const badAmount of ['0', '-50', '10.12345', '999999999999999']) {
            const quoteRes = await posted(teller, /^\/api\/quotes$/, () => amountInput.fill(badAmount));
            expect(quoteRes.ok, `POST /api/quotes (${badAmount}, ${lang}) must be refused`).toBe(false);
            expect(quoteRes.text, `POST /api/quotes (${badAmount}, ${lang}) status`).toMatch(/^400\b/);

            const fieldErr = teller.locator('.desk-error').first();
            await expect(
              fieldErr,
              `desk-error visible for amount '${badAmount}' (${lang}, steps/rate/step.tsx:96)`,
            ).toBeVisible({ timeout: 10_000 });
            expectWordedRefusal(await fieldErr.innerText(), `desk amount ${badAmount} (${lang})`);
            await expect(amountInput, `amount input keeps '${badAmount}' (${lang})`).toHaveValue(badAmount);

            // Submitting with an invalid quote stays on step 0; no customer/payment step or transactionCreate.
            await submitBtn.click();
            await expect(currencySelect, 'still on rate step 0 after invalid submit').toBeVisible();
            await expect(teller.locator('button[name="skipCustomer"]')).toHaveCount(0);
          }
        }
        await shot(teller, 'desk', 'validation-desk-amount');

        // Advance with a valid quote to test payment-step refusals in `th` and `en` before `transactionCreate`/`transactionConfirm`,
        // then complete the exchange so we can test `reversalRequest` and `reversalPost` refusals.
        for (const lang of STAFF_LANGS) {
          await openInLang(teller, lang, '/desk?branch=' + users.branch);
          await expect(
            teller.locator(`select[name="currency"] option[value="${currency}"]`),
            'currency option loaded',
          ).toHaveCount(1, { timeout: 20_000 });

          const validQuoteRes = await posted(teller, /^\/api\/quotes$/, async () => {
            await teller.locator('select[name="type"]').selectOption('BUY');
            await teller.locator('select[name="currency"]').selectOption(currency);
            await teller.locator('input[name="amount"]').fill(EXCHANGE_AMOUNT);
          });
          expect(validQuoteRes.ok, `valid POST /api/quotes: ${validQuoteRes.text}`).toBeTruthy();
          await expect(teller.locator('.desk-rate-line strong')).toBeVisible({ timeout: 15_000 });
          await teller.locator('form.desk-form button[type="submit"]').click();

          const skipBtn = teller.locator('button[name="skipCustomer"]');
          await expect(skipBtn, 'skipCustomer visible (steps/customer/step.tsx:169)').toBeVisible({ timeout: 15_000 });
          const reserveRes = await posted(teller, /^\/api\/rpc$/, () => skipBtn.click());
          expect(reserveRes.ok, `reserveCustomer on skip: ${reserveRes.text}`).toBeTruthy();

          const confirmBookingBtn = teller.getByRole('button', { name: /ยืนยันการจอง|Confirm booking/ });
          await expect(confirmBookingBtn, 'booking step visible (steps/booking/step.tsx:54)').toBeVisible({
            timeout: 15_000,
          });
          await confirmBookingBtn.click();

          const methodSelect = teller.locator('select[name="method"]');
          await expect(methodSelect, 'payment step visible (steps/payment/step.tsx:135)').toBeVisible({
            timeout: 15_000,
          });
          await methodSelect.selectOption('Cash');
          const accountField = teller.locator('input[name="account"]');
          const savedAccount = await accountField.inputValue();

          await teller.locator('input[name="paymentChecked"]').check();
          await accountField.fill('');
          await teller.locator('form.desk-form button[type="submit"]').click();
          const payErr = teller.locator('[role="alert"].desk-error');
          await expect(payErr, `payment incomplete error visible (${lang}, steps/payment/step.tsx:224)`).toBeVisible({
            timeout: 10_000,
          });
          expectWordedRefusal(await payErr.innerText(), `payment empty account (${lang})`);
          await expect(accountField, 'account input stays empty').toHaveValue('');

          await accountField.fill(savedAccount || 'Cash');
          teller.once('dialog', (d) => d.dismiss());
          await teller.locator('form.desk-form button[type="submit"]').click();
          await expect(payErr, `payment countNotConfirmed error visible (${lang})`).toBeVisible({ timeout: 10_000 });
          expectWordedRefusal(await payErr.innerText(), `payment count not confirmed (${lang})`);
          await expect(accountField, 'account input preserved after dismissed confirm').toHaveValue(
            savedAccount || 'Cash',
          );
        }

        // Complete the exchange on the last open payment form so `/desk/transactions/{id}` mounts `request-reversal`.
        teller.once('dialog', (d) => d.accept());
        const createRes = await posted(teller, /^\/api\/transactions$/, () =>
          teller.locator('form.desk-form button[type="submit"]').click(),
        );
        expect(createRes.ok, `POST /api/transactions: ${createRes.text}`).toBeTruthy();
        const txId = String((JSON.parse(createRes.body) as TransactionAnswer).id ?? '');
        expect(txId, 'created transaction id').not.toBe('');

        const confirmRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/confirm$`),
          async () => {},
        );
        expect(confirmRes.ok, `POST /api/transactions/${txId}/confirm`).toBeTruthy();

        const completeRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/complete$`),
          async () => {},
        );
        expect(completeRes.ok, `POST /api/transactions/${txId}/complete`).toBeTruthy();

        // ── 2. Refunds: `reversalRequest` (`/desk/transactions/{id}`) and `reversalPost` (`/desk?branch=MAIN`) ──
        // Citations: TransactionScreen.tsx:147-162,181,215,225-230; ReasonDialog.tsx:32-50;
        // RefundsStep.tsx:14,54-55,76,87-96,128-136; refunds/i18n.ts:20,47.
        for (const lang of STAFF_LANGS) {
          await openInLang(teller, lang, `/desk/transactions/${txId}`);
          const requestBtn = teller.locator('button[data-action="request-reversal"]');
          await expect(requestBtn, 'request-reversal button visible (TransactionScreen.tsx:215)').toBeVisible({
            timeout: 20_000,
          });

          // Cases 1–3 on `reversalRequest`: empty (''), whitespace ('   '), and >500 chars ('x'.repeat(505)).
          for (const badReason of ['', '   ', 'x'.repeat(505)]) {
            await requestBtn.click();
            const reasonDialog = teller.locator('dialog.xc-dialog');
            await expect(reasonDialog, 'ReasonDialog open (ReasonDialog.tsx:32)').toBeVisible({ timeout: 10_000 });
            const reasonInput = reasonDialog.locator('input[type="text"]');
            await reasonInput.fill(badReason);
            await expect(reasonInput, 'ReasonDialog input holds typed value before confirm').toHaveValue(badReason);

            const revRes = await posted(
              teller,
              new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/reversal-requests$`),
              () => reasonDialog.locator('.xc-dialog-actions button').last().click(),
            );
            expect(revRes.ok, `reversalRequest (${badReason.length} chars, ${lang}) refused`).toBe(false);
            expect(revRes.text, 'reversalRequest status').toMatch(/^400\b/);

            const dtxMsg = teller.locator('.xc-dtx-message[role="status"]');
            await expect(dtxMsg, `dtx refusal message visible (${lang}, TransactionScreen.tsx:181)`).toBeVisible({
              timeout: 10_000,
            });
            expectWordedRefusal(await dtxMsg.innerText(), `reversalRequest (${lang})`);
            await expect(
              teller.locator('button[data-action="request-reversal"]'),
              'request-reversal button still available because no reversal was created',
            ).toBeVisible();
          }
        }
        await shot(teller, 'desk', 'validation-reversal-request');

        // Submit a valid reversal request (`runMarker`) and have finance approve it so we can test `reversalPost`.
        const requestBtn = teller.locator('button[data-action="request-reversal"]');
        await requestBtn.click();
        const reasonDialog = teller.locator('dialog.xc-dialog');
        await reasonDialog.locator('input[type="text"]').fill(runMarker);
        const validRevRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/reversal-requests$`),
          () => reasonDialog.locator('.xc-dialog-actions button').last().click(),
        );
        expect(validRevRes.ok, `valid reversalRequest: ${validRevRes.text}`).toBeTruthy();
        const revBody = JSON.parse(validRevRes.body) as ReversalRequestAnswer;
        const reversalId = String(revBody.id ?? '');
        const approvalRequestId = String(revBody.approvalRequestId ?? '');
        expect(reversalId).not.toBe('');
        expect(approvalRequestId).not.toBe('');

        await finance.goto('/#reconciliation');
        const requestRow = finance.locator(`[data-request="${approvalRequestId}"]`);
        await expect(requestRow, 'pending reversal on /#reconciliation').toBeVisible({ timeout: 20_000 });
        await requestRow.locator('input[name="reason"]').fill(`${runMarker}-approve`);
        const decideRes = await posted(
          finance,
          new RegExp(`^/api/approvals/${encodeURIComponent(approvalRequestId)}/decide$`),
          () => requestRow.locator('button[data-action="approve"]').click(),
        );
        expect(decideRes.ok, `approve reversal: ${decideRes.text}`).toBeTruthy();

        // On `/desk?branch=MAIN` as `branchManager` (who has no open teller shift), test `reversalPost`:
        //   1) cancelling `ConfirmDialog` sends nothing and keeps `[data-reversal="${reversalId}"]`;
        //   2) confirming `ConfirmDialog` without an open shift is refused with `noOpenShift` in `th` and `en`.
        for (const lang of STAFF_LANGS) {
          await openInLang(manager, lang, '/desk?branch=' + users.branch);
          const mgrRefundsBtn = manager.locator('button[data-desk-task="refunds"]');
          await expect(mgrRefundsBtn, 'refunds task button (steps/refunds/task.tsx:53)').toBeVisible({
            timeout: 20_000,
          });
          await mgrRefundsBtn.click();

          const revRow = manager.locator(`[data-reversal="${reversalId}"]`);
          await expect(revRow, `reversal row visible (${lang}, RefundsStep.tsx:76)`).toBeVisible({ timeout: 15_000 });
          const postBtn = manager.locator(`button[data-action="post"][data-id="${reversalId}"]`);

          // Cancel ConfirmDialog -> nothing posted, row remains.
          await postBtn.click();
          const confirmDialog = manager.locator('dialog.xc-dialog');
          await expect(confirmDialog, 'ConfirmDialog visible (RefundsStep.tsx:128)').toBeVisible({ timeout: 10_000 });
          await confirmDialog.locator('.xc-dialog-actions button').first().click();
          await expect(revRow, 'reversal row still present after cancel').toBeVisible();

          // Confirm without an open shift -> `POST /api/reversals/{id}/post` returns 409 `noOpenShift`.
          await postBtn.click();
          await expect(confirmDialog).toBeVisible({ timeout: 10_000 });
          const postRes = await posted(
            manager,
            new RegExp(`^/api/reversals/${encodeURIComponent(reversalId)}/post$`),
            () => confirmDialog.locator('.xc-dialog-actions button').last().click(),
          );
          expect(postRes.ok, `reversalPost without open shift (${lang}) refused`).toBe(false);
          expect(postRes.text, 'reversalPost without open shift status').toMatch(/^409\b/);

          const refundErr = manager.locator('.desk-refunds-error-msg');
          await expect(refundErr, `desk-refunds-error-msg visible (${lang}, RefundsStep.tsx:55)`).toBeVisible({
            timeout: 10_000,
          });
          expectWordedRefusal(await refundErr.innerText(), `reversalPost noOpenShift (${lang})`);
          await expect(revRow, 'reversal row kept after noOpenShift refusal').toBeVisible();
        }
        await shot(manager, 'desk', 'validation-reversal-post');

        // `teller` (whose shift is open) posts the reversal so the shift drawer returns to its opening balance.
        await teller.goto('/desk?branch=' + users.branch);
        await teller.locator('button[data-desk-task="refunds"]').click();
        await teller.locator(`button[data-action="post"][data-id="${reversalId}"]`).click();
        const tellerConfirm = teller.locator('dialog.xc-dialog');
        await expect(tellerConfirm).toBeVisible({ timeout: 10_000 });
        const tellerPostRes = await posted(
          teller,
          new RegExp(`^/api/reversals/${encodeURIComponent(reversalId)}/post$`),
          () => tellerConfirm.locator('.xc-dialog-actions button').last().click(),
        );
        expect(tellerPostRes.ok, `teller reversalPost: ${tellerPostRes.text}`).toBeTruthy();

        // ── 3. Walk-in ticket (`/desk/queue` — `ticketIssueWalkIn`) ──
        // Citations: CounterBar.tsx:68-74,114-139,151-157; desk/queue/counter/i18n.ts:7,22.
        await manager.goto('/desk/queue?branch=' + users.branch);
        const closeQueueBtn = manager.locator('button[name="closeQueue"]');
        await expect(closeQueueBtn, 'closeQueue button visible (CounterBar.tsx:128)').toBeVisible({ timeout: 20_000 });
        try {
          const closeQRes = await posted(manager, /^\/api\/queue\/close$/, () => closeQueueBtn.click());
          expect(closeQRes.ok, `POST /api/queue/close: ${closeQRes.text}`).toBeTruthy();

          for (const lang of STAFF_LANGS) {
            await openInLang(manager, lang, '/desk/queue?branch=' + users.branch);
            const issueBtn = manager.locator('button[name="issueWalkIn"]');
            await expect(issueBtn, 'issueWalkIn button visible (CounterBar.tsx:69)').toBeVisible({ timeout: 15_000 });
            const walkInRes = await posted(manager, /^\/api\/queue\/tickets$/, () => issueBtn.click());
            expect(walkInRes.ok, `ticketIssueWalkIn when queue closed (${lang}) refused`).toBe(false);

            const queueErr = manager.locator('.desk-error[role="alert"]');
            await expect(queueErr, `queue error alert visible (${lang}, CounterBar.tsx:153)`).toBeVisible({
              timeout: 10_000,
            });
            expectWordedRefusal(await queueErr.innerText(), `ticketIssueWalkIn queueClosed (${lang})`);
          }
          await shot(manager, 'desk', 'validation-queue-walkin');
        } finally {
          const openQueueBtn = manager.locator('button[name="openQueue"]');
          if (await openQueueBtn.isVisible().catch(() => false)) {
            await posted(manager, /^\/api\/queue\/open$/, () => openQueueBtn.click()).catch(() => {});
          }
        }

        // ── 4. Desk bookings (`/desk/bookings` — `bookingCancel`, `bookingReschedule`, new booking modal) ──
        // Citations: DeskBookingsScreen.tsx:368-380,522-535,601-615,618-860,863-1023;
        // desk/bookings/i18n.ts:15,25,50,60; commands/bookings/cancel.rs:104-115.
        const tomorrow = tomorrowBangkokIsoDate();
        const slotsRes = await customer.request.get(
          `/api/v3/views/booking_slots?branch=${encodeURIComponent(users.branch)}&filter=${encodeURIComponent(JSON.stringify({ date: tomorrow }))}`,
        );
        expect(slotsRes.ok(), `GET booking_slots for ${tomorrow}`).toBeTruthy();
        const slotsBody = (await slotsRes.json()) as BookingSlotsViewAnswer;
        const openSlot = (slotsBody.data?.slots ?? []).find((s) => s.status === 'OPEN' && s.remaining > 0);
        expect(openSlot, `open slot on ${tomorrow}`).toBeTruthy();

        const createBookingRes = await customer.request.post('/api/bookings', {
          headers: { 'Idempotency-Key': `${runMarker}-booking` },
          data: {
            branchId: users.branch,
            slotId: openSlot!.id,
            currency: 'USD',
            side: 'SELL',
            quantity: '100',
            phone: '0812345678',
          },
        });
        expect(createBookingRes.ok(), `POST /api/bookings for ${tomorrow}`).toBeTruthy();
        const createdBooking = (await createBookingRes.json()) as BookingCreateAnswer;
        createdBookingId = String(createdBooking.id ?? '');
        expect(createdBookingId, 'createdBookingId').not.toBe('');

        for (const lang of STAFF_LANGS) {
          await openInLang(manager, lang, '/desk/bookings');
          const deskBookings = manager.locator('[data-screen="desk-bookings"]');
          await expect(deskBookings, 'desk-bookings screen visible (DeskBookingsScreen.tsx:368)').toBeVisible({
            timeout: 20_000,
          });

          const dateInput = manager.locator('input[name="date"]');
          await dateInput.fill(tomorrow);
          await dateInput.dispatchEvent('input');

          // Part A: New Booking modal (`[data-action="new-booking"]`) with invalid quantities ('0', '-10', '1.5').
          await manager.locator('[data-action="new-booking"]').click();
          const modal = manager.locator('.xc-modal-overlay');
          await expect(modal, 'new-booking modal visible (DeskBookingsScreen.tsx:690)').toBeVisible({
            timeout: 10_000,
          });
          await modal.locator('select[name="currency"]').selectOption('USD');
          const qtyInput = modal.locator('input[name="quantity"]');

          for (const badQty of ['0', '-10', '1.5']) {
            await qtyInput.fill(badQty);
            await modal.locator('[data-action="submit-booking"]').click();
            const fieldErr = modal.locator('[role="alert"].field-error');
            await expect(
              fieldErr,
              `new-booking quantity field-error visible for '${badQty}' (${lang}, DeskBookingsScreen.tsx:761)`,
            ).toBeVisible({ timeout: 10_000 });
            expectWordedRefusal(await fieldErr.innerText(), `desk new-booking quantity ${badQty} (${lang})`);
            await expect(qtyInput, `quantity input keeps '${badQty}' (${lang})`).toHaveValue(badQty);
          }
          await modal.locator('[data-action="close-modal"]').click();

          // Part B: Cancel booking dialog (`[data-action="cancel"]`, `bookingCancel`) with empty ('') and whitespace ('   ') reasons.
          const bookingRow = manager.locator(`[data-booking-id="${createdBookingId}"]`);
          await expect(
            bookingRow,
            `booking row ${createdBookingId} visible on ${tomorrow} (DeskBookingsScreen.tsx:522)`,
          ).toBeVisible({ timeout: 15_000 });

          await bookingRow.locator('[data-action="cancel"]').click();
          await expect(modal, 'cancel booking modal visible (DeskBookingsScreen.tsx:988)').toBeVisible({
            timeout: 10_000,
          });
          const cancelReasonInput = modal.locator('input[name="reason"]');
          const confirmCancelBtn = modal.locator('[data-action="confirm-cancel"]');

          for (const badReason of ['', '   ']) {
            await cancelReasonInput.fill(badReason);
            await confirmCancelBtn.click();
            const bannerErr = modal.locator('[role="alert"].xc-error-banner');
            await expect(
              bannerErr,
              `cancel reason error banner visible for ${JSON.stringify(badReason)} (${lang}, DeskBookingsScreen.tsx:995)`,
            ).toBeVisible({ timeout: 10_000 });
            expectWordedRefusal(
              await bannerErr.innerText(),
              `bookingCancel reason ${JSON.stringify(badReason)} (${lang})`,
            );
            await expect(cancelReasonInput, 'cancel reason input keeps value').toHaveValue(badReason);
          }
          await modal.locator('[data-action="close-cancel"]').click();

          // Also verify server-side `bookingCancel` (`POST /api/bookings/{id}/cancel`) refuses whitespace reason.
          const apiCancelRes = await manager.request.post(
            `/api/bookings/${encodeURIComponent(createdBookingId)}/cancel`,
            {
              headers: { 'Idempotency-Key': `${runMarker}-bad-cancel-${lang}` },
              data: { reason: '   ' },
            },
          );
          expect(apiCancelRes.status(), 'bookingCancel whitespace reason HTTP 400').toBe(400);

          // Part C: Reschedule dialog (`[data-action="reschedule"]`, `bookingReschedule`) with no target slot selected ('').
          await bookingRow.locator('[data-action="reschedule"]').click();
          await expect(modal, 'reschedule modal visible (DeskBookingsScreen.tsx:911)').toBeVisible({ timeout: 10_000 });
          const targetSlotSelect = modal.locator('select[name="targetSlotId"]');
          await expect(targetSlotSelect, 'targetSlotId select empty initially').toHaveValue('');
          await expect(
            modal.locator('[data-action="confirm-reschedule"]'),
            'confirm-reschedule disabled when targetSlotId is empty (DeskBookingsScreen.tsx:954)',
          ).toBeDisabled();
          await modal.locator('[data-action="close-reschedule"]').click();
        }
        await shot(manager, 'desk', 'validation-desk-bookings');
      } finally {
        if (createdBookingId) {
          await manager.request
            .post(`/api/bookings/${encodeURIComponent(createdBookingId)}/cancel`, {
              headers: { 'Idempotency-Key': `${runMarker}-cleanup-cancel` },
              data: { reason: `${runMarker}-cleanup` },
            })
            .catch(() => {});
        }
        if (started === 'started') {
          await cutOff(manager);
        }
        if (shiftOpened) {
          await closeShift(teller, manager);
        }
      }
    } finally {
      await teller.close();
      await manager.close();
      await finance.close();
      await customer.close();
    }
  });
});
