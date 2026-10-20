// Shift open and close on /staff (staff shift card, web/src/screens/staff/shift/ShiftCard.tsx), against a running v3.
// The teller counts, the branch manager verifies: the verifier must not be the counter
// (api/crates/xc-cash/src/domain/person_rules.rs verifier_is_not_counter). Accounts come from ../users.json.
import { expect, test, type Browser, type Page } from '@playwright/test';
import {
  closeShift,
  hasAccount,
  openShift,
  posted,
  shot,
  signIn,
  users,
  type Role,
} from './helpers';

async function asRole(browser: Browser, role: Role): Promise<Page> {
  const page = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
  await signIn(page, role, '/staff');
  return page;
}

const FUND_QTY = '100'; // of every THB note: far more than the opening count takes

// The transfers row for one id: `tr[data-transfer-id]` (web/src/screens/staff/transfers/TransfersCard.tsx:574).
const transferRow = (page: Page, id: string) => page.locator(`tr[data-transfer-id="${id}"]`);

test.describe.serial('shift open and close', () => {
  test.skip(!hasAccount('branchManager'), 'no branchManager in users.json: run scripts/reseed.sh');

  // Since LG-4 (#929, design ledger-inventory.md §4.4 row 1) verifying an OPEN count moves the counted cash out of
  // the branch's ledger SAFE (count_service.rs post_opening_in), so an empty safe refuses it with stockInsufficient.
  // Cash enters a safe only by an EXTERNAL_IN transfer (BANK -> SAFE, kinds/external.rs). A5 (cash-shift-transfer.md:17):
  // within the limit the source branch's BRANCH_MANAGER approves, never the creator (approval_service.rs:260), and an
  // above-limit one goes to EXECUTIVE. So FINANCE (CashManage) records it, the branch manager approves it on the admin
  // approvals screen, and FINANCE dispatches and receives it.
  test('finance funds the safe from the bank (EXTERNAL_IN), the branch manager approves it', async ({ browser }) => {
    test.skip(!hasAccount('finance'), 'no finance in users.json: run scripts/reseed.sh');
    const finance = await asRole(browser, 'finance');
    await finance.getByTestId('transfer-new').click();
    const form = finance.getByTestId('transfer-form');
    await expect(form).toBeVisible({ timeout: 15_000 });
    await form.getByTestId('transfer-kind').selectOption('EXTERNAL_IN');
    await form.getByTestId('transfer-note').fill('e2e: fund the safe for the shift test');
    const thbRows = form.locator('[data-denomination-row]').filter({ hasText: 'THB' });
    const thbCount = await thbRows.count();
    expect(thbCount, 'THB rows in the transfer form').toBeGreaterThan(0);
    for (let i = 0; i < thbCount; i++) await thbRows.nth(i).locator('input[data-denomination-id]').fill(FUND_QTY);
    await shot(finance, 'cash', 'fund-1-form');
    const submitted = finance.waitForResponse(
      (r) => r.request().method() === 'POST' && /\/api\/cash-transfers\/[^/]+\/submit$/.test(new URL(r.url()).pathname),
      { timeout: 15_000 },
    );
    const created = await posted(finance, /^\/api\/cash-transfers$/, () => form.getByTestId('transfer-save').click());
    console.log(`[fund] POST /api/cash-transfers ${created.text}`);
    expect(created.ok, `transfer created: ${created.text}`).toBeTruthy();
    const id = String(JSON.parse(created.body).id ?? '');
    expect(id, 'the new transfer id').not.toBe('');
    const sub = await submitted;
    const subText = `${sub.status()} ${(await sub.text().catch(() => '')).slice(0, 300)}`;
    console.log(`[fund] submit ${subText}`);
    expect(sub.ok(), `transfer submitted: ${subText}`).toBeTruthy();
    await finance.reload();
    const row = transferRow(finance, id);
    await expect(row).toBeVisible({ timeout: 20_000 });
    await shot(finance, 'cash', 'fund-2-submitted');

    // The branch manager decides it on the admin approvals screen (admin.html at /, menu `approvals`).
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    await signIn(manager, 'branchManager', '/');
    await manager.goto('/#approvals');
    const card = manager.locator('[data-request]').filter({ has: manager.locator('[data-request-type="CASH_TRANSFER"]') })
      .filter({ has: manager.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) }).first();
    await expect(card, 'a pending CASH_TRANSFER approval for the branch manager').toBeVisible({ timeout: 20_000 });
    await shot(manager, 'cash', 'fund-3-approval');
    const decided = await posted(manager, /^\/api\/approvals\/[^/]+\/decide$/, () =>
      card.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click());
    console.log(`[fund-manager] decide ${decided.text}`);
    expect(decided.ok, `approval decided: ${decided.text}`).toBeTruthy();
    await manager.close();

    await finance.reload();
    await expect(row, 'the transfer is approved').toContainText('APPROVED', { timeout: 20_000 });
    const dispatched = await posted(finance, /\/api\/cash-transfers\/[^/]+\/dispatch$/, () => row.getByTestId('transfer-dispatch').click());
    console.log(`[fund] dispatch ${dispatched.text}`);
    expect(dispatched.ok, `transfer dispatched: ${dispatched.text}`).toBeTruthy();
    await finance.reload();
    await expect(row, 'the transfer is dispatched').toContainText('DISPATCHED', { timeout: 20_000 });
    await row.getByTestId('transfer-receive').click();
    const recv = finance.getByTestId('transfer-receive-form');
    await expect(recv).toBeVisible({ timeout: 15_000 });
    const recvRows = recv.locator('[data-denomination-row]').filter({ hasText: 'THB' });
    for (let i = 0; i < (await recvRows.count()); i++) await recvRows.nth(i).locator('input[data-denomination-id]').fill(FUND_QTY);
    const received = await posted(finance, /\/api\/cash-transfers\/[^/]+\/receive$/, () => recv.getByTestId('transfer-receive-save').click());
    console.log(`[fund] receive ${received.text}`);
    expect(received.ok, `transfer received: ${received.text}`).toBeTruthy();
    await finance.reload();
    await expect(row, 'the transfer is received').toContainText('RECEIVED', { timeout: 20_000 });
    await shot(finance, 'cash', 'fund-4-received');
    await finance.close();
  });

  test('teller counts the opening, the branch manager verifies it', async ({ browser }) => {
    const teller = await asRole(browser, 'teller');
    const manager = await asRole(browser, 'branchManager');
    await openShift(teller, manager);
    await teller.close();
    await manager.close();
  });

  test('teller counts the close, the branch manager verifies it', async ({ browser }) => {
    const teller = await asRole(browser, 'teller');
    const manager = await asRole(browser, 'branchManager');
    await closeShift(teller, manager);
    await teller.close();
    await manager.close();
  });
});
