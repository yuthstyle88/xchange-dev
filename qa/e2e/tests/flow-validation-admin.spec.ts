// QA-R9-14 — e2e round 9: invalid input on the admin forms (rate drafts never submitted)
// (design docs/design/qa-e2e.md §9.1 (b), §9.2 row 14; ticket #4298).
// Written by ticket #4298 (QA-R9-14); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Caller citations (`git grep -n "<route>" -- web/src` on `main`):
// 1. Rate drafts and policy (`/#rates`, screen root `.admin-rates-screen` at web/src/screens/admin/rates/Rates.tsx:313):
//    - `rateDraftCreate` (`POST /api/rate-drafts`): web/src/screens/admin/rates/RatesDraftCard.tsx:157
//    - `rateDraftEdit` (`PUT /api/rate-drafts/{id}`): web/src/screens/admin/rates/RatesDraftCard.tsx:278, :330
//    - `ratePolicySave` (`POST /api/rate-policies`): web/src/screens/admin/rates/Rates.tsx:54,
//      web/src/screens/admin/rates/RatesDraftCard.tsx:310
//    - Draft i18n: web/src/screens/admin/rates/i18n.draft.ts:29-30 (th), :61-62 (en)
//    - Policy i18n: web/src/screens/admin/rates/i18n.policy.ts:13-14,24-26 (th), :41-42,52-54 (en)
//    - Rule (F4, E-4): NEVER call `POST /api/rate-drafts/{id}/submit` (RatesDraftCard.tsx:210) or
//      `POST /api/rate-drafts/{id}/publish` (RatesDraftCard.tsx:234).
// 2. Approval reason (`/#approvals`, screen root `.admin-approvals-screen` at web/src/screens/admin/approvals/Approvals.tsx:319):
//    - `approvalDecide` (`POST /api/approvals/{id}/decide`): web/src/screens/admin/parts/decide.ts:21
//      (wired in web/src/screens/admin/approvals/Approvals.tsx:120-136, :262)
//    - Approvals i18n: web/src/screens/admin/approvals/i18n.ts:9 (th), :42 (en)
// 3. Verification reject and request-update reason (`/operations/verification`, screen root `data-testid="verification-screen"`
//    at web/src/screens/operations/verification/VerificationScreen.tsx:685):
//    - `verificationCaseReject` (`POST /api/operations/verification/cases/{id}/reject`):
//      web/src/screens/operations/verification/VerificationScreen.tsx:432
//    - `verificationCaseRequestUpdate` (`POST /api/operations/verification/cases/{id}/request-update`):
//      web/src/screens/operations/verification/VerificationScreen.tsx:441
//    - Verification i18n: web/src/screens/operations/verification/i18n.ts:54 (th), :118 (en)
// 4. Slot capacity (`/#booking-slots`, screen root `.booking-slots-screen` at web/src/screens/admin/booking-slots/BookingSlots.tsx:383):
//    - `slotCapacitySet` (`PUT /api/booking-slots/{id}/capacity`):
//      web/src/screens/admin/booking-slots/BookingSlots.tsx:276
//    - Booking slots i18n: web/src/screens/admin/booking-slots/i18n.ts:39 (th), :89 (en)
// 5. Closure add (`/#booking-slots`, screen root `.booking-slots-screen` at web/src/screens/admin/booking-slots/BookingSlots.tsx:383):
//    - `closureAdd` (`POST /api/branches/{id}/closures`):
//      web/src/screens/admin/booking-slots/BookingSlots.tsx:219
//    - Booking slots i18n: web/src/screens/admin/booking-slots/i18n.ts:38 (th), :88 (en)
import { expect, test, type Browser, type Page } from '@playwright/test';
import {
  hasAccount,
  mePermissions,
  posted,
  shot,
  signIn,
  STAFF_LANGS,
  users,
  watchConsole,
  withLang,
} from './helpers';

type StaffLang = 'th' | 'en';
const STAFF_LANG_LIST = STAFF_LANGS as readonly StaffLang[];

/** 1x1 transparent PNG (67 bytes; starts with PNG signature 89 50 4E 47 0D 0A 1A 0A, case_service.rs:199-201). */
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
  'base64',
);

const RATE_DRAFT_MESSAGES: Record<
  StaffLang,
  {
    manualValueRequired: string;
    reasonRequired: string;
  }
> = {
  // web/src/screens/admin/rates/i18n.draft.ts:29-30, :61-62
  th: {
    manualValueRequired: 'กรุณาระบุเรทที่ถูกต้อง',
    reasonRequired: 'กรุณาระบุเหตุผล',
  },
  en: {
    manualValueRequired: 'Valid manual rate is required',
    reasonRequired: 'Reason is required',
  },
};

const RATE_POLICY_MESSAGES: Record<
  StaffLang,
  {
    guardInvalid: string;
    maxAgeInvalid: string;
    staleLimitRequired: string;
  }
> = {
  // web/src/screens/admin/rates/i18n.policy.ts:24-26, :52-54
  th: {
    guardInvalid: 'กรอบเรทต้องเป็นทศนิยมไม่เกิน 4 ตำแหน่ง และค่าต่ำสุดต้องไม่เกินค่าสูงสุด',
    maxAgeInvalid: 'อายุเรทอ้างอิงสูงสุดต้องเป็นจำนวนเต็มบวก (ชั่วโมง)',
    staleLimitRequired: 'กรุณาระบุวงเงินมากกว่า 0 (ทศนิยมไม่เกิน 2 ตำแหน่ง)',
  },
  en: {
    guardInvalid: 'Each guard side needs min <= max with at most 4 decimal places',
    maxAgeInvalid: 'Max reference age must be a whole number of hours above 0',
    staleLimitRequired: 'Enter a stale limit above 0 with at most 2 decimal places',
  },
};

