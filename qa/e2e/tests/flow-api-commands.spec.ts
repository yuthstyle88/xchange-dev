// QA-R9-13 — flow: commands with no screen, driven through page.request (design docs/design/qa-e2e.md §9.1, §9.4, §9.5; ticket #4297).
// Written by ticket #4297 (QA-R9-13); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Commands covered (docs/design/qa-e2e.md §9.4 rows 8, 34, 54, 55, 61, 67, 68, 71, 108, 111):
// 1. branchPriceSourceSet      — POST /api/branches/price-source
//    - api/src/presentation/http/v3/commands/branch_price_source.rs:38-56,72-77
//    - api/crates/xc-rate/src/infrastructure/branch/price_source.rs:51-118
// 2. cashExceptionRaise        — POST /api/v3/commands/cash_exception_raise
//    - api/src/presentation/http/v3/commands/cash_exception_raise.rs:54-175
//    - api/src/presentation/dto/v3/cash_exception.rs:29-53
// 3. cashExceptionReview       — POST /api/v3/commands/cash_exception_review
//    - api/src/presentation/http/v3/commands/cash_exception_review.rs:48-156
//    - api/src/presentation/dto/v3/cash_exception.rs:9-24
//    - api/src/presentation/http/v3/views/finance_reconciliation.rs:49-68 (GET /api/v3/views/finance_reconciliation)
// 4. customerContactVerify     — POST /api/v3/commands/customer_contact_verify
//    - api/src/presentation/http/v3/commands/customer_contact_verify.rs:28-95
//    - api/crates/xc-customer/src/application/customer/contact_verify.rs:24-30,47-57,93-193
// 5. marginGuardSet            — PUT /api/margin-guard
//    - api/src/presentation/http/v3/commands/margin_guard_set.rs:28-90
//    - api/src/presentation/http/v3/views/margin_guard.rs:34-72 (GET /api/v3/views/margin_guard)
// 6. transactionCancel         — POST /api/transactions/{id}/cancel
//    - api/src/presentation/http/v3/commands/transactions.rs:124-129,184-188,423-448
//    - api/src/application/workflows/exchange/cancel.rs:43-71
// 7. appSettingsSave           — PUT /api/app-settings
//    - web/src/shared/api/generated/app_settings_save.ts:6-8
//    - api/src/presentation/http/v3/commands/app_settings_save.rs:27-52
//    - api/crates/xc-app/src/application/settings/app_settings_service.rs:13-62
// 8. exchangeSettingsSave      — PUT /api/exchange-settings
//    - web/src/shared/api/generated/exchange_settings.ts:22-34
//    - api/src/presentation/http/v3/commands/exchange_settings_save.rs:43-46,106-152
//    - api/src/presentation/http/v3/views/exchange_settings.rs:33-62 (GET /api/v3/views/exchange_settings)
// 9. branchExchangeLimitSave   — PUT /api/branches/{id}/exchange-limit
//    - web/src/shared/api/generated/exchange_settings.ts:7-17,34
//    - api/src/presentation/http/v3/commands/exchange_settings_save.rs:51-56,170-240
// 10. supersedeReferral        — POST /api/referral-attributions/{id}/supersede
//    - web/src/shared/api/generated/supersede_referral.ts:6-12
//    - api/src/presentation/http/v3/commands/supersede_referral.rs:39-97
//    - api/src/presentation/http/v3/commands/attach_referral.rs:107-139
import { expect, type APIResponse, type Page, test } from '@playwright/test';
import { signIn, users } from './helpers';

interface V3ErrorEnvelope {
  error?: {
    code?: string;
    field?: string;
    message?: string;
  };
}

let idemSeq = 0;

/** Headers required by the v3 command seam (api/src/presentation/http/v3/command.rs:256-291). */
function commandHeaders(prefix: string): Record<string, string> {
  idemSeq += 1;
  return {
    'x-xchange-request': '1',
    origin: new URL(users.baseUrl).origin,
    'content-type': 'application/json',
    'idempotency-key': `qa-r9-13-${prefix}-${Date.now()}-${idemSeq}`,
  };
}

async function sendCommand(
  page: Page,
  method: 'POST' | 'PUT',
  url: string,
  data: unknown,
  prefix: string,
): Promise<APIResponse> {
  const headers = commandHeaders(prefix);
  return method === 'PUT'
    ? page.request.put(url, { headers, data })
    : page.request.post(url, { headers, data });
}

