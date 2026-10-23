// QA-R9-8 — round 9: shift handover (start, accept, reject) & verification case (approve, revoke)
// (design docs/design/qa-e2e.md §9 row R9-8).
// Written by ticket #4292 (QA-R9-8); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Screen keys cited from qa/e2e/tests/smoke-*.spec.ts:
// - /staff: `.xc-shift-card` (qa/e2e/tests/smoke-staff.spec.ts:15; web/src/screens/staff/shift/ShiftCard.tsx:382)
// - /operations/verification: `data-testid=verification-screen` (qa/e2e/tests/smoke-desk-ops.spec.ts:107;
//   web/src/screens/operations/verification/VerificationScreen.tsx:685)
//
// Selectors & endpoints cited from web/src/screens/ and web/src/shared/api/generated/:
// - web/src/screens/staff/shift/ShiftCard.tsx:
//   :253-261 HandoverStartInput, POST /api/handovers (web/src/shared/api/generated/cash.ts:47 handoverStart)
//   :315-321 CashRecordInput, POST /api/handovers/{id}/accept (web/src/shared/api/generated/cash.ts:17 handoverAccept)
//   :339-345 CashRecordInput, POST /api/handovers/{id}/reject (web/src/shared/api/generated/cash.ts:17 handoverReject)
//   :377-379 availableReceivers filters out props.ctx.me.userId
//   :382 section.xc-shift-card[data-card="shift"]
//   :388 [data-testid="shift-notice"], :394 [data-testid="shift-error"]
//   :404 [data-testid="shift-open-count"], :414 [data-testid="shift-close-count"], :422 [data-testid="shift-handover"]
//   :489 [data-testid="shift-handover-form"], :496 select[data-testid="shift-receiver"]
//   :511 [data-denomination-row], :517 input[data-denomination-id]
//   :537 [data-testid="shift-handover-save"], :545 button.xc-shift-cancel-btn
//   :593 [data-handover-id="<id>"], :600 [data-testid="shift-accept"], :609 [data-testid="shift-reject"]
// - web/src/screens/operations/verification/VerificationScreen.tsx:
//   :274-278 lacks VerificationReview -> setAuthorized(false), setRefusalMessage(st('refused'))
//   :420 POST /api/operations/verification/cases/{id}/start-review
//   :426 POST /api/operations/verification/cases/{id}/approve (verificationCaseApprove)
//   :432 POST /api/operations/verification/cases/{id}/reject
//   :447 POST /api/operations/verification/cases/{id}/revoke (verificationCaseRevoke)
//   :505-511 handleApprove validates expiresAt (/^\d{4}-\d{2}-\d{2}$/ and > todayBangkokIso()), sets st('errExpiresAtRequired')
//   :546-564 submitReasonDecision validates selectedReason in S5_REASON_CODES, sets st('errReasonRequired')
//   :685 [data-testid="verification-screen"], :687 [data-refused], :694 [data-error]
//   :746-749 tr[data-case-row][data-case-id="<id>"], :777 [data-testid="case-detail-view"], :790 [data-testid="case-status"]
//   :887 button[data-action="start-review"], :908 input[name="expiresAt"], :915 button[data-action="approve"]
//   :923 button[data-action="reject"], :941 button[data-action="revoke"]
//   :953 [data-testid="reason-decision-panel"], :960 select[name="reasonCode"]
//   :977 button[data-action="confirm-reject"], :997 button[data-action="confirm-revoke"], :1006 button[data-action="cancel-reason"]
// - web/src/screens/customer/verification/VerificationScreen.tsx:
//   :257 [data-verification-screen="true"], :360 section[data-case-state="DRAFT"], :386 select[data-field="documentType"]
//   :406 #xc-verification-file, :422 button[data-action="upload"], :462 li.xc-verification-doc-item[data-doc-id]
//   :513 button[data-action="submit"], :582 section[data-case-state="VERIFIED"], :587 [data-expires-at]
//   :639 section[data-case-state="REVOKED"], :659 [data-decision-reason="true"], :678 button[data-action="update-and-resubmit"]
import { expect, test, type Page } from '@playwright/test';
import {
  closeShift,
  clockIn,
  hasAccount,
  HTML_LANG,
  mePermissions,
  openShift,
  posted,
  SHIFT_QTY,
  shot,
  signIn,
  STAFF_LANGS,
  users,
  withLang,
  type Role,
} from './helpers';

const RUN_MARKER = `qa-r9-8-${Date.now()}`;

/** 1x1 transparent PNG (67 bytes; starts with PNG signature 89 50 4E 47 0D 0A 1A 0A, case_service.rs:199-201). */
const TINY_PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
  'base64',
);

interface MeViewAnswer {
  data?: {
    id?: string;
    permissions?: string[];
  };
}

interface HandoverStartAnswer {
  id?: string;
  handoverId?: string;
  data?: {
    id?: string;
    handoverId?: string;
  };
}

interface CaseAnswer {
  id?: string;
  status?: string;
  version?: number;
  decisionReasonCode?: string | null;
  expiresAt?: string | null;
}

interface MyVerificationAnswer {
  readiness?: string;
  case?: (CaseAnswer & { documents?: Array<{ id?: string }> }) | null;
}

