// QA-S7 — smoke: /staff cards, /staff/report, /staff/documents, /staff-guide (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S7; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { STAFF_LANGS, signIn, smoke } from './helpers';

test.describe('QA-S7 smoke: /staff cards, /staff/report, /staff/documents, /staff-guide', () => {
  // key: web/src/screens/staff/shift/ShiftCard.tsx:382
  test('/staff (teller)', async ({ page }) => {
    await signIn(page, 'teller', '/staff');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-teller',
      path: '/staff',
      role: 'teller',
      key: '.xc-shift-card',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/staff/transfers/TransfersCard.tsx:366
  test('/staff (branchManager)', async ({ page }) => {
    await signIn(page, 'branchManager', '/staff');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-manager',
      path: '/staff',
      role: 'branchManager',
      key: '[data-testid="transfer-new"]',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/staff/floor/FloorCard.tsx:295
  test('/staff (finance)', async ({ page }) => {
    await signIn(page, 'finance', '/staff');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-finance',
      path: '/staff',
      role: 'finance',
      key: '[data-testid="floor-target-form"]',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/staff-report/StaffReport.tsx:178
  test('/staff/report', async ({ page }) => {
    await signIn(page, 'teller', '/staff/report');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-report',
      path: '/staff/report',
      role: 'teller',
      key: '.staff-report',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/documents/DocumentsScreen.tsx:47
  test('/staff/documents', async ({ page }) => {
    await signIn(page, 'teller', '/staff/documents');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-documents',
      path: '/staff/documents',
      role: 'teller',
      key: '.documents-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/staff-guide/StaffGuide.tsx:26
  test('/staff-guide', async ({ page }) => {
    await signIn(page, 'teller', '/staff-guide');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-guide',
      path: '/staff-guide',
      role: 'teller',
      key: 'nav button',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/staff-guide/StaffGuide.tsx:26
  test('/staff-guide.html', async ({ page }) => {
    await signIn(page, 'teller', '/staff-guide.html');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-guide-html',
      path: '/staff-guide.html',
      role: 'teller',
      key: 'nav button',
      langs: STAFF_LANGS,
    });
  });
});
