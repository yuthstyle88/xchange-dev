// QA-R9-7 — flow: branch profile (`branchSave`); consent text (`consentTextPublish`) and retention (`retentionSet`) validation only
// (design docs/design/qa-e2e.md §9 row R9-7, §9.1, §9.2; ticket #4291).
// Written by ticket #4291 (QA-R9-7); this file is its whole File scope.
// Import only from '@playwright/test' and './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Page } from '@playwright/test';
import { hasAccount, mePermissions, shot, signIn, users, withLang } from './helpers';

// Expected i18n copy for branch profile (`web/src/screens/admin/branches/i18n.ts:28-30, 75-77`).
const BRANCH_TEXTS = {
  th: {
    timezoneRequired: 'กรุณากรอกเขตเวลา', // web/src/screens/admin/branches/i18n.ts:28
    codeConflict: 'รหัสสาขาซ้ำกับสาขาอื่น', // web/src/screens/admin/branches/i18n.ts:29
    saved: 'บันทึกข้อมูลสาขาเรียบร้อยแล้ว', // web/src/screens/admin/branches/i18n.ts
  },
  en: {
    timezoneRequired: 'Timezone is required', // web/src/screens/admin/branches/i18n.ts:75
    codeConflict: 'Branch code is already used by another branch', // web/src/screens/admin/branches/i18n.ts:76
    saved: 'Branch profile saved successfully', // web/src/screens/admin/branches/i18n.ts
  },
} as const;

// Expected i18n copy for consent and retention (`web/src/screens/admin/consent/i18n.ts:7, 10, 22, 25`).
const CONSENT_TEXTS = {
  th: {
    bodyRequired: 'กรุณาระบุข้อความ', // web/src/screens/admin/consent/i18n.ts:7
    periodInvalid: 'ระยะเวลาไม่ถูกต้อง', // web/src/screens/admin/consent/i18n.ts:10
  },
  en: {
    bodyRequired: 'Consent text body is required', // web/src/screens/admin/consent/i18n.ts:22
    periodInvalid: 'Retention period must be a positive whole number', // web/src/screens/admin/consent/i18n.ts:25
  },
} as const;

interface StoredBranchProfile {
  code: string;
  address: string;
  timezone: string;
  bookingEnabled: boolean;
}

async function openBranchesAsAdmin(page: Page, hash = '#branches'): Promise<void> {
  const path = `/?branch=${encodeURIComponent(users.branch)}${hash}`;
  await page.goto(path);
  // Screen root `.branches-screen` (`web/src/screens/admin/branches/BranchesScreen.tsx:237`)
  // and form `.branches-form` (`web/src/screens/admin/branches/BranchesScreen.tsx:283`).
  await expect(page.locator('.branches-screen')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('form.branches-form')).toBeVisible({ timeout: 20_000 });
  // Wait for `loadProfile` (`BranchesScreen.tsx:95-136`) to populate `#branch-code` (`BranchesScreen.tsx:286`).
  await expect(page.locator('#branch-code')).not.toHaveValue('', { timeout: 20_000 });
}

async function readBranchForm(page: Page): Promise<StoredBranchProfile> {
  // Inputs on `web/src/screens/admin/branches/BranchesScreen.tsx:286-327`.
  const code = await page.locator('#branch-code').inputValue();
  const address = await page.locator('#branch-address').inputValue();
  const timezone = await page.locator('#branch-timezone').inputValue();
  const bookingEnabled = await page.locator('#branch-booking').isChecked();
  return { code, address, timezone, bookingEnabled };
}

async function restoreBranchForm(page: Page, orig: StoredBranchProfile): Promise<void> {
  await openBranchesAsAdmin(page);
  // Inputs `#branch-code` (:286), `#branch-address` (:298), `#branch-timezone` (:309), `#branch-booking` (:321).
  const codeInput = page.locator('#branch-code');
  const addressInput = page.locator('#branch-address');
  const timezoneInput = page.locator('#branch-timezone');
  const bookingInput = page.locator('#branch-booking');
  // Submit button `form.branches-form button[type="submit"]` (`BranchesScreen.tsx:380`).
  const submitBtn = page.locator('form.branches-form button[type="submit"]');
  // Success status `[role="status"].branches-form-success` (`BranchesScreen.tsx:273`).
  const successStatus = page.locator('[role="status"].branches-form-success');

  await codeInput.fill(orig.code);
  await addressInput.fill(orig.address);
  await timezoneInput.fill(orig.timezone);
  await bookingInput.setChecked(orig.bookingEnabled);

  const putPromise = page.waitForResponse(
    (res) => res.request().method() === 'PUT' && new URL(res.url()).pathname === '/api/branches/profile',
    { timeout: 15_000 },
  );
  await submitBtn.click();
  const putRes = await putPromise;
  expect(putRes.ok(), `restore PUT /api/branches/profile: ${putRes.status()}`).toBe(true);
  await expect(successStatus).toBeVisible({ timeout: 15_000 });
}