const APPROVAL_DECIDE_MESSAGES: Record<StaffLang, string> = {
  // web/src/screens/admin/approvals/i18n.ts:9, :42
  th: 'ต้องระบุเหตุผลเมื่อปฏิเสธ',
  en: 'Reason is required for rejection',
};

const VERIFICATION_REASON_MESSAGES: Record<StaffLang, string> = {
  // web/src/screens/operations/verification/i18n.ts:54, :118
  th: 'ข้อมูลไม่ถูกต้อง กรุณาเลือกรหัสเหตุผล',
  en: 'Validation failed: please select a reason code',
};

const BOOKING_SLOTS_MESSAGES: Record<
  StaffLang,
  {
    slotCapacityValidationFailed: string;
    closureValidationFailed: string;
  }
> = {
  // web/src/screens/admin/booking-slots/i18n.ts:38-39, :88-89
  th: {
    slotCapacityValidationFailed: 'ความจุต้องมากกว่า 0',
    closureValidationFailed: 'กรุณากรอกวันที่และเหตุผลให้ครบถ้วน',
  },
  en: {
    slotCapacityValidationFailed: 'Capacity must be greater than 0',
    closureValidationFailed: 'Date and reason are required',
  },
};

function futureDateBangkok(daysAhead: number): string {
  const d = new Date(Date.now() + 7 * 3600_000 + daysAhead * 24 * 3600_000);
  return d.toISOString().slice(0, 10);
}

/** Waits for one PUT whose path matches while `act` runs; mirrors `posted` (`helpers.ts:365-371`) for PUT commands. */
async function putted(
  page: Page,
  path: RegExp,
  act: () => Promise<void>,
): Promise<{ ok: boolean; text: string; body: string }> {
  const res = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && path.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  await act();
  const r = await res;
  const body = await r.text().catch(() => '');
  return { ok: r.ok(), text: `${r.status()} ${body.slice(0, 300)}`, body };
}

/**
 * Sets an arbitrary (including empty or unrecognized) value on `select[name="reasonCode"]`
 * in `VerificationScreen.tsx:960` and dispatches `change` so both DOM ref (`reasonSelectRef.value`)
 * and Solid signal (`reasonCode()`) hold `rawValue`.
 */
async function forceVerificationReasonValue(page: Page, rawValue: string): Promise<void> {
  const select = page.locator('[data-testid="reason-decision-panel"] select[name="reasonCode"]');
  await expect(select).toBeVisible({ timeout: 15_000 });
  await select.evaluate((el: HTMLSelectElement, val: string) => {
    let opt = Array.from(el.options).find((o) => o.value === val);
    if (!opt) {
      opt = document.createElement('option');
      opt.value = val;
      opt.textContent = val || '--';
      el.prepend(opt);
    }
    el.value = val;
    opt.selected = true;
    el.dispatchEvent(new Event('change', { bubbles: true }));
  }, rawValue);
}

/**
 * Ensures at least one `PENDING` approval request exists on `/#approvals`.
 * If none is currently pending and `finance` is configured, creates and submits a 1-note
 * `EXTERNAL_IN` cash transfer carrying `runTag` (which stays `DRAFT` and never touches the safe
 * when rejected in `finally`).
 */
async function ensurePendingApprovalRequest(
  browser: Browser,
  admin: Page,
  runTag: string,
): Promise<{ createdReqId: string | null }> {
  const res = await admin.request.get(
    `/api/v3/views/approval_requests?branch=${encodeURIComponent(users.branch)}&status=PENDING`,
  );
  if (res.ok()) {
    const body = (await res.json().catch(() => ({}))) as {
      data?: { requests?: Array<{ id: string; status: string }> };
    };
    if ((body.data?.requests?.length ?? 0) > 0) {
      return { createdReqId: null };
    }
  }

  if (!hasAccount('finance')) {
    return { createdReqId: null };
  }

  const finance = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });
  try {
    await signIn(finance, 'finance', '/staff');
    await finance.getByTestId('transfer-new').click();
    const form = finance.getByTestId('transfer-form');
    await expect(form).toBeVisible({ timeout: 15_000 });
    await form.getByTestId('transfer-kind').selectOption('EXTERNAL_IN');
    await form.getByTestId('transfer-note').fill(runTag);
    const firstThbInput = form
      .locator('[data-denomination-row]')
      .filter({ hasText: 'THB' })
      .first()
      .locator('input[data-denomination-id]');
    await expect(firstThbInput).toBeVisible({ timeout: 20_000 });
    await firstThbInput.fill('1');

    const submitted = finance.waitForResponse(
      (r) => r.request().method() === 'POST' && /\/api\/cash-transfers\/[^/]+\/submit$/.test(new URL(r.url()).pathname),
      { timeout: 15_000 },
    );
    const created = await posted(finance, /^\/api\/cash-transfers$/, () => form.getByTestId('transfer-save').click());
    expect(created.ok, `finance create transfer for approval validation: ${created.text}`).toBeTruthy();
    const transferId = String((JSON.parse(created.body) as { id?: string }).id ?? '');
    const subRes = await submitted;
    expect(subRes.ok(), `finance submit transfer ${transferId}: ${subRes.status()}`).toBeTruthy();

    const checkRes = await admin.request.get(
      `/api/v3/views/approval_requests?branch=${encodeURIComponent(users.branch)}&status=PENDING&type=CASH_TRANSFER`,
    );
    const checkBody = (await checkRes.json().catch(() => ({}))) as {
      data?: { requests?: Array<{ id: string; objectId: string; status: string }> };
    };
    const match = checkBody.data?.requests?.find((r) => r.objectId === transferId && r.status === 'PENDING');
    return { createdReqId: match?.id ?? null };
  } finally {
    await finance.close().catch(() => {});
  }
}

