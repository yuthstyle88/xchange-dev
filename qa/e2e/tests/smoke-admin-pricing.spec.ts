// QA-F10 — smoke: admin pricing menu (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-F10; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { hasAccount, mePermissions, signIn, smoke, STAFF_LANGS, users } from './helpers';

test.describe('QA-F10 smoke: admin pricing menu', () => {
  // ADMIN lacks ReferenceRatesRead (role_grants.rs:181-186, permission_module.rs:102); menu hidden (menu.ts:14, AdminShell.tsx:279-282,544-558);
  // fallback prevents .pricing-screen (AdminShell.tsx:285-300, Pricing.tsx:214); pricing_board refuses 403 ownerOnly (pricing_board.rs:2,69, owner_route.rs:22-26, policy.rs:273, policy_error.rs:106,132)
  test('/#pricing refused to admin: menu not offered, screen not rendered, pricing_board 403 ownerOnly', async ({ page }) => {
    test.skip(!hasAccount('admin'), 'no admin account in users.json: run scripts/reseed.sh');
    await signIn(page, 'admin', '/#pricing');
    const perms = await mePermissions(page);
    expect(perms, 'admin does not hold ReferenceRatesRead (role_grants.rs:181-186)').not.toContain('ReferenceRatesRead');

    await expect(page.locator('h2.title')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('nav.admin-nav a[href="#pricing"]')).toHaveCount(0);

    await expect(page.locator('.pricing-screen')).toHaveCount(0);
    await expect(page.locator('nav.admin-nav a[aria-current="page"]')).not.toHaveAttribute('href', '#pricing');

    const url = `/api/v3/views/pricing_board?branch=${encodeURIComponent(users.branch)}`;
    const res = await page.request.get(url);
    const body = await res.text();
    console.log(`[pricing_board admin] GET ${url} -> ${res.status()} ${body.slice(0, 200)}`);
    expect(res.status(), body).toBe(403);
    const code = /"code":"(\w+)"/.exec(body)?.[1] ?? '';
    expect(code, `refusal code: ${body}`).toBe('ownerOnly');
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

  // key: web/src/screens/admin/pricing/Pricing.tsx:154 (toolbar, search, reload, detail; needs ReferenceRatesRead)
  test('read-only clicks', async ({ page }) => {
    let writeCalls = 0;
    await page.route('**/api/pricing/**', async (route) => {
      writeCalls++;
      await route.abort('blockedbyclient');
    });
    await page.route('**/api/rate-references/**', async (route) => {
      writeCalls++;
      await route.abort('blockedbyclient');
    });

    const role = hasAccount('executive') ? 'executive' : hasAccount('finance') ? 'finance' : 'admin';
    await signIn(page, role, '/#pricing');
    const perms = await mePermissions(page);
    test.skip(!perms.includes('ReferenceRatesRead'), `${role} lacks ReferenceRatesRead`);
    await expect(page.locator('.pricing-screen').first(), '/#pricing loaded').toBeVisible({ timeout: 20_000 });

    const filter = page.locator('select[name="filter"]');
    await filter.selectOption('toApprove');
    await filter.selectOption('all');

    const search = page.locator('input[name="search"]');
    await search.fill('USD');
    await search.fill('');

    await page.locator('button[name="reload"]').click();
    // Pricing.tsx:319-321: .pricing-settings renders once pricing_board sets board()
    await expect(page.locator('.pricing-settings'), 'the pricing board loaded').toBeVisible({ timeout: 20_000 });

    // PricingGrid.tsx:21-22,46-72: on a branch with no published rates, all rows are disabled and folded behind toggleDisabled
    const toggleDisabled = page.locator('button[name="toggleDisabled"]');
    if (await toggleDisabled.isVisible().catch(() => false)) {
      await toggleDisabled.click();
    }

    const codeBtn = page.locator('.pricing-btn-code').first();
    // wait for the board to show a row (enabled or unfolded disabled) or the empty state
    await expect(page.locator('.pricing-btn-code, [data-testid="pricing-grid-empty"]').first()).toBeVisible({ timeout: 20_000 });
    const hasRow = await codeBtn.isVisible().catch(() => false);
    if (hasRow) {
      await codeBtn.click();
      await expect(page.locator('.pricing-detail'), 'the pricing detail opened').toBeVisible({ timeout: 20_000 });
      // TierEditor.tsx:28,183: one row per tier, TIER_NOS = [1, 2, 3, 4]
      await expect(page.locator('.pricing-detail .tier-editor-table tbody tr'), 'the tier editor shows 4 tiers').toHaveCount(4);
    } else {
      // PricingGrid.tsx:29: an empty board must not fail the test
      await expect(page.getByTestId('pricing-grid-empty'), 'the empty board').toBeVisible();
    }

    expect(writeCalls, 'zero write calls reached the route handler').toBe(0);
  });
});