test.describe('QA-R9-7 flow: branch profile (`branchSave`); consent text and retention validation only', () => {
  test('branchSave (a) happy path: admin updates branch address, reloads to verify, and restores in finally', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    await signIn(page, 'admin', `/?branch=${encodeURIComponent(users.branch)}`);
    await openBranchesAsAdmin(page);

    // Branch selector `#branch-select` (`web/src/screens/admin/branches/BranchesScreen.tsx:242`).
    const branchSelect = page.locator('#branch-select');
    await expect(branchSelect).toBeVisible({ timeout: 15_000 });

    // Form fields (`web/src/screens/admin/branches/BranchesScreen.tsx:286-327, 380`).
    const addressInput = page.locator('#branch-address');
    const submitBtn = page.locator('form.branches-form button[type="submit"]');
    const successBanner = page.locator('[role="status"].branches-form-success');
    const errorAlert = page.locator('[role="alert"].branches-form-error');

    const orig = await readBranchForm(page);
    const markerAddress = `qa-r9-7-${Date.now()}`;
    const restoreFailures: string[] = [];

    try {
      const putPromise = page.waitForResponse(
        (res) => res.request().method() === 'PUT' && new URL(res.url()).pathname === '/api/branches/profile',
        { timeout: 15_000 },
      );

      await addressInput.fill(markerAddress);
      await expect(submitBtn).toBeEnabled();
      await submitBtn.click();

      const putRes = await putPromise;
      expect(putRes.ok(), `PUT /api/branches/profile responded ok: ${putRes.status()}`).toBe(true);
      await expect(successBanner).toBeVisible({ timeout: 15_000 });
      await expect(errorAlert).toHaveCount(0);

      // Reload the page and read the saved change back (`BranchesScreen.tsx:106-116`).
      await page.reload();
      await expect(page.locator('.branches-screen')).toBeVisible({ timeout: 20_000 });
      await expect(page.locator('#branch-code')).not.toHaveValue('', { timeout: 20_000 });
      await expect(addressInput).toHaveValue(markerAddress);

      await shot(page, 'admin', 'r9-7-branch-saved');
    } finally {
      try {
        await restoreBranchForm(page, orig);
        await page.reload();
        await expect(page.locator('.branches-screen')).toBeVisible({ timeout: 20_000 });
        await expect(page.locator('#branch-code')).not.toHaveValue('', { timeout: 20_000 });
        await expect(addressInput).toHaveValue(orig.address);
      } catch (err) {
        console.error('Failed to restore branch profile in finally:', err);
        restoreFailures.push(String(err));
      }
    }

    expect(restoreFailures, 'branch profile restored to original values in finally').toEqual([]);
  });

  test('branchSave (b) invalid inputs: 4 invalid cases refused and worded in th and en, keeping typed input', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    await signIn(page, 'admin', `/?branch=${encodeURIComponent(users.branch)}`);
    await openBranchesAsAdmin(page);

    const orig = await readBranchForm(page);
    const restoreFailures: string[] = [];

    // Form controls (`web/src/screens/admin/branches/BranchesScreen.tsx:270, 286, 309, 339-368, 380`).
    const codeInput = page.locator('#branch-code');
    const timezoneInput = page.locator('#branch-timezone');
    const firstDayRow = page.locator('.branches-hours-row').first();
    const firstDayCheckbox = firstDayRow.locator('input[type="checkbox"]');
    const firstDayOpenInput = firstDayRow.locator('input.branches-time-input[type="time"]').first();
    const submitBtn = page.locator('form.branches-form button[type="submit"]');
    const errorAlert = page.locator('[role="alert"].branches-form-error');

    try {
      for (const lang of ['th', 'en'] as const) {
        await withLang(page, lang);
        await page.reload();
        await openBranchesAsAdmin(page);

        // Case 1: whitespace-only timezone (`#branch-timezone`, `BranchesScreen.tsx:185-189, 309`).
        // Whitespace satisfies HTML5 `required` (`:315`) and triggers `handleSave`'s `!timezone().trim()` check (`:186`).
        await timezoneInput.fill('   ');
        await submitBtn.click();
        await expect(errorAlert).toBeVisible({ timeout: 10_000 });
        await expect(errorAlert).toHaveText(BRANCH_TEXTS[lang].timezoneRequired);
        await expect(timezoneInput, `${lang}: #branch-timezone keeps typed whitespace`).toHaveValue('   ');
        await timezoneInput.fill(orig.timezone);

        // Case 2: whitespace-only branch code (`#branch-code`, `BranchesScreen.tsx:205, 286`;
        // `api/crates/xc-rate/src/application/branch/service.rs:237-239` refuses empty trimmed code).
        const emptyCodeResPromise = page.waitForResponse(
          (res) => res.request().method() === 'PUT' && new URL(res.url()).pathname === '/api/branches/profile',
          { timeout: 15_000 },
        );
        await codeInput.fill('   ');
        await submitBtn.click();
        const emptyCodeRes = await emptyCodeResPromise;
        expect(emptyCodeRes.status(), `${lang}: empty code refused with 400`).toBe(400);
        await expect(errorAlert).toBeVisible({ timeout: 10_000 });
        await expect(errorAlert).not.toHaveText(/^(?:validationFailed|\s*)$/);
        await expect(codeInput, `${lang}: #branch-code keeps typed whitespace`).toHaveValue('   ');
        await codeInput.fill(orig.code);

        // Case 3: unknown IANA timezone (`#branch-timezone`, `BranchesScreen.tsx:207, 309`;
        // `api/crates/xc-rate/src/domain/branch/branch_hours.rs:59-60` refuses unknown timezone).
        const badTz = 'Not/A_Zone_qa_r9_7';
        const badTzResPromise = page.waitForResponse(
          (res) => res.request().method() === 'PUT' && new URL(res.url()).pathname === '/api/branches/profile',
          { timeout: 15_000 },
        );
        await timezoneInput.fill(badTz);
        await submitBtn.click();
        const badTzRes = await badTzResPromise;
        expect(badTzRes.status(), `${lang}: invalid IANA timezone refused with 400`).toBe(400);
        await expect(errorAlert).toBeVisible({ timeout: 10_000 });
        await expect(errorAlert).not.toHaveText(/^(?:validationFailed|\s*)$/);
        await expect(timezoneInput, `${lang}: #branch-timezone keeps typed invalid timezone`).toHaveValue(badTz);
        await timezoneInput.fill(orig.timezone);

        // Case 4: open day with cleared opening time (`BranchesScreen.tsx:198-200, 339-368`;
        // `api/crates/xc-rate/src/domain/branch/branch_hours.rs:89-93` refuses non-HH:MM open/close).
        await firstDayCheckbox.setChecked(true);
        await expect(firstDayOpenInput).toBeVisible();
        const badHoursResPromise = page.waitForResponse(
          (res) => res.request().method() === 'PUT' && new URL(res.url()).pathname === '/api/branches/profile',
          { timeout: 15_000 },
        );
        await firstDayOpenInput.fill('');
        await submitBtn.click();
        const badHoursRes = await badHoursResPromise;
        expect(badHoursRes.status(), `${lang}: malformed opening hours refused with 400`).toBe(400);
        await expect(errorAlert).toBeVisible({ timeout: 10_000 });
        await expect(errorAlert).not.toHaveText(/^(?:validationFailed|\s*)$/);
        await expect(firstDayOpenInput, `${lang}: time input keeps cleared value`).toHaveValue('');
      }
    } finally {
      try {
        await withLang(page, 'th');
        await page.reload();
        // Reload so the check reads what the server holds, not the edited form.
        await openBranchesAsAdmin(page);
        const check = await readBranchForm(page).catch(() => null);
        if (
          !check ||
          check.code !== orig.code ||
          check.address !== orig.address ||
          check.timezone !== orig.timezone ||
          check.bookingEnabled !== orig.bookingEnabled
        ) {
          await restoreBranchForm(page, orig);
        }
      } catch (err) {
        console.error('Failed to verify/restore branch profile after invalid cases:', err);
        restoreFailures.push(String(err));
      }
    }

    expect(restoreFailures, 'branch profile remains intact after invalid input checks').toEqual([]);
  });

  test('consentTextPublish & retentionSet (b) validation only (§9.2): invalid inputs refused in th and en without sending writes', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');

    // §9.2: consentTextPublish and retentionSet are validation-only; never send a write request.
    const unexpectedWrites: string[] = [];
    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      if (
        (req.method() === 'POST' && path === '/api/consent-texts') ||
        (req.method() === 'PUT' && /^\/api\/retention-policies\/[^/]+$/.test(path))
      ) {
        unexpectedWrites.push(`${req.method()} ${path}`);
      }
    });

    await signIn(page, 'admin', `/?branch=${encodeURIComponent(users.branch)}`);

    // Screen root `.admin-consent-screen` (`web/src/screens/admin/consent/Consent.tsx:187`),
    // publish form `.consent-publish-form` (`Consent.tsx:204`),
    // retention section `.retention-section` (`Consent.tsx:261`).
    const screen = page.locator('.admin-consent-screen');
    const publishForm = screen.locator('.consent-publish-form');
    // Textarea `.consent-publish-form textarea` (`Consent.tsx:235`),
    // error `.consent-publish-form .error` (`Consent.tsx:243`),
    // publish button `.consent-publish-form button.primary` (`Consent.tsx:250`).
    const bodyTextarea = publishForm.locator('textarea');
    const publishError = publishForm.locator('.error');
    const publishBtn = publishForm.locator('button.primary');

    for (const lang of ['th', 'en'] as const) {
      await withLang(page, lang);
      await page.goto(`/?branch=${encodeURIComponent(users.branch)}#consent`);
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await expect(publishForm).toBeVisible({ timeout: 15_000 });
      await expect(screen.locator('.retention-section')).toBeVisible({ timeout: 15_000 });

      // 1. consentTextPublish invalid case A: empty body (`Consent.tsx:154-157`).
      await bodyTextarea.fill('');
      await publishBtn.click();
      await expect(publishError).toBeVisible({ timeout: 10_000 });
      await expect(publishError).toHaveText(CONSENT_TEXTS[lang].bodyRequired);
      await expect(bodyTextarea, `${lang}: empty consent body stays in textarea`).toHaveValue('');

      // 2. consentTextPublish invalid case B: whitespace-only body (`Consent.tsx:154-157`).
      const blankBody = '   \n  ';
      await bodyTextarea.fill(blankBody);
      await publishBtn.click();
      await expect(publishError).toBeVisible({ timeout: 10_000 });
      await expect(publishError).toHaveText(CONSENT_TEXTS[lang].bodyRequired);
      await expect(bodyTextarea, `${lang}: whitespace consent body stays in textarea`).toHaveValue(blankBody);

      // 3. retentionSet invalid cases (`Consent.tsx:17, 42-50, 68-92`):
      // Rows `[data-retention="IDENTITY_VALUE"]`, `[data-retention="CONTACT"]`, `[data-retention="CLOSED_CUSTOMER"]` (`Consent.tsx:17, 68`),
      // input `input[type="text"]` (`Consent.tsx:74`), error `.error` (`Consent.tsx:83`), save button `button[type="button"]` (`Consent.tsx:85`).
      // Never submit an empty string because `days === ''` means `null` (never purged) and would send a PUT (`Consent.tsx:43`).
      const invalidRetentionCases: ReadonlyArray<{ dataType: string; typed: string; label: string }> = [
        { dataType: 'IDENTITY_VALUE', typed: '0', label: 'zero days' },
        { dataType: 'IDENTITY_VALUE', typed: '-5', label: 'negative days' },
        { dataType: 'CONTACT', typed: 'abc', label: 'non-numeric string' },
        { dataType: 'CLOSED_CUSTOMER', typed: '12.5', label: 'fractional days' },
      ];

      for (const item of invalidRetentionCases) {
        const row = screen.locator(`[data-retention="${item.dataType}"]`);
        const periodInput = row.locator('input[type="text"]');
        const rowError = row.locator('.error');
        const saveBtn = row.locator('button[type="button"]');

        await expect(row).toBeVisible({ timeout: 10_000 });
        await periodInput.fill(item.typed);
        await saveBtn.click();

        await expect(rowError, `${lang} (${item.label}): error shown on ${item.dataType}`).toBeVisible({
          timeout: 10_000,
        });
        await expect(rowError, `${lang} (${item.label}): worded error matches i18n`).toHaveText(
          CONSENT_TEXTS[lang].periodInvalid,
        );
        await expect(periodInput, `${lang} (${item.label}): input keeps typed value`).toHaveValue(item.typed);
      }

      await shot(page, 'admin', `r9-7-consent-validation-${lang}`, lang);
    }

    expect(
      unexpectedWrites,
      '§9.2 validation-only: zero POST /api/consent-texts or PUT /api/retention-policies/* requests sent',
    ).toEqual([]);
  });

  test('branchSave, consentTextPublish & retentionSet (c) permission: teller lacks menu links and API refuses writes with 403', async ({
    page,
  }) => {
    // Menu and route RBAC across roles is also covered in `qa/e2e/tests/smoke-rbac-matrix.spec.ts:34-38, 141-169`
    // (`consent` needs `ConsentPolicyManage` at `web/src/screens/admin/consent/menu.ts:14`;
    // `branches` needs `BranchesManage` at `web/src/screens/admin/branches/menu.ts:12`).
    test.skip(!hasAccount('teller'), 'no teller account in users.json: run scripts/reseed.sh');
    await signIn(page, 'teller', '/');

    const perms = await mePermissions(page);
    expect(perms, 'teller lacks BranchesManage').not.toContain('BranchesManage');
    expect(perms, 'teller lacks ConsentPolicyManage').not.toContain('ConsentPolicyManage');

    // Nav links `nav.admin-nav a[href="#branches"]` and `nav.admin-nav a[href="#consent"]` (`web/src/screens/admin/AdminShell.tsx:274-282`).
    await expect(page.locator('nav.admin-nav a[href="#branches"]')).toHaveCount(0);
    await expect(page.locator('nav.admin-nav a[href="#consent"]')).toHaveCount(0);

    // Direct hash navigation to `#branches` or `#consent` does not mount `.branches-screen` (`BranchesScreen.tsx:237`)
    // or `.admin-consent-screen` (`Consent.tsx:187`).
    await page.goto('/#branches');
    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('.branches-screen')).toHaveCount(0);

    await page.goto('/#consent');
    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('.admin-consent-screen')).toHaveCount(0);

    // Direct API command calls as `teller` are refused by policy (400 roleNotAllowed/roleNotAllowedHere or
    // 403 outOfScope, api/src/presentation/http/v3/error.rs:80-115) before the input is parsed
    // (`api/src/presentation/http/v3/command.rs:367` before `:383`). Every payload is also invalid, so a broken
    // permission check still writes nothing: no branch change, no published text, no armed purge (§9.2).
    const sameOriginHeaders = {
      'x-xchange-request': '1',
      origin: new URL(users.baseUrl).origin,
    };
    const assertTellerRoleRefused = async (
      res: { status(): number; json(): Promise<unknown> },
      label: string,
    ): Promise<void> => {
      const status = res.status();
      const body = (await res.json().catch(() => ({}))) as {
        error?: { code?: string };
        code?: string;
      };
      const code = body.error?.code ?? body.code ?? '';
      expect([400, 403], `${label}: status ${status} (code=${code})`).toContain(status);
      if (status === 400) {
        expect(['roleNotAllowed', 'roleNotAllowedHere'], `${label}: 400 error code`).toContain(code);
      } else {
        expect(code, `${label}: 403 error code`).toBe('outOfScope');
      }
    };

    const branchRes = await page.request.put('/api/branches/profile', {
      headers: {
        'idempotency-key': `qa-r9-7-branch-${Date.now()}`,
        ...sameOriginHeaders,
      },
      data: {
        branch: users.branch,
        code: users.branch,
        address: 'teller-refused',
        timezone: 'Not/A_Zone_qa_r9_7',
        openingHours: {},
        bookingEnabled: false,
      },
    });
    await assertTellerRoleRefused(branchRes, 'teller PUT /api/branches/profile');

    const consentRes = await page.request.post('/api/consent-texts', {
      headers: {
        'idempotency-key': `qa-r9-7-consent-${Date.now()}`,
        ...sameOriginHeaders,
      },
      data: {
        branch: users.branch,
        purpose: 'TERMS',
        lang: 'th',
        body: '',
      },
    });
    await assertTellerRoleRefused(consentRes, 'teller POST /api/consent-texts');

    const retentionRes = await page.request.put('/api/retention-policies/IDENTITY_VALUE', {
      headers: {
        'idempotency-key': `qa-r9-7-retention-${Date.now()}`,
        ...sameOriginHeaders,
      },
      data: {
        branch: users.branch,
        dataType: 'IDENTITY_VALUE',
        periodDays: -1,
      },
    });
    await assertTellerRoleRefused(retentionRes, 'teller PUT /api/retention-policies/IDENTITY_VALUE');
  });
});