/**
 * Ensures at least one decidable (`SUBMITTED` or `UNDER_REVIEW`) verification case row exists on
 * `/operations/verification`. If the queue is empty and `customer` is configured, opens/submits a
 * customer verification case with `TINY_PNG` and returns its `caseId` so `finally` can reject it cleanly.
 */
async function ensureDecidableVerificationCase(
  browser: Browser,
  admin: Page,
): Promise<{ targetCaseId: string | null; createdForTest: boolean }> {
  const qRes = await admin.request.get('/api/v3/views/verification_queue');
  if (qRes.ok()) {
    const qBody = (await qRes.json().catch(() => ({}))) as {
      data?: Array<{ id?: string; status?: string }>;
    };
    const existing = (qBody.data ?? []).find(
      (item) => item.id && (item.status === 'SUBMITTED' || item.status === 'UNDER_REVIEW'),
    );
    if (existing?.id) {
      return { targetCaseId: existing.id, createdForTest: false };
    }
  }

  if (!hasAccount('customer')) {
    return { targetCaseId: null, createdForTest: false };
  }

  const customer = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
  try {
    await signIn(customer, 'customer', '/app/verification');
    await expect(customer.locator('[data-verification-screen="true"]')).toBeVisible({ timeout: 20_000 });
    const stateSection = customer.locator('section[data-case-state]').first();
    await expect(stateSection).toBeVisible({ timeout: 20_000 });
    const state = (await stateSection.getAttribute('data-case-state')) ?? 'NONE';

    if (state === 'VERIFIED') {
      return { targetCaseId: null, createdForTest: false };
    }

    let caseId = '';
    if (state === 'NONE') {
      const openRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
        customer.locator('button[data-action="start"]').click(),
      );
      expect(openRes.ok, `customer start verification case: ${openRes.text}`).toBeTruthy();
      caseId = String((JSON.parse(openRes.body) as { id?: string }).id ?? '');
    } else if (state === 'REJECTED' || state === 'EXPIRED') {
      const openRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
        customer.locator('button[data-action="update-and-resubmit"]').click(),
      );
      expect(openRes.ok, `customer update-and-resubmit verification case: ${openRes.text}`).toBeTruthy();
      caseId = String((JSON.parse(openRes.body) as { id?: string }).id ?? '');
    } else {
      const myVerRes = await customer.request.get('/api/v3/views/my_verification');
      const myVer = (await myVerRes.json().catch(() => ({}))) as {
        case?: { id?: string; status?: string } | null;
      };
      caseId = String(myVer.case?.id ?? '');
    }

    if (!caseId) {
      return { targetCaseId: null, createdForTest: false };
    }

    const curState = (await customer.locator('section[data-case-state]').first().getAttribute('data-case-state')) ?? '';
    if (curState === 'DRAFT') {
      const select = customer.locator('section[data-case-state="DRAFT"] select[data-field="documentType"]');
      await expect(select).toBeVisible({ timeout: 15_000 });
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
      expect(upRes.ok, `upload passport PNG: ${upRes.text}`).toBeTruthy();
      const subRes = await posted(
        customer,
        new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/submit$`),
        () => customer.locator('button[data-action="submit"]').click(),
      );
      expect(subRes.ok, `submit verification case: ${subRes.text}`).toBeTruthy();
    } else if (curState === 'NEEDS_UPDATE') {
      const docCount = await customer.locator('li.xc-verification-doc-item[data-doc-id]').count();
      if (docCount === 0) {
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
        expect(upRes.ok, `upload doc on NEEDS_UPDATE: ${upRes.text}`).toBeTruthy();
      }
      const resubRes = await posted(
        customer,
        new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/submit$`),
        () => customer.locator('button[data-action="resubmit"]').click(),
      );
      expect(resubRes.ok, `resubmit verification case: ${resubRes.text}`).toBeTruthy();
    }

    return { targetCaseId: caseId, createdForTest: true };
  } finally {
    await customer.close().catch(() => {});
  }
}

