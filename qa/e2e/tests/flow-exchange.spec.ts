// QA-F6 — flow: desk exchange (quote -> customer -> booking -> payment -> receipt) and same-day reversal
// (design docs/design/qa-e2e.md §4 row F6; docs/design/blueprint/quote-transaction.md; reversal V-2,
// api/crates/xc-exchange/src/domain/reversal/window.rs:16-18).
// Written by ticket #3998 (QA-F6); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - desk/steps.ts:7-13                        steps 0 -> 4 (rate, customer, booking, payment, receipt)
// - desk/steps/rate/step.tsx:62-94,104,136    select[name="type"], select[name="currency"], input[name="amount"], .desk-rate-line strong, submit Next (rate/i18n.ts:5,22)
// - desk/steps/customer/step.tsx:159-177,256  button "ข้ามข้อมูลลูกค้า →" / "Skip customer details →" (customer/i18n.ts:18,44)
// - desk/steps/booking/step.tsx:54            button "ยืนยันการจอง →" / "Confirm booking →" (booking/i18n.ts:10,22)
// - desk/steps/payment/step.tsx:110,135,168,229,246 select[name="method"], [name="account"], input[name="paymentChecked"], window.confirm, submit "ยืนยันและบันทึกบัญชี →" / "Confirm and save →" (payment/i18n.ts:17,41)
// - desk/steps/receipt/step.tsx:21,32-34      section.desk-form strong (/^XC-\d{8}-\d{6}$/)
// - desk-transaction/TransactionScreen.tsx:215,225-230 [data-action="request-reversal"], ReasonDialog (staff/parts/ReasonDialog.tsx:35-46)
// - admin/reconciliation/reversals/ReversalsPart.tsx:58-79 [data-request="<requestId>"], input[name="reason"], [data-action="approve"]
// - desk/steps/refunds/task.tsx:53            [data-desk-task="refunds"]
// - desk/steps/refunds/RefundsStep.tsx:87-96,128-136 [data-action="post"][data-id="<reversalId>"], ConfirmDialog (staff/parts/ConfirmDialog.tsx:20-33)
import { expect, test } from '@playwright/test';
import { closeShift, hasAccount, openShift, posted, shot, signIn, users, type Role } from './helpers';

/** Small constant amount so THB paid out stays inside the opening float of SHIFT_QTY notes and below any approval limit (§8 row 2, §9). */
const EXCHANGE_AMOUNT = '1';

interface TransactionAnswer {
  id?: string;
  status?: string;
  documentRef?: string | null;
}

interface ReversalRequestAnswer {
  id?: string;
  approvalRequestId?: string;
}