/** Asserts (b): 400 Bad Request with a non-empty worded error message that is not just the raw error code (error.rs:48-77). */
async function expectWorded400(res: APIResponse, label: string): Promise<void> {
  const text = await res.text().catch(() => '');
  expect(res.status(), `${label}: expected 400, got ${res.status()} ${text}`).toBe(400);
  const parsed = JSON.parse(text) as V3ErrorEnvelope;
  const code = (parsed.error?.code ?? '').trim();
  const message = (parsed.error?.message ?? '').trim();
  expect(code, `${label}: error.code present`).not.toBe('');
  expect(message, `${label}: error.message non-empty`).not.toBe('');
  expect(message, `${label}: error.message is worded, not raw code`).not.toBe(code);
}

/** Asserts (c): 401/403 refusal with a permission/scope refusal code (error.rs:80-115, command.rs:133-156). */
async function expectPermissionRefused(res: APIResponse, label: string): Promise<void> {
  const text = await res.text().catch(() => '');
  expect([401, 403], `${label}: expected 401/403, got ${res.status()} ${text}`).toContain(res.status());
  const parsed = JSON.parse(text) as V3ErrorEnvelope;
  const code = (parsed.error?.code ?? '').trim();
  const message = (parsed.error?.message ?? '').trim();
  expect(
    ['outOfScope', 'roleNotAllowed', 'roleNotAllowedHere', 'ownerOnly', 'employeeAuthRequired', 'employeeAccount', 'signIn'],
    `${label}: refusal code (${code})`,
  ).toContain(code);
  expect(message, `${label}: refusal error.message non-empty`).not.toBe('');
}

function bangkokDateString(): string {
  return new Intl.DateTimeFormat('en-CA', {
    timeZone: 'Asia/Bangkok',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
  }).format(new Date());
}