test.describe.serial('QA-R9-14: invalid input on the admin forms (rate drafts never submitted)', () => {
  test('1. /#rates: rateDraftCreate, rateDraftEdit, and ratePolicySave refuse invalid inputs in th and en and never submit or publish a draft', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json');
    const runTag = `qa-r9-validation-admin-${Date.now()}`;
    const log = watchConsole(page);

    await signIn(page, 'admin', '/');
    const perms = await mePermissions(page);
    test.skip(
      !perms.includes('RatesDraft') || !perms.includes('RatePolicySave'),
      `admin lacks RatesDraft or RatePolicySave (${perms.join(', ')})`,
    );

    const forbiddenDraftSubmissions: string[] = [];
    const unexpectedWrites: string[] = [];
    let allowDraftSetupWrites = false;

    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      const method = req.method();
      // Never submit or publish a rate draft (F4, E-4):
      // - POST /api/rate-drafts/{id}/submit (RatesDraftCard.tsx:210)
      // - POST /api/rate-drafts/{id}/publish (RatesDraftCard.tsx:234)
      if (/^\/api\/rate-drafts\/[^/]+\/(submit|publish)$/.test(path)) {
        forbiddenDraftSubmissions.push(`${method} ${path}`);
      }
      if (!allowDraftSetupWrites) {
        if (
          (method === 'POST' && path === '/api/rate-drafts') ||
          (method === 'PUT' && /^\/api\/rate-drafts\/[^/]+$/.test(path)) ||
          (method === 'POST' && path === '/api/rate-policies')
        ) {
          unexpectedWrites.push(`${method} ${path}`);
        }
      }
    });

    for (const lang of STAFF_LANG_LIST) {
      await withLang(page, lang);
      await page.goto('about:blank');

      const initPublishedRes = page.waitForResponse(
        (r) => r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/published_rates',
        { timeout: 20_000 },
      );
      let initSawDraftsList = false;
      const initDraftsRes = page.waitForResponse(
        (r) => {
          if (r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/rate_drafts') {
            initSawDraftsList = true;
            return true;
          }
          return false;
        },
        { timeout: 20_000 },
      );
      const initDraftDetailRes = page
        .waitForResponse(
          (r) =>
            initSawDraftsList &&
            r.request().method() === 'GET' &&
            new URL(r.url()).pathname === '/api/v3/views/rate_draft',
          { timeout: 15_000 },
        )
        .catch(() => null);

      await page.goto('/#rates');

      // Screen root `.admin-rates-screen` (web/src/screens/admin/rates/Rates.tsx:313).
      const screen = page.locator('.admin-rates-screen');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      // Policy card root: `.rates-section` containing `[data-testid="save-policy"]` (RatesPolicyCard.tsx:391, :402).
      const policyCard = screen.locator('.rates-section').filter({ has: page.getByTestId('save-policy') });
      await expect(policyCard).toBeVisible({ timeout: 20_000 });

      // Draft card root: `.rates-section` containing `.btn-new-draft` (RatesDraftCard.tsx:355, :372).
      const draftCard = screen.locator('.rates-section').filter({ has: page.locator('button.btn-new-draft') });
      await expect(draftCard).toBeVisible({ timeout: 20_000 });

      // Wait for initial `readPublished` and `readDrafts` (plus follow-up `rate_draft` if an open draft exists,
      // Rates.tsx:122-160, :224-230) before checking `saveLinesBtn` or clicking `btn-new-draft`.
      const [, draftsResp] = await Promise.all([initPublishedRes, initDraftsRes]);
      const draftsJson = (await draftsResp.json().catch(() => null)) as {
        data?: { drafts?: Array<{ id: string; status: string }> };
      } | null;
      const openDraft = draftsJson?.data?.drafts?.find((d) =>
        ['DRAFT', 'PENDING_APPROVAL', 'APPROVED'].includes(d.status),
      );
      if (openDraft) {
        await initDraftDetailRes;
      }
      await expect(draftCard.locator('button.btn-new-draft')).toBeEnabled({ timeout: 15_000 });

      // Ensure an editable DRAFT is open so `rateDraftEdit` (`button.btn-save-lines`, RatesDraftCard.tsx:382) is rendered.
      // If none is editable yet, create one via `button.btn-new-draft` (`POST /api/rate-drafts`, RatesDraftCard.tsx:157)
      // and tag its first AUTO line reason with `runTag` via `PUT /api/rate-drafts/{id}` (RatesDraftCard.tsx:278).
      const saveLinesBtn = draftCard.locator('button.btn-save-lines');
      if (openDraft?.status === 'DRAFT') {
        await expect(saveLinesBtn).toBeVisible({ timeout: 15_000 });
        await expect(saveLinesBtn).toBeEnabled({ timeout: 15_000 });
      }
      // An open DRAFT can have zero lines (created while the published set was empty), so make a fresh one.
      if (
        !(await saveLinesBtn.isVisible().catch(() => false)) ||
        (await draftCard.locator('tr[data-draft-line]').count()) === 0
      ) {
        allowDraftSetupWrites = true;
        try {
          // `handleNewDraft` (`RatesDraftCard.tsx:139-176`) calls `POST /api/rate-references/fetch`,
          // `POST /api/rate-drafts`, `GET /api/v3/views/rate_draft` (:160), then `readDrafts` (`Rates.tsx:137-160`)
          // which calls `GET /api/v3/views/rate_drafts` (:139) and `GET /api/v3/views/rate_draft` a second time (:149)
          // before `guarded` clears `busy()` (`Rates.tsx:89-96`). Wait until all views answer and `btn-save-lines`
          // is enabled before touching the draft lines (#4334).
          let sawNewDraftsList = false;
          const newDraftDetailFirst = page.waitForResponse(
            (r) =>
              !sawNewDraftsList &&
              r.request().method() === 'GET' &&
              new URL(r.url()).pathname === '/api/v3/views/rate_draft',
            { timeout: 15_000 },
          );
          const newDraftsList = page.waitForResponse(
            (r) => {
              if (r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/rate_drafts') {
                sawNewDraftsList = true;
                return true;
              }
              return false;
            },
            { timeout: 15_000 },
          );
          const newDraftDetailSecond = page.waitForResponse(
            (r) =>
              sawNewDraftsList &&
              r.request().method() === 'GET' &&
              new URL(r.url()).pathname === '/api/v3/views/rate_draft',
            { timeout: 15_000 },
          );

          const created = await posted(page, /^\/api\/rate-drafts$/, () =>
            draftCard.locator('button.btn-new-draft').click(),
          );
          expect(created.ok, `POST /api/rate-drafts (${lang}): ${created.text}`).toBeTruthy();
          await Promise.all([newDraftDetailFirst, newDraftsList, newDraftDetailSecond]);
          await expect(saveLinesBtn).toBeVisible({ timeout: 15_000 });
          await expect(saveLinesBtn).toBeEnabled({ timeout: 15_000 });

          const firstLine = draftCard.locator('tr[data-draft-line]').first();
          await expect(
            firstLine,
            `new draft has no lines (${lang}): the published rate set for the branch is empty`,
          ).toBeVisible({ timeout: 15_000 });
          const setupReasonInput = firstLine.locator('input[name="reason"]');
          await setupReasonInput.fill(runTag);
          await expect(setupReasonInput).toHaveValue(runTag);
          await expect(saveLinesBtn).toBeEnabled({ timeout: 15_000 });
          const tagged = await putted(page, /^\/api\/rate-drafts\/[^/]+$/, () => saveLinesBtn.click());
          expect(tagged.ok, `PUT /api/rate-drafts/{id} marker (${lang}): ${tagged.text}`).toBeTruthy();
          await expect(saveLinesBtn).toBeEnabled({ timeout: 15_000 });
        } finally {
          allowDraftSetupWrites = false;
        }
      }

      // ── 1a. `rateDraftEdit` (`PUT /api/rate-drafts/{id}`, RatesDraftCard.tsx:247-299) — 4 invalid inputs ──
      const firstRow = draftCard.locator('tr[data-draft-line]').first();
      await expect(firstRow).toBeVisible({ timeout: 15_000 });
      const buyModeSelect = firstRow.locator('select[name="buyMode"]');
      await buyModeSelect.selectOption('MANUAL');
      const manualBuyInput = firstRow.locator('input[name="manualBuy"]');
      const reasonInput = firstRow.locator('input[name="reason"]');
      await expect(manualBuyInput).toBeVisible({ timeout: 10_000 });

      const draftMsg = RATE_DRAFT_MESSAGES[lang];
      for (const badRate of ['', '0', '-5', '32.123456789']) {
        await manualBuyInput.fill(badRate);
        await reasonInput.fill(runTag);
        await expect(manualBuyInput).toHaveValue(badRate);
        await expect(reasonInput).toHaveValue(runTag);
        await expect(saveLinesBtn).toBeEnabled();
        await saveLinesBtn.click();

        const rateErr = firstRow.locator('[data-field-error="manualBuy"]');
        await expect(rateErr, `rateDraftEdit manualBuy error (${lang}, input="${badRate}")`).toHaveText(
          draftMsg.manualValueRequired,
        );
        await expect(manualBuyInput, 'invalid manualBuy input is kept').toHaveValue(badRate);
        await expect(reasonInput, 'runTag reason input is kept').toHaveValue(runTag);
        expect(unexpectedWrites, 'no write sent on invalid manualBuy').toEqual([]);
      }

      // Invalid case 5 on `rateDraftEdit`: valid manualBuy ('32.50') with empty reason ('')
      await manualBuyInput.fill('32.50');
      await reasonInput.fill('');
      await expect(manualBuyInput).toHaveValue('32.50');
      await expect(reasonInput).toHaveValue('');
      await expect(saveLinesBtn).toBeEnabled();
      await saveLinesBtn.click();
      const reasonErr = firstRow.locator('[data-field-error="reason"]');
      await expect(reasonErr, `rateDraftEdit empty reason error (${lang})`).toHaveText(draftMsg.reasonRequired);
      await expect(manualBuyInput, 'valid manualBuy is kept when reason is empty').toHaveValue('32.50');
      await expect(reasonInput, 'empty reason input is kept').toHaveValue('');
      expect(unexpectedWrites, 'no write sent on empty manual reason').toEqual([]);

      // Restore line mode to AUTO locally so we leave draft inputs clean before testing policy.
      await buyModeSelect.selectOption('AUTO');

      // ── 1b. `ratePolicySave` (`POST /api/rate-policies`, Rates.tsx:54, RatesPolicyCard.tsx:266-389) — 4 invalid inputs ──
      const savePolicyBtn = policyCard.getByTestId('save-policy');
      const maxAgeInput = policyCard.locator('input[name="maxReferenceAgeHours"]');
      const onStaleSelect = policyCard.locator('select[name="onStale"]');
      const origMaxAge = await maxAgeInput.inputValue();
      const origOnStale = await onStaleSelect.inputValue();
      const policyMsg = RATE_POLICY_MESSAGES[lang];

      // Policy invalid 1 & 2: maxReferenceAgeHours = '0' and '-2' (RatesPolicyCard.tsx:167-170, :434)
      for (const badAge of ['0', '-2']) {
        await maxAgeInput.fill(badAge);
        await savePolicyBtn.click();
        const ageAlert = policyCard.locator('[role="alert"][data-field-error="maxReferenceAgeHours"]');
        await expect(ageAlert, `ratePolicySave maxReferenceAgeHours error (${lang}, "${badAge}")`).toHaveText(
          policyMsg.maxAgeInvalid,
        );
        await expect(maxAgeInput, 'invalid maxReferenceAgeHours input is kept').toHaveValue(badAge);
        expect(unexpectedWrites, 'no POST /api/rate-policies on invalid maxReferenceAgeHours').toEqual([]);
      }
      await maxAgeInput.fill(origMaxAge || '24');

      // Policy invalid 3: onStale = 'LIMIT' with staleLimitThb = '0' (RatesPolicyCard.tsx:171-174, :441)
      await onStaleSelect.selectOption('LIMIT');
      const staleLimitInput = policyCard.locator('input[name="staleLimitThb"]');
      await expect(staleLimitInput).toBeVisible({ timeout: 10_000 });
      await staleLimitInput.fill('0');
      await savePolicyBtn.click();
      const staleAlert = policyCard.locator('[role="alert"][data-field-error="staleLimitThb"]');
      await expect(staleAlert, `ratePolicySave staleLimitThb error (${lang})`).toHaveText(policyMsg.staleLimitRequired);
      await expect(staleLimitInput, 'invalid staleLimitThb input is kept').toHaveValue('0');
      expect(unexpectedWrites, 'no POST /api/rate-policies on invalid staleLimitThb').toEqual([]);
      await onStaleSelect.selectOption(origOnStale || 'WARN');

      // Policy invalid 4: per-currency guard min > max ('5' > '1', RatesPolicyCard.tsx:138-165, :427)
      const firstGuard = policyCard.locator('tr[data-guard-line]').first();
      await expect(firstGuard).toBeVisible({ timeout: 10_000 });
      const guardBuyMin = firstGuard.locator('input[name="guardBuyMin"]');
      const guardBuyMax = firstGuard.locator('input[name="guardBuyMax"]');
      const origBuyMin = await guardBuyMin.inputValue();
      const origBuyMax = await guardBuyMax.inputValue();

      await guardBuyMin.fill('5');
      await guardBuyMax.fill('1');
      await savePolicyBtn.click();
      const guardAlert = policyCard.locator('[role="alert"][data-field-error="guards"]');
      await expect(guardAlert, `ratePolicySave guard min>max error (${lang})`).toHaveText(policyMsg.guardInvalid);
      await expect(guardBuyMin, 'guardBuyMin input is kept').toHaveValue('5');
      await expect(guardBuyMax, 'guardBuyMax input is kept').toHaveValue('1');
      expect(unexpectedWrites, 'no POST /api/rate-policies on invalid guard bounds').toEqual([]);

      await guardBuyMin.fill(origBuyMin);
      await guardBuyMax.fill(origBuyMax);
      await shot(page, 'flow-validation-admin', `1-rates-refused-${lang}`);
    }

    expect(
      forbiddenDraftSubmissions,
      'must NEVER call POST /api/rate-drafts/{id}/submit or /publish (F4, E-4)',
    ).toEqual([]);
    expect(unexpectedWrites, 'no invalid draft edit or policy save request was sent').toEqual([]);
    expect(log.errors, 'console errors on /#rates').toEqual([]);
  });

  test('2. /#approvals: approvalDecide refuses empty and whitespace rejection reasons in th and en', async ({
    browser,
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json');
    const runTag = `qa-r9-validation-admin-${Date.now()}`;
    const log = watchConsole(page);

    await signIn(page, 'admin', '/');
    const perms = await mePermissions(page);
    test.skip(!perms.includes('ApprovalsDecide'), `admin lacks ApprovalsDecide (${perms.join(', ')})`);

    const { createdReqId } = await ensurePendingApprovalRequest(browser, page, runTag);

    const decideRequests: string[] = [];
    let allowCleanupDecide = false;
    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      // POST /api/approvals/{id}/decide (web/src/screens/admin/parts/decide.ts:21)
      if (!allowCleanupDecide && req.method() === 'POST' && /^\/api\/approvals\/[^/]+\/decide$/.test(path)) {
        decideRequests.push(`${req.method()} ${path}`);
      }
    });

    try {
      for (const lang of STAFF_LANG_LIST) {
        await withLang(page, lang);
        await page.goto('/#approvals');

        // Screen root `.admin-approvals-screen` (web/src/screens/admin/approvals/Approvals.tsx:319).
        const screen = page.locator('.admin-approvals-screen');
        await expect(screen).toBeVisible({ timeout: 20_000 });
        await expect(screen.locator('.approvals-queue, .empty').first()).toBeVisible({ timeout: 20_000 });

        const cards = screen.locator('[data-request]');
        const count = await cards.count();
        test.skip(count === 0, 'no pending approval request available on /#approvals');

        const targetCard = createdReqId ? screen.locator(`[data-request="${createdReqId}"]`).first() : cards.first();
        await expect(targetCard).toBeVisible({ timeout: 15_000 });

        const form = targetCard.locator('.approval-decision-form');
        const reasonInput = form.locator('input[type="text"]');
        const rejectBtn = form.locator('button.danger');
        const expectedMsg = APPROVAL_DECIDE_MESSAGES[lang];

        // 3 invalid inputs on `approvalDecide` (Approvals.tsx:120-128, decide.ts:25-33): empty and whitespace reasons
        for (const badReason of ['', ' ', '   ']) {
          await reasonInput.fill(badReason);
          await rejectBtn.click();

          const alert = form.locator('[role="alert"].error');
          await expect(alert, `approvalDecide empty reason alert (${lang}, ${JSON.stringify(badReason)})`).toHaveText(
            expectedMsg,
          );
          await expect(reasonInput, 'typed reason input is kept').toHaveValue(badReason);
          await expect(targetCard, 'pending request card remains in queue').toBeVisible();
          expect(decideRequests, 'no POST /api/approvals/{id}/decide sent on empty reason').toEqual([]);
        }

        await shot(page, 'flow-validation-admin', `2-approvals-refused-${lang}`);
      }
    } finally {
      // If we created a temporary CASH_TRANSFER request solely for this validation check,
      // reject it with `runTag` so the PENDING queue is restored and no safe balance changes.
      if (createdReqId) {
        allowCleanupDecide = true;
        await page.goto('/#approvals').catch(() => {});
        const cleanupCard = page.locator(`[data-request="${createdReqId}"]`).first();
        if (await cleanupCard.isVisible().catch(() => false)) {
          await cleanupCard.locator('.approval-decision-form input[type="text"]').fill(runTag);
          await posted(page, /^\/api\/approvals\/[^/]+\/decide$/, () =>
            cleanupCard.locator('.approval-decision-form button.danger').click(),
          ).catch(() => {});
        }
      }
    }

    expect(decideRequests, 'no POST /api/approvals/{id}/decide sent during invalid attempts').toEqual([]);
    expect(log.errors, 'console errors on /#approvals').toEqual([]);
  });

  test('3. /operations/verification: verificationCaseReject and verificationCaseRequestUpdate refuse empty/invalid reasons in th and en', async ({
    browser,
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json');
    const runTag = `qa-r9-validation-admin-${Date.now()}`;
    const log = watchConsole(page);

    await signIn(page, 'admin', '/operations/verification');
    const perms = await mePermissions(page);
    test.skip(
      !perms.includes('VerificationQueueRead') || !perms.includes('VerificationDecide'),
      `admin lacks VerificationQueueRead or VerificationDecide (${perms.join(', ')})`,
    );

    const { targetCaseId, createdForTest } = await ensureDecidableVerificationCase(browser, page);
    test.skip(!targetCaseId, 'no decidable verification case in queue and could not open one');

    const decisionWrites: string[] = [];
    let allowCleanupWrite = false;
    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      // - POST /api/operations/verification/cases/{id}/reject (VerificationScreen.tsx:432)
      // - POST /api/operations/verification/cases/{id}/request-update (VerificationScreen.tsx:441)
      if (
        !allowCleanupWrite &&
        req.method() === 'POST' &&
        /^\/api\/operations\/verification\/cases\/[^/]+\/(reject|request-update)$/.test(path)
      ) {
        decisionWrites.push(`${req.method()} ${path}`);
      }
    });

    try {
      for (const lang of STAFF_LANG_LIST) {
        await withLang(page, lang);
        await page.goto('/operations/verification');

        // Screen root `[data-testid="verification-screen"]` (VerificationScreen.tsx:685).
        const screen = page.getByTestId('verification-screen');
        await expect(screen).toBeVisible({ timeout: 20_000 });

        const row = page.locator(`tr[data-case-row][data-case-id="${targetCaseId}"]`);
        await expect(row, `verification case row ${targetCaseId}`).toBeVisible({ timeout: 20_000 });
        await row.click();

        const detail = page.getByTestId('case-detail-view');
        await expect(detail).toBeVisible({ timeout: 15_000 });
        const statusBefore = (await page.getByTestId('case-status').innerText()).trim();
        const expectedMsg = VERIFICATION_REASON_MESSAGES[lang];

        // ── 3a. `verificationCaseReject` (`POST /api/operations/verification/cases/{id}/reject`, VerificationScreen.tsx:432) ──
        await page.locator('button[data-action="reject"]').click();
        const rejectPanel = page.getByTestId('reason-decision-panel');
        await expect(rejectPanel).toBeVisible({ timeout: 15_000 });
        const reasonSelect = rejectPanel.locator('select[name="reasonCode"]');
        const confirmRejectBtn = rejectPanel.locator('button[data-action="confirm-reject"]');

        for (const badReason of ['', '   ', runTag]) {
          await forceVerificationReasonValue(page, badReason);
          await confirmRejectBtn.click();

          const errBanner = screen.locator('[data-error][role="alert"]');
          await expect(errBanner, `verificationCaseReject reason error (${lang}, "${badReason}")`).toContainText(
            expectedMsg,
          );
          await expect(reasonSelect, 'reason select keeps its value').toHaveValue(badReason);
          await expect(page.getByTestId('case-status'), 'case status unchanged').toHaveText(statusBefore);
          expect(decisionWrites, 'no POST .../reject sent on invalid reason').toEqual([]);
        }

        // ── 3b. `verificationCaseRequestUpdate` (`POST /api/operations/verification/cases/{id}/request-update`, VerificationScreen.tsx:441) ──
        await page.locator('button[data-action="request-update"]').click();
        const reqUpdatePanel = page.getByTestId('reason-decision-panel');
        await expect(reqUpdatePanel).toBeVisible({ timeout: 15_000 });
        const confirmReqUpdateBtn = reqUpdatePanel.locator('button[data-action="confirm-request-update"]');

        for (const badReason of ['', '   ', runTag]) {
          await forceVerificationReasonValue(page, badReason);
          await confirmReqUpdateBtn.click();

          const errBanner = screen.locator('[data-error][role="alert"]');
          await expect(
            errBanner,
            `verificationCaseRequestUpdate reason error (${lang}, "${badReason}")`,
          ).toContainText(expectedMsg);
          await expect(reasonSelect, 'reason select keeps its value').toHaveValue(badReason);
          await expect(page.getByTestId('case-status'), 'case status unchanged').toHaveText(statusBefore);
          expect(decisionWrites, 'no POST .../request-update sent on invalid reason').toEqual([]);
        }

        await shot(page, 'flow-validation-admin', `3-verification-refused-${lang}`);
      }
    } finally {
      // If we opened/submitted a customer verification case solely for this validation test,
      // reject it cleanly so the reviewer queue is restored.
      if (createdForTest && targetCaseId) {
        allowCleanupWrite = true;
        await page.goto('/operations/verification').catch(() => {});
        const row = page.locator(`tr[data-case-row][data-case-id="${targetCaseId}"]`);
        if (await row.isVisible().catch(() => false)) {
          await row.click().catch(() => {});
          const status = (await page.getByTestId('case-status').innerText().catch(() => '')).trim();
          if (status === 'SUBMITTED') {
            await posted(
              page,
              new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(targetCaseId)}/start-review$`),
              () => page.locator('button[data-action="start-review"]').click(),
            ).catch(() => {});
          }
          await page.locator('button[data-action="reject"]').click().catch(() => {});
          await page
            .locator('[data-testid="reason-decision-panel"] select[name="reasonCode"]')
            .selectOption('OTHER')
            .catch(() => {});
          await posted(
            page,
            new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(targetCaseId)}/reject$`),
            () => page.locator('button[data-action="confirm-reject"]').click(),
          ).catch(() => {});
        }
      }
    }

    expect(decisionWrites, 'no verification reject or request-update write sent during invalid attempts').toEqual([]);
    expect(log.errors, 'console errors on /operations/verification').toEqual([]);
  });

  test('4. /#booking-slots: slotCapacitySet and closureAdd refuse invalid inputs in th and en', async ({ page }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json');
    const runTag = `qa-r9-validation-admin-${Date.now()}`;
    const log = watchConsole(page);

    await signIn(page, 'admin', '/');
    const perms = await mePermissions(page);
    test.skip(!perms.includes('BranchesManage'), `admin lacks BranchesManage (${perms.join(', ')})`);

    const slotAndClosureWrites: string[] = [];
    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      const method = req.method();
      // - PUT /api/booking-slots/{id}/capacity (web/src/screens/admin/booking-slots/BookingSlots.tsx:276)
      // - POST /api/branches/{id}/closures (web/src/screens/admin/booking-slots/BookingSlots.tsx:219)
      if (
        (method === 'PUT' && /^\/api\/booking-slots\/[^/]+\/capacity$/.test(path)) ||
        (method === 'POST' && /^\/api\/branches\/[^/]+\/closures$/.test(path))
      ) {
        slotAndClosureWrites.push(`${method} ${path}`);
      }
    });

    await page.goto('/#booking-slots');
    // Screen root `.booking-slots-screen` (web/src/screens/admin/booking-slots/BookingSlots.tsx:383).
    const screen = page.locator('.booking-slots-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });

    // Ensure session branch is on `XC-B` where booking slots are seeded (flow-booking.spec.ts:120-139).
    const branchButton = page.locator('.account-bar button[aria-haspopup="menu"]');
    const startBranch = ((await branchButton.innerText().catch(() => '')) || '').trim();
    if (!/XC-B/.test(startBranch) && (await branchButton.isVisible().catch(() => false))) {
      await branchButton.click();
      const option = page.getByRole('menuitemradio', { name: /^XC-B$/ });
      if (await option.isVisible().catch(() => false)) {
        const switched = page
          .waitForResponse(
            (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/auth/session/branch',
            { timeout: 15_000 },
          )
          .catch(() => null);
        await option.click();
        await switched;
      }
    }

    try {
      for (const lang of STAFF_LANG_LIST) {
        await withLang(page, lang);
        await page.goto('/#booking-slots');
        await expect(screen).toBeVisible({ timeout: 20_000 });

        const msg = BOOKING_SLOTS_MESSAGES[lang];
        const errorBanner = screen.locator('[role="alert"].error.field-error');

        // ── 4a. `slotCapacitySet` (`PUT /api/booking-slots/{id}/capacity`, BookingSlots.tsx:259-289) — 4 invalid inputs ──
        const firstSlot = screen.locator('.slot-item').first();
        await expect(firstSlot, 'at least one seeded .slot-item on /#booking-slots').toBeVisible({ timeout: 20_000 });
        const slotCapInput = firstSlot.locator('input[name="slotCapacity"]');
        const slotCapBtn = firstSlot.locator('.slot-capacity-control button');
        const origSlotCap = await slotCapInput.inputValue();

        // 0, negative ('-3'), non-integer ('2.5'), and empty/non-number ('')
        for (const badCap of ['0', '-3', '2.5', '']) {
          await slotCapInput.fill(badCap);
          await slotCapBtn.click();

          await expect(
            errorBanner,
            `slotCapacitySet error (${lang}, input=${JSON.stringify(badCap)})`,
          ).toHaveText(msg.slotCapacityValidationFailed);
          await expect(slotCapInput, 'invalid slotCapacity input is kept').toHaveValue(badCap);
          expect(slotAndClosureWrites, 'no PUT /api/booking-slots/{id}/capacity sent').toEqual([]);
        }
        await slotCapInput.fill(origSlotCap);

        // ── 4b. `closureAdd` (`POST /api/branches/{id}/closures`, BookingSlots.tsx:201-236) — 4 invalid inputs ──
        const closureForm = screen.locator('.closures-form');
        const closedOnInput = closureForm.locator('input[name="closedOn"]');
        const reasonInput = closureForm.locator('input[name="reason"]');
        const addClosureBtn = closureForm.locator('button');
        const closuresBefore = await screen.locator('.closure-item').count();
        const futureDate = futureDateBangkok(14);

        const invalidClosureCases: Array<{ closedOn: string; reason: string; desc: string }> = [
          { closedOn: '2020-01-01', reason: '', desc: 'past date with empty reason' },
          { closedOn: futureDate, reason: '', desc: 'future date with empty reason' },
          { closedOn: futureDate, reason: '   ', desc: 'future date with whitespace reason' },
          { closedOn: '', reason: runTag, desc: 'empty date with non-empty reason' },
        ];

        for (const c of invalidClosureCases) {
          await closedOnInput.fill(c.closedOn);
          await reasonInput.fill(c.reason);
          await addClosureBtn.click();

          await expect(errorBanner, `closureAdd error (${lang}, ${c.desc})`).toHaveText(msg.closureValidationFailed);
          await expect(closedOnInput, `closedOn kept (${c.desc})`).toHaveValue(c.closedOn);
          await expect(reasonInput, `reason kept (${c.desc})`).toHaveValue(c.reason);
          await expect(screen.locator('.closure-item'), 'no closure created').toHaveCount(closuresBefore);
          expect(slotAndClosureWrites, 'no POST /api/branches/{id}/closures sent').toEqual([]);
        }

        await shot(page, 'flow-validation-admin', `4-booking-slots-refused-${lang}`);
      }
    } finally {
      const original = startBranch.replace(/^สาขา\s*|^Branch\s*/i, '').trim();
      if (original && !/XC-B/.test(startBranch)) {
        await page.locator('.account-bar button[aria-haspopup="menu"]').click().catch(() => {});
        await page.getByRole('menuitemradio', { name: original }).click().catch(() => {});
      }
    }

    expect(slotAndClosureWrites, 'no slot capacity or closure write sent during invalid attempts').toEqual([]);
    expect(log.errors, 'console errors on /#booking-slots').toEqual([]);
  });
});
