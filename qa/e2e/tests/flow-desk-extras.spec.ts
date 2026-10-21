// QA-R9-9 — flow: desk draft begin, referral attach, counter pause/resume, and phone verification
// (design docs/design/qa-e2e.md §9.1–§9.4; ticket #4293).
// Written by ticket #4293 (QA-R9-9); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/:
// - screens/desk/DeskScreen.tsx:41                     .desk-shell
// - screens/desk/Summary.tsx:61                        aside.desk-summary CustomerLoyalty
// - screens/desk/steps/rate/step.tsx:62-94,104,136     select[name="type"], select[name="currency"], input[name="amount"], .desk-rate-line strong, button[type="submit"]
// - screens/desk/steps/customer/step.tsx:69,101-110    draftBegin POST /api/tx-drafts on /staff/desk (desk-path.ts:43-56)
// - screens/desk/steps/customer/step.tsx:185,211,215,219,250,256,259 input[name="passport"], input[name="name"], input[name="passportNo"], select[name="nationality"], p.desk-error[role="alert"], skip button, button[type="submit"]
// - screens/desk/steps/booking/step.tsx:54             button[type="submit"] ("ยืนยันการจอง →" / "Confirm booking →")
// - screens/desk/steps/payment/step.tsx:135,168,229,246 select[name="method"], input[name="account"], input[name="paymentChecked"], button[type="submit"]
// - screens/desk/steps/receipt/step.tsx:34,40          section.desk-form strong, ReferralCodeField
// - screens/desk/referral/ReferralCodeField.tsx:28,71,88,92,100,112,122,123 .desk-referral-field, input.desk-referral-input, button.desk-referral-btn, p.desk-referral-error[role="alert"], p.desk-referral-success, strong.desk-referral-code
// - screens/desk/queue/counter/CounterBar.tsx:66,75,84,98 .desk-actions, button[name="callNext"], button[name="pause"], button[name="resume"]
// - screens/desk/loyalty/CustomerLoyalty.tsx:13        .desk-loyalty
// - screens/desk/loyalty/phone/PhoneVerify.tsx:71,74,86,104,125,133,140,150,162 input[name="phone"], button[name="sendCode"], input[name="code"], button[name="confirmCode"], p.desk-error[role="alert"]
// - shared/i18n/LangSwitch.tsx:10-18                   .xc-shell-header button[type="button"] ("EN" / "TH")
// - screens/desk-transaction/TransactionScreen.tsx:215 button[data-action="request-reversal"], dialog.xc-dialog (staff/parts/ReasonDialog.tsx:32)
// - screens/admin/reconciliation/reversals/ReversalsPart.tsx:58-79 [data-request="<approvalRequestId>"], input[name="reason"], button[data-action="approve"]
// - screens/desk/steps/refunds/task.tsx:53             button[data-desk-task="refunds"]
// - screens/desk/steps/refunds/RefundsStep.tsx:87-96,128-136 button[data-action="post"][data-id="<reversalId>"], dialog.xc-dialog
// - smoke-rbac-matrix.spec.ts:99-117                   customer has no desk (redirected to /login?next=…)
import { expect, test, type Page } from '@playwright/test';
import {
  closeShift,
  cutOff,
  hasAccount,
  openShift,
  posted,
  shot,
  signIn,
  STAFF_LANGS,
  startService,
  users,
  withLang,
  type Role,
} from './helpers';

/** Small constant amount so THB paid out stays inside the opening float of SHIFT_QTY notes and below any approval limit (§8 row 2, §9). */
const EXCHANGE_AMOUNT = '1';

/** Valid 8-character alphanumeric referral code for attachReferral (api/crates/xc-referral/src/domain/referral_code.rs). */
const VALID_REFERRAL_CODE = 'QAR99REF';

/**
 * customerPhoneStart may send a real SMS (api/src/infrastructure/notify/notify_sms_sender.rs), so only the seeded test
 * customer's own number is used (deploy/server/seed-test-booking.sh CUSTOMER_PHONE, default +66800000102).
 */
const TEST_PHONE = users.customer.phone ?? '+66800000102';

/** Raw error-code camelCase / UPPER_SNAKE tokens that must never surface as user-facing error text (§9.4 (b)). */
const RAW_CODE =
  /^(validationFailed|wrongCode|expired|locked|smsUnavailable|referralAlreadyCaptured|outOfScope|roleNotAllowedHere|noOpenShift|saveClosed|invalidRequest|serverError|notFound|ORIGIN_REJECTED)$/;

/** 1x1 transparent PNG for passport file upload validation tests on step 1. */
const ONE_PIXEL_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==',
  'base64',
);