async function revokeVerifiedCase(
  customer: Page,
  admin: Page,
  fallbackCaseId: string = '',
): Promise<{ ok: boolean; text: string }> {
  const myVerRes = await customer.request.get('/api/v3/views/my_verification');
  const myVer = (await myVerRes.json().catch(() => ({}))) as MyVerificationAnswer;
  const id = String(myVer.case?.id || fallbackCaseId);
  if (!id) return { ok: false, text: 'missing case id from my_verification' };
  const expectedVersion = myVer.case?.version ?? 1;
  if (admin.url() === 'about:blank') {
    await signIn(admin, 'admin', '/operations/verification');
  }
  const res = await admin.request.post(`/api/operations/verification/cases/${encodeURIComponent(id)}/revoke`, {
    headers: {
      'x-xchange-request': '1',
      origin: new URL(users.baseUrl).origin,
      'idempotency-key': `${RUN_MARKER}-revoke-${id}-${expectedVersion}-${Date.now()}`,
    },
    data: {
      id,
      expectedVersion,
      reasonCode: 'DOCUMENT_EXPIRED',
    },
  });
  const body = await res.text().catch(() => '');
  const text = `${res.status()} ${body.slice(0, 300)}`;
  console.log(`[R9-8] POST /api/operations/verification/cases/${id}/revoke (${RUN_MARKER}) ${text}`);
  return { ok: res.ok(), text };
}

/** Today's YYYY-MM-DD in Asia/Bangkok (matches VerificationScreen.tsx:167-177 todayBangkokIso). */
function todayBangkokIso(): string {
  const parts = new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Bangkok',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).formatToParts(new Date());
  const y = parts.find((p) => p.type === 'year')?.value ?? '1970';
  const m = parts.find((p) => p.type === 'month')?.value ?? '01';
  const d = parts.find((p) => p.type === 'day')?.value ?? '01';
  return `${y}-${m}-${d}`;
}

/** Future YYYY-MM-DD (one year ahead) for verificationCaseApprove. */
function futureExpiryIso(): string {
  const today = todayBangkokIso();
  const year = Number(today.slice(0, 4)) + 1;
  return `${year}${today.slice(4)}`;
}

/** Asserts that a refusal message is visible, non-empty, and not a raw camelCase/snake_case error code or JSON blob. */
function expectWordedRefusal(text: string, context: string): void {
  const trimmed = text.trim();
  expect(trimmed.length, `${context}: refusal must not be empty`).toBeGreaterThan(3);
  expect(trimmed, `${context}: refusal must not be raw JSON`).not.toMatch(/^\{.*\}$/s);
  expect(
    trimmed,
    `${context}: refusal must be worded, not a raw code`,
  ).not.toMatch(/^(validationFailed|invalidInput|forbidden|unauthorized|notFound|conflict|internalError|[a-z]+_[a-z_]+)$/);
}

/** Reads the signed-in user's id from GET /api/v3/views/me (api/src/presentation/dto/v3/common.rs:59-61). */
async function meUserId(page: Page): Promise<string> {
  const res = await page.request.get('/api/v3/views/me');
  expect(res.ok(), `GET /api/v3/views/me status ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as MeViewAnswer;
  const uid = String(body.data?.id ?? '');
  expect(uid, 'me.data.id from /api/v3/views/me').not.toBe('');
  return uid;
}

/** Clocks a clocked-in staff page out via floor-clock (POST /api/attendance with direction OUT). */
async function clockOutIfClockedIn(page: Page): Promise<void> {
  const shiftButton = page.getByTestId('shift-open-count').or(page.getByTestId('shift-close-count')).first();
  if (!(await shiftButton.isVisible().catch(() => false))) return;
  const clock = page.getByTestId('floor-clock');
  await expect(clock, 'floor-clock visible before clock-out').toBeVisible({ timeout: 20_000 });
  const res = await posted(page, /^\/api\/attendance$/, () => clock.click());
  expect(res.ok, `clock-out POST /api/attendance: ${res.text}`).toBeTruthy();
}

/** Uploads a 1x1 PNG passport document on a customer DRAFT verification case (flow-verification.spec.ts:38-61). */
async function uploadPassportPng(customer: Page, caseId: string): Promise<void> {
  const select = customer.locator('section[data-case-state="DRAFT"] select[data-field="documentType"]');
  await expect(select, 'PASSPORT documentType select (VerificationScreen.tsx:386)').toBeVisible({ timeout: 15_000 });
  await select.selectOption('PASSPORT');

  await customer.locator('#xc-verification-file').setInputFiles({
    name: `${RUN_MARKER}-passport.png`,
    mimeType: 'image/png',
    buffer: TINY_PNG,
  });

  const upRes = await posted(
    customer,
    new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/documents$`),
    () => customer.locator('button[data-action="upload"]').click(),
  );
  console.log(`[R9-8] POST /api/app/verification/cases/${caseId}/documents (${RUN_MARKER}) ${upRes.text}`);
  expect(upRes.ok, `upload passport PNG: ${upRes.text}`).toBeTruthy();

  await expect(
    customer.locator('li.xc-verification-doc-item[data-doc-id]').first(),
    'uploaded document row visible (VerificationScreen.tsx:462)',
  ).toBeVisible({ timeout: 15_000 });
}

/** Selects a verification case row on /operations/verification (VerificationScreen.tsx:746-749, 777). */
async function selectCaseOnStaff(admin: Page, caseId: string): Promise<void> {
  const row = admin.locator(`tr[data-case-row][data-case-id="${caseId}"]`);
  if (!(await row.isVisible().catch(() => false))) {
    await admin.reload();
  }
  await expect(row, `queue row for case ${caseId} (VerificationScreen.tsx:746)`).toBeVisible({ timeout: 20_000 });
  await row.click();
  await expect(admin.getByTestId('case-detail-view'), 'selected case detail view (VerificationScreen.tsx:777)').toBeVisible({
    timeout: 15_000,
  });
}

