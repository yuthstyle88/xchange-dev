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
// - desk-transaction/TransactionScreen.tsx:129-145,204-219 [data-action="request-reversal"], ReasonDialog (staff/parts/ReasonDialog.tsx:35-46), GET /api/v3/views/receipt_document
// - desk-transaction/ReceiptsSection.tsx:25-48 section[data-section="receipts"], [data-action="reprint"][data-number]
// - admin/reconciliation/reversals/ReversalsPart.tsx:58-79 [data-request="<requestId>"], input[name="reason"], [data-action="approve"]
// - desk/steps/refunds/task.tsx:53            [data-desk-task="refunds"]
// - desk/steps/refunds/RefundsStep.tsx:87-96,128-136 [data-action="post"][data-id="<reversalId>"], ConfirmDialog (staff/parts/ConfirmDialog.tsx:20-33)
// - staff-report/StaffReport.tsx:75-101,178-190 .staff-report, input[type="date"], .staff-report-sheet, .staff-report-metrics
import { expect, test, type Page } from '@playwright/test';
import { closeShift, cutOff, hasAccount, openShift, posted, shot, signIn, startService, users, type Role } from './helpers';

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

interface ReceiptDocumentAnswer {
  data?: {
    receipt?: {
      number?: string;
      revision?: number;
      issuedAt?: string;
      html?: string;
    } | null;
  };
}

interface StaffDailyReportAnswer {
  data?: {
    day?: string;
    completed?: number;
    buyVolume?: string;
    sellVolume?: string;
    reversedVolume?: string;
  };
}

