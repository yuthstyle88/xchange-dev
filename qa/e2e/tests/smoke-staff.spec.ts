// QA-S7 — smoke: /staff cards, /staff/report, /staff/documents, /staff-guide (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S7; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
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

  // key: web/src/screens/staff-report/StaffReport.tsx:178. Smoked as the branch manager: `staff_daily_report` admits only
  // ShiftCloseCertify, FinanceReconcile, or OverviewRead (`api/crates/xc-auth/src/domain/policy/areas/staff_report.rs:13-17`;
  // #4155 removed TELLER's OverviewRead grant, so a teller is refused 400 roleNotAllowed).
  test('/staff/report', async ({ page }) => {
    await signIn(page, 'branchManager', '/staff/report');
    await smoke(page, {
      area: 'staff',
      screen: 'staff-report',
      path: '/staff/report',
      role: 'branchManager',
      key: '.staff-report',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/staff-report/StaffReport.tsx:166-170,201-206 (`staff_report.rs:13-17`, #4155, #4162)
  test('/staff/report (teller is refused)', async ({ page }) => {
    await signIn(page, 'teller', '/staff');
    const reportResPromise = page.waitForResponse(
      (r) => r.request().method() === 'GET' && new URL(r.url()).pathname === '/api/v3/views/staff_daily_report',
      { timeout: 20_000 },
    );
    await page.goto('/staff/report');
    const reportRes = await reportResPromise;
    const reportBody = await reportRes.text().catch(() => '');
    expect(reportRes.status(), `teller GET /api/v3/views/staff_daily_report status: ${reportBody}`).toBe(400);
    expect(reportBody, 'teller GET /api/v3/views/staff_daily_report code').toContain('roleNotAllowed');
    const refusal = page.locator('.staff-report-refusal');
    await expect(refusal, 'teller on /staff/report sees refusal state (StaffReport.tsx:202)').toBeVisible({
      timeout: 20_000,
    });
    await expect(refusal, 'refusal words roleNotAllowed (i18n.ts:10,52)').toContainText(
      /บทบาทนี้ไม่มีสิทธิ์อ่านรายงานเงิน|This role may not read the money report/,
    );
    await expect(
      page.locator('.staff-report-sheet, .staff-report-table, .staff-report table tbody tr'),
      'teller on /staff/report must not see sheet rows (StaffReport.tsx:75,103)',
    ).toHaveCount(0);
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
      allowConsole: ['documentsNotFound404'],
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
