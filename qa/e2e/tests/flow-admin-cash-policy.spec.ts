// QA-R9-5 (#4289) — e2e round 9: cash limits, transfer policy, reconcile, denomination groups, reconciliation certify
// (design docs/design/qa-e2e.md §9.1–§9.4). This file is the ticket's whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Page } from '@playwright/test';
import { hasAccount, mePermissions, posted, shot, signIn, users, withLang } from './helpers';

const adminPath = (hash: string) => `/?branch=${encodeURIComponent(users.branch)}#${hash}`;

// Worded refusal strings from each screen's i18n.ts table:
// - web/src/screens/admin/cash-limits/i18n.ts:13,29
const CASH_LIMITS_ERR = {
  th: 'เกณฑ์ยอดนับต่างไม่ถูกต้อง ต้องเป็นตัวเลขตั้งแต่ 0 ขึ้นไปและทศนิยมไม่เกิน 2 ตำแหน่ง',
  en: 'Invalid count difference limit: enter a non-negative amount with at most 2 decimal places',
} as const;

// - web/src/screens/admin/cash-policy/i18n.ts:16,36
const CASH_POLICY_ERR = {
  th: 'วงเงินไม่ถูกต้อง ต้องเป็นจำนวนตั้งแต่ 0 ถึง 100,000,000 และมีทศนิยมไม่เกิน 2 ตำแหน่ง',
  en: 'Invalid limit amount: enter a number from 0 to 100,000,000 with at most 2 decimal places',
} as const;

// - web/src/screens/admin/denomination-groups/i18n.ts:9,12,25,28
const DENOM_GROUP_INVALID = {
  th: 'รหัสกลุ่มไม่ถูกต้อง',
  en: 'Invalid group code',
} as const;

const DENOM_KEY_REQUIRED = {
  th: 'กรุณาระบุคีย์ป้ายเรท',
  en: 'Rate board key is required',
} as const;

async function waitCashLimitsReady(page: Page): Promise<void> {
  // web/src/screens/admin/cash-limits/CashLimits.tsx:149-211
  await expect(page.locator('[data-section="cash-limits"]')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('[data-current-limit]')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('input[data-action="no-limit"]')).toBeEnabled({ timeout: 20_000 });
}

async function loadCashLimits(page: Page, nav: () => Promise<unknown>): Promise<void> {
  // Same-URL `#cash-limits` goto is a same-document hash change (helpers.ts:344-346) that skips `readLimits` (CashLimits.tsx:105).
  if (String(nav).includes('goto') && page.url() !== 'about:blank') {
    await page.goto('about:blank');
  }
  const viewLoaded = page.waitForResponse(
    (r) =>
      r.request().method() === 'GET' &&
      new URL(r.url()).pathname === '/api/v3/views/cash_limits' &&
      r.ok(),
    { timeout: 20_000 },
  );
  await nav();
  await viewLoaded;
  await waitCashLimitsReady(page);
}

async function waitCashPolicyReady(page: Page): Promise<void> {
  // web/src/screens/admin/cash-policy/CashPolicy.tsx:173-255
  await expect(page.locator('[data-section="cash-policy"]')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('input[data-currency]').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('input[data-currency]').first()).toBeEnabled({ timeout: 20_000 });
}

async function selectFirstDenomCurrency(page: Page): Promise<string> {
  // web/src/screens/admin/denomination-groups/DenominationGroups.tsx:226-251
  await expect(page.locator('.denomination-groups-screen')).toBeVisible({ timeout: 20_000 });
  const select = page.locator('.denomination-groups-currency-picker select');
  await expect(select).toBeVisible({ timeout: 20_000 });
  const options = select.locator('option:not([value=""])');
  await expect(options.first()).toBeAttached({ timeout: 20_000 });
  const count = await options.count();
  let chosen = '';
  for (let i = 0; i < count; i++) {
    const val = (await options.nth(i).getAttribute('value')) ?? '';
    if (val === 'USD') {
      chosen = 'USD';
      break;
    }
    if (!chosen && val) chosen = val;
  }
  expect(chosen, 'at least one currency option in denomination-groups').toBeTruthy();
  await select.selectOption(chosen);
  await expect(page.locator('.add-group-form input[type="text"]')).toBeVisible({ timeout: 15_000 });
  return chosen;
}

