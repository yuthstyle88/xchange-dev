// QA-F18 — flow: admin reads (design docs/design/qa-e2e.md §4 row F18; ticket #4160).
// Written by ticket #4160 (QA-F18); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Browser, type Download } from '@playwright/test';
import { hasAccount, shot, signIn, users } from './helpers';

let cachedCustomerId: string | undefined;

async function getTestCustomerId(browser: Browser): Promise<string> {
  if (cachedCustomerId) return cachedCustomerId;
  const context = await browser.newContext({ baseURL: users.baseUrl });
  const page = await context.newPage();
  try {
    await signIn(page, 'customer', '/app');
    const res = await page.request.get('/api/v3/views/my_profile');
    expect(res.ok()).toBe(true);
    const json = (await res.json()) as { data?: { customerId?: string }; customerId?: string };
    const id = json?.data?.customerId ?? json?.customerId;
    expect(id).toBeTruthy();
    cachedCustomerId = id;
    return id!;
  } finally {
    await context.close().catch(() => {});
  }
}

async function readFirstLine(download: Download): Promise<string> {
  const stream = await download.createReadStream();
  expect(stream).not.toBeNull();
  let text = '';
  for await (const chunk of stream!) {
    text += String(chunk);
    if (text.includes('\n')) break;
  }
  return text.split(/\r?\n/)[0]?.trim() ?? '';
}

test.describe('F18 admin reads', () => {
  test('finance on /#reports exports transactions CSV', async ({ page }) => {
    test.skip(!hasAccount('finance'), 'no finance account in users.json: run scripts/reseed.sh');
    await signIn(page, 'finance', '/#reports');
    const screen = page.locator('.reports-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });
    const exportBtn = screen.locator('#export');
    await expect(exportBtn).toBeVisible({ timeout: 15_000 });
    const downloadPromise = page.waitForEvent('download');
    await exportBtn.click();
    const download = await downloadPromise;
    expect(download.suggestedFilename()).toMatch(/\.csv$/);
    const header = await readFirstLine(download);
    expect(header.length).toBeGreaterThan(0);
    expect(header).toContain(',');
    await shot(page, 'admin-reads', 'f18-1-reports-finance');
  });

  test('finance on /#reports exports employee CSV', async ({ page }) => {
    test.skip(!hasAccount('finance'), 'no finance account in users.json: run scripts/reseed.sh');
    await signIn(page, 'finance', '/#reports');
    const screen = page.locator('.reports-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });
    const employeeTab = screen.locator('#workspace-reports-2-tab');
    await expect(employeeTab).toBeVisible({ timeout: 15_000 });
    await employeeTab.click();
    const employeeCsvBtn = screen.locator('#employee-csv');
    await expect(employeeCsvBtn).toBeVisible({ timeout: 15_000 });
    const downloadPromise = page.waitForEvent('download');
    await employeeCsvBtn.click();
    const download = await downloadPromise;
    expect(download.suggestedFilename()).toMatch(/\.csv$/);
    const header = await readFirstLine(download);
    expect(header.length).toBeGreaterThan(0);
    expect(header).toContain(',');
    await shot(page, 'admin-reads', 'f18-2-reports-employee');
  });

  test('branch manager looks for #export on /#reports', async ({ page }) => {
    test.skip(!hasAccount('branchManager'), 'no branchManager account in users.json: run scripts/reseed.sh');
    await signIn(page, 'branchManager', '/#reports');
    const screen = page.locator('.reports-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });
    const exportBtn = screen.locator('#export');
    // Either outcome is accepted (ticket #4160 row 3): no export button, or a refused export. The refusal shows on
    // the error line as a worded sentence (Reports.tsx:247-254,359 wordedError), not the raw `outOfScope` code.
    if (await exportBtn.isVisible()) {
      await exportBtn.click();
      const errorLine = screen.locator('[role="alert"].error');
      await expect(errorLine, 'refused export shows the worded error line').toBeVisible({ timeout: 15_000 });
      await expect(errorLine).not.toHaveText(/^\s*$/);
      console.log(`[F18] branch manager: export refused: ${(await errorLine.innerText()).trim()}`);
    } else {
      await expect(exportBtn).toHaveCount(0);
      console.log('[F18] branch manager: no export button');
    }
    await shot(page, 'admin-reads', 'f18-3-reports-manager');
  });

  test('admin searches the test customer on /#customers', async ({ page, browser }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    test.skip(!hasAccount('customer'), 'no customer account in users.json: run scripts/reseed.sh');
    const targetId = await getTestCustomerId(browser);
    await signIn(page, 'admin', '/#customers');
    const screen = page.locator('.customers-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });
    const searchInput = screen.locator('input[type="search"]');
    await expect(searchInput).toBeVisible({ timeout: 15_000 });
    await searchInput.fill(targetId);
    const rows = screen.locator('.customers-table tbody tr');
    await expect(rows).toHaveCount(1, { timeout: 20_000 });
    await expect(rows.first()).toContainText(targetId);
    await shot(page, 'admin-reads', 'f18-4-customers-search');
  });

  test('admin opens test customer history on /#customers', async ({ page, browser }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    test.skip(!hasAccount('customer'), 'no customer account in users.json: run scripts/reseed.sh');
    const targetId = await getTestCustomerId(browser);
    await signIn(page, 'admin', '/#customers');
    const screen = page.locator('.customers-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });
    const searchInput = screen.locator('input[type="search"]');
    await expect(searchInput).toBeVisible({ timeout: 15_000 });
    await searchInput.fill(targetId);
    const rows = screen.locator('.customers-table tbody tr');
    await expect(rows).toHaveCount(1, { timeout: 20_000 });
    await rows.first().locator('button.ref, button.secondary').first().click();
    const dialog = page.locator('dialog[open]');
    await expect(dialog).toBeVisible({ timeout: 15_000 });
    await expect(dialog.locator('dl.details')).toContainText(targetId);
    await shot(page, 'admin-reads', 'f18-5-customer-history');
    await dialog.locator('header button').click();
    await expect(dialog).not.toBeVisible({ timeout: 10_000 });
  });
});
