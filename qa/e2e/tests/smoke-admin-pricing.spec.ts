// QA-F10 — smoke: admin pricing menu (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-F10; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { hasAccount, signIn, smoke, STAFF_LANGS } from './helpers';

test.describe('QA-F10 smoke: admin pricing menu', () => {
  // key: web/src/screens/admin/pricing/Pricing.tsx:153 (menu.ts:15 needs ReferenceRatesRead)
  test('/#pricing as admin', async ({ page }) => {
    await signIn(page, 'admin', '/#pricing');
    await smoke(page, {
      area: 'admin-money',
      screen: 'pricing',
      path: '/#pricing',
      role: 'admin',
      key: '.pricing-screen',
      langs: STAFF_LANGS,
      needs: 'ReferenceRatesRead',
    });
  });

  // key: web/src/screens/admin/pricing/Pricing.tsx:153 (menu.ts:15 needs ReferenceRatesRead; finance holds it)
  test('/#pricing as finance', async ({ page }) => {
    test.skip(!hasAccount('finance'), 'no finance account in users.json: run scripts/reseed.sh');
    await signIn(page, 'finance', '/#pricing');
    await smoke(page, {
      area: 'admin-money',
      screen: 'pricing-finance',
      path: '/#pricing',
      role: 'finance',
      key: '.pricing-screen',
      langs: STAFF_LANGS,
      needs: 'ReferenceRatesRead',
    });
  });

  // key: web/src/screens/admin/pricing/Pricing.tsx:154 (toolbar, search, reload, detail)
  test('read-only clicks, admin', async ({ page }) => {
    let writeCalls = 0;
    await page.route('**/api/pricing/**', async (route) => {
      writeCalls++;
      await route.abort('blockedbyclient');
    });
    await page.route('**/api/rate-references/**', async (route) => {
      writeCalls++;
      await route.abort('blockedbyclient');
    });

    await signIn(page, 'admin', '/#pricing');
    await expect(page.locator('.pricing-screen').first(), '/#pricing loaded').toBeVisible({ timeout: 20_000 });

    const filter = page.locator('select[name="filter"]');
    await filter.selectOption('toApprove');
    await filter.selectOption('all');

    const search = page.locator('input[name="search"]');
    await search.fill('USD');
    await search.fill('');

    await page.locator('button[name="reload"]').click();

    const codeBtn = page.locator('.pricing-btn-code').first();
    // wait for the board to answer (a row or the empty state) before choosing a branch
    await expect(page.locator('.pricing-btn-code, [data-testid="pricing-grid-empty"]').first()).toBeVisible({ timeout: 20_000 });
    const hasRow = await codeBtn.isVisible().catch(() => false);
    if (hasRow) {
      await codeBtn.click();
      await expect(page.locator('.pricing-detail'), 'the pricing detail opened').toBeVisible({ timeout: 20_000 });
      // TierEditor.tsx:26,128: one row per tier, TIER_NOS = [1, 2, 3, 4]
      await expect(page.locator('.pricing-detail .tier-editor-table tbody tr'), 'the tier editor shows 4 tiers').toHaveCount(4);
    } else {
      // PricingGrid.tsx:29: an empty board must not fail the test
      await expect(page.getByTestId('pricing-grid-empty'), 'the empty board').toBeVisible();
    }

    const toggleDisabled = page.locator('button[name="toggleDisabled"]');
    if (await toggleDisabled.isVisible().catch(() => false)) {
      await toggleDisabled.click();
    }

    expect(writeCalls, 'zero write calls reached the route handler').toBe(0);
  });
});
