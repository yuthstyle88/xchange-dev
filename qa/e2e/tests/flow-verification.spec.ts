// QA-F9 — flow: customer verification across customer (/app/verification) and reviewer (/operations/verification)
// (design docs/design/qa-e2e.md §4 row F9; docs/design/blueprint/verification.md).
// Written by ticket #4028 (QA-F9); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - customer/verification/VerificationScreen.tsx:255,321,344,360,372,386,406,422,462,513,524,537,632,644,663
//   section[data-case-state=…], [data-action="start"], select[data-field="documentType"], #xc-verification-file,
//   [data-action="upload"], li[data-doc-id], [data-action="submit"], [data-action="resubmit"],
//   [data-decision-reason], [data-action="update-and-resubmit"]
// - operations/verification/VerificationScreen.tsx:720,746-749,790,885-890,922-936,953-992
//   [data-action="refresh"], tr[data-case-row][data-case-id="<id>"], [data-testid="case-status"],
//   [data-action="start-review"], [data-action="request-update"], [data-action="reject"],
//   [data-testid="reason-decision-panel"], select[name="reasonCode"],
//   [data-action="confirm-request-update"], [data-action="confirm-reject"]
// - api/crates/xc-verification/src/application/case_service.rs:195-209
//   first bytes checked for PNG signature [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] (1 B–15 MB)
import { expect, test, type Page } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users, type Role } from './helpers';

/** 1x1 transparent PNG (67 bytes; starts with PNG signature 89 50 4E 47 0D 0A 1A 0A, case_service.rs:199-201). */
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
  'base64',
);

interface CaseAnswer {
  id?: string;
  status?: string;
  decisionReasonCode?: string | null;
}

interface MyVerificationAnswer {
  readiness?: string;
  case?: (CaseAnswer & { documents?: Array<{ id?: string }> }) | null;
}