interface DraftBeginAnswer {
  id?: string;
}

interface TransactionAnswer {
  id?: string;
  status?: string;
  documentRef?: string | null;
}

interface ReversalRequestAnswer {
  id?: string;
  approvalRequestId?: string;
}

interface QueueBoardViewAnswer {
  data?: {
    myCounter?: {
      status?: string;
    } | null;
  };
}

interface ReferralAttributionsViewAnswer {
  data?: {
    rows?: Array<{
      id?: string;
      code?: string;
      status?: string;
      source?: string;
    }>;
  };
}

interface DraftItem {
  id?: string;
  type?: string;
  currency?: string;
  amount?: number;
  status?: string;
  documentRef?: string;
}

interface AdminSnapshotRpcAnswer {
  operations?: {
    drafts?: DraftItem[];
  };
}

/** Reads `operations.drafts` from `POST /api/staff/rpc` (`adminSnapshot`, api/src/presentation/http/rpc.rs:113-116, api/src/application/compat/snapshot.rs:85-87, v2_json.rs:509). */
async function readStaffDrafts(page: Page): Promise<DraftItem[]> {
  const res = await page.request.post('/api/staff/rpc', {
    headers: {
      Origin: users.baseUrl,
      'x-xchange-request': '1',
    },
    data: { action: 'adminSnapshot', payload: {} },
  });
  expect(res.ok(), `POST /api/staff/rpc adminSnapshot status ${res.status()}`).toBeTruthy();
  const body = (await res.json()) as AdminSnapshotRpcAnswer;
  expect(typeof body.operations, 'adminSnapshot answer has operations').toBe('object');
  return body.operations?.drafts ?? [];
}

