// QA-F11 — RBAC matrix: each role sees the menus, cards and pages its permissions allow (design docs/design/qa-e2e.md §3, §4).
// Written by ticket #4029 (QA-F11); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { hasAccount, mePermissions, type Role, signIn, users } from './helpers';

interface AdminMenuRule {
  id: string;
  order: number;
  needs: 'all' | string;
}

// Menu → needs table from web/src/screens/admin/<id>/menu.ts (sorted by order, matching AdminShell.tsx:274-282).
const ADMIN_MENUS: readonly AdminMenuRule[] = [
  // overview: menu.ts:16
  { id: 'overview', order: 1, needs: 'all' },
  // transactions: menu.ts:16
  { id: 'transactions', order: 2, needs: 'all' },
  // cash: menu.ts:12
  { id: 'cash', order: 4, needs: 'all' },
  // rates: menu.ts:12
  { id: 'rates', order: 5, needs: 'all' },
  // payments: menu.ts:17
  { id: 'payments', order: 6, needs: 'all' },
  // reports: menu.ts:12
  { id: 'reports', order: 7, needs: 'all' },
  // customers: menu.ts:14
  { id: 'customers', order: 8, needs: 'all' },
  // slips: menu.ts:16
  { id: 'slips', order: 9, needs: 'all' },
  // approvals: menu.ts:14
  { id: 'approvals', order: 11, needs: 'ApprovalsDecide' },
  // consent: menu.ts:14
  { id: 'consent', order: 12, needs: 'ConsentPolicyManage' },
  // denomination-groups: menu.ts:14
  { id: 'denomination-groups', order: 13, needs: 'RatesDraft' },
  // branches: menu.ts:12
  { id: 'branches', order: 14, needs: 'BranchesManage' },
  // users: menu.ts:12
  { id: 'users', order: 15, needs: 'UsersManage' },
  // alert-settings: menu.ts:12
  { id: 'alert-settings', order: 16, needs: 'AlertSettingsSave' },
  // executive: menu.ts:14
  { id: 'executive', order: 17, needs: 'OverviewRead' },
  // cash-policy: menu.ts:13
  { id: 'cash-policy', order: 18, needs: 'CashTransferPolicySet' },
  // reconciliation: menu.ts:14
  { id: 'reconciliation', order: 19, needs: 'FinanceReconcile' },
  // rate-alert-settings: menu.ts:13
  { id: 'rate-alert-settings', order: 20, needs: 'RateAlertSettingsSave' },
  // notices: menu.ts:13
  { id: 'notices', order: 21, needs: 'all' },
  // starting-prices: menu.ts:13
  { id: 'starting-prices', order: 22, needs: 'OrgStartingPricesManage' },
  // campaigns: menu.ts:13
  { id: 'campaigns', order: 23, needs: 'MarketingCampaigns' },
  // campaign-performance: menu.ts:13
  { id: 'campaign-performance', order: 24, needs: 'MarketingPerformance' },
  // booking-slots: menu.ts:13
  { id: 'booking-slots', order: 25, needs: 'BranchesManage' },
  // members: menu.ts:12
  { id: 'members', order: 26, needs: 'MarketingMembers' },
  // loyalty: menu.ts:13
  { id: 'loyalty', order: 27, needs: 'LoyaltyRulesManage' },
  // cash-limits: menu.ts:13
  { id: 'cash-limits', order: 28, needs: 'CashLimitsManage' },
  // pricing: menu.ts:14
  { id: 'pricing', order: 29, needs: 'ReferenceRatesRead' },
];

interface StaffCardRule {
  id: string;
  visible: (perms: readonly string[]) => boolean;
}

// Staff cards `[data-card=<id>]` (StaffPage.tsx:169-170) and their `visible` rules from web/src/screens/staff/*/card.ts:8-9.
const STAFF_CARDS: readonly StaffCardRule[] = [
  // floor: web/src/screens/staff/floor/card.ts:8
  { id: 'floor', visible: (perms) => perms.includes('AttendanceClock') },
  // shift: web/src/screens/staff/shift/card.ts:8-9
  { id: 'shift', visible: (perms) => perms.includes('ShiftOperate') || perms.includes('ShiftCloseCertify') },
  // transfers: web/src/screens/staff/transfers/card.ts:8-9
  { id: 'transfers', visible: (perms) => perms.includes('CashManage') },
  // rates: web/src/screens/staff/rates/card.ts:8
  { id: 'rates', visible: (perms) => perms.includes('CompatRead') },
  // holds: web/src/screens/staff/holds/card.ts:8-9
  { id: 'holds', visible: (perms) => perms.includes('CustomerHoldPlace') || perms.includes('VerificationReview') },
  // attendance: web/src/screens/staff/attendance/card.ts:8
  { id: 'attendance', visible: (perms) => perms.includes('AttendanceClock') },
];

