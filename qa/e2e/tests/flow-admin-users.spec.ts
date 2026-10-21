// QA-R9-1 — flow: admin user management and permission groups (design docs/design/qa-e2e.md §9.1; ticket #4285).
// Written by ticket #4285 (QA-R9-1); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Browser, type Locator, type Page } from '@playwright/test';
import { hasAccount, shot, signIn, STAFF_LANGS, users, withLang } from './helpers';

// Raw error-code camelCase / UPPER_SNAKE tokens that must never surface as user-facing error text (§9.4 (b)).
const RAW_CODE =
  /^(validationFailed|codeTaken|employeeCodeTaken|outOfScope|roleNotAllowed|userNotActive|statusNotAllowed|selfChange|lastOwner|holdsCash|notBound|invalidRequest|serverError|notFound|builtIn|inUse|guardrail:[1-5]|ORIGIN_REJECTED)$/;

const REVIEWER_ROLE_CODE = 'VERIFICATION_REVIEWER';

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

// Locate tr[data-user-id="<id>"] (UsersScreen.tsx:624), paging with button[data-action="more"] (UsersScreen.tsx:762) if needed.
async function ensureUserRow(screen: Locator, userId: string): Promise<Locator> {
  await expect(screen.locator('table.users-table')).toBeVisible({ timeout: 20_000 }); // UsersScreen.tsx:604
  const row = screen.locator(`tr[data-user-id="${userId}"]`); // UsersScreen.tsx:624
  while ((await row.count()) === 0) {
    const moreBtn = screen.locator('button[data-action="more"]'); // UsersScreen.tsx:762
    if (!(await moreBtn.isVisible().catch(() => false))) break;
    await moreBtn.click();
    await expect(moreBtn)
      .toBeEnabled({ timeout: 15_000 })
      .catch(() => {});
  }
  await expect(
    row,
    `seeded marketing user row tr[data-user-id="${userId}"] must be present on /#users`,
  ).toHaveCount(1, { timeout: 15_000 });
  return row;
}

// Active (non-ended) role tags on a user row: .users-role-tag:not([data-role-ended]) with .users-role-name (UsersScreen.tsx:637-641).
function activeRoleTag(row: Locator, roleCode: string): Locator {
  return row.locator('.users-role-tag:not([data-role-ended])').filter({
    has: row.page().locator('.users-role-name', { hasText: new RegExp(`^${roleCode}$`) }),
  });
}

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
    const screen = page.locator('.users-screen'); // UsersScreen.tsx:532
    await expect(screen).toBeVisible({ timeout: 20_000 });
    await screen.locator('button[data-users-tab="users"]').click(); // UsersScreen.tsx:543
    const row = await ensureUserRow(screen, targetUserId);

    // Restore suspended → resume (UsersScreen.tsx:729) or disabled → enable (UsersScreen.tsx:741).
    const resumeBtn = row.locator('button[data-users-action="resume"]'); // UsersScreen.tsx:729
    if (await resumeBtn.isVisible().catch(() => false)) {
      await resumeBtn.click();
      await expect(
        row.locator('.users-status-badge.users-status-active'), // UsersScreen.tsx:494,670
      ).toBeVisible({ timeout: 15_000 });
    }
    const enableBtn = row.locator('button[data-users-action="enable"]'); // UsersScreen.tsx:741
    if (await enableBtn.isVisible().catch(() => false)) {
      await enableBtn.click();
      await expect(
        row.locator('.users-status-badge.users-status-active'), // UsersScreen.tsx:494,670
      ).toBeVisible({ timeout: 15_000 });
    }

    // Restore granted role → revoke (UsersScreen.tsx:655).
    const reviewerTag = activeRoleTag(row, REVIEWER_ROLE_CODE);
    if ((await reviewerTag.count()) > 0) {
      await reviewerTag.first().locator('button.users-btn-revoke').click(); // UsersScreen.tsx:655
      await expect(reviewerTag).toHaveCount(0, { timeout: 15_000 });
    }

    // Restore employee profile fields if captured (UsersScreen.tsx:690,1022,1031-1083).
    if (originalProfile) {
      const profileBtn = row.locator('button[data-users-action="profile"]'); // UsersScreen.tsx:690
      if (await profileBtn.isVisible().catch(() => false)) {
        await profileBtn.click();
        const dialog = screen.locator('.users-dialog[role="dialog"]'); // UsersScreen.tsx:1022
        await expect(dialog).toBeVisible({ timeout: 10_000 });
        await dialog
          .locator('input#profile-employee-code[name="employeeCode"]') // UsersScreen.tsx:1031-1032
          .fill(originalProfile.employeeCode);
        await dialog
          .locator('input#profile-department[name="department"]') // UsersScreen.tsx:1042-1043
          .fill(originalProfile.department);
        await dialog
          .locator('input#profile-position[name="position"]') // UsersScreen.tsx:1053-1054
          .fill(originalProfile.position);
        await dialog
          .locator('input#profile-tell-code[name="tellCode"]') // UsersScreen.tsx:1064-1065
          .fill(originalProfile.tellCode);
        await dialog.locator('button[data-users-action="profile-save"]').click(); // UsersScreen.tsx:1083
        await expect(dialog).toHaveCount(0, { timeout: 15_000 });
      }
    }
  } catch (e) {
    console.log('[r9-1 cleanup] restoreMarketingUser failed:', e);
  }
}

