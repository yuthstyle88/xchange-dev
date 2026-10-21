// QA-R9-1 — flow: admin user management and permission groups (design docs/design/qa-e2e.md §9.1; ticket #4285).
// Written by ticket #4285 (QA-R9-1); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Browser, type Locator, type Page } from '@playwright/test';
import { hasAccount, shot, signIn, STAFF_LANGS, users, withLang } from './helpers';

// Raw error-code camelCase / UPPER_SNAKE tokens that must never surface as user-facing error text (§9.4 (b)).
const RAW_CODE =
  /^(validationFailed|codeTaken|employeeCodeTaken|outOfScope|roleNotAllowed|userNotActive|statusNotAllowed|selfChange|lastOwner|holdsCash|notBound|invalidRequest|serverError|notFound|guardrail:[123]|ORIGIN_REJECTED)$/;

let cachedMarketingUserId: string | undefined;

async function getMarketingUserId(browser: Browser): Promise<string> {
  if (cachedMarketingUserId) return cachedMarketingUserId;
  const context = await browser.newContext({ baseURL: users.baseUrl });
  const page = await context.newPage();
  try {
    await signIn(page, 'marketing', '/#campaigns');
    const res = await page.request.get('/api/v3/views/me');
    expect(res.ok(), `GET /api/v3/views/me returned ${res.status()}`).toBe(true);
    const body = (await res.json()) as { data?: { userId?: string }; userId?: string };
    const id = body.data?.userId ?? body.userId;
    expect(id, 'marketing /api/v3/views/me must return userId').toBeTruthy();
    cachedMarketingUserId = id!;
    return id!;
  } finally {
    await context.close().catch(() => {});
  }
}

// Locate the target user's <tr> in .users-table tbody tr by .users-cell-id (UsersScreen.tsx:601,615-618).
function userRowById(screen: Locator, userId: string): Locator {
  return screen
    .locator('.users-table tbody tr')
    .filter({ has: screen.page().locator('.users-cell-id', { hasText: userId }) });
}

// Active (non-ended) role tags on a user row (UsersScreen.tsx:636-644).
function activeRoleTag(row: Locator, roleText: RegExp): Locator {
  return row.locator('.users-role-tag:not(.users-role-ended)').filter({ hasText: roleText });
}

// Role label for VERIFICATION_REVIEWER in th ("ผู้ตรวจเอกสาร") or en ("Document reviewer") (UsersScreen.tsx:52, i18n.ts:75,242).
const REVIEWER_ROLE_LABEL = /ผู้ตรวจเอกสาร|Document reviewer/;

async function restoreMarketingUser(
  page: Page,
  targetUserId: string,
  originalProfile?: {
    employeeCode: string;
    department: string;
    position: string;
    tellCode: string;
  },
): Promise<void> {
  try {
    await page.goto('/#users');
    const screen = page.locator('.users-screen'); // UsersScreen.tsx:522
    await expect(screen).toBeVisible({ timeout: 20_000 });
    const row = userRowById(screen, targetUserId);
    if ((await row.count()) === 0) return;

    // Restore suspended → resume (UsersScreen.tsx:729) or disabled → enable (UsersScreen.tsx:741).
    const resumeBtn = row.locator('button[data-users-action="resume"]');
    if (await resumeBtn.isVisible().catch(() => false)) {
      await resumeBtn.click();
      await expect(row.locator('.users-status-badge.users-status-active')).toBeVisible({ timeout: 15_000 });
    }
    const enableBtn = row.locator('button[data-users-action="enable"]');
    if (await enableBtn.isVisible().catch(() => false)) {
      await enableBtn.click();
      await expect(row.locator('.users-status-badge.users-status-active')).toBeVisible({ timeout: 15_000 });
    }

    // Restore granted role → revoke (UsersScreen.tsx:655).
    const reviewerTag = activeRoleTag(row, REVIEWER_ROLE_LABEL);
    if ((await reviewerTag.count()) > 0) {
      await reviewerTag.first().locator('button.users-btn-revoke').click();
      await expect(reviewerTag).toHaveCount(0, { timeout: 15_000 });
    }

    // Restore employee profile fields if captured (UsersScreen.tsx:690,1033-1083).
    if (originalProfile) {
      const profileBtn = row.locator('button[data-users-action="profile"]');
      if (await profileBtn.isVisible().catch(() => false)) {
        await profileBtn.click();
        const modal = screen.locator('.users-modal[role="dialog"]'); // UsersScreen.tsx:1016
        await expect(modal).toBeVisible({ timeout: 10_000 });
        await modal.locator('input#profile-emp-code[name="employeeCode"]').fill(originalProfile.employeeCode);
        await modal.locator('input#profile-dept[name="department"]').fill(originalProfile.department);
        await modal.locator('input#profile-pos[name="position"]').fill(originalProfile.position);
        await modal.locator('input#profile-tell[name="tellCode"]').fill(originalProfile.tellCode);
        await modal.locator('button[data-users-action="profile-save"]').click();
        await expect(modal).toHaveCount(0, { timeout: 15_000 });
      }
    }
  } catch {
    // Best-effort cleanup in finally.
  }
}