test.describe.serial('QA-R9-13 API commands with no screen', () => {
  test('marginGuardSet (PUT /api/margin-guard): (a) save & restore, (b) invalid bodies, (c) permission refusal', async ({
    page,
  }) => {
    // (a) Happy path + restore in finally (margin_guard_set.rs:28-90; view margin_guard.rs:34-72;
    // column max_spread_share is numeric(5,4) in migrations/20261205101000_campaigns.sql:77).
    await signIn(page, 'admin', '/');
    const viewUrl = `/api/v3/views/margin_guard?branch=${encodeURIComponent(users.branch)}`;
    const beforeRes = await page.request.get(viewUrl);
    expect(beforeRes.ok(), `GET ${viewUrl}`).toBeTruthy();
    const beforeBody = (await beforeRes.json()) as { data?: { maxSpreadShare?: string | null } };
    const originalShare: string | null = beforeBody.data?.maxSpreadShare ?? null;
    const nextShare = originalShare !== null && Number(originalShare) === 0.25 ? '0.2200' : '0.2500';

    try {
      const saveRes = await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: nextShare },
        'margin-guard-a',
      );
      const saveText = await saveRes.text();
      expect(saveRes.ok(), `PUT /api/margin-guard: ${saveRes.status()} ${saveText}`).toBeTruthy();
      const saved = JSON.parse(saveText) as { maxSpreadShare?: string | null };
      expect(saved.maxSpreadShare).toBe(nextShare);

      const afterRes = await page.request.get(viewUrl);
      expect(afterRes.ok(), `GET ${viewUrl} after save`).toBeTruthy();
      const afterBody = (await afterRes.json()) as { data?: { maxSpreadShare?: string | null } };
      expect(afterBody.data?.maxSpreadShare).toBe(nextShare);
    } finally {
      const restoreRes = await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: originalShare },
        'margin-guard-restore',
      );
      expect(restoreRes.ok(), `restore margin_guard to ${String(originalShare)}`).toBeTruthy();
    }

    // (b) Invalid bodies: negative (< 0), > 1, non-decimal string, number instead of string (margin_guard_set.rs:70-86; command.rs:383-393).
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: '-0.10' },
        'margin-guard-b1',
      ),
      'marginGuardSet negative maxSpreadShare',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: '1.50' },
        'margin-guard-b2',
      ),
      'marginGuardSet maxSpreadShare > 1',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: 'abc' },
        'margin-guard-b3',
      ),
      'marginGuardSet non-decimal maxSpreadShare',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: 0.2 },
        'margin-guard-b4',
      ),
      'marginGuardSet numeric maxSpreadShare',
    );

    // (c) Permission refusal: teller lacks RatePolicySave / Executive (margin_guard_set.rs:43,57-59).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'PUT',
        '/api/margin-guard',
        { branch: users.branch, maxSpreadShare: '0.2000' },
        'margin-guard-c',
      ),
      'marginGuardSet as teller',
    );
  });

  test('exchangeSettingsSave (PUT /api/exchange-settings): (a) save & restore, (b) invalid bodies, (c) permission refusal', async ({
    page,
  }) => {
    // (a) Happy path + restore in finally (exchange_settings_save.rs:106-152; view exchange_settings.rs:33-62;
    // bounds 30..=3600 in api/crates/xc-exchange/src/infrastructure/settings/repository.rs:24-25,75).
    await signIn(page, 'admin', '/');
    const viewUrl = `/api/v3/views/exchange_settings?branch=${encodeURIComponent(users.branch)}`;
    const beforeRes = await page.request.get(viewUrl);
    expect(beforeRes.ok(), `GET ${viewUrl}`).toBeTruthy();
    const beforeBody = (await beforeRes.json()) as { data?: { quoteTtlSeconds?: number } };
    const originalTtl = beforeBody.data?.quoteTtlSeconds;
    expect(typeof originalTtl, 'exchange_settings.quoteTtlSeconds is a number').toBe('number');
    const nextTtl = originalTtl === 360 ? 330 : 360;

    try {
      const saveRes = await sendCommand(
        page,
        'PUT',
        '/api/exchange-settings',
        { branch: users.branch, quoteTtlSeconds: nextTtl },
        'exchange-settings-a',
      );
      const saveText = await saveRes.text();
      expect(saveRes.ok(), `PUT /api/exchange-settings: ${saveRes.status()} ${saveText}`).toBeTruthy();
      const saved = JSON.parse(saveText) as { quoteTtlSeconds?: number };
      expect(saved.quoteTtlSeconds).toBe(nextTtl);

      const afterRes = await page.request.get(viewUrl);
      expect(afterRes.ok(), `GET ${viewUrl} after save`).toBeTruthy();
      const afterBody = (await afterRes.json()) as { data?: { quoteTtlSeconds?: number } };
      expect(afterBody.data?.quoteTtlSeconds).toBe(nextTtl);
    } finally {
      const restoreRes = await sendCommand(
        page,
        'PUT',
        '/api/exchange-settings',
        { branch: users.branch, quoteTtlSeconds: originalTtl! },
        'exchange-settings-restore',
      );
      expect(restoreRes.ok(), `restore quoteTtlSeconds to ${String(originalTtl)}`).toBeTruthy();
    }

    // (b) Invalid bodies: missing field, < 30, > 3600 (repository.rs:24-25,75; exchange_settings_save.rs:124-131).
    await expectWorded400(
      await sendCommand(page, 'PUT', '/api/exchange-settings', { branch: users.branch }, 'exchange-settings-b1'),
      'exchangeSettingsSave missing quoteTtlSeconds',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/exchange-settings',
        { branch: users.branch, quoteTtlSeconds: 29 },
        'exchange-settings-b2',
      ),
      'exchangeSettingsSave quoteTtlSeconds < 30',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/exchange-settings',
        { branch: users.branch, quoteTtlSeconds: 3601 },
        'exchange-settings-b3',
      ),
      'exchangeSettingsSave quoteTtlSeconds > 3600',
    );

    // (c) Permission refusal: teller lacks RatePolicySave (exchange_settings_save.rs:116).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'PUT',
        '/api/exchange-settings',
        { branch: users.branch, quoteTtlSeconds: originalTtl! },
        'exchange-settings-c',
      ),
      'exchangeSettingsSave as teller',
    );
  });

  test('branchExchangeLimitSave (PUT /api/branches/{id}/exchange-limit): (a) save & restore, (b) invalid bodies, (c) permission refusal', async ({
    page,
  }) => {
    // (a) Happy path + restore in finally (exchange_settings_save.rs:170-240; view exchange_settings.rs:33-62).
    await signIn(page, 'admin', '/');
    const viewUrl = `/api/v3/views/exchange_settings?branch=${encodeURIComponent(users.branch)}`;
    const routeUrl = `/api/branches/${encodeURIComponent(users.branch)}/exchange-limit`;
    const beforeRes = await page.request.get(viewUrl);
    expect(beforeRes.ok(), `GET ${viewUrl}`).toBeTruthy();
    const beforeBody = (await beforeRes.json()) as { data?: { approvalLimitThb?: string | null } };
    const originalLimit = beforeBody.data?.approvalLimitThb ?? null;
    const nextLimit = originalLimit === '500000.00' ? '450000.00' : '500000.00';

    try {
      const saveRes = await sendCommand(
        page,
        'PUT',
        routeUrl,
        { branch: users.branch, approvalLimitThb: nextLimit },
        'branch-limit-a',
      );
      const saveText = await saveRes.text();
      expect(saveRes.ok(), `PUT ${routeUrl}: ${saveRes.status()} ${saveText}`).toBeTruthy();
      const saved = JSON.parse(saveText) as { approvalLimitThb?: string | null };
      expect(saved.approvalLimitThb).toBe(nextLimit);

      const afterRes = await page.request.get(viewUrl);
      expect(afterRes.ok(), `GET ${viewUrl} after save`).toBeTruthy();
      const afterBody = (await afterRes.json()) as { data?: { approvalLimitThb?: string | null } };
      expect(afterBody.data?.approvalLimitThb).toBe(nextLimit);
    } finally {
      const restoreRes = await sendCommand(
        page,
        'PUT',
        routeUrl,
        { branch: users.branch, approvalLimitThb: originalLimit },
        'branch-limit-restore',
      );
      expect(restoreRes.ok(), `restore approvalLimitThb to ${String(originalLimit)}`).toBeTruthy();
    }

    // (b) Invalid bodies: zero, negative, non-2-decimal money string (exchange_settings_save.rs:197-212).
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        routeUrl,
        { branch: users.branch, approvalLimitThb: '0.00' },
        'branch-limit-b1',
      ),
      'branchExchangeLimitSave zero approvalLimitThb',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        routeUrl,
        { branch: users.branch, approvalLimitThb: '-100.00' },
        'branch-limit-b2',
      ),
      'branchExchangeLimitSave negative approvalLimitThb',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        routeUrl,
        { branch: users.branch, approvalLimitThb: '100.123' },
        'branch-limit-b3',
      ),
      'branchExchangeLimitSave >2 decimals approvalLimitThb',
    );

    // (c) Permission refusal: teller lacks BranchesManage (exchange_settings_save.rs:172).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'PUT',
        routeUrl,
        { branch: users.branch, approvalLimitThb: '500000.00' },
        'branch-limit-c',
      ),
      'branchExchangeLimitSave as teller',
    );
  });

  test('appSettingsSave (PUT /api/app-settings): (b) invalid bodies and (c) permission refusal only', async ({
    page,
  }) => {
    // Why (a) is skipped (docs/design/qa-e2e.md §9.2; ticket #4297 §3):
    // customer_app_settings is seeded to planner_draft_ttl_seconds = 604800 (api/migrations/20261106100000_customer_app.sql:99-100)
    // and no GET view exists to record the previous value before saving or to read the change back after a reload.
    await signIn(page, 'admin', '/');

    // (b) Invalid bodies: 0 (NonZeroU32), -1 (u32), string for u32, missing field
    // (app_settings_service.rs:48-50; app_settings_save.rs:19-24; command.rs:383-393).
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/app-settings',
        { branch: users.branch, plannerDraftTtlSeconds: 0 },
        'app-settings-b1',
      ),
      'appSettingsSave plannerDraftTtlSeconds = 0',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/app-settings',
        { branch: users.branch, plannerDraftTtlSeconds: -1 },
        'app-settings-b2',
      ),
      'appSettingsSave negative plannerDraftTtlSeconds',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'PUT',
        '/api/app-settings',
        { branch: users.branch, plannerDraftTtlSeconds: 'abc' },
        'app-settings-b3',
      ),
      'appSettingsSave non-numeric plannerDraftTtlSeconds',
    );
    await expectWorded400(
      await sendCommand(page, 'PUT', '/api/app-settings', { branch: users.branch }, 'app-settings-b4'),
      'appSettingsSave missing plannerDraftTtlSeconds',
    );

    // (c) Permission refusal: teller lacks SystemSettings (app_settings_save.rs:36; app_settings_service.rs:41-47).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'PUT',
        '/api/app-settings',
        { branch: users.branch, plannerDraftTtlSeconds: 604_800 },
        'app-settings-c',
      ),
      'appSettingsSave as teller',
    );
  });

  test('cashExceptionRaise & cashExceptionReview: (a) raise OPEN & review to REVIEWED, (b) invalid bodies, (c) permission refusal', async ({
    page,
  }) => {
    // (a) Paired happy path: cashExceptionRaise creates an OPEN row visible in GET /api/v3/views/finance_reconciliation,
    // and cashExceptionReview marks it REVIEWED so no OPEN cash exception remains on the branch
    // (cash_exception_raise.rs:121-230; cash_exception_review.rs:48-156; reconciliation.rs:233-259).
    await signIn(page, 'admin', '/');
    const marker = `qa-r9-13-${Date.now()}`;
    const today = bangkokDateString();
    const reconUrl = `/api/v3/views/finance_reconciliation?branch=${encodeURIComponent(users.branch)}&date=${today}`;
    let raisedId: string | null = null;
    let reviewed = false;

    try {
      const raiseRes = await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_raise',
        {
          branch: users.branch,
          place: 'DRAWER',
          reason: marker,
          differences: [{ currency: 'THB', amount: '10.00' }],
        },
        'cash-exc-raise-a',
      );
      const raiseText = await raiseRes.text();
      expect(raiseRes.ok(), `POST cash_exception_raise: ${raiseRes.status()} ${raiseText}`).toBeTruthy();
      const raised = JSON.parse(raiseText) as {
        exceptionId?: string;
        status?: string;
      };
      expect(raised.status).toBe('OPEN');
      expect(raised.exceptionId).toBeTruthy();
      raisedId = raised.exceptionId!;

      const openSheetRes = await page.request.get(reconUrl);
      expect(openSheetRes.ok(), `GET ${reconUrl} after raise`).toBeTruthy();
      const openSheet = (await openSheetRes.json()) as {
        data?: { exceptions?: { rows?: Array<{ id: string; summary: string }> } };
      };
      const openRows = openSheet.data?.exceptions?.rows ?? [];
      expect(
        openRows.some((r) => r.id === raisedId && r.summary.includes(marker)),
        `finance_reconciliation open exceptions includes ${raisedId}`,
      ).toBe(true);

      const reviewRes = await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_review',
        {
          branch: users.branch,
          exceptionId: raisedId,
          note: `${marker}-reviewed`,
        },
        'cash-exc-review-a',
      );
      const reviewText = await reviewRes.text();
      expect(reviewRes.ok(), `POST cash_exception_review: ${reviewRes.status()} ${reviewText}`).toBeTruthy();
      const reviewOut = JSON.parse(reviewText) as {
        exceptionId?: string;
        status?: string;
      };
      expect(reviewOut.exceptionId).toBe(raisedId);
      expect(reviewOut.status).toBe('REVIEWED');
      reviewed = true;

      const closedSheetRes = await page.request.get(reconUrl);
      expect(closedSheetRes.ok(), `GET ${reconUrl} after review`).toBeTruthy();
      const closedSheet = (await closedSheetRes.json()) as {
        data?: { exceptions?: { rows?: Array<{ id: string }> } };
      };
      const remainingRows = closedSheet.data?.exceptions?.rows ?? [];
      expect(
        remainingRows.some((r) => r.id === raisedId),
        `finance_reconciliation open exceptions no longer includes reviewed ${raisedId}`,
      ).toBe(false);
    } finally {
      if (raisedId && !reviewed) {
        const cleanupRes = await sendCommand(
          page,
          'POST',
          '/api/v3/commands/cash_exception_review',
          {
            branch: users.branch,
            exceptionId: raisedId,
            note: `${marker}-cleanup`,
          },
          'cash-exc-review-cleanup',
        );
        if (!cleanupRes.ok()) {
          const cleanupText = await cleanupRes.text().catch(() => '');
          console.warn(
            `[R9-13] cleanup cashExceptionReview for ${raisedId} failed: ${cleanupRes.status()} ${cleanupText}`,
          );
        }
      }
    }

    // (b) Invalid bodies for cashExceptionRaise (cash_exception_raise.rs:54-119,157-175).
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_raise',
        {
          branch: users.branch,
          place: 'DRAWER',
          reason: '   ',
          differences: [{ currency: 'THB', amount: '10.00' }],
        },
        'cash-exc-raise-b1',
      ),
      'cashExceptionRaise empty reason',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_raise',
        {
          branch: users.branch,
          place: 'VAULT',
          reason: marker,
          differences: [{ currency: 'THB', amount: '10.00' }],
        },
        'cash-exc-raise-b2',
      ),
      'cashExceptionRaise invalid place',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_raise',
        {
          branch: users.branch,
          place: 'DRAWER',
          reason: marker,
          differences: [{ currency: 'THB', amount: '0.00' }],
        },
        'cash-exc-raise-b3',
      ),
      'cashExceptionRaise zero difference amount',
    );

    // (b) Invalid bodies for cashExceptionReview (cash_exception_review.rs:84-102).
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_review',
        { branch: users.branch, exceptionId: '   ', note: marker },
        'cash-exc-review-b1',
      ),
      'cashExceptionReview empty exceptionId',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_review',
        { branch: users.branch, exceptionId: 'exc-unknown', note: '   ' },
        'cash-exc-review-b2',
      ),
      'cashExceptionReview empty note',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_review',
        { branch: users.branch, exceptionId: 'exc-unknown' },
        'cash-exc-review-b3',
      ),
      'cashExceptionReview missing note',
    );

    // (c) Permission refusal for both cashExceptionRaise and cashExceptionReview: teller lacks FinanceReconcile
    // (cash_exception_raise.rs:144-155; cash_exception_review.rs:70-82).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_raise',
        {
          branch: users.branch,
          place: 'DRAWER',
          reason: marker,
          differences: [{ currency: 'THB', amount: '10.00' }],
        },
        'cash-exc-raise-c',
      ),
      'cashExceptionRaise as teller',
    );
    await expectPermissionRefused(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/cash_exception_review',
        { branch: users.branch, exceptionId: 'exc-unknown', note: marker },
        'cash-exc-review-c',
      ),
      'cashExceptionReview as teller',
    );
  });

  test('branchPriceSourceSet (POST /api/branches/price-source): (b) invalid bodies and (c) permission refusal only', async ({
    page,
  }) => {
    // Why (a) is skipped (docs/design/qa-e2e.md §9.2; ticket #4297 §3):
    // Switching a branch to 'RATE_BOARD' cannot be undone ('RATE_BOARD' -> 'BOT_DRAFT') without publishing an approved
    // BoT rate draft in the same transaction (branch_price_source.rs:6-9; price_source.rs:86-91), which violates E-4.
    await signIn(page, 'admin', '/');

    // (b) Invalid bodies: missing priceSource, wrong enum value, bad draftId UUID (branch_price_source.rs:38-43,89-96).
    await expectWorded400(
      await sendCommand(page, 'POST', '/api/branches/price-source', { branch: users.branch }, 'price-source-b1'),
      'branchPriceSourceSet missing priceSource',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/branches/price-source',
        { branch: users.branch, priceSource: 'UNKNOWN_SOURCE' },
        'price-source-b2',
      ),
      'branchPriceSourceSet invalid priceSource enum',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/branches/price-source',
        { branch: users.branch, priceSource: 'BOT_DRAFT', draftId: 'not-a-uuid' },
        'price-source-b3',
      ),
      'branchPriceSourceSet invalid draftId UUID',
    );

    // (c) Permission refusal: teller lacks RatesDraft / Executive org-wide (branch_price_source.rs:73; price_source.rs:59-67).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'POST',
        '/api/branches/price-source',
        { branch: users.branch, priceSource: 'BOT_DRAFT' },
        'price-source-c',
      ),
      'branchPriceSourceSet as teller',
    );
  });

  test('customerContactVerify (POST /api/v3/commands/customer_contact_verify): (b) invalid bodies and (c) permission refusal only', async ({
    page,
  }) => {
    // Why (a) is skipped (docs/design/qa-e2e.md §9.2; ticket #4297 §3):
    // customerContactVerify revokes the customer's active PHONE row in customer_identities and inserts a new encrypted
    // identity row with no delete/undo command (contact_verify.rs:150-183), permanently altering seeded customer records.
    await signIn(page, 'admin', '/');

    // (b) Invalid bodies: wrong type for customerId, wrong type for phone, missing phone
    // (customer_contact_verify.rs:28-33; command.rs:383-393).
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/customer_contact_verify',
        { branch: users.branch, customerId: 123, phone: '0812345678' },
        'contact-verify-b1',
      ),
      'customerContactVerify non-string customerId',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/customer_contact_verify',
        { branch: users.branch, customerId: 'CUST-1', phone: 123 },
        'contact-verify-b2',
      ),
      'customerContactVerify non-string phone',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/customer_contact_verify',
        { branch: users.branch, customerId: 'CUST-1' },
        'contact-verify-b3',
      ),
      'customerContactVerify missing phone',
    );

    // (c) Permission refusal: customer lacks ExchangeTransact / saveTransaction (customer_contact_verify.rs:47).
    await signIn(page, 'customer', '/app');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'POST',
        '/api/v3/commands/customer_contact_verify',
        { branch: users.branch, customerId: 'CUST-1', phone: '0812345678' },
        'contact-verify-c',
      ),
      'customerContactVerify as customer',
    );
  });

  test('transactionCancel (POST /api/transactions/{id}/cancel): (b) invalid bodies and (c) permission refusal only', async ({
    page,
  }) => {
    // Why (a) is skipped (docs/design/qa-e2e.md §9.2; ticket #4297 §3):
    // Creating a DRAFT transaction requires an open teller shift, an active quote, and an uploaded passport scan
    // (transactions.rs:104,341; create.rs:121-136), and cancelling permanently records a CANCELLED row in transactions
    // (cancel.rs:58-67) that cannot be removed.
    await signIn(page, 'admin', '/');
    const zeroUuid = '00000000-0000-0000-0000-000000000000';

    // (b) Invalid bodies: non-UUID path id, wrong type for reason, invalid branch (transactions.rs:124-129,184-188,439; command.rs:342-393).
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        '/api/transactions/not-a-uuid/cancel',
        { branch: users.branch, reason: 'bad id' },
        'tx-cancel-b1',
      ),
      'transactionCancel non-UUID id',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        `/api/transactions/${zeroUuid}/cancel`,
        { branch: users.branch, reason: 12345 },
        'tx-cancel-b2',
      ),
      'transactionCancel non-string reason',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        `/api/transactions/${zeroUuid}/cancel`,
        { branch: 'invalid branch!', reason: 'bad branch' },
        'tx-cancel-b3',
      ),
      'transactionCancel invalid branch',
    );

    // (c) Permission refusal: customer lacks ExchangeTransact / saveTransaction (transactions.rs:425).
    await signIn(page, 'customer', '/app');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'POST',
        `/api/transactions/${zeroUuid}/cancel`,
        { branch: users.branch, reason: 'unauthorized' },
        'tx-cancel-c',
      ),
      'transactionCancel as customer',
    );
  });

  test('supersedeReferral (POST /api/referral-attributions/{id}/supersede): (b) invalid bodies and (c) permission refusal only', async ({
    page,
  }) => {
    // Why (a) is skipped (docs/design/qa-e2e.md §9.2; ticket #4297 §3):
    // supersedeReferral requires an existing ACTIVE referral_attributions row (created on a transaction via attachReferral
    // in R9-9 flow-desk-extras.spec.ts) and permanently marks the old row SUPERSEDED while inserting a new ACTIVE row
    // with no delete/undo command (supersede_referral.rs:4-6,55-92).
    await signIn(page, 'admin', '/');
    const zeroUuid = '00000000-0000-0000-0000-000000000000';
    const routeUrl = `/api/referral-attributions/${zeroUuid}/supersede`;

    // (b) Invalid bodies: missing referralCode, empty reason, invalid referralCode format
    // (supersede_referral.rs:71-74; referral_code.rs:14-26; attach_referral.rs:122-133).
    await expectWorded400(
      await sendCommand(page, 'POST', routeUrl, { branch: users.branch, reason: 'missing code' }, 'supersede-b1'),
      'supersedeReferral missing referralCode',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        routeUrl,
        { branch: users.branch, referralCode: 'REF12345', reason: '   ' },
        'supersede-b2',
      ),
      'supersedeReferral empty reason',
    );
    await expectWorded400(
      await sendCommand(
        page,
        'POST',
        routeUrl,
        { branch: users.branch, referralCode: 'ab!', reason: 'invalid format' },
        'supersede-b3',
      ),
      'supersedeReferral bad referralCode format',
    );

    // (c) Permission refusal: teller lacks ReferralSupersede (supersede_referral.rs:2,41,77-79).
    await signIn(page, 'teller', '/staff');
    await expectPermissionRefused(
      await sendCommand(
        page,
        'POST',
        routeUrl,
        { branch: users.branch, referralCode: 'REF12345', reason: 'unauthorized' },
        'supersede-c',
      ),
      'supersedeReferral as teller',
    );
  });
});