const STAFF_ROLES: readonly Role[] = ['teller', 'branchManager', 'finance', 'marketing', 'admin'];

function adminBasePath(role: Role): string {
  return role === 'admin' ? `/?branch=${encodeURIComponent(users.branch)}` : '/';
}

test.describe('QA-F11 smoke: RBAC matrix across menus, staff cards and verification', () => {
  test('signed-in customer is forwarded to /app from staff, desk, admin and verification routes', async ({ page }) => {
    if (!hasAccount('customer')) {
      console.log('[rbac] skipping customer: no customer account in users.json');
      test.skip(true, 'no customer account in users.json: run scripts/reseed.sh');
    }
    await signIn(page, 'customer', '/app');

    const gatedPaths = ['/', '/staff', `/desk?branch=${encodeURIComponent(users.branch)}`, '/operations/verification'];
    for (const target of gatedPaths) {
      // The customer is signed in above, so since #4351 the login page forwards them to /app. Wait for that
      // forward and the home page before the next goto, so no navigation is still in flight.
      await page.goto(target, { waitUntil: 'commit' });
      await page.waitForURL((url) => url.pathname === '/app', { timeout: 20_000 });
      await expect(
        page.locator('.xc-customer-home'),
        `customer visiting ${target} settles on /app`,
      ).toBeVisible({ timeout: 20_000 });
      expect(new URL(page.url()).pathname, `customer visiting ${target} lands on /app`).toBe('/app');
    }
  });

  for (const role of STAFF_ROLES) {
    test(`${role}: admin menu links and refused hashes match me.permissions`, async ({ page }) => {
      if (!hasAccount(role)) {
        console.log(`[rbac] skipping ${role}: no ${role} account in users.json`);
        test.skip(true, `no ${role} account in users.json: run scripts/reseed.sh`);
      }
      const basePath = adminBasePath(role);
      await signIn(page, role, basePath);
      const perms = await mePermissions(page);
      expect(perms.length, 'me.permissions is empty: /api/v3/views/me failed or the account has no grants').toBeGreaterThan(0);

      await page.goto(basePath);
      const heading = page.locator('h2.title');
      await expect(heading, `${role} (${basePath}): lowest-order allowed menu heading visible`).toBeVisible({
        timeout: 20_000,
      });
      const lowestOrderHeading = (await heading.innerText()).trim();
      expect(lowestOrderHeading, `${role}: lowest-order allowed menu title is non-empty`).toBeTruthy();

      const lowestAllowed = ADMIN_MENUS.find((m) => m.needs === 'all' || perms.includes(m.needs));
      expect(lowestAllowed, `${role}: at least one allowed menu exists`).toBeTruthy();

      const refusedIds: string[] = [];
      for (const menu of ADMIN_MENUS) {
        const allowed = menu.needs === 'all' || perms.includes(menu.needs);
        const link = page.locator(`nav.admin-nav a[href="#${menu.id}"]`);
        if (allowed) {
          await expect(link, `${role}: allowed menu link #${menu.id} (needs ${menu.needs}) exists`).toHaveCount(1);
        } else {
          await expect(link, `${role}: refused menu link #${menu.id} (needs ${menu.needs}) absent`).toHaveCount(0);
          refusedIds.push(menu.id);
        }
      }

      for (const refusedId of refusedIds) {
        await page.goto(`${basePath}#${refusedId}`);
        await expect(heading, `${role} (#${refusedId}): heading visible`).toBeVisible({ timeout: 20_000 });
        expect(
          new URL(page.url()).hash,
          `${role} (#${refusedId}): refused hash stays #${refusedId} (AdminShell.test.tsx:755)`,
        ).toBe(`#${refusedId}`);
        const currentLink = page.locator('nav.admin-nav a[aria-current="page"]');
        await expect(
          currentLink,
          `${role} (#${refusedId}): aria-current="page" is on the lowest-order allowed menu (#${lowestAllowed!.id})`,
        ).toHaveAttribute('href', `#${lowestAllowed!.id}`);
        await expect(
          heading,
          `${role} (#${refusedId}): heading matches lowest-order allowed menu (${lowestOrderHeading})`,
        ).toHaveText(lowestOrderHeading);
      }
    });

    test(`${role}: /staff cards match visible rules for me.permissions`, async ({ page }) => {
      if (!hasAccount(role)) {
        console.log(`[rbac] skipping ${role}: no ${role} account in users.json`);
        test.skip(true, `no ${role} account in users.json: run scripts/reseed.sh`);
      }
      await signIn(page, role, '/staff');
      const perms = await mePermissions(page);
      expect(perms.length, 'me.permissions is empty: /api/v3/views/me failed or the account has no grants').toBeGreaterThan(0);

      await page.goto('/staff');
      await expect(page.getByTestId('staff-hours'), `${role}: /staff loaded`).toBeVisible({ timeout: 20_000 });

      for (const card of STAFF_CARDS) {
        const expected = card.visible(perms);
        const locator = page.locator(`[data-card="${card.id}"]`);
        if (expected) {
          await expect(locator, `${role}: /staff card [data-card="${card.id}"] present`).toHaveCount(1);
        } else {
          await expect(locator, `${role}: /staff card [data-card="${card.id}"] absent`).toHaveCount(0);
        }
      }
    });

    test(`${role}: /operations/verification [data-refused] matches VerificationReview permission`, async ({ page }) => {
      if (!hasAccount(role)) {
        console.log(`[rbac] skipping ${role}: no ${role} account in users.json`);
        test.skip(true, `no ${role} account in users.json: run scripts/reseed.sh`);
      }
      await signIn(page, role, '/operations/verification');
      const perms = await mePermissions(page);
      expect(perms.length, 'me.permissions is empty: /api/v3/views/me failed or the account has no grants').toBeGreaterThan(0);

      await page.goto('/operations/verification');
      const screen = page.getByTestId('verification-screen');
      await expect(screen, `${role}: /operations/verification screen mounted`).toBeVisible({ timeout: 20_000 });

      const refused = screen.locator('[data-refused]');
      if (!perms.includes('VerificationReview')) {
        await expect(
          refused,
          `${role}: [data-refused] visible without VerificationReview (VerificationScreen.tsx:274-277)`,
        ).toBeVisible({ timeout: 20_000 });
      } else {
        await expect(
          screen.locator('.xc-verification-grid'),
          `${role}: verification queue grid visible with VerificationReview`,
        ).toBeVisible({ timeout: 20_000 });
        await expect(refused, `${role}: [data-refused] hidden with VerificationReview`).toHaveCount(0);
      }
    });
  }

  test('sanity pins: marketing lacks UsersManage and ApprovalsDecide; teller lacks ApprovalsDecide', async ({
    browser,
  }) => {
    if (hasAccount('marketing')) {
      const mktPage = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await signIn(mktPage, 'marketing', '/');
        const mktPerms = await mePermissions(mktPage);
        expect(mktPerms.length, 'me.permissions is empty: /api/v3/views/me failed or the account has no grants').toBeGreaterThan(0);
        expect(mktPerms, 'marketing lacks UsersManage').not.toContain('UsersManage');
        expect(mktPerms, 'marketing lacks ApprovalsDecide').not.toContain('ApprovalsDecide');
      } finally {
        await mktPage.close().catch(() => {});
      }
    } else {
      console.log('[rbac] skipping marketing sanity pin: no marketing account in users.json');
    }

    if (hasAccount('teller')) {
      const tellerPage = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await signIn(tellerPage, 'teller', '/staff');
        const tellerPerms = await mePermissions(tellerPage);
        expect(tellerPerms.length, 'me.permissions is empty: /api/v3/views/me failed or the account has no grants').toBeGreaterThan(0);
        expect(tellerPerms, 'teller lacks ApprovalsDecide').not.toContain('ApprovalsDecide');
      } finally {
        await tellerPage.close().catch(() => {});
      }
    } else {
      console.log('[rbac] skipping teller sanity pin: no teller account in users.json');
      test.skip(true, 'no teller account in users.json: run scripts/reseed.sh');
    }
  });
});