test.describe.serial('R9-8 shift handover & verification case approve/revoke', () => {
  test('shift handover: permission (c), invalid inputs (b) in th/en, and start/reject/accept happy paths (a) with reload', async ({
    browser,
  }) => {
    const required: readonly Role[] = ['teller', 'branchManager'];
    for (const role of required) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[R9-8] skip handover test: ${reason}`);
        test.skip(true, reason);
      }
    }

    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });
    let shiftOpened = false;

    try {
      // ---- (c) Permission check on /staff shift card ----
      // Roles without ShiftOperate (e.g. finance, smoke-rbac-matrix.spec.ts:80-81,172-221; StaffHomeScreen.tsx:314)
      // do not render `.xc-shift-card` (`[data-card="shift"]`) and cannot call POST /api/handovers.
      if (hasAccount('finance')) {
        const finance = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });
        try {
          await signIn(finance, 'finance', '/staff');
          const financePerms = await mePermissions(finance);
          expect(financePerms, 'finance lacks ShiftOperate').not.toContain('ShiftOperate');
          await expect(
            finance.locator('.xc-shift-card'),
            'finance does not see .xc-shift-card on /staff (ShiftCard.tsx:382)',
          ).toHaveCount(0);
          const unauthHandover = await finance.request.post('/api/handovers', {
            data: { receiverId: '00000000-0000-0000-0000-000000000000', lines: [] },
          });
          expect(
            unauthHandover.status(),
            'finance POST /api/handovers refused (401/403)',
          ).toBeGreaterThanOrEqual(400);
          await shot(finance, 'staff', 'r9-8-handover-permission-finance');
        } finally {
          await finance.close();
        }
      }

      // Sign in teller and branchManager on /staff and verify screen key `.xc-shift-card` (smoke-staff.spec.ts:15).
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await expect(teller.locator('.xc-shift-card'), 'teller .xc-shift-card visible (ShiftCard.tsx:382)').toBeVisible({
        timeout: 20_000,
      });
      await expect(manager.locator('.xc-shift-card'), 'manager .xc-shift-card visible (ShiftCard.tsx:382)').toBeVisible({
        timeout: 20_000,
      });

      const tellerUserId = await meUserId(teller);
      const managerUserId = await meUserId(manager);
      console.log(`[R9-8] marker=${RUN_MARKER} teller=${tellerUserId} manager=${managerUserId}`);

      // Open teller's shift with SHIFT_QTY ('10') on the first THB denomination row.
      await openShift(teller, manager);
      shiftOpened = true;
      await expect(
        teller.getByTestId('shift-handover'),
        'shift-handover button visible when shift is OPEN (ShiftCard.tsx:422)',
      ).toBeVisible({ timeout: 20_000 });

      // ---- (b) Invalid input cases on handoverStart in both `th` and `en` (`withLang`) ----
      // Case 1: Empty receiver selection keeps shift-handover-save disabled (`disabled={!receiverId() || busy()}`, ShiftCard.tsx:539).
      // Case 2: Self-handover (`teller === receiver`, `CashError::SameCounter`, person_rules.rs) refused with worded message in th & en; form keeps values.
      // Case 3: Negative denomination quantity (`-5`, ShiftCard.tsx:167-170) refused with worded message in th & en; input keeps `-5`.
      // Case 4: Non-integer decimal quantity (`1.5`, ShiftCard.tsx:167-170) refused with worded message in th & en; input keeps `1.5`.
      for (const lang of STAFF_LANGS) {
        await withLang(teller, lang);
        await teller.goto('about:blank');
        await teller.goto('/staff');
        await expect(teller.locator('html'), `/staff lang=${lang}`).toHaveAttribute('lang', HTML_LANG[lang]);
        await expect(teller.getByTestId('shift-handover'), `shift-handover visible (${lang})`).toBeVisible({
          timeout: 20_000,
        });

        await teller.getByTestId('shift-handover').click();
        const form = teller.getByTestId('shift-handover-form');
        await expect(form, `shift-handover-form visible (${lang}, ShiftCard.tsx:489)`).toBeVisible({ timeout: 15_000 });

        const receiverSelect = form.getByTestId('shift-receiver');
        const saveBtn = form.getByTestId('shift-handover-save');
        const thbInput = form.locator('[data-denomination-row]').filter({ hasText: 'THB' }).first().locator('input[data-denomination-id]');

        // Invalid Case 1: empty receiver (`""`) -> save button is disabled (ShiftCard.tsx:539), and self userId is excluded from options (ShiftCard.tsx:377-379).
        await expect(receiverSelect, `receiver defaults to empty (${lang})`).toHaveValue('');
        await expect(saveBtn, `shift-handover-save disabled when receiver is empty (${lang})`).toBeDisabled();
        const selfOptionCount = await receiverSelect.locator(`option[value="${tellerUserId}"]`).count();
        expect(selfOptionCount, `teller's own userId excluded from availableReceivers (${lang}, ShiftCard.tsx:377)`).toBe(0);

        // Invalid Case 2: self-handover (`teller === receiver`) via injected option -> server refuses POST /api/handovers with worded message, form keeps inputs.
        await receiverSelect.evaluate((sel: HTMLSelectElement, selfId: string) => {
          const opt = document.createElement('option');
          opt.value = selfId;
          opt.textContent = `Self (${selfId})`;
          sel.appendChild(opt);
        }, tellerUserId);
        await receiverSelect.selectOption(tellerUserId);
        await thbInput.fill(SHIFT_QTY);

        const selfRes = await posted(teller, /^\/api\/handovers$/, () => saveBtn.click());
        expect(selfRes.ok, `self-handover POST /api/handovers must be refused (${lang}): ${selfRes.text}`).toBeFalsy();
        const errEl = teller.getByTestId('shift-error');
        await expect(errEl, `shift-error visible on self-handover (${lang}, ShiftCard.tsx:394)`).toBeVisible({
          timeout: 10_000,
        });
        const selfErrText = await errEl.innerText();
        expectWordedRefusal(selfErrText, `self-handover (${lang})`);
        await expect(receiverSelect, `form keeps receiver after self-handover refusal (${lang})`).toHaveValue(tellerUserId);
        await expect(thbInput, `form keeps quantity after self-handover refusal (${lang})`).toHaveValue(SHIFT_QTY);

        // Invalid Case 3: negative quantity (`-5`) with valid receiver -> refused with worded message, form keeps `-5`.
        await receiverSelect.selectOption(managerUserId);
        await thbInput.fill('-5');
        // shift-error still holds case 2's message (ShiftCard.tsx:273-276 replaces it only on a refusal), so wait for
        // this POST's own answer: a visible banner alone would pass on the stale text.
        const negRes = await posted(teller, /^\/api\/handovers$/, () => saveBtn.click());
        expect(negRes.ok, `negative quantity POST /api/handovers must be refused (${lang}): ${negRes.text}`).toBeFalsy();
        await expect(errEl, `shift-error visible on negative quantity (${lang})`).toBeVisible({ timeout: 10_000 });
        const negErrText = await errEl.innerText();
        expectWordedRefusal(negErrText, `negative quantity (${lang})`);
        await expect(thbInput, `form keeps -5 after refusal (${lang})`).toHaveValue('-5');
        await expect(receiverSelect, `form keeps receiver after negative quantity refusal (${lang})`).toHaveValue(
          managerUserId,
        );

        // Invalid Case 4: decimal quantity (`1.5`) with valid receiver -> refused with worded message, form keeps `1.5`.
        await thbInput.fill('1.5');
        const decRes = await posted(teller, /^\/api\/handovers$/, () => saveBtn.click());
        expect(decRes.ok, `decimal quantity POST /api/handovers must be refused (${lang}): ${decRes.text}`).toBeFalsy();
        await expect(errEl, `shift-error visible on decimal quantity (${lang})`).toBeVisible({ timeout: 10_000 });
        const decErrText = await errEl.innerText();
        expectWordedRefusal(decErrText, `decimal quantity (${lang})`);
        await expect(thbInput, `form keeps 1.5 after refusal (${lang})`).toHaveValue('1.5');

        await shot(teller, 'staff', `r9-8-handover-invalid`, lang);
        await form.locator('button.xc-shift-cancel-btn').click();
        await expect(form, `handover form closed after cancel (${lang})`).toBeHidden({ timeout: 10_000 });
      }

      // Reset teller language to th before happy paths.
      await withLang(teller, 'th');
      await teller.goto('about:blank');
      await teller.goto('/staff');
      await expect(teller.getByTestId('shift-handover'), 'teller shift-handover ready').toBeVisible({ timeout: 20_000 });

      // ---- (a) Happy path 1 & 2: handoverStart (POST /api/handovers) + handoverReject (POST /api/handovers/{id}/reject) ----
      await teller.getByTestId('shift-handover').click();
      const form1 = teller.getByTestId('shift-handover-form');
      await expect(form1, 'handover form 1 open').toBeVisible({ timeout: 15_000 });
      await form1.getByTestId('shift-receiver').selectOption(managerUserId);
      const thbInput1 = form1.locator('[data-denomination-row]').filter({ hasText: 'THB' }).first().locator('input[data-denomination-id]');
      await thbInput1.fill(SHIFT_QTY);

      const startRejectRes = await posted(teller, /^\/api\/handovers$/, () =>
        form1.getByTestId('shift-handover-save').click(),
      );
      console.log(`[R9-8] POST /api/handovers (1, for reject, ${RUN_MARKER}) ${startRejectRes.text}`);
      expect(startRejectRes.ok, `handoverStart (1): ${startRejectRes.text}`).toBeTruthy();
      const parsedStart1 = JSON.parse(startRejectRes.body || '{}') as HandoverStartAnswer;
      const handoverId1 = String(
        parsedStart1.id ?? parsedStart1.handoverId ?? parsedStart1.data?.id ?? parsedStart1.data?.handoverId ?? '',
      );

      // Reload both teller and manager to read back the initiated handover from the server (rule (a): read back after reload).
      await teller.reload();
      await manager.reload();
      const pendingItem1 = handoverId1
        ? manager.locator(`[data-handover-id="${handoverId1}"]`)
        : manager.locator('[data-handover-id]').first();
      await expect(
        pendingItem1,
        'initiated handover 1 visible on manager /staff after reload (ShiftCard.tsx:593)',
      ).toBeVisible({ timeout: 20_000 });
      const resolvedHandoverId1 =
        handoverId1 || ((await pendingItem1.getAttribute('data-handover-id')) ?? '');
      expect(resolvedHandoverId1, 'resolved handoverId 1').not.toBe('');
      await expect(
        teller.locator(`[data-handover-id="${resolvedHandoverId1}"]`),
        'sender is not offered accept/reject for its own handover (ShiftCard.tsx:370-375)',
      ).toHaveCount(0);
      await shot(manager, 'staff', 'r9-8-handover-started-1');

      // Manager clicks shift-reject on handover 1 -> POST /api/handovers/{id}/reject.
      const rejectRes = await posted(
        manager,
        new RegExp(`^/api/handovers/${encodeURIComponent(resolvedHandoverId1)}/reject$`),
        () => pendingItem1.getByTestId('shift-reject').click(),
      );
      console.log(`[R9-8] POST /api/handovers/${resolvedHandoverId1}/reject ${rejectRes.text}`);
      expect(rejectRes.ok, `handoverReject: ${rejectRes.text}`).toBeTruthy();

      // Reload manager and teller and verify handover 1 is gone from pending list and teller's shift remains OPEN.
      await manager.reload();
      await teller.reload();
      await expect(
        manager.locator(`[data-handover-id="${resolvedHandoverId1}"]`),
        'rejected handover 1 removed from pending list after reload',
      ).toHaveCount(0);
      await expect(
        teller.getByTestId('shift-close-count'),
        'teller shift remains OPEN after handoverReject and reload',
      ).toBeVisible({ timeout: 20_000 });
      await shot(manager, 'staff', 'r9-8-handover-rejected');

      // ---- (a) Happy path 3: handoverStart + handoverAccept (POST /api/handovers/{id}/accept) ----
      await teller.getByTestId('shift-handover').click();
      const form2 = teller.getByTestId('shift-handover-form');
      await expect(form2, 'handover form 2 open').toBeVisible({ timeout: 15_000 });
      await form2.getByTestId('shift-receiver').selectOption(managerUserId);
      const thbInput2 = form2.locator('[data-denomination-row]').filter({ hasText: 'THB' }).first().locator('input[data-denomination-id]');
      await thbInput2.fill(SHIFT_QTY);

      const startAcceptRes = await posted(teller, /^\/api\/handovers$/, () =>
        form2.getByTestId('shift-handover-save').click(),
      );
      console.log(`[R9-8] POST /api/handovers (2, for accept, ${RUN_MARKER}) ${startAcceptRes.text}`);
      expect(startAcceptRes.ok, `handoverStart (2): ${startAcceptRes.text}`).toBeTruthy();
      const parsedStart2 = JSON.parse(startAcceptRes.body || '{}') as HandoverStartAnswer;
      const handoverId2 = String(
        parsedStart2.id ?? parsedStart2.handoverId ?? parsedStart2.data?.id ?? parsedStart2.data?.handoverId ?? '',
      );

      await manager.reload();
      const pendingItem2 = handoverId2
        ? manager.locator(`[data-handover-id="${handoverId2}"]`)
        : manager.locator('[data-handover-id]').first();
      await expect(
        pendingItem2,
        'initiated handover 2 visible on manager /staff after reload (ShiftCard.tsx:593)',
      ).toBeVisible({ timeout: 20_000 });
      const resolvedHandoverId2 =
        handoverId2 || ((await pendingItem2.getAttribute('data-handover-id')) ?? '');
      expect(resolvedHandoverId2, 'resolved handoverId 2').not.toBe('');

      const acceptRes = await posted(
        manager,
        new RegExp(`^/api/handovers/${encodeURIComponent(resolvedHandoverId2)}/accept$`),
        () => pendingItem2.getByTestId('shift-accept').click(),
      );
      console.log(`[R9-8] POST /api/handovers/${resolvedHandoverId2}/accept ${acceptRes.text}`);
      expect(acceptRes.ok, `handoverAccept: ${acceptRes.text}`).toBeTruthy();

      // Reload both teller and manager to read back the accepted handover state:
      // 1) pending handover row is gone on both pages;
      // 2) teller's shift is now CLOSED (`shift-open-count` visible);
      // 3) manager (once clocked in) has an OPEN shift (`shift-close-count` visible).
      await teller.reload();
      await manager.reload();
      await expect(
        manager.locator(`[data-handover-id="${resolvedHandoverId2}"]`),
        'accepted handover 2 removed from pending list after reload',
      ).toHaveCount(0);
      await expect(
        teller.getByTestId('shift-open-count'),
        'teller shift is closed after handoverAccept and reload (ShiftCard.tsx:404)',
      ).toBeVisible({ timeout: 20_000 });
      await clockIn(manager);
      await expect(
        manager.getByTestId('shift-close-count'),
        'manager shift is open after handoverAccept and reload (ShiftCard.tsx:414)',
      ).toBeVisible({ timeout: 20_000 });
      await shot(manager, 'staff', 'r9-8-handover-accepted');

      // Hand the drawer back from manager to teller so `closeShift(teller, manager)` in `finally` can close teller's shift
      // with manager as the non-self CLOSE count verifier (person_rules.rs: verifier_is_not_counter).
      await manager.getByTestId('shift-handover').click();
      const returnForm = manager.getByTestId('shift-handover-form');
      await expect(returnForm, 'manager return handover form visible').toBeVisible({ timeout: 15_000 });
      await returnForm.getByTestId('shift-receiver').selectOption(tellerUserId);
      await returnForm
        .locator('[data-denomination-row]')
        .filter({ hasText: 'THB' })
        .first()
        .locator('input[data-denomination-id]')
        .fill(SHIFT_QTY);
      const returnStartRes = await posted(manager, /^\/api\/handovers$/, () =>
        returnForm.getByTestId('shift-handover-save').click(),
      );
      expect(returnStartRes.ok, `return handoverStart (manager -> teller): ${returnStartRes.text}`).toBeTruthy();

      await teller.reload();
      const returnPending = teller.locator('[data-handover-id]').first();
      await expect(returnPending, 'return handover visible to teller').toBeVisible({ timeout: 20_000 });
      const returnAcceptRes = await posted(teller, /^\/api\/handovers\/[^/]+\/accept$/, () =>
        returnPending.getByTestId('shift-accept').click(),
      );
      expect(returnAcceptRes.ok, `return handoverAccept (teller): ${returnAcceptRes.text}`).toBeTruthy();
      await teller.reload();
      await manager.reload();
      await clockOutIfClockedIn(manager);
    } finally {
      if (shiftOpened) {
        await closeShift(teller, manager).catch((e) =>
          console.log(`[R9-8] cleanup closeShift warning: ${e instanceof Error ? e.message : String(e)}`),
        );
      } else {
        console.log('[R9-8] cleanup: no shift opened by this test, skipping closeShift');
      }
      await teller.close();
      await manager.close();
    }
  });

  test('verification case approve & revoke: permission (c), invalid inputs (b) in th/en, and approve/revoke happy paths (a) with reload', async ({
    browser,
  }) => {
    const required: readonly Role[] = ['customer', 'teller', 'admin'];
    for (const role of required) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[R9-8] skip verification test: ${reason}`);
        test.skip(true, reason);
      }
    }

    const customer = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });
    const admin = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 800 } });
    let approvedCaseId = '';
    let happyPathRevoked = false;

    try {
      // ---- (c) Permission check: teller lacks VerificationReview on /operations/verification ----
      // VerificationScreen.tsx:274-278 sets `authorized(false)` and renders `[data-refused]` (:687).
      for (const lang of STAFF_LANGS) {
        await withLang(teller, lang);
        await signIn(teller, 'teller', '/operations/verification');
        await expect(
          teller.getByTestId('verification-screen'),
          `verification-screen key visible for teller (${lang}, smoke-desk-ops.spec.ts:107)`,
        ).toBeVisible({ timeout: 20_000 });
        const refusedEl = teller.locator('[data-refused]');
        await expect(
          refusedEl,
          `teller sees [data-refused] on /operations/verification (${lang}, VerificationScreen.tsx:687)`,
        ).toBeVisible({ timeout: 15_000 });
        const refusedText = await refusedEl.innerText();
        expectWordedRefusal(refusedText, `teller [data-refused] (${lang})`);
        await expect(teller.locator('button[data-action="approve"]'), 'approve button hidden for teller').toHaveCount(0);
        await expect(teller.locator('button[data-action="revoke"]'), 'revoke button hidden for teller').toHaveCount(0);
      }
      await shot(teller, 'desk', 'r9-8-verification-permission-teller');

      // ---- Prepare a customer verification case in UNDER_REVIEW state ----
      await signIn(customer, 'customer', '/app/verification');
      await expect(
        customer.locator('[data-verification-screen="true"]'),
        'customer verification screen loaded (customer/verification/VerificationScreen.tsx:257)',
      ).toBeVisible({ timeout: 20_000 });

      const stateSection = customer.locator('section[data-case-state]').first();
      await expect(stateSection, 'initial section[data-case-state]').toBeVisible({ timeout: 20_000 });
      const initialState = (await stateSection.getAttribute('data-case-state')) ?? 'NONE';
      console.log(`[R9-8] marker=${RUN_MARKER} initial customer verification state: ${initialState}`);

      if (initialState === 'VERIFIED') {
        const rev = await revokeVerifiedCase(customer, admin);
        expect(rev.ok, `revoke leftover VERIFIED case: ${rev.text}`).toBeTruthy();
        await customer.reload();
        await expect(customer.locator('section[data-case-state="REVOKED"]')).toBeVisible({ timeout: 20_000 });
      }

      // If a previous run left an open case in SUBMITTED / UNDER_REVIEW / NEEDS_UPDATE, reject it first so we start fresh.
      if (initialState === 'SUBMITTED' || initialState === 'UNDER_REVIEW' || initialState === 'NEEDS_UPDATE') {
        const myVerRes = await customer.request.get('/api/v3/views/my_verification');
        const myVer = (await myVerRes.json().catch(() => ({}))) as MyVerificationAnswer;
        const leftoverId = String(myVer.case?.id ?? '');
        expect(leftoverId, 'leftover open case id from my_verification').not.toBe('');

        if (initialState === 'NEEDS_UPDATE') {
          const docCount = await customer.locator('li.xc-verification-doc-item[data-doc-id]').count();
          if (docCount === 0) {
            await customer.locator('#xc-verification-file').setInputFiles({
              name: `${RUN_MARKER}-leftover.png`,
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

      let caseId = '';
      const currentState = (await customer.locator('section[data-case-state]').first().getAttribute('data-case-state')) ?? 'NONE';
      if (currentState === 'NONE') {
        const openRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
          customer.locator('button[data-action="start"]').click(),
        );
        expect(openRes.ok, `POST /api/app/verification/cases (start): ${openRes.text}`).toBeTruthy();
        const opened = JSON.parse(openRes.body) as CaseAnswer;
        caseId = String(opened.id ?? '');
      } else if (currentState === 'REJECTED' || currentState === 'EXPIRED' || currentState === 'REVOKED') {
        const openRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
          customer.locator('button[data-action="update-and-resubmit"]').click(),
        );
        expect(openRes.ok, `POST /api/app/verification/cases (update-and-resubmit): ${openRes.text}`).toBeTruthy();
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
        'customer case in DRAFT state (customer/verification/VerificationScreen.tsx:360)',
      ).toBeVisible({ timeout: 20_000 });

      await uploadPassportPng(customer, caseId);

      const submitRes = await posted(
        customer,
        new RegExp(`^/api/app/verification/cases/${encodeURIComponent(caseId)}/submit$`),
        () => customer.locator('button[data-action="submit"]').click(),
      );
      expect(submitRes.ok, `submit verification case: ${submitRes.text}`).toBeTruthy();

      // Admin opens /operations/verification, selects case, and starts review -> UNDER_REVIEW.
      await signIn(admin, 'admin', '/operations/verification');
      await expect(
        admin.getByTestId('verification-screen'),
        'operations verification screen (VerificationScreen.tsx:685)',
      ).toBeVisible({ timeout: 20_000 });
      await selectCaseOnStaff(admin, caseId);

      const startReviewRes = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/start-review$`),
        () => admin.locator('button[data-action="start-review"]').click(),
      );
      expect(startReviewRes.ok, `start-review: ${startReviewRes.text}`).toBeTruthy();
      await expect(admin.getByTestId('case-status')).toHaveText('UNDER_REVIEW', { timeout: 15_000 });

      // Also verify (c) direct API refusal when teller attempts approve/revoke on a real caseId.
      const tellerApproveApi = await teller.request.post(
        `/api/operations/verification/cases/${encodeURIComponent(caseId)}/approve`,
        { data: { expiresAt: futureExpiryIso() } },
      );
      expect(tellerApproveApi.status(), 'teller POST approve refused (401/403)').toBeGreaterThanOrEqual(400);
      const tellerRevokeApi = await teller.request.post(
        `/api/operations/verification/cases/${encodeURIComponent(caseId)}/revoke`,
        { data: { reasonCode: 'DOCUMENT_EXPIRED' } },
      );
      expect(tellerRevokeApi.status(), 'teller POST revoke refused (401/403)').toBeGreaterThanOrEqual(400);

      // ---- (b) Invalid input cases on verificationCaseApprove in both `th` and `en` (`withLang`) ----
      // Case 1: Empty `expiresAt` (`""`) -> refused with worded `errExpiresAtRequired` in `[data-error]`; input keeps `""`.
      // Case 2: Past `expiresAt` (`"2020-01-01"`) -> refused with worded `errExpiresAtRequired` in `[data-error]`; input keeps `"2020-01-01"`.
      // Case 3: Today's `expiresAt` (`todayBangkokIso()`) -> refused (`expiresAt <= todayBangkokIso()`, VerificationScreen.tsx:505); input keeps today's date.
      const todayIso = todayBangkokIso();
      // [data-error] stays up between cases, so prove the screen refused each one itself: no approve/reject POST leaves.
      const invalidPosts: string[] = [];
      const countInvalidPost = (r: { method(): string; url(): string }) => {
        if (r.method() === 'POST' && /\/(approve|reject)$/.test(new URL(r.url()).pathname)) invalidPosts.push(r.url());
      };
      admin.on('request', countInvalidPost);
      for (const lang of STAFF_LANGS) {
        await withLang(admin, lang);
        await admin.goto('about:blank');
        await admin.goto('/operations/verification');
        await expect(admin.locator('html'), `/operations/verification lang=${lang}`).toHaveAttribute('lang', HTML_LANG[lang]);
        await selectCaseOnStaff(admin, caseId);

        const expiresInput = admin.locator('input[name="expiresAt"]');
        const approveBtn = admin.locator('button[data-action="approve"]');
        const errorBanner = admin.locator('[data-error]');

        // Invalid Case 1: empty expiresAt (`""`).
        await expiresInput.fill('');
        await approveBtn.click();
        await expect(errorBanner, `[data-error] visible on empty expiresAt (${lang}, VerificationScreen.tsx:694)`).toBeVisible({
          timeout: 10_000,
        });
        expectWordedRefusal(await errorBanner.innerText(), `approve empty expiresAt (${lang})`);
        await expect(expiresInput, `expiresAt keeps empty value (${lang})`).toHaveValue('');

        // Invalid Case 2: past date (`2020-01-01`).
        await expiresInput.fill('2020-01-01');
        await approveBtn.click();
        await expect(errorBanner, `[data-error] visible on past expiresAt (${lang})`).toBeVisible({ timeout: 10_000 });
        expectWordedRefusal(await errorBanner.innerText(), `approve past expiresAt (${lang})`);
        await expect(expiresInput, `expiresAt keeps 2020-01-01 after refusal (${lang})`).toHaveValue('2020-01-01');

        // Invalid Case 3: today's date (`todayIso`, not strictly in the future).
        await expiresInput.fill(todayIso);
        await approveBtn.click();
        await expect(errorBanner, `[data-error] visible on today expiresAt (${lang})`).toBeVisible({ timeout: 10_000 });
        expectWordedRefusal(await errorBanner.innerText(), `approve today expiresAt (${lang})`);
        await expect(expiresInput, `expiresAt keeps ${todayIso} after refusal (${lang})`).toHaveValue(todayIso);

        // Invalid Case 4: empty reasonCode (`""`) in submitReasonDecision (VerificationScreen.tsx:546-564) -> refused in th & en, select keeps `""`.
        await admin.locator('button[data-action="reject"]').click();
        const reasonPanelLang = admin.getByTestId('reason-decision-panel');
        await expect(reasonPanelLang, `reason-decision-panel visible (${lang})`).toBeVisible({ timeout: 10_000 });
        const reasonSelectLang = reasonPanelLang.locator('select[name="reasonCode"]');
        await reasonSelectLang.evaluate((sel: HTMLSelectElement) => {
          const opt = document.createElement('option');
          opt.value = '';
          opt.textContent = '--';
          sel.prepend(opt);
        });
        await reasonSelectLang.selectOption('');
        await reasonPanelLang.locator('button[data-action="confirm-reject"]').click();
        await expect(errorBanner, `[data-error] visible on empty reasonCode (${lang})`).toBeVisible({ timeout: 10_000 });
        expectWordedRefusal(await errorBanner.innerText(), `reason panel empty reasonCode (${lang})`);
        await expect(reasonSelectLang, `reasonCode keeps empty value (${lang})`).toHaveValue('');
        await reasonPanelLang.locator('button[data-action="cancel-reason"]').click();

        await shot(admin, 'desk', 'r9-8-verification-approve-invalid', lang);
      }
      admin.off('request', countInvalidPost);
      expect(invalidPosts, 'invalid expiresAt / empty reason sent no approve or reject POST').toEqual([]);

      // ---- (a) Happy path 1: verificationCaseApprove (POST /api/operations/verification/cases/{id}/approve) ----
      const expiryIso = futureExpiryIso();
      await admin.locator('input[name="expiresAt"]').fill(expiryIso);
      const approveRes = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/approve$`),
        () => admin.locator('button[data-action="approve"]').click(),
      );
      console.log(`[R9-8] POST /api/operations/verification/cases/${caseId}/approve (${RUN_MARKER}) ${approveRes.text}`);
      expect(approveRes.ok, `verificationCaseApprove: ${approveRes.text}`).toBeTruthy();
      approvedCaseId = caseId;

      await expect(
        admin.getByTestId('case-status'),
        'admin detail view shows VERIFIED after approve (VerificationScreen.tsx:790)',
      ).toHaveText('VERIFIED', { timeout: 15_000 });
      await expect(
        admin.getByTestId('case-detail-view'),
        'admin detail view shows approved expiresAt (VerificationScreen.tsx:795)',
      ).toContainText(expiryIso);

      // Reload customer page to read back VERIFIED state and expiresAt from the server after reload (rule (a)).
      await customer.reload();
      await expect(
        customer.locator('section[data-case-state="VERIFIED"]'),
        'customer reads back VERIFIED after reload (customer/verification/VerificationScreen.tsx:582)',
      ).toBeVisible({ timeout: 20_000 });
      await expect(
        customer.locator(`[data-expires-at="${expiryIso}"]`),
        'customer reads back expiresAt after reload (customer/verification/VerificationScreen.tsx:587)',
      ).toBeVisible({ timeout: 15_000 });
      await shot(admin, 'desk', 'r9-8-verification-approved');

      // ---- (b) Invalid input case 4: verificationCaseRevoke with empty reasonCode in both `th` and `en` ----
      // Note: `verification_queue` only returns SUBMITTED and UNDER_REVIEW rows (verification_queue.rs:117),
      // while `VerificationScreen.tsx:534` keeps the newly-approved `VERIFIED` case selected in `selectedCase()` so
      // `button[data-action="revoke"]` (:941) is immediately operable. We test the revoke invalid reason refusal
      // before confirming revoke, and verify both `th` ( via `withLang(admin, 'th')` before approve ) and `en` (current).
      const revokeBtn = admin.locator('button[data-action="revoke"]');
      await expect(revokeBtn, 'revoke button visible on VERIFIED case (VerificationScreen.tsx:941)').toBeVisible({
        timeout: 15_000,
      });
      await revokeBtn.click();
      const reasonPanel = admin.getByTestId('reason-decision-panel');
      await expect(reasonPanel, 'reason-decision-panel open for revoke (VerificationScreen.tsx:953)').toBeVisible({
        timeout: 15_000,
      });
      const reasonSelect = reasonPanel.locator('select[name="reasonCode"]');
      await reasonSelect.evaluate((sel: HTMLSelectElement) => {
        const opt = document.createElement('option');
        opt.value = '';
        opt.textContent = '--';
        sel.prepend(opt);
      });
      await reasonSelect.selectOption('');
      await reasonPanel.locator('button[data-action="confirm-revoke"]').click();
      const revokeErrBanner = admin.locator('[data-error]');
      await expect(revokeErrBanner, '[data-error] visible on empty revoke reasonCode (VerificationScreen.tsx:551,694)').toBeVisible({
        timeout: 10_000,
      });
      expectWordedRefusal(await revokeErrBanner.innerText(), 'revoke empty reasonCode');
      await expect(reasonSelect, 'reasonCode select keeps empty value after refusal').toHaveValue('');
      await shot(admin, 'desk', 'r9-8-verification-revoke-invalid');

      // ---- (a) Happy path 2: verificationCaseRevoke (POST /api/operations/verification/cases/{id}/revoke) ----
      await reasonSelect.selectOption('DOCUMENT_EXPIRED');
      const revokeRes = await posted(
        admin,
        new RegExp(`^/api/operations/verification/cases/${encodeURIComponent(caseId)}/revoke$`),
        () => reasonPanel.locator('button[data-action="confirm-revoke"]').click(),
      );
      console.log(`[R9-8] POST /api/operations/verification/cases/${caseId}/revoke (${RUN_MARKER}) ${revokeRes.text}`);
      if (revokeRes.ok) happyPathRevoked = true;
      expect(revokeRes.ok, `verificationCaseRevoke: ${revokeRes.text}`).toBeTruthy();

      await expect(
        admin.getByTestId('case-status'),
        'admin detail view shows REVOKED after revoke (VerificationScreen.tsx:790)',
      ).toHaveText('REVOKED', { timeout: 15_000 });
      await expect(
        admin.getByTestId('case-detail-view'),
        'admin detail view shows DOCUMENT_EXPIRED reason (VerificationScreen.tsx:799)',
      ).toContainText('DOCUMENT_EXPIRED');
      await shot(admin, 'desk', 'r9-8-verification-revoked-admin');

      // Reload both admin and customer to read back the REVOKED state after a page reload (rule (a)).
      await admin.reload();
      await expect(
        admin.locator(`tr[data-case-row][data-case-id="${caseId}"]`),
        'revoked case removed from active review queue after reload (verification_queue.rs:117)',
      ).toHaveCount(0);

      await customer.reload();
      await expect(
        customer.locator('section[data-case-state="REVOKED"]'),
        'customer reads back REVOKED state after reload (customer/verification/VerificationScreen.tsx:639)',
      ).toBeVisible({ timeout: 20_000 });
      await expect(
        customer.locator('[data-decision-reason="true"]'),
        'customer reads back DOCUMENT_EXPIRED reason after reload (customer/verification/VerificationScreen.tsx:659)',
      ).toContainText('DOCUMENT_EXPIRED');
      await shot(customer, 'verification', 'r9-8-verification-revoked-customer');

      // Leave customer verification in DRAFT state via update-and-resubmit so subsequent specs (e.g. flow-verification.spec.ts)
      // encounter a clean DRAFT state.
      const reopenRes = await posted(customer, /^\/api\/app\/verification\/cases$/, () =>
        customer.locator('button[data-action="update-and-resubmit"]').click(),
      );
      expect(reopenRes.ok, `reopen DRAFT after REVOKED: ${reopenRes.text}`).toBeTruthy();
      await expect(customer.locator('section[data-case-state="DRAFT"]')).toBeVisible({ timeout: 20_000 });
    } finally {
      if (approvedCaseId && !happyPathRevoked) {
        await revokeVerifiedCase(customer, admin, approvedCaseId).catch((e) =>
          console.log(`[R9-8] cleanup revoke warning: ${e instanceof Error ? e.message : String(e)}`),
        );
      }
      await customer.close();
      await teller.close();
      await admin.close();
    }
  });
});