test.describe.serial('F6 desk exchange', () => {
  test('teller completes a cash BUY exchange and reverses it the same business day', async ({ browser }) => {
    const requiredRoles: readonly Role[] = ['teller', 'branchManager', 'finance'];
    for (const role of requiredRoles) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[F6] skip: ${reason}`);
        test.skip(true, reason);
      }
    }

    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const finance = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    try {
      // Step 1: teller + manager open the teller's shift on /staff (helpers.ts openShift asserts opening count + verify).
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await openShift(teller, manager);

      try {
        // Step 2: teller opens /desk?branch=<branch>, selects BUY + currency + amount '1', waits for POST /api/quotes, submits Next.
        const currency = users.currency ?? 'USD';
        await teller.goto('/desk?branch=' + users.branch);
        await expect(
          teller.locator(`select[name="currency"] option[value="${currency}"]`),
          `rate for ${currency} loaded in desk_config (rate/step.tsx:81)`,
        ).toHaveCount(1, { timeout: 20_000 });

        const quoteRes = await posted(teller, /^\/api\/quotes$/, async () => {
          await teller.locator('select[name="type"]').selectOption('BUY');
          await teller.locator('select[name="currency"]').selectOption(currency);
          await teller.locator('input[name="amount"]').fill(EXCHANGE_AMOUNT);
        });
        console.log(`[F6] POST /api/quotes ${quoteRes.text}`);
        expect(quoteRes.ok, `POST /api/quotes: ${quoteRes.text}`).toBeTruthy();

        await expect(
          teller.locator('.desk-rate-line strong'),
          `quoted rate displayed after POST /api/quotes (${quoteRes.text})`,
        ).toBeVisible({ timeout: 15_000 });
        await shot(teller, 'desk', 'exchange-1-rate');

        await teller.getByRole('button', { name: /ถัดไป · พาสปอร์ต \/ ลูกค้า|Next · Passport \/ customer/ }).click();

        const skipBtn = teller.getByRole('button', { name: /ข้ามข้อมูลลูกค้า|Skip customer details/ });
        await expect(skipBtn, 'customer step shows (steps/customer/step.tsx:256)').toBeVisible({ timeout: 15_000 });

        // Step 3: customer step — "Skip customer details" auto-fills WALK-IN + placeholder image and reserves the customer.
        const reserveRes = await posted(teller, /^\/api\/rpc$/, () => skipBtn.click());
        console.log(`[F6] POST /api/rpc (reserveCustomer) ${reserveRes.text}`);
        expect(reserveRes.ok, `reserveCustomer on skip: ${reserveRes.text}`).toBeTruthy();

        const confirmBookingBtn = teller.getByRole('button', { name: /ยืนยันการจอง|Confirm booking/ });
        await expect(confirmBookingBtn, 'booking step shows (steps/booking/step.tsx:54)').toBeVisible({
          timeout: 15_000,
        });
        await shot(teller, 'desk', 'exchange-2-booking');

        // Step 4: booking step — "Confirm booking" advances to the payment step.
        await confirmBookingBtn.click();

        const methodSelect = teller.locator('select[name="method"]');
        await expect(methodSelect, 'payment step shows (steps/payment/step.tsx:135)').toBeVisible({ timeout: 15_000 });

        // Step 5: payment step — method Cash, account = first offered option, tick paymentChecked, accept window.confirm, submit.
        await methodSelect.selectOption('Cash');
        const accountField = teller.locator('[name="account"]');
        await expect(accountField, 'payment account control (steps/payment/step.tsx:168)').toBeVisible({
          timeout: 15_000,
        });
        const accountOptions = accountField.locator('option');
        if ((await accountOptions.count()) > 0) {
          const firstOption = (await accountOptions.first().getAttribute('value')) ?? '';
          await accountField.selectOption(firstOption);
        } else if ((await accountField.inputValue()).trim() === '') {
          await accountField.fill('Cash');
        }

        await teller.locator('input[name="paymentChecked"]').check();
        await shot(teller, 'desk', 'exchange-3-payment');

        teller.once('dialog', (d) => d.accept());
        const confirmPromise = posted(teller, /^\/api\/transactions\/[^/]+\/confirm$/, async () => {});
        const completePromise = posted(teller, /^\/api\/transactions\/[^/]+\/complete$/, async () => {});
        confirmPromise.catch(() => {});
        completePromise.catch(() => {});

        const createRes = await posted(teller, /^\/api\/transactions$/, () =>
          teller.getByRole('button', { name: /ยืนยันและบันทึกบัญชี|Confirm and save/ }).click(),
        );
        console.log(`[F6] POST /api/transactions ${createRes.text}`);
        expect(createRes.ok, `POST /api/transactions: ${createRes.text}`).toBeTruthy();
        const createdTx = JSON.parse(createRes.body) as TransactionAnswer;
        const txId = String(createdTx.id ?? '');
        expect(txId, `transaction id from POST /api/transactions: ${createRes.text}`).not.toBe('');

        const confirmRes = await confirmPromise;
        console.log(`[F6] POST /api/transactions/${txId}/confirm ${confirmRes.text}`);
        expect(confirmRes.ok, `POST /api/transactions/${txId}/confirm: ${confirmRes.text}`).toBeTruthy();
        const confirmedTx = JSON.parse(confirmRes.body) as TransactionAnswer;
        expect(
          confirmedTx.status,
          `POST /api/transactions/${txId}/confirm status: ${confirmRes.text}`,
        ).toBe('CONFIRMED');

        const completeRes = await completePromise;
        console.log(`[F6] POST /api/transactions/${txId}/complete ${completeRes.text}`);
        expect(completeRes.ok, `POST /api/transactions/${txId}/complete: ${completeRes.text}`).toBeTruthy();
        const completedTx = JSON.parse(completeRes.body) as TransactionAnswer;
        expect(
          completedTx.status,
          `POST /api/transactions/${txId}/complete status: ${completeRes.text}`,
        ).toBe('COMPLETED');

        // Step 6: receipt step — <strong> inside section.desk-form matches /^XC-\d{8}-\d{6}$/ (steps/receipt/step.tsx:32-34).
        const receiptRef = teller.locator('section.desk-form strong').first();
        await expect(
          receiptRef,
          `receipt documentRef after complete (${completeRes.text})`,
        ).toHaveText(/^XC-\d{8}-\d{6}$/, { timeout: 20_000 });
        await shot(teller, 'desk', 'exchange-4-receipt');

        // Step 7: teller requests a reversal on /desk/transactions/{id} via ReasonDialog (staff/parts/ReasonDialog.tsx:35-46).
        await teller.goto('/desk/transactions/' + txId);
        const requestBtn = teller.locator('button[data-action="request-reversal"]');
        await expect(
          requestBtn,
          'request-reversal button on /desk/transactions/{id} (TransactionScreen.tsx:215)',
        ).toBeVisible({ timeout: 20_000 });
        await requestBtn.click();

        const reasonDialog = teller.locator('dialog.xc-dialog');
        await expect(reasonDialog, 'ReasonDialog open (ReasonDialog.tsx:32)').toBeVisible({ timeout: 15_000 });
        await reasonDialog.locator('input[type="text"]').fill('e2e: reverse test desk exchange');

        const revReqRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/reversal-requests$`),
          () => reasonDialog.locator('.xc-dialog-actions button').last().click(),
        );
        console.log(`[F6] POST /api/transactions/${txId}/reversal-requests ${revReqRes.text}`);
        expect(revReqRes.ok, `POST /api/transactions/${txId}/reversal-requests: ${revReqRes.text}`).toBeTruthy();
        const revReqBody = JSON.parse(revReqRes.body) as ReversalRequestAnswer;
        const reversalId = String(revReqBody.id ?? '');
        const approvalRequestId = String(revReqBody.approvalRequestId ?? '');
        expect(reversalId, `reversalId from reversal-requests: ${revReqRes.text}`).not.toBe('');
        expect(approvalRequestId, `approvalRequestId from reversal-requests: ${revReqRes.text}`).not.toBe('');
        await shot(teller, 'desk', 'exchange-5-reversal-requested');

        // Step 8: finance signs in and approves the reversal request on admin /#reconciliation
        // (admin/reconciliation/reversals/ReversalsPart.tsx:58-79).
        await signIn(finance, 'finance', '/');
        await finance.goto('/#reconciliation');
        const requestRow = finance.locator(`[data-request="${approvalRequestId}"]`);
        await expect(
          requestRow,
          `pending reversal request ${approvalRequestId} on /#reconciliation (ReversalsPart.tsx:58)`,
        ).toBeVisible({ timeout: 20_000 });
        await requestRow.locator('input[name="reason"]').fill('e2e: finance approves same-day reversal');
        const decideRes = await posted(
          finance,
          new RegExp(`^/api/approvals/${encodeURIComponent(approvalRequestId)}/decide$`),
          () => requestRow.locator('button[data-action="approve"]').click(),
        );
        console.log(`[F6] POST /api/approvals/${approvalRequestId}/decide ${decideRes.text}`);
        expect(decideRes.ok, `POST /api/approvals/${approvalRequestId}/decide: ${decideRes.text}`).toBeTruthy();
        await shot(finance, 'admin', 'exchange-6-reversal-approved');

        // Step 9: teller returns to /desk?branch=…, opens [data-desk-task="refunds"], posts [data-action="post"][data-id=<reversalId>]
        // and confirms in ConfirmDialog (steps/refunds/RefundsStep.tsx:87-96,128-136).
        await teller.goto('/desk?branch=' + users.branch);
        const refundsTaskBtn = teller.locator('button[data-desk-task="refunds"]');
        await expect(refundsTaskBtn, 'refunds nav button on /desk (steps/refunds/task.tsx:53)').toBeVisible({
          timeout: 20_000,
        });
        await refundsTaskBtn.click();

        const postBtn = teller.locator(`button[data-action="post"][data-id="${reversalId}"]`);
        await expect(
          postBtn,
          `post button for reversal ${reversalId} (steps/refunds/RefundsStep.tsx:87-90)`,
        ).toBeVisible({ timeout: 20_000 });
        await postBtn.click();

        const confirmDialog = teller.locator('dialog.xc-dialog');
        await expect(confirmDialog, 'ConfirmDialog for posting reversal (RefundsStep.tsx:128-136)').toBeVisible({
          timeout: 15_000,
        });
        const postRes = await posted(
          teller,
          new RegExp(`^/api/reversals/${encodeURIComponent(reversalId)}/post$`),
          () => confirmDialog.locator('.xc-dialog-actions button').last().click(),
        );
        console.log(`[F6] POST /api/reversals/${reversalId}/post ${postRes.text}`);
        expect(postRes.ok, `POST /api/reversals/${reversalId}/post: ${postRes.text}`).toBeTruthy();
        await shot(teller, 'desk', 'exchange-7-reversal-posted');
      } finally {
        // Step 10: always close the teller's shift so a failed step still leaves the shift closed.
        await teller.goto('/staff');
        await manager.goto('/staff');
        await closeShift(teller, manager);
      }
    } finally {
      await teller.close().catch(() => {});
      await manager.close().catch(() => {});
      await finance.close().catch(() => {});
    }
  });
});