async function uploadPassportPng(customer: Page, caseId: string): Promise<void> {
  const select = customer.locator('section[data-case-state="DRAFT"] select[data-field="documentType"]');
  await expect(select, 'PASSPORT documentType select (VerificationScreen.tsx:386)').toBeVisible({ timeout: 15_000 });
  await select.selectOption('PASSPORT');

  await customer.locator('#xc-verification-file').setInputFiles({
    name: 'passport.png',
    mimeType: 'image/png',
    buffer: TINY_PNG,
  });

  const upRes = await posted(
    customer,
    new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/documents$`),
    () => customer.locator('button[data-action="upload"]').click(),
  );
  console.log(`[F9] POST /api/app/verification/cases/${caseId}/documents ${upRes.text}`);
  expect(upRes.ok, `upload passport PNG: ${upRes.text}`).toBeTruthy();

  await expect(
    customer.locator('li.xc-verification-doc-item[data-doc-id]').first(),
    'uploaded document row visible (VerificationScreen.tsx:462)',
  ).toBeVisible({ timeout: 15_000 });
}

async function selectCaseOnStaff(admin: Page, caseId: string): Promise<void> {
  const row = admin.locator(`tr[data-case-row][data-case-id="${caseId}"]`);
  if (!(await row.isVisible().catch(() => false))) {
    await admin.reload();
  }
  await expect(row, `queue row for case ${caseId} (VerificationScreen.tsx:746)`).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(admin.getByTestId('case-detail-view'), 'selected case detail view').toBeVisible({ timeout: 15_000 });
}

test.describe.serial('F9 customer verification', () => {
  test('customer submits passport, admin requests update, customer resubmits, admin rejects', async ({ browser }) => {
    const required: readonly Role[] = ['customer', 'admin'];
    for (const role of required) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[F9] skip: ${reason}`);
        test.skip(true, reason);
      }
    }

    const customer = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
    const admin = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });

    try {
      // Step 1: customer opens /app/verification; start if NONE, update-and-resubmit if REJECTED/EXPIRED,
      // or continue from an already-open case so we reach a clean DRAFT (or SUBMITTED) with its case id.
      await signIn(customer, 'customer', '/app/verification');
      await expect(
        customer.locator('[data-verification-screen="true"]'),
        'customer verification screen loaded (VerificationScreen.tsx:257)',
      ).toBeVisible({ timeout: 20_000 });

      const stateSection = customer.locator('section[data-case-state]').first();
      await expect(stateSection, 'initial section[data-case-state]').toBeVisible({ timeout: 20_000 });
      const initialState = (await stateSection.getAttribute('data-case-state')) ?? 'NONE';
      console.log(`[F9] initial customer case state: ${initialState}`);

      let caseId = '';

      if (initialState === 'VERIFIED') {
        throw new Error('customer verification case is VERIFIED; cannot open a new case without changing shared data');
      }

      // If an earlier interrupted run left an open case in SUBMITTED / UNDER_REVIEW / NEEDS_UPDATE,
      // reject it first via admin so Step 1 starts a fresh DRAFT case cleanly through the full 8-step table.
      if (initialState === 'SUBMITTED' || initialState === 'UNDER_REVIEW' || initialState === 'NEEDS_UPDATE') {
        const myVerRes = await customer.request.get('/api/v3/views/my_verification');
        const myVer = (await myVerRes.json().catch(() => ({}))) as MyVerificationAnswer;
        const leftoverId = String(myVer.case?.id ?? '');
        expect(leftoverId, 'leftover open case id from my_verification').not.toBe('');

        if (initialState === 'NEEDS_UPDATE') {
          const docCount = await customer.locator('li.xc-verification-doc-item[data-doc-id]').count();
          if (docCount === 0) {
            await customer.locator('#xc-verification-file').setInputFiles({
              name: 'passport.png',
              mimeType: 'image/png',
              buffer: TINY_PNG,
            });
            const upLeftover = await posted(
              customer,
              new RegExp(`^/api/app/verification/cases/${encodeURIComponent(leftoverId)}/documents$`),
              () => customer.locator('button[data-action="upload"]').click(),
            );
            expect(upLeftover.ok, `upload doc on leftover NEEDS_UPDATE: ${upLeftover.text}`).toBeTruthy();
          }
          const resubLeftover = await posted(
            customer,
            new RegExp(`^/api/app/verification/cases/${encodeURIComponent(leftoverId)}/submit$`),
            () => customer.locator('button[data-action="resubmit"]').click(),
          );
          expect(resubLeftover.ok, `resubmit leftover NEEDS_UPDATE: ${resubLeftover.text}`).toBeTruthy();
        }

        await signIn(admin, 'admin', '/operations/verification');
        await selectCaseOnStaff(admin, leftoverId);
        const curStatus = (await admin.getByTestId('case-status').innerText()).trim();
        if (curStatus === 'SUBMITTED') {
          const sr = await posted(
            admin,
            new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(leftoverId)}/start-review$`),
            () => admin.locator('button[data-action="start-review"]').click(),
          );
          expect(sr.ok, `start-review leftover case: ${sr.text}`).toBeTruthy();
        }
        await admin.locator('button[data-action="reject"]').click();
        await admin.locator('[data-testid="reason-decision-panel"] select[name="reasonCode"]').selectOption('OTHER');
        const rej = await posted(
          admin,
          new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(leftoverId)}/reject$`),
          () => admin.locator('button[data-action="confirm-reject"]').click(),
        );
        expect(rej.ok, `reject leftover case: ${rej.text}`).toBeTruthy();

        await customer.reload();
        await expect(customer.locator('section[data-case-state="REJECTED"]')).toBeVisible({ timeout: 20_000 });
      }

      const currentState = (await customer.locator('section[data-case-state]').first().getAttribute('data-case-state')) ?? 'NONE';
      if (currentState === 'NONE') {
        const openRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
          customer.locator('button[data-action="start"]').click(),
        );
        console.log(`[F9] POST /api/app/verification/cases (start) ${openRes.text}`);
        expect(openRes.ok, `POST /api/app/verification/cases: ${openRes.text}`).toBeTruthy();
        const opened = JSON.parse(openRes.body) as CaseAnswer;
        caseId = String(opened.id ?? '');
      } else if (currentState === 'REJECTED' || currentState === 'EXPIRED') {
        const openRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
          customer.locator('button[data-action="update-and-resubmit"]').click(),
        );
        console.log(`[F9] POST /api/app/verification/cases (update-and-resubmit) ${openRes.text}`);
        expect(openRes.ok, `POST /api/app/verification/cases: ${openRes.text}`).toBeTruthy();
        const opened = JSON.parse(openRes.body) as CaseAnswer;
        caseId = String(opened.id ?? '');
      } else if (currentState === 'DRAFT') {
        const myVerRes = await customer.request.get('/api/v3/views/my_verification');
        const myVer = (await myVerRes.json().catch(() => ({}))) as MyVerificationAnswer;
        caseId = String(myVer.case?.id ?? '');
      }

      expect(caseId, 'opened/active verification case id').not.toBe('');
      await expect(
        customer.locator('section[data-case-state="DRAFT"]'),
        'customer case is in DRAFT state (VerificationScreen.tsx:360)',
      ).toBeVisible({ timeout: 20_000 });
      await shot(customer, 'verification', 'f9-1-draft');

      // Step 2: customer selects PASSPORT, sets input files with tiny real PNG buffer, clicks upload -> document row appears.
      await uploadPassportPng(customer, caseId);
      await shot(customer, 'verification', 'f9-2-uploaded');

      // Step 3: customer clicks submit -> SUBMITTED.
      const submitRes = await posted(
        customer,
        new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/submit$`),
        () => customer.locator('button[data-action="submit"]').click(),
      );
      console.log(`[F9] POST /api/app/verification/cases/${caseId}/submit ${submitRes.text}`);
      expect(submitRes.ok, `submit verification case: ${submitRes.text}`).toBeTruthy();
      await expect(
        customer.locator('section[data-case-state="SUBMITTED"]'),
        'customer sees SUBMITTED state (VerificationScreen.tsx:537)',
      ).toBeVisible({ timeout: 20_000 });
      await shot(customer, 'verification', 'f9-3-submitted');

      // Step 4: admin opens /operations/verification, selects row [data-case-id=<id>], clicks start-review -> UNDER_REVIEW.
      await signIn(admin, 'admin', '/operations/verification');
      await expect(
        admin.getByTestId('verification-screen'),
        'operations verification screen (VerificationScreen.tsx:685)',
      ).toBeVisible({ timeout: 20_000 });
      await selectCaseOnStaff(admin, caseId);

      const startReview1 = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/start-review$`),
        () => admin.locator('button[data-action="start-review"]').click(),
      );
      console.log(`[F9] POST /api/operations/verification/cases/${caseId}/start-review (1) ${startReview1.text}`);
      expect(startReview1.ok, `start-review (1): ${startReview1.text}`).toBeTruthy();
      await expect(
        admin.getByTestId('case-status'),
        'case-status is UNDER_REVIEW after first start-review (VerificationScreen.tsx:790)',
      ).toHaveText('UNDER_REVIEW', { timeout: 15_000 });
      await shot(admin, 'verification', 'f9-4-under-review-1');

      // Step 5: admin clicks request-update, chooses DOCUMENT_UNREADABLE, confirms -> NEEDS_UPDATE.
      await admin.locator('button[data-action="request-update"]').click();
      const reasonPanel1 = admin.getByTestId('reason-decision-panel');
      await expect(reasonPanel1, 'reason-decision-panel open (VerificationScreen.tsx:953)').toBeVisible({
        timeout: 15_000,
      });
      await reasonPanel1.locator('select[name="reasonCode"]').selectOption('DOCUMENT_UNREADABLE');
      const reqUpdateRes = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/request-update$`),
        () => reasonPanel1.locator('button[data-action="confirm-request-update"]').click(),
      );
      console.log(`[F9] POST /api/operations/verification/cases/${caseId}/request-update ${reqUpdateRes.text}`);
      expect(reqUpdateRes.ok, `request-update: ${reqUpdateRes.text}`).toBeTruthy();
      await expect(
        admin.getByTestId('case-status'),
        'case-status is NEEDS_UPDATE after request-update (VerificationScreen.tsx:790)',
      ).toHaveText('NEEDS_UPDATE', { timeout: 15_000 });
      await shot(admin, 'verification', 'f9-5-needs-update');

      // Step 6: customer reloads -> NEEDS_UPDATE, reason DOCUMENT_UNREADABLE shows, resubmit -> SUBMITTED.
      await customer.reload();
      const needsUpdateSection = customer.locator('section[data-case-state="NEEDS_UPDATE"]');
      await expect(
        needsUpdateSection,
        'customer sees NEEDS_UPDATE section (VerificationScreen.tsx:360)',
      ).toBeVisible({ timeout: 20_000 });
      await expect(
        needsUpdateSection.locator('[data-decision-reason]'),
        'customer sees DOCUMENT_UNREADABLE reason (VerificationScreen.tsx:372)',
      ).toHaveText('DOCUMENT_UNREADABLE', { timeout: 15_000 });
      await shot(customer, 'verification', 'f9-6a-customer-needs-update');

      const resubmitRes = await posted(
        customer,
        new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/submit$`),
        () => needsUpdateSection.locator('button[data-action="resubmit"]').click(),
      );
      console.log(`[F9] POST /api/app/verification/cases/${caseId}/submit (resubmit) ${resubmitRes.text}`);
      expect(resubmitRes.ok, `resubmit verification case: ${resubmitRes.text}`).toBeTruthy();
      await expect(
        customer.locator('section[data-case-state="SUBMITTED"]'),
        'customer sees SUBMITTED after resubmit (VerificationScreen.tsx:537)',
      ).toBeVisible({ timeout: 20_000 });
      await shot(customer, 'verification', 'f9-6b-resubmitted');

      // Step 7: admin clicks start-review, then reject with OTHER and confirms -> REJECTED; row leaves the queue.
      await admin.reload();
      await selectCaseOnStaff(admin, caseId);

      const startReview2 = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/start-review$`),
        () => admin.locator('button[data-action="start-review"]').click(),
      );
      console.log(`[F9] POST /api/operations/verification/cases/${caseId}/start-review (2) ${startReview2.text}`);
      expect(startReview2.ok, `start-review (2): ${startReview2.text}`).toBeTruthy();
      await expect(admin.getByTestId('case-status')).toHaveText('UNDER_REVIEW', { timeout: 15_000 });

      await admin.locator('button[data-action="reject"]').click();
      const reasonPanel2 = admin.getByTestId('reason-decision-panel');
      await expect(reasonPanel2, 'reason-decision-panel open for reject (VerificationScreen.tsx:953)').toBeVisible({
        timeout: 15_000,
      });
      await reasonPanel2.locator('select[name="reasonCode"]').selectOption('OTHER');
      const rejectRes = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/reject$`),
        () => reasonPanel2.locator('button[data-action="confirm-reject"]').click(),
      );
      console.log(`[F9] POST /api/operations/verification/cases/${caseId}/reject ${rejectRes.text}`);
      expect(rejectRes.ok, `reject verification case: ${rejectRes.text}`).toBeTruthy();
      await expect(
        admin.getByTestId('case-status'),
        'case-status is REJECTED after reject (VerificationScreen.tsx:790)',
      ).toHaveText('REJECTED', { timeout: 15_000 });
      await expect(
        admin.locator(`tr[data-case-row][data-case-id="${caseId}"]`),
        'rejected case row leaves the verification queue',
      ).toHaveCount(0, { timeout: 15_000 });
      await shot(admin, 'verification', 'f9-7-rejected-staff');

      // Step 8: customer reloads -> REJECTED, reason OTHER shown, update-and-resubmit visible.
      await customer.reload();
      const rejectedSection = customer.locator('section[data-case-state="REJECTED"]');
      await expect(
        rejectedSection,
        'customer sees REJECTED section (VerificationScreen.tsx:632)',
      ).toBeVisible({ timeout: 20_000 });
      await expect(
        rejectedSection.locator('[data-decision-reason]'),
        'customer sees OTHER decision reason (VerificationScreen.tsx:644)',
      ).toHaveText('OTHER', { timeout: 15_000 });
      await expect(
        rejectedSection.locator('button[data-action="update-and-resubmit"]'),
        'update-and-resubmit button visible for the next run (VerificationScreen.tsx:663)',
      ).toBeVisible({ timeout: 15_000 });
      await shot(customer, 'verification', 'f9-8-rejected-customer');
    } finally {
      await customer.close().catch(() => {});
      await admin.close().catch(() => {});
    }
  });
});