/** Reverses a completed Cash exchange so the teller's drawer nets back to opening figures before closeShift (flow-exchange.spec.ts:233-309). */
async function reverseCompletedExchange(teller: Page, finance: Page, txId: string): Promise<void> {
  await teller.goto('/desk/transactions/' + txId);
  const requestBtn = teller.locator('button[data-action="request-reversal"]'); // TransactionScreen.tsx:215
  await expect(requestBtn).toBeVisible({ timeout: 20_000 });
  await expect(requestBtn).toBeEnabled({ timeout: 15_000 });
  await requestBtn.click();

  const reasonDialog = teller.locator('dialog.xc-dialog'); // ReasonDialog.tsx:32
  await expect(reasonDialog).toBeVisible({ timeout: 15_000 });
  await reasonDialog.locator('input[type="text"]').fill(`qa-r9-9-reverse-${Date.now()}`);

  const revReqRes = await posted(
    teller,
    new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/reversal-requests$`),
    () => reasonDialog.locator('.xc-dialog-actions button').last().click(),
  );
  expect(revReqRes.ok, `POST /api/transactions/${txId}/reversal-requests: ${revReqRes.text}`).toBeTruthy();
  const revReqBody = JSON.parse(revReqRes.body) as ReversalRequestAnswer;
  const reversalId = String(revReqBody.id ?? '');
  const approvalRequestId = String(revReqBody.approvalRequestId ?? '');
  expect(reversalId).not.toBe('');
  expect(approvalRequestId).not.toBe('');

  await signIn(finance, 'finance', '/');
  await finance.goto('/#reconciliation');
  const requestRow = finance.locator(`[data-request="${approvalRequestId}"]`); // ReversalsPart.tsx:58
  await expect(requestRow).toBeVisible({ timeout: 20_000 });
  await requestRow.locator('input[name="reason"]').fill(`qa-r9-9-approve-${Date.now()}`);
  const decideRes = await posted(
    finance,
    new RegExp(`^/api/approvals/${encodeURIComponent(approvalRequestId)}/decide$`),
    () => requestRow.locator('button[data-action="approve"]').click(),
  );
  expect(decideRes.ok, `POST /api/approvals/${approvalRequestId}/decide: ${decideRes.text}`).toBeTruthy();

  await teller.goto('/desk?branch=' + users.branch);
  const refundsTaskBtn = teller.locator('button[data-desk-task="refunds"]'); // refunds/task.tsx:53
  await expect(refundsTaskBtn).toBeVisible({ timeout: 20_000 });
  await refundsTaskBtn.click();

  const postBtn = teller.locator(`button[data-action="post"][data-id="${reversalId}"]`); // RefundsStep.tsx:87-90
  await expect(postBtn).toBeVisible({ timeout: 20_000 });
  await postBtn.click();

  const confirmDialog = teller.locator('dialog.xc-dialog'); // RefundsStep.tsx:128-136
  await expect(confirmDialog).toBeVisible({ timeout: 15_000 });
  const postRes = await posted(
    teller,
    new RegExp(`^/api/reversals/${encodeURIComponent(reversalId)}/post$`),
    () => confirmDialog.locator('.xc-dialog-actions button').last().click(),
  );
  expect(postRes.ok, `POST /api/reversals/${reversalId}/post: ${postRes.text}`).toBeTruthy();
}

test.describe.serial('QA-R9-9 desk draft begin, referral attach, counter pause/resume, phone verification', () => {
  test('(a) happy path: counterPause, counterResume, draftBegin, and attachReferral persist and read back after reload', async ({
    browser,
  }) => {
    const requiredRoles: readonly Role[] = ['teller', 'branchManager', 'finance'];
    for (const role of requiredRoles) {
      test.skip(!hasAccount(role), `no ${role} account in users.json: run scripts/reseed.sh`);
    }

    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const finance = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    try {
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await openShift(teller, manager);
      let started: 'started' | 'alreadyOpen' | undefined;
      let completedTxId: string | undefined;

      try {
        started = await startService(manager);

        // 1. counterPause (POST /api/queue/counter/pause) & counterResume (POST /api/queue/counter/resume)
        // on /desk/queue (CounterBar.tsx:66,75,84,98; queue-commands.ts:28,45-46).
        await teller.goto('/desk/queue');
        const counterBar = teller.locator('.desk-actions').first(); // CounterBar.tsx:66
        await expect(counterBar).toBeVisible({ timeout: 20_000 });

        const pauseBtn = teller.locator('button[name="pause"]'); // CounterBar.tsx:84
        const resumeBtn = teller.locator('button[name="resume"]'); // CounterBar.tsx:98
        const callNextBtn = teller.locator('button[name="callNext"]'); // CounterBar.tsx:75

        // Ensure counter starts ACTIVE before pausing.
        if (await resumeBtn.isVisible().catch(() => false)) {
          const preResume = await posted(teller, /^\/api\/queue\/counter\/resume$/, () => resumeBtn.click());
          expect(preResume.ok, `pre-resume counter: ${preResume.text}`).toBeTruthy();
        }
        await expect(pauseBtn).toBeVisible({ timeout: 15_000 });
        await expect(callNextBtn).toBeEnabled({ timeout: 15_000 });

        const pauseRes = await posted(teller, /^\/api\/queue\/counter\/pause$/, () => pauseBtn.click());
        expect(pauseRes.ok, `POST /api/queue/counter/pause: ${pauseRes.text}`).toBeTruthy();
        await expect(resumeBtn).toBeVisible({ timeout: 15_000 });
        await expect(callNextBtn).toBeDisabled({ timeout: 15_000 });

        // Reload /desk/queue and verify paused state reads back.
        await teller.reload();
        await expect(resumeBtn, 'resume button visible after reload when paused (CounterBar.tsx:98)').toBeVisible({
          timeout: 20_000,
        });
        await expect(callNextBtn).toBeDisabled({ timeout: 15_000 });
        const pausedBoardRes = await teller.request.get(
          `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
        );
        expect(pausedBoardRes.ok()).toBeTruthy();
        const pausedBoard = (await pausedBoardRes.json()) as QueueBoardViewAnswer;
        expect(pausedBoard.data?.myCounter?.status).toBe('PAUSED');
        await shot(teller, 'desk', 'r9-9-counter-paused');

        // Resume counter and verify active state reads back after reload (leaving the counter running per §3).
        const resumeRes = await posted(teller, /^\/api\/queue\/counter\/resume$/, () => resumeBtn.click());
        expect(resumeRes.ok, `POST /api/queue/counter/resume: ${resumeRes.text}`).toBeTruthy();
        await expect(pauseBtn).toBeVisible({ timeout: 15_000 });
        await expect(callNextBtn).toBeEnabled({ timeout: 15_000 });

        await teller.reload();
        await expect(pauseBtn, 'pause button visible after reload when resumed (CounterBar.tsx:84)').toBeVisible({
          timeout: 20_000,
        });
        await expect(callNextBtn).toBeEnabled({ timeout: 15_000 });
        const activeBoardRes = await teller.request.get(
          `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
        );
        expect(activeBoardRes.ok()).toBeTruthy();
        const activeBoard = (await activeBoardRes.json()) as QueueBoardViewAnswer;
        expect(activeBoard.data?.myCounter?.status).toBe('ACTIVE');
        await shot(teller, 'desk', 'r9-9-counter-resumed');

        // 2. draftBegin (POST /api/tx-drafts) on /staff/desk (desk-path.ts:43-56; customer/step.tsx:69,101-110).
        const currency = users.currency ?? 'USD';
        await withLang(teller, 'th');
        await teller.goto('/staff/desk');
        await expect(
          teller.locator(`select[name="currency"] option[value="${currency}"]`),
          `rate for ${currency} loaded in staff_desk_config (rate/step.tsx:81)`,
        ).toHaveCount(1, { timeout: 20_000 });

        const quoteRes = await posted(teller, /^\/api\/quotes$/, async () => {
          await teller.locator('select[name="type"]').selectOption('BUY'); // rate/step.tsx:62
          await teller.locator('select[name="currency"]').selectOption(currency); // rate/step.tsx:78
          await teller.locator('input[name="amount"]').fill(EXCHANGE_AMOUNT); // rate/step.tsx:94
        });
        expect(quoteRes.ok, `POST /api/quotes: ${quoteRes.text}`).toBeTruthy();
        await expect(teller.locator('.desk-rate-line strong')).toBeVisible({ timeout: 15_000 }); // rate/step.tsx:104

        await teller
          .getByRole('button', { name: /ถัดไป · พาสปอร์ต \/ ลูกค้า|Next · Passport \/ customer/ })
          .click(); // rate/step.tsx:136

        const skipBtn = teller.getByRole('button', { name: /ข้ามข้อมูลลูกค้า|Skip customer details/ }); // customer/step.tsx:256
        await expect(skipBtn).toBeVisible({ timeout: 15_000 });

        const draftResPromise = teller.waitForResponse(
          (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/tx-drafts',
          { timeout: 15_000 },
        );
        const reserveRes = await posted(teller, /^\/api\/staff\/rpc$/, () => skipBtn.click());
        const draftRes = await draftResPromise;
        const draftBodyText = await draftRes.text();
        expect(draftRes.ok(), `POST /api/tx-drafts: ${draftRes.status()} ${draftBodyText}`).toBeTruthy();
        expect(reserveRes.ok, `POST /api/staff/rpc reserveCustomer: ${reserveRes.text}`).toBeTruthy();

        const draftAnswer = JSON.parse(draftBodyText) as DraftBeginAnswer;
        const draftId = String(draftAnswer.id ?? '');
        expect(draftId, 'POST /api/tx-drafts must return draft id').not.toBe('');

        // Assert the OPEN draft row appears in operations.drafts.
        const draftsAfterBegin = await readStaffDrafts(teller);
        const openDraft = draftsAfterBegin.find((d) => d.id === draftId);
        expect(openDraft, `draft ${draftId} must appear in operations.drafts`).toBeTruthy();
        expect(openDraft?.status).toBe('OPEN');
        expect(openDraft?.type).toBe('BUY');
        expect(openDraft?.currency).toBe(currency);
        await shot(teller, 'desk', 'r9-9-draft-begun');

        // 3. Complete the 1-USD Cash exchange so ReceiptStep renders ReferralCodeField (receipt/step.tsx:40).
        const confirmBookingBtn = teller.getByRole('button', { name: /ยืนยันการจอง|Confirm booking/ }); // booking/step.tsx:54
        await expect(confirmBookingBtn).toBeVisible({ timeout: 15_000 });
        await confirmBookingBtn.click();

        const methodSelect = teller.locator('select[name="method"]'); // payment/step.tsx:135
        await expect(methodSelect).toBeVisible({ timeout: 15_000 });
        await methodSelect.selectOption('Cash');

        const accountField = teller.locator('input[name="account"]'); // payment/step.tsx:168
        await expect(accountField).toBeVisible({ timeout: 15_000 });
        if ((await accountField.inputValue()).trim() === '') {
          await accountField.fill('Cash');
        }
        await teller.locator('input[name="paymentChecked"]').check(); // payment/step.tsx:229

        teller.once('dialog', (d) => d.accept()); // payment/step.tsx:110
        const createRes = await posted(teller, /^\/api\/transactions$/, () =>
          teller.getByRole('button', { name: /ยืนยันและบันทึกบัญชี|Confirm and save/ }).click(), // payment/step.tsx:246
        );
        expect(createRes.ok, `POST /api/transactions: ${createRes.text}`).toBeTruthy();
        const createdTx = JSON.parse(createRes.body) as TransactionAnswer;
        const txId = String(createdTx.id ?? '');
        expect(txId).not.toBe('');

        const confirmRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/confirm$`),
          async () => {},
        );
        expect(confirmRes.ok, `POST /api/transactions/${txId}/confirm: ${confirmRes.text}`).toBeTruthy();

        const completeRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/complete$`),
          async () => {},
        );
        expect(completeRes.ok, `POST /api/transactions/${txId}/complete: ${completeRes.text}`).toBeTruthy();
        completedTxId = txId;

        const receiptRef = teller.locator('section.desk-form strong').first(); // receipt/step.tsx:34
        await expect(receiptRef).toHaveText(/^XC-\d{8}-\d{6}$/, { timeout: 20_000 });
        const receiptDocRef = (await receiptRef.innerText()).trim();

        const referralField = teller.locator('.desk-referral-field'); // ReferralCodeField.tsx:88
        await expect(referralField).toBeVisible({ timeout: 15_000 });
        const referralInput = referralField.locator('input.desk-referral-input'); // ReferralCodeField.tsx:92
        const referralBtn = referralField.locator('button.desk-referral-btn'); // ReferralCodeField.tsx:100
        const referralError = referralField.locator('p.desk-referral-error[role="alert"]'); // ReferralCodeField.tsx:112

        // Before attaching a valid code, verify (b) malformed referral code ('AB-12C') is refused with worded alert
        // in both th and en (switching via .xc-shell-header LangSwitch, LangSwitch.tsx:10-18) and preserves input.
        const langSwitchBtn = teller.locator('.xc-shell-header button[type="button"]'); // LangSwitch.tsx:10-18
        for (const lang of STAFF_LANGS) {
          const currentHtmlLang = await teller.locator('html').getAttribute('lang');
          if (currentHtmlLang !== lang) {
            await langSwitchBtn.click();
            await expect(teller.locator('html')).toHaveAttribute('lang', lang);
          }
          await referralInput.fill('AB-12C');
          const badRefRes = await posted(
            teller,
            new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/referral$`),
            () => referralBtn.click(),
          );
          expect(badRefRes.ok, `[${lang}] malformed referral code must be refused: ${badRefRes.text}`).toBe(false);
          await expect(referralError, `[${lang}] referral error alert must be visible`).toBeVisible({
            timeout: 10_000,
          });
          const errText = (await referralError.innerText()).trim();
          expect(errText.length).toBeGreaterThan(0);
          expect(errText).not.toMatch(RAW_CODE);
          if (lang === 'th') {
            expect(errText).toMatch(/[\u0E00-\u0E7F]/);
          } else {
            expect(errText).toMatch(/[A-Za-z]/);
          }
          await expect(referralInput, `[${lang}] referral input must keep typed value`).toHaveValue('AB-12C');
        }

        // 4. attachReferral (POST /api/transactions/{id}/referral) happy path (ReferralCodeField.tsx:28,71,122,123).
        await referralInput.fill(VALID_REFERRAL_CODE);
        const attachRes = await posted(
          teller,
          new RegExp(`^/api/transactions/${encodeURIComponent(txId)}/referral$`),
          () => referralBtn.click(),
        );
        expect(attachRes.ok, `POST /api/transactions/${txId}/referral: ${attachRes.text}`).toBeTruthy();

        const successEl = referralField.locator('p.desk-referral-success'); // ReferralCodeField.tsx:122
        await expect(successEl).toBeVisible({ timeout: 15_000 });
        await expect(successEl.locator('strong.desk-referral-code')).toHaveText(VALID_REFERRAL_CODE); // ReferralCodeField.tsx:123
        await shot(teller, 'desk', 'r9-9-referral-attached');

        // Reload and verify both the completed draft (draftBegin -> COMPLETED) and referral attribution read back.
        await teller.reload();
        await expect(teller.locator('.desk-shell')).toBeVisible({ timeout: 20_000 }); // DeskScreen.tsx:41

        const draftsAfterSave = await readStaffDrafts(teller);
        const completedDraft = draftsAfterSave.find((d) => d.id === draftId);
        expect(completedDraft, `draft ${draftId} must persist after reload`).toBeTruthy();
        expect(completedDraft?.status).toBe('COMPLETED');
        expect(completedDraft?.documentRef).toBe(receiptDocRef);

        const attrRes = await teller.request.get(
          `/api/v3/views/referral_attributions?branch=${encodeURIComponent(users.branch)}&transaction=${encodeURIComponent(txId)}`,
        );
        expect(attrRes.ok(), `GET /api/v3/views/referral_attributions status ${attrRes.status()}`).toBeTruthy();
        const attrBody = (await attrRes.json()) as ReferralAttributionsViewAnswer;
        const activeCapture = (attrBody.data?.rows ?? []).find((r) => r.status === 'ACTIVE');
        expect(activeCapture, `ACTIVE referral attribution for ${txId} must read back after reload`).toBeTruthy();
        expect(activeCapture?.code).toBe(VALID_REFERRAL_CODE);
        expect(activeCapture?.source).toBe('DESK');
      } finally {
        if (completedTxId) {
          await reverseCompletedExchange(teller, finance, completedTxId);
        }
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

  test('(b) invalid inputs are refused with worded messages in th and en and form state is preserved', async ({
    page,
  }) => {
    test.skip(!hasAccount('teller'), 'no teller account in users.json: run scripts/reseed.sh');

    const ts = Date.now();
    const marker = `qa-r9-9-${ts}`;
    const currency = users.currency ?? 'USD';

    await signIn(page, 'teller', `/desk?branch=${encodeURIComponent(users.branch)}`);

    for (const lang of STAFF_LANGS) {
      await withLang(page, lang);
      await page.goto(`/desk?branch=${encodeURIComponent(users.branch)}`);
      await expect(
        page.locator(`select[name="currency"] option[value="${currency}"]`),
        `[${lang}] rate for ${currency} loaded in desk_config (rate/step.tsx:81)`,
      ).toHaveCount(1, { timeout: 20_000 });

      // Step 0 -> Step 1 (rate/step.tsx:62-94,136).
      const quoteRes = await posted(page, /^\/api\/quotes$/, async () => {
        await page.locator('select[name="type"]').selectOption('BUY');
        await page.locator('select[name="currency"]').selectOption(currency);
        await page.locator('input[name="amount"]').fill(EXCHANGE_AMOUNT);
      });
      expect(quoteRes.ok, `[${lang}] POST /api/quotes: ${quoteRes.text}`).toBeTruthy();
      await expect(page.locator('.desk-rate-line strong')).toBeVisible({ timeout: 15_000 });
      await page
        .getByRole('button', { name: /ถัดไป · พาสปอร์ต \/ ลูกค้า|Next · Passport \/ customer/ })
        .click();

      const nameInput = page.locator('input[name="name"]'); // customer/step.tsx:211
      const passportNoInput = page.locator('input[name="passportNo"]'); // customer/step.tsx:215
      const nationalitySelect = page.locator('select[name="nationality"]'); // customer/step.tsx:219
      const passportFileInput = page.locator('input[name="passport"]'); // customer/step.tsx:185
      const customerSubmitBtn = page.locator('form.desk-form button[type="submit"]'); // customer/step.tsx:259
      const customerError = page.locator('form.desk-form p.desk-error[role="alert"]'); // customer/step.tsx:250

      await expect(nameInput).toBeVisible({ timeout: 15_000 });

      // Case 1: Customer step submitted without a passport scan/photo (customer/step.tsx:148-150; customer/i18n.ts:21,47).
      await nameInput.fill(marker);
      await passportNoInput.fill('P1234567');
      await nationalitySelect.selectOption('THA');
      await customerSubmitBtn.click();

      await expect(customerError, `[${lang}] missing passport file must show alert`).toBeVisible({ timeout: 10_000 });
      const noPhotoText = (await customerError.innerText()).trim();
      expect(noPhotoText.length).toBeGreaterThan(0);
      expect(noPhotoText).not.toMatch(RAW_CODE);
      if (lang === 'th') {
        expect(noPhotoText).toContain('กรุณาแนบไฟล์สแกนหรือรูปพาสปอร์ตก่อน');
      } else {
        expect(noPhotoText).toContain('Please attach a passport scan or photo first');
      }
      await expect(nameInput).toHaveValue(marker);
      await expect(passportNoInput).toHaveValue('P1234567');

      // Case 2: Customer step submitted with passport image attached but missing required passportNo (customer/step.tsx:152-154; customer/i18n.ts:22,48).
      await passportFileInput.setInputFiles({
        name: 'passport.png',
        mimeType: 'image/png',
        buffer: ONE_PIXEL_PNG,
      });
      await passportNoInput.fill('');
      await customerSubmitBtn.click();

      await expect(customerError, `[${lang}] missing required customer field must show alert`).toBeVisible({
        timeout: 10_000,
      });
      const incompleteText = (await customerError.innerText()).trim();
      expect(incompleteText.length).toBeGreaterThan(0);
      expect(incompleteText).not.toMatch(RAW_CODE);
      if (lang === 'th') {
        expect(incompleteText).toContain('กรุณากรอกชื่อ เลขพาสปอร์ต และสัญชาติ');
      } else {
        expect(incompleteText).toContain('Please fill in the name, passport number and nationality');
      }
      await expect(nameInput).toHaveValue(marker);

      // Advance to Step 2 via skip customer so CustomerLoyalty & PhoneVerify mount in aside.desk-summary (Summary.tsx:61, CustomerLoyalty.tsx:13).
      const skipBtn = page.getByRole('button', { name: /ข้ามข้อมูลลูกค้า|Skip customer details/ }); // customer/step.tsx:256
      const reserveRes = await posted(page, /^\/api\/rpc$/, () => skipBtn.click());
      expect(reserveRes.ok, `[${lang}] reserveCustomer on skip: ${reserveRes.text}`).toBeTruthy();

      const loyaltyBox = page.locator('.desk-loyalty'); // CustomerLoyalty.tsx:13
      await expect(loyaltyBox).toBeVisible({ timeout: 15_000 });
      const phoneInput = loyaltyBox.locator('input[name="phone"]'); // PhoneVerify.tsx:125
      const sendCodeBtn = loyaltyBox.locator('button[name="sendCode"]'); // PhoneVerify.tsx:133
      const phoneAlert = loyaltyBox.locator('p.desk-error[role="alert"]'); // PhoneVerify.tsx:162

      // Case 3: customerPhoneStart (POST /api/customers/{id}/phone-verifications) with malformed phone (PhoneVerify.tsx:71,86,125,133,162).
      const badPhone = `bad-phone-${ts}`;
      await phoneInput.fill(badPhone);
      const badPhoneRes = await posted(
        page,
        /^\/api\/customers\/[^/]+\/phone-verifications$/,
        () => sendCodeBtn.click(),
      );
      expect(badPhoneRes.ok, `[${lang}] malformed phone must be refused: ${badPhoneRes.text}`).toBe(false);
      await expect(phoneAlert, `[${lang}] malformed phone alert must be visible`).toBeVisible({ timeout: 10_000 });
      const badPhoneText = (await phoneAlert.innerText()).trim();
      expect(badPhoneText.length).toBeGreaterThan(0);
      expect(badPhoneText).not.toMatch(RAW_CODE);
      if (lang === 'th') {
        expect(badPhoneText).toMatch(/[\u0E00-\u0E7F]/);
      } else {
        expect(badPhoneText).toMatch(/[A-Za-z]/);
      }
      await expect(phoneInput, `[${lang}] phone input must keep typed value`).toHaveValue(badPhone);

      // Case 4: customerPhoneStart then customerPhoneConfirm (POST /api/customers/{id}/phone-verifications/{challengeId}/confirm)
      // with a wrong OTP code ('000000'), which must be refused (ticket #4293 §3; PhoneVerify.tsx:74,104,140,150,162; phone/i18n.ts:8,22).
      const validPhone = TEST_PHONE;
      await phoneInput.fill(validPhone);
      const startRes = await posted(
        page,
        /^\/api\/customers\/[^/]+\/phone-verifications$/,
        () => sendCodeBtn.click(),
      );

      if (startRes.ok) {
        const codeInput = loyaltyBox.locator('input[name="code"]'); // PhoneVerify.tsx:140
        const confirmCodeBtn = loyaltyBox.locator('button[name="confirmCode"]'); // PhoneVerify.tsx:150
        await expect(codeInput, `[${lang}] OTP code input must appear after customerPhoneStart`).toBeVisible({
          timeout: 10_000,
        });
        await codeInput.fill('000000');

        const confirmRes = await posted(
          page,
          /^\/api\/customers\/[^/]+\/phone-verifications\/[^/]+\/confirm$/,
          () => confirmCodeBtn.click(),
        );
        expect(confirmRes.ok, `[${lang}] wrong OTP code '000000' must be refused`).toBe(false);
        await expect(phoneAlert, `[${lang}] wrong OTP alert must be visible`).toBeVisible({ timeout: 10_000 });
        const wrongCodeText = (await phoneAlert.innerText()).trim();
        expect(wrongCodeText.length).toBeGreaterThan(0);
        expect(wrongCodeText).not.toMatch(RAW_CODE);
        if (lang === 'th') {
          expect(wrongCodeText).toContain('รหัส OTP ไม่ถูกต้อง');
        } else {
          expect(wrongCodeText).toContain('Incorrect OTP code');
        }
        await expect(codeInput, `[${lang}] code input must preserve wrong OTP`).toHaveValue('000000');
      } else {
        // If SMS service is unavailable on the host (503 smsUnavailable, phone/i18n.ts:11,25), assert worded alert and input preservation.
        // Any other refusal is a failure, not a reason to skip the wrong-OTP case (phone_verification_service.rs:84).
        expect(startRes.text, `[${lang}] customerPhoneStart refused for a reason other than smsUnavailable`).toMatch(/^503 .*smsUnavailable/s);
        console.log(`[R9-9] [${lang}] SMS unavailable on this host: wrong-OTP confirm not reachable (${startRes.text})`);
        await expect(phoneAlert, `[${lang}] SMS refusal alert must be visible`).toBeVisible({ timeout: 10_000 });
        const smsErrText = (await phoneAlert.innerText()).trim();
        expect(smsErrText.length).toBeGreaterThan(0);
        expect(smsErrText).not.toMatch(RAW_CODE);
        await expect(phoneInput).toHaveValue(validPhone);
      }
    }
    await shot(page, 'desk', 'r9-9-invalid-inputs');
  });

  test('(c) permission: customer has no desk and cannot execute desk draft, referral, counter, or phone verification commands', async ({
    page,
  }) => {
    // Also proven by smoke-rbac-matrix.spec.ts:99-117 (customer is redirected to /login?next=… from /desk?branch=MAIN and /staff).
    test.skip(!hasAccount('customer'), 'no customer account in users.json: run scripts/reseed.sh');

    await signIn(page, 'customer', '/app');

    // UI gate: customer navigating to /desk, /staff/desk, or /desk/queue is redirected away and desk UI is not rendered.
    for (const deskPath of [`/desk?branch=${encodeURIComponent(users.branch)}`, '/staff/desk', '/desk/queue']) {
      await page.goto(deskPath);
      await page.waitForLoadState('domcontentloaded');
      const pathname = new URL(page.url(), users.baseUrl).pathname;
      expect(pathname, `customer visiting ${deskPath} must be redirected to /login`).toBe('/login');
      await expect(page.locator('.desk-shell')).toHaveCount(0); // DeskScreen.tsx:41
    }

    // Backend gate: direct POSTs as customer to all 6 commands in scope are refused with 401 or 403.
    const headers = {
      Origin: users.baseUrl,
      'x-xchange-request': '1',
      'idempotency-key': `qa-r9-9-cust-${Date.now()}`,
    };
    const nilUuid = '00000000-0000-0000-0000-000000000000';

    const draftRes = await page.request.post('/api/tx-drafts', {
      headers: { ...headers, 'idempotency-key': `${headers['idempotency-key']}-draft` },
      data: { branch: users.branch, side: 'BUY', currency: 'USD', amount: '1' },
    });
    expect([401, 403], `POST /api/tx-drafts as customer returned ${draftRes.status()}`).toContain(draftRes.status());

    const refRes = await page.request.post(`/api/transactions/${nilUuid}/referral`, {
      headers: { ...headers, 'idempotency-key': `${headers['idempotency-key']}-ref` },
      data: { branch: users.branch, referralCode: VALID_REFERRAL_CODE },
    });
    expect([401, 403], `POST /api/transactions/{id}/referral as customer returned ${refRes.status()}`).toContain(
      refRes.status(),
    );

    const pauseRes = await page.request.post('/api/queue/counter/pause', {
      headers: { ...headers, 'idempotency-key': `${headers['idempotency-key']}-pause` },
      data: { branch: users.branch },
    });
    expect([401, 403], `POST /api/queue/counter/pause as customer returned ${pauseRes.status()}`).toContain(
      pauseRes.status(),
    );

    const resumeRes = await page.request.post('/api/queue/counter/resume', {
      headers: { ...headers, 'idempotency-key': `${headers['idempotency-key']}-resume` },
      data: { branch: users.branch },
    });
    expect([401, 403], `POST /api/queue/counter/resume as customer returned ${resumeRes.status()}`).toContain(
      resumeRes.status(),
    );

    const phoneStartRes = await page.request.post('/api/customers/CX-00000001/phone-verifications', {
      headers: { ...headers, 'idempotency-key': `${headers['idempotency-key']}-pstart` },
      data: { phone: TEST_PHONE },
    });
    expect(
      [401, 403],
      `POST /api/customers/{id}/phone-verifications as customer returned ${phoneStartRes.status()}`,
    ).toContain(phoneStartRes.status());

    const phoneConfirmRes = await page.request.post(`/api/customers/CX-00000001/phone-verifications/${nilUuid}/confirm`, {
      headers: { ...headers, 'idempotency-key': `${headers['idempotency-key']}-pconfirm` },
      data: { code: '000000' },
    });
    expect(
      [401, 403],
      `POST /api/customers/{id}/phone-verifications/{challengeId}/confirm as customer returned ${phoneConfirmRes.status()}`,
    ).toContain(phoneConfirmRes.status());

    await shot(page, 'desk', 'r9-9-customer-refused');
  });
});