async function cleanupMatrixGroup(page: Page, groupCode: string): Promise<void> {
  try {
    await page.goto('/#users');
    const screen = page.locator('.users-screen'); // UsersScreen.tsx:532
    await expect(screen).toBeVisible({ timeout: 20_000 });
    await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
    const matrixPanel = screen.locator('[data-users-panel="matrix"] .users-card'); // UsersScreen.tsx:1102, PermissionMatrix.tsx:213
    await expect(matrixPanel).toBeVisible({ timeout: 15_000 });
    const groupRow = matrixPanel.locator(`tr[data-matrix-group="${groupCode}"]`); // PermissionMatrix.tsx:267
    if ((await groupRow.count()) > 0) {
      page.once('dialog', (d) => d.accept()); // PermissionMatrix.tsx:199
      await groupRow.locator(`button[data-matrix-delete="${groupCode}"]`).click(); // PermissionMatrix.tsx:303
      await expect(groupRow).toHaveCount(0, { timeout: 15_000 });
    }
  } catch (e) {
    console.log('[r9-1 cleanup] cleanupMatrixGroup failed:', e);
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
      const screen = page.locator('.users-screen'); // UsersScreen.tsx:532
      await expect(screen).toBeVisible({ timeout: 20_000 });

      const row = await ensureUserRow(screen, targetUserId);

      // 1. userProfileSave (POST /api/users/profile, UsersScreen.tsx:90,414,690,1022-1089)
      await row.locator('button[data-users-action="profile"]').click(); // UsersScreen.tsx:690
      const profileDialog = screen.locator('.users-dialog[role="dialog"]'); // UsersScreen.tsx:1022
      await expect(profileDialog).toBeVisible({ timeout: 10_000 });

      const empInput = profileDialog.locator('input#profile-employee-code[name="employeeCode"]'); // UsersScreen.tsx:1031-1032
      const deptInput = profileDialog.locator('input#profile-department[name="department"]'); // UsersScreen.tsx:1042-1043
      const posInput = profileDialog.locator('input#profile-position[name="position"]'); // UsersScreen.tsx:1053-1054
      const tellInput = profileDialog.locator('input#profile-tell-code[name="tellCode"]'); // UsersScreen.tsx:1064-1065

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
      await profileDialog.locator('button[data-users-action="profile-save"]').click(); // UsersScreen.tsx:1083
      await expect(profileDialog).toHaveCount(0, { timeout: 15_000 });

      // Reload and read back employeeCode from column 0 (UsersScreen.tsx:625) and all 4 fields from the reopened profile dialog (UsersScreen.tsx:1031-1065).
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterProfile = await ensureUserRow(screen, targetUserId);
      await expect(rowAfterProfile.locator('td.users-td').nth(0)).toHaveText(empCode); // UsersScreen.tsx:625

      await rowAfterProfile.locator('button[data-users-action="profile"]').click(); // UsersScreen.tsx:690
      await expect(profileDialog).toBeVisible({ timeout: 10_000 });
      await expect(
        profileDialog.locator('input#profile-employee-code[name="employeeCode"]'),
      ).toHaveValue(empCode);
      await expect(profileDialog.locator('input#profile-department[name="department"]')).toHaveValue(
        dept,
      );
      await expect(profileDialog.locator('input#profile-position[name="position"]')).toHaveValue(pos);
      await expect(profileDialog.locator('input#profile-tell-code[name="tellCode"]')).toHaveValue(
        tellCode,
      );
      await profileDialog.locator('button[type="button"].users-btn-secondary').click(); // UsersScreen.tsx:1075
      await expect(profileDialog).toHaveCount(0, { timeout: 10_000 });

      // 2. userGrant (POST /api/users/grant, UsersScreen.tsx:84,371,701,931-1012)
      await rowAfterProfile.locator('button.users-btn-grant').click(); // UsersScreen.tsx:701
      const grantDialog = screen.locator('.users-dialog[role="dialog"]'); // UsersScreen.tsx:931
      await expect(grantDialog).toBeVisible({ timeout: 10_000 });
      await grantDialog.locator('select#grant-role').selectOption(REVIEWER_ROLE_CODE); // UsersScreen.tsx:940
      const branchSelect = grantDialog.locator('select#grant-branch'); // UsersScreen.tsx:959
      await expect(branchSelect).toBeVisible();
      await grantDialog.locator('button[type="submit"].users-btn-primary').click(); // UsersScreen.tsx:1005-1006
      await expect(grantDialog).toHaveCount(0, { timeout: 15_000 });

      // Reload and read back the granted role tag (UsersScreen.tsx:637-641).
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterGrant = await ensureUserRow(screen, targetUserId);
      await expect(activeRoleTag(rowAfterGrant, REVIEWER_ROLE_CODE)).toHaveCount(1, {
        timeout: 15_000,
      });
      await shot(page, 'admin-users', 'r9-1-user-profile-and-grant');

      // 3. userRevoke (POST /api/users/revoke, UsersScreen.tsx:85,438,655)
      await activeRoleTag(rowAfterGrant, REVIEWER_ROLE_CODE)
        .first()
        .locator('button.users-btn-revoke') // UsersScreen.tsx:655
        .click();
      await expect(activeRoleTag(rowAfterGrant, REVIEWER_ROLE_CODE)).toHaveCount(0, {
        timeout: 15_000,
      });

      // Reload and confirm the active role tag remains revoked.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterRevoke = await ensureUserRow(screen, targetUserId);
      await expect(activeRoleTag(rowAfterRevoke, REVIEWER_ROLE_CODE)).toHaveCount(0, {
        timeout: 15_000,
      });

      // 4. userSuspend (POST /api/users/suspend, UsersScreen.tsx:87,484,709)
      await rowAfterRevoke.locator('button[data-users-action="suspend"]').click(); // UsersScreen.tsx:709
      await expect(
        rowAfterRevoke.locator('.users-status-badge.users-status-suspended'), // UsersScreen.tsx:500,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify SUSPENDED persists.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterSuspend = await ensureUserRow(screen, targetUserId);
      await expect(
        rowAfterSuspend.locator('.users-status-badge.users-status-suspended'),
      ).toBeVisible({ timeout: 15_000 });

      // 5. userResume (POST /api/users/resume, UsersScreen.tsx:88,486,729)
      await rowAfterSuspend.locator('button[data-users-action="resume"]').click(); // UsersScreen.tsx:729
      await expect(
        rowAfterSuspend.locator('.users-status-badge.users-status-active'), // UsersScreen.tsx:494,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify ACTIVE persists after resume.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterResume = await ensureUserRow(screen, targetUserId);
      await expect(
        rowAfterResume.locator('.users-status-badge.users-status-active'),
      ).toBeVisible({ timeout: 15_000 });

      // 6. userDisable (POST /api/users/disable, UsersScreen.tsx:86,482,717)
      await rowAfterResume.locator('button.users-btn-disable').click(); // UsersScreen.tsx:717
      await expect(
        rowAfterResume.locator('.users-status-badge.users-status-disabled'), // UsersScreen.tsx:502,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify DISABLED persists.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterDisable = await ensureUserRow(screen, targetUserId);
      await expect(
        rowAfterDisable.locator('.users-status-badge.users-status-disabled'),
      ).toBeVisible({ timeout: 15_000 });

      // 7. userEnable (POST /api/users/enable, UsersScreen.tsx:89,488,741)
      await rowAfterDisable.locator('button[data-users-action="enable"]').click(); // UsersScreen.tsx:741
      await expect(
        rowAfterDisable.locator('.users-status-badge.users-status-active'), // UsersScreen.tsx:494,670
      ).toBeVisible({ timeout: 15_000 });

      // Reload and verify ACTIVE persists after enable.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      const rowAfterEnable = await ensureUserRow(screen, targetUserId);
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
      const screen = page.locator('.users-screen'); // UsersScreen.tsx:532
      await expect(screen).toBeVisible({ timeout: 20_000 });

      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      const matrixPanel = screen.locator('[data-users-panel="matrix"] .users-card'); // UsersScreen.tsx:1102, PermissionMatrix.tsx:213
      await expect(matrixPanel).toBeVisible({ timeout: 15_000 });

      // 1. adminGroupAdd (POST /api/admin/groups/add, PermissionMatrix.tsx:30,185,228,365-440)
      await matrixPanel.locator('button[data-matrix-add-open]').click(); // PermissionMatrix.tsx:228
      const addForm = matrixPanel.locator('.users-matrix-panel[data-matrix-add-form]'); // PermissionMatrix.tsx:365
      await expect(addForm).toBeVisible({ timeout: 10_000 });

      await addForm.locator('input[data-matrix-add-code]').fill(groupCode); // PermissionMatrix.tsx:373
      await addForm.locator('input[data-matrix-add-name]').fill(groupName); // PermissionMatrix.tsx:383
      await addForm.locator('select[data-matrix-add-base]').selectOption('TELLER'); // PermissionMatrix.tsx:392
      await addForm.locator('button[data-matrix-add-submit]').click(); // PermissionMatrix.tsx:433
      await expect(addForm).toHaveCount(0, { timeout: 15_000 });

      const groupRow = matrixPanel.locator(`tr[data-matrix-group="${groupCode}"]`); // PermissionMatrix.tsx:267
      await expect(groupRow).toBeVisible({ timeout: 15_000 });
      await expect(groupRow).toContainText(groupName);

      // Reload and verify the new group row persists on the Permission Matrix tab.
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      await expect(groupRow).toBeVisible({ timeout: 15_000 });
      await expect(groupRow).toContainText(groupName);

      // 2. adminGroupSave (POST /api/admin/groups/save, PermissionMatrix.tsx:31,171,293,321-361)
      await groupRow.locator(`button[data-matrix-edit="${groupCode}"]`).click(); // PermissionMatrix.tsx:293
      const editor = matrixPanel.locator('.users-matrix-panel:not([data-matrix-add-form])'); // PermissionMatrix.tsx:321
      await expect(editor).toBeVisible({ timeout: 10_000 });

      const reportsPerm = editor.locator('input[type="checkbox"][data-matrix-perm="MarketingPerformance"]'); // PermissionMatrix.tsx:332
      await expect(reportsPerm).not.toBeChecked();
      await reportsPerm.check();
      await editor.locator('button[data-matrix-save]').click(); // PermissionMatrix.tsx:354
      await expect(editor).toHaveCount(0, { timeout: 15_000 });

      // Reload and verify the saved permission level (REPORTS = VIEW -> "ดูข้อมูล" / "View") and checkbox state persist (PermissionMatrix.tsx:120-121,280, i18n.ts:96,263).
      await page.reload();
      await expect(screen).toBeVisible({ timeout: 20_000 });
      await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
      await expect(groupRow).toBeVisible({ timeout: 15_000 });
      const reportsCell = groupRow.locator(`td[data-matrix-level="${groupCode}:REPORTS"]`); // PermissionMatrix.tsx:280
      await expect(reportsCell).toContainText(/ดูข้อมูล|View/); // i18n.ts:96,263

      await groupRow.locator(`button[data-matrix-edit="${groupCode}"]`).click(); // PermissionMatrix.tsx:293
      await expect(editor).toBeVisible({ timeout: 10_000 });
      await expect(
        editor.locator('input[type="checkbox"][data-matrix-perm="MarketingPerformance"]'), // PermissionMatrix.tsx:332
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
      await expect(matrixPanel).toBeVisible({ timeout: 15_000 });
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
    const ownCodes: string[] = [];
    // v3 commands need the same-origin headers and an Idempotency-Key header (command.rs:256-268).
    const writeHeaders = (key: string) => ({
      'x-xchange-request': '1',
      origin: new URL(users.baseUrl).origin,
      'Idempotency-Key': key,
    });

    try {
      await signIn(page, 'admin', '/#users');
      const screen = page.locator('.users-screen'); // UsersScreen.tsx:532
      await expect(screen).toBeVisible({ timeout: 20_000 });

      for (const lang of STAFF_LANGS) {
        await withLang(page, lang);
        await page.goto('/#users');
        await expect(screen).toBeVisible({ timeout: 20_000 });

        // Case 1: userGrant on a non-active user returns 409 userNotActive -> worded in th ('บัญชีนี้ไม่ได้ใช้งานอยู่', th.ts:60) and en ('This account is not active', en.ts:62) while keeping #grant-valid-from and #grant-valid-to (UsersScreen.tsx:931,934,975,987).
        const row = await ensureUserRow(screen, targetUserId);
        await row.locator('button.users-btn-grant').click(); // UsersScreen.tsx:701
        const grantDialog = screen.locator('.users-dialog[role="dialog"]'); // UsersScreen.tsx:931
        await expect(grantDialog).toBeVisible({ timeout: 10_000 });

        const fromInput = grantDialog.locator('input#grant-valid-from[name="grantValidFrom"]'); // UsersScreen.tsx:975-976
        const toInput = grantDialog.locator('input#grant-valid-to[name="grantValidTo"]'); // UsersScreen.tsx:987-988
        await fromInput.fill('2026-12-01');
        await toInput.fill('2026-12-31');

        const suspendRes = await page.request.post('/api/users/suspend', {
          headers: writeHeaders(`qa-r9-1-inv-susp-${lang}-${Date.now()}`),
          data: { userId: targetUserId },
        });
        expect(suspendRes.ok(), `POST /api/users/suspend returned ${suspendRes.status()}`).toBe(true);

        try {
          await grantDialog.locator('button[type="submit"].users-btn-primary').click(); // UsersScreen.tsx:1005-1006

          const grantErr = grantDialog.locator('[role="alert"].users-form-error'); // UsersScreen.tsx:934
          await expect(grantErr, `[${lang}] userGrant on suspended user must show alert`).toBeVisible({
            timeout: 15_000,
          });
          const grantErrText = (await grantErr.innerText()).trim();
          expect(grantErrText).not.toMatch(RAW_CODE);
          if (lang === 'th') {
            expect(grantErrText).toMatch(/[\u0E00-\u0E7F]/);
            expect(grantErrText).toContain('บัญชีนี้ไม่ได้ใช้งานอยู่');
          } else {
            expect(grantErrText).toContain('This account is not active');
          }
          await expect(fromInput).toHaveValue('2026-12-01');
          await expect(toInput).toHaveValue('2026-12-31');
        } finally {
          await page.request.post('/api/users/resume', {
            headers: writeHeaders(`qa-r9-1-inv-res-${lang}-${Date.now()}`),
            data: { userId: targetUserId },
          });
        }
        await grantDialog.locator('button[type="button"].users-btn-secondary').click(); // UsersScreen.tsx:999
        await expect(grantDialog).toHaveCount(0, { timeout: 10_000 });

        // Switch to Permission Matrix tab for Cases 2–4 (UsersScreen.tsx:543,1102, PermissionMatrix.tsx:213).
        await screen.locator('button[data-users-tab="matrix"]').click(); // UsersScreen.tsx:543
        const matrixPanel = screen.locator('[data-users-panel="matrix"] .users-card'); // UsersScreen.tsx:1102, PermissionMatrix.tsx:213
        await expect(matrixPanel).toBeVisible({ timeout: 15_000 });

        await matrixPanel.locator('button[data-matrix-add-open]').click(); // PermissionMatrix.tsx:228
        const addForm = matrixPanel.locator('.users-matrix-panel[data-matrix-add-form]'); // PermissionMatrix.tsx:365
        await expect(addForm).toBeVisible({ timeout: 10_000 });

        const codeInput = addForm.locator('input[data-matrix-add-code]'); // PermissionMatrix.tsx:373
        const nameInput = addForm.locator('input[data-matrix-add-name]'); // PermissionMatrix.tsx:383
        const baseSelect = addForm.locator('select[data-matrix-add-base]'); // PermissionMatrix.tsx:392
        const submitBtn = addForm.locator('button[data-matrix-add-submit]'); // PermissionMatrix.tsx:433
        const matrixErr = matrixPanel.locator('div[role="alert"][data-matrix-error]'); // PermissionMatrix.tsx:238

        // Case 2: Guardrail 2 breach on adminGroupAdd — non-admin base (TELLER) with UsersManage -> 409 guardrail:2 -> matrixGuardrail_2 (PermissionMatrix.tsx:55-56, i18n.ts:125,292).
        await codeInput.fill(validCode);
        await nameInput.fill(markerName);
        await baseSelect.selectOption('TELLER');
        const usersManageCb = addForm.locator('input[type="checkbox"][data-matrix-perm="UsersManage"]'); // PermissionMatrix.tsx:410
        await usersManageCb.check();
        await submitBtn.click();

        await expect(matrixErr, `[${lang}] Guardrail 2 refusal must show alert`).toBeVisible({
          timeout: 15_000,
        });
        const g2Text = (await matrixErr.innerText()).trim();
        expect(g2Text).not.toMatch(RAW_CODE);
        if (lang === 'th') {
          expect(g2Text).toMatch(/[\u0E00-\u0E7F]/);
          expect(g2Text).toContain('การจัดการผู้ใช้งานและตั้งค่าระบบ');
        } else {
          expect(g2Text).toContain(
            'User Management and System Settings above View are allowed only for groups with base EXECUTIVE or ADMIN.',
          );
        }
        await expect(codeInput).toHaveValue(validCode);
        await expect(nameInput).toHaveValue(markerName);
        await expect(baseSelect).toHaveValue('TELLER');
        await expect(usersManageCb).toBeChecked();

        // Case 3: Guardrail 3 breach on adminGroupAdd — ADMIN base with ReportsExport (Full outside USER_MANAGEMENT/SYSTEM_SETTINGS) -> 409 guardrail:3 -> matrixGuardrail_3 (PermissionMatrix.tsx:57-58, i18n.ts:126,293).
        await usersManageCb.uncheck();
        await baseSelect.selectOption('ADMIN');
        const reportsExportCb = addForm.locator('input[type="checkbox"][data-matrix-perm="ReportsExport"]'); // PermissionMatrix.tsx:410
        await reportsExportCb.check();
        await submitBtn.click();

        await expect(matrixErr, `[${lang}] Guardrail 3 refusal must show alert`).toBeVisible({
          timeout: 15_000,
        });
        const g3Text = (await matrixErr.innerText()).trim();
        expect(g3Text).not.toMatch(RAW_CODE);
        if (lang === 'th') {
          expect(g3Text).toMatch(/[\u0E00-\u0E7F]/);
          expect(g3Text).toContain('กลุ่มที่มีฐานเป็น ADMIN');
        } else {
          expect(g3Text).toContain(
            'Groups with base ADMIN may hold at most View level outside User Management and System Settings.',
          );
        }
        await expect(codeInput).toHaveValue(validCode);
        await expect(nameInput).toHaveValue(markerName);
        await expect(baseSelect).toHaveValue('ADMIN');
        await expect(reportsExportCb).toBeChecked();

        // Case 4 setup: save a valid group of this run's own, so case 4 never edits a seeded group (§9.2, E-4).
        const ownCode = `${validCode}_${lang.toUpperCase()}`;
        ownCodes.push(ownCode);
        await reportsExportCb.uncheck();
        await baseSelect.selectOption('TELLER');
        await codeInput.fill(ownCode);
        await nameInput.fill(`${markerName}-${lang}`);
        await submitBtn.click();
        await expect(addForm).toHaveCount(0, { timeout: 15_000 });

        // Case 4: Guardrail 2 breach on adminGroupSave — this run's TELLER-based group gains SystemSettings -> 409 guardrail:2 -> matrixGuardrail_2 (PermissionMatrix.tsx:55-56,293,321,332,354, i18n.ts:125,292).
        const ownRow = matrixPanel.locator(`tr[data-matrix-group="${ownCode}"]`); // PermissionMatrix.tsx:267
        await expect(ownRow).toBeVisible({ timeout: 15_000 });
        await ownRow.locator(`button[data-matrix-edit="${ownCode}"]`).click(); // PermissionMatrix.tsx:293
        const editor = matrixPanel.locator('.users-matrix-panel:not([data-matrix-add-form])'); // PermissionMatrix.tsx:321
        await expect(editor).toBeVisible({ timeout: 10_000 });

        const sysSettingsEditCb = editor.locator('input[type="checkbox"][data-matrix-perm="SystemSettings"]'); // PermissionMatrix.tsx:332
        await sysSettingsEditCb.check();
        await editor.locator('button[data-matrix-save]').click(); // PermissionMatrix.tsx:354

        await expect(matrixErr, `[${lang}] adminGroupSave Guardrail 2 refusal must show alert`).toBeVisible({
          timeout: 15_000,
        });
        const saveErrText = (await matrixErr.innerText()).trim();
        expect(saveErrText).not.toMatch(RAW_CODE);
        if (lang === 'th') {
          expect(saveErrText).toMatch(/[\u0E00-\u0E7F]/);
          expect(saveErrText).toContain('การจัดการผู้ใช้งานและตั้งค่าระบบ');
        } else {
          expect(saveErrText).toContain(
            'User Management and System Settings above View are allowed only for groups with base EXECUTIVE or ADMIN.',
          );
        }
        await expect(editor).toBeVisible();
        await expect(sysSettingsEditCb).toBeChecked();

        await editor.locator('button[type="button"].users-btn-secondary').click(); // PermissionMatrix.tsx:346
        await expect(editor).toHaveCount(0, { timeout: 10_000 });
      }
      await shot(page, 'admin-users', 'r9-1-invalid-inputs');
    } finally {
      await cleanupMatrixGroup(page, validCode);
      for (const code of ownCodes) await cleanupMatrixGroup(page, code);
      await restoreMarketingUser(page, targetUserId);
    }
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

    // UI gate: nav link a[href="#users"] requires UsersManage (menu.ts:9-12, AdminShell.tsx:504) and .users-screen (UsersScreen.tsx:532) is not rendered.
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
