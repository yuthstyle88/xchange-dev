// QA-S4 — smoke: admin money menus (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S4; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { signIn, smoke, STAFF_LANGS } from './helpers';

test.describe('QA-S4 smoke: admin money menus', () => {
  // key: web/src/screens/admin/overview/Overview.tsx:205
  test('/#overview', async ({ page }) => {
    await signIn(page, 'admin', '/#overview');
    await smoke(page, {
      area: 'admin-money',
      screen: 'overview',
      path: '/#overview',
      role: 'admin',
      key: '.overview-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/transactions/Transactions.tsx:269
  test('/#transactions', async ({ page }) => {
    await signIn(page, 'admin', '/#transactions');
    await smoke(page, {
      area: 'admin-money',
      screen: 'transactions',
      path: '/#transactions',
      role: 'admin',
      key: '.transactions-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/cash/Cash.tsx:257
  test('/#cash', async ({ page }) => {
    await signIn(page, 'admin', '/#cash');
    await smoke(page, {
      area: 'admin-money',
      screen: 'cash',
      path: '/#cash',
      role: 'admin',
      key: '.admin-cash-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/rates/Rates.tsx:313
  test('/#rates', async ({ page }) => {
    await signIn(page, 'admin', '/#rates');
    await smoke(page, {
      area: 'admin-money',
      screen: 'rates',
      path: '/#rates',
      role: 'admin',
      key: '.admin-rates-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/payments/Payments.tsx:381
  test('/#payments', async ({ page }) => {
    await signIn(page, 'admin', '/#payments');
    await smoke(page, {
      area: 'admin-money',
      screen: 'payments',
      path: '/#payments',
      role: 'admin',
      key: '.xc-payments',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/reports/Reports.tsx:357
  test('/#reports', async ({ page }) => {
    await signIn(page, 'admin', '/#reports');
    await smoke(page, {
      area: 'admin-money',
      screen: 'reports',
      path: '/#reports',
      role: 'admin',
      key: '.reports-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/slips/Slips.tsx:190
  test('/#slips', async ({ page }) => {
    await signIn(page, 'admin', '/#slips');
    await smoke(page, {
      area: 'admin-money',
      screen: 'slips',
      path: '/#slips',
      role: 'admin',
      key: '.slips-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/reconciliation/Reconciliation.tsx:124
  test('/#reconciliation', async ({ page }) => {
    await signIn(page, 'admin', '/#reconciliation');
    await smoke(page, {
      area: 'admin-money',
      screen: 'reconciliation',
      path: '/#reconciliation',
      role: 'admin',
      key: '.reconciliation',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/starting-prices/StartingPrices.tsx:322
  test('/#starting-prices', async ({ page }) => {
    await signIn(page, 'admin', '/#starting-prices');
    await smoke(page, {
      area: 'admin-money',
      screen: 'starting-prices',
      path: '/#starting-prices',
      role: 'admin',
      key: '.xc-sp-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/denomination-groups/DenominationGroups.tsx:226
  test('/#denomination-groups', async ({ page }) => {
    await signIn(page, 'admin', '/#denomination-groups');
    await smoke(page, {
      area: 'admin-money',
      screen: 'denomination-groups',
      path: '/#denomination-groups',
      role: 'admin',
      key: '.denomination-groups-screen',
      langs: STAFF_LANGS,
    });
  });
});