test.describe.serial('QA-R9-5: cash limits, transfer policy, reconcile, denomination groups, reconciliation certify', () => {
  test('cashLimitsSave (PUT /api/cash-limits): happy path, reload read-back, and restore in finally', async ({
    page,
  }) => {
    await signIn(page, 'admin', adminPath('cash-limits'));
    await loadCashLimits(page, () => page.goto(adminPath('cash-limits')));

    const noLimitCheckbox = page.locator('input[data-action="no-limit"]');
    const amountInput = page.locator('#cash-limits-amount');
    const saveBtn = page.locator('button[data-action="save"]');
    const currentBanner = page.locator('[data-current-limit]');

    const origNoLimit = await noLimitCheckbox.isChecked();
    const origAmount = origNoLimit ? '' : (await amountInput.inputValue()).trim();
    const nextAmount = !origNoLimit && Number(origAmount) === 125.5 ? '250.75' : '125.50';

    try {
      if (origNoLimit) {
        await noLimitCheckbox.uncheck();
      }
      await expect(amountInput).toBeEnabled();
      await amountInput.fill(nextAmount);

      const saved = await posted(page, /^\/api\/cash-limits$/, () => saveBtn.click(), 'PUT');
      expect(saved.ok, `PUT /api/cash-limits accepted: ${saved.text}`).toBeTruthy();
      await expect(currentBanner).toContainText(nextAmount, { timeout: 15_000 });

      await loadCashLimits(page, () => page.reload());
      await expect(noLimitCheckbox).not.toBeChecked();
      expect(Number(await amountInput.inputValue())).toBe(Number(nextAmount));
      await expect(currentBanner).toContainText(nextAmount);
      await shot(page, 'admin-cash-policy', 'cash-limits-saved');
    } finally {
      await loadCashLimits(page, () => page.goto(adminPath('cash-limits')));
      if (origNoLimit) {
        if (!(await noLimitCheckbox.isChecked())) {
          await noLimitCheckbox.check();
        }
      } else {
        if (await noLimitCheckbox.isChecked()) {
          await noLimitCheckbox.uncheck();
        }
        await amountInput.fill(origAmount || '0');
      }
      const restored = await posted(page, /^\/api\/cash-limits$/, () => saveBtn.click(), 'PUT');
      expect(restored.ok, `PUT /api/cash-limits restored in finally: ${restored.text}`).toBeTruthy();
    }
  });

  test('transferPolicySave (PUT /api/cash-transfer-policy): happy path, reload read-back, and restore in finally', async ({
    page,
  }) => {
    await signIn(page, 'admin', adminPath('cash-policy'));
    await page.goto(adminPath('cash-policy'));
    await waitCashPolicyReady(page);

    const inputs = page.locator('input[data-currency]');
    const count = await inputs.count();
    expect(count, 'currency rows on /#cash-policy').toBeGreaterThan(0);

    const origLimits = new Map<string, string>();
    for (let i = 0; i < count; i++) {
      const inp = inputs.nth(i);
      const code = (await inp.getAttribute('data-currency')) ?? '';
      if (code) {
        origLimits.set(code, (await inp.inputValue()).trim());
      }
    }

    const targetCurrency = origLimits.has('THB') ? 'THB' : Array.from(origLimits.keys())[0]!;
    const origVal = origLimits.get(targetCurrency) ?? '';
    const nextVal = origVal !== '' && Number(origVal) === 50000 ? '75000.00' : '50000.00';
    const targetInput = page.locator(`input[data-currency="${targetCurrency}"]`);
    const saveBtn = page.locator('[data-section="cash-policy"] button[data-action="save"]');

    try {
      await targetInput.fill(nextVal);
      const saved = await posted(page, /^\/api\/cash-transfer-policy$/, () => saveBtn.click(), 'PUT');
      expect(saved.ok, `PUT /api/cash-transfer-policy accepted: ${saved.text}`).toBeTruthy();

      await page.reload();
      await waitCashPolicyReady(page);
      expect(Number(await page.locator(`input[data-currency="${targetCurrency}"]`).inputValue())).toBe(
        Number(nextVal),
      );
      await shot(page, 'admin-cash-policy', 'cash-policy-saved');
    } finally {
      await page.goto(adminPath('cash-policy'));
      await waitCashPolicyReady(page);
      for (const [code, val] of origLimits.entries()) {
        await page.locator(`input[data-currency="${code}"]`).fill(val);
      }
      const restored = await posted(page, /^\/api\/cash-transfer-policy$/, () => saveBtn.click(), 'PUT');
      expect(restored.ok, `PUT /api/cash-transfer-policy restored in finally: ${restored.text}`).toBeTruthy();
    }
  });

  test('denominationGroupCreate + denominationGroupMap + denominationGroupRemove: create, map board key, reload read-back, and remove', async ({
    page,
  }) => {
    await signIn(page, 'admin', adminPath('denomination-groups'));
    await page.goto(adminPath('denomination-groups'));
    const currency = await selectFirstDenomCurrency(page);

    const stamp = Date.now();
    // Group code must satisfy `/^[A-Z0-9_]{1,32}$/` (DenominationGroups.tsx:190); board key carries `qa-r9-5-<stamp>`.
    const groupCode = `QA_R9_5_${stamp}`;
    const markerKey = `qa-r9-5-${stamp}`;

    const newGroupInput = page.locator('.add-group-form input[type="text"]');
    const addBtn = page.locator('.add-group-form button');
    const row = page.locator(`.group-row[data-group="${groupCode}"]`);

    try {
      // 1. denominationGroupCreate (POST /api/denomination-groups)
      await newGroupInput.fill(groupCode);
      const created = await posted(page, /^\/api\/denomination-groups$/, () => addBtn.click());
      expect(created.ok, `POST /api/denomination-groups accepted: ${created.text}`).toBeTruthy();
      await expect(row).toBeVisible({ timeout: 15_000 });

      // 2. denominationGroupMap (POST /api/denomination-groups/map)
      const boardKeyInput = row.locator('input[type="text"]');
      const mapSaveBtn = row.locator('button').first();
      await boardKeyInput.fill(markerKey);
      const mapped = await posted(page, /^\/api\/denomination-groups\/map$/, () => mapSaveBtn.click());
      expect(mapped.ok, `POST /api/denomination-groups/map accepted: ${mapped.text}`).toBeTruthy();
      await expect(row.locator('.board-key-display')).toHaveText(markerKey, { timeout: 15_000 });

      // 3. Reload and read back both group creation and mapped key
      await page.reload();
      await expect(page.locator('.denomination-groups-screen')).toBeVisible({ timeout: 20_000 });
      await page.locator('.denomination-groups-currency-picker select').selectOption(currency);
      await expect(row).toBeVisible({ timeout: 15_000 });
      await expect(row.locator('.board-key-display')).toHaveText(markerKey);
      await expect(row.locator('input[type="text"]')).toHaveValue(markerKey);
      await shot(page, 'admin-cash-policy', 'denomination-group-mapped');

      // 4. denominationGroupRemove (POST /api/denomination-groups/remove)
      const removeBtn = row.locator('button').nth(1);
      await expect(removeBtn).toBeVisible();
      const removed = await posted(page, /^\/api\/denomination-groups\/remove$/, () => removeBtn.click());
      expect(removed.ok, `POST /api/denomination-groups/remove accepted: ${removed.text}`).toBeTruthy();
      await expect(row).toHaveCount(0);

      // 5. Reload and confirm removal persisted
      await page.reload();
      await expect(page.locator('.denomination-groups-screen')).toBeVisible({ timeout: 20_000 });
      await page.locator('.denomination-groups-currency-picker select').selectOption(currency);
      await expect(page.locator('.add-group-form input[type="text"]')).toBeVisible({ timeout: 15_000 });
      await expect(row).toHaveCount(0);
    } finally {
      // Cleanup in case the test failed between create and remove
      await page.goto(adminPath('denomination-groups')).catch(() => {});
      const select = page.locator('.denomination-groups-currency-picker select');
      if (await select.isVisible().catch(() => false)) {
        await select.selectOption(currency).catch(() => {});
        if ((await row.count()) > 0) {
          const removeBtn = row.locator('button').nth(1);
          if (await removeBtn.isVisible().catch(() => false)) {
            await posted(page, /^\/api\/denomination-groups\/remove$/, () => removeBtn.click()).catch(() => {});
          }
        }
      }
    }
  });

  test('transferReconcile (POST /api/cash-transfers/{id}/reconcile): happy path and reload read-back when a RECEIVED row exists', async ({
    page,
  }) => {
    const role = hasAccount('finance') ? 'finance' : 'admin';
    await signIn(page, role, adminPath('cash'));
    await page.goto(adminPath('cash'));

    // web/src/screens/admin/cash/CashFlows.tsx:412,497,522,561
    await expect(page.locator('.cash-flows')).toBeVisible({ timeout: 20_000 });
    const transfersSection = page.locator('[data-section="transfers"]');
    await expect(transfersSection).toBeVisible({ timeout: 20_000 });

    const reconcileBtn = transfersSection.locator('button[data-action="reconcile"]').first();
    if ((await reconcileBtn.count()) === 0) {
      console.log('[qa-r9-5] transferReconcile: skipping — no RECEIVED transfer row on host');
      test.skip(true, 'no RECEIVED transfer row on host to reconcile');
      return;
    }

    // The row carries no id; the reconcile button does (CashFlows.tsx:561 data-id).
    const transferId = (await reconcileBtn.getAttribute('data-id')) ?? '';
    expect(transferId, 'data-id on the RECEIVED row reconcile button').toBeTruthy();

    await reconcileBtn.click();
    // web/src/screens/admin/cash/CashFlows.tsx:342-369
    const dialog = page.locator('form.cash-transfer-dialog');
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    const marker = `qa-r9-5-${Date.now()}`;
    await dialog.locator('#reconcile-note').fill(marker);

    const reconciled = await posted(page, /\/api\/cash-transfers\/[^/]+\/reconcile$/, () =>
      dialog.locator('button[data-action="confirm"]').click(),
    );
    expect(reconciled.ok, `POST /api/cash-transfers/${transferId}/reconcile accepted: ${reconciled.text}`).toBeTruthy();

    await page.reload();
    await expect(transfersSection).toBeVisible({ timeout: 20_000 });
    await expect(transfersSection.locator('tbody tr').first()).toBeVisible({ timeout: 20_000 });
    // Only a RECEIVED row offers reconcile (CashFlows.tsx:559), so its button gone = the state moved.
    await expect(transfersSection.locator(`button[data-action="reconcile"][data-id="${transferId}"]`)).toHaveCount(0);
    await shot(page, 'admin-cash-policy', 'transfer-reconciled');
  });

  test('reconciliationCertify (POST /api/v3/commands/reconciliation_certify): happy path and reload read-back', async ({
    page,
  }) => {
    const role = hasAccount('finance') ? 'finance' : 'admin';
    await signIn(page, role, adminPath('reconciliation'));
    await page.goto(adminPath('reconciliation'));

    // web/src/screens/admin/reconciliation/Reconciliation.tsx:46,124,221-243
    await expect(page.locator('.reconciliation')).toBeVisible({ timeout: 20_000 });
    const certifySection = page.locator('section.reconciliation-certify');
    await expect(certifySection).toBeVisible({ timeout: 20_000 });

    const marker = `qa-r9-5-${Date.now()}`;
    const noteInput = certifySection.locator('input[type="text"]');
    const certifyBtn = certifySection.locator('button[type="button"]');
    await noteInput.fill(marker);

    const certified = await posted(page, /^\/api\/v3\/commands\/reconciliation_certify$/, () => certifyBtn.click());
    if (!certified.ok && certified.body.includes('samePerson')) {
      console.log(`[qa-r9-5] reconciliationCertify: skipping — ${role} verified a count today (409 samePerson)`);
      test.skip(true, `${role} verified a count today (409 samePerson)`);
      return;
    }
    expect(certified.ok, `POST /api/v3/commands/reconciliation_certify accepted: ${certified.text}`).toBeTruthy();

    const certItem = certifySection.locator('li[data-certification]').filter({ hasText: marker }).first();
    await expect(certItem).toBeVisible({ timeout: 15_000 });

    await page.reload();
    await expect(certifySection).toBeVisible({ timeout: 20_000 });
    await expect(certifySection.locator('li[data-certification]').filter({ hasText: marker }).first()).toBeVisible({
      timeout: 15_000,
    });
    await shot(page, 'admin-cash-policy', 'reconciliation-certified');
  });

  test('invalid input refusals in th and en: negative limit, too many decimals, empty/malformed group name, empty board key', async ({
    page,
  }) => {
    // 1 & 2. cashLimitsSave (CashLimits.tsx:31-37, 134-137, 189): negative limit and >2 decimal places
    await signIn(page, 'admin', adminPath('cash-limits'));
    await loadCashLimits(page, () => page.goto(adminPath('cash-limits')));

    for (const lang of ['th', 'en'] as const) {
      await withLang(page, lang);
      await loadCashLimits(page, () => page.reload());
      const noLimitCheckbox = page.locator('input[data-action="no-limit"]');
      if (await noLimitCheckbox.isChecked()) {
        await noLimitCheckbox.uncheck();
      }
      const amountInput = page.locator('#cash-limits-amount');
      const saveBtn = page.locator('button[data-action="save"]');
      const fieldError = page.locator('[data-field-error="countDifferenceLimitThb"]');

      // Case 1: negative limit
      await amountInput.fill('-10');
      await saveBtn.click();
      await expect(fieldError).toBeVisible();
      await expect(fieldError).toHaveText(CASH_LIMITS_ERR[lang]);
      await expect(amountInput).toHaveValue('-10');

      // Case 2: too many decimals (>2 dp)
      await amountInput.fill('12.345');
      await saveBtn.click();
      await expect(fieldError).toBeVisible();
      await expect(fieldError).toHaveText(CASH_LIMITS_ERR[lang]);
      await expect(amountInput).toHaveValue('12.345');
    }

    // 3. transferPolicySave (CashPolicy.tsx:46-52, 137-149, 214): negative limit and >2 decimal places
    await page.goto(adminPath('cash-policy'));
    await waitCashPolicyReady(page);

    for (const lang of ['th', 'en'] as const) {
      await withLang(page, lang);
      await page.reload();
      await waitCashPolicyReady(page);
      const firstInput = page.locator('input[data-currency]').first();
      const saveBtn = page.locator('[data-section="cash-policy"] button[data-action="save"]');
      const alert = page.locator('[data-section="cash-policy"] [role="alert"].error-message');

      await firstInput.fill('-5');
      await saveBtn.click();
      await expect(alert).toBeVisible();
      await expect(alert).toHaveText(CASH_POLICY_ERR[lang]);
      await expect(firstInput).toHaveValue('-5');

      await firstInput.fill('10.123');
      await saveBtn.click();
      await expect(alert).toBeVisible();
      await expect(alert).toHaveText(CASH_POLICY_ERR[lang]);
      await expect(firstInput).toHaveValue('10.123');
    }

    // 4 & 5. denominationGroupCreate & denominationGroupMap (DenominationGroups.tsx:38-45, 188-195):
    // Note: denomination_group_service.rs:53-56 treats an existing valid group code as idempotent Ok(()),
    // while DenominationGroups.tsx refuses empty/malformed group codes (`groupInvalid`) and empty board keys (`keyRequired`).
    await page.goto(adminPath('denomination-groups'));
    await selectFirstDenomCurrency(page);

    for (const lang of ['th', 'en'] as const) {
      await withLang(page, lang);
      await page.reload();
      await selectFirstDenomCurrency(page);
      const newGroupInput = page.locator('.add-group-form input[type="text"]');
      const addBtn = page.locator('.add-group-form button');
      const addError = page.locator('.add-group-form .error-msg');

      // Case 4a: empty group name
      await newGroupInput.fill('');
      await addBtn.click();
      await expect(addError).toBeVisible();
      await expect(addError).toHaveText(DENOM_GROUP_INVALID[lang]);
      await expect(newGroupInput).toHaveValue('');

      // Case 4b: malformed group name (lowercase/hyphens rejected by /^[A-Z0-9_]{1,32}$/)
      await newGroupInput.fill('qa-r9-5-invalid');
      await addBtn.click();
      await expect(addError).toBeVisible();
      await expect(addError).toHaveText(DENOM_GROUP_INVALID[lang]);
      await expect(newGroupInput).toHaveValue('qa-r9-5-invalid');

      // Case 5: empty rate-board key on an existing group row
      const firstRow = page.locator('.group-row').first();
      await expect(firstRow).toBeVisible();
      const boardKeyInput = firstRow.locator('input[type="text"]');
      const origKey = await boardKeyInput.inputValue();
      await boardKeyInput.fill('');
      await firstRow.locator('button').first().click();
      const rowError = firstRow.locator('.row-error');
      await expect(rowError).toBeVisible();
      await expect(rowError).toHaveText(DENOM_KEY_REQUIRED[lang]);
      await expect(boardKeyInput).toHaveValue('');
      await boardKeyInput.fill(origKey);
    }
  });

  test('permission: teller is not offered cash-limits, cash-policy, denomination-groups, reconciliation, or transferReconcile', async ({
    page,
  }) => {
    if (!hasAccount('teller')) {
      console.log('[qa-r9-5] skipping teller permission check: no teller account in users.json');
      test.skip(true, 'no teller account in users.json: run scripts/reseed.sh');
      return;
    }

    await signIn(page, 'teller', '/');
    const perms = await mePermissions(page);
    expect(perms).not.toContain('CashLimitsManage');
    expect(perms).not.toContain('CashTransferPolicySet');
    expect(perms).not.toContain('RatesDraft');
    expect(perms).not.toContain('FinanceReconcile');

    // Cross-ref smoke-rbac-matrix.spec.ts:35-66:
    // - #denomination-groups (order 13, needs RatesDraft)
    // - #cash-policy (order 18, needs CashTransferPolicySet)
    // - #reconciliation (order 19, needs FinanceReconcile)
    // - #cash-limits (order 28, needs CashLimitsManage)
    for (const refused of ['cash-limits', 'cash-policy', 'denomination-groups', 'reconciliation']) {
      await expect(page.locator(`nav.admin-nav a[href="#${refused}"]`)).toHaveCount(0);
    }

    // Navigating to `/#cash`: teller lacks CashManage and FinanceReconcile (CashFlows.tsx:35-36, 496, 557),
    // so neither the transfers section nor the reconcile action is offered.
    await page.goto('/#cash');
    await expect(page.locator('.cash-flows')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('[data-section="transfers"]')).toHaveCount(0);
    await expect(page.locator('button[data-action="reconcile"]')).toHaveCount(0);

    // Cross-ref smoke-rbac-matrix.spec.ts:82-83 & TransfersCard.tsx:418,617:
    // On /staff, teller lacks CashManage and FinanceReconcile, so [data-card="transfers"] and [data-testid="transfer-reconcile"] are absent.
    await page.goto('/staff');
    await expect(page.getByTestId('staff-hours')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('[data-card="transfers"]')).toHaveCount(0);
    await expect(page.getByTestId('transfer-reconcile')).toHaveCount(0);
  });
});
