import { expect, test, type Page } from '@playwright/test';
import {
  clockIn,
  ensureShiftClosed,
  hasAccount,
  openShift,
  posted,
  SHIFT_QTY,
  shot,
  signIn,
  users,
  verifyCount,
  type Role,
} from './helpers';

// Flow F7 (docs/design/qa-e2e.md §4 F7):
//   Short closing count -> manager verify returns AWAITING_FINANCE (shift stays OPEN) ->
//   finance approves COUNT_DIFFERENCE -> manager re-verifies (VERIFIED) -> shift closes.

const REQUIRED: Role[] = ['teller', 'branchManager', 'finance'];

test.describe.serial('F7 count difference', () => {
  let teller: Page | undefined;
  let manager: Page | undefined;
  let finance: Page | undefined;

  test.afterAll(async () => {
    try {
      if (teller && manager) {
        await signIn(teller, 'teller', '/staff');
        await signIn(manager, 'branchManager', '/staff');
        await ensureShiftClosed(teller, manager);
      }
    } finally {
      await Promise.all([teller?.close(), manager?.close(), finance?.close()]);
    }
  });

  test('short closing count requires finance approval before manager re-verify closes the shift', async ({
    browser,
  }) => {
    const missing = REQUIRED.filter((r) => !hasAccount(r));
    test.skip(
      missing.length > 0,
      `missing password in users.local.json for: ${missing.map((r) => `${r} (${users[r]?.identifier ?? 'unmapped'})`).join(', ')}`,
    );

    const tellerPage = await browser.newPage();
    const managerPage = await browser.newPage();
    const financePage = await browser.newPage();
    teller = tellerPage;
    manager = managerPage;
    finance = financePage;

    // 1. Open shift cleanly from a known baseline.
    await signIn(tellerPage, 'teller', '/staff');
    await signIn(managerPage, 'branchManager', '/staff');
    await openShift(tellerPage, managerPage);

    // 2. Teller clocks in and submits a short closing count (first THB denomination = SHIFT_QTY - 1).
    await clockIn(tellerPage);
    await tellerPage.getByTestId('shift-close-count').click();
    const form = tellerPage.getByTestId('shift-count-form');
    await expect(form).toBeVisible();
    const thb = form.locator('[data-denomination-row]').filter({ hasText: 'THB' }).first();
    await expect(thb, 'a THB denomination row in the count form').toBeVisible({ timeout: 15_000 });
    await thb.locator('input[data-denomination-id]').fill(String(Number(SHIFT_QTY) - 1));
    const closeCount = await posted(tellerPage, /^\/api\/cash-counts$/, () =>
      tellerPage.getByTestId('shift-count-save').click(),
    );
    expect(closeCount.ok, `short closing count accepted: ${closeCount.text}`).toBeTruthy();

    // 3. Manager verifies CLOSE -> 200 with status AWAITING_FINANCE + approvalRequestId, shift-notice visible.
    const firstVerify = await verifyCount(managerPage, 'CLOSE');
    expect(firstVerify.ok, `first CLOSE verify returned 200: ${firstVerify.text}`).toBeTruthy();
    expect(firstVerify.text).toContain('AWAITING_FINANCE');
    expect(firstVerify.text).toContain('approvalRequestId');
    await expect(managerPage.getByTestId('shift-notice')).toBeVisible();

    // 4. Teller reloads -> shift-close-count is still visible (shift stays OPEN).
    await tellerPage.reload();
    await expect(tellerPage.getByTestId('shift-close-count'), 'shift stays OPEN while awaiting finance').toBeVisible({
      timeout: 20_000,
    });

    // 5. Manager views /#cash -> [data-section="counts-waiting"] shows CLOSE row with negative Difference.
    await signIn(managerPage, 'branchManager', '/');
    await managerPage.goto('/#cash');
    const waitingCloseRow = managerPage
      .locator('[data-section="counts-waiting"] tbody tr')
      .filter({ hasText: 'CLOSE' })
      .first();
    await expect(waitingCloseRow, 'CLOSE row in counts-waiting').toBeVisible({ timeout: 20_000 });
    const diffCell = waitingCloseRow.locator('td').nth(6);
    await expect(diffCell, 'negative difference cell').toContainText(/-\d/);
    await shot(managerPage, 'cash', 'count-diff-waiting');

    // 6. Finance signs in on /, filters COUNT_DIFFERENCE on /#approvals, and approves the pending request.
    await signIn(financePage, 'finance', '/');
    await financePage.goto('/#approvals');
    const filterBtn = financePage.locator('[data-type-filter="COUNT_DIFFERENCE"]');
    await expect(filterBtn).toBeVisible({ timeout: 20_000 });
    await filterBtn.click();

    const card = financePage
      .locator('[data-request]')
      .filter({ has: financePage.locator('[data-request-type="COUNT_DIFFERENCE"]') })
      .filter({ has: financePage.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) })
      .first();
    await expect(card, 'pending COUNT_DIFFERENCE approval card').toBeVisible({ timeout: 20_000 });
    const decided = await posted(financePage, /\/api\/approvals\/[^/]+\/decide$/, () =>
      card.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click(),
    );
    expect(decided.ok, `COUNT_DIFFERENCE approved: ${decided.text}`).toBeTruthy();

    // 7. Manager returns to /staff and re-verifies CLOSE -> 200 with status VERIFIED.
    await managerPage.goto('/staff');
    const secondVerify = await verifyCount(managerPage, 'CLOSE');
    expect(secondVerify.ok, `second CLOSE verify returned 200: ${secondVerify.text}`).toBeTruthy();
    expect(secondVerify.text).toContain('VERIFIED');

    // 8. Teller reloads -> shift-open-count visible (shift closed); teller clicks floor-clock to clock out.
    await tellerPage.reload();
    await expect(tellerPage.getByTestId('shift-open-count'), 'shift is closed after re-verify').toBeVisible({
      timeout: 20_000,
    });
    const out = await posted(tellerPage, /^\/api\/attendance$/, () =>
      tellerPage.getByTestId('floor-clock').click(),
    );
    expect(out.ok, `teller clocked out: ${out.text}`).toBeTruthy();
  });
});