/** Clicks [data-action="reprint"][data-number] for `revision` on /desk/transactions/{id} and asserts a non-null receipt_document. */
async function expectReprintRevision(
  page: Page,
  revision: 1 | 2,
  expectedNumber: string,
): Promise<void> {
  const row = page
    .locator('section[data-section="receipts"] tbody tr')
    .filter({ has: page.locator('td', { hasText: new RegExp(`^\\s*${revision}\\s*·`) }) });
  const reprintBtn = row.locator('button[data-action="reprint"][data-number]');
  await expect(
    reprintBtn,
    `reprint button for revision ${revision} on /desk/transactions/{id} (ReceiptsSection.tsx:35-43)`,
  ).toBeVisible({ timeout: 20_000 });
  await expect(reprintBtn).toBeEnabled({ timeout: 15_000 });
  const number = (await reprintBtn.getAttribute('data-number')) ?? '';
  expect(number, `revision ${revision} data-number`).toBe(expectedNumber);

  const resPromise = page.waitForResponse(
    (r) =>
      r.request().method() === 'GET' &&
      new URL(r.url()).pathname === '/api/v3/views/receipt_document' &&
      new URL(r.url()).searchParams.get('number') === number,
    { timeout: 15_000 },
  );
  await reprintBtn.click();
  const res = await resPromise;
  const bodyText = await res.text().catch(() => '');
  console.log(`[F6] GET /api/v3/views/receipt_document?number=${number} ${res.status()}`);
  expect(
    res.ok(),
    `GET /api/v3/views/receipt_document?number=${number}: ${res.status()} ${bodyText.slice(0, 300)}`,
  ).toBeTruthy();
  const parsed = JSON.parse(bodyText) as ReceiptDocumentAnswer;
  const receipt = parsed.data?.receipt ?? null;
  expect(receipt, `receipt_document for revision ${revision} (${number}): ${bodyText.slice(0, 300)}`).not.toBeNull();
  expect(receipt?.number, `receipt_document.number for revision ${revision}`).toBe(expectedNumber);
  expect(receipt?.revision, `receipt_document.revision for revision ${revision}`).toBe(revision);
  expect(receipt?.html ?? '', `receipt_document.html for revision ${revision}`).not.toBe('');
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
      // Step 1: teller + manager open the teller's shift on /staff (helpers.ts openShift asserts opening count + verify),
      // then manager ensures branch-day service is OPEN so save is not refused SaveClosed (branch_day.rs:99-101).
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await openShift(teller, manager);
      let started: 'started' | 'alreadyOpen' | undefined;

      try {
        started = await startService(manager);

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
        const createRes = await posted(teller, /^\/api\/transactions$/, () =>
          teller.getByRole('button', { name: /ยืนยันและบันทึกบัญชี|Confirm and save/ }).click(),
        );
        console.log(`[F6] POST /api/transactions ${createRes.text}`);
        expect(createRes.ok, `POST /api/transactions: ${createRes.text}`).toBeTruthy();
        const createdTx = JSON.parse(createRes.body) as TransactionAnswer;
        const txId = String(createdTx.id ?? '');
        expect(txId, `transaction id from POST /api/transactions: ${createRes.text}`).not.toBe('');

        const confirmRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/confirm$`),
          async () => {},
        );
        console.log(`[F6] POST /api/transactions/${txId}/confirm ${confirmRes.text}`);
        expect(confirmRes.ok, `POST /api/transactions/${txId}/confirm: ${confirmRes.text}`).toBeTruthy();
        const confirmedTx = JSON.parse(confirmRes.body) as TransactionAnswer;
        expect(
          confirmedTx.status,
          `POST /api/transactions/${txId}/confirm status: ${confirmRes.text}`,
        ).toBe('CONFIRMED');

        const completeRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/complete$`),
          async () => {},
        );
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
        const receiptDocRef = (await receiptRef.innerText()).trim();
        await shot(teller, 'desk', 'exchange-4-receipt');

        // Step 7: teller opens /desk/transactions/{id}, reprints revision 1 (ReceiptsSection.tsx:35-43,
        // TransactionScreen.tsx:129-145), then requests a reversal via ReasonDialog (staff/parts/ReasonDialog.tsx:35-46).
        await teller.goto('/desk/transactions/' + txId);
        await expectReprintRevision(teller, 1, receiptDocRef);

        const requestBtn = teller.locator('button[data-action="request-reversal"]');
        await expect(
          requestBtn,
          'request-reversal button on /desk/transactions/{id} (TransactionScreen.tsx:215)',
        ).toBeVisible({ timeout: 20_000 });
        await expect(requestBtn).toBeEnabled({ timeout: 15_000 });
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

        // Step 10: after the reversal posts, teller returns to /desk/transactions/{id} and reprints revision 2
        // (reversal_post.rs:363-373 numbers revision 2 `${original_number}-R2`).
        await teller.goto('/desk/transactions/' + txId);
        await expectReprintRevision(teller, 2, `${receiptDocRef}-R2`);

        // Step 11: branch manager opens /staff/report for today (StaffReport.tsx:75-101,178-190;
        // xchange-v3-wave3-staff-report.md:24-35,64-65; staff_daily_report.rs:127-156).
        // Note: StaffDailyReportData carries aggregate day figures (completed, buyVolume, sellVolume, reversedVolume)
        // for the branch's business date (matching the YYYYMMDD date in receiptDocRef), not per-transaction documentRef rows.
        const receiptDay = `${receiptDocRef.slice(3, 7)}-${receiptDocRef.slice(7, 9)}-${receiptDocRef.slice(9, 11)}`;
        const reportResPromise = manager.waitForResponse(
          (r) => r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/staff_daily_report',
          { timeout: 20_000 },
        );
        await manager.goto('/staff/report');
        const reportRes = await reportResPromise;
        const reportBody = await reportRes.text().catch(() => '');
        console.log(`[F6] GET /api/v3/views/staff_daily_report ${reportRes.status()}`);
        expect(
          reportRes.ok(),
          `GET /api/v3/views/staff_daily_report: ${reportRes.status()} ${reportBody.slice(0, 300)}`,
        ).toBeTruthy();
        const reportJson = JSON.parse(reportBody) as StaffDailyReportAnswer;
        const report = reportJson.data;
        expect(report?.day, 'staff_daily_report day matches receipt business date').toBe(receiptDay);
        expect(report?.completed ?? 0, `staff_daily_report completed: ${reportBody.slice(0, 300)}`).toBeGreaterThanOrEqual(1);
        expect(
          Number(report?.buyVolume ?? '0'),
          `staff_daily_report buyVolume: ${reportBody.slice(0, 300)}`,
        ).toBeGreaterThan(0);
        expect(
          Number(report?.reversedVolume ?? '0'),
          `staff_daily_report reversedVolume: ${reportBody.slice(0, 300)}`,
        ).toBeGreaterThan(0);

        await expect(
          manager.locator('.staff-report-sheet'),
          'staff-report-sheet on /staff/report (StaffReport.tsx:75)',
        ).toBeVisible({ timeout: 20_000 });
        await expect(
          manager.locator('.staff-report input[type="date"]'),
          'staff-report date input shows today (StaffReport.tsx:182-190)',
        ).toHaveValue(receiptDay);
        const metricValues = manager.locator('.staff-report-metrics p b');
        await expect(metricValues.nth(0), 'completed count rendered on /staff/report').toHaveText(
          String(report?.completed),
        );
        await expect(metricValues.nth(1), 'buyVolume rendered on /staff/report').not.toHaveText(/^0(?:\.0+)?$/);
        await expect(metricValues.nth(3), 'reversedVolume rendered on /staff/report').not.toHaveText(/^0(?:\.0+)?$/);
        await shot(manager, 'staff', 'exchange-8-staff-report');
      } finally {
        // Step 12: cut off service if this test started it, then close the teller's shift.
        if (started === 'started') await cutOff(manager);
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