async function cleanupMatrixGroup(page: Page, groupCode: string): Promise<void> {
  try {
    await page.goto('/#users');
    const screen = page.locator('.users-screen'); // UsersScreen.tsx:522
    await expect(screen).toBeVisible({ timeout: 20_000 });
    await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
    const panel = screen.locator('.matrix-panel'); // PermissionMatrix.tsx:221
    await expect(panel).toBeVisible({ timeout: 15_000 });
    const groupRow = panel.locator(`tr[data-matrix-group="${groupCode}"]`); // PermissionMatrix.tsx:267
    if ((await groupRow.count()) > 0) {
      page.once('dialog', (d) => d.accept()); // PermissionMatrix.tsx:199
      await groupRow.locator(`button[data-matrix-delete="${groupCode}"]`).click(); // PermissionMatrix.tsx:303
      await expect(groupRow).toHaveCount(0, { timeout: 15_000 });
    }
  } catch {
    // Best-effort cleanup in finally.
  }
}

test.describe('QA-R9-1 admin user management and permission groups', () => {
  test('(a) happy path: userProfileSave, userGrant, userRevoke, userSuspend, userResume, userDisable, userEnable persist across reload', async ({
    page,
    browser,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    test.skip(!hasAccount('marketing'), 'no marketing account in users.json: run scripts/reseed.sh');

    const targetUserId = await getMarketingUserId(browser);
    const ts = Date.now();
    const marker = `qa-r9-1-${ts}`;
    const empCode = `EMP-${ts}`;
    const dept = `${marker}-dept`;
    const pos = `${marker}-pos`;
    const tellCode = `T${String(ts).slice(-6)}`;

    let originalProfile:
      | { employeeCode: string; department: string; position: string; tellCode: string }
      | undefined;

    try {
      await signIn(page, 'admin', '/#users');
      const screen = page.locator('.users-screen'); // UsersScreen.tsx:522
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const row = userRowById(screen, targetUserId);
      await expect(row, 'seeded marketing user row must be visible on /#users').toHaveCount(1, {
        timeout: 20_000,
      });

      // 1. userProfileSave (POST /api/users/profile, UsersScreen.tsx:90,414,690,1016-1086)
      await row.locator('button[data-users-action="profile"]').click(); // UsersScreen.tsx:690
      const profileModal = screen.locator('.users-modal[role="dialog"]'); // UsersScreen.tsx:1016
      await expect(profileModal).toBeVisible({ timeout: 10_000 });

      const empInput = profileModal.locator('input#profile-emp-code[name="employeeCode"]'); // UsersScreen.tsx:1033-1035
      const deptInput = profileModal.locator('input#profile-dept[name="department"]'); // UsersScreen.tsx:1043-1045
      const posInput = profileModal.locator('input#profile-pos[name="position"]'); // UsersScreen.tsx:1053-1055
      const tellInput = profileModal.locator('input#profile-tell[name="tellCode"]'); // UsersScreen.tsx:1063-1065

      originalProfile = {
        employeeCode: await empInput.inputValue(),
        department: await deptInput.inputValue(),
        position: await posInput.inputValue(),
        tellCode: await tellInput.inputValue(),
      };

      await empInput.fill(empCode);
      await deptInput.fill(dept);
      await posInput.fill(pos);
      await tellInput.fill(tellCode);
      await profileModal.locator('button[data-users-action="profile-save"]').click(); // UsersScreen.tsx:1083
      await expect(profileModal).toHaveCount(0, { timeout: 15_000 });

      // Reload and read back profile fields from the table row and modal (UsersScreen.tsx:620-625).
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterProfile = userRowById(screen, targetUserId);
      await expect(rowAfterProfile).toHaveCount(1, { timeout: 20_000 });
      await expect(rowAfterProfile.locator('td').nth(1)).toHaveText(empCode);
      await expect(rowAfterProfile.locator('td').nth(2)).toHaveText(`${dept} · ${pos}`);
      await expect(rowAfterProfile.locator('td').nth(3)).toHaveText(tellCode);

      // 2. userGrant (POST /api/users/grant, UsersScreen.tsx:84,371,701,924-1010)
      await rowAfterProfile.locator('button.users-btn-grant').click(); // UsersScreen.tsx:701
      const grantModal = screen.locator('.users-modal[role="dialog"]'); // UsersScreen.tsx:924
      await expect(grantModal).toBeVisible({ timeout: 10_000 });
      await grantModal.locator('select#grant-role').selectOption('VERIFICATION_REVIEWER'); // UsersScreen.tsx:940
      const branchSelect = grantModal.locator('select#grant-branch'); // UsersScreen.tsx:959
      await expect(branchSelect).toBeVisible();
      await grantModal.locator('button[type="submit"].users-btn-primary').click(); // UsersScreen.tsx:1005
      await expect(grantModal).toHaveCount(0, { timeout: 15_000 });

      // Reload and read back the granted role tag (UsersScreen.tsx:636-644).
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterGrant = userRowById(screen, targetUserId);
      await expect(activeRoleTag(rowAfterGrant, REVIEWER_ROLE_LABEL)).toHaveCount(1, {
        timeout: 15_000,
      });
      await shot(page, 'admin-users', 'r9-1-user-profile-and-grant');

      // 3. userRevoke (POST /api/users/revoke, UsersScreen.tsx:85,438,655)
      await activeRoleTag(rowAfterGrant, REVIEWER_ROLE_LABEL)
        .first()
        .locator('button.users-btn-revoke') // UsersScreen.tsx:655
        .click();
      await expect(activeRoleTag(rowAfterGrant, REVIEWER_ROLE_LABEL)).toHaveCount(0, {
        timeout: 15_000,
      });

      // Reload and confirm the active role tag remains revoked.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterRevoke = userRowById(screen, targetUserId);
      await expect(activeRoleTag(rowAfterRevoke, REVIEWER_ROLE_LABEL)).toHaveCount(0, {
        timeout: 15_000,
      });

      // 4. userSuspend (POST /api/users/suspend, UsersScreen.tsx:87,484,709)
      await rowAfterRevoke.locator('button[data-users-action="suspend"]').click(); // UsersScreen.tsx:709
      await expect(
        rowAfterRevoke.locator('.users-status-badge.users-status-suspended'), // UsersScreen.tsx:499,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify SUSPENDED persists.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterSuspend = userRowById(screen, targetUserId);
      await expect(
        rowAfterSuspend.locator('.users-status-badge.users-status-suspended'),
      ).toBeVisible({ timeout: 15_000 });

      // 5. userResume (POST /api/users/resume, UsersScreen.tsx:88,486,729)
      await rowAfterSuspend.locator('button[data-users-action="resume"]').click(); // UsersScreen.tsx:729
      await expect(
        rowAfterSuspend.locator('.users-status-badge.users-status-active'), // UsersScreen.tsx:495,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify ACTIVE persists after resume.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterResume = userRowById(screen, targetUserId);
      await expect(
        rowAfterResume.locator('.users-status-badge.users-status-active'),
      ).toBeVisible({ timeout: 15_000 });

      // 6. userDisable (POST /api/users/disable, UsersScreen.tsx:86,482,717)
      await rowAfterResume.locator('button.users-btn-disable').click(); // UsersScreen.tsx:717
      await expect(
        rowAfterResume.locator('.users-status-badge.users-status-disabled'), // UsersScreen.tsx:501,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify DISABLED persists.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterDisable = userRowById(screen, targetUserId);
      await expect(
        rowAfterDisable.locator('.users-status-badge.users-status-disabled'),
      ).toBeVisible({ timeout: 15_000 });

      // 7. userEnable (POST /api/users/enable, UsersScreen.tsx:89,488,741)
      await rowAfterDisable.locator('button[data-users-action="enable"]').click(); // UsersScreen.tsx:741
      await expect(
        rowAfterDisable.locator('.users-status-badge.users-status-active'),
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify ACTIVE persists after enable.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterEnable = userRowById(screen, targetUserId);
      await expect(
        rowAfterEnable.locator('.users-status-badge.users-status-active'),
      ).toBeVisible({ timeout: 15_000 });
      await shot(page, 'admin-users', 'r9-1-user-status-restored');
    } finally {
      await restoreMarketingUser(page, targetUserId, originalProfile);
    }
  });

  test('(a) happy path: adminGroupAdd, adminGroupSave, adminGroupDelete persist across reload', async ({
    page,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');

    const ts = Date.now();
    const groupName = `qa-r9-1-${ts}`;
    const groupCode = `QA_R9_1_${ts}`;

    try {
      await signIn(page, 'admin', '/#users');
      const screen = page.locator('.users-screen'); // UsersScreen.tsx:522
      await expect(screen).toBeVisible({ timeout: 20_000 });

      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      const panel = screen.locator('.matrix-panel'); // PermissionMatrix.tsx:221
      await expect(panel).toBeVisible({ timeout: 15_000 });

      // 1. adminGroupAdd (POST /api/admin/groups/add, PermissionMatrix.tsx:30,185,228,364-441)
      await panel.locator('button[data-matrix-add-open]').click(); // PermissionMatrix.tsx:228
      const addModal = panel.locator('.users-modal[role="dialog"][data-matrix-add-modal]'); // PermissionMatrix.tsx:364
      await expect(addModal).toBeVisible({ timeout: 10_000 });

      await addModal.locator('input#matrix-group-code[data-matrix-add-code]').fill(groupCode); // PermissionMatrix.tsx:371-373
      await addModal.locator('input#matrix-group-name[data-matrix-add-name]').fill(groupName); // PermissionMatrix.tsx:381-383
      await addModal.locator('select#matrix-group-base[data-matrix-add-base]').selectOption('TELLER'); // PermissionMatrix.tsx:390-392
      await addModal.locator('button[type="submit"][data-matrix-add-submit]').click(); // PermissionMatrix.tsx:433
      await expect(addModal).toHaveCount(0, { timeout: 15_000 });

      const groupRow = panel.locator(`tr[data-matrix-group="${groupCode}"]`); // PermissionMatrix.tsx:267
      await expect(groupRow).toBeVisible({ timeout: 15_000 });
      await expect(groupRow).toContainText(groupName);

      // Reload and verify the new group row persists on the Permission Matrix tab.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      await expect(groupRow).toBeVisible({ timeout: 15_000 });
      await expect(groupRow).toContainText(groupName);

      // 2. adminGroupSave (POST /api/admin/groups/save, PermissionMatrix.tsx:31,171,293,315-360)
      await groupRow.locator(`button[data-matrix-edit="${groupCode}"]`).click(); // PermissionMatrix.tsx:293
      const editor = panel.locator(`.matrix-editor[data-matrix-editor="${groupCode}"]`); // PermissionMatrix.tsx:315-318
      await expect(editor).toBeVisible({ timeout: 10_000 });

      const reportsPerm = editor.locator('input[type="checkbox"][data-matrix-perm="MarketingPerformance"]'); // PermissionMatrix.tsx:332
      await expect(reportsPerm).not.toBeChecked();
      await reportsPerm.check();
      await editor.locator('button[data-matrix-save]').click(); // PermissionMatrix.tsx:354
      await expect(editor).toHaveCount(0, { timeout: 15_000 });

      // Reload and verify the saved permission level (REPORTS = VIEW) and checkbox state persist (PermissionMatrix.tsx:280).
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      await expect(groupRow).toBeVisible({ timeout: 15_000 });
      const reportsCell = groupRow.locator(`td[data-matrix-level="${groupCode}:REPORTS"]`); // PermissionMatrix.tsx:280
      await expect(reportsCell).toContainText(/ดูอย่างเดียว|View only/); // PermissionMatrix.tsx:38-45, i18n.ts:95,262

      await groupRow.locator(`button[data-matrix-edit="${groupCode}"]`).click(); // PermissionMatrix.tsx:293
      await expect(editor).toBeVisible({ timeout: 10_000 });
      await expect(
        editor.locator('input[type="checkbox"][data-matrix-perm="MarketingPerformance"]'),
      ).toBeChecked();
      await shot(page, 'admin-users', 'r9-1-matrix-group-saved');

      // 3. adminGroupDelete (POST /api/admin/groups/delete, PermissionMatrix.tsx:32,202,303)
      page.once('dialog', (d) => d.accept()); // PermissionMatrix.tsx:199
      await groupRow.locator(`button[data-matrix-delete="${groupCode}"]`).click(); // PermissionMatrix.tsx:303
      await expect(groupRow).toHaveCount(0, { timeout: 15_000 });

      // Reload and verify the deleted group remains absent.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      await expect(panel).toBeVisible({ timeout: 15_000 });
      await expect(groupRow).toHaveCount(0);
    } finally {
      await cleanupMatrixGroup(page, groupCode);
    }
  });

  test('(b) invalid inputs are refused with worded messages in th and en and form state is preserved', async ({
    page,
    browser,
  }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    test.skip(!hasAccount('marketing'), 'no marketing account in users.json: run scripts/reseed.sh');

    const targetUserId = await getMarketingUserId(browser);
    const ts = Date.now();
    const markerName = `qa-r9-1-inv-${ts}`;
    const validCode = `QA_R9_1_INV_${ts}`;

    await signIn(page, 'admin', '/#users');
    const screen = page.locator('.users-screen'); // UsersScreen.tsx:522
    await expect(screen).toBeVisible({ timeout: 20_000 });

    for (const lang of STAFF_LANGS) {
      await withLang(page, lang);
      await page.goto('/#users');
      await expect(screen).toBeVisible({ timeout: 20_000 });

      // Case 1: userGrant with validTo earlier than validFrom (user_grant.rs:79-84, UsersScreen.tsx:934,975,988).
      const row = userRowById(screen, targetUserId);
      await expect(row).toHaveCount(1, { timeout: 20_000 });
      await row.locator('button.users-btn-grant').click(); // UsersScreen.tsx:701
      const grantModal = screen.locator('.users-modal[role="dialog"]'); // UsersScreen.tsx:924
      await expect(grantModal).toBeVisible({ timeout: 10_000 });

      const fromInput = grantModal.locator('input#grant-from[type="date"]'); // UsersScreen.tsx:975
      const toInput = grantModal.locator('input#grant-to[type="date"]'); // UsersScreen.tsx:988
      await fromInput.fill('2026-12-10');
      await toInput.fill('2026-12-01');
      await grantModal.locator('button[type="submit"].users-btn-primary').click(); // UsersScreen.tsx:1005

      const grantErr = grantModal.locator('[role="alert"].users-form-error'); // UsersScreen.tsx:934
      await expect(grantErr, `[${lang}] invalid grant date range must show alert`).toBeVisible({
        timeout: 15_000,
      });
      const grantErrText = (await grantErr.innerText()).trim();
      expect(grantErrText.length).toBeGreaterThan(0);
      expect(grantErrText).not.toMatch(RAW_CODE);
      await expect(fromInput).toHaveValue('2026-12-10');
      await expect(toInput).toHaveValue('2026-12-01');
      await grantModal.locator('button[type="button"].users-btn-cancel').click(); // UsersScreen.tsx:998
      await expect(grantModal).toHaveCount(0, { timeout: 10_000 });

      // Switch to Permission Matrix tab for Cases 2–4 (UsersScreen.tsx:543, PermissionMatrix.tsx:221).
      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      const panel = screen.locator('.matrix-panel'); // PermissionMatrix.tsx:221
      await expect(panel).toBeVisible({ timeout: 15_000 });

      await panel.locator('button[data-matrix-add-open]').click(); // PermissionMatrix.tsx:228
      const addModal = panel.locator('.users-modal[role="dialog"][data-matrix-add-modal]'); // PermissionMatrix.tsx:364
      await expect(addModal).toBeVisible({ timeout: 10_000 });

      const codeInput = addModal.locator('input#matrix-group-code[data-matrix-add-code]'); // PermissionMatrix.tsx:371-373
      const nameInput = addModal.locator('input#matrix-group-name[data-matrix-add-name]'); // PermissionMatrix.tsx:381-383
      const baseSelect = addModal.locator('select#matrix-group-base[data-matrix-add-base]'); // PermissionMatrix.tsx:390-392
      const submitBtn = addModal.locator('button[type="submit"][data-matrix-add-submit]'); // PermissionMatrix.tsx:433
      const matrixErr = panel.locator('div[role="alert"][data-matrix-error]'); // PermissionMatrix.tsx:238

      // Case 2: Guardrail 1 breach — non-admin base (TELLER) with UsersManage (PermissionMatrix.tsx:53-55,179, i18n.ts:124,291).
      await codeInput.fill(validCode);
      await nameInput.fill(markerName);
      await baseSelect.selectOption('TELLER');
      const usersManageCb = addModal.locator('input[type="checkbox"][data-matrix-add-perm="UsersManage"]'); // PermissionMatrix.tsx:412
      await usersManageCb.check();
      await submitBtn.click();

      await expect(matrixErr, `[${lang}] Guardrail 1 refusal must show alert`).toBeVisible({
        timeout: 10_000,
      });
      const g1Text = (await matrixErr.innerText()).trim();
      expect(g1Text).not.toMatch(RAW_CODE);
      if (lang === 'th') {
        expect(g1Text).toMatch(/[\u0E00-\u0E7F]/);
      } else {
        expect(g1Text).toContain('Non-admin groups cannot hold Create or Full');
      }
      await expect(codeInput).toHaveValue(validCode);
      await expect(nameInput).toHaveValue(markerName);
      await expect(usersManageCb).toBeChecked();

      // Case 3: Guardrail 3 breach — non-admin/exec base (TELLER) with SystemSettings (PermissionMatrix.tsx:58-60, i18n.ts:128,295).
      await usersManageCb.uncheck();
      const sysSettingsCb = addModal.locator('input[type="checkbox"][data-matrix-add-perm="SystemSettings"]'); // PermissionMatrix.tsx:412
      await sysSettingsCb.check();
      await submitBtn.click();

      await expect(matrixErr, `[${lang}] Guardrail 3 refusal must show alert`).toBeVisible({
        timeout: 10_000,
      });
      const g3Text = (await matrixErr.innerText()).trim();
      expect(g3Text).not.toMatch(RAW_CODE);
      if (lang === 'th') {
        expect(g3Text).toMatch(/[\u0E00-\u0E7F]/);
      } else {
        expect(g3Text).toContain('Only Executive or Admin groups can hold System Settings');
      }
      await expect(codeInput).toHaveValue(validCode);
      await expect(nameInput).toHaveValue(markerName);
      await expect(sysSettingsCb).toBeChecked();

      // Case 4: Invalid group code ('A-1' contains '-') refused by backend validate_code (admin_groups.rs:253-262).
      await sysSettingsCb.uncheck();
      await codeInput.fill('A-1');
      await submitBtn.click();

      await expect(matrixErr, `[${lang}] invalid group code must show alert`).toBeVisible({
        timeout: 15_000,
      });
      const codeErrText = (await matrixErr.innerText()).trim();
      expect(codeErrText.length).toBeGreaterThan(0);
      expect(codeErrText).not.toMatch(RAW_CODE);
      await expect(codeInput).toHaveValue('A-1');
      await expect(nameInput).toHaveValue(markerName);

      await addModal.locator('button[type="button"].users-btn-cancel').click(); // PermissionMatrix.tsx:425
      await expect(addModal).toHaveCount(0, { timeout: 10_000 });
    }
    await shot(page, 'admin-users', 'r9-1-invalid-inputs');
  });

  test('(c) permission: branchManager cannot access /#users or execute user/group write commands', async ({
    page,
    browser,
  }) => {
    test.skip(
      !hasAccount('branchManager'),
      'no branchManager account in users.json: run scripts/reseed.sh',
    );
    test.skip(!hasAccount('marketing'), 'no marketing account in users.json: run scripts/reseed.sh');

    const targetUserId = await getMarketingUserId(browser);
    await signIn(page, 'branchManager', '/#users');

    // UI gate: nav link a[href="#users"] requires UsersManage (menu.ts:10-12) and .users-screen (UsersScreen.tsx:522) is not rendered.
    await expect(page.locator('nav.admin-nav')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('nav.admin-nav a[href="#users"]')).toHaveCount(0);
    await expect(page.locator('.users-screen')).toHaveCount(0);

    // Backend gate: direct POSTs to /api/users/grant and /api/admin/groups/add as branchManager are refused with 403.
    const grantRes = await page.request.post('/api/users/grant', {
      headers: { Origin: users.baseUrl },
      data: {
        idempotencyKey: `qa-r9-1-bm-grant-${Date.now()}`,
        userId: targetUserId,
        version: 1,
        role: 'TELLER',
        branch: 'MAIN',
      },
    });
    expect(grantRes.status(), 'POST /api/users/grant as branchManager must be 403').toBe(403);

    const groupRes = await page.request.post('/api/admin/groups/add', {
      headers: { Origin: users.baseUrl },
      data: {
        idempotencyKey: `qa-r9-1-bm-group-${Date.now()}`,
        version: 1,
        code: `QA_R9_1_BM_${Date.now()}`,
        name: `qa-r9-1-bm-${Date.now()}`,
        base: 'TELLER',
        permissions: [],
      },
    });
    expect(groupRes.status(), 'POST /api/admin/groups/add as branchManager must be 403').toBe(403);
    await shot(page, 'admin-users', 'r9-1-branch-manager-refused');
  });
});
