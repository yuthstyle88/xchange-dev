// QA-S5 — smoke: admin org menus and the queue bar (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S5; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { STAFF_LANGS, signIn, smoke } from './helpers';

test.describe('QA-S5 smoke: admin org menus and the queue bar', () => {
  // key: web/src/screens/admin/customers/Customers.tsx:419
  test('/#customers', async ({ page }) => {
    await signIn(page, 'admin', '/#customers');
    await smoke(page, {
      area: 'admin-org',
      screen: 'customers',
      path: '/#customers',
      role: 'admin',
      key: '.customers-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/employees/Employees.tsx:317
  test('/#employees', async ({ page }) => {
    await signIn(page, 'admin', '/#employees');
    await smoke(page, {
      area: 'admin-org',
      screen: 'employees',
      path: '/#employees',
      role: 'admin',
      key: '.employees-card',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/approvals/Approvals.tsx:319
  test('/#approvals', async ({ page }) => {
    await signIn(page, 'admin', '/#approvals');
    await smoke(page, {
      area: 'admin-org',
      screen: 'approvals',
      path: '/#approvals',
      role: 'admin',
      key: '.admin-approvals-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/consent/Consent.tsx:187
  test('/#consent', async ({ page }) => {
    await signIn(page, 'admin', '/#consent');
    await smoke(page, {
      area: 'admin-org',
      screen: 'consent',
      path: '/#consent',
      role: 'admin',
      key: '.admin-consent-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/branches/BranchesScreen.tsx:237
  test('/#branches', async ({ page }) => {
    await signIn(page, 'admin', '/#branches');
    await smoke(page, {
      area: 'admin-org',
      screen: 'branches',
      path: '/#branches',
      role: 'admin',
      key: '.branches-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/users/UsersScreen.tsx:499
  test('/#users', async ({ page }) => {
    await signIn(page, 'admin', '/#users');
    await smoke(page, {
      area: 'admin-org',
      screen: 'users',
      path: '/#users',
      role: 'admin',
      key: '.users-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/executive/Executive.tsx:285
  test('/#executive', async ({ page }) => {
    await signIn(page, 'admin', '/#executive');
    await smoke(page, {
      area: 'admin-org',
      screen: 'executive',
      path: '/#executive',
      role: 'admin',
      key: '.executive',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/notices/Notices.tsx:32
  test('/#notices', async ({ page }) => {
    await signIn(page, 'admin', '/#notices');
    await smoke(page, {
      area: 'admin-org',
      screen: 'notices',
      path: '/#notices',
      role: 'admin',
      key: '.admin-notices',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/booking-slots/BookingSlots.tsx:383
  test('/#booking-slots', async ({ page }) => {
    await signIn(page, 'admin', '/#booking-slots');
    await smoke(page, {
      area: 'admin-org',
      screen: 'booking-slots',
      path: '/#booking-slots',
      role: 'admin',
      key: '.booking-slots-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/admin/queue/QueueBar.tsx:197-200. The bar shows nothing until the queue has news (a toast,
  // :171-195) and its dialog stays closed, so `smoke`'s visibility check cannot apply: assert it is mounted instead.
  test('/#overview (the QueueBar shown on every menu)', async ({ page }) => {
    await signIn(page, 'admin', '/#overview');
    await page.goto('/#overview');
    await expect(page.locator('dialog.queue-dialog'), 'the admin QueueBar is mounted').toBeAttached({ timeout: 20_000 });
  });
});
